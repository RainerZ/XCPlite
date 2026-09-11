# fetchcontent_example - consuming xcplite from source via CMake FetchContent
#
# This standalone project does not need an installed xcplite. CMake clones the
# xcplite repository at configure time into build/_deps/xcplite-src, adds it as a
# subdirectory and builds it together with this example.
#
# Requires: CMake >= 3.14, git, network access on the first configure.

cmake_minimum_required(VERSION 3.14)

project(fetchcontent_example VERSION 1.0.0 LANGUAGES C CXX)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS OFF)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

include(FetchContent)

# ---------------------------------------------------------------------------
# Where to fetch xcplite from
#
# Pin a released tag for reproducible builds. 
# For local development against a checked-out xcplite source tree, bypass the git clone with
#   cmake -B build -S . -DFETCHCONTENT_SOURCE_DIR_XCPLITE=../..
# ---------------------------------------------------------------------------
set(XCPLITE_GIT_REPOSITORY "https://github.com/RainerZ/XCPlite.git" CACHE STRING "xcplite git repository")
set(XCPLITE_GIT_TAG        "V2.2.2"                                 CACHE STRING "xcplite git tag, branch or commit")

FetchContent_Declare(xcplite
    GIT_REPOSITORY ${XCPLITE_GIT_REPOSITORY}
    GIT_TAG        ${XCPLITE_GIT_TAG}
    GIT_SHALLOW    TRUE
)

# ---------------------------------------------------------------------------
# xcplite build options
#
# These must be set before FetchContent_MakeAvailable(). Plain set() is enough:
# xcplite honors normal variables from the parent project for its option()s
# (policy CMP0077) and for XCPLITE_CONFIGURATION. The
# set(... CACHE <type> "" FORCE) form works as well.
#
# XCPLITE_INSTALL is not set here: it defaults to OFF when xcplite is not the
# top-level project, so 'cmake --install build' installs only this example.
# ---------------------------------------------------------------------------
set(XCPLITE_CONFIGURATION  "default")   # default | no_a2l | ptp | shm | raw
set(XCPLITE_BUILD_EXAMPLES OFF)
set(XCPLITE_BUILD_TESTS    OFF)
set(XCPLITE_BUILD_TOOLS    OFF)

FetchContent_MakeAvailable(xcplite)

# ---------------------------------------------------------------------------
# Example executables
#
# xcplite::xcplite is the same target name the installed package exports, so
# this part is identical to examples/external_example. Linking it provides the
# include directories, compile definitions and dependencies (Threads, m, atomic).
# ---------------------------------------------------------------------------
add_executable(fetchcontent_example src/main.c)
target_link_libraries(fetchcontent_example PRIVATE xcplite::xcplite)

add_executable(fetchcontent_example_cpp src/main.cpp)
target_link_libraries(fetchcontent_example_cpp PRIVATE xcplite::xcplite)

if(CMAKE_C_COMPILER_ID MATCHES "GNU|Clang")
    target_compile_options(fetchcontent_example PRIVATE -Wall -Wextra -pedantic)
    target_compile_options(fetchcontent_example_cpp PRIVATE -Wall -Wextra -pedantic)
elseif(CMAKE_C_COMPILER_ID STREQUAL "MSVC")
    target_compile_options(fetchcontent_example PRIVATE /W3)
    target_compile_options(fetchcontent_example_cpp PRIVATE /W3)
endif()

message(STATUS "Building fetchcontent_example (C and C++)")
message(STATUS "  xcplite source: ${xcplite_SOURCE_DIR}")
message(STATUS "  xcplite configuration: ${XCPLITE_CONFIGURATION}")
