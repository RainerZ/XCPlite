#!/bin/bash
# test_multi_thread_image.sh - End-to-end test of the measurement of a large array: the image of multi_thread_demo
#
# Starts build/multi_thread_demo and measures its images with xcpclient to MDF files, in continuous render mode:
#   1. The JPEG image of the video: 'jpeg' (a buffer of 256 KB in one DAQ list with 33 ODTs), 'jpeg_size' and 'jpeg_image' with
#      the event jpeg. Each sample has to be a complete JPEG image of the video resolution, the image numbers increase.
#   2. The uncompressed image 'pixels' (an array of X_RES * Y_RES bytes with the event batch_done, 1600 x 1200 in one DAQ list with
#      242 ODTs), without zoom animation (mandelbrot.zoom 1): the last sample has to match a reference render of the area.
#      It needs about 50 KB DAQ memory, more than the default OPTION_DAQ_MEM_SIZE of 16 KB: if the XCP server rejects the DAQ
#      setup with a memory overflow, this part is skipped with a message.
# Both measurements: at least MIN_SAMPLES complete samples, no lost DAQ packets, no dropped incomplete samples.
# The images are checked by mdf_image_check.py.
#
# Usage:
#   test/test_multi_thread_image.sh [--keep]     --keep: keep the temporary directory with the logs and the MDF file
#
# Requires ./build.sh examples (default configuration), xcpclient on PATH (or XCPCLIENT=<path>) and python3.
# Uses UDP port 5555 of multi_thread_demo, no other instance may be running.
# Environment: BUILD_DIR (default build), XCPCLIENT (default xcpclient), MEASURE_TIME in s (default 3), MIN_SAMPLES (default 5)

set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BUILD_DIR="${BUILD_DIR:-${WORKSPACE_ROOT}/build}"
XCPCLIENT="${XCPCLIENT:-xcpclient}"
MEASURE_TIME="${MEASURE_TIME:-3}"
MIN_SAMPLES="${MIN_SAMPLES:-5}"
DEMO="${BUILD_DIR}/multi_thread_demo"
KEEP=false
[ "${1:-}" = "--keep" ] && KEEP=true

fail() {
    echo "FAIL: $*"
    exit 1
}

# Preconditions
[ -x "$DEMO" ] || fail "$DEMO not found, run ./build.sh examples"
command -v "$XCPCLIENT" >/dev/null 2>&1 || fail "xcpclient not found, install it or set XCPCLIENT=<path>"
command -v python3 >/dev/null 2>&1 || fail "python3 not found"
[ -f "${WORKSPACE_ROOT}/XCP_104.aml" ] || fail "${WORKSPACE_ROOT}/XCP_104.aml not found"
if command -v lsof >/dev/null 2>&1 && lsof -nP -iUDP:5555 >/dev/null 2>&1; then
    fail "UDP port 5555 is in use, stop the running XCP server (multi_thread_demo) first"
fi

# The demo and xcpclient run in separate directories of a temporary directory, the demo writes its A2L file into its
# working directory, xcpclient uploads it into its own
TMP_DIR="$(mktemp -d)"
DEMO_PID=""
cleanup() {
    if [ -n "$DEMO_PID" ] && kill -0 "$DEMO_PID" 2>/dev/null; then
        kill -INT "$DEMO_PID" 2>/dev/null
        for _ in $(seq 1 50); do
            kill -0 "$DEMO_PID" 2>/dev/null || break
            sleep 0.1
        done
        kill -KILL "$DEMO_PID" 2>/dev/null
    fi
    if [ "$KEEP" = true ]; then
        echo "Logs and MDF file kept in $TMP_DIR"
    else
        rm -rf "$TMP_DIR"
    fi
}
trap cleanup EXIT
mkdir -p "$TMP_DIR/demo" "$TMP_DIR/client"
cp "${WORKSPACE_ROOT}/XCP_104.aml" "$TMP_DIR/client/"

# Start the demo and wait until its XCP server listens on port 5555 (its console output to the log file is buffered)
(cd "$TMP_DIR/demo" && exec "$DEMO" >demo.log 2>&1) &
DEMO_PID=$!
if command -v lsof >/dev/null 2>&1; then
    for _ in $(seq 1 100); do
        lsof -nP -a -p "$DEMO_PID" -iUDP:5555 >/dev/null 2>&1 && break
        kill -0 "$DEMO_PID" 2>/dev/null || fail "multi_thread_demo terminated, see $TMP_DIR/demo/demo.log"
        sleep 0.1
    done
    lsof -nP -a -p "$DEMO_PID" -iUDP:5555 >/dev/null 2>&1 || fail "multi_thread_demo does not listen on UDP port 5555"
    sleep 0.5 # Worker threads
else
    sleep 3
fi
kill -0 "$DEMO_PID" 2>/dev/null || fail "multi_thread_demo terminated, see $TMP_DIR/demo/demo.log"

# Run xcpclient in the client directory, the output is appended to $TMP_DIR/client/xcpclient.log
run_xcpclient() {
    (cd "$TMP_DIR/client" && "$XCPCLIENT" --dest-addr 127.0.0.1 --port 5555 --udp "$@" >>xcpclient.log 2>&1)
}

# Check the statistics of the last measurement: Measurement done, <n> events, ... , <p> packets, <l> packets lost
# $1: name of the measurement, sets SAMPLES
check_statistics() {
    local summary
    summary="$(grep "Measurement done" "$TMP_DIR/client/xcpclient.log" | tail -1)"
    [ -n "$summary" ] || fail "$1: no measurement summary from xcpclient"
    echo "  $summary"
    SAMPLES="$(echo "$summary" | sed -E 's/.*Measurement done, ([0-9]+) events.*/\1/')"
    local lost
    lost="$(echo "$summary" | sed -E 's/.* ([0-9]+) packets lost.*/\1/')"
    [ "$SAMPLES" -ge "$MIN_SAMPLES" ] || fail "$1: only $SAMPLES samples, expected at least $MIN_SAMPLES"
    [ "$lost" -eq 0 ] || fail "$1: $lost DAQ packets lost"
    if tail -20 "$TMP_DIR/client/xcpclient.log" | grep -q "incomplete samples"; then
        fail "$1: incomplete samples dropped"
    fi
}

WORK_C="${WORKSPACE_ROOT}/examples/multi_thread_demo/src/work.c"

# 1. The JPEG image, the demo renders continuously by default
echo "Measuring jpeg for ${MEASURE_TIME} s ..."
run_xcpclient --upload-a2l --mea jpeg jpeg_size jpeg_image --time "$MEASURE_TIME" --mdf jpeg.mf4 || fail "jpeg: xcpclient failed, see $TMP_DIR/client/xcpclient.log"
check_statistics jpeg
python3 -I "${SCRIPT_DIR}/mdf_image_check.py" jpeg "$TMP_DIR/client/jpeg.mf4" "$WORK_C" || fail "jpeg: image check"
JPEG_SAMPLES=$SAMPLES

# 2. The uncompressed image, without zoom animation, so the image is the default area of work.c
# Switch continuous mode off (ends the zoom), set zoom 1, the measurement switches continuous mode on again, the zoom restarts at
# the default width. The calibration parameters persist in the demo, xcpclient sets one with each call
run_xcpclient --a2l multi_thread_demo.a2l --cal mandelbrot.continuous 0 || fail "xcpclient could not set mandelbrot.continuous"
run_xcpclient --a2l multi_thread_demo.a2l --cal mandelbrot.zoom 1 || fail "xcpclient could not set mandelbrot.zoom"
echo "Measuring pixels for ${MEASURE_TIME} s ..."
if ! run_xcpclient --a2l multi_thread_demo.a2l --cal mandelbrot.continuous 1 --mea pixels --time "$MEASURE_TIME" --mdf pixels.mf4; then
    if tail -5 "$TMP_DIR/client/xcpclient.log" | grep -q "Memory overflow"; then
        echo "SKIP pixels: the DAQ memory of the XCP server is too small, pixels needs about 50 KB (OPTION_DAQ_MEM_SIZE in src/xcplib_cfg.h)"
        echo "PASS: jpeg $JPEG_SAMPLES events, 0 packets lost, complete JPEG images (pixels skipped)"
        exit 0
    fi
    fail "pixels: xcpclient failed, see $TMP_DIR/client/xcpclient.log"
fi
check_statistics pixels
python3 -I "${SCRIPT_DIR}/mdf_image_check.py" pixels "$TMP_DIR/client/pixels.mf4" "$WORK_C" || fail "pixels: image check"
echo "PASS: jpeg $JPEG_SAMPLES events, pixels $SAMPLES events, 0 packets lost, complete JPEG images, pixels match the reference"
