# FetchContent Example - Building xcplite from Source in Your Project

This example demonstrates how to consume **xcplite via CMake `FetchContent`**: your project names the xcplite git repository and a tag, and CMake clones and builds xcplite as part of your own build. There is no separate install step. This is the typical workflow for:

- **Version pinning in code** - the xcplite version is part of your `CMakeLists.txt`
- **Single build tree** - one `cmake -B build` builds xcplite and your application
- **CI builds** - no pre-installed xcplite on the build machine
- **Cross-compiling** - xcplite is compiled with the same toolchain and flags as your project

For the alternative, consuming a pre-built and installed xcplite with `find_package(xcplite)`, see [external_example](../external_example/).

## Project Structure

```
fetchcontent_example/
├── CMakeLists.txt          # Independent build configuration with FetchContent_Declare(xcplite ...)
├── build.sh                # Build script (optionally against the local working tree)
├── src/
│   ├── main.c              # Simple XCP example application for C
│   └── main.cpp            # Simple XCP example application for C++
└── README.md               # This file
```

## Quick Start

```bash
cd examples/fetchcontent_example
./build.sh
```

This clones xcplite from the repository and tag pinned in `CMakeLists.txt` into `build/_deps/xcplite-src/`, builds it, and builds the two example executables against it.

To build against the xcplite source tree this example is part of (no clone, picks up local changes):

```bash
./build.sh local
```

Run:

```bash
./build/fetchcontent_example
./build/fetchcontent_example_cpp
```

Connect with CANape:
- Protocol: XCP on Ethernet
- Address: localhost
- Port: 5555
- Transport: TCP

## How It Works

### CMakeLists.txt

The relevant part of the `CMakeLists.txt`:

```cmake
include(FetchContent)

FetchContent_Declare(xcplite
    GIT_REPOSITORY https://github.com/vectorgrp/XCPlite.git
    GIT_TAG        V2.2.2
    GIT_SHALLOW    TRUE
)

# xcplite build options, set before FetchContent_MakeAvailable()
set(XCPLITE_CONFIGURATION  "default")   # default | no_a2l | ptp | shm | raw
set(XCPLITE_BUILD_EXAMPLES OFF)
set(XCPLITE_BUILD_TESTS    OFF)

FetchContent_MakeAvailable(xcplite)

add_executable(fetchcontent_example src/main.c)
target_link_libraries(fetchcontent_example PRIVATE xcplite::xcplite)
```

`FetchContent_MakeAvailable` runs xcplite's own `CMakeLists.txt` as a subdirectory of this project. The `xcplite::xcplite` target is the same name the installed package exports, so the application part of the build file is identical to `external_example`. Linking it provides the include directories, the compile definitions of the selected configuration, and the dependencies (Threads, m, atomic where needed).

### What xcplite does and does not do as a subproject

When xcplite detects that it is not the top-level project:

- It does not change your `CMAKE_INSTALL_PREFIX`.
- It does not overwrite your `CMAKE_C_FLAGS_<CONFIG>` / `CMAKE_CXX_FLAGS_<CONFIG>`.
- It generates no install rules (`XCPLITE_INSTALL` defaults to `OFF`). Pass `-DXCPLITE_INSTALL=ON` if `cmake --install` of your project should install xcplite too.
- It does not enable `CMAKE_EXPORT_COMPILE_COMMANDS`.

### Selecting the xcplite configuration

`XCPLITE_CONFIGURATION` selects the library configuration (see [Build configurations](../../docs/BUILDING.md#build-configurations)). Set it as a normal variable before `FetchContent_MakeAvailable()`, as shown above, or use the cache form `set(XCPLITE_CONFIGURATION "no_a2l" CACHE STRING "" FORCE)`.

The `rtos` configuration additionally requires the consuming project to provide the FreeRTOS kernel and lwIP headers to the `xcplite` target and to define `_FREE_RTOS`; see `examples/freertos_demo/freertos_emu_demo/CMakeLists.txt` for the pattern.

### Overriding the source location

CMake's standard override lets you point FetchContent at an existing checkout instead of cloning. `./build.sh local` does exactly this:

```bash
cmake -B build -S . -DFETCHCONTENT_SOURCE_DIR_XCPLITE=/path/to/XCPlite
```

The repository and tag can also be changed on the command line without editing the file:

```bash
cmake -B build -S . -DXCPLITE_GIT_REPOSITORY=https://github.com/<fork>/XCPlite.git -DXCPLITE_GIT_TAG=main
```

## See Also

- [Building XCPlite](../../docs/BUILDING.md) - Detailed build instructions, including the FetchContent section
- [external_example](../external_example/) - Same application, consuming an installed xcplite via `find_package`
- [hello_xcp](../hello_xcp/) - Basic XCP example built from the root project
