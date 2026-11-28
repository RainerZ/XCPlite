# MCP scenarios

End-to-end tests of the MCP server mode of xcpclient (`xcpclient --mcp`, see [docs/MCP.md](../../../../docs/MCP.md)) against a running XCPlite target.

A scenario starts `xcpclient --mcp` as a child process, sends MCP messages to its stdin as a host would, and checks the tool results. The target is `build/hello_xcp`. Each tool call and its result are printed, a scenario ends with `OK` or with the list of failed expectations and exit status 1.

These scenarios need a built target and take about a minute. The tests which need no target are in `cargo test` (`tests/mcp_stdio.rs` and the unit tests).

## Run

```bash
./build.sh                                # build/hello_xcp
cargo build --release                     # in tools/xcpclient
python3 tests/mcp/run_all.py              # all scenarios
python3 tests/mcp/measure.py              # one scenario, with its output
```

Python 3 without additional packages. Tested on macOS.

`run_all.py` needs port 5555 free: stop a running XCP server first.

| Scenario | Covers | Target |
|---|---|---|
| `basic.py` | Connect with A2L upload, status, lists with paging, read, write with read back, concurrent calls, a lost connection, reconnect, shutdown | own |
| `errors.py` | `--mcp-read-only`, TCP against a UDP target, an unreachable target, a host name, SIGTERM, an option which is not allowed with `--mcp` | any |
| `stateless.py` | Protocol revision 2026-07-28 without the initialize handshake | any |
| `measure.py` | `xcp_sample`, a cancelled sample, recordings into MDF, CSV, MCAP and Perfetto files, timed recordings, read and status while measuring, disconnect and shutdown during a recording | any |
| `measure_lost.py` | The target dies during a recording | own |
| `files.py` | `xcp_create_a2l`, `xcp_load_a2l`, the EPK of an ELF file against the target, `xcp_save_parameters`, `xcp_load_parameters` | any |
| `same_directory.py` | xcpclient in the working directory of the target: the A2L file of the target keeps its content | own |

Target "own": the scenario stops and restarts the target, or starts it in a certain directory. It refuses to run while an XCP server is running on port 5555. Target "any": the scenario uses an XCP server which is already running, otherwise it starts `hello_xcp` and stops it at the end. A scenario takes over the XCP session of the target, another client connected to it is disconnected.

## Environment

| Variable | Default | |
|---|---|---|
| `XCPCLIENT` | The newer of `target/release/xcpclient` and `target/debug/xcpclient` | The xcpclient to test |
| `XCP_TARGET` | `build/hello_xcp` | The target to start. The expectations are those of `hello_xcp` |
| `XCP_MCP_WORK` | A new temporary directory | Working directory of the run, printed at the end |

Nothing is written into the repository. The target runs in `srv` of the working directory, xcpclient in `cli`, and its own files (uploaded A2L file) go to `cli/cache`, not to the cache directory of the user.

## Writing a scenario

`mcplib.py` has the pieces: `Target` starts the target, `Mcp` starts xcpclient and has `call(tool, arguments, ok=..., contains=..., lacks=...)`, which prints the result and checks it. `ok=False` expects a tool error (`isError`), `ok=None` a protocol error. `start_call` and `result` are for a call which is answered later. `check(condition, what)` records another expectation, `finish()` prints the summary and sets the exit status.
