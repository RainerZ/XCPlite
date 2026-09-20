#!/bin/bash

# A2L file creator for the frida example
# XCPlite build with option OPTION_SECTION_REGISTRATION

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# The script syncs the example project to the target, builds it there, runs it with XCP on Ethernet,
# downloads the ELF file to the local machine and creates an A2L file. 
# Prerequisites:
# - The target machine must be Linux
# - The target must be reachable via SSH and have rsync installed
# - The local machine must have rsync and scp installed
# - The local machine must have xcpclient installed


#======================================================================================================================
# Parameters
#======================================================================================================================

LOGFILE="$REPO_ROOT/examples/frida_demo/CANape/frida_demo.log"
#LOGFILE='/dev/stdout'
#LOGFILE="/dev/null"

# A2L file path on local machine
A2LFILE="$REPO_ROOT/examples/frida_demo/CANape/frida_demo.a2l"

# ELF file path on local machine
ELFFILE="$REPO_ROOT/examples/frida_demo/CANape/frida_demo.elf"

# Build type for target executable: Release, RelWithDebInfo or Debug
# RelWithDebInfo is default to demonstrate operation with with -O1 and NDEBUG
# Optimization level >= -O1 keeps variables in registers whenever possible, so these local variables cannot be measured
# Debug mode is the least efficient but keeps all variables and stack frames intact
BUILD_TYPE="RelWithDebInfo"
# -O0
#BUILD_TYPE="Debug"
# -O2 no debug symbols
#BUILD_TYPE="Release"

# Run a simple test calibration and measurement
TEST=false
# CSV measurement file path on local machine
CSVFILE="$REPO_ROOT/examples/frida_demo/CANape/frida_demo.csv"


# Target connection details
TARGET_USER="rainer"
TARGET_HOST="192.168.8.135"
TARGET_PATH="~/XCPlite-frida_demo"
TARGET_BUILD_DIR="examples/frida_demo/build"
TARGET_BINARY="frida_demo"

# Path to xcpclient tool executable (assuming cargo installed it to ~/.cargo/bin)
XCPCLIENT="xcpclient"


#======================================================================================================================
# Sync Target, Build Application on Target, Download ELF, Start ECU, ...
#======================================================================================================================


echo "========================================================================================================"
echo "A2L file creator for the frida_demo example project"
echo "========================================================================================================"

mkdir -p "$(dirname "$LOGFILE")"
echo "Logging to $LOGFILE enabled"
echo "" > "$LOGFILE"

#======================================================================================================================
# ECU_ONLINE
# Sync target, build, upload ELF, start application on target
#======================================================================================================================

# Sync target
echo "Sync target ..."            
rsync -avz --delete \
    --include='/build.sh' \
    --include='/CMakeLists.txt' \
    --include='/cmake/***' \
    --include='/inc/***' \
    --include='/src/***' \
    --include='/examples/' \
    --exclude='/examples/frida_demo/build/***' \
    --include='/examples/frida_demo/***' \
    --exclude='*' \
    "$REPO_ROOT/" "$TARGET_USER@$TARGET_HOST:$TARGET_PATH/" 1> /dev/null
if [ $? -ne 0 ]; then
    echo "❌ FAILED: Rsync with target"
    exit 1
fi


# Build on target
# Always a clean build: if the target has no NTP its clock may skew
# local XCPlite library build
echo "Clean $BUILD_TYPE (local XCPlite library) build on Target ..."
ssh "$TARGET_USER@$TARGET_HOST" \
    "cd $TARGET_PATH/examples/frida_demo && ./build.sh $BUILD_TYPE local clean" \
    >> "$LOGFILE" 2>&1
if [ $? -ne 0 ]; then
    echo "❌ FAILED: Build on target"
    exit 1
fi


# Download the target executable for the local A2L generation process
echo "Downloading ELF file from target $TARGET_PATH/$TARGET_BUILD_DIR/$TARGET_BINARY to $ELFFILE ..."
scp "$TARGET_USER@$TARGET_HOST:$TARGET_PATH/$TARGET_BUILD_DIR/$TARGET_BINARY" "$ELFFILE" 1> /dev/null
if [ $? -ne 0 ]; then
    echo "❌ FAILED: Download $TARGET_PATH/$TARGET_BUILD_DIR/$TARGET_BINARY"
    exit 1
fi



#======================================================================================================================
# Create A2L file
# Create the A2L from ELF file with xcpclient tool
#======================================================================================================================

echo ""
echo "========================================================================================================"
echo "Creating A2L file from XCPlite ELF file ..."
echo "========================================================================================================"
echo ""
# Remove the A2L file of a previous run, so a failed generation can not leave a stale A2L file behind
rm -f "$A2LFILE"
# The unit filter '^main_c$' is a regular expression on the file name of the compilation unit with '.' replaced by '_',
# anchored to select main.c only 
XCPCLIENT_ARGS=(--log-level=3 --verbose=0 --dest-addr="$TARGET_HOST" --udp --offline --elf "$ELFFILE" --elf-unit-filter '^main_c$' --create-a2l --a2l "$A2LFILE" --default-event=mainloop)
echo "Command: $XCPCLIENT ${XCPCLIENT_ARGS[*]}"
"$XCPCLIENT" "${XCPCLIENT_ARGS[@]}" >> "$LOGFILE"
if [ $? -ne 0 ] || [ ! -f "$A2LFILE" ]; then
    echo "❌ FAILED: xcpclient could not create the A2L file $A2LFILE, see $LOGFILE"
    grep "\[ERROR\]" "$LOGFILE"
    exit 1
fi



echo ""
echo "✅ SUCCESS:"
echo "Created a new A2L file $A2LFILE"
echo ""


#======================================================================================================================
# Test
#======================================================================================================================

if [ "$TEST" = true ]; then

ssh "$TARGET_USER@$TARGET_HOST" "cd $TARGET_PATH && ./$TARGET_BUILD_DIR/$TARGET_BINARY" &
sleep 1

echo "========================================================================================================"
echo "Test connect"
echo "========================================================================================================"
$XCPCLIENT --log-level=3 --dest-addr=$TARGET_HOST:5555 --udp --a2l "$A2LFILE" --list-mea . --list-cal . 
sleep 1

echo "========================================================================================================"
echo "Test measurement"
echo "========================================================================================================"
# Log measurement to stdout
$XCPCLIENT --log-level=3 --dest-addr=$TARGET_HOST:5555 --udp --a2l "$A2LFILE"  --mea foo_ctx --time 3 --verbose=2
# Log measurement to CSV file
#$XCPCLIENT --log-level=3 --dest-addr=$TARGET_HOST:5555 --udp --a2l "$A2LFILE"  --mea foo_ctx --time 3 --csv "$CSVFILE"
read -p "Press any key to continue..." -n1 -s
sleep 1

ssh "$TARGET_USER@$TARGET_HOST" "pkill -f frida_demo" 

fi