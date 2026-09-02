#!/bin/bash

# A2L file creator for the no_a2l_demo_cpp example project

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# The script syncs the example project to the target, builds it there, runs it with XCP on Ethernet,
# downloads the ELF file to the local machine and creates an A2L file.  
# Prerequisites:
# - The target must be reachable via SSH and have rsync installed
# - The local machine must have rsync and scp installed
# - The local machine must have xcpclient installed


#======================================================================================================================
# Parameters
#======================================================================================================================

LOGFILE="$REPO_ROOT/examples/no_a2l_demo_cpp/CANape/no_a2l_demo_cpp.log"
#LOGFILE='/dev/stdout'
#LOGFILE="/dev/null"

# A2L file path on local machine
A2LFILE="$REPO_ROOT/examples/no_a2l_demo_cpp/CANape/no_a2l_demo_cpp.a2l"

# ELF file path on local machine
ELFFILE="$REPO_ROOT/examples/no_a2l_demo_cpp/CANape/no_a2l_demo_cpp.elf"

# Build type for target executable: Release, RelWithDebInfo or Debug
# RelWithDebInfo is default to demonstrate operation with with -O1 and NDEBUG
# Optimization level >= -O1 keeps variables in registers whenever possible, so local variables cannot be measured
# The most efficient solution to keep local variables measurable is to use the DaqCapture macro, another option is mto ark the variable as volatile (with the provided macro XCP_MEA
# Debug mode is the least efficient but keeps all variables and stack frames intact
BUILD_TYPE="RelWithDebInfo"
# -O0
#BUILD_TYPE="Debug"
# -O2 no debug symbols
#BUILD_TYPE="Release"

# Run a simple test calibration and measurement
TEST=true
#TEST=false


# Target connection details
TARGET_USER="parallels"
TARGET_HOST="10.211.55.4"
#TARGET_USER="rainer"
#TARGET_HOST="192.168.0.206"
TARGET_PATH="~/XCPlite-Test"
TARGET_BUILD_DIR="build-no_a2l"
TARGET_BINARY="no_a2l_demo_cpp"

# Path to xcpclient tool executable (assuming cargo installed it to ~/.cargo/bin)
XCPCLIENT="xcpclient"


#======================================================================================================================
# Sync Target, Build Application on Target, Download ELF, Start ECU, ...
#======================================================================================================================


echo "========================================================================================================"
echo "A2L file creator for the no_a2l_demo_cpp example project"
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
    --include='/examples/no_a2l_demo/***' \
    --include='/examples/no_a2l_demo_cpp/***' \
    --exclude='*' \
    "$REPO_ROOT/" "$TARGET_USER@$TARGET_HOST:$TARGET_PATH/" 1> /dev/null
if [ $? -ne 0 ]; then
    echo "❌ FAILED: Rsync with target"
    exit 1
fi


# Build on target
echo "Build executable on Target ..."
ssh "$TARGET_USER@$TARGET_HOST" "cd $TARGET_PATH && ./build.sh $BUILD_TYPE no_a2l examples" 1> /dev/null
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
# Create the  A2L from ELF file with xcpclient tool
#======================================================================================================================

echo ""
echo "========================================================================================================"
echo "Creating A2L file from XCPlite ELF file ..."
echo "========================================================================================================"
echo ""
# --log-level is program flow verbosity
# --verbose is information detail level
echo "Command: $XCPCLIENT --log-level=3 --verbose=5 --dest-addr=$TARGET_HOST --udp --offline --elf \"$ELFFILE\" --create-a2l --a2l \"$A2LFILE\""
$XCPCLIENT --log-level=3 --verbose=5 --dest-addr=$TARGET_HOST --udp --offline --elf "$ELFFILE" --create-a2l --a2l "$A2LFILE"  >> "$LOGFILE"
if [ $? -ne 0 ]; then
    echo "❌ FAILED: xcpclient returned error"
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
read -p "Press any key to continue..." -n1 -s
$XCPCLIENT --log-level=3 --dest-addr=$TARGET_HOST:5555 --udp --a2l "$A2LFILE" --list-mea . --list-cal . 

echo "========================================================================================================"
echo "Test measurement"
echo "========================================================================================================"
read -p "Press any key to continue..." -n1 -s
$XCPCLIENT --log-level=2 --dest-addr=$TARGET_HOST:5555 --udp --a2l "$A2LFILE"  --mea counter --time 1 --verbose 2


ssh "$TARGET_USER@$TARGET_HOST" "pkill -f no_a2l_demo_cpp" 

fi