# xcpclient

XCP test client and A2L generator for XCPlite, implemented in Rust

XCP client v4.x for testing XCP servers and managing A2L and HEX files. Changes are documented in [CHANGELOG.md](CHANGELOG.md).


## Scope

xcpclient has two purposes:

- **Testing xcplib**: integration tests, test sequences, measurement and calibration from the command line and in scripts.
- **Generating A2L files** for XCPlite applications: upload from the target or creation from ELF/DWARF debug information.

What it is not:

- It is not a general purpose XCP client. It is a partial XCP implementation with protocol settings hard-coded for XCPlite, and it does not promise to work with other XCP implementations.
- It is not a replacement for a full featured measurement and calibration tool like CANape.


## Features

This tool can:
- Connect to XCP on Ethernet servers via TCP or UDP and show information about the XCP protocol and the target ECU
- Upload A2L or ELF files from XCP servers (GET_ID command)
- Create A2L files from ELF/DWARF debug information including event and memory segment information obtained from the XCP server or from an ELF file
- Create A2L file templates for from a XCPlite ELF/DWARF
- Fix A2L files with event and memory segment information from the XCP server
- Read and write calibration variables (CAL)
- Upload (from target) and download (to target) binary files (Intel-HEX) with calibration segment data
- List available measurement variables and parameters with regex patterns
- Test data acquisition (DAQ)
- Execute test sequences
- Check the EPK of the A2L file against the EPK reported by the target, abort on mismatch unless `--yes` is given
- Load options from a TOML configuration file (`--config`, see `xcpclient.toml`)
- Run as MCP server for AI assistants (`--mcp`), see [MCP server mode](#mcp-server-mode)


## Offline A2L generation

xcpclient contains the XCPlite specific ELF/DWARF to A2L generator: `xcpclient --create-a2l --elf <binary>` reads the markers the
XCPlite instrumentation macros leave in the ELF file (events, calibration segments, trigger points, metadata) and the DWARF debug
information, and writes a complete A2L file without any runtime A2L code in the application. The workflow, the rules for the
application code, the naming of types and variables, the supported types and the diagnostics are described in
[docs/OFFLINE_A2L.md](../../docs/OFFLINE_A2L.md), the markers in [docs/TECHNICAL.md](../../docs/TECHNICAL.md#instrumentation-markers-for-offline-a2l-tools).

The generator reads ELF files with DWARF debug information (Linux, QNX, embedded targets). macOS is not supported: executables built
on macOS (Mach-O) contain no DWARF debug information, the macOS linker leaves it in the object files and in the `.dSYM` bundle.
xcpclient rejects Mach-O files with an error message, generate the A2L file from a Linux build of the application instead.

xcpclient exits with status 1 on any error (connection failed, ELF or A2L file not found or not usable, test failed), so scripts can
detect failures by the exit status.


## Usage

```text
XCP client v4.3.0 for testing XCP servers and managing A2L and HEX files

Usage: xcpclient [OPTIONS]

Options:
      --config <FILE>      Load arguments from a TOML config file
      --log-level <LEVEL>  Program flow log level (Off=0, Error=1, Warn=2, Info=3, Debug=4, Trace=5) [default: 3]
      --verbose <LEVEL>    ELF parsing log verbosity level [default: 0]
  -y, --yes                Automatically confirm EPK mismatch and other safety warnings
  -h, --help               Print help (see more with '--help')
  -V, --version            Print version

Connection:
      --dest-addr <ADDR>       XCP server address (IP address or IP:port) [default: 127.0.0.1]
      --port <PORT>            XCP server port number, used when --dest-addr has no port [default: 5555]
      --bind-addr <ADDR>       Local bind address (IP address or IP:port) [default: 0.0.0.0]
      --baud-rate <BAUD_RATE>  Baud rate for XCP communication (only applicable for certain protocols) [default: 115200]
      --tcp                    Use TCP for XCP communication
      --udp                    Use UDP for XCP communication
      --sxi                    Use SxI for XCP communication
      --connect-mode <MODE>    XCP connect mode [default: 0]
      --offline                Force offline mode, no network communication

A2L file:
      --a2l <FILE>           Name of the A2L file
      --upload-a2l           Upload the A2L file from the XCP server
      --create-a2l           Create an A2L file from XCP server information and, with --elf or --upload-elf, from ELF
                             debug information
      --create-a2l-template  Create a minimal A2L template from XCP server event and memory segment information only
      --fix-a2l              Correct event ids and calibration segment numbers of the given A2L file with XCP server
                             information
      --embed-aml            Embed the XCP A2ML description into a created A2L file, instead of including XCP_104.aml

ELF file:
      --upload-elf               Upload the ELF file from the XCP server
      --elf <FILE>               Create an A2L file from the debug information of this ELF file
      --elf-unit-limit <N>       Parse only compilation units <= n
      --elf-unit-limit-min <N>   Parse only compilation units >= n [default: 0]
      --elf-var-filter <REGEX>   Register only ELF variables whose names match this regex
      --elf-skip-no-metadata     Skip ELF variables without metadata (XCP_UNIT, XCP_LIMITS, XCP_COMMENT)
      --elf-unit-filter <REGEX>  Register only ELF variables from compilation units (source files) whose names match
                                 this regex

Calibration:
      --bin <FILE>          Pathname of the parameter file (.bin or Intel-HEX .hex) for --upload-bin and --download-bin
      --upload-bin          Upload the working page data of all calibration segments into a parameter file
      --download-bin        Download the working page data of all calibration segments from a parameter file to the
                            target
      --list-cal <REGEX>    List the calibration variables in the A2L file matching this regex
      --cal <NAME> <VALUE>  Set a calibration variable to a value

Measurement:
      --list-mea <REGEX>         List the measurement variables in the A2L file matching this regex
      --mea <NAME>...            Measure these variables with DAQ, names separated by space or regular expressions (e.g.
                                 ".*")
      --default-event <ID|NAME>  Event for variables without a fixed event, given by event id or event name
      --time <S>                 Measurement duration in s, 0 means infinite [default: 0]

Measurement output files:
      --csv <FILE>          Save measurement data to a CSV file instead of printing it to the console
      --mdf <FILE>          Save measurement data to an ASAM MDF 4.1 file
      --mcap <FILE>         Save measurement data to an MCAP file (https://mcap.dev)
      --perfetto <FILE>     Save measurement data to a trace file for Perfetto (https://ui.perfetto.dev)
      --trace-auto <LEVEL>  Trace signals added to the measurement with --perfetto [default: links] [possible values:
                            off, context, links, all]

Test:
      --test  Execute a test sequence on the XCP server

MCP server:
      --mcp            Run as MCP server on stdin/stdout, offering the xcpclient functions as tools to an AI assistant
      --mcp-read-only  Do not offer the tools which change the target (writing calibration parameters)
      --mcp-dir <DIR>  Directory for the files the MCP server writes on its own
```

```text
XCP client v4.3.0 for testing XCP servers and managing A2L and HEX files

Usage: xcpclient [OPTIONS]

Options:
      --config <FILE>
          Load arguments from a TOML config file
          
          Command-line arguments take precedence.

      --log-level <LEVEL>
          Program flow log level (Off=0, Error=1, Warn=2, Info=3, Debug=4, Trace=5)
          
          [default: 3]

      --verbose <LEVEL>
          ELF parsing log verbosity level
          
          [default: 0]

  -y, --yes
          Automatically confirm EPK mismatch and other safety warnings
          
          Use in scripts to suppress interactive prompts.

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version

Connection:
      --dest-addr <ADDR>
          XCP server address (IP address or IP:port)
          
          If the port is omitted, --port is used.
          
          [default: 127.0.0.1]

      --port <PORT>
          XCP server port number, used when --dest-addr has no port
          
          [default: 5555]

      --bind-addr <ADDR>
          Local bind address (IP address or IP:port)
          
          If the port is omitted, the system assigns an available port.
          
          [default: 0.0.0.0]

      --baud-rate <BAUD_RATE>
          Baud rate for XCP communication (only applicable for certain protocols)
          
          [default: 115200]

      --tcp
          Use TCP for XCP communication

      --udp
          Use UDP for XCP communication

      --sxi
          Use SxI for XCP communication

      --connect-mode <MODE>
          XCP connect mode
          
          [default: 0]

      --offline
          Force offline mode, no network communication
          
          The communication parameters are used to create the A2L file.

A2L file:
      --a2l <FILE>
          Name of the A2L file
          
          If not specified, the A2L file name is read from the XCP server.

      --upload-a2l
          Upload the A2L file from the XCP server
          
          Requires that the XCP server supports GET_ID A2L upload.

      --create-a2l
          Create an A2L file from XCP server information and, with --elf or --upload-elf, from ELF debug information
          
          Events and memory segments are read from the XCP server, all visible measurement and calibration variables are
          inserted from the ELF file if specified with --elf or --upload-elf. Requires that the XCP server supports the
          GET_EVENT_INFO and GET_SEGMENT_INFO commands.

      --create-a2l-template
          Create a minimal A2L template from XCP server event and memory segment information only
          
          No variables or types are registered, the result is a skeleton A2L file. Requires that the XCP server supports
          the GET_EVENT_INFO and GET_SEGMENT_INFO commands.

      --fix-a2l
          Correct event ids and calibration segment numbers of the given A2L file with XCP server information
          
          The A2L file is rewritten, the original is kept as <file>.a2l.bak. Only for A2L files created by xcpclient
          (--elf and --create-a2l), typically generated offline without section registration. Requires that the XCP
          server supports the GET_EVENT_INFO and GET_SEGMENT_INFO commands.

      --embed-aml
          Embed the XCP A2ML description into a created A2L file, instead of including XCP_104.aml
          
          Applies to A2L files created by xcpclient (--elf, --create-a2l, --create-a2l-template). The A2L file is
          self-contained, XCP_104.aml is not needed next to it. --fix-a2l keeps an embedded A2ML description.

ELF file:
      --upload-elf
          Upload the ELF file from the XCP server
          
          Requires that the XCP server supports the proprietary GET_ID ELF upload command.

      --elf <FILE>
          Create an A2L file from the debug information of this ELF file
          
          If connected to an XCP server, events and memory segments are read from the XCP server. ELF files with DWARF
          debug information only (Linux, QNX, embedded targets), macOS Mach-O executables are not supported.

      --elf-unit-limit <N>
          Parse only compilation units <= n

      --elf-unit-limit-min <N>
          Parse only compilation units >= n
          
          [default: 0]

      --elf-var-filter <REGEX>
          Register only ELF variables whose names match this regex
          
          If not specified (or empty), all variables are registered.

      --elf-skip-no-metadata
          Skip ELF variables without metadata (XCP_UNIT, XCP_LIMITS, XCP_COMMENT)
          
          Only variables that have at least one metadata annotation are included in the A2L output.

      --elf-unit-filter <REGEX>
          Register only ELF variables from compilation units (source files) whose names match this regex
          
          If not specified (or empty), variables from all compilation units are registered.

Calibration:
      --bin <FILE>
          Pathname of the parameter file (.bin or Intel-HEX .hex) for --upload-bin and --download-bin

      --upload-bin
          Upload the working page data of all calibration segments into a parameter file
          
          Requires that the XCP server supports GET_ID A2L upload. A .bin file is an XCPlite persistence file with the
          event list and the context list (conv.contexts in the A2L file), a .hex file contains the segment data only.

      --download-bin
          Download the working page data of all calibration segments from a parameter file to the target

      --list-cal <REGEX>
          List the calibration variables in the A2L file matching this regex

      --cal <NAME> <VALUE>
          Set a calibration variable to a value

Measurement:
      --list-mea <REGEX>
          List the measurement variables in the A2L file matching this regex

      --mea <NAME>...
          Measure these variables with DAQ, names separated by space or regular expressions (e.g. ".*")

      --default-event <ID|NAME>
          Event for variables without a fixed event, given by event id or event name
          
          Applies to global variables and static variables in functions without an event trigger. Used for their DAQ
          measurement and assigned to them as default event when an A2L file is created from an ELF file. An event name
          is looked up in the event list (from the XCP server, the ELF file or the A2L file), xcpclient aborts when it
          is not found. If not specified, such variables get no event and can not be measured with xcpclient.

      --time <S>
          Measurement duration in s, 0 means infinite
          
          [default: 0]

Measurement output files:
      --csv <FILE>
          Save measurement data to a CSV file instead of printing it to the console
          
          CSV format: time_ns,daq,name,value,unit (one row per measurement sample). Values are physical values (factor
          and offset applied), the value of an enum is its label.

      --mdf <FILE>
          Save measurement data to an ASAM MDF 4.1 file
          
          One channel group per DAQ list, the payload of the DAQ packets is stored as received. Values are raw values,
          the MDF file contains the conversions (factor and offset, enum labels) and units. May be combined with --csv,
          --mcap and --perfetto.

      --mcap <FILE>
          Save measurement data to an MCAP file (https://mcap.dev)
          
          One channel per DAQ list, the topic is the event name, the payload of the DAQ packets is stored as received,
          as ros1 messages. Values are raw values, MCAP has no conversions or units, they are in the comments of the
          message definitions and in the channel metadata. May be combined with --csv, --mdf and --perfetto.

      --perfetto <FILE>
          Save measurement data to a trace file for Perfetto (https://ui.perfetto.dev)
          
          Chrome Trace Event JSON format. Spans (DaqSpan) are slices in a track per context and thread, numeric signals
          are counter tracks, enum signals are state tracks, span links (xcp::LinkSpan) are flows (arrows) from the
          producer span to the consumer span. The span and link signals of the contexts needed for the trace are added
          to the measurement, see --trace-auto. May be combined with --csv, the added signals are not written to the CSV
          file.

      --trace-auto <LEVEL>
          Trace signals added to the measurement with --perfetto
          
          off: none, only hints which signals are missing.
          
          context: the spans of the contexts of the measured signals.
          
          links: also the span links of these contexts and, if there are any, the spans of all contexts as possible
          producers.
          
          all: the spans and span links of all contexts.
          
          The context of a signal is the prefix of its name or of the name of its event (<context>.<name>).
          
          [default: links]
          [possible values: off, context, links, all]

Test:
      --test
          Execute a test sequence on the XCP server
          
          If there is a calibration segment named params (see daq_test, c_demo), a calibration consistency test is
          performed.

MCP server:
      --mcp
          Run as MCP server on stdin/stdout, offering the xcpclient functions as tools to an AI assistant
          
          The MCP host starts xcpclient with this option, xcpclient keeps the connection to the target between the tool
          calls, see docs/MCP.md. --dest-addr, --port, --udp/--tcp, --bind-addr, --a2l, --elf and --default-event are
          the defaults of the connect tool. Log output goes to stderr.

      --mcp-read-only
          Do not offer the tools which change the target (writing calibration parameters)

      --mcp-dir <DIR>
          Directory for the files the MCP server writes on its own
          
          These are the A2L file uploaded from the target, XCP_104.aml and an A2L file created from an ELF file without
          a given name. Default is the cache directory of the user for xcpclient, never the working directory, where an
          XCPlite application started in the same directory writes its A2L file.
```


## MCP server mode

With `--mcp`, xcpclient is a server for the Model Context Protocol (MCP): an AI assistant such as Claude Code operates the target in natural language ("connect to 192.168.0.206", "what is counter_max", "set it to 2000", "record all signals for a minute"). xcpclient keeps the XCP connection between the requests. [docs/MCP.md](../../docs/MCP.md) explains MCP, the tools and their limits.

The MCP host starts xcpclient, it is not started by hand. For Claude Code, in the directory of the project:

```bash
claude mcp add xcp -- xcpclient --mcp --dest-addr 192.168.0.206 --udp
```

Tools:

| Tool | |
|---|---|
| `xcp_status` | State of the connection, target, EPK, A2L, running measurement |
| `xcp_connect`, `xcp_disconnect` | Connect to a target and load its A2L description |
| `xcp_list_measurements`, `xcp_list_parameters` | Find signals and parameters by regular expression |
| `xcp_read` | Current values of parameters and signals |
| `xcp_write` | Set a calibration parameter, not available with `--mcp-read-only` |
| `xcp_sample` | Observe signals for some seconds: count, rate, min, max, mean, last value |
| `xcp_measure_start`, `xcp_measure_stop` | Record signals into a file (MDF, MCAP, CSV, Perfetto) in the background |
| `xcp_load_a2l` | Load or replace the A2L description of the connected target: from an A2L file, created from an ELF file, or uploaded |
| `xcp_create_a2l` | Create an A2L file from an ELF file, offline or with the events and calibration segments of the connected target |
| `xcp_save_parameters` | Save all calibration segments of the target into a `.hex` or `.bin` file |
| `xcp_load_parameters` | Load a `.hex` file into the target, not available with `--mcp-read-only` |

A recording without a given file name is written to the working directory as `<target>_<date>_<time>.mf4`, a parameter file as `<target>_<date>_<time>.hex`. Measurement and parameter files are never overwritten, an A2L file only with `overwrite`. The warnings xcpclient logs while a tool runs (variables of an ELF file which can not be described, an A2L file which differs from the target) are part of the tool result.

The target options (`--dest-addr`, `--port`, `--udp`/`--tcp`, `--a2l`, `--elf`, `--default-event`) are the defaults of `xcp_connect`, the default protocol is UDP. The uploaded A2L file and `XCP_104.aml` are written into a directory of their own (`--mcp-dir`, default `~/Library/Caches/xcpclient` on macOS and `~/.cache/xcpclient` on Linux), so the server can run in the working directory of the application without touching its files. `xcp_status` shows the path of the A2L file. File names in tool arguments (`a2l`, `elf`) are relative to the working directory the host starts xcpclient in. The log output is on stderr.

The scope of xcpclient is the same in this mode, see [Scope](#scope).

## Build and Install

```bash
cd tools/xcpclient
cargo install --path . --locked
```

## Test

```bash
cd tools/xcpclient
cargo test
```

The MCP server mode has end-to-end scenarios against a running target (`build/hello_xcp`) in addition, see [tests/mcp/README.md](tests/mcp/README.md):

```bash
python3 tests/mcp/run_all.py
```


## Examples

### List all calibration or measurement variables

```bash
xcpclient --dest-addr 192.168.0.206 --udp --list-cal .
xcpclient --dest-addr 192.168.0.206 --udp --list-mea .
```

### Set a calibration variable

Set variable counter_max to 1000
```bash
xcpclient --dest-addr 192.168.0.206 --port 5555 --tcp --cal counter_max 1000
```

### Measure variables

Measure everything from uploaded A2L file for 5 seconds with detailed log to terminal:  

```bash
xcpclient --dest-addr=127.0.0.1  --udp --upload-a2l --mea ".*" --time 5 --verbose 2
```

With A2L file given:  

```bash
xcpclient --dest-addr=192.168.0.206  --tcp --a2l hello_xcp.a2l  --mea ".*" 
```

Variables without a fixed event (global variables in an A2L file generated from an ELF file) are measured with the event given by `--default-event`. The event is given by its id or by its name (a C identifier, looked up in the event list of the XCP server, the ELF file or the A2L file, xcpclient aborts if the name is not found). When the A2L file is created from an ELF file, `--default-event` assigns the event to such variables in the A2L file:

```bash
xcpclient --dest-addr=192.168.0.146 --udp --a2l freertos_demo.a2l --default-event 0 --mea global_counter --time 5
xcpclient --dest-addr=192.168.0.146 --udp --a2l freertos_demo.a2l --default-event mainloop --mea global_counter --time 5
```

With ELF file given, creates an A2L file from the ELF file and XCP server information about events and memory segments, then measures the specified variable:  

```bash
xcpclient --udp --elf build/hello_xcp --mea "counter" --verbose 2
```

### Upload the A2L file to be used from the target

```bash
xcpclient --dest-addr=192.168.0.206:5555 --tcp --upload-a2l   
```

### Create an A2L file for a target from ELF without on-target A2L generation support

Create an A2L file from the ELF file (and optional XCP server information about events and memory segments), then save the A2L file as hello_xcp.a2l:
```bash

# offline
xcpclient --offline --elf examples/no_a2l_demo/CANape/no_a2l_demo.elf --a2l examples/no_a2l_demo/CANape/no_a2l_demo_template.a2l --create-a2l-template 
xcpclient --offline --elf examples/no_a2l_demo/CANape/no_a2l_demo.elf --a2l examples/no_a2l_demo/CANape/no_a2l_demo.a2l --create-a2l

# online with XCP server information about events and memory segments
xcpclient --dest-addr=192.168.0.206:5555 --udp  --create-a2l --elf no_a2l_demo.elf --a2l no_a2l_demo.a2l 
```

Upload the ELF file into hello_xcp.elf and create an A2L file from the ELF file and XCP server information about events and memory segments, then save the A2L file as hello_xcp.a2l:
```bash
xcpclient --dest-addr=192.168.0.206:5555 --udp --create-a2l --upload-elf --elf hello_xcp.elf --a2l hello_xcp.a2l 
```

### Upload the current calibration data from the target into an Intel-HEX file

```bash
xcpclient --dest-addr=192.168.0.206:5555 --udp --a2l hello_xcp.a2l --upload-bin --bin hello_xcp.hex
```


#### Demo

```bash
# A2L from the ELF file of no_a2l_demo, offline
cargo r -- --elf no_a2l_demo.elf --log-level 3 --create-a2l --a2l no_a2l_demo.a2l --offline

# A2L with the event and segment information of the running target
cargo r -- --dest-addr 192.168.0.206 --udp --elf no_a2l_demo.elf --log-level 3 --create-a2l --a2l no_a2l_demo.a2l

cargo r -- --dest-addr 192.168.0.206 --udp --elf no_a2l_demo.elf --log-level 3 --create-a2l --a2l no_a2l_demo.a2l --list-mea 'counter'

cargo r -- --dest-addr 192.168.0.206 --udp --elf no_a2l_demo.elf --log-level 3 --create-a2l --a2l no_a2l_demo.a2l --mea 'counter' --time 5 --verbose 2
```




## Measurement limits

The console output shows each sample with the name of its event:

```
EVENT: SigGen1.clip (id=2, daq=2), lost=0, t=3855188843115083ns (+3473us)
 SigGen1.input = -17.514199949800968 Volt
 SigGen1.clipped = -17.514199949800968 Volt
```

Signals of an event which are adjacent in memory (same address extension, no gap between them) are merged and measured with one ODT entry, for example the members of a struct or the elements of an array. This saves DAQ memory and copy operations in the XCP server. Gaps are not bridged, a struct with members which are not measured needs an ODT entry for each run of measured members.

xcpclient uses one DAQ list with one ODT (one DAQ packet) for up to 123 ODT entries of an event. An event with more ODT entries, or with more signal bytes than fit into one DAQ packet, gets as many DAQ lists as needed. The XCP server samples all DAQ lists of an event with the same timestamp. In the console output and in the CSV file such an event appears once per DAQ list, with the same event name and timestamp and a different `daq` number.

A large signal (an array larger than one ODT entry) is measured with a dedicated DAQ list, it is not merged or mixed with other signals. It is split into consecutive ODT entries, in as many ODTs as needed. The packets of the first ODT have the timestamp, the others not. xcpclient reassembles the packets of a sample, the outputs get the complete signal with the timestamp of the first ODT. A sample with a lost ODT (a gap of the transport layer counter, or an ODT out of sequence) is dropped as a whole, the measurement summary shows the number of DAQ packets and of dropped samples.

The ODT entries are written with `WRITE_DAQ_MULTIPLE`, up to (`MAX_CTO` - 2) / 8 = 30 ODT entries per command with the 248 byte `MAX_CTO` of XCPlite. If the XCP server answers `ERR_CMD_UNKNOWN`, xcpclient uses `WRITE_DAQ` for the rest of the session.

A signal can not be split over several DAQ lists. xcpclient reports a signal which is too large as an error and does not start the measurement. The limits are properties of the XCP server:

- The maximum size of an ODT entry (`MAX_ODT_ENTRY_SIZE_DAQ` from `GET_DAQ_RESOLUTION_INFO`). In XCPlite this is `XCP_MAX_ODT_ENTRY_SIZE` (248 bytes), the protocol limit is 255 bytes. It is the limit for merged signals and for the parts of a large signal.
- The payload of a DAQ packet, `MAX_DTO` without the 4 byte header and the 4 byte timestamp (the timestamp only in the first ODT of a DAQ list). In XCPlite this is `XCPTL_MAX_DTO_SIZE - 8`, 1016 bytes by default and 240 bytes with `OPTION_QUEUE_64_FIX_SIZE`. `XCPTL_MAX_DTO_SIZE` can be increased up to `XCPTL_MAX_SEGMENT_SIZE - 8`, jumbo frames with a larger `OPTION_MTU`.
- 251 ODTs per DAQ list: the ODT number is the PID of the DAQ packet, 0xFC..0xFF are reserved. A large signal can have 1016 + 250 * 1020 = 256016 bytes with the default `XCPTL_MAX_DTO_SIZE` of 1024, about 2 MB with 7960 (`OPTION_MTU` 8000).
- The DAQ memory of the XCP server (`OPTION_DAQ_MEM_SIZE` in XCPlite, 12 KiB by default): 12 bytes per DAQ list, 8 per ODT and 6 per ODT entry, a large signal needs about 6 bytes per 248 bytes. A setup which does not fit is rejected by the server with a memory overflow error.

Structs are not affected, they are measured as their basic type members (`points_._0.x`, `parameters.counter_max`).

Measurable are MEASUREMENTs and CHARACTERISTICs with an event (`IF_DATA XCP DAQ_EVENT` with a fixed event or a default event list, as xcplib writes measurement arrays). A characteristic without event, for example a calibration parameter, can not be measured, also not with `--default-event`. `--list-mea` marks characteristics and arrays (`[x x y]`).

An array (`MATRIX_DIM`) is measured as one signal with all its elements, so it has to fit into one DAQ list (see above), larger arrays are skipped with a warning. The elements are in memory order, x varies fastest (a C array `[y][x]` has `MATRIX_DIM x y`):

- Console: the first 16 elements, `curve = [0 1 2 ...]`
- CSV: arrays are skipped
- MDF: one channel of the element type with a CA block (fixed size array), the dimensions are row oriented (last varies fastest), so `MATRIX_DIM x y` is `[y, x]` in the MDF file
- MCAP: a fixed size ros1 array of the elements, `float32[32] matrix_f32`, the dimensions are in the channel metadata (`matrix_f32.dim` = `8x4`)
- Perfetto: arrays are skipped

### Lost packets and UDP receive buffer

The XCP server numbers its messages with a 16 bit transport layer counter. xcpclient detects lost DAQ packets from the gaps of this counter and reports them every second and at the end of the measurement. A packet is lost when the transmit queue of the XCP server overflows (XCPlite counts these packets in the counter, it prints `Transmit queue overflow`) or when the UDP receive buffer of xcpclient is full and the system drops a whole datagram (up to about 75 packets with MTU 1500, about 400 with MTU 8000). A gap of 65536 or more packets can not be detected.

xcpclient requests an 8 MB UDP receive buffer and logs the size it got. The system default is too small for high data rates (Linux 208 KB, macOS 768 KB). On Linux the size is limited by `net.core.rmem_max` (default 208 KB, xcpclient then gets 416 KB, Linux reports twice the set size) and xcpclient warns. To allow 8 MB:

```bash
sudo sysctl -w net.core.rmem_max=8388608
```

Add `net.core.rmem_max=8388608` to `/etc/sysctl.conf` to keep it after a reboot.



### DAQ memory

The number of signals which can be measured at the same time is limited by the DAQ memory of the XCP server. XCPlite needs 6 bytes per ODT entry and 20 bytes per DAQ list of `OPTION_DAQ_MEM_SIZE` (default 12288 bytes, about 2000 signals when no signals can be merged). The server rejects a larger measurement with a memory overflow error. xcpclient uses at most 1024 DAQ lists.



## Save measurement data to MDF4

`--mdf <file>` writes the measurement to an ASAM MDF 4.1 file, which can be opened with CANape, asammdf and other MDF tools.

```bash
xcpclient --udp --mea '.*' --time 5 --mdf measurement.mf4
```

The payload of the DAQ packets is stored as received, without decoding, the MDF metadata describes its binary layout. Each DAQ list is a channel group with the event name as acquisition name and the ECU name as source. A record is the DAQ list number (2 byte record id), the timestamp (64 bit, ticks since the measurement start, the master channel `time` in s) and the payload of the DAQ packet. The signals are raw values with their conversion: factor and offset as linear conversion, enums as value to text conversion, and the unit.

The file is unsorted (all channel groups in one data group), MDF tools sort it when they open it. It is finalized when the measurement stops. If xcpclient is terminated without stopping the measurement, the file stays unfinalized (`UnFinMF`), MDF tools can repair it.

## Save measurement data to MCAP

`--mcap <file>` writes the measurement to an [MCAP](https://mcap.dev) file, the container format of Foxglove, Lichtblick and ROS 2 tools.

```bash
xcpclient --udp --mea '.*' --time 5 --mcap measurement.mcap
```

MCAP stores timestamped messages, a schema describes their format. Each DAQ list is a channel, its topic is the event name (`/Mainloop`). A message is the payload of a DAQ packet as received, `log_time` is the DAQ timestamp in ns. The schema is a ROS 1 message definition (`ros1msg`): ros1 serializes packed and little endian, so the definition describes the binary layout of the payload exactly, gaps are `uint8[n]` padding fields. The field names are the signal names with `.` and other characters which are not allowed replaced by `_` (`Mainloop.value1` is the field `Mainloop_value1`).

MCAP has no conversions and units, the values are raw values. Factor and offset, enum labels and the unit of each signal are in the comments of the message definition and in the channel metadata (`<field>.factor`, `<field>.offset`, `<field>.labels`, `<field>.unit`, `<field>.name` is the signal name). A metadata record `xcp` contains the ECU name and the timestamp resolution.

The messages are written in zstd compressed chunks of 1 MB, the index and statistics are written when the measurement stops. If xcpclient is terminated without stopping the measurement, the last chunk is lost and the file has no index, `mcap recover` from the [MCAP CLI](https://mcap.dev/guides/cli) repairs it.

## Export spans and signals to Perfetto

`--perfetto <file>` writes the measurement as a trace file in the Chrome Trace Event JSON format. Open it with <https://ui.perfetto.dev> ("Open trace file"), the file is processed locally in the browser.

```bash
xcpclient --udp --mea '.*' --time 5 --perfetto trace.json
xcpclient --udp --mea 'Mainloop.channel1' --time 5 --perfetto trace.json
```

The second command measures one signal, xcpclient adds the span and link signals needed for the trace, see "Trace signals" below.

| In Perfetto | From XCP |
|---|---|
| Process | A context (`xcp::Context`, for example `SigGen1`). Signals which do not belong to a context are in a process named after the ECU |
| Thread track | A thread which executed spans in the context (`thread 1`, `thread 2`, ...), from the signal `<context>.thread` |
| Slice | A span (`DaqSpan`), nested spans are stacked |
| Counter track | A numeric signal as physical value, with its unit |
| State track | An enum signal, one slice per state, named with the enum label |
| Flow (arrow) | A span link (`xcp::LinkSpan`), from the producer span to the consumer span |

- A context is found in the A2L file by its signals `<context>.span`, `.level` and `.span_dt`. A signal belongs to the context which is the prefix of its name or, if none, of the name of its event (`<context>.<name>`). Each context is a process, also when its spans are not measured.
- `<context>.thread` needs `OPTION_DAQ_SPAN_THREAD` on the target (not defined by default). Without it, there is one track `spans` per context. This is correct as long as a context is executed by one thread at a time, otherwise spans are cut and xcpclient prints a warning with the number of samples which did not fit.
- Lost samples and a measurement which starts inside a span are handled: the open spans are resynchronised to the nesting level of the next sample, a span the begin of which was not seen is named `(unknown)`. Spans which are still open at the end of the measurement end at the last sample of their track.
- A span link is a sample of the event `<context>.links` of the consumer context, with the signals `<context>.link_context` (enum `conv.contexts`), `.link_time` and, if available, `.link_span`, `.link_thread` and `.link_consumer_thread`. It is written as a flow in format v1: `"ph":"s"` on the track of the producer context at the producer timestamp, and `"ph":"f"` with `"bp":"e"` on the track of the consumer at the time of the sample. Perfetto binds each end to the innermost slice which encloses its timestamp. The span signals of the producer context must be measured as well (added with `--trace-auto links`), links which can not be resolved (producer context not measured, captured before the measurement start) are counted and reported. In the Perfetto UI, click a slice to see its arrows.
- Timestamps are relative to the start of the measurement, with ns resolution.
- A span is written when it ends, so the file is not sorted by time. Perfetto sorts on import.

See `examples/thread_context_demo` for contexts and spans.

### Trace signals

With `--perfetto`, it is enough to select the signals of interest with `--mea`. The span and link signals of the contexts are added to the measurement, depending on `--trace-auto`:

| `--trace-auto` | Added |
|---|---|
| `off` | Nothing. Warns which signals are missing for spans and arrows |
| `context` | The spans of the contexts of the selected signals |
| `links` (default) | Also the span links of these contexts and, if there are any, the spans of all contexts. The producer of a span link is only known at runtime, so every context is a possible producer |
| `all` | The spans and span links of all contexts |

The added signals are listed in the log and are not written to the CSV file. The spans of a context with a fast task add data rate, in `thread_context_demo` the spans of `SigGen1` (three nested spans every 25 µs) are most of the measured data. Use `--trace-auto context` to keep them out, the arrows from these contexts are then counted as not resolved.



## Notes

- The CSV file has the columns `time_ns,daq,name,value,unit`, the values are physical values or enum labels.
- At least one sample of any event is needed every 2.147 s (with 1 ns ticks), otherwise the 64 bit
  timestamp is ambiguous.
- Maybe add a `--raw` switch in xcpclient for unconverted values in console and CSV.
