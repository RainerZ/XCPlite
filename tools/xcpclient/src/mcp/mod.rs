//-----------------------------------------------------------------------------
// Module mcp
// MCP server mode of xcpclient (--mcp), see docs/MCP.md
//
// An MCP server offers tools to a language model: functions with a name, a description and typed arguments. This module defines
// the tools of xcpclient with the MCP SDK (rmcp) and serves them on stdin/stdout. It has no XCP knowledge: a tool forwards its
// arguments to the session actor (session::SessionHandle) and returns the text of the reply. A reply Err is a tool execution
// error (isError), the model reads it and reacts.
//
// The names, descriptions and argument documentation in this file are the interface to the model: it has nothing else to
// decide which tool to call and how. The doc comments of the argument struct fields become the argument descriptions.

use std::error::Error;

#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::session::{ConnectRequest, CreateA2lRequest, LIST_LIMIT_DEFAULT, ListRequest, LoadA2lRequest, MeasureRequest, SampleRequest, SessionHandle, TransportProtocol};

mod stdio;
pub use stdio::{McpOutput, protect_stdout};

//------------------------------------------------------------------------
// Tool arguments

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum ProtocolArg {
    Udp,
    Tcp,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ConnectArgs {
    /// IP address or host name of the target, for example "192.168.0.206". Omit it to use the default target of this server.
    host: Option<String>,
    /// XCP port of the target. Default 5555.
    port: Option<u16>,
    /// Transport protocol. Default udp.
    protocol: Option<ProtocolArg>,
    /// Path of an A2L file which describes the target. Omit it to upload the A2L file from the target.
    a2l: Option<String>,
    /// Path of the ELF file of the application on the target (Linux or embedded build with debug information). The A2L file is
    /// created from it. Needed for targets which have no A2L file to upload.
    elf: Option<String>,
    /// Event id or event name used to measure variables which have no event of their own (global variables).
    default_event: Option<String>,
    /// Use the given A2L file although its EPK (software version) differs from the EPK of the target. Set it to true only after the
    /// user confirmed this.
    accept_epk_mismatch: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListArgs {
    /// Regular expression matched against the names, for example "counter" or "^SigGen1\\.". Default: all.
    pattern: Option<String>,
    /// Maximum number of entries returned. Default 50, maximum 200.
    limit: Option<usize>,
    /// Number of matches to skip, to get the next page of a long list. Default 0.
    offset: Option<usize>,
}

impl From<ListArgs> for ListRequest {
    fn from(args: ListArgs) -> ListRequest {
        ListRequest {
            pattern: args.pattern.unwrap_or_default(),
            limit: args.limit.unwrap_or(LIST_LIMIT_DEFAULT),
            offset: args.offset.unwrap_or(0),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ReadArgs {
    /// Exact names of parameters or signals, as listed by xcp_list_parameters and xcp_list_measurements. At most 50.
    names: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WriteArgs {
    /// Exact name of a calibration parameter, as listed by xcp_list_parameters.
    name: String,
    /// The new value. It is checked against the limits of the parameter.
    value: f64,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SampleArgs {
    /// Exact names of measurement signals, as listed by xcp_list_measurements. At most 20.
    names: Vec<String>,
    /// How long to observe the signals, in milliseconds. Default 1000, maximum 30000.
    duration_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct MeasureStartArgs {
    /// Exact names of the signals to record, as listed by xcp_list_measurements. Give either names or pattern.
    names: Option<Vec<String>>,
    /// Regular expression which selects the signals to record by name, "." for all signals. Give either names or pattern.
    pattern: Option<String>,
    /// Stop the recording by itself after this number of seconds. Default: it runs until xcp_measure_stop.
    duration_s: Option<u64>,
    /// File name for an ASAM MDF 4 file (.mf4), the format for CANape and other measurement tools. This is the default: if no
    /// file is given at all, an MDF file named after the target and the time is written.
    mdf: Option<String>,
    /// File name for an MCAP file (.mcap), for Foxglove and ROS tools.
    mcap: Option<String>,
    /// File name for a CSV file with one row per sample: time_ns, daq, name, value, unit.
    csv: Option<String>,
    /// File name for a Perfetto trace file (.json) with spans, signals and span links, to open with ui.perfetto.dev.
    perfetto: Option<String>,
    /// With perfetto: which span and link signals are added to the recording: "off", "context", "links" (default) or "all".
    trace_auto: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LoadA2lArgs {
    /// Path of an A2L file which describes the target. Give either a2l or elf, or neither to upload the A2L file from the target.
    a2l: Option<String>,
    /// Path of the ELF file of the application on the target. The A2L file is created from it and from the events and calibration
    /// segments the target reports.
    elf: Option<String>,
    /// With a2l: correct the event ids and calibration segment numbers in the A2L file with those of the target and rewrite the file
    /// (the original is kept as .bak). Only for A2L files created by xcpclient. Use it when a tool reported that they differ.
    fix: Option<bool>,
    /// Event id or event name used to measure variables which have no event of their own (global variables).
    default_event: Option<String>,
    /// Use the file although its EPK (software version) differs from the EPK of the target. Set it to true only after the user
    /// confirmed this.
    accept_epk_mismatch: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct CreateA2lArgs {
    /// Path of the ELF file of the application, a Linux or embedded build with DWARF debug information. Not a macOS executable.
    elf: String,
    /// Path of the A2L file to write.
    a2l: String,
    /// Replace the A2L file if it exists. Default false: an existing file is an error.
    overwrite: Option<bool>,
    /// Write a template with the events and calibration segments only, without variables. Default false.
    template: Option<bool>,
    /// Do not use the connected target, create the A2L file from the ELF file alone. Default false: if a target is connected, its
    /// events and calibration segments are used and its EPK is compared with the ELF file.
    offline: Option<bool>,
    /// Regular expression: only variables with matching names are described. Default: all.
    var_filter: Option<String>,
    /// Regular expression: only variables of compilation units (source files) with matching names are described. Default: all.
    unit_filter: Option<String>,
    /// Describe only variables which have metadata (unit, limits or comment) in the source code. Default false.
    skip_no_metadata: Option<bool>,
    /// Embed the XCP A2ML description into the A2L file instead of including XCP_104.aml. Default false.
    embed_aml: Option<bool>,
    /// Event id or event name assigned to variables which have no event of their own (global variables).
    default_event: Option<String>,
    /// Create the file although the EPK (software version) of the ELF file differs from the EPK of the connected target. Set it to
    /// true only after the user confirmed this.
    accept_epk_mismatch: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SaveParametersArgs {
    /// File name with the extension .hex (Intel-HEX with the data of the calibration segments, can be loaded again) or .bin
    /// (XCPlite persistence file). Default: a .hex file named after the target and the time.
    file: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LoadParametersArgs {
    /// Path of an Intel-HEX file (.hex) written by xcp_save_parameters for this software version.
    file: String,
}

//------------------------------------------------------------------------
// The MCP server

/// The MCP server of xcpclient, all instances share the one XCP session
#[derive(Clone)]
pub struct XcpMcpServer {
    session: SessionHandle,
    tool_router: ToolRouter<XcpMcpServer>,
}

#[tool_router]
impl XcpMcpServer {
    /// read_only: do not offer the tools which change the target
    pub fn new(session: SessionHandle, read_only: bool) -> XcpMcpServer {
        let mut tool_router = Self::tool_router();
        if read_only {
            tool_router.remove_route("xcp_write");
            tool_router.remove_route("xcp_load_parameters");
        }
        XcpMcpServer { session, tool_router }
    }

    #[tool(
        description = "Show the state of the XCP connection: whether a target (ECU) is connected, its name, address and software version (EPK), \
                       the loaded A2L description with the number of signals and parameters, and messages of the target. \
                       Call this first. The connection is kept between calls, a target which is already connected must not be connected again.",
        annotations(title = "XCP status", read_only_hint = true, open_world_hint = false)
    )]
    async fn xcp_status(&self) -> Result<String, String> {
        self.session.status().await
    }

    #[tool(
        description = "Connect to an XCP target (ECU) and load its description (A2L): the names, types and addresses of its measurement \
                       signals and calibration parameters. By default the A2L file is uploaded from the target. If the target has none, give \
                       the ELF file of its application (elf) or an A2L file (a2l). \
                       If the same target is already connected, nothing happens, unless a2l or elf is given: then the description is loaded again. \
                       Connecting to another target requires xcp_disconnect first. Returns the status.",
        annotations(title = "XCP connect", read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn xcp_connect(&self, Parameters(args): Parameters<ConnectArgs>) -> Result<String, String> {
        let request = ConnectRequest {
            host: args.host,
            port: args.port,
            protocol: args.protocol.map(|p| match p {
                ProtocolArg::Udp => TransportProtocol::Udp,
                ProtocolArg::Tcp => TransportProtocol::Tcp,
            }),
            a2l: args.a2l,
            elf: args.elf,
            default_event: args.default_event,
            accept_epk_mismatch: args.accept_epk_mismatch.unwrap_or(false),
        };
        self.session.connect(request).await
    }

    #[tool(
        description = "Disconnect from the XCP target.",
        annotations(title = "XCP disconnect", read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn xcp_disconnect(&self) -> Result<String, String> {
        self.session.disconnect().await
    }

    #[tool(
        description = "List the measurement signals of the connected target whose names match a regular expression, one per line: \
                       name, type (with array dimensions), unit and the event which samples it. \
                       Use it to find the exact name of a signal, never guess names. Long lists are paged with limit and offset.",
        annotations(title = "XCP list measurement signals", read_only_hint = true, open_world_hint = false)
    )]
    async fn xcp_list_measurements(&self, Parameters(args): Parameters<ListArgs>) -> Result<String, String> {
        self.session.list_measurements(args.into()).await
    }

    #[tool(
        description = "List the calibration parameters of the connected target whose names match a regular expression, one per line: \
                       name, type, current value read from the target and limits. \
                       Use it to find the exact name of a parameter, never guess names. Long lists are paged with limit and offset.",
        annotations(title = "XCP list calibration parameters", read_only_hint = true, open_world_hint = false)
    )]
    async fn xcp_list_parameters(&self, Parameters(args): Parameters<ListArgs>) -> Result<String, String> {
        self.session.list_parameters(args.into()).await
    }

    #[tool(
        description = "Read the current values of calibration parameters and measurement signals from the target, given by their exact names. \
                       The values are read once (polling). A signal which can not be read this way, for example a local variable on the stack \
                       of a function, is reported as not readable, the other values are still returned.",
        annotations(title = "XCP read values", read_only_hint = true, open_world_hint = false)
    )]
    async fn xcp_read(&self, Parameters(args): Parameters<ReadArgs>) -> Result<String, String> {
        self.session.read(args.names).await
    }

    #[tool(
        description = "Set a calibration parameter of the target to a new value. This changes the behavior of the running application on the \
                       target immediately. Call it only when the user asked for exactly this change, never to try something out. \
                       The value is checked against the limits of the parameter, written and read back. Returns the old and the new value, \
                       report both to the user. Only scalar parameters, signals can not be written.",
        annotations(
            title = "XCP write parameter",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn xcp_write(&self, Parameters(args): Parameters<WriteArgs>) -> Result<String, String> {
        self.session.write(args.name, args.value).await
    }

    #[tool(
        description = "Observe measurement signals for some seconds and return statistics for each: number of samples, sample rate, minimum, \
                       maximum, mean and last value. Use it to see how a signal behaves over time, for example its amplitude, its range or \
                       whether it changes at all. The call returns when the time is over. No file is written. \
                       For a single current value use xcp_read. While a recording (xcp_measure_start) is running, only its signals can be observed.",
        annotations(
            title = "XCP sample signals",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn xcp_sample(&self, Parameters(args): Parameters<SampleArgs>, context: RequestContext<RoleServer>) -> Result<String, String> {
        let sample = self.session.sample(SampleRequest {
            names: args.names,
            duration_ms: args.duration_ms,
        });
        // A cancelled call ends the wait, the session actor then closes the sample window
        tokio::select! {
            result = sample => result,
            _ = context.ct.cancelled() => Err("xcp_sample was cancelled".to_string()),
        }
    }

    #[tool(
        description = "Start recording measurement signals into a file (MDF by default, also MCAP, CSV or a Perfetto trace). \
                       The recording runs in the background, this call returns immediately. It ends with xcp_measure_stop or by itself after \
                       duration_s. Select the signals by exact names or by a regular expression. Only one recording at a time. \
                       The measurement data is in the files, tool results contain summaries only. Existing files are not overwritten. \
                       xcp_status shows the progress, xcp_read returns the latest values of the recorded signals.",
        annotations(
            title = "XCP start recording",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn xcp_measure_start(&self, Parameters(args): Parameters<MeasureStartArgs>) -> Result<String, String> {
        self.session
            .measure_start(MeasureRequest {
                names: args.names.unwrap_or_default(),
                pattern: args.pattern,
                duration_s: args.duration_s.unwrap_or(0),
                mdf: args.mdf,
                mcap: args.mcap,
                csv: args.csv,
                perfetto: args.perfetto,
                trace_auto: args.trace_auto,
            })
            .await
    }

    #[tool(
        description = "Stop the recording started with xcp_measure_start and complete its files. Returns the summary: duration, number of \
                       events, rates, lost packets and the files with their sizes.",
        annotations(
            title = "XCP stop recording",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn xcp_measure_stop(&self) -> Result<String, String> {
        self.session.measure_stop().await
    }

    #[tool(
        description = "Load or replace the A2L description of the connected target: from an A2L file (a2l), created from the ELF file of the \
                       application (elf), or uploaded from the target (no argument). Use it when xcp_connect could not load a description, or after \
                       the software on the target was rebuilt. Returns the status and the warnings found in the file, for example event ids which \
                       differ from the target.",
        annotations(
            title = "XCP load A2L description",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn xcp_load_a2l(&self, Parameters(args): Parameters<LoadA2lArgs>) -> Result<String, String> {
        self.session
            .load_a2l(LoadA2lRequest {
                a2l: args.a2l,
                elf: args.elf,
                fix: args.fix.unwrap_or(false),
                default_event: args.default_event,
                accept_epk_mismatch: args.accept_epk_mismatch.unwrap_or(false),
            })
            .await
    }

    #[tool(
        description = "Create an A2L file from the ELF file of an XCPlite application: the description of its measurement signals, calibration \
                       parameters, events and calibration segments, from the debug information and the XCPlite markers in the ELF file. \
                       Works without a target (offline). If a target is connected, its events and calibration segments are used as well. \
                       The file is only written, the description in use is not changed (use xcp_load_a2l for that). \
                       Returns the numbers of objects and the warnings of the generator, which say what could not be described and why.",
        annotations(
            title = "XCP create A2L file",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn xcp_create_a2l(&self, Parameters(args): Parameters<CreateA2lArgs>) -> Result<String, String> {
        self.session
            .create_a2l(CreateA2lRequest {
                elf: args.elf,
                a2l: args.a2l,
                template: args.template.unwrap_or(false),
                embed_aml: args.embed_aml.unwrap_or(false),
                var_filter: args.var_filter,
                unit_filter: args.unit_filter,
                skip_no_metadata: args.skip_no_metadata.unwrap_or(false),
                default_event: args.default_event,
                offline: args.offline.unwrap_or(false),
                accept_epk_mismatch: args.accept_epk_mismatch.unwrap_or(false),
                overwrite: args.overwrite.unwrap_or(false),
            })
            .await
    }

    #[tool(
        description = "Save the current values of all calibration parameters of the target into a file: the data of all calibration segments, \
                       read from the target. Use it to keep a parameter set before changes. An existing file is not overwritten.",
        annotations(
            title = "XCP save parameters",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn xcp_save_parameters(&self, Parameters(args): Parameters<SaveParametersArgs>) -> Result<String, String> {
        self.session.save_parameters(args.file).await
    }

    #[tool(
        description = "Load a parameter file (.hex) into the target: all calibration parameters of the segments in the file are overwritten with \
                       the values from the file. This changes the behavior of the running application on the target immediately. \
                       Call it only when the user asked for exactly this, and only with a file which was saved from the same software version: \
                       the file contains memory addresses, not parameter names, and is not checked against the software on the target.",
        annotations(
            title = "XCP load parameters",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn xcp_load_parameters(&self, Parameters(args): Parameters<LoadParametersArgs>) -> Result<String, String> {
        self.session.load_parameters(args.file).await
    }
}

#[tool_handler(
    router = self.tool_router,
    name = "xcpclient",
    instructions = "XCP measurement and calibration client for one target (ECU) running XCPlite. \
                    The connection to the target is kept between calls. Call xcp_status first: if a target is already connected, do not connect again. \
                    Never guess signal or parameter names, look them up with xcp_list_measurements and xcp_list_parameters (regular expressions). \
                    xcp_write and xcp_load_parameters change the behavior of a running machine. Call them only when the user asked for exactly this change, and report what was changed. \
                    For one current value use xcp_read, to observe signals for some seconds use xcp_sample, to record into a file use xcp_measure_start and xcp_measure_stop. \
                    Measurement data is written to files, tool results contain summaries only. \
                    When a tool reports that the connection is lost, tell the user, and reconnect with xcp_connect only if the user wants it."
)]
impl ServerHandler for XcpMcpServer {}

//------------------------------------------------------------------------
// Serve on stdin and stdout

/// Run the MCP server on stdin and the given output until the client closes stdin or the process is terminated
/// (Ctrl-C, SIGTERM), then disconnect from the target
/// output is the channel for the MCP messages, see protect_stdout
pub async fn run_stdio<W>(session: SessionHandle, read_only: bool, output: W) -> Result<(), Box<dyn Error>>
where
    W: tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let server = XcpMcpServer::new(session.clone(), read_only);
    info!("MCP server on stdio started{}", if read_only { ", read only" } else { "" });
    let service = server.serve((tokio::io::stdin(), output)).await?;
    tokio::select! {
        reason = service.waiting() => info!("MCP server stopped: {:?}", reason),
        _ = terminate_signal() => info!("MCP server terminated by signal"),
    }
    // Disconnect from the target
    session.shutdown().await;
    Ok(())
}

// Ctrl-C or, on Unix, SIGTERM
async fn terminate_signal() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = sigterm.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

//------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionDefaults;

    fn session() -> SessionHandle {
        SessionHandle::spawn(SessionDefaults {
            dest_addr: "127.0.0.1".to_string(),
            port: 5555,
            protocol: None,
            bind_addr: "0.0.0.0:0".parse().unwrap(),
            connect_mode: 0,
            a2l: String::new(),
            elf: String::new(),
            default_event: None,
            file_dir: std::env::temp_dir().join("xcpclient-test"),
        })
        .unwrap()
    }

    fn tool_names(server: &XcpMcpServer) -> Vec<String> {
        let mut names: Vec<String> = server.tool_router.list_all().iter().map(|tool| tool.name.to_string()).collect();
        names.sort();
        names
    }

    // The tools the model sees, without the writing tools in read only mode
    #[tokio::test]
    async fn test_tool_list() {
        let server = XcpMcpServer::new(session(), false);
        assert_eq!(
            tool_names(&server),
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
        let server = XcpMcpServer::new(session(), true);
        assert!(!tool_names(&server).contains(&"xcp_write".to_string()));
        assert!(!tool_names(&server).contains(&"xcp_load_parameters".to_string()));
        assert_eq!(tool_names(&server).len(), 12);
    }

    // Every tool has a description and documented arguments, this is all the model knows about it
    #[tokio::test]
    async fn test_tool_descriptions() {
        let server = XcpMcpServer::new(session(), false);
        for tool in server.tool_router.list_all() {
            assert!(tool.description.as_ref().is_some_and(|d| d.len() > 20), "{} has no description", tool.name);
            // A description is one text, a lost line continuation in the source leaves the indentation in it
            assert!(
                tool.description.as_ref().is_some_and(|d| !d.contains("  ")),
                "{} has a description with runs of spaces",
                tool.name
            );
            let schema = serde_json::to_value(&tool.input_schema).unwrap();
            if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
                for (name, property) in properties {
                    assert!(property.get("description").is_some(), "argument {} of {} has no description", name, tool.name);
                }
            }
        }
        // xcp_connect needs no argument, the others have required arguments where they make no sense without
        let connect = server.tool_router.get("xcp_connect").unwrap();
        let schema = serde_json::to_value(&connect.input_schema).unwrap();
        assert!(schema.get("required").is_none_or(|r| r.as_array().is_some_and(|r| r.is_empty())), "{}", schema);
        let write = server.tool_router.get("xcp_write").unwrap();
        let schema = serde_json::to_value(&write.input_schema).unwrap();
        assert_eq!(schema["required"], serde_json::json!(["name", "value"]));
    }
}
