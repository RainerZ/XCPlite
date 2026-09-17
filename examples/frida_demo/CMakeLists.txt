# frida_demo - XCPlite measurement of a function hooked with Frida Gum
#
# Standalone project: xcplite is built from source via CMake FetchContent (like examples/fetchcontent_example),
# the Frida Gum instrumentation engine comes as a prebuilt static "devkit" tarball from the Frida GitHub release page.
#
# Requires: CMake >= 3.14, git, network access on the first configure (xcplite clone and devkit download).

cmake_minimum_required(VERSION 3.14)

project(frida_demo VERSION 1.0.0 LANGUAGES C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS OFF)

include(FetchContent)

# ---------------------------------------------------------------------------
# xcplite
# Pin a released tag for reproducible builds.
# For local development against a checked-out xcplite source tree, bypass the git clone with
#   cmake -B build -S . -DFETCHCONTENT_SOURCE_DIR_XCPLITE=../..
# (./build.sh local does this)
# ---------------------------------------------------------------------------
set(XCPLITE_GIT_REPOSITORY "https://github.com/vectorgrp/XCPlite" CACHE STRING "xcplite git repository")
set(XCPLITE_GIT_TAG        "V2.2.2"                               CACHE STRING "xcplite git tag, branch or commit")

FetchContent_Declare(xcplite
    GIT_REPOSITORY ${XCPLITE_GIT_REPOSITORY}
    GIT_TAG        ${XCPLITE_GIT_TAG}
    GIT_SHALLOW    TRUE
)

# xcplite build options, must be set before FetchContent_MakeAvailable()
# The default configuration is used unchanged: the demo needs the runtime A2L generator and calibration segments.
set(XCPLITE_CONFIGURATION  "default")
set(XCPLITE_BUILD_EXAMPLES OFF)
set(XCPLITE_BUILD_TESTS    OFF)
set(XCPLITE_BUILD_TOOLS    OFF)

FetchContent_MakeAvailable(xcplite)

# ---------------------------------------------------------------------------
# Frida Gum devkit
#
# A devkit is a tarball with three files: frida-gum.h (umbrella header, GLib included),
# libfrida-gum.a (static library, all dependencies bundled) and frida-gum-example.c.
# It is downloaded for the host platform from https://github.com/frida/frida/releases.
#
# To use an already extracted devkit (offline build, other version), point FRIDA_DEVKIT_DIR at its directory:
#   cmake -B build -S . -DFRIDA_DEVKIT_DIR=/path/to/devkit
# ---------------------------------------------------------------------------
set(FRIDA_VERSION    "17.18.0" CACHE STRING "Frida release version of the Gum devkit")
set(FRIDA_DEVKIT_DIR ""        CACHE PATH   "Directory of an extracted frida-gum devkit (empty: download)")

if(FRIDA_DEVKIT_DIR STREQUAL "")
    # Map the CMake platform names to the Frida devkit names
    if(CMAKE_SYSTEM_NAME STREQUAL "Darwin")
        set(FRIDA_OS "macos")
    elseif(CMAKE_SYSTEM_NAME STREQUAL "Linux")
        set(FRIDA_OS "linux")
    else()
        message(FATAL_ERROR "frida_demo: no Frida Gum devkit mapping for ${CMAKE_SYSTEM_NAME}, set FRIDA_DEVKIT_DIR")
    endif()
    if(CMAKE_SYSTEM_PROCESSOR MATCHES "^(arm64|aarch64)$")
        set(FRIDA_ARCH "arm64")
    elseif(CMAKE_SYSTEM_PROCESSOR MATCHES "^(x86_64|AMD64)$")
        set(FRIDA_ARCH "x86_64")
    elseif(CMAKE_SYSTEM_PROCESSOR MATCHES "^armv7")
        set(FRIDA_ARCH "armhf")
    else()
        message(FATAL_ERROR "frida_demo: no Frida Gum devkit mapping for ${CMAKE_SYSTEM_PROCESSOR}, set FRIDA_DEVKIT_DIR")
    endif()

    set(FRIDA_DEVKIT_NAME "frida-gum-devkit-${FRIDA_VERSION}-${FRIDA_OS}-${FRIDA_ARCH}")
    if(CMAKE_VERSION VERSION_GREATER_EQUAL "3.24")
        set(FRIDA_EXTRACT_TIMESTAMP DOWNLOAD_EXTRACT_TIMESTAMP TRUE)
    endif()
    FetchContent_Declare(frida_gum
        URL "https://github.com/frida/frida/releases/download/${FRIDA_VERSION}/${FRIDA_DEVKIT_NAME}.tar.xz"
        ${FRIDA_EXTRACT_TIMESTAMP}
    )
    # The tarball has no CMakeLists.txt, FetchContent_MakeAvailable only downloads and extracts it
    FetchContent_MakeAvailable(frida_gum)
    set(FRIDA_DEVKIT_DIR "${frida_gum_SOURCE_DIR}")
endif()

if(NOT EXISTS "${FRIDA_DEVKIT_DIR}/frida-gum.h" OR NOT EXISTS "${FRIDA_DEVKIT_DIR}/libfrida-gum.a")
    message(FATAL_ERROR "frida_demo: frida-gum.h or libfrida-gum.a not found in ${FRIDA_DEVKIT_DIR}")
endif()

add_library(frida_gum STATIC IMPORTED GLOBAL)
set_target_properties(frida_gum PROPERTIES
    IMPORTED_LOCATION             "${FRIDA_DEVKIT_DIR}/libfrida-gum.a"
    INTERFACE_INCLUDE_DIRECTORIES "${FRIDA_DEVKIT_DIR}"
)
# System libraries the static devkit needs, taken from the compile command in frida-gum-example.c of the devkit
if(CMAKE_SYSTEM_NAME STREQUAL "Darwin")
    target_link_libraries(frida_gum INTERFACE resolv m dl)
else()
    target_link_libraries(frida_gum INTERFACE dl resolv m pthread)
endif()

# ---------------------------------------------------------------------------
# Example executable
# ---------------------------------------------------------------------------
add_executable(frida_demo src/main.c)
target_link_libraries(frida_demo PRIVATE xcplite::xcplite frida_gum)
# Export the symbols of the executable (-rdynamic), so dladdr() can resolve the malloc() callers to function names
set_target_properties(frida_demo PROPERTIES ENABLE_EXPORTS ON)

if(CMAKE_C_COMPILER_ID MATCHES "GNU|Clang")
    target_compile_options(frida_demo PRIVATE -Wall -Wextra)
    # Optimized build with debug info: -O1 keeps the local variables of the hook helper function addressable in memory,
    # they are measured stack frame relative (see examples/fetchcontent_example/CMakeLists.txt for the details)
    target_compile_options(frida_demo PRIVATE $<$<CONFIG:RelWithDebInfo>:-O1>)
endif()

message(STATUS "Building frida_demo")
message(STATUS "  xcplite source: ${xcplite_SOURCE_DIR}")
message(STATUS "  Frida Gum devkit: ${FRIDA_DEVKIT_DIR}")
message(STATUS "  build type: ${CMAKE_BUILD_TYPE}")
