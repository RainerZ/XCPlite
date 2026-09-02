# Changelog

All notable changes to XCPlite are documented in this file.

## [V2.1.11]

* Allow custom memory attributes for queue32m
* Avoid mutex allocation when queue32m uses critical sections
* Support optional recursive FreeRTOS mutexes
* Fix typos in FreeRTOS documentation and comments
* Fix UDP segment size calculation from MTU
* Make FreeRTOS queue segment count configurable


## [V2.1.10]

- Build cleanup: `shm.h` and `persistence.h` are only included where the corresponding feature is enabled, reducing overhead for minimal embedded source subsets
- `xcpclient` related changes: 
    - EPK validation — A2L file EPK is now compared against the EPK reported by the target; mismatch prints a warning and aborts unless overridden with `--yes`
    - Dependency on `mc_registry` switched to public `RainerZ/xcp-lite` 3.0.9
    - Improved error message when AML file is not found
    - Uses ELF section boundary variables as fallback event detection when `xcp_evts` section is absent
    - Fixed ESP32 image containing multiple DROM segments.
    - Handle C++ class types and fix empty typedefs for template structs in offline A2L generation
    - The DWARF type reader was merged up to a2ltool 3.4.1.
    - Unique names for field typedefs in the A2L writer (`xcp_registry` update)
    - Fixed the size of pointer members in structs and classes
    - No panic on ELF files without `xcp_evts` section or with duplicate event markers
    - README section on supported types and limitations, unit tests with a C++ type fixture
    - Fixed free_rtos_esp32_demo linker map section based event detection when dynamic event management is disabled.
- Documentation improvements.
- Fixed metadata macros XCP_LIMITS (missing ;) and XCP_READ_WRITE ([])
- Fixed link type_detection_test against xcplite for XCPLITE_CONFIGURATION define


## [V2.1.9]

- XCP DAQ event info (#define XCP_ENABLE_DAQ_EVENT_INFO) support for disabled OPTION_DAQ_EVENT_LIST with section registered events
- New example `no_a2l_demo_cpp` demonstrating build-time A2L generation for C++
- A2L generator declarations conditionally removed from public API headers (`inc/xcplib.h`, `inc/xcplib.hpp`) for non-A2L configurations
- C++ namespace alias `xcplib` removed (breaking change for code using `xcplib::` instead of `xcplite::`)
- `XcpSetA2lName` and `XcpGetA2lName` are now independent from enabling the A2L generator
- Hardcoded maximum calibration segment name length limited by alignment requirements
- Fixed `cpp_demo` shutdown issue
- Windows build fixes
- Removed code definition of `_GNU_SOURCE`, made it public in CMakeLists.txt

- `xcpclient` related changes:
    - New `XCP_READ_WRITE` metadata macro
    - Added enum conversion support in ELF->A2L converter
    - Updated `mc_registry`, `gimli` and `object` crate dependencies to latest versions
    - New options `--elf_skip_no_metadata` and `--config <xcpclient.toml>`
    - Various bugfixes in debuginfo parsing and `no_a2l_demo` event handling


## [V2.1.6] 

- Critical bugfix in ´XcpEventExt_´, undefined behaviour when DAQ is not running 
- Optimizations for microcontroller/RTOS builds, optimized locking (critical section instead of mutex) and static allocated queue (new ´queue32m.c´)
- Link time, deterministic event id assignment (´tXcpEventDescriptor´ in section ´xcp_evts´)
- Implemented ´XCP_LIMIT´, XCP_COMMENT and ´XCP_UNIT´ metadata definition to support A2L generation in RTOS use cases (new linker section ´xcp_meta´). See example no_a2l_demo for details
- ´xcpclient´ tool refactoring and bugfixes, xcp_registry dependency switched back to ´xcplite´ crate main on VectorGrp
- Made OPTION_DAQ_EVENT_LIST an configuration override and removed special configuration handling with XCPLIB_FOR_RUST
- Enabled 1ms event ´async´ (event id 0) as default event for global variables in RTOS builds
- ´XcpGetEcuEpk´ renamed to ´XcpGetLocalEpk´ for clarity in SHM mode
- silkit_demo updated
- ´DaqCreateEventExt´ fixed
- Asserts removed from the DAQ trigger macros 


## [V2.1.5]

- freertos_stm32_demo with FreeRTOS and lwip for STM32 microcontrollers
- More flexibility how to provide the DAQ timebase
- Single source demo code for all FreeRTOS examples


## [V2.1.3]

- cmake build system revisited, build configurations and build options
- Improved convenience build script
- README files for all example applications
- xcpclient only uses the registry (xcp-lite xcp_registry crate), not the full Rust xcp-lite stack


## [V2.1.0]

- esp32_freertos_demo with FreeRTOS and lwip
- freertos_demo using FreeRTOS POSIX emulator port
- Heap allocation removed from socket platform abstraction layer
- Improved compatibility for 32 bit microcontroller RTOS operating systems (like freeRTOS, zephyr, ThreadX etc.)
- Improved support for offline A2L creation from ELF files, file system dependency is now optional 
- XCP event and segment descriptor memory sections to preregister events and calibration segments/block in XcpInit, for deterministic event numbers without .BIN file
- The A2L generator in xcpclient can create an A2L file template with XCP events and segments from the ELF file only, by inspecting the event and segment descriptor memory sections
- Optional custom GET_ID to download the ELF file instead of the A2L file
- Addressing schema XCPLITE__AXSDD, memory access via callbacks, no calibration segment management 


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
- xcpclient tool in Rust for testing and for offline A2L generation
- Improved Windows performance


### Breaking changes

- The signature of XpcInit has changed to
```c
void XcpInit(const char *name, const char *epk, uint8_t mode);
```
- The return value contract of `socketRecv` and `socketRecvFrom` has changed. Only code that uses these functions directly (i.e. code that includes `platform.h` is affected)

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
- Generated A2L file uses the `project_no` identifier to indicate the configured addressing schema (currently ACSDD or CASDD)
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
