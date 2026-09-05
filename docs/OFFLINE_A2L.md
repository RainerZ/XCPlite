# Offline A2L Generation with xcpclient

XCPlite can generate the A2L file on the target at runtime ([on-target A2L generation](TECHNICAL.md#on-target-a2l-file-generation)),
or the A2L file is generated offline from the ELF file of the application by the ELF/DWARF to A2L generator built into the
`xcpclient` tool (`tools/xcpclient/`). Offline generation is used by the `no_a2l` (Linux,MacOS) and `rtos` (FreeRTOS) build configurations: the library is built without A2L generator and without file system dependency, which reduces code size on microcontroller and RTOS targets.

The generator is specific to XCPlite. It knows the markers the XCPlite instrumentation macros leave in the ELF file and the relative
addressing modes of XCPlite, so it creates a complete A2L file for measurement variables on the stack, calibration parameters in
segments and complex types, which a general purpose A2L creator can not reconstruct from the debug information alone.

See `examples/no_a2l_demo`, `examples/no_a2l_demo_cpp` and `examples/freertos_demo` for complete examples with build scripts.

## Concept

The instrumentation macros place information in the ELF file at compile and link time. The library and the generator use it:

| Source in the ELF file | Written by | Used for |
|---|---|---|
| `xcp_evts` section | `DaqCreateEvent`, `DaqCreateEventExt`, `DaqCreateAndTriggerEvent` | All events with name, cycle time and priority. The position of the descriptor in the section is the event id, on the target and in the A2L file |
| `xcp_cals` section | `CalSegDecl`, `CalSegDeclRef`, `CalSegCreate` | All calibration segments with the address and the size of their default page. The order of the descriptors is the segment number |
| `xcp_epk` section | `XcpCreateEpk` | The EPK software version string and its address |
| `xcp_meta` section | `XCP_UNIT`, `XCP_LIMITS`, `XCP_COMMENT`, `XCP_READ_WRITE` | Metadata of measurement and calibration objects |
| DWARF scope of the `trg__<modes>__<event>` anchor variables | `DaqTriggerEvent`, `DaqCreateAndTriggerEvent`, `DaqTriggerEventExt`, `DaqEventVar` | The function in which an event is triggered, its stack frame and the addressing modes available at the trigger point |
| `XCPLITE__<signature>` variable | libxcplite | The addressing scheme of the target (`CASDD`, `ACSDD`, `AXSDD`, `CXSDD`), see [addressing modes](TECHNICAL.md#addressing-modes) |
| DWARF variables and types, ELF symbol table | Compiler and linker | Names, addresses, stack frame offsets and types of all global, static and local variables |

The macro expansions and the naming conventions of the markers are the contract between the library and the generator, they are
described in [TECHNICAL.md — Instrumentation Markers for Offline A2L Tools](TECHNICAL.md#instrumentation-markers-for-offline-a2l-tools).

The same link time information is used by the library itself: `XcpInit` registers the events and calibration segments from the
sections in a deterministic order, so the event ids and segment numbers in the A2L file stay valid independent of the code execution
order, without a persistence file.

## Workflow

1. Build the application with debug information: `-g`, `CMAKE_BUILD_TYPE=Debug` or `RelWithDebInfo`. Optimized builds work, see the
   rules for the application code below.
2. Generate the A2L file from the ELF file, offline or with the running target:

```bash
# Offline: events and segments from the ELF file only, transport layer parameters for the A2L IF_DATA from the command line
xcpclient --offline --udp --dest-addr 192.168.0.206 --elf build-no_a2l/no_a2l_demo --create-a2l --a2l no_a2l_demo.a2l

# Online: events and segments are checked against the running target (GET_EVENT_INFO, GET_SEGMENT_INFO), the EPK is checked
xcpclient --udp --dest-addr 192.168.0.206 --elf build-no_a2l/no_a2l_demo --create-a2l --a2l no_a2l_demo.a2l

# The ELF file may also be uploaded from the target (OPTION_ENABLE_ELF_UPLOAD)
xcpclient --udp --dest-addr 192.168.0.206 --upload-elf --elf no_a2l_demo.elf --create-a2l --a2l no_a2l_demo.a2l

# Restrict the variables to compilation units and names (regular expressions)
xcpclient --offline --elf build-no_a2l/no_a2l_demo --create-a2l --a2l no_a2l_demo.a2l --elf-unit-filter main --elf-var-filter "^(counter|params)"

# Only annotated variables (XCP_UNIT, XCP_LIMITS, XCP_COMMENT)
xcpclient --offline --elf build-no_a2l/no_a2l_demo --create-a2l --a2l no_a2l_demo.a2l --elf-skip-no-metadata

# A2L skeleton with events, segments and IF_DATA only, to be completed with other tools
xcpclient --offline --elf build-no_a2l/no_a2l_demo --create-a2l-template --a2l no_a2l_demo_template.a2l
```

3. Use the A2L file in the XCP tool. The xcpclient test client itself can work with the ELF file directly (`--elf` with `--mea`,
   `--cal`, `--list-mea`), no A2L file is needed for it.

`examples/no_a2l_demo_cpp/create_a2l.sh` shows a complete round trip: sync the sources to the target, build there, download the ELF
file and generate the A2L file. The command line reference is in [tools/xcpclient/README.md](../tools/xcpclient/README.md).

### Calibration segment addressing

The addressing scheme of the target is read from the `XCPLITE__<signature>` variable in the ELF file and written as `PROJECT_NO` into
the A2L header:

- `XCPLITE__ACSDD` (`OPTION_CAL_SEGMENTS_ABS` defined): calibration parameters are addressed by the absolute address of their default
  page with address extension 0. This is the usual choice for microcontrollers. The default pages of all segments must be in the 32 bit
  address range of the target and have static lifetime.
- `XCPLITE__CASDD` (default): calibration parameters are addressed by segment number and offset with address extension 0, the default
  pages may be anywhere in a 64 bit address space. The segment numbers are read from the target when connected, or derived from the order
  of the descriptors in the `xcp_cals` section.

## Rules for the application code

- Use the instrumentation macros (`DaqCreateEvent`, `DaqTriggerEvent`, `CalSegDecl`, `XcpCreateEpk`, ...), not the C API functions
  (`XcpCreateEvent`, `XcpCreateCalSeg`). Only the macros emit the sections and the anchor variables.
- Mark local measurement variables `volatile` (or use the `XCP_MEA` attribute). Otherwise an optimizing compiler might keep them in registers, the DWARF entry has no location and the variable is skipped.
- Declare calibration segments with `CalSegDecl` or `CalSegDeclRef` and give the default page static lifetime. File scope is
  recommended. The segment name and the name of the default page variable are identical by convention, the generator relies on it.
- Metadata macros name the object with `__` as path separator: `XCP_UNIT(params__delay_us, "us")` annotates the field `delay_us` of the
  instance `params`. A macro placed in the same namespace as the variable, or in the same function as a local variable, does not need a
  scope prefix: `XCP_COMMENT(input, ...)` in namespace `motor_control` annotates `motor_control.input`, `XCP_COMMENT(counter, ...)` in
  function `foo` annotates `foo.counter`. Explicit prefixes (`foo__counter`) are possible anyway.

## What the generator derives from the ELF file

This section describes how `xcpclient --create-a2l` discovers events, calibration segments, variables and types, for contributors
and for developers who need to understand why a variable does or does not appear in the generated A2L file.

### Events

Every descriptor in the `xcp_evts` section is an event. The event id is the position of the descriptor in the section, which is also how
`XcpInit` assigns the ids on the target. If the same event is created at several places (a `DaqCreateEvent` in several functions or
compilation units), the descriptor with the lowest address wins, on the target and in the generator. Without an `xcp_evts` section (a
linker script may merge it into another output section) the linker symbols `__start_xcp_evts` and `__stop_xcp_evts` are used. Without
both, the events get placeholder ids, which are corrected from the event information of the target when connected.

### Calibration segments

Every descriptor in the `xcp_cals` section is a calibration segment or block. The default page variable of a segment has the same name
as the segment, its DWARF type gives the size and the layout: the parameters become a `TYPEDEF_STRUCTURE` with an `INSTANCE`, or
`CHARACTERISTIC` objects for basic types. Variables whose address lies within a segment are calibration parameters, all other variables
are measurements.

### Trigger points and local variables

The trigger macros emit a static variable `trg__<modes>__<event>` in the function in which the event is triggered. Its DWARF scope gives
the function, the canonical frame address (CFA) of the function at the trigger point and the addressing modes available there (the mode
letters, see the marker contract). Local variables of that function are registered with stack frame relative addresses (address
extension 2) and the event as fixed event, static variables in the function get the event as well. Global variables and static variables
in functions without an event trigger are registered without a fixed event, in this case it is in the responsibility of the XCP tool user to assign an event which allows correct visibilty and consistent capture of the associated variables. CANape usually defaults to polling in this case, and each available event may be selected for synchronous data acquisition. 
(@@@@ TODO: Maybe add a feature to define a default event (see OPTION_ASYNC_EVENT), so CANape does not default to polling)

### Variables and symbols

- The address of a variable comes from its `DW_AT_location`. Variables without a location (declarations, `static const` data in a
  namespace, the metadata markers in optimized builds) are resolved from the ELF symbol table: by `DW_AT_linkage_name`, by name, by the
  Itanium mangled name of a namespace scope variable (`_ZN13motor_controlL5inputE`) or by a unique name suffix (`_ZZ4mainE7counter` for a
  static local). For variables inside a function only symbols with local binding are considered, a global symbol with the same name
  belongs to a different variable.
- GCC describes a namespace scope variable with a declaration entry inside the namespace and a definition entry at compilation unit level
  (`DW_AT_specification`), both are merged into one variable.
- Variables with the same name get distinct A2L names: static variables in functions are prefixed with the function (`foo.counter`),
  global variables defined in several namespaces with their namespace (`motor_control.input`).

### Type names

The `DW_AT_name` of a `DW_TAG_structure_type` or `DW_TAG_class_type` entry is the unqualified type name (`Input` for
`motor_control::Input`). The enclosing namespace, class or function is only visible from the position of the entry in the DWARF tree:
it is a child of the `DW_TAG_namespace`, `DW_TAG_class_type` or `DW_TAG_subprogram` entry. Type entries have no `DW_AT_linkage_name`,
only variables and functions carry a mangled name. Every compilation unit which uses a type has its own copy of the type entry.

A2L has one flat name space for `TYPEDEF_STRUCTURE`, so the generator records the enclosing scopes of the type entries while traversing
the tree and names the typedefs as follows:

- The typedef is named after the type. If struct/class types with the same name exist in different scopes, all of them are qualified
  with their scope (`motor_control.Input`, `valve_control.Input`, `MotorController.Params`), types with a unique name keep their plain name.
  Type names which are not valid A2L identifiers (template instantiations such as `TplStruct<float>`) are sanitized (`TplStruct_float_`).
- Typedefs with identical content are merged: the same type used by several variables, or the copies of a type from several compilation units.
- A name which is still used by a typedef with different content (types without a scope in different C files, or a type used for
  measurement and for calibration variables) simply get a numeric suffix (`state_1`), which is reported as a warning.
- The `TYPEDEF_MEASUREMENT` or `TYPEDEF_CHARACTERISTIC` of a struct field is named after the field. If another structure has a field
  with the same name but a different type or metadata, the name is qualified with the sanitized structure name (`TplStruct_float_.value`).

### Metadata

Each metadata macro emits a constant named `xcp_meta__<kind>__<name>` into the `xcp_meta` section, with `<kind>` one of `unit`, `min`,
`max`, `comment` or `read_write`. After the variables are registered, the constants are matched to their A2L objects: the name is looked
up qualified with the scope of the marker first (its namespace, or its function for a local variable), then unqualified. `__` in the name
is the path separator for the fields of typedef instances (`params__delay_us`, `motor_control__input__speed`). Metadata never adds
objects, it only annotates variables which were registered from the sources above. With `--elf-skip-no-metadata` every variable without
any annotation is removed from the A2L file, a convenient way to publish only explicitly curated signals.

## Supported types and limitations

The DWARF type information is mapped to A2L objects as follows:

| C/C++ type | A2L representation |
|---|---|
| `bool`, integer and floating point types | `MEASUREMENT` or `CHARACTERISTIC` of the matching A2L data type |
| `enum` | integer of the enum's size; for variables the enumerators become a verbal conversion table, enum struct members are plain integers |
| one- and two-dimensional arrays | `MEASUREMENT` / `CHARACTERISTIC` with `MATRIX_DIM` (`VAL_BLK`, `CURVE`, `MAP`); arrays of structs become arrays of typedef instances |
| `struct`, `class`, template instantiations | `TYPEDEF_STRUCTURE` + `INSTANCE`; nested structs and classes become nested typedefs; private members are included; base class members are flattened into the derived type for all combinations of `struct`/`class` bases; `static`/`constexpr` members are skipped |
| pointers as struct or class members | the address value as unsigned integer of the target's pointer size, the pointee is not followed |

Not supported, future extensions, cases skipped and reported as warnings (log level 2 and above):

- Variables of pointer type (measure the pointed-to variable instead).
- Unions, bitfields and function pointers. A struct member of such a type is written as a one byte `UBYTE` placeholder so that the
  remaining members of the structure keep their offsets.
- Arrays with more than two dimensions (written as a byte array placeholder) (@@@@ TODO verify this claim).
- C++ pointer-to-member types (`DW_TAG_ptr_to_member_type`): a struct or class containing one cannot be read at all, so it and every
  class deriving from it end up without members. This is a limitation of the a2ltool DWARF reader this code is based on.
- C++ library containers (`std::vector`, `std::string`, smart pointers, ...) are read as the structs they are; the heap data behind them
  is not reachable (@@@@ TODO: Maybe add a blacklist feature to remove these).
- Variables addressed relative to a base pointer (`DaqTriggerEventExt`, the dynamic slots of `DaqEventVar`, address extension 3 and
  above) are not generated yet, only absolute and stack frame relative addressing is.
  (@@@@ TODO: Create a concept how to handle this)
- Thread local variables, function parameters and the capture buffers of `DaqCapture` are not evaluated yet.
  (@@@@ TODO: future feature ?)
- Local variable in functions without events are skipped
  (@@@@ TODO: future feature ?, trigger if called by ?, with stack unwinding check)

## Diagnostics

Messages worth knowing when a variable is missing or looks wrong in the A2L file:

| Message | Meaning |
|---|---|
| `Struct/class type 'x' in unit has a different definition or object type than the existing typedef 'x', registered as typedef 'x_1'` | Two different types with the same name and no scope to qualify them with, or one type used for measurement and for calibration. Both get their own typedef. |
| `Local variable 'x' in function 'f' skipped, could not find event for dyn addressing mode` | The function contains no event trigger, so there is no stack frame anchor for its local variables. |
| `Variable 'x' not registered, no address` (log level 4) | The variable has no DWARF location, typically a local variable held in a register. Make it `volatile`. |
| `Global variable 'x' not registered, address ... out of the 32 bit XCP address range` | The variable is outside the addressable range, see the addressing modes. |
| `Metadata 'xcp_meta__...': no matching registry entry for '...'` | The annotated variable was not registered, or the name does not match. Check the scope prefix and the `__` path. |
| `Metadata variable '...' address is 0` | The marker has no DWARF location and no resolveable symbol. |
| `New event '...' found, created with undefined event id ...` | No `xcp_evts` section and no linker symbols. Connect to the target to get the ids. |
| `Calibration segment reference page variable 'x' has N usable definitions, expected 1` | The name of the default page variable is ambiguous, restrict the compilation units with `--elf-unit-filter`. |
| `EPK mismatch: A2L file '...' has EPK '...', target reports EPK '...'` | The A2L file does not belong to the running build. `--yes` overrides the check. |

## Other tools

The A2L template generated with `--create-a2l-template` contains the IF_DATA, the EPK, the memory segments and the events, and can be
completed with other tools:

- **Vector CANape A2L editor with ELF support**: create a new XCP on Ethernet device from the template, enable the ELF file in the
  device configuration and add measurement and calibration objects in the A2L editor. For calibration segments, add the default page
  structure as an `INSTANCE` of a `TYPEDEF_STRUCTURE` or the parameters as `CHARACTERISTIC` objects.
- **Vector A2L-Toolset A2L-Creator**: the examples contain metadata annotations as comments (`@@ SYMBOL`, `@@ STRUCTURE`, ...) for
  the commercial A2L Creator.
- **a2ltool** (open source), for example to add the calibration segment `params` and the measurement `counter` to the template:

```bash
a2ltool --update --measurement-regex "counter" --characteristic-regex "params" --elffile no_a2l_demo.elf --enable-structures --output no_a2l_demo.a2l
```

These tools reconstruct absolute addresses only. Stack frame relative and segment relative addressing needs the XCPlite specific
generator.
