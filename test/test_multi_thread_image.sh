#!/bin/bash
# test_multi_thread_image.sh - End-to-end test of the measurement of a large array: the image of multi_thread_demo
#
# Starts build/multi_thread_demo, measures its image 'pixels' (an array of X_RES * Y_RES bytes with the event batch_done,
# 1600 x 1200 in one DAQ list with 242 ODTs) with xcpclient in continuous render mode without zoom animation
# (mandelbrot.zoom 1) to an MDF file and checks:
#   - the measurement: at least MIN_SAMPLES complete samples, no lost DAQ packets, no dropped incomplete samples
#   - the image: the last sample in the MDF file against a reference render of the same area (mdf_image_check.py)
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

# Measure the image in continuous render mode, without zoom animation, so the image is the default area of work.c
# The demo starts zooming in continuous mode: switch continuous mode off (ends the zoom), set zoom 1, the measurement switches
# continuous mode on again, the zoom restarts at the default width
# The calibration parameters persist in the demo, xcpclient sets one with each call
set_cal() {
    (cd "$TMP_DIR/client" && "$XCPCLIENT" --dest-addr 127.0.0.1 --port 5555 --udp --upload-a2l --cal "$1" "$2" >>xcpclient_cal.log 2>&1) ||
        fail "xcpclient could not set $1, see $TMP_DIR/client/xcpclient_cal.log"
}
set_cal mandelbrot.continuous 0
set_cal mandelbrot.zoom 1
echo "Measuring pixels for ${MEASURE_TIME} s ..."
(cd "$TMP_DIR/client" &&
    "$XCPCLIENT" --dest-addr 127.0.0.1 --port 5555 --udp --a2l multi_thread_demo.a2l --cal mandelbrot.continuous 1 --mea pixels --time "$MEASURE_TIME" --mdf pixels.mf4 \
        >xcpclient.log 2>&1)
XCP_STATUS=$?
grep -E "Measurement done|WARN|ERROR" "$TMP_DIR/client/xcpclient.log"
[ $XCP_STATUS -eq 0 ] || fail "xcpclient exit status $XCP_STATUS"

# Measurement statistics: Measurement done, <n> events, ... , <p> packets, <l> packets lost
SUMMARY="$(grep "Measurement done" "$TMP_DIR/client/xcpclient.log")"
[ -n "$SUMMARY" ] || fail "no measurement summary from xcpclient"
SAMPLES="$(echo "$SUMMARY" | sed -E 's/.*Measurement done, ([0-9]+) events.*/\1/')"
LOST="$(echo "$SUMMARY" | sed -E 's/.* ([0-9]+) packets lost.*/\1/')"
[ "$SAMPLES" -ge "$MIN_SAMPLES" ] || fail "only $SAMPLES samples of pixels, expected at least $MIN_SAMPLES"
[ "$LOST" -eq 0 ] || fail "$LOST DAQ packets lost"
if grep -q "incomplete samples" "$TMP_DIR/client/xcpclient.log"; then
    fail "incomplete samples dropped"
fi

# The image
python3 -I "${SCRIPT_DIR}/mdf_image_check.py" "$TMP_DIR/client/pixels.mf4" "${WORKSPACE_ROOT}/examples/multi_thread_demo/src/work.c" || fail "image check"
echo "PASS: $SAMPLES samples of pixels, 0 packets lost, image matches the reference"
