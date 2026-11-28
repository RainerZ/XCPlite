# Changelog xcpclient

All notable changes to xcpclient are documented in this file. Changes to the XCPlite library are in the [XCPlite changelog](../../CHANGELOG.md).

xcpclient is released together with XCPlite. Each entry names the xcpclient version (`version` in `Cargo.toml`) and the XCPlite version it was released with. The xcpclient version was not always increased in the past: some XCPlite releases contain xcpclient changes with an unchanged version number, some versions were released together with several XCPlite versions. The XCPlite versions are noted for each entry.


## [4.3.0] - XCPlite V2.4.5 (unreleased)

- Measurement of arrays.
- New MCP server mode (`--mcp`, `--mcp-read-only`, `--mcp-dir`, cargo feature `mcp`, default on): xcpclient offers its functions as tools to an AI assistant and keeps the XCP connection between the tool calls, see `docs/MCP.md`.
- MCP server mode, measurement: `xcp_sample` observes signals for a time window and returns statistics (count, rate, minimum, maximum, mean, last value), `xcp_measure_start` and `xcp_measure_stop` record signals into MDF, MCAP, CSV or Perfetto files in the background, optionally for a given duration. `xcp_read` returns the last sample of a signal of the running measurement, `xcp_status` shows the progress. A recording is stopped and its files are completed on disconnect, on shutdown and when the connection is lost.
- MCP server mode, A2L and parameter files: `xcp_load_a2l` loads or replaces the description of the connected target, `xcp_create_a2l` creates an A2L file from an ELF file (offline or with the event and segment information of the connected target), `xcp_save_parameters` and `xcp_load_parameters` save and load the calibration segments (`.hex`, `.bin`). The warnings and errors logged while a tool runs are part of its result, warnings about single variables of an ELF file are summarized by reason.
- The EPK of an ELF file is compared with the EPK of the target, when an A2L file is created from it while connected (`--elf` with `--create-a2l`, MCP `xcp_connect`, `xcp_load_a2l`, `xcp_create_a2l`). A mismatch is handled like the one of a loaded A2L file: the command line asks or needs `--yes` and ends with exit status 1 otherwise, the MCP tools need `accept_epk_mismatch`. Before, an ELF file of another software version gave an A2L file with wrong addresses without a warning.
- xcpclient no longer panics, errors are reported with an error message and exit status.
- Command-line help: the text is wrapped to the terminal width (at most 120 columns, clap feature `wrap_help`), the options are grouped (Connection, A2L file, ELF file, Calibration, Measurement, Measurement output files, Test, MCP server). `-h` shows a one-line summary per option, `--help` the details. Before, both showed the same unwrapped text with lines of up to 555 characters.
- Fixed: an A2L upload (`--upload-a2l`, MCP `xcp_connect`) emptied the A2L file of an XCPlite application running in the same directory, when the A2L name contains a dot (`hello_xcp_V2.3`, the EPK is part of the name). The upload then failed with `A2L upload failed`. The check which avoids overwriting this file, and the name `<name>_upload.a2l` used instead, were built with `Path::with_extension`, which takes `.3` as the file extension. The default name of the parameter file (`--upload-bin`, `--download-bin` without `--bin`) had the same problem with an ECU name with a dot.
- A measurement whose stop command fails (connection lost) completes its output files. Before, an MDF file could stay unfinalized when the receive task had terminated.
- Uploaded A2L and ELF files are written to a temporary file `<file>.part`, which gets its name when the upload is complete. An existing file is not touched before, a failed upload leaves no empty file.
- MCP server mode: the files the server writes on its own (uploaded A2L file, `XCP_104.aml`, an A2L file created from an ELF file without a given name) go to the cache directory of the user (`~/Library/Caches/xcpclient`, `~/.cache/xcpclient`), not to the working directory. `--mcp-dir` sets another directory, it no longer changes the working directory.
- The steps of xcpclient (connect, A2L upload/create/load, parameter read/write, parameter files, measurement) are methods of a `Session` in the library instead of one function in `main.rs`. The writers and `a2l_fix` are modules of the library.


## [4.2.0] - XCPlite V2.4.3 (2026-11-26)

Version 4.1.0 existed only on the development branch and was not released.

- New option `--perfetto <file>`: writes spans, span links and signals as a trace file for Perfetto (Chrome Trace Event JSON format), see `README.md`. The span and link signals needed for the trace are automatically added to the measurement (option `--trace-auto`).
- Print values in physical unit or as enum strings in console and csv. Enum conversion is used to display span names.
- The CSV file has a new column `unit`.
- New option `--mdf <file>`: writes an MDF4 file.
- New option `--mcap <file>`: writes an MCAP file.
- `--upload-bin` writes the new context list and events with their context id, if there are any.
- The BIN reader shows the context list and the memory segment numbers.
- Improved timestamp wraparound detection for very sporadic events.
- The console output of `--mea` shows events by name, `EVENT: <event> (id=<id>, daq=<daq>), ...`.
- Measurement (`--mea`) is no longer limited to 123 signals or one packet per event.
- Measurement setup merges signals of an event which are adjacent in memory.
- Measurement reports a signal which is too large for the XCP server and more than 1024 DAQ lists as an error. These cases panicked before.
- Fixed reporting of lost DAQ packets.
- The UDP socket requests an 8 MB receive buffer to reduce UDP packet loss on high data rate.
- Fixed A2L and ELF upload of files smaller than the maximum command size, they were reported as not available. `GET_ID` size 0 is reported as file not available without sending `UPLOAD`.


## [4.0.1] - XCPlite V2.3.5 (2026-11-21)

The version was not increased for this release.

- The registry crate `xcp_registry` is used from crates.io (version 0.1) instead of as a git dependency on xcp-lite. It has moved to its own repository, <https://github.com/RainerZ/xcp-registry>


## [4.0.1] - XCPlite V2.3.2 (2026-10-14)

- The DWARF location expression parser no longer floods the log with warnings about variables which are dropped anyway by the filters, or which are markers the XCPlite macros generate in the code of the application (`evt__`, `trg__`, `cap__`, `xcp_meta__`, `evt_id_<event>`, ...).
- A local variable without any `DW_AT_location` in the debug information is now reported with a warning instead of silently missing from the A2L file.
- The messages of the location expression parser now name the compilation unit and the declaration line of the variable (`main_c:178: 'delay': ...`), the variable name alone does not identify which of several variables with that name is meant.
- A variable which the compiler spread over several locations (`DW_OP_piece`) is now reported as split into slices. A single piece covering only a part of the variable was silently registered with the address of the slice before, it is now rejected like the multi piece case.
- Static variables in functions now always get the function prefix in the A2L name (`main.counter`), before only when the name was used more than once. A metadata marker in the function did not reach a static variable with a unique name. The default page of a calibration segment declared in a function keeps the segment name. Renames objects in existing A2L files, e.g. `evt_id_<event>` of an event created in a function.
- The segment index variables `calseg_id_<segment>` and `calblk_id_<block>` of the calibration segment declaration macros are no longer registered as measurements.
- `--fix-a2l` now rewrites the A2L file with the event ids and calibration segment numbers of the target, the original is kept as `<file>.a2l.bak`. The event ids in the addresses of dynamic addressing mode (stack variables) and the segment numbers in calibration segment relative addresses are corrected as well, it panicked on those before. Only A2L files created by xcpclient are accepted.
- Bugfix: without the `xcp_cals` section, the `epk` calibration segment was not always numbered 0 in the A2L file, the segments were numbered in the order of the debug information, where the application usually comes before the library.
- Without the `xcp_cals` section (no `OPTION_SECTION_REGISTRATION`), a calibration segment declared with `CalSegDecl`, `CalBlkDecl` or `CalSegDeclRef` is reported, the target never creates it. More than one calibration segment is reported as well, their numbers are in declaration order, the target numbers them in creation order.
- Without the `xcp_epk` section (no `OPTION_SECTION_REGISTRATION`), the EPK string is read from the variable `xcp_epk__` of `XcpCreateEpk`, using its DWARF location and the initialized data of the ELF file. A missing `XcpCreateEpk` call, or an EPK string which the compiler or linker removed, is reported with a warning and a hint what to do.
- `ADDR_EPK` is now the address the target maps to its EPK (`XCP_ADDR_EPK`, `0xFFFFFF00` with absolute, `0x80000000` with segment relative addressing of calibration segments), not the address of the `XcpCreateEpk` string. The target keeps its own copy of the EPK given to `XcpInit`, the EPK check of a tool now reads the EPK the target actually uses.
- The EPK in the A2L file is truncated to 31 characters and space, tab and colon are replaced with an underscore, as `XcpInit` does on the target. A changed EPK is reported with a warning.
- The `epk` calibration segment has the size of the target segment (32 bytes) in an offline A2L file, not the length of the EPK string. In online mode, the different address of the `epk` segment of the target is no longer reported as a mismatch.
- Bugfix: in online mode with absolute addressing, a calibration segment which the target reported was not used to find the calibration variables in it.
- Bugfix: `--list-cal` and `--cal` with `--elf` panicked on a calibration segment struct. The registry created from the ELF file is now flattened like a loaded A2L file, the A2L file keeps the typedef structure.
- `--test` collects GET_DAQ_CLOCK latency statistics
- New option `--embed-aml`


## [4.0.0] - XCPlite V2.2.1, V2.2.2 (2026-09-14)

Breaking change: the frame address convention of the XCPlite trigger macros changed, A2L files for stack variables created by xcpclient 4.x need XCPlite V2.2.1 or later and vice versa.

- New option `--default-event <id>`: event used for the DAQ measurement of variables without a fixed event, such as global variables.
- Captured local variables of the event triggers are registered from the capture structs (`cap__<event>`).
- New command line option elf-unit-limit-min, to reduce large logs and unwanted A2L entries.
- Improved warnings for clangs non frame based based locals.
- The addressing mode signature of the target (`XCPLITE__<signature>`) is read from the ELF symbol table instead of the DWARF variables, so it is found when the debug information of the XCPlite library is not parsed or present.
- The compilers which built the ELF file are logged.
- Metadata markers (`XCP_COMMENT`, `XCP_UNIT`, `XCP_LIMITS`, `XCP_READ_WRITE`) of local variables in C compilation units built with GCC are applied again.
- A metadata marker at file scope is no longer applied to the local variables of the same name in functions.
- Functions with an event trigger which the compiler inlined are detected (abstract instance, inlined copies and the out of line copy) and stack frame relative variables are not registered.
- DWARF 5 location lists referenced by index (`DW_FORM_loclistx`, clang) are evaluated like the location lists of DWARF 4.
- A location list is accepted only if all its entries describe the same memory location, which replaces the previous unsafe heuristic approach.
- The relative addresses of local variables are now the offsets from the DWARF frame base without correction, the call frame information parser is removed. The trigger macros pass a matching base to the target: `xcp_get_frame_addr()` in `xcplib.h` is `__builtin_dwarf_cfa()` under GCC (CFA on every architecture) and `__builtin_frame_address(0)` under clang (frame pointer register).


## [3.0.10] - XCPlite V2.1.12, V2.1.14 (2026-09-06)

- Improved variable registration for global and static variables.
- Global variables and local static variables in functions without event trigger are registered, but not associated to any specific default DAQ event.
- Unique typedef names for struct/class types with the same name in different namespaces, classes or functions (`motor_control.Input`, `valve_control.Input`). Previously all variables of such types referenced the first registered typedef and were shown with the wrong type.
- Global variables with the same name in different namespaces are registered with namespace qualified names (`motor_control.input`, `valve_control.input`) instead of dropping all but the first one.
- A struct type which is used for measurement and for calibration variables gets separate typedefs.
- The GCC declaration and definition entries of a namespace scope variable are merged, they no longer produce duplicate instance errors.
- Variables with internal linkage in a namespace and without DWARF location (e.g. metadata markers `XCP_COMMENT(motor_control__input, ...)` inside a namespace) are resolved by their mangled symbol name.
- Metadata annotations placed in the same namespace or function as the variable no longer need a scope prefix (`XCP_COMMENT(input, ...)` in namespace `motor_control` annotates `motor_control.input`), explicit prefixes keep working. Metadata for the typedef fields of namespace qualified instances.
- Local variables without a DWARF location (optimized away) are no longer resolved to the address of a global variable with the same name, only symbols with local binding are considered for them.
- Documentation: new `docs/OFFLINE_A2L.md` for the offline A2L generation with xcpclient (workflow, naming rules, supported types, diagnostics), README updated.


## [3.0.9] - XCPlite V2.1.9, V2.1.11 (2026-09-03)

V2.1.9 was not released on its own, its changes were released with V2.1.11.

Changes of XCPlite V2.1.9:

- Support for the new `XCP_READ_WRITE` metadata macro.
- Added enum conversion support in ELF->A2L converter.
- Updated `mc_registry`, `gimli` and `object` crate dependencies to latest versions.
- New options `--elf_skip_no_metadata` and `--config <xcpclient.toml>`.
- Various bugfixes in debuginfo parsing and `no_a2l_demo` event handling.

Changes of XCPlite V2.1.11:

- EPK validation — A2L file EPK is now compared against the EPK reported by the target; mismatch prints a warning and aborts unless overridden with `--yes`.
- Dependency on `mc_registry` switched to public `RainerZ/xcp-lite` 3.0.9.
- Improved error message when AML file is not found.
- Uses ELF section boundary variables as fallback event detection when `xcp_evts` section is absent.
- Fixed ESP32 image containing multiple DROM segments.
- Handle C++ class types and fix empty typedefs for template structs in offline A2L generation.
- The DWARF type reader was merged up to a2ltool 3.4.1.
- Unique names for field typedefs in the A2L writer (`xcp_registry` update).
- Fixed the size of pointer members in structs and classes.
- No panic on ELF files without `xcp_evts` section or with duplicate event markers.
- README section on supported types and limitations, unit tests with a C++ type fixture.
- Fixed free_rtos_esp32_demo linker map section based event detection when dynamic event management is disabled.


## [3.0.1] - XCPlite V2.1.6 (2026-07-08)

- Refactoring and bugfixes.
- `xcp_registry` dependency switched back to the `xcplite` crate main on VectorGrp.
- Support for the `XCP_LIMITS`, `XCP_COMMENT` and `XCP_UNIT` metadata in the `xcp_meta` section for A2L generation from ELF files.
- Link-time, deterministic event ids from the `tXcpEventDescriptor` constants in the section `xcp_evts`.


## [2.1.3] - XCPlite V2.1.3, V2.1.5 (2026-06-18)

The version was increased to 2.1.3 with XCPlite V2.1.5, XCPlite V2.1.3 was released with version 2.1.0.

- xcpclient only uses the registry (xcp-lite `xcp_registry` crate), not the full Rust xcp-lite stack.


## [2.1.0] - XCPlite V2.1.0 (2026-06-05)

- Improved support for offline A2L creation from ELF files.
- The A2L generator can create an A2L file template with XCP events and segments from the ELF file only, by inspecting the event and segment descriptor memory sections.
- Upload of the ELF file instead of the A2L file (optional custom `GET_ID` of the target).


## [2.0.0] - XCPlite V2.0.1 to V2.0.4 (2026-04-09)

- New xcpclient tool in Rust, for testing and for offline A2L generation.
