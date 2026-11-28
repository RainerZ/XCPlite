# xcpclient MCP Server

**Status: specification, implementation in progress (phase 0, see [8.5](#85-phases)).** This document explains what an MCP server is, specifies an MCP server mode for `xcpclient` (`tools/xcpclient/`) and describes how to implement it. The decisions taken, the open ones and the points noted for later are in [section 10](#10-decisions-and-open-points).

The facts about the MCP specification, the Rust SDK and the client applications were looked up on 2026-11-28, the sources are linked in [section 11](#11-sources). MCP changes quickly, check them again before implementing.

## 1. Summary

`xcpclient --mcp` turns xcpclient into a server which an AI assistant (Claude Code, Claude Desktop and others) can operate. The assistant gets a small set of functions: connect, list signals, read and write parameters, measure. xcpclient keeps the XCP connection open between the calls, so a conversation looks like this:

> **You:** Connect to target 192.168.0.206 at port 5555
> **Assistant:** Connected to `hello_xcp` (UDP), EPK `v10_21_15_02`. A2L uploaded: 3 events, 12 signals, 3 parameters.
> **You:** What is counter_max? Set it to 2000 and check that counter now runs up to 2000.
> **Assistant:** `counter_max` was 1000, now 2000. I observed `counter` for 3 s: min 0, max 1999.

The server contains no AI. It executes function calls, exactly like the command line does. Understanding the sentence and choosing the function is done by the language model in the assistant application.

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

| Option | Meaning |
|---|---|
| `--mcp` | Run as MCP server on stdin/stdout |
| `--mcp-http [ADDR]` | Run as MCP server on Streamable HTTP, default `127.0.0.1:8555`, endpoint `/mcp` |
| `--mcp-read-only` | Do not offer the tools which change the target (`xcp_write`, `xcp_load_parameters`). The model does not even see them |
| `--mcp-dir <DIR>` | Directory for files read and written by tools (A2L, ELF, measurement and parameter files), default: the current directory. Paths outside are rejected |
| `--mcp-token <TOKEN>` | HTTP only: require `Authorization: Bearer <TOKEN>` |

In MCP mode the existing options and the config file (`--config xcpclient.toml`) are the **defaults of the tool arguments**: `--dest-addr`, `--port`, `--udp`/`--tcp`, `--bind-addr`, `--a2l`, `--elf`, `--default-event`, `--trace-auto`. With `xcpclient --mcp --dest-addr 192.168.0.206 --udp`, the sentence "connect to the target" is enough. The options which trigger an action on the command line (`--mea`, `--cal`, `--list-*`, `--upload-*`, `--create-a2l`, `--test`, `--time`) are rejected together with `--mcp`.

Logging goes to stderr in both modes, at the level given with `--log-level`. Hosts show or store the stderr of a stdio server (Claude Code: in its MCP log).

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
- **Session actor**: a single task which owns the `XcpClient`, the registry and the state. It takes one command at a time from a channel. XCP is a strict request and response protocol with one command in flight, the actor enforces this without locks, also when two hosts call tools at the same time. It also owns the timers: the end of a timed measurement, the end of a sample window.
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

`xcp_connect` is idempotent: connected to the same target, it returns the current state and does nothing. Connected to another target, it is an error which names the current target. The model then asks the user or disconnects first, it does not silently drop a connection with a running measurement.

### 4.4 Description of the target

`xcp_connect` loads the description like the command line does, in this order:

1. `a2l` given: load the file, compare its EPK with the EPK of the target, compare the event ids and segment numbers with the target (warning in the result, as `--fix-a2l` would correct).
2. `elf` given: create the A2L file from the ELF file and the event and segment information of the target (as `--elf --create-a2l`).
3. Neither: upload the A2L file from the target (`GET_ID`), store it in the MCP directory.
4. The target has no A2L file (`no_a2l`, `rtos` configuration): state `connected`. The result says that `xcp_load_a2l` with an A2L or ELF file is needed.

An EPK mismatch is an error. It is overridden with the argument `accept_epk_mismatch: true`, which the tool description tells the model to set only when the user confirmed. This replaces the `Continue anyway? [y/N]` prompt and `--yes`.

### 4.5 Reading values and measuring

There are three ways to get values, with different costs:

| Tool | For | How |
|---|---|---|
| `xcp_read` | "What is the value now?" | Signals of a running measurement: the last sample from the ValueWriter. Everything else which can be uploaded: polling (`SHORT_UPLOAD`). `xcp_read` never starts a measurement. A value which is neither in the running measurement nor can be uploaded is refused, with the hint to use `xcp_sample` |
| `xcp_sample` | "Watch it for 5 seconds" | A DAQ measurement of the given signals for a time window of at most 30 s, the call blocks and reports progress. Result: count, rate, min, max, mean, first and last value per signal. No file |
| `xcp_measure_start`, `xcp_measure_stop` | "Record into a file" | A DAQ measurement in the background, into MDF, MCAP, CSV or Perfetto files, with a time limit or until stopped. The start returns immediately, the stop returns the statistics |

Polling in XCPlite covers more than global variables and parameters: an upload from an address with dynamic addressing mode is possible as well, the XCP server answers it when the event of the address occurs. If the event does not occur, the upload stalls, which is rare. The consequence for the server: a timeout of such an upload is not a lost connection (4.6), it is reported for this value (`task2.x: no response, event 'task2' is not triggered`) and the session stays connected. How the XCP server and `XcpClient` behave after such a timeout (late response, next command) has to be checked in phase 1.

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
| `xcp_create_a2l` | no | any | `--create-a2l --elf --offline` | 4 |
| `xcp_save_parameters` | no | ready | `--upload-bin --bin` | 4 |
| `xcp_load_parameters` | **yes** | ready | `--download-bin --bin` | 4 |

"Changes the target" is the tool annotation: these tools get `readOnlyHint: false, destructiveHint: true`, all others which only read get `readOnlyHint: true`. `xcp_connect`, `xcp_disconnect` and the measurement tools change the XCP session but not the behavior of the application in the target, they get `readOnlyHint: false, destructiveHint: false`.

### 5.1 Server instructions

The text the model gets about the server as a whole:

```text
XCP measurement and calibration client for one target (ECU).
The connection to the target is kept between calls. Call xcp_status first: if it is already connected, do not connect again.
Never guess signal or parameter names, look them up with xcp_list_measurements and xcp_list_parameters (regular expressions).
xcp_write and xcp_load_parameters change the behavior of a running machine. Call them only when the user asked for exactly
this change, and report the old and the new value.
Measurement data is written to files, tool results contain summaries only.
```

### 5.2 `xcp_status`

No arguments. Works in every state and is the first call of a conversation.

```text
State: measuring
Target: hello_xcp at 192.168.0.206:5555 (UDP), connected for 4 min
EPK: v10_21_15_02, matches hello_xcp.a2l
XCP: protocol 1.4, MAX_CTO 248, MAX_DTO 1024, resources CAL DAQ
Description: hello_xcp.a2l (uploaded), 3 events, 2 calibration segments, 12 signals, 3 parameters
Measurement: 4 signals, running 12.4 s of 60 s, 12400 events, 1000 event/s, 0 packets lost, hello_xcp_20261128_140530.mf4
Target messages: (last SERV_TEXT lines, if any)
```

In state `disconnected` after a loss: `State: disconnected. Connection to hello_xcp at 192.168.0.206:5555 lost 2 min ago (no response to SHORT_UPLOAD).`

### 5.3 `xcp_connect`

| Argument | Type | Default | |
|---|---|---|---|
| `host` | string | `--dest-addr` | IP address of the target |
| `port` | integer | `--port`, 5555 | |
| `protocol` | `udp`, `tcp` | `--udp`/`--tcp`, else `udp` | See open decision 2 |
| `a2l` | string | `--a2l` | A2L file to use instead of an upload |
| `elf` | string | `--elf` | ELF file to create the A2L file from |
| `default_event` | string | `--default-event` | Event id or name for variables without a fixed event |
| `accept_epk_mismatch` | boolean | false | Only after the user confirmed |

Result: the lines `Target`, `EPK`, `XCP` and `Description` of `xcp_status`, and warnings (event ids differ, no A2L available). Errors: `Target 192.168.0.206:5555 does not respond (UDP, 3 s). Is the application running, is the protocol right?`, `Already connected to hello_xcp at 192.168.0.206:5555. Call xcp_disconnect first.`, `EPK mismatch: ...`.

### 5.4 `xcp_disconnect`

No arguments. Stops a running measurement (the result contains its statistics), sends `DISCONNECT`, forgets the description.

### 5.5 `xcp_list_measurements`, `xcp_list_parameters`

| Argument | Type | Default | |
|---|---|---|---|
| `pattern` | string | `.` | Regular expression on the name, as `--list-mea` and `--list-cal` |
| `limit` | integer | 50 | Maximum 200 |
| `offset` | integer | 0 | For the next page |

One line per object, parameters with their current value:

```text
3 of 3 parameters matching 'counter|delay'
counter_max  u16  = 1000   limits 0..65535   segment params
delay_us     u32  = 1000   limits 0..999999  unit us  segment params
...
```

```text
50 of 1234 signals matching '.', next page: offset 50. Use a more specific pattern.
SigGen1.input    f64      unit Volt  event SigGen1.clip
matrix_f32       f32[8x4]            event mainloop
global_counter   u32                 no event, needs default_event
...
```

### 5.6 `xcp_read`

| Argument | Type | |
|---|---|---|
| `names` | array of strings | Exact names of parameters or signals, at most 50 |

```text
counter_max = 1000                     (parameter, polled)
SigGen1.input = -17.514 Volt           (measured, sample 3 ms old)
state = RUNNING                        (enum, polled)
local_var: can not be uploaded and is not part of a running measurement, use xcp_sample
```

`xcp_read` never starts a measurement, see 4.5.

Values are physical values with unit and enum labels, as in the CSV output. Arrays show the first 16 elements, as the console output.

### 5.7 `xcp_write`

| Argument | Type | |
|---|---|---|
| `name` | string | Exact name of a parameter |
| `value` | number | Checked against the A2L limits |

The value is written with `SHORT_DOWNLOAD` and read back:

```text
counter_max: 1000 -> 2000 (limits 0..65535, segment params, working page)
```

Errors: `counter_max: value 70000 exceeds the limits 0..65535, not written`, `counter_maxx not found. Similar: counter_max`. Scope of the first version as `--cal`: scalar parameters of basic types. Curves, maps and enum labels as input are later.

The change is in the working page (RAM) of the target. Whether it survives a restart depends on the target (persistence, freeze), the result says so if the target reports freeze support.

### 5.8 `xcp_sample`

| Argument | Type | Default | |
|---|---|---|---|
| `names` | array of strings | | Exact signal names, at most 20 |
| `duration_ms` | integer | 1000 | Maximum 30000 |

```text
Sampled 2 signals for 3.0 s, 0 packets lost
counter        3000 samples, 1000/s   min 0        max 1999     mean 999.5    last 1042
SigGen1.input  120000 samples, 40000/s  min -17.51  max 17.51  mean 0.002  last 3.07  Volt
```

Sends a progress notification every second. A cancelled call stops the measurement.

### 5.9 `xcp_measure_start`, `xcp_measure_stop`

`xcp_measure_start`:

| Argument | Type | Default | |
|---|---|---|---|
| `names` | array of strings | | Exact signal names, or |
| `pattern` | string | | a regular expression, as `--mea` |
| `duration_s` | integer | 0 | 0: until `xcp_measure_stop` |
| `mdf`, `mcap`, `csv`, `perfetto` | string | | Output files in the MCP directory. None given: MDF with the name `<target>_<date>_<time>.mf4` |
| `trace_auto` | `off`, `context`, `links`, `all` | `--trace-auto` | With `perfetto` |

Returns immediately: the number of signals and DAQ lists, signals which were skipped and why (no event, array too large), the file names. An existing file is not overwritten.

`xcp_measure_stop` has no arguments and returns the final statistics, as the command line prints them: duration, events, rates, packets, lost packets, dropped samples, files with their sizes. After a measurement which ended by itself (`duration_s`), `xcp_status` shows this summary until the next measurement starts.

### 5.10 A2L and parameter file tools (phase 4)

- `xcp_load_a2l {a2l?, elf?, fix?}`: load or replace the description of the connected target, `fix` as `--fix-a2l`.
- `xcp_create_a2l {elf, a2l, template?, embed_aml?, var_filter?, unit_filter?, skip_no_metadata?}`: create an A2L file from an ELF file, offline or with the event and segment information of the connected target. Result: counts, and the warnings and diagnostics described in [OFFLINE_A2L.md](OFFLINE_A2L.md).
- `xcp_save_parameters {file}`: upload all calibration segments into a `.hex` or `.bin` file.
- `xcp_load_parameters {file}`: download a `.hex` or `.bin` file into the target. Changes the target.

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
mcp = ["dep:rmcp", "dep:axum"]

[dependencies]
rmcp = { version = "3.12", optional = true, features = ["server", "macros", "transport-io", "transport-streamable-http-server"] }
axum = { version = "0.8", optional = true }      # HTTP server for --mcp-http, rmcp provides a tower service
serde_json = "1"                                  # today a dev-dependency
```

A tool is a function with a description, its arguments are a struct with a derived JSON schema. The doc comments of the fields become the argument descriptions the model sees. Sketch, following the rmcp examples, not compiled:

```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ConnectArgs {
    /// IP address of the target
    host: Option<String>,
    /// XCP port, default 5555
    port: Option<u16>,
    /// Transport protocol
    protocol: Option<Protocol>,
    // ...
}

#[derive(Clone)]
struct XcpMcpServer {
    session: SessionHandle, // sender of the command channel to the session actor
}

#[tool_router]
impl XcpMcpServer {
    #[tool(description = "Connect to an XCP target (ECU) and load its A2L description. ...")]
    async fn xcp_connect(&self, Parameters(args): Parameters<ConnectArgs>) -> Result<CallToolResult, McpError> {
        match self.session.connect(args.into()).await {
            Ok(info) => Ok(CallToolResult::success(vec![ContentBlock::text(info.to_string())])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(e.to_string())])), // isError: true, for the model
        }
    }

    #[tool(description = "State of the XCP connection ...", annotations(read_only_hint = true))]
    async fn xcp_status(&self) -> Result<CallToolResult, McpError> { /* ... */ }
}

#[tool_handler(name = "xcpclient", version = env!("CARGO_PKG_VERSION"), instructions = INSTRUCTIONS)]
impl ServerHandler for XcpMcpServer {}

// --mcp
let service = XcpMcpServer::new(session).serve(rmcp::transport::stdio()).await?;
service.waiting().await?; // returns when stdin is closed

// --mcp-http
let service = StreamableHttpService::new(move || Ok(XcpMcpServer::new(session.clone())), LocalSessionManager::default().into(),
    StreamableHttpServerConfig::default().with_allowed_origins([/* localhost origins */]));
axum::serve(TcpListener::bind(addr).await?, axum::Router::new().route_service("/mcp", service)).await?;
```

Every host connection gets its own `XcpMcpServer`, all share one `SessionHandle`, so all see the same XCP session. Note that Origin validation is off by default in rmcp 3.12 and has to be configured.

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

Still to do in phase 0, see 8.3: typed results and a `SessionError` where the tools need them (`connect` and `load_description` only log what they find), stdout and panic cleanup.

Step two adds:

```
src/session/actor.rs     SessionHandle and the actor task: commands over mpsc, replies over oneshot, timers
src/value_writer/mod.rs  DaqWriter with the last sample per DAQ list and window statistics
src/mcp/mod.rs           the rmcp server: tools, argument structs, result formatting, instructions
src/mcp/transport.rs     stdio and HTTP startup, shutdown handling
```

The typed results implement `Display` in the line format shown in section 5. The command line prints them, the MCP layer returns them, both show the same text.

### 8.3 What a long running server needs from the existing code

| Topic | Today | Needed |
|---|---|---|
| stdout | `env_logger` logs to stdout, `println!`/`print!` in `main.rs`, `xcp_client/decoder.rs`, `elf_reader`, `bin_reader`, `xcp_test_executor.rs`, `ConsoleWriter::stdout()` | In MCP mode the logger target is stderr and nothing prints to stdout. On stdio, one stray line on stdout breaks the protocol |
| `SERV_TEXT` | Printed to stdout (`ServTextDecoder` in `main.rs`) | Kept in a ring buffer of the session, shown by `xcp_status` |
| Panics | `assert!`, `panic!("Empty string")`, `unwrap()` on target responses and on the registry (`self.registry.as_ref().unwrap()`) | Errors on the paths the tools use. As a safety net the actor runs each command so that a panic is caught, reported as tool error and the session reset to `disconnected` |
| Errors | `Box<dyn Error>`, which is not `Send` | `SessionError` (a `thiserror` enum, `Send`) at the boundary of the session, with messages written for the model. If the `XcpClient` futures are not `Send`, the actor runs on its own thread with a current-thread runtime, nothing else changes |
| EPK mismatch | Interactive prompt on stdin, or `--yes` | Decision as argument of `Session::load_description`, the prompt stays in `main.rs` |
| Calibration objects | `calibration_object_list` only grows, `create_calibration_object` pushes a new object on every call (the measurement object list is cleared at measurement stop) | Look up before create. A daemon which reads a parameter every few seconds leaks otherwise |
| Reconnect | One connect per process | `XcpClient` is created per connect and dropped at disconnect, the receive task ends with it |
| Shutdown | Disconnect at the end of `xcp_client()` | One shutdown path for stdin closed, SIGTERM and Ctrl-C: stop measurement, finalize files, `DISCONNECT` (R9) |

### 8.4 Tests

- **Unit**: the session state machine and the result formatting against the fake transport which the tests in `src/xcp_client/cal.rs` already use.
- **Integration**, registered with CTest as the other integration tests: start `hello_xcp` from `build/`, start `xcpclient --mcp` as child process, send JSON-RPC lines to its stdin and check the results: connect, list, read, write and read back, sample, measure into an MDF file, kill the target and check for `disconnected`, connect again. A second run with a legacy `initialize` handshake.
- **stdout hygiene**: the integration test fails on any line on stdout which is not a JSON-RPC message.
- **Manual**: the MCP Inspector, a debugger for MCP servers with a web UI:

  ```bash
  npx @modelcontextprotocol/inspector --protocol-era auto xcpclient --mcp
  ```

  It lists the tools as the model sees them and calls them with arguments from a form. Use it to read the descriptions with fresh eyes before trying them with a model.

### 8.5 Phases

| Phase | Content | Result |
|---|---|---|
| 0 | Session refactoring (8.2 step one), stdout and panic cleanup (8.3). No MCP code | Command line unchanged, `test/test_examples.sh` passes |
| 1 | `--mcp` on stdio, session actor, tools `xcp_status`, `xcp_connect`, `xcp_disconnect`, `xcp_list_*`, `xcp_read` (polling), `xcp_write`, `--mcp-read-only` | The conversation of section 1 works in Claude Code, except the last sentence |
| 2 | ValueWriter, `xcp_sample`, `xcp_measure_start`, `xcp_measure_stop`, `xcp_read` of measured signals from the ValueWriter | Measurement |
| 3 | `--mcp-http`, clean shutdown as daemon | The connection survives host sessions (R1 in full) |
| 4 | `xcp_load_a2l`, `xcp_create_a2l`, `xcp_save_parameters`, `xcp_load_parameters`, `--mcp-dir` checks | Feature parity with the command line |

## 9. Setup in the hosts

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
| 1 | Which transport first | stdio first, the HTTP daemon in phase 3 |
| 4 | Targets per server process | One. Two targets (Pi and ESP32) are two servers, registered for example as `xcp-pi` and `xcp-esp32` |
| 5 | Parameter writes | Available by default, xcpclient is a lab tool. The host asks for permission, `--mcp-read-only` switches them off |
| | Reading values | `xcp_read` never starts a measurement: last sample of a running measurement or upload, otherwise refused (4.5) |
| | Alive polling | None (4.6). Noted for later, see below |
| | Security | Deferred, see section 7 |

### Open, the recommendation is used until decided otherwise

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

- **Alive check.** No keep-alive polling for now, a lost target is detected by the next command or by missing DAQ packets. To be reconsidered with the daemon (phase 3), where `xcp_status` of a connection which is idle for hours should be true. It depends on the outcome of the point above: a polling client must not disturb another client.

## 11. Sources

Looked up on 2026-11-28:

- MCP specification, revision 2026-07-28: [versioning](https://modelcontextprotocol.io/specification/versioning), [changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog), [transports](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports), [tools](https://modelcontextprotocol.io/specification/2026-07-28/server/tools), [progress](https://modelcontextprotocol.io/specification/2026-07-28/basic/patterns/progress), [elicitation](https://modelcontextprotocol.io/specification/2026-07-28/client/elicitation)
- Rust SDK: [crates.io/crates/rmcp](https://crates.io/crates/rmcp), [github.com/modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk) (README and `examples/servers`)
- Claude Code: [MCP](https://code.claude.com/docs/en/mcp), [environment variables](https://code.claude.com/docs/en/env-vars) (timeouts, `MAX_MCP_OUTPUT_TOKENS`)
- Claude Desktop: [local servers](https://modelcontextprotocol.io/docs/develop/connect-local-servers), [remote servers](https://modelcontextprotocol.io/docs/develop/connect-remote-servers), [mcp-remote](https://github.com/geelen/mcp-remote)
- MCP Inspector: [github.com/modelcontextprotocol/inspector](https://github.com/modelcontextprotocol/inspector)

Not verified: the rmcp code in 8.1 is not compiled, the attribute names of the annotations other than `read_only_hint` are assumed. How Claude Desktop handles a local HTTP server is taken from its documentation, not tried.
