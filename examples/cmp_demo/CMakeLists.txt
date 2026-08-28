cmake_minimum_required(VERSION 3.10)

project(cmp_demo VERSION 1.0.0 LANGUAGES C)

# Standalone project, like examples/external_example: it is NOT built from the xcplite root
# CMakeLists, it consumes an INSTALLED xcplite. See README.md.
#
# xcplite must be installed from the "raw" configuration, which enables the raw Ethernet
# transport (OPTION_ENABLE_UDP_RAW):
#
#   cd <xcplite>
#   cmake -B build-raw -S . -DXCPLITE_CONFIGURATION=raw -DCMAKE_INSTALL_PREFIX=<install>
#   cmake --build build-raw --target install
#
# then point this project at it:
#
#   cmake -B build -S . -Dxcplite_DIR=<install>/lib/cmake/xcplite
#   cmake --build build

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS OFF)

if(NOT CMAKE_SYSTEM_NAME STREQUAL "Linux")
    message(FATAL_ERROR
        "cmp_demo is Linux only: its Ethernet HAL backend uses AF_PACKET.\n"
        "On other platforms the library additionally has to be built with "
        "OPTION_UDP_RAW_HAL_EXTERNAL, because it has no built in backend there.")
endif()

find_package(xcplite REQUIRED)

# The application supplies the Ethernet HAL backend itself.
#
# socket_raw_hal_cmp.c defines all six eth_hal_* functions that libxcplite's socket_raw.c
# needs. Because libxcplite is a STATIC library, the linker only pulls an archive member to
# resolve an undefined symbol - and these are already defined here, so the built in AF_PACKET
# backend (socket_raw_hal_linux.o) is never pulled in and there is no symbol clash.
# This would NOT hold for a shared libxcplite, which would resolve its own symbols internally.
add_executable(cmp_demo
    src/main.c
    src/socket_raw_hal_cmp.c
    src/cmp.c
)

target_link_libraries(cmp_demo PRIVATE xcplite::xcplite)

if(CMAKE_C_COMPILER_ID MATCHES "GNU|Clang")
    target_compile_options(cmp_demo PRIVATE -Wall -Wextra -pedantic)
endif()

message(STATUS "Building cmp_demo")
message(STATUS "  xcplite version: ${xcplite_VERSION}")
message(STATUS "  Ethernet HAL   : supplied by this project (src/socket_raw_hal_cmp.c)")
message(STATUS "  CMP envelope   : see src/cmp.c - currently a pass through")
