#!/bin/bash

# Build script for fetchcontent_example
#
#   ./build.sh          fetch xcplite from the git repository/tag pinned in CMakeLists.txt
#   ./build.sh local    build against this repository's working tree instead (no clone)
#   ./build.sh clean    remove the build directory first (can be combined with local)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

EXTRA_ARGS=()
for arg in "$@"; do
    case "$arg" in
        clean) rm -rf "${SCRIPT_DIR}/build" ;;
        local) EXTRA_ARGS+=("-DFETCHCONTENT_SOURCE_DIR_XCPLITE=${REPO_ROOT}") ;;
        *) echo "Unknown argument: $arg"; exit 1 ;;
    esac
done

cd "${SCRIPT_DIR}"
cmake -B build -S . -DCMAKE_BUILD_TYPE=Debug "${EXTRA_ARGS[@]}"
cmake --build build
