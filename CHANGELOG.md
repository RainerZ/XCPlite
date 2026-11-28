# Changelog

All notable changes to XCPlite are documented in this file.

The changes of the `xcpclient` tool are documented in [tools/xcpclient/CHANGELOG.md](tools/xcpclient/CHANGELOG.md), this file only notes the xcpclient version of each release.


## [V2.4.5]

- xcpclient V4.3.0: measurement of arrays, MCP server mode, see [tools/xcpclient/CHANGELOG.md](tools/xcpclient/CHANGELOG.md).


## [V2.4.3]

- Breaking change:
    * Functions in platform.h `clockGetLast`, `clockGetMonotonicNsLast`, `clockGetMonotonicUsLast`, `clockGetRealtimeNsLast` and `clockGetRealtimeUsLast` have been removed to fixed a data race in the POSIX clock functions (Linux, macOS, QNX), so the timestamps of one thread could go backwards by several µs.
    * New unit test `clock_race_test`: several threads call the clock concurrently for 2 s and check that the values of each thread never go backwards.

- Breaking change: (impact only on use cases with run-time A2L generation using event instances)
    * Support for anonymous multi-threaded event instances (`<name>_<index>`) has been removed. 
    * The API functions `XcpCreateEventInstance`, `DaqCreateEventInstance`, `DaqGetEventInstanceId` and `XcpGetEventIndex` do not exist anymore. Create an event with dynamic name (`DaqCreateEvent_s`) instead and manage your event instances by name or use the new named event instances (event context).
    * Binary persistence file version 0x0206: the context list (name and owning application, in context id order) follows the header, an event references its context by context id. On load, the contexts are restored with their saved ids before the events, so context ids stay stable across restarts (they are visible in the A2L file, `conv.contexts`, and in the span link measurements), also for contexts without events and when events are created in a different order than their contexts. Files of version 0x0205 are still loaded, a file which contains the old anonymous event instances is rejected. `tools/bintool` shows an event of a context as `<context>.<event>`.

Additions for contexts, spans and span links:

- New `thread_context_demo`: contexts, spans and span links inspired by OpenTelemetry, measured with XCP and shown in Perfetto. The old experimental event context in `multi_thread_demo` has been removed, the demo now only shows the same event triggered by several worker threads, like the Rust xcp-lite `rayon_demo`.
- New C++ API in the new header `xcplib_context.hpp` (included by `xcplib.hpp`, option `OPTION_DAQ_CONTEXT_LIST`, default and shm configuration only): `xcp::Context`, `xcp::Scope`, `DaqContext`, `DaqSpan`, `DaqEventCtxVar` and span links (`xcp::CaptureSpanLink`, `xcp::LinkSpan`). The same code running for several class instances or threads gets separate measurements per context, an event of a context is named `<context>.<event>`. The context list is implemented in the C core (`src/ctx.c`).
- Span names are registered at link time (`OPTION_DAQ_SPAN_SECTION_REGISTRATION`, default), or at runtime where linker sections are not available (Windows, SHM mode).
- New option `OPTION_DAQ_SPAN_THREAD` (not defined by default): records the thread index with each span, for several threads in one context. Experimental, might be removed again.

Tools:

- xcpclient V4.2.0: Perfetto trace export of spans and span links, MDF4 and MCAP measurement files, see [tools/xcpclient/CHANGELOG.md](tools/xcpclient/CHANGELOG.md).
- `tools/bintool` shows the context list and the memory segment numbers.

Bugfixes in A2L and ELF file upload (#151, contributed by pamirmundt):

- An empty or unreadable A2L or ELF file no longer aborts the XCP server with an assertion in debug builds, `GET_ID` returns size 0 (file not available).
- A new `GET_ID` upload request restarts an unfinished upload instead of asserting.
- `UPLOAD` reads from the MTA offset instead of the current file position. `SET_MTA` with address extension `XCP_ADDR_EXT_FILE` (0xFD) is accepted, so retries and out of order reads work, see `docs/TECHNICAL.md`. An out of range read is refused without ending the upload, a short read has its own error message.
- The upload file is closed by the new function `ApplXcpCloseFile()` on the next upload `GET_ID`, an accepted `CONNECT`, `DISCONNECT`, `XcpDeinit()` or an I/O error, instead of after the last block.
- `A2L_MODE_WRITE_ONCE`: an empty cached A2L file (for example left by a crashed run) is generated again instead of being provided for upload.
- New unit tests `file_upload_test` and `a2l_cache_test` (CTest `a2l_cache_empty`, `a2l_cache_nonempty`).

Changed core files for events with named context:
* a2l_writer.c: handle event context. 
* persistence.c: write and load the context list, an event references its context by id instead of the former event instance index
* xcplite.c: added context list, tXcpContext, tXcpContextList, XcpCreateEventWithIndex; removed `XcpCreateEventInstance`, `DaqCreateEventInstance`, `DaqGetEventInstanceId` and `XcpGetEventIndex`



## [V2.3.5]

- xcpclient V4.0.1 (version not increased): the registry crate `xcp_registry` is used from crates.io, see [tools/xcpclient/CHANGELOG.md](tools/xcpclient/CHANGELOG.md).


## [V2.3.4]

- Fixed potential integer overflows in calibration memory size calculation
- Fixed queue64f and queue64v memory size checks
- Fixed socket_raw_test and daq_config_test, added a new persistence_test


## [V2.3.2]

- Bugfixes: 
    - `DaqTriggerEventExt` and the C variant of `DaqEventVar` in `xcplib.h` initialized their event id marker with `XCP_UNDEFINED_EVENT_ID` instead of the link-time id `XCP_EVENT_SECTION_GET_LINKTIME_ID`. Affected only clang on Linux.
    - Example point_cloud_demo, duplicate event in section registration mode,

- New macros `DaqDeclareEvent` / `DaqDeclareEventExt` in `xcplib.h` (section registration only): declare an event at file scope. Pure declarations without a statement, usable with any compiler.

- New configuration option `OPTION_CAL_RCU_REFCOUNT` provides an alternative calibration RCU algorithm. It is disabled in the any configuration. Enabling it, causes a breaking change in the C API, the C++ API stays compatible. The alternative version makes some compromises to get calibration changes always visibility in the the first look after a publish. Recommended only for slow reader configuration use-cases. Not recommended and not tested on micro-controller platforms. Lock-free, but not wait-free.

- New configuration option `OPTION_SECTION_REGISTRATION` to explicitly enable registration of events and calibration segments at link time in the sections `xcp_evts` and `xcp_cals`, as used by the offline A2L generator `xcpclient --elf`. Disabled in the default configuration. It was enabled before in all configurations, which caused some issues. Enabled in the configurations `no_a2l` and `rtos` (mandatory without `OPTION_DAQ_EVENT_LIST`). The `default` configuration now creates events at runtime again in creation order, and the trigger macros look up the event by name on their first execution, so the event descriptor does not need to be visible at the trigger site. 

- `CalSegDecl`, `CalBlkDecl` and the C++ `CalSegDeclRef`/`CalSegDecl` are now a compile-time error without `OPTION_SECTION_REGISTRATION`. They only emit a section descriptor, the segment is lazily created by the section scan in `XcpInit()`, so without section registration the segment was never created and calibration silently had no effect. Use `CalSegCreate` after `XcpInit()`.

- Section registration with `OPTION_DAQ_EVENT_LIST`: `XcpInit()` reports an event which is created at more than one place (`DaqCreateEvent`, `DaqCreateAndTriggerEvent`, ...) as an error. The event is created only once, but the trigger macros use the position of their descriptor in the section as event id, so the triggers of all following events silently used wrong event ids.

- freertos_demo: `xcp_demo.c` supports builds with and without `OPTION_SECTION_REGISTRATION`, to show the consequences of doing so. The linker section method is still the strongly recommended way for rtos targets. Without it, the events and the calibration segment `parameters` are created in a fixed order in `startXcpServer()` after `XcpInit()`, before the tasks start, so the event ids and segment numbers are the same after each restart. The event creation macros in the tasks find the events by name.

- silkit_demo: FetchContent support

- `XcpCreateEpk`: the EPK string variable is renamed from `gXcpEpkString` to `xcp_epk__`. Its address is stored in a volatile pointer, so the string is no longer removed by the compiler in optimized builds, or by the linker with `--gc-sections`, when there is no `xcp_epk` section (no `OPTION_SECTION_REGISTRATION`).

- xcpclient V4.0.1: offline A2L generation without `OPTION_SECTION_REGISTRATION`, EPK handling, `--fix-a2l` rewrites the A2L file, see [tools/xcpclient/CHANGELOG.md](tools/xcpclient/CHANGELOG.md).



## [V2.2.3]

- Improved CalSeg RCU, see CAL_RCU.md
- Fixed CalSeg RCU with deactivated XCP (XcpInit(XCP_MODE_DEACTIVATE))
- Fixed Ethernet command framing and socket error handling (#135)
- Test: 
    - Add passive_test_c/cpp verifying XCP_MODE_DEACTIVATE behavior. 
    - Register self-verifying tests with CTest (labels + timeouts).
    - Unify the CHECK macro for self-verifying tests


## [V2.2.2] 

- New macros `DaqTriggerEventCapture` and `DaqCreateAndTriggerEventCapture` in `xcplib.h` to measure local variables which are not addressable via the frame pointer or which the user does not want to spill.

- Bugfixes and improvements in the XCP protocol layer (see #126). 
    - Associating 2 DAQ lists to the same event now works correctly.
    - Memory overflow detection and handling of stale state on DAQ configuration errors and retries.

- CMake: xcplite can now be consumed via `FetchContent`/`add_subdirectory` (as suggested by vectorgrp/XCPlite#132):
    - New alias target `xcplite::xcplite`, the same name the installed package exports.
    - Install prefix default and per-build-type compiler flags are only applied when xcplite is the top-level project.
    - New option `XCPLITE_INSTALL` (default `ON` in top-level, `OFF` as subproject) controls the install rules.
    - New standalone `examples/fetchcontent_example`, documented in `docs/BUILDING.md`.
    - New CMake variable `XCPLITE_CFG_OVERRIDE`: path to an application specific configuration override header, applied on top of `xcplib_cfg.h` (configuration `default` only). Demonstrated by `examples/fetchcontent_example/config/xcplib_app_cfg.h`.

- Split the large `platform.c/.h` into `platform.c/.h` (threads, mutex, clock, sleep, memory, atomics) and `sockets.c/.h` (socket abstraction for all platforms).

- IPv4 fragmentation is now prevented on the socket transport: `socketOpen` sets the DF bit on UDP sockets. Oversized segments now fail with `EMSGSIZE`. Default `OPTION_MTU` reduced to prevent immediate errors, as fragmentation is now detected.

- Minor fixes:    
    - `create_thread()` on Windows and Linux now returns 0 on success.
    - Fixed `PLATFORM_32_BIT` typo in `xcplib_cfg.h`, which prevented the automatic selection of `OPTION_QUEUE_32` on 32 bit platforms.
    - Fixed missing `#include <errno.h>` in `shm.c`, which broke the `shm` configuration build on macOS.
    - Window build workaround to disable section based event pre-registration.
    - Fixed older XCP protocol layer versions 0x0100 to 0x0102.
    - CMake: `-fno-omit-frame-pointer` and `-fno-optimize-sibling-calls` removed from the RelWithDebInfo flags. The clang frame pointer is forced by `__builtin_frame_address()` in exactly the functions which trigger events (GCC needs none, it locates locals relative to the CFA).
    - Trigger macros are protected against tail call optimizations. Tail calls are prevented by `XCP_NO_TAIL_CALL()`.

- Improved and fixed freertos_stm32_demo:
    - Tested on real hardware.
    - CANape project updated.
    - Missing xcp_meta section added to ld.
    - The lwIP socket path now reports a segment that does not fit the link MTU. lwIP sets no DF option.

- `xcp_get_frame_addr()` in `xcplib.h` is `__builtin_dwarf_cfa()` under GCC (CFA on every architecture) and `__builtin_frame_address(0)` under clang (frame pointer register). Stack relative addresses of A2L files created by xcpclient V4.x need this version.

- xcpclient V4.0.0, see [tools/xcpclient/CHANGELOG.md](tools/xcpclient/CHANGELOG.md).

- New raw Ethernet transport `OPTION_ENABLE_UDP_RAW`: XCP on UDP/IPv4 implemented inside xcplib on top of a thin raw Ethernet HAL, for targets without a TCP/IP stack. See `docs/SOCKET_RAW.md`.
    - New build configuration `raw` (`src/xcplib_raw_cfg.h`, `build-raw/`) with the new example `udp_raw_demo` (for testing on Linux only).
    - The `rtos` configuration keeps using the lwIP socket API - the raw transport is a separate configuration, not an override yet.
    - Optional zero copy transmit for the raw Ethernet transport (`OPTION_UDP_RAW_ZERO_COPY`, on by default). Command responses keep the copying path.
    - New `OPTION_UDP_RAW_HAL_EXTERNAL`: an application can supply its own raw Ethernet HAL from outside the library.
    - Default OPTION_MTU in the `raw` configuration set to **1420**, below the 1500 of a standard Ethernet link, to leave headroom for a HAL backend like CMP which encapsulates the frame before putting it on the wire. 

- New experimental example `cmp_demo`: an emulated **ASAM CMP** (Capture Module Protocol V1.1) capture module carrying XCP, for testing XCP tools which communicate through capture modules. Nothing CMP specific is in `libxcplite` — the whole protocol lives in the example, behind the new `eth_hal_*` interface.

    
## [V2.1.14]

- Support static allocation and improve FreeRTOS robustness (#124):
    - Support static allocation for FreeRTOS tasks and mutexes.
    - Use static storage when configSUPPORT_STATIC_ALLOCATION is enabled.
    - Preserve dynamic allocation and recursive mutex support.
    - Update FreeRTOS documentation and remove obsolete queue TODOs.
    - Fix FreeRTOS delay rounding and overflow.
    - Avoid unaligned IPv4 address access in FreeRTOS/lwIP.
    - Fix synchronization of FreeRTOS queue state.
    - Correct FreeRTOS queue and lwIP documentation
    - Fix FreeRTOS emulator clock test build.


## [V2.1.12]

- xcpclient V3.0.10: improved offline A2L generation for global and static variables and C++ namespaces, see [tools/xcpclient/CHANGELOG.md](tools/xcpclient/CHANGELOG.md).
- no_a2l_demo, no_a2l_demo_cpp: more demo cases, no_a2l_demo_cpp demonstrates types and variables with the same name in different namespaces.
- Documentation: new `docs/OFFLINE_A2L.md` for the offline A2L generation with xcpclient (workflow, naming rules, supported types, diagnostics), `docs/TECHNICAL.md` keeps the library topics and the instrumentation marker contract, xcpclient and no_a2l_demo READMEs updated
    


## [V2.1.11]

- FreeRTOS related changes:
    - Allow custom memory attributes for queue32m.
    - Avoid mutex allocation when queue32m uses critical sections.
    - Support optional recursive FreeRTOS mutexes.
    - Fix typos in FreeRTOS documentation and comments.
    - Fix UDP segment size calculation from MTU.
    - Make FreeRTOS queue segment count configurable.

- Build cleanup: `shm.h` and `persistence.h` are only included where the corresponding feature is enabled, reducing overhead for minimal embedded source subsets
- xcpclient V3.0.9: EPK validation, C++ class types, see [tools/xcpclient/CHANGELOG.md](tools/xcpclient/CHANGELOG.md).
- Documentation improvements.
- Fixed metadata macros XCP_LIMITS (missing ;) and XCP_READ_WRITE ([]).
- Fixed link type_detection_test against xcplite for XCPLITE_CONFIGURATION define.


## [V2.1.9]

- XCP DAQ event info (#define XCP_ENABLE_DAQ_EVENT_INFO) support for disabled OPTION_DAQ_EVENT_LIST with section registered events.
- New example `no_a2l_demo_cpp` demonstrating build-time A2L generation for C++.
- A2L generator declarations conditionally removed from public API headers (`inc/xcplib.h`, `inc/xcplib.hpp`) for non-A2L configurations.
- C++ namespace alias `xcplib` removed (breaking change for code using `xcplib::` instead of `xcplite::`).
- `XcpSetA2lName` and `XcpGetA2lName` are now independent from enabling the A2L generator.
- Hardcoded maximum calibration segment name length limited by alignment requirements.
- Fixed `cpp_demo` shutdown issue.
- Windows build fixes.
- Removed code definition of `_GNU_SOURCE`, made it public in CMakeLists.txt.

- New `XCP_READ_WRITE` metadata macro.

- xcpclient V3.0.9 (released with V2.1.11): enum conversion, new options `--elf_skip_no_metadata` and `--config`, see [tools/xcpclient/CHANGELOG.md](tools/xcpclient/CHANGELOG.md).


## [V2.1.6] 

- Critical bugfix in ´XcpEventExt_´, undefined behaviour when DAQ is not running.
- Optimizations for microcontroller/RTOS builds, optimized locking (critical section instead of mutex) and static allocated queue (new ´queue32m.c´).
- Link-time, deterministic event id assignment (´tXcpEventDescriptor´ in section ´xcp_evts´).
- Implemented ´XCP_LIMIT´, XCP_COMMENT and ´XCP_UNIT´ metadata definition to support A2L generation in RTOS use cases (new linker section ´xcp_meta´). See example no_a2l_demo for details.
- xcpclient V3.0.1, see [tools/xcpclient/CHANGELOG.md](tools/xcpclient/CHANGELOG.md).
- Made OPTION_DAQ_EVENT_LIST an configuration override and removed special configuration handling with XCPLIB_FOR_RUST.
- Enabled 1ms event ´async´ (event id 0) as default event for global variables in RTOS builds.
- ´XcpGetEcuEpk´ renamed to ´XcpGetLocalEpk´ for clarity in SHM mode.
- silkit_demo updated.
- ´DaqCreateEventExt´ fixed.
- Asserts removed from the DAQ trigger macros.


## [V2.1.5]

- freertos_stm32_demo with FreeRTOS and lwip for STM32 microcontrollers
- More flexibility how to provide the DAQ timebase
- Single source demo code for all FreeRTOS examples
- xcpclient V2.1.3 (no further changes), see [tools/xcpclient/CHANGELOG.md](tools/xcpclient/CHANGELOG.md)


## [V2.1.3]

- cmake build system revisited, build configurations and build options
- Improved convenience build script
- README files for all example applications
- xcpclient (version not increased, 2.1.0): only uses the registry (xcp-lite xcp_registry crate), not the full Rust xcp-lite stack, see [tools/xcpclient/CHANGELOG.md](tools/xcpclient/CHANGELOG.md)


## [V2.1.0]

- esp32_freertos_demo with FreeRTOS and lwip
- freertos_demo using FreeRTOS POSIX emulator port
- Heap allocation removed from socket platform abstraction layer
- Improved compatibility for 32 bit microcontroller RTOS operating systems (like freeRTOS, zephyr, ThreadX etc.)
- Improved support for offline A2L creation from ELF files, file system dependency is now optional 
- XCP event and segment descriptor memory sections to preregister events and calibration segments/block in XcpInit, for deterministic event numbers without .BIN file
- xcpclient V2.1.0: the A2L generator can create an A2L file template with XCP events and segments from the ELF file only, by inspecting the event and segment descriptor memory sections, see [tools/xcpclient/CHANGELOG.md](tools/xcpclient/CHANGELOG.md)
- Optional custom GET_ID to download the ELF file instead of the A2L file
- Addressing scheme XCPLITE__AXSDD, memory access via callbacks, no calibration segment management 


## [V2.0.4]

- gcc compatibility issue fixed


## [V2.0.3]

- Bug fixes and improvements, see commit history for details
- `silkit_demo` with optional separate XCP server participant


## [V2.0.2]

- New demo `silkit_demo` demonstrating XCP instrumentation of a sil-kit participant
- Minor bug fixes and improvements
- Application specific memory addressing mode fixed, demo in example c_demo


## [V2.0.1]

- General refactoring and code cleanup, various minor code improvements and optimizations
- Renamed to `libxcplite` with external package name `xcplite`
- DAQ performance optimization, lock-less transmit queue for vectored IO
- New function XcpCreateCalBlk to create calibration blocks without A2L memory segments
- Simplified build script and CMake configuration, build script option to install libxcplite 
- Changed default encoding for dynamic addressing to 10 Bit event and 22 Bit signed offset, allowing for up to 4GB addressable range per event with max 1024 events
- New variadic C++ macro/template to create A2L typedefs and their components in one call with automatic type deduction (see hello_xcp_cpp example)
- New API functions to specify input quantities for axis
- New convenience functions in platform.c: clockGetMonotonicNs() and clockGetRealtimeNs()
- New demo `ptp4l_demo` demonstrating minimum requirement to achieve XCP PTP support
- Refactoring in platform.c socket abstraction: socket error handling, OPTION_SOCKET_HW_TIMESTAMPS, new function socketSetTimeout, improved debug logging for socketRecvFrom to help diagnose interface index issues on Linux, SOCKET keeps track of configured interface
- Log level 6 for very verbose debug logging
- Define GNU_SOURCE in cmakelists.txt
- xcpclient tool (V2.0.0) in Rust for testing and for offline A2L generation, see [tools/xcpclient/CHANGELOG.md](tools/xcpclient/CHANGELOG.md)
- Improved Windows performance


### Breaking changes

- The signature of XpcInit has changed to
```c
void XcpInit(const char *name, const char *epk, uint8_t mode);
```
- The return value contract of `socketRecv` and `socketRecvFrom` has changed. Only code that uses these functions directly (i.e. code that includes `socket.h` is affected)

### Experimental

- Multi application mode (OPTION_SHM_MODE). See `SHM.md` for details.  


## [V1.2.1]


## [V1.1.0]

### Added
- New variadic data acquisition C macro or C++ macro/template  (`DaqEventVar`)
- Support for enabling and disabling individual DAQ events at runtime (`DaqEventEnable`, `DaqEventDisable`)
- New demo `point_cloud_demo` demonstrating how to visualize arrays of objects in the CANape 3D scene window

### Changed
- Removed .out extension from the binaries on Linux and macOS builds

### Improvements
- Improved thread safety of event creation to avoid unnecessary THREAD_LOCAL state
- Optimized A2L generation macros, inlined address calculations moved into A2L creator functions
- Auto addressing mode now supports absolute and relative addressing
- More idiomatic C++ code in examples

### Fixed
- Bug in transmit thread queue handling causing too many packets sent
- Fixed cpp_demo assertion

### Minor Changes
- A2L transport layer section removed when bound to ANY and IP address auto-detection is off
- Support for hardware timestamping in platform.c for Linux
- Async event and prescaler turned off by default
- Improved code documentation and comments
- Refactored example applications for better clarity
- Various minor code improvements and optimizations


## [V1.0.0]

### Breaking API Changes

- XcpInit signature changed to
```c
void XcpInit(const char *name, const char *epk, bool activate);
```

- A2lInit signature changed to
```c
    bool A2lInit(const uint8_t addr[4], uint16_t port, bool use_tcp, uint32_t mode);
```

- CalSeg constructor signature changed, takes a pointer to the default parameters 
```c
    CalSeg(const char *name, const T *default_params)
```

- A2lTypedefBegin gets a pointer to any instance of the type.  
- A2lTypedefParameterComponent does not need the typename parameter anymore
- A2lTypedefMeasurementComponent does not need the typename parameter anymore and has a comment parameter
```c
{
    A2lTypedefBegin(ParametersT, &kParameters, "A2L Typedef for ParametersT");
    A2lTypedefParameterComponent(min, "Minimum random number value", "Volt", -100.0, 100.0);
    A2lTypedefParameterComponent(max, "Maximum random number value", "Volt", -100.0, 100.0);
    A2lTypedefEnd();
} 

- DaqCreateEventInstance does not return the event id anymore, new function DaqGetEventInstanceId to get the event id from the name
```c
    DaqCreateEventInstance("task");
    tXcpEventId task_event_id = DaqGetEventInstanceId("task");
```

- BIN format changes to enable future feature extensions, old BIN files are not compatible anymore


### Added
- Absolute or relative calibration parameter segment addressing (`OPTION_CAL_SEGMENTS_ABS` in `xcplib_cfg.h`)
- More flexible addressing scheme configuration (see `xcp_cfg.h`)
- Generated A2L file uses the `project_no` identifier to indicate the configured addressing scheme (currently ACSDD or CASDD)
- Support for more than one base address in relative address mode, variadic function to trigger event with multiple base addresses
- Optional async event with 1ms cycle time and prescaler support (`OPTION_DAQ_ASYNC_EVENT` in `xcplib_cfg.h`)
- Different options to control the behavior of calibration segment persistence and freeze
- Memory optimization for event/daq-list mapping
- XCP_ENABLE_COPY_CAL_PAGE_WORKAROUND to enable workaround for CANape init calibration segments bug
- Variadic macro to create, trigger, and register local and member variables in one call with automatic addressing mode deduction (see hello_xcp_cpp example)
```c
    DaqEventExtVar(avg_calc1, this,                                               //
                   (input, "Input value for floating average", "V", 0.0, 1000.0), //
                   (average, "Current calculated average"),                       //
                   (current_index_, "Current position in ring buffer"),           //
                   (sample_count_, "Number of samples collected"),                //
                   (sum_, "Running sum of all samples")

    );
```

### Changed
- Signature of `xcplib::CreateCalSeg` changed, pointer to reference page
- Automatic EPK segment is now optional (`OPTION_CAL_SEGMENT_EPK` in `xcplib_cfg.h`)

### Experimental
- Tool `bintool` to convert XCPlite-specific BIN files to Intel-HEX format and apply Intel-HEX files to BIN files
- New demo `no_a2l_demo` to demonstrate workflows without runtime A2L generation (using a XCPlite-specific A2L creator, see README.md of `no_a2l_demo`)
- New demo `bpf_demo` to demonstrate usage of XCPlite together with eBPF programs for Linux kernel tracing (see README.md of `bpf_demo`)
- Internal naming convention refactored to support A2L creation for dynamic objects from ELF/DWARF binaries
- Rust xcp-lite >V1.0.0 uses the calibration segment management of XCPlite instead of implementing its own

### Fixed
- Various bug fixes



## [V0.9.2]

### Breaking Changes
- Breaking changes from V0.6

### Added
- Lockless transmit queue (works on x86-64 strong and ARM-64 weak memory models)
- Measurement and read access to variables on stack
- Calibration segments for lock-free and thread-safe calibration parameter access, consistent calibration changes, and page switches
- Support for multiple segments with working and reference page and independent page switching
- Build as a library
- Used as FFI library for the Rust xcp-lite version

### Changed
- Refactored A2L generation macros
