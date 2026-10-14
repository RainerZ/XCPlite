#!/bin/bash

# Build script for frida_demo
#
#   ./build.sh [debug|release|relwithdebinfo] [gcc|clang] [local] [clean]
#
#   debug | release | relwithdebinfo   CMake build type, case insensitive (default: debug)
#   gcc | clang   Build with gcc or clang (default: CC from the environment, otherwise the CMake default compiler).
#                 Note: on macOS gcc is a clang shim.
#   local         Build against this repository's working tree instead of cloning xcplite
#   clean         Remove the build directory first
#
# Arguments can be given in any order.
# The Frida Gum devkit is downloaded by CMake on the first configure (see CMakeLists.txt, FRIDA_DEVKIT_DIR).

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

usage() {
    sed -n '3,14p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

CLEAN=false
BUILD_TYPE=Debug
CMAKE_ARGS=()

for arg in "$@"; do
    case "$(echo "$arg" | tr '[:upper:]' '[:lower:]')" in
        debug)          BUILD_TYPE=Debug ;;
        release)        BUILD_TYPE=Release ;;
        relwithdebinfo) BUILD_TYPE=RelWithDebInfo ;;
        gcc)       CC=gcc ;;
        clang)     CC=clang ;;
        local)     CMAKE_ARGS+=("-DFETCHCONTENT_SOURCE_DIR_XCPLITE=${REPO_ROOT}") ;;
        clean)     CLEAN=true ;;
        -h|--help) usage; exit 0 ;;
        *)         echo "Unknown argument: $arg"; echo ""; usage; exit 1 ;;
    esac
done

if [[ -n "${CC:-}" ]] && ! command -v "$CC" > /dev/null; then
    echo "Compiler not found in PATH: $CC"
    exit 1
fi

# Pass the compiler explicitly, CMake caches it on the first configure and ignores CC afterwards
if [[ -n "${CC:-}" ]]; then CMAKE_ARGS+=("-DCMAKE_C_COMPILER=$CC"); fi

if [[ "$CLEAN" == true ]]; then
    echo "Removing ${SCRIPT_DIR}/build"
    rm -rf "${SCRIPT_DIR}/build"
fi

echo "Compiler: ${CC:-default}"
echo "Build type: ${BUILD_TYPE}"

cd "${SCRIPT_DIR}"
cmake -B build -S . -DCMAKE_BUILD_TYPE="${BUILD_TYPE}" "${CMAKE_ARGS[@]}"
cmake --build build --parallel
