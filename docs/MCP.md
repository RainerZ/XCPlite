# xcpclient MCP Server

**Status: phases 1, 2 and 4 are implemented (see [8.5](#85-phases)): `xcpclient --mcp` on stdio with all tools of [section 5](#5-tools). The HTTP daemon (phase 3) is postponed, it is specified here and not implemented.**

The facts about the MCP specification, the Rust SDK and the client applications were looked up on 2026-11-28, the sources are linked in [section 11](#11-sources). MCP changes quickly, check them again before implementing.

## 1. Summary

`xcpclient --mcp` turns xcpclient into a server which an AI assistant (Claude Code, Claude Desktop and others) can operate. The assistant gets a small set of functions: connect, list signals, read and write parameters, measure. xcpclient keeps the XCP connection open between the calls, so a conversation looks like this:

> **You:** Connect to target 192.168.0.206 at port 5555
> **Assistant:** Connected to `hello_xcp` (UDP), EPK `v10_21_15_02`. A2L uploaded: 3 events, 12 signals, 3 parameters.
> **You:** What is counter_max? Set it to 2000 and check that counter now runs up to 2000.
> **Assistant:** `counter_max` was 1000, now 2000. I observed `counter` for 3 s: min 0, max 1999.

The server contains no AI. It executes function calls, exactly like the command line does. Understanding the sentence and choosing the function is done by the language model in the assistant application.

### Scope

The MCP server does not change what xcpclient is (see `tools/xcpclient/README.md`): its major use cases are testing xcplib and generating A2L files. It does not promise to work with other XCP implementations, and it is not a replacement for a full featured measurement and calibration tool. For this specification this means:

- The tool set stays small. It covers what the command line covers, not what CANape covers: no curve and map editors, no parameter set management, no measurement configurations, no data analysis.
- The XCP server on the other side is XCPlite. Robustness means: an unexpected response or an unsupported XCP server gives an error message and a clean session state, not that xcpclient works around it.

## 2. MCP in a nutshell

### 2.1 The problem

A language model (LLM) produces text. To do something in the real world it needs *tools*: functions it can ask the surrounding application to call. The model is told which tools exist, with a name, a description and the parameters of each. When it decides that a tool helps to answer the user, it does not write an answer but a structured request "call tool X with these arguments". The application executes the call and gives the result back to the model as text. The model reads it and continues: with the answer to the user, or with the next tool call.

Each application once had its own format for such tools. The **Model Context Protocol (MCP)** is the open standard which replaced them: a tool provider is written once as an *MCP server* and works with every application which speaks MCP. The protocol was published by Anthropic in November 2024 and is developed as an open specification at [modelcontextprotocol.io](https://modelcontextprotocol.io).

### 2.2 The three roles

```
+---------------------------- Host ----------------------------+
|  Claude Code, Claude Desktop, an IDE, ...                     |
|                                                               |
|   User  <-->  LLM  <-->  MCP client  ------- MCP -------->  MCP server  -- XCP -->  Target
|                          (one per server)                   (xcpclient)            (ECU)
+---------------------------------------------------------------+
```

- The **host** is the application the user talks to. It runs the conversation with the LLM.
- The **MCP client** is the part of the host which talks to one MCP server. A host with five servers has five clients. You never write a client, it is part of the host.
- The **MCP server** offers the tools. This is what xcpclient becomes.

### 2.3 What happens when you say "Connect to target 192.168.0.206 at port 5555"

1. When the host starts, its MCP client asks the server which tools it has (`tools/list`). xcpclient answers with a list. One entry:

   ```json
   {
     "name": "xcp_connect",
     "description": "Connect to an XCP target (ECU) and load its A2L description. ...",
     "inputSchema": {
       "type": "object",
       "properties": {
         "host":     { "type": "string",  "description": "IP address of the target" },
         "port":     { "type": "integer", "description": "XCP port, default 5555" },
         "protocol": { "type": "string",  "enum": ["udp", "tcp"] }
       },
       "required": ["host"]
     }
   }
   ```

2. The host gives this list to the LLM together with your sentence.
3. The LLM recognizes that `xcp_connect` fits and produces a tool call: `xcp_connect {"host": "192.168.0.206", "port": 5555}`.
4. The host asks you for permission (depending on its settings) and sends the call to the server (`tools/call`).
5. xcpclient connects, uploads the A2L file and returns a text: `Connected to hello_xcp at 192.168.0.206:5555 (UDP) ...`.
6. The LLM reads the result and writes the answer you see.

Two consequences for the design of an MCP server:

- **The descriptions are the user interface.** The model has nothing but the tool names, descriptions and parameter schemas to decide what to call. A vague description leads to wrong calls, the same as a misleading command line help.
- **Results and error messages are read by the model.** A good error says what to do next: `Not connected. Call xcp_connect first.` The model then does that on its own.

### 2.4 On the wire

MCP messages are [JSON-RPC 2.0](https://www.jsonrpc.org/specification): a request has a `method`, `params` and an `id`, the response carries the same `id` and a `result` or an `error`. A notification is a message without `id`, it gets no response. The tool call from above:

```json
{ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
  "params": { "name": "xcp_connect", "arguments": { "host": "192.168.0.206", "port": 5555 } } }
```

```json
{ "jsonrpc": "2.0", "id": 7,
  "result": { "resultType": "complete", "isError": false,
              "content": [ { "type": "text", "text": "Connected to hello_xcp at 192.168.0.206:5555 (UDP) ..." } ] } }
```

(Simplified: a request also carries the protocol version and the capabilities of the client in `params._meta`.)

There are two kinds of errors, and the difference matters:

| Error | Reported as | Example | Who handles it |
|---|---|---|---|
| Tool execution error | A normal result with `"isError": true` and a text | `Target does not respond`, `counter_max: value 70000 exceeds the limit 65535` | The LLM reads it and reacts |
| Protocol error | JSON-RPC `error` | Unknown tool, missing required argument | The host, it is a bug in the client or server |

### 2.5 MCP seen from XCP

MCP solves for an LLM the problem which XCP and A2L solve for a calibration tool. The concepts map well:

| MCP | XCP |
|---|---|
| Host with LLM and user | CANape with the user in front of it |
| MCP server | XCP server in the ECU |
| `server/discover` (older revisions: `initialize`): versions and capabilities | `CONNECT`, `GET_COMM_MODE_INFO` |
| `tools/list`: what exists and how it is called | The A2L file |
| `tools/call` request and response | Command (CTO) and response |
| Notifications, for example progress | `EV` and `SERV` packets |
| Transports: stdio, Streamable HTTP | Transport layers: Ethernet, CAN, SxI |

The difference: the XCP client is a deterministic program, the MCP "client brain" is a language model which chooses its commands by reading descriptions. It can call a tool with a wrong name it guessed, call tools in an unexpected order, or try again after an error. The server has to be robust against all of this, and the tool set has to be small and unambiguous.

### 2.6 What a server can offer

| Primitive | Controlled by | What it is | Used by xcpclient |
|---|---|---|---|
| **Tools** | The model | Functions with arguments and a result | Yes, everything in the first version |
| **Resources** | The application or the user | Data the host can attach to the conversation, addressed by a URI, for example a file | Later, maybe: the A2L file, the last measurement summary |
| **Prompts** | The user | Prepared prompt templates, shown as slash commands in some hosts | No |

Additional mechanisms used here:

- **Progress notifications**: a long running tool call reports progress (`notifications/progress`). Hosts show it and reset their timeouts.
- **Elicitation**: the server asks the user a question in the middle of a tool call, for example "EPK mismatch, continue anyway?". Not every host supports it, so the server needs a fallback, see [section 7](#7-safety-and-security).
- **Tool annotations**: hints per tool such as `readOnlyHint` and `destructiveHint`. Hosts use them to decide whether to ask the user for confirmation. They are hints, the default of `destructiveHint` is `true`.
- **Server instructions**: a short text the server gives to the host, which becomes part of what the model knows about the server. The manual for the model, see [section 5.1](#51-server-instructions).

### 2.7 Transports, and what "running in the background" means

MCP defines two transports. They differ in who starts the server and how long it lives:

| | stdio | Streamable HTTP |
|---|---|---|
| Who starts the server | The host, as a child process | You (terminal, launchd, systemd), independent of any host |
| Connection | JSON-RPC messages as lines on stdin and stdout of the process | HTTP POST to one URL, for example `http://127.0.0.1:8555/mcp` |
| Lifetime | From the start of the host session to its end | Until you stop it |
| Number of hosts | Exactly one | Any number, at the same time |
| The XCP connection lives | As long as the host session | Across host sessions |
| Setup | One line of host configuration | Start the server, then one line of host configuration |
| Rule to obey | **Nothing but MCP messages on stdout**, logging goes to stderr | Bind to 127.0.0.1, check the `Origin` header |

With stdio the server already runs in the background: you never start it by hand, the host does, and it stays alive during the whole conversation. The XCP connection is opened by the first `xcp_connect` and stays open for all following tool calls. It ends when you close the host session.

With Streamable HTTP xcpclient is a daemon. You can close Claude Code, open it again tomorrow, and the connection to the target is still there, a measurement started yesterday is still recording. Several hosts (Claude Code in two terminals, an IDE) see the same connection.

(A third, older transport "HTTP+SSE" is deprecated and not used here.)

### 2.8 Stateless protocol, stateful server

Until revision 2025-11-25, an MCP connection started with a handshake (`initialize`) and HTTP connections had a session id. The current revision **2026-07-28** removed both: every request is self-contained, there is no protocol session. A server which needs state across calls must own it itself.

For xcpclient this fits well, and it is the central design point of this specification: **the XCP connection is state of the xcpclient process, not of an MCP session.** There is exactly one XCP session per xcpclient process. Whoever calls a tool, in whatever order and from whichever host, talks about this one session. This is what "connects only once" means.

Hosts older than 2026-07-28 still use the handshake. The Rust SDK handles both, the server code does not differ.

### 2.9 Limits to know

- **The server cannot speak first.** It only answers tool calls. A measurement running in the background cannot tell the model "signal X just exceeded 5 V". The model sees it when it calls `xcp_status` or `xcp_read` the next time.
- **Tool results cost context.** Every result is read by the model and fills its context window. Claude Code warns above 10 000 tokens per result and cuts at 25 000 (`MAX_MCP_OUTPUT_TOKENS`). So: no measurement data through MCP, measurement data goes to files (MDF, MCAP, CSV, Perfetto), the model gets a summary. Lists are limited and paged.
- **Tool calls should be short.** Hosts have timeouts (Claude Code: 60 s to the first byte for HTTP servers, 5 min without progress for HTTP and 30 min for stdio). A long measurement is therefore two calls, start and stop, not one call which blocks.
- **The model can be wrong.** It may call a tool the user did not ask for. Tools which change something in the target are separated from the read-only tools, so the host can ask for confirmation for exactly these.

## 3. Requirements

Given:

| | Requirement |
|---|---|
| R1 | xcpclient can run in the background |
| R2 | It maintains a connection state and connects only once to a target |
| R3 | When it is connected over MCP, the target is operated in natural language, for example "Connect to target 192.168.0.206 at port 5555" |

Derived, proposed with this specification:

| | Requirement | Reason |
|---|---|---|
| R4 | The command line of xcpclient is unchanged. MCP mode and command line use the same code for connect, A2L handling, calibration and measurement | No second implementation which drifts apart. `test/test_examples.sh` keeps guarding the behavior |
| R5 | stdio and Streamable HTTP transport | stdio works with every host and needs no setup, HTTP is the daemon of R1 |
| R6 | The server never terminates because of a target: no panic on unexpected XCP responses, a lost connection is a state | A daemon which dies with the target is useless. Today `src/xcp_client/` uses `assert!`, `panic!` and `unwrap()` on target responses, see 8.3 |
| R7 | Changes in the target (parameter writes, downloads of parameter files) are explicit, checked against the A2L limits and the EPK, and can be switched off | The model can be wrong, section 2.9 |
| R8 | Tool results are small: summaries, limited lists | Context budget, section 2.9 |
| R9 | On shutdown (stdin closed, SIGTERM, Ctrl-C) a running measurement is stopped, the files are finalized and `DISCONNECT` is sent | An MDF file which is not finalized needs repair, see `tools/xcpclient/README.md` |

## 4. Concept

### 4.1 Operating modes

```bash
xcpclient --mcp                          # MCP server on stdio, started by the host
xcpclient --mcp-http                     # MCP server on http://127.0.0.1:8555/mcp, a daemon
xcpclient --mcp-http 127.0.0.1:9000      # other address or port
```

New options:

| Option | Meaning | State |
|---|---|---|
| `--mcp` | Run as MCP server on stdin/stdout | Implemented |
| `--mcp-read-only` | Do not offer the tools which change the target (`xcp_write`, `xcp_load_parameters`). The model does not even see them | Implemented |
| `--mcp-dir <DIR>` | Directory for the files the server writes on its own: the uploaded A2L file, `XCP_104.aml`, an A2L file created from an ELF file without a given name. Default: the cache directory of the user for xcpclient (macOS `~/Library/Caches/xcpclient`, Linux `$XDG_CACHE_HOME/xcpclient` or `~/.cache/xcpclient`, Windows `%LOCALAPPDATA%\xcpclient\cache`). The working directory is not changed: file names in tool arguments (`a2l`, `elf`) are relative to the directory the host starts xcpclient in | Implemented |
| `--mcp-http [ADDR]` | Run as MCP server on Streamable HTTP, default `127.0.0.1:8555`, endpoint `/mcp` | Phase 3 |
| `--mcp-token <TOKEN>` | HTTP only: require `Authorization: Bearer <TOKEN>` | Later |

In MCP mode the existing options and the config file (`--config xcpclient.toml`) are the **defaults of the tool arguments**: `--dest-addr`, `--port`, `--udp`/`--tcp`, `--bind-addr`, `--a2l`, `--elf`, `--default-event`, `--trace-auto`. With `xcpclient --mcp --dest-addr 192.168.0.206 --udp`, the sentence "connect to the target" is enough. Without `--udp` or `--tcp` the default protocol is UDP, not TCP as on the command line. The options which trigger an action on the command line (`--mea`, `--cal`, `--list-*`, `--upload-*`, `--create-a2l`, `--test`, `--time`, the output files and others) are rejected together with `--mcp`.

Logging goes to stderr in both modes, at the level given with `--log-level`. Hosts show or store the stderr of a stdio server (Claude Code: in its MCP log). In stdio mode stdout is redirected to stderr for the whole process, see 8.3.

xcpclient is built with the MCP mode by default (cargo feature `mcp`). `cargo build --no-default-features` builds it without, the options do not exist then.

### 4.2 Architecture

```
 Host (Claude Code)                      xcpclient --mcp                                   Target
+------------------+        +------------------------------------------------+        +-------------+
|                  | stdio  |  MCP layer (rmcp)                               |        |             |
|  LLM, MCP client |<------>|   transport, tool router, argument schemas,    |        |             |
|                  |  or    |   result formatting                            |        |             |
+------------------+  HTTP  |        |  commands (mpsc), replies (oneshot)   |        |             |
                            |        v                                        |        |             |
                            |  Session actor - one task, owns everything:     |  XCP   |             |
                            |   XcpClient, registry (A2L), state machine,     |<------>| XCP server  |
                            |   measurement, timers                           | UDP/TCP|             |
                            |        ^                                        |        |             |
                            |        | DAQ packets                            |        |             |
                            |  Receive task -> DaqDecoder -> writers:         |        |             |
                            |   MDF, MCAP, CSV, Perfetto, ValueWriter         |        |             |
                            +------------------------------------------------+        +-------------+
```

- **MCP layer**: generated mostly by the Rust SDK. One function per tool: parse the arguments, send a command to the session actor, format the reply as text. No XCP knowledge.
- **Session actor** (`src/session/actor.rs`): a single task which owns the `Session` with the `XcpClient`, the registry and the state. It runs on its own thread with a single threaded tokio runtime, because the futures of the XCP client are not `Send`. It takes one command at a time from a channel. XCP is a strict request and response protocol with one command in flight, the actor enforces this without locks, also when two hosts call tools at the same time. It also owns the timers: the end of a timed measurement, the end of a sample window.
- **Receive task and writers**: as today. The DAQ packets go from the receive task through the `DaqDecoder` to the writers, without passing the actor. A measurement runs on its own after the start, the actor is free for other commands, for example a parameter write while measuring.
- **ValueWriter**: a new `DaqWriter` which keeps the last sample of each DAQ list (a copy of the payload, decoded only when asked for) and, for a sample window, statistics of selected signals. This is what `xcp_read` and `xcp_sample` answer from.

The command line stays a second front end on the same session code, see [section 8.2](#82-refactoring-the-session-out-of-mainrs).

### 4.3 Session state

```
                 xcp_connect                A2L loaded              xcp_measure_start
 disconnected -------------> connected ---------------> ready ---------------------> measuring
      ^                          |                        |   <---------------------     |
      |                          |                        |     xcp_measure_stop,        |
      |                          |                        |     duration elapsed         |
      +--------------------------+------------------------+------------------------------+
                         xcp_disconnect, connection lost
```

| State | Meaning | Tools which work |
|---|---|---|
| `disconnected` | No XCP connection. Also after a lost connection, the reason is kept for `xcp_status` | `xcp_status`, `xcp_connect`, `xcp_create_a2l` (offline) |
| `connected` | XCP connected, no description of the target. The target has no A2L file to upload and none was given | also `xcp_disconnect`, `xcp_load_a2l` |
| `ready` | Description loaded: names, addresses, types, events are known | all |
| `measuring` | A recording runs in the background | all, except a second measurement, see 4.5 |

`xcp_connect` is idempotent: connected to the same target, it returns the current state and does nothing. The exception: if `a2l`, `elf` or `accept_epk_mismatch` is given, or no description is loaded yet, the description is loaded again on the existing connection. `xcp_load_a2l` does the same on its own, without the connect. Connected to another target, it is an error which names the current target. The model then asks the user or disconnects first, it does not silently drop a connection with a running measurement.

### 4.4 Description of the target

`xcp_connect` loads the description like the command line does, in this order:

1. `a2l` given: load the file, compare its EPK with the EPK of the target, compare the event ids and segment numbers with the target (warning in the result, as `--fix-a2l` would correct).
2. `elf` given: create the A2L file from the ELF file and the event and segment information of the target (as `--elf --create-a2l`).
3. Neither: upload the A2L file from the target (`GET_ID`), store it in the MCP directory (`--mcp-dir`). The A2L file of an XCPlite target includes the A2ML description `XCP_104.aml` by name, xcpclient writes this file into the MCP directory if it is not there.

The MCP directory is never the working directory by default. An XCPlite application writes its A2L file `<name>.a2l` into its working directory when a client connects, and reads it for the upload. A server started in the same directory, which is the normal case when a host starts it in the project directory, must not write a file with this name there: this emptied the A2L file of the application (fixed, see `tools/xcpclient/CHANGELOG.md`). Two more things protect the file: on the command line an upload does not overwrite an existing `<name>.a2l`, it goes to `<name>_upload.a2l`, and every upload is written to a temporary file which gets its name when the upload is complete.
4. The target has no A2L file (`no_a2l`, `rtos` configuration): state `connected`. The result says that `xcp_load_a2l` with an A2L or ELF file is needed.

An EPK mismatch is an error. It is overridden with the argument `accept_epk_mismatch: true`, which the tool description tells the model to set only when the user confirmed. This replaces the `Continue anyway? [y/N]` prompt and `--yes`. `xcp_status` then shows that the EPK differs.

An A2L file created from an ELF file (2.) is checked in the same way: the EPK in the ELF file (`XcpCreateEpk`) is compared with the EPK of the target, a mismatch is an error unless `accept_epk_mismatch` is given. The command line does the same, it asks or needs `--yes`. An ELF file without EPK can not be compared, the result says so.

### 4.5 Reading values and measuring

There are three ways to get values, with different costs:

| Tool | For | How |
|---|---|---|
| `xcp_read` | "What is the value now?" | Signals of a running measurement: the last sample from the ValueWriter. Everything else which can be uploaded: polling (`SHORT_UPLOAD`). `xcp_read` never starts a measurement. A value which is neither in the running measurement nor can be uploaded is refused, with the hint to use `xcp_sample` |
| `xcp_sample` | "Watch it for 5 seconds" | A DAQ measurement of the given signals for a time window of at most 30 s, the call returns when it is over. Result: count, rate, min, max, mean and last value per signal. No file |
| `xcp_measure_start`, `xcp_measure_stop` | "Record into a file" | A DAQ measurement in the background, into MDF, MCAP, CSV or Perfetto files, with a time limit or until stopped. The start returns immediately, the stop returns the statistics |

Polling in XCPlite covers more than global variables and parameters: an upload from an address with dynamic addressing mode is possible as well, the XCP server answers it when the event of the address occurs. Tried with `hello_xcp`: the local variables of `main` and of a function, measured with stack relative addressing, are read by `xcp_read`. If the event does not occur, the upload stalls, which is rare. The consequence for the server: a timeout of such an upload is not a lost connection (4.6), it is reported for this value (`task2.x: no response, event 'task2' is not triggered`) and the session stays connected. This is implemented generally: when an XCP command of a tool fails, the actor checks with `GET_STATUS` whether the XCP server still responds. If it does, only the value is reported as not readable. A late response is discarded before the next command. Not tried yet with an event which does not occur.

XCP has one DAQ setup, which can not be changed while it runs. While a recording runs, `xcp_sample` works only for signals which are part of the recording. For others the error says so: `SigGen1.input is not part of the running measurement. Stop it with xcp_measure_stop or sample it after the measurement.`

A signal whose event never triggers gives no sample. `xcp_sample` reports this per signal (`no sample within 2 s, event 'task2' is not triggered`) instead of failing as a whole.

### 4.6 Lost connection

A command which gets no response within `CMD_TIMEOUT` (3 s) or a closed TCP connection means the target is gone (reset, power off, cable). The session goes to `disconnected` with the reason, a running measurement is stopped locally and its files are finalized. The tool call which hit the timeout returns `Connection to hello_xcp lost (no response to SHORT_UPLOAD). Call xcp_connect to reconnect.`

There is **no keep-alive polling and no automatic reconnect**, for two reasons:

- With UDP, XCPlite gives the session to whoever sends last: a message from another address or port disconnects the current client (`handleXcpCommand` in `src/xcpethtl.c`). When CANape or a command line xcpclient connects to the same target, the MCP server is displaced. A keep-alive from the displaced MCP server would in turn disconnect CANape, every few seconds. So after a timeout the server sends nothing until an explicit `xcp_connect`.
- After a restart the target may run another software. A new connect checks the EPK again and reloads the description, a silent reconnect would keep stale addresses.

The price: `xcp_status` shows `ready` for a target which died a minute ago, until the next tool call notices. With a running measurement the server notices earlier, because the DAQ packets stop.

Both topics are not closed, they are noted in [section 10](#10-decisions-and-open-points): what should happen when another client connects to the target while the MCP server is connected, and whether an alive check is needed later.

## 5. Tools

All tools have the prefix `xcp_`, because some hosts show tool names without the server name. In Claude Code, with the server registered as `xcp`, the full name is `mcp__xcp__xcp_connect`.

| Tool | Changes the target | Needs state | Command line equivalent | Phase |
|---|---|---|---|---|
| `xcp_status` | no | any | | 1 |
| `xcp_connect` | no | any | `--dest-addr --port --udp/--tcp --a2l --elf --upload-a2l` | 1 |
| `xcp_disconnect` | no | any | | 1 |
| `xcp_list_measurements` | no | ready | `--list-mea` | 1 |
| `xcp_list_parameters` | no | ready | `--list-cal` | 1 |
| `xcp_read` | no | ready | | 1, measured signals in 2 |
| `xcp_write` | **yes** | ready | `--cal` | 1 |
| `xcp_sample` | no | ready | `--mea --time` | 2 |
| `xcp_measure_start` | no | ready | `--mea --time --csv --mdf --mcap --perfetto` | 2 |
| `xcp_measure_stop` | no | measuring | Ctrl-C | 2 |
| `xcp_load_a2l` | no | connected | `--a2l`, `--elf`, `--upload-a2l`, `--fix-a2l` | 4 |
| `xcp_create_a2l` | no | any | `--create-a2l --elf`, `--create-a2l-template`, `--offline` | 4 |
| `xcp_save_parameters` | no | ready | `--upload-bin --bin` | 4 |
| `xcp_load_parameters` | **yes** | ready | `--download-bin --bin` | 4 |

"Changes the target" is the tool annotation: these tools get `readOnlyHint: false, destructiveHint: true`, all others which only read get `readOnlyHint: true`. `xcp_connect`, `xcp_disconnect` and the measurement tools change the XCP session but not the behavior of the application in the target, they get `readOnlyHint: false, destructiveHint: false`.

### 5.1 Server instructions

The text the model gets about the server as a whole:

```text
XCP measurement and calibration client for one target (ECU) running XCPlite.
The connection to the target is kept between calls. Call xcp_status first: if a target is already connected, do not connect again.
Never guess signal or parameter names, look them up with xcp_list_measurements and xcp_list_parameters (regular expressions).
xcp_write changes the behavior of a running machine. Call it only when the user asked for exactly this change, and report the
old and the new value.
For one current value use xcp_read, to observe signals for some seconds use xcp_sample, to record into a file use
xcp_measure_start and xcp_measure_stop.
Measurement data is written to files, tool results contain summaries only.
When a tool reports that the connection is lost, tell the user, and reconnect with xcp_connect only if the user wants it.
```

### 5.2 `xcp_status`

No arguments. Works in every state and is the first call of a conversation.

```text
State: ready
Target: hello_xcp at 192.168.0.206:5555 (UDP), connected for 4 min
EPK: V2.3, matches the A2L file
XCP: protocol 1.4, MAX_CTO 248, MAX_DTO 1464, resources CAL DAQ
Description: hello_xcp_V2.3.a2l (loaded from file), 2 events, 2 calibration segments, 9 signals, 3 parameters
Messages of the target (SERV_TEXT):
  (the last 20 lines, if any)
```

The states are `disconnected`, `connected, no description of the target loaded`, `ready` and `measuring`. Not connected, it shows the default target of `xcp_connect`, and after a lost connection the reason:

```text
State: disconnected
Connection to hello_xcp at 192.168.0.206:5555 (UDP) lost (ShortUpload: Command response timeout), 2 min ago
Default target of xcp_connect: 192.168.0.206 port 5555 (UDP)
```

While a recording runs, and after it:

```text
State: measuring
...
Measurement: 9 signals in 2 DAQ lists, running 12.4 s of 60 s, 19220 events, 1550 event/s, 0.028 MByte/s, 0 packets lost, last sample 2 ms ago, files: hello_xcp_20261128_180907.mf4 (0 bytes)
```

```text
Last measurement: 9 signals measured for 60.0 s (the requested duration is over): 93000 events, 1550 event/s, 0.028 MByte/s, 0 packets lost. Files: hello_xcp_20261128_180907.mf4 (2.5 MB)
```

`last sample ... ago` is how a target which stopped sending shows up: `xcp_status` sends no command and does not detect a lost connection by itself. The size of a file which is being written lags behind, the writers buffer.

### 5.3 `xcp_connect`

| Argument | Type | Default | |
|---|---|---|---|
| `host` | string | `--dest-addr` | IP address, IP:port or host name of the target |
| `port` | integer | `--port`, 5555 | |
| `protocol` | `udp`, `tcp` | `--udp`/`--tcp`, else `udp` | See open decision 2 |
| `a2l` | string | `--a2l` | A2L file to use instead of an upload |
| `elf` | string | `--elf` | ELF file to create the A2L file from. With `a2l`: the name of the A2L file to write |
| `default_event` | string | `--default-event` | Event id or name for variables without a fixed event |
| `accept_epk_mismatch` | boolean | false | Only after the user confirmed |

Result: `Connected.` and the status. Errors:

```text
Could not connect to 192.168.0.206:5555 (UDP): Connect: Command response timeout. Is the application running, are address, port and protocol (udp or tcp) right?
```

```text
Already connected to hello_xcp at 192.168.0.206:5555 (UDP). Call xcp_disconnect first to connect to 192.168.0.146:5555 (UDP).
```

```text
The description of the target could not be loaded: EPK mismatch: the A2L file has EPK 'V9.9', the target reports EPK 'V2.3'. The A2L file describes another software version, addresses may be wrong. Ask the user. To use this A2L file anyway, call xcp_connect again with accept_epk_mismatch = true

Current state:
State: connected, no description of the target loaded
...
```

When the description can not be loaded, the target stays connected. A description which was loaded before stays in use.

### 5.4 `xcp_disconnect`

No arguments. Stops a running recording first, the result then contains its summary. Sends `DISCONNECT` and forgets the description.

### 5.5 `xcp_list_measurements`, `xcp_list_parameters`

| Argument | Type | Default | |
|---|---|---|---|
| `pattern` | string | `.` | Regular expression on the name, as `--list-mea` and `--list-cal` |
| `limit` | integer | 50 | Maximum 200 |
| `offset` | integer | 0 | For the next page |

One line per object, parameters with their current value, read from the target:

```text
3 of 3 parameters matching '.'
params.counter_max  u16 = 1024  limits 0..65535
params.delay_us  u32 = 1000  limits 0..500000
params.flow_rate  f32 = 0.301  limits 0..2
```

```text
4 of 12 signals matching '.', next page: offset 4. Use a more specific pattern to get fewer
diff_temp  f64  event calc_power
outside_temperature  u8  event mainloop
matrix_f32  f32[8x4]  unit Volt  event mainloop
global_counter  u16  no event, needs default_event
```

### 5.6 `xcp_read`

| Argument | Type | |
|---|---|---|
| `names` | array of strings | Exact names of parameters or signals, at most 50 |

```text
params.counter_max = 1024  (parameter, limits 0..65535)
counter = 402  (measured, sample 2 ms old)
global_counter = 776519  (polled)
heat_power = 8728.99 Watt  (polled)
state = RUNNING  (polled)
other_var: can not be read by upload (ShortUpload: Access denied)
nosuch_counter: not found. Similar: params.counter_max, global_counter
```

A parameter is shown as `xcp_write` takes it, with its limits. A signal is shown as physical value with unit, an enum as its label, as in the CSV output. Arrays show the first 16 elements. A name which can not be read does not fail the call, as long as one value could be read.

`xcp_read` never starts a measurement, see 4.5. A signal of the running measurement (a recording or an open `xcp_sample` window) is answered with its last sample, without a command to the target. Everything else is read by upload. A measured signal which has no sample yet is tried by upload, the message names its event if this fails as well.

### 5.7 `xcp_write`

| Argument | Type | |
|---|---|---|
| `name` | string | Exact name of a parameter |
| `value` | number | Checked against the A2L limits |

The old value is read, the new value is checked against the limits, written with `SHORT_DOWNLOAD` and read back:

```text
params.counter_max: 1024 -> 500  (u16, limits 0..65535)
```

Errors:

```text
params.counter_max: value 99999999 exceeds the limits 0..65535, not written
```

```text
counter_max not found, nothing written. Similar: params.counter_max
```

```text
global_counter is a measurement variable, not a calibration parameter. Only parameters can be written.
```

Scope as `--cal`: scalar parameters of basic types, the value as `--cal` and `--list-cal` handle it (open decision 8). Curves, maps and enum labels as input are later.

The change is in the working page (RAM) of the target. Whether it survives a restart depends on the target (persistence, freeze).

### 5.8 `xcp_sample`

| Argument | Type | Default | |
|---|---|---|---|
| `names` | array of strings | | Exact signal names, at most 20 |
| `duration_ms` | integer | 1000 | 100 to 30000 |

The call returns when the window is over:

```text
Sampled 4 signals for 2.0 s, 0 packets lost
heat_power  1552 samples, 775/s  min 8729  max 8729  mean 8729  last 8729 Watt
t1  1552 samples, 775/s  min 50  max 50  mean 50  last 50
counter  1552 samples, 775/s  min 0  max 1000  mean 498.58  last 771
state  3 samples, 1.5/s  last RUNNING
task2.x: no sample in 2.0 s, its event 'task2' was not triggered
nosuch: not found.
params.counter_max: is a calibration parameter without event, it can not be measured. Read it with xcp_read.
```

Minimum, maximum and mean are of the physical value, of the first element for an array, with 6 significant digits. An enum has no statistics, only its last label. The call is an error only if no signal had a sample.

Without a recording, `xcp_sample` starts a measurement of the signals and stops it at the end of the window. While a recording runs, it uses the recording and can only cover its signals:

```text
heat_power: not part of the running measurement. Stop it with xcp_measure_stop or sample it after the measurement.
```

While the window is open, the server answers `xcp_status`, `xcp_read`, `xcp_write` and the lists. The tools which change the measurement or the connection are refused until it is over (`xcp_sample is running for another 2.5 s, call this tool again when it is done.`). A cancelled call closes the window and stops its measurement. Progress notifications are not sent, the window is at most 30 s.

### 5.9 `xcp_measure_start`, `xcp_measure_stop`

`xcp_measure_start`:

| Argument | Type | Default | |
|---|---|---|---|
| `names` | array of strings | | Exact signal names, or |
| `pattern` | string | | a regular expression, `.` for all. Not both |
| `duration_s` | integer | 0 | 0: until `xcp_measure_stop` |
| `mdf`, `mcap`, `csv`, `perfetto` | string | | Output files, relative to the working directory. None given: an MDF file `<target>_<date>_<time>.mf4` in the working directory |
| `trace_auto` | `off`, `context`, `links`, `all` | `links` | With `perfetto` |

It returns immediately:

```text
Measurement started: 2 signals in 1 DAQ list, for 2 s, it stops by itself.
Files: m.mf4, m.csv
nosuch: not found.
```

Signals without event are not measured, with names they are listed with the reason, with a pattern they are counted. An existing file is not overwritten: `The file 'm.csv' exists, a measurement does not overwrite files. Give another file name.` There is one recording at a time.

`xcp_measure_stop` has no arguments and returns the summary:

```text
9 signals measured for 2.6 s (stopped by xcp_measure_stop): 3930 events, 1532 event/s, 0.028 MByte/s, 0 packets lost. Files: hello_xcp_20261128_180907.mf4 (107 KB)
```

The summary of a recording which ended in another way is shown by `xcp_status` as `Last measurement`, with the reason: `the requested duration is over`, `stopped by xcp_disconnect`, `the connection was lost`, `xcpclient was shut down`. In all these cases the files are completed.

### 5.10 `xcp_load_a2l`

Load or replace the description of the connected target. Not while a recording runs.

| Argument | Type | Default | |
|---|---|---|---|
| `a2l` | string | | A2L file to load. Or |
| `elf` | string | | ELF file to create the A2L file from, with the events and calibration segments of the target. With `a2l`: the name of the A2L file to write. Neither: upload the A2L file from the target |
| `fix` | boolean | false | With `a2l`: correct the event ids and calibration segment numbers in the file with those of the target and rewrite it, as `--fix-a2l`. Only for A2L files created by xcpclient |
| `default_event` | string | `--default-event` | |
| `accept_epk_mismatch` | boolean | false | Only after the user confirmed |

Result: `Description loaded.`, the status, and the warnings found on the way.

### 5.11 `xcp_create_a2l`

Create an A2L file from an ELF file, without using it. Works in every state. Not connected, or with `offline`, the file is created from the ELF file alone. Connected, the events and calibration segments of the target are used as well and the EPKs are compared. The description in use is not changed.

| Argument | Type | Default | |
|---|---|---|---|
| `elf` | string | | ELF file with DWARF debug information |
| `a2l` | string | | A2L file to write |
| `overwrite` | boolean | false | Replace the A2L file if it exists |
| `template` | boolean | false | Events and calibration segments only, as `--create-a2l-template` |
| `offline` | boolean | false | Do not use the connected target |
| `var_filter`, `unit_filter` | string | | Regular expressions on variable and compilation unit names, as `--elf-var-filter`, `--elf-unit-filter` |
| `skip_no_metadata` | boolean | false | As `--elf-skip-no-metadata` |
| `embed_aml` | boolean | false | As `--embed-aml`. Without it, `XCP_104.aml` is written next to the A2L file if it is not there |
| `default_event` | string | `--default-event` | |
| `accept_epk_mismatch` | boolean | false | Only after the user confirmed |

```text
A2L file out/offline.a2l created from no_a2l_demo.elf (offline, from the ELF file only): 4 events, 3 calibration segments, 49 signals, 14 parameters. out/XCP_104.aml written, the A2L file includes it.
Warning: 188 variables not described, location is a register: main_c:333 'p_counter_control', main_c:346 'p_params', main_c:363 'delay', ...
Warning: 78 variables not described, not registered, it has no location in the debug information, not implemented yet: 'len' in safe_strnlen, 'i' in XcpPrintDaqList, ...
Warning: 2 variables not described, has unsupported type: 'heap_struct', 'gModuleAddr'
```

The warnings are those the generator logs (see [OFFLINE_A2L.md](OFFLINE_A2L.md)). An ELF file has hundreds of variables which can not be described, mostly local variables the compiler keeps in registers. They are reported as one line for each reason, with the number of variables and three examples, the full list is in the log.

Errors: `The file 'out/offline.a2l' exists. To replace it, call xcp_create_a2l again with overwrite = true.`, an EPK mismatch as in 5.3, a macOS executable (the message of the ELF reader says that ELF files are needed).

### 5.12 `xcp_save_parameters`, `xcp_load_parameters`

`xcp_save_parameters` reads all calibration segments from the target and writes them into a file.

| Argument | Type | Default | |
|---|---|---|---|
| `file` | string | `<target>_<date>_<time>.hex` in the working directory | `.hex`: Intel-HEX with the data of the segments. `.bin`: XCPlite persistence file, with events and EPK |

```text
2 calibration segments saved to p.hex (156 bytes).
```

An existing file is not overwritten.

`xcp_load_parameters` writes a `.hex` file into the working page of the target. It changes the target and does not exist with `--mcp-read-only`.

| Argument | Type | |
|---|---|---|
| `file` | string | An Intel-HEX file written by `xcp_save_parameters` |

```text
2 of 2 calibration segments loaded from p.hex into the working page of the target.
```

The file contains addresses, not names, and has no EPK. It is not checked against the software on the target, the tool description tells the model to use only a file of the same software version. A segment is loaded only if the file has all of its bytes, a segment the file has no data for is skipped with a warning. A `.bin` file can not be loaded, as on the command line.

## 6. Example conversation

| You say | The model calls |
|---|---|
| Connect to target 192.168.0.206 at port 5555 | `xcp_status`, then `xcp_connect {host: "192.168.0.206", port: 5555}` |
| Are we still connected? Which software is running? | `xcp_status` |
| Which signals does the signal generator have? | `xcp_list_measurements {pattern: "SigGen"}` |
| What is counter_max? | `xcp_read {names: ["counter_max"]}` |
| Set it to 2000 | `xcp_write {name: "counter_max", value: 2000}` |
| Does counter now run up to 2000? | `xcp_sample {names: ["counter"], duration_ms: 3000}` |
| Record everything for a minute as MDF | `xcp_measure_start {pattern: ".*", duration_s: 60}` |
| How is the measurement going? | `xcp_status` |
| Disconnect | `xcp_disconnect` |

The value of an LLM in front of xcpclient is in the requests which need several steps and some judgment: "Find out which parameter limits the amplitude of SigGen1 and halve it", "Compare the cycle times of all events with their nominal values", "Create the A2L file for build/no_a2l_demo.elf and tell me which variables were skipped and why". The model chains the tools on its own and reads the results in between.

## 7. Safety and security

**Deferred.** Security is not a topic of the first phases: xcpclient is a lab tool, used with stdio transport on the machine of the developer. Of this section, only the checks which the command line already has (A2L limits, EPK check) and `--mcp-read-only` are implemented for now. The rest is kept here as the list to go through before the HTTP mode is used outside of a lab network.

A model which can write parameters into a running machine needs limits. The mechanisms, from the outside to the inside:

1. **Host permission.** Hosts ask the user before a tool call, per tool. In Claude Code, allow the read-only tools permanently (`mcp__xcp__xcp_status`, `mcp__xcp__xcp_read`, ... in `permissions.allow` of `.claude/settings.json`) and keep `xcp_write` and `xcp_load_parameters` on "ask". This is why reading and writing are separate tools.
2. **`--mcp-read-only`.** The writing tools do not exist. For a target where nothing may be changed.
3. **Checks in the server.** A2L limits for every write. EPK check at connect, a mismatch is an error unless `accept_epk_mismatch` is given. Exact names only for reading and writing, no regular expressions.
4. **File access.** Tools read and write files only inside the MCP directory, existing files are not overwritten.
5. **Network (HTTP mode).** The server binds to 127.0.0.1 by default and checks the `Origin` header (a web page in your browser can send requests to localhost, the check rejects them, the MCP specification requires it). Binding to another address requires `--mcp-token`. Whoever can reach the port can calibrate the target.
6. **Text from the target is data.** Signal names, comments from the A2L file and `SERV_TEXT` messages reach the model as part of tool results. A model can be influenced by text it reads. The server passes such text only as values in clearly structured lines and limits its length. For targets you do not trust, use `--mcp-read-only`.

## 8. Implementation

### 8.1 MCP SDK

The official Rust SDK is the crate [`rmcp`](https://crates.io/crates/rmcp), version 3.12 (November 2026). It is built on tokio, which xcpclient already uses, implements revision 2026-07-28 and the older ones, and generates the tool list and the JSON schemas from Rust types. MSRV is Rust 1.88, edition 2024 as xcpclient.

```toml
[features]
default = ["mcp"]
mcp = ["dep:rmcp", "dep:schemars", "dep:libc"]

[dependencies]
rmcp = { version = "3.12", default-features = false, features = ["server", "macros", "transport-io"], optional = true }
schemars = { version = "1", optional = true }     # JSON schemas of the tool arguments
futures-util = "0.3"                              # session actor: catch a panic of a command

[target."cfg(unix)".dependencies]
libc = { version = "0.2", optional = true }       # redirection of stdout
```

The HTTP mode (phase 3) adds the rmcp feature `transport-streamable-http-server` and an HTTP server (axum 0.8), rmcp provides a tower service.

A tool is a function with a description, its arguments are a struct with a derived JSON schema. The doc comments of the fields become the argument descriptions the model sees. A tool function returns `Result<String, String>`: `Ok` is the text of the result, `Err` is a tool execution error (`isError: true`), which the model reads. From `src/mcp/mod.rs`, shortened:

```rust
#[derive(Debug, Deserialize, JsonSchema)]
struct WriteArgs {
    /// Exact name of a calibration parameter, as listed by xcp_list_parameters.
    name: String,
    /// The new value. It is checked against the limits of the parameter.
    value: f64,
}

#[derive(Clone)]
pub struct XcpMcpServer {
    session: SessionHandle, // handle to the session actor, all server instances share the one XCP session
    tool_router: ToolRouter<XcpMcpServer>,
}

#[tool_router]
impl XcpMcpServer {
    pub fn new(session: SessionHandle, read_only: bool) -> XcpMcpServer {
        let mut tool_router = Self::tool_router();
        if read_only {
            tool_router.remove_route("xcp_write"); // the model does not see the tool
        }
        XcpMcpServer { session, tool_router }
    }

    #[tool(description = "Show the state of the XCP connection ...", annotations(read_only_hint = true))]
    async fn xcp_status(&self) -> Result<String, String> {
        self.session.status().await
    }

    #[tool(description = "Set a calibration parameter ...", annotations(read_only_hint = false, destructive_hint = true))]
    async fn xcp_write(&self, Parameters(args): Parameters<WriteArgs>) -> Result<String, String> {
        self.session.write(args.name, args.value).await
    }
}

#[tool_handler(router = self.tool_router, name = "xcpclient", instructions = "XCP measurement and calibration client ...")]
impl ServerHandler for XcpMcpServer {}

// --mcp: stdin and the original stdout, see 8.3
let service = XcpMcpServer::new(session.clone(), read_only).serve((tokio::io::stdin(), output)).await?;
service.waiting().await?; // returns when stdin is closed
session.shutdown().await; // DISCONNECT
```

The MCP layer has no XCP knowledge. Everything a tool does is an operation of the session actor.

### 8.2 Refactoring: the session out of main.rs

Today the whole sequence (connect, identify, A2L upload or creation or load, EPK check, event and segment check, list, calibrate, measure, disconnect) is one function of 800 lines, `xcp_client()` in `src/main.rs`, driven by the command line arguments. The writers (`daq_writer`, `csv_writer`, `mdf_writer`, `mcap_writer`, `perfetto_writer`, `console_writer`) are modules of the binary, not of the library (`src/lib.rs` exports `xcp_client`, `elf_reader`, `bin_reader` only).

Step one, without any MCP code and without a change in behavior. **Done**: the code is moved as it was, with its log messages, `assert!`s and `Box<dyn Error>`. The command line was compared with the binary before the change, 38 command lines against `hello_xcp` and the ELF fixtures: same exit status, same output and same files, except for timestamps and measured values.

```
src/lib.rs                     exports also session, the writers and a2l_fix. main.rs uses the library, it has no modules of its own
                               except the test executor
src/session/mod.rs             struct Session: owns XcpClient and the state. One method per step:
                                 connect, disconnect, load_description,
                                 find_parameters, read_parameter, write_parameter, upload_parameters, download_parameters,
                                 find_measurements, describe_measurement, start_measurement, stop_measurement
src/session/description.rs     load_description: upload, create or load the A2L file, EPK check, event and segment check, fix
src/session/default_event.rs   DefaultEvent (--default-event), moved from main.rs
src/main.rs                    command line: parses Args, calls the Session methods in the fixed order, prints the results.
                               The EPK mismatch prompt is a callback given to load_description
```

Typed results and a `SessionError` are added in phase 1 where the tools need them, today `connect` and `load_description` only log what they find.

Step two, the MCP mode. **Phase 1 done:**

```
src/session/actor.rs           SessionHandle and the actor task on its own thread: commands over mpsc, replies over oneshot.
                               The operations of the tools (status, connect, disconnect, list, read, write), the session state,
                               the check for a lost connection, the text of the results and error messages
src/mcp/mod.rs                 the rmcp server: tools with descriptions and annotations, argument structs, instructions, run_stdio
src/mcp/stdio.rs               protect_stdout: redirection of stdout to stderr
src/main.rs                    --mcp, --mcp-read-only, --mcp-dir
tests/mcp_stdio.rs             integration test, see 8.4
```

**Phase 2 done:**

```
src/value_writer/mod.rs        ValueStore and ValueWriter: a DaqWriter which keeps the last sample of each DAQ list (decoded when asked for)
                               and the statistics of a time window for selected signals
src/session/actor/measure.rs   the measurement operations of the actor: sample, measure start, measure stop, the timers for the end of
                               a sample window and of a timed recording, the summaries
src/session/mod.rs             start_measurement_of (names and additional writers), measurement_progress, abort_measurement
```

**Phase 4 done:**

```
src/session/actor/files.rs     the file operations of the actor: load A2L, create A2L, save and load parameters
src/session/description.rs     build_description, shared by load_description and the new create_a2l_file, which writes an A2L file
                               without changing the description of the session. The EPK of an ELF file is compared with the target
src/log_capture.rs             CaptureLogger: the warnings and errors logged while a tool runs are appended to its result.
                               Warnings about single variables of an ELF file are counted by reason
```

A sample window is answered when it is over: the actor keeps the reply and goes on serving commands, it wakes up at the end of the window. The MCP tool waits for the reply and for the cancellation of the request at the same time.

The actor replies with text (`Result<String, String>`), which the MCP layer passes on. The data behind it is typed (`TargetInfo`, `DescriptionInfo`, `ParameterInfo`, `XcpMeasurementInfo` in the session and the XCP client), the formatting is in `actor.rs`. A typed reply for `structuredContent` (open decision 6) would be a change in these two files.

### 8.3 What a long running server needs from the existing code

| Topic | Before | State |
|---|---|---|
| stdout | `env_logger` logs to stdout, about 120 `println!`/`print!` in `main.rs`, `elf_reader`, `bin_reader`, `xcp_client/decoder.rs`, `xcp_test_executor.rs` and `ConsoleWriter::stdout()` | **Done.** The print statements stay as they are, most are the verbose output of the ELF reader. Instead, the MCP mode protects the protocol channel for the whole process (`mcp::protect_stdout`): at startup it keeps a duplicate of the original stdout for the MCP transport and redirects file descriptor 1 to stderr, the logger target is stderr. This also covers prints in libraries. Unix only, on Windows nothing is redirected and nothing but the MCP transport may write to stdout |
| `SERV_TEXT` | Printed to stdout (`ServTextDecoder` in `main.rs`) | **Done.** The actor gives `Session::connect` a text decoder which keeps the last 20 lines, shown by `xcp_status`. Not tried with a target which sends `SERV_TEXT` |
| Panics on target responses | `assert!`, `panic!` and `unwrap()` on responses of the XCP server | **Done.** A response which is shorter than the client reads it is an error (`min_response_len` in `transport.rs`). Protocol settings xcpclient does not support (DAQ timestamp, DAQ header, address granularity, byte order) give the error `XCP server not supported by xcpclient: ...`. A target without ECU name, A2L name or EPK is not a panic. Printing an error of a command which was missing in the `XcpCommand` enum (`GET_VERSION`, `GET_COMM_MODE_INFO`) panicked, for example a timeout during connect |
| Panics on wrong use | `unwrap()` on the registry, the socket and the channels, for a client which is not connected or has no description. A parameter file name without extension panicked | **Done.** Errors (`Not connected to an XCP server`, `No A2L description of the target loaded`, `File name ... has no extension`). Internal invariants (command builder alignment and such) are still asserts. As a safety net the actor catches a panic of a command: the session is dropped, the state is `disconnected` with the reason, the tool call returns an error |
| Late responses | A response which arrived after the command timeout stayed in the response channel and was taken as the response to the next command | **Done.** Discarded with a warning before a command is sent. Needed for a session which goes on after a timeout, for example of an upload with dynamic addressing (4.5) |
| Errors | `Box<dyn Error>`, which is not `Send` | **Done, differently.** The session keeps `Box<dyn Error>`. The actor runs on its own thread with a single threaded runtime, so nothing has to be `Send`, and converts errors to text at its boundary. The messages for the model are written in the actor. A typed `SessionError` was not needed |
| Measurement stop | `XcpClient::stop_measurement` returned before the DAQ decoder was stopped, if the receive task had terminated: the files stayed open | **Done.** The decoder is always stopped. For a lost connection the measurement is completed locally without commands (`abort_measurement`) |
| Lost connection | Not detected, `is_connected` is a flag of the client | **Done.** When an XCP command of a tool fails, the actor sends `GET_STATUS`. No response: the session is dropped, the state is `disconnected` with the reason (4.6). This costs a second command timeout, a lost target is reported after 6 s |
| EPK mismatch | Interactive prompt on stdin, or `--yes` | **Done.** A callback of `Session::load_description`, the prompt is in `main.rs`, the actor decides by `accept_epk_mismatch` |
| Calibration objects | `calibration_object_list` only grew, `create_calibration_object` pushed a new object on every call | **Done.** The object of a characteristic is replaced when it is accessed again |
| Reconnect | One connect per process | **Done.** The actor creates a `Session` with its `XcpClient` per connect and drops it at disconnect, the receive task ends with it |
| Shutdown | Disconnect at the end of `xcp_client()` | **Done** for stdio: stdin closed, SIGTERM and Ctrl-C end `run_stdio`. The actor closes an open sample window, stops a running recording, completes its files and disconnects (R9). With a tool call still open when stdin is closed, the SDK waits about 5 s before it ends |

### 8.4 Tests

- **Unit** (`cargo test`): the text formatting of the results, the `SERV_TEXT` buffer, the actor without a target (status, the errors of the tools which need a connection, an invalid regular expression, a refused TCP connection), the tool list with and without `--mcp-read-only`, and that every tool and every argument has a description.
- **Integration without target** (`cargo test`, `tests/mcp_stdio.rs`): starts the built `xcpclient --mcp` as a child process and sends MCP messages to its stdin: the handshake of the older protocol revisions, the stateless revision 2026-07-28 without handshake, tool list, annotations, tool errors, protocol errors, `--mcp-read-only`, the rejected options, exit status 0 after stdin is closed. It runs with log level trace and fails on any line on stdout which is not an MCP message.
- **With a target** (`tools/xcpclient/tests/mcp/`, `python3 tests/mcp/run_all.py`): seven scenario scripts which start `build/hello_xcp` and `xcpclient --mcp`, send MCP messages as a host would and check the tool results. They cover what the following lines list for the phases, and the regression of the A2L file emptied by an upload in the directory of the target. They need a built target and take about a minute, they are not part of `cargo test` or CTest. See the `README.md` there.
- **Phase 2 with a target** (`measure.py`, `measure_lost.py`): sample of signals of two events with a wrong name and a parameter in the list, status, read and a refused connect while a sample window is open, a cancelled sample, a recording of all signals with the default file, status, read and sample during the recording, a parameter write during the recording, a second recording refused, stop, a timed recording into MDF and CSV, MCAP and Perfetto, an existing file, disconnect during a recording, the target killed during a recording (the MDF file is completed, the state is `disconnected`), stdin closed during a recording with an open sample window.
- **Phase 4 with a target** (`files.py`): A2L creation offline from the ELF fixtures and from `no_a2l_demo.elf` (existing file, overwrite, template, embedded A2ML, filters, an invalid filter, a missing file, a macOS executable), connected with an ELF file of another EPK (refused, accepted) and without EPK, load of an A2L file (copy, another EPK, fix of a file not created by xcpclient, missing file), reload by upload, refused during a recording, parameters saved as `.hex` and `.bin` (default name, existing file, wrong extension), two parameters changed and restored by loading the `.hex` file, a `.bin` file, an empty and a broken `.hex` file. On the command line: `--elf --create-a2l` against a target with another EPK ends with exit status 1, with `--yes` it creates the file.
- **With a host**: not done yet. The tools were not called by a language model so far, so the descriptions are untested where it matters. See section 9 for the setup.
- **Manual**: the MCP Inspector, a debugger for MCP servers with a web UI:

  ```bash
  npx @modelcontextprotocol/inspector --protocol-era auto xcpclient --mcp
  ```

  It lists the tools as the model sees them and calls them with arguments from a form. Use it to read the descriptions with fresh eyes before trying them with a model.

### 8.5 Phases

| Phase | Content | Result |
|---|---|---|
| 0 | **Done.** Session refactoring (8.2 step one), panic cleanup (8.3). No MCP code | Command line unchanged, compared with the binary before the change |
| 1 | **Done.** `--mcp` on stdio with stdout redirection, session actor, tools `xcp_status`, `xcp_connect`, `xcp_disconnect`, `xcp_list_*`, `xcp_read` (polling), `xcp_write`, `--mcp-read-only`, `--mcp-dir` | Tested over stdio against `hello_xcp`. To do: try it in Claude Code (the conversation of section 1, except the last sentence) |
| 2 | **Done.** ValueWriter, `xcp_sample`, `xcp_measure_start`, `xcp_measure_stop`, `xcp_read` of measured signals from the ValueWriter, shutdown with a running measurement | Tested over stdio against `hello_xcp`. Not done: progress notifications of `xcp_sample` |
| 3 | **Postponed.** `--mcp-http`, clean shutdown as daemon | The connection survives host sessions (R1 in full) |
| 4 | **Done.** `xcp_load_a2l`, `xcp_create_a2l`, `xcp_save_parameters`, `xcp_load_parameters`, the EPK check of ELF files, warnings in tool results | Tested over stdio against `hello_xcp`. Not done: upload of the ELF file from the target (`--upload-elf`), a template from the target information alone (`--create-a2l` without ELF file), the checks of file paths (section 7, deferred) |

## 9. Setup in the hosts

The host starts the `xcpclient` it finds in the path. Install the current build first (`cargo install --path tools/xcpclient --locked`), or give the full path of `tools/xcpclient/target/debug/xcpclient` in the commands below.

### Claude Code

stdio, for the current project only (scope `local`, the default) or shared with the project in `.mcp.json` (scope `project`):

```bash
claude mcp add xcp -- xcpclient --mcp
```

```bash
claude mcp add --scope project xcp -- xcpclient --mcp --config xcpclient.toml
```

The second command writes `.mcp.json` in the project root:

```json
{
  "mcpServers": {
    "xcp": { "command": "xcpclient", "args": ["--mcp", "--config", "xcpclient.toml"] }
  }
}
```

Without the `claude` command line, for example with only the Claude desktop app installed: write this `.mcp.json` by hand into the project root, with the full path of xcpclient as `command`. A file with a path of your machine does not belong into the repository.

A new server is loaded when a session starts, and a server from `.mcp.json` has to be approved once.

HTTP, with the daemon started before:

```bash
xcpclient --mcp-http --config xcpclient.toml
```

```bash
claude mcp add --transport http xcp http://127.0.0.1:8555/mcp
```

### Claude Desktop

`claude_desktop_config.json` (macOS: `~/Library/Application Support/Claude/`) takes stdio servers, in the same `mcpServers` format as `.mcp.json` above, with an absolute path as `command`. A local HTTP server is not reachable from its "custom connector" setting, which connects from the cloud. To use the daemon from Claude Desktop, a stdio to HTTP bridge is needed (`npx mcp-remote http://127.0.0.1:8555/mcp --allow-http`). Not verified in practice.

## 10. Decisions and open points

### Decided (2026-11-28)

| | Decision | |
|---|---|---|
| 1 | Which transport first | stdio first. The HTTP daemon (phase 3) is postponed, decided after phases 1, 2 and 4 were done |
| 4 | Targets per server process | One. Two targets (Pi and ESP32) are two servers, registered for example as `xcp-pi` and `xcp-esp32` |
| 5 | Parameter writes | Available by default, xcpclient is a lab tool. The host asks for permission, `--mcp-read-only` switches them off |
| | Reading values | `xcp_read` never starts a measurement: last sample of a running measurement or upload, otherwise refused (4.5) |
| | Alive polling | None (4.6). Noted for later, see below |
| | Security | Deferred, see section 7 |
| | Measurement files | A recording without a given file name is written to the working directory, as `<target>_<date>_<time>.mf4`. Given file names are relative to the working directory. A measurement never overwrites a file |
| | EPK of an ELF file | Compared with the EPK of the target when an A2L file is created from it, in the MCP tools and on the command line. A mismatch needs `accept_epk_mismatch` or `--yes` |
| | A2L file which exists | `xcp_create_a2l` replaces it only with `overwrite = true`. Parameter and measurement files are never overwritten |
| | Files of the server | The files the server writes on its own go to the cache directory of the user, not to the working directory (4.1, 4.4). Decided after an upload into the working directory had emptied the A2L file of the application running there |

### Open, the recommendation is used until decided otherwise

Phase 1 is implemented with the recommendations of 2, 3, 6, 7 and 8.

| | Decision | Options | Recommendation |
|---|---|---|---|
| 2 | Protocol when the user does not say UDP or TCP | (a) UDP. (b) TCP, as the command line today (with a warning). (c) Try TCP, then UDP | (a). The XCPlite examples default to UDP (`OPTION_USE_TCP false`). A wrong guess costs 3 s and gives an error which names the protocol |
| 3 | After a lost connection | (a) State `disconnected`, reconnect only by `xcp_connect`. (b) Automatic reconnect | (a), see 4.6. With (b) the server fights with CANape for a UDP target |
| 6 | Result format | (a) Text only. (b) Text and `structuredContent` with an `outputSchema` per tool | (a) for phases 1 and 2. Line oriented text needs fewer tokens than JSON and is what the model reads. (b) is useful when scripts use the MCP server as an automation API, the typed results of 8.2 make it a small addition |
| 7 | Cargo feature | (a) `mcp` in the default features. (b) Optional, `cargo install --features mcp` | (a). One binary. The cost is more dependencies (rmcp, axum, schemars) and a longer build, noticeable on the Pi |
| 8 | Value semantics of `xcp_write` | (a) As `--cal` today. (b) Physical values and enum labels, converted with the A2L conversion | (a) in phase 1, check what `--cal` does with a parameter which has factor and offset before deciding (b) |

### To be discussed

- **Another client connects while the MCP server is connected.** With UDP, XCPlite disconnects the current client when a message arrives from another address or port, and the new client takes over (`handleXcpCommand` in `src/xcpethtl.c`). The MCP server does not notice until its next command, and this command in turn disconnects the new client before it times out. Open questions: Is this takeover the behavior wanted from XCPlite, or should a connected server reject a second client (`ERR_CMD_BUSY` or no response)? How does the MCP server learn that it was displaced without disturbing the other client (an `EV_SESSION_TERMINATED` event from the XCP server to the displaced client would do it)? What does TCP do with a second connection? What should a daemon do which was displaced: stay `disconnected` until told, or take the target back when the other client is gone?

### Noted for later

- **A target which stops sending during a recording** is not detected, as long as no tool sends a command: `xcp_status` and `xcp_read` of measured signals answer from what was received. `xcp_status` shows the age of the last sample. A watchdog on the DAQ data would have to know which events are sporadic.
- **Progress notifications** for `xcp_sample`. Not needed for the stdio transport with a window of at most 30 s, to be looked at with the HTTP transport, where hosts have shorter timeouts.

- **Alive check.** No keep-alive polling for now, a lost target is detected by the next command or by missing DAQ packets. To be reconsidered with the daemon (phase 3), where `xcp_status` of a connection which is idle for hours should be true. It depends on the outcome of the point above: a polling client must not disturb another client.

## 11. Sources

Looked up on 2026-11-28:

- MCP specification, revision 2026-07-28: [versioning](https://modelcontextprotocol.io/specification/versioning), [changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog), [transports](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports), [tools](https://modelcontextprotocol.io/specification/2026-07-28/server/tools), [progress](https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/progress), [elicitation](https://modelcontextprotocol.io/specification/2026-07-28/client/elicitation)
- Rust SDK: [crates.io/crates/rmcp](https://crates.io/crates/rmcp), [github.com/modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk) (README and `examples/servers`)
- Claude Code: [MCP](https://code.claude.com/docs/en/mcp), [environment variables](https://code.claude.com/docs/en/env-vars) (timeouts, `MAX_MCP_OUTPUT_TOKENS`)
- Claude Desktop: [local servers](https://modelcontextprotocol.io/docs/develop/connect-local-servers), [remote servers](https://modelcontextprotocol.io/docs/develop/connect-remote-servers), [mcp-remote](https://github.com/geelen/mcp-remote)
- MCP Inspector: [github.com/modelcontextprotocol/inspector](https://github.com/modelcontextprotocol/inspector)

Not verified: how Claude Desktop handles a local HTTP server is taken from its documentation, not tried.
