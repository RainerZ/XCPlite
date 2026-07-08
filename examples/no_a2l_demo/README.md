# no_a2l_demo Demo

Demonstrates XCPlite usage without runtime on-target A2L database generation.

libxcplite can be built without A2L generation and persistence support, reducing code size
and file system dependencies for microcontroller / RTOS targets (FreeRTOS, Zephyr, ThreadX, ...).
The configuration is applied via `xcplib_no_a2l_cfg.h` using the `XCPLIB_CFG_OVERRIDE` mechanism —
see the [Library Configuration Override](#library-configuration-override) section below.

The A2L database is instead generated offline by the `xcpclient` tool (ELF/DWARF → A2L converter),
which is part of this repository under `tools/xcpclient/`.



## The XCPlite Build Time A2L Generation Concept
  
The fundamental idea is to provide a specialized A2L database creator (ELF -> A2L converter) designed exclusively for XCPlite.  
The XCPlite A2L creator knows implementation details of the XCPlite code instrumentation library to automate the A2L generation process as much as possible:  
- It automatically detects all events and calibration memory segments created by the XCPlite instrumentation macros.
- It detects the code locations of the event trigger points and automatically associates local and member variables with complex types existing in each events scope.  
- It can add metadata for calibration parameters and measurement variables, such as physical units, min/max limits, and scaling information.


The test XCP client in the xcpclient tool can work with the ELF file directly, no need for a separate A2L file. 


### Concept of the xcpclient A2L Creator

An XCPlite specific A2L creator/writer with ELF/DWARF reader is built into the xcpclient tool.  

Option 1: A2L template generation:

- Creates a complete A2L template with IF_DATA, epk version, memory segments and events from ELF by detecting static segment and event marker variables created by the XCPlite code instrumentation


Option 2: Full A2L content generation:

- Add calibration parameters
    Option 1: #undef OPTION_CAL_SEGMENTS_ABS
        XCP configured for segment relative addressing mode
        Detect calibration parameters by their segment number and offset.
    Option 2: #define OPTION_CAL_SEGMENTS_ABS
        XCP configured for absolute calibration segment addressing
        This is the preferred option for microcontrollers
        The reference pages of all calibration parameters must be in addressable (4 GB - 32bit) global memory (.bss or .rodata segment must be in this range)
        Detect calibration parameters by the address of their default/reference page by naming convention and segment marker variable

- Add measurement variables
    Global or static measurements must be in addressable (4 GB - 32bit) global memory (.bss segment must be in this range)
    Takes all global, static and local variables into account in specified compilation units
    Try to detect an appropriate fixed event for each variable by detecting a event trigger in the same function, if not use the unsafe default event named `async` as default event

- Add all types required as TYPEDEF_STRUCTURE

Content generation step 2 can alternatively be done manually, with any other A2L tool from Vector or open source




## Library Configuration Override

The default configuration options for xcplite are set in `src/xcplib_cfg.h`.
All defaults are defined there; nothing needs to be edited for a `no_a2l` build.

For this use case a dedicated override file `src/xcplib_no_a2l_cfg.h` adjusts only the settings that
differ from the defaults:

```c
// src/xcplib_no_a2l_cfg.h — lean override, only differences from xcplib_cfg.h defaults
#undef  OPTION_ENABLE_PERSISTENCE       // no file system needed on bare-metal / RTOS targets
#define OPTION_CAL_SEGMENTS_ABS         // absolute addressing has address extension 0, calibration segments may be accessed with the absolute address of their static default page
#undef  OPTION_CAL_SEGMENT_EPK          // no EPK segment
#define OPTION_QUEUE_32                 // 32-bit queue
#undef  OPTION_QUEUE_64_VAR_SIZE
#undef  OPTION_QUEUE_64_FIX_SIZE
#undef  OPTION_ENABLE_A2L_GENERATOR     // no on-target A2L generation
#undef  OPTION_ENABLE_A2L_UPLOAD        // no A2L upload via XCP
#undef OPTION_ENABLE_ELF_UPLOAD         // no ELF upload via XCP, A2L creator works with ELF file on disk as well
#define OPTION_DAQ_ASYNC_EVENT          // Create event 'async' with id 0 as default event for global variables

```

The override is applied at the end of `xcplib_cfg.h` via:

```c
#ifdef XCPLIB_CFG_OVERRIDE
#include XCPLIB_CFG_OVERRIDE
#endif
```

Pass the override file to xcplite at compile time:

```cmake
# CMakeLists.txt
target_compile_definitions(xcplite PRIVATE "XCPLIB_CFG_OVERRIDE=\"xcplib_no_a2l_cfg.h\"")
```

This is already set when building with `XCPLITE_CONFIGURATION=no_a2l`.
The same pattern can be used to create any other application-specific configuration override.

> **Note:** The no_a2l_demo must be built in isolation because the override disables the A2L generator that other examples depend on. A dedicated build directory `build-no_a2l` avoids cache conflicts.

---

## How xcpclient finds your events and variables

`xcpclient --create-a2l` discovers events, calibration segments, and local variables
automatically — **without any runtime A2L calls in your code** — by reading named ELF
sections and DWARF debug info written by the XCPlite macros.

To make this work correctly, follow these rules:

1. **Always use the macros, never the raw C API** (`XcpCreateEvent`, `XcpCreateCalSeg`, etc.).
   Only the macros emit the ELF section data and DWARF anchors that xcpclient needs.

2. **Mark local measurement variables `volatile`** in optimized builds.
   Without `volatile` the compiler may eliminate stack variables or give them unreliable
   DWARF location expressions:
   ```c
   void myTask(void) {
       volatile uint32_t counter = 0;  // XCP: keep on stack for offline A2L
       DaqCreateAndTriggerEvent(myTask);
   }
   ```

3. **Build with debug information** (`-g` / `CMAKE_BUILD_TYPE=Debug` or `RelWithDebInfo`).
   xcpclient reads DWARF; stripped builds have no type or location data.

4. **`CalSegDecl` must be at file scope** (outside any function) so the descriptor is
   allocated for the program lifetime and xcpclient can find it.

For the full ELF/DWARF mechanics — section layouts, the `trg__` anchor naming convention,
and address encoding — see
[docs/TECHNICAL.md — Offline A2L Generation](../../docs/TECHNICAL.md#offline-a2l-generation--elfdwarf-internals).

---

## API subset for no-A2L workflows

The following API subset remains fully available and is unchanged:

### Server

| Function | Purpose |
|---|---|
| `XcpInit(name, epk, mode)` | Initialize the XCP core |
| `XcpEthServerInit(addr, port, tcp, queue_size)` | Start the XCP/Ethernet server |
| `XcpEthServerShutdown()` | Stop the server |
| `XcpSetLogLevel(level)` | Set log verbosity |

### Events and measurement triggers (`xcplib.h`)

| Macro | Emits to ELF | Purpose |
|---|---|---|
| `DaqCreateEvent(name)` | `xcp_evts` | Register a named DAQ event; emits `tXcpEventDescriptor` |
| `DaqCreateEventExt(name)` | `xcp_evts` | Register a named DAQ event with cycle time and priority; emits `tXcpEventDescriptor` |
| `DaqCreateAndTriggerEvent(name)` | `xcp_evts` + DWARF anchor | Declare and trigger in one step — ideal for short-lived functions |
| `DaqTriggerEvent(name)`, `DaqTriggerEventAt` | DWARF anchor (`trg__AAS__`) | Trigger and anchor local variable scope for xcpclient |
| `DaqTriggerEventExt(name, base)`, `DaqTriggerEventExt_s(name, base)` | DWARF anchor (`trg__AASD__`) | Trigger with explicit base pointer (heap / relative addressing) |
| `DaqEventVar(event_name, ...)` | — | Declare a variadic DAQ event |
| `DaqEventEnable(name)` / `DaqEventDisable(name)` | — | Enable/disable individual events at runtime |


### Calibration (`xcplib.h`)

> **These macros are essential for offline A2L generation.** Only `CalSegDecl` + `CalSegCreate`
> emit the `xcp_cals` section descriptor that xcpclient reads. `XcpCreateCalSeg()` does not.

| Macro / Function | Emits to ELF | Purpose |
|---|---|---|
| `CalSegDecl(name)`, `CalSegCreate(name)` | `xcp_cals` (at file scope) | Declare + emit `tXcpCalSegDescriptor` into `.xcp_cals` |
| `CalSegLock(name)` | — | Lock segment for read; returns `const T *` to the active page |
| `CalSegUnlock(name)` | — | Release the lock |

### C++ RAII calibration wrapper (`xcplib.hpp`)

For C++ no-A2L builds, use `CalSegDeclRef` or `CalSegDecl` — these emit the `xcp_cals`
section descriptor (like the C `CalSegDecl`) and additionally create a typed `CalSegRef<T>`
RAII handle. Registration is done by `XcpInit()` from the section data; no runtime
`XcpCreateCalSeg()` call is needed.

| Macro / Class | Emits to ELF | Purpose |
|---|---|---|
| `CalSegDeclRef(val, handle)` | `xcp_cals` (at file scope) | Declare section descriptor + create named `CalSegRef<T>` handle |
| `CalSegDecl(val)` | `xcp_cals` (at file scope) | Shorthand — handle is named `val##_calseg` |
| `handle.lock()` | — | Lock and return `CalSegGuard` (RAII `const T *`, auto-unlocks) |

> **Note:** `xcp::CalSeg<T>(name, ptr)` registers at runtime via `XcpCreateCalSeg()` and
> is for builds **with** on-target A2L generation. Do not use it in no-A2L builds.

### What is NOT available without on-target A2L

The following are compiled out when `OPTION_ENABLE_A2L_GENERATOR` is not set:

- `A2lCreateMeasurement*` / `A2lCreateParameter*` / `A2lCreateCurve*` / `A2lCreateMap*`
- `A2lTypedefBegin` / `A2lTypedefEnd` / `A2lTypedefXxxComponent`
- `A2lCreateTypedefInstance` / `A2lCreateTypedefReference`
- `A2lSetAbsoluteAddrMode` / `A2lSetRelativeAddrMode` / `A2lSetStackAddrMode` / `A2lSetSegmentAddrMode`
- `A2lCreateLinearConversion` / `A2lCreateEnumConversion`
- `A2lFinalize` / `A2lOnce`

These are replaced by the offline `xcpclient --create-a2l --elf <binary>` workflow.

---

## Using the xcpclient tool for A2L generation

```bash
# Build the no_a2l_demo (isolated build directory build-no_a2l/)
./build.sh no_a2l

# Or directly with CMake
cmake -B build-no_a2l -S . -DXCPLITE_CONFIGURATION=no_a2l -DCMAKE_BUILD_TYPE=Debug -DXCPLITE_BUILD_EXAMPLES=ON
cmake --build build-no_a2l
# or with forcing ARM architecture (macOS)
cmake -B build-no_a2l -S . -DXCPLITE_CONFIGURATION=no_a2l -DCMAKE_BUILD_TYPE=Debug -DXCPLITE_BUILD_EXAMPLES=ON -DCMAKE_OSX_ARCHITECTURES=arm64
cmake --build build-no_a2l

# Generate an A2L file for the no_a2l_demo application offline from its ELF file
# Example:
# Add all variables
xcpclient  --offline --elf build-no_a2l/no_a2l_demo --a2l no_a2l_demo.a2l --create-a2l --verbose 1
# Add the given IP address:port and protocol to the generated A2L file
xcpclient --udp --dest-addr 192.168.0.206  --offline --elf build-no_a2l/no_a2l_demo  --a2l no_a2l_demo.a2l  --create-a2l 
# Filter on specific variables and compilation units
xcpclient --offline --udp --dest-addr 192.168.0.206 --elf build-no_a2l/no_a2l_demo   --a2l no_a2l_demo.a2l --create-a2l --elf-unit-filter main --elf-var-filter "^(counter|params)"


# Connect to the XCP on UDP server on 192.168.0.206:5555, upload ELF file from target (requires OPTION_ENABLE_ELF_UPLOAD) and create the A2L file
xcpclient --udp --dest-addr 192.168.0.206  --elf no_a2l_demo.elf --upload-elf  --create-a2l --elf-unit-filter main --elf-var-filter "^(counter|params)"

# Measurement of variable global_counter with the ELF file only
xcpclient --udp --dest-addr=192.168.0.206:5555 --elf no_a2l_demo.elf --elf-var-filter "global_counter" --mea ".*" --time 5 --csv no_a2l_demo.csv

```


## Other A2L generation options

### Using Vector CANape integrated A2L editor and ELF file support

Drop the template generated by xcpclient into CANape and create a new XCP on Ethernet device.  
Enable access to the ELF file in the device configuration.  
Use the A2L editor to add individual measurement parameters.  
For calibration segments or blocks, add the complete default value structure as an INSTANCE of TYPEDEF_STRUCTURE or add the variables as CHARACTERISTIC.  


### Using Vector A2L-Toolset A2L-Creator to add measurement and calibration metadata

The example code contains some A2L creator metadata annotation to add metadata such as calibration variable limits and physical units.  
The A2L Creator is a commercial Vector product.  

### Using Open Source a2ltool

Example:
Add the calibration segment `params` and the measurement variable `counter` to the A2L template:

```bash
a2ltool  --update --measurement-regex "counter"  --characteristic-regex "params" --elffile  no_a2l_demo.elf  --enable-structures --output no_a2l_demo.a2l 
```





### TODO List and open issues

- Add C++, name spaces, classes, member functions, ...
- The A2L creator may create the BOOL conversion rule and detect the size of the bool type
- Heap measurement variables
    The A2L creator can not handle heap variables yet
    Needs to detect trg__AAS or trg__AASD type and analyze the argument type of DaqTriggerEvent(), pointer to type
- Thread local variables
    The A2L creator can not handle thread local variables yet
    The DAQ capture method does not work for TLS, need a ApplXcpGetTlsBaseAddress() function, maybe introduce AAST type
    Detect the base address of the TLS block, like it is done in ApplXcpGetBaseAddr()/xcp_get_base_addr() for the global variables
    The DaqCapture macros as an alternative, does not work yet
- Function parameters
    Define a macro to declare function parameters as XCP_MEA, which spills them to stack
    A2L Creator ELF reader parser must detect the function parameters with the CFA offset in the stack frame
- Make sure the event trigger location and the variable location have the same CFA (not seen any violations yet)
