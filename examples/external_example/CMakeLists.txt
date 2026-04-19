cmake_minimum_required(VERSION 3.10)

project(external_example VERSION 1.0.0 LANGUAGES C CXX)

# Set C standard
set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)
set(CMAKE_C_EXTENSIONS OFF)

# Set C++ standard
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# Find xcplite package
# Three ways to find xcplite (in order of precedence):
#
# 1. Via xcplite_DIR pointing to the cmake config directory:
#    cmake -Dxcplite_DIR=../../build/install/lib/cmake/xcplite ..
#
# 2. Via CMAKE_PREFIX_PATH pointing to the install root:
#    cmake -DCMAKE_PREFIX_PATH=../../build/install ..
#
# 3. System-wide installation (if installed to /usr/local or similar):
#    cmake ..

find_package(xcplite REQUIRED)

# Link against xcplite
# The xcplite::xcplite target automatically provides:
# - Include directories
# - Library dependencies (Threads, math, atomic if needed)
# - Compile definitions

# Create C executable
add_executable(external_example src/main.c)
target_link_libraries(external_example PRIVATE xcplite::xcplite)

# Create C++ executable
add_executable(external_example_cpp src/main.cpp)
target_link_libraries(external_example_cpp PRIVATE xcplite::xcplite)

# Optional: Add compiler warnings
if(CMAKE_C_COMPILER_ID MATCHES "GNU|Clang")
    target_compile_options(external_example PRIVATE -Wall -Wextra -pedantic)
    target_compile_options(external_example_cpp PRIVATE -Wall -Wextra -pedantic)
elseif(CMAKE_C_COMPILER_ID STREQUAL "MSVC")
    target_compile_options(external_example PRIVATE /W3)
    target_compile_options(external_example_cpp PRIVATE /W3)
endif()

# Print configuration info
message(STATUS "Building external_example (C and C++)")
message(STATUS "  xcplite version: ${xcplite_VERSION}")
message(STATUS "  xcplite libraries: ${xcplite_LIBRARIES}")
message(STATUS "  xcplite include dirs: ${xcplite_INCLUDE_DIRS}")
