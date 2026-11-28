//-----------------------------------------------------------------------------
// Integration test of the MCP server mode: xcpclient --mcp as a child process, MCP messages on its stdin and stdout
//
// The test needs no XCP target. It checks what can go wrong without one: the protocol on the real stdio transport,
// that nothing but MCP messages is written to stdout, the tool list and the tool errors of a server which is not connected.
// The tools with a target are tested by hand or by a script against an XCPlite example, see docs/MCP.md.

#![cfg(feature = "mcp")]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

struct McpClient {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
    next_id: u64,
}

impl McpClient {
    // Start xcpclient --mcp with additional arguments
    fn start(args: &[&str]) -> McpClient {
        let mut child = Command::new(env!("CARGO_BIN_EXE_xcpclient"))
            .arg("--mcp")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start xcpclient --mcp");
        let stdin = child.stdin.take();
        let stdout: ChildStdout = child.stdout.take().unwrap();
        // Read the lines of stdout in a thread, to wait for them with a timeout
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        McpClient { child, stdin, lines, next_id: 0 }
    }

    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{}", message).unwrap();
        stdin.flush().unwrap();
    }

    // Send a request and wait for its response
    // Every line on stdout must be a JSON-RPC message
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let line = self.lines.recv_timeout(Duration::from_secs(20)).expect("response of xcpclient --mcp");
            let message: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("not an MCP message on stdout: {:?} ({})", line, e));
            assert_eq!(message["jsonrpc"], "2.0", "{}", line);
            if message["id"] == json!(id) {
                return message;
            }
        }
    }

    // Protocol revisions up to 2025-11-25 start with a handshake
    fn initialize(&mut self) -> Value {
        let response = self.request(
            "initialize",
            json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } }),
        );
        self.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        response
    }

    // Call a tool, returns (isError, text)
    fn call(&mut self, tool: &str, arguments: Value) -> (bool, String) {
        let response = self.request("tools/call", json!({ "name": tool, "arguments": arguments }));
        let result = &response["result"];
        assert!(result.is_object(), "tool call failed with a protocol error: {}", response);
        let text = result["content"][0]["text"].as_str().unwrap_or_default().to_string();
        (result["isError"].as_bool().unwrap_or(false), text)
    }

    // Close stdin, the server terminates with exit status 0 and has written nothing else to stdout
    fn close(mut self) {
        drop(self.stdin.take());
        let status = self.child.wait().unwrap();
        assert!(status.success(), "exit status {:?}", status);
        assert!(self.lines.recv_timeout(Duration::from_secs(5)).is_err(), "output on stdout after the last response");
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn tool_names(response: &Value) -> Vec<String> {
    let mut names: Vec<String> = response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

#[test]
fn test_mcp_stdio_without_target() {
    // Log level trace: a lot of log output, none of it may reach stdout
    let mut mcp = McpClient::start(&["--log-level", "5", "--dest-addr", "127.0.0.1", "--port", "5999", "--tcp"]);

    let response = mcp.initialize();
    assert_eq!(response["result"]["serverInfo"]["name"], "xcpclient");
    assert_eq!(response["result"]["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(response["result"]["capabilities"]["tools"].is_object());
    assert!(response["result"]["instructions"].as_str().unwrap().contains("xcp_status"));

    let tools = mcp.request("tools/list", json!({}));
    assert_eq!(
        tool_names(&tools),
        [
            "xcp_connect",
            "xcp_create_a2l",
            "xcp_disconnect",
            "xcp_list_measurements",
            "xcp_list_parameters",
            "xcp_load_a2l",
            "xcp_load_parameters",
            "xcp_measure_start",
            "xcp_measure_stop",
            "xcp_read",
            "xcp_sample",
            "xcp_save_parameters",
            "xcp_status",
            "xcp_write"
        ]
    );
    // The hints a host uses to ask the user for confirmation
    for tool in tools["result"]["tools"].as_array().unwrap() {
        let changes_target = tool["name"] == "xcp_write" || tool["name"] == "xcp_load_parameters";
        assert_eq!(tool["annotations"]["destructiveHint"].as_bool().unwrap_or(false), changes_target, "{}", tool["name"]);
    }

    // Not connected: status shows the default target, the other tools say what to do
    let (is_error, text) = mcp.call("xcp_status", json!({}));
    assert!(!is_error);
    assert!(text.starts_with("State: disconnected"), "{}", text);
    assert!(text.contains("127.0.0.1 port 5999 (TCP)"), "{}", text);
    for (tool, arguments) in [
        ("xcp_read", json!({ "names": ["counter"] })),
        ("xcp_write", json!({ "name": "counter", "value": 1 })),
        ("xcp_list_parameters", json!({})),
        ("xcp_list_measurements", json!({ "pattern": "counter" })),
        ("xcp_sample", json!({ "names": ["counter"] })),
        ("xcp_measure_start", json!({ "pattern": "." })),
        ("xcp_load_a2l", json!({})),
        ("xcp_save_parameters", json!({})),
        ("xcp_load_parameters", json!({ "file": "p.hex" })),
    ] {
        let (is_error, text) = mcp.call(tool, arguments);
        assert!(is_error, "{}", tool);
        assert_eq!(text, "Not connected. Call xcp_connect first.", "{}", tool);
    }

    let (is_error, text) = mcp.call("xcp_measure_stop", json!({}));
    assert!(is_error);
    assert_eq!(text, "No measurement is running.");

    // A target which refuses the connection: a tool error with a hint, the server goes on
    let (is_error, text) = mcp.call("xcp_connect", json!({}));
    assert!(is_error);
    assert!(text.starts_with("Could not connect to 127.0.0.1:5999 (TCP)"), "{}", text);
    assert!(mcp.call("xcp_status", json!({})).1.starts_with("State: disconnected"));

    // A missing argument is a tool error the model can correct, an unknown tool is a protocol error
    let (is_error, text) = mcp.call("xcp_write", json!({ "name": "counter" }));
    assert!(is_error);
    assert!(text.contains("value"), "{}", text);
    let response = mcp.request("tools/call", json!({ "name": "no_such_tool", "arguments": {} }));
    assert_eq!(response["error"]["code"], -32602, "{}", response);

    mcp.close();
}

// Protocol revision 2026-07-28: no handshake, every request carries the protocol version
#[test]
fn test_mcp_stdio_stateless() {
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "0" },
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    let mut mcp = McpClient::start(&["--log-level", "0"]);
    let response = mcp.request("server/discover", json!({ "_meta": meta }));
    assert!(response["result"]["supportedVersions"].as_array().unwrap().contains(&json!("2026-07-28")), "{}", response);
    let response = mcp.request("tools/call", json!({ "name": "xcp_status", "arguments": {}, "_meta": meta }));
    assert_eq!(response["result"]["resultType"], "complete", "{}", response);
    assert!(response["result"]["content"][0]["text"].as_str().unwrap().starts_with("State: disconnected"));
    mcp.close();
}

// --mcp-read-only: the tools which change the target do not exist
#[test]
fn test_mcp_stdio_read_only() {
    let mut mcp = McpClient::start(&["--mcp-read-only", "--log-level", "0"]);
    mcp.initialize();
    let tools = mcp.request("tools/list", json!({}));
    assert_eq!(
        tool_names(&tools),
        [
            "xcp_connect",
            "xcp_create_a2l",
            "xcp_disconnect",
            "xcp_list_measurements",
            "xcp_list_parameters",
            "xcp_load_a2l",
            "xcp_measure_start",
            "xcp_measure_stop",
            "xcp_read",
            "xcp_sample",
            "xcp_save_parameters",
            "xcp_status"
        ]
    );
    let response = mcp.request("tools/call", json!({ "name": "xcp_write", "arguments": { "name": "counter", "value": 1 } }));
    assert!(response["error"].is_object(), "{}", response);
    mcp.close();
}

// The options which select an action of the command line are rejected with --mcp, nothing is written to stdout
#[test]
fn test_mcp_rejects_action_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_xcpclient"))
        .args(["--mcp", "--mea", "counter"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--mea can not be used with --mcp"));
}
