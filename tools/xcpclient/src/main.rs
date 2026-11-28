//-----------------------------------------------------------------------------
// xcpclient - XCP test tool
//
// - Connect to XCP on Ethernet servers via TCP or UDP
// - Upload A2L or ELF files from XCP servers (GET_ID command)
// - Create complete A2L files from ELF debug information the XCP server event and memory segment information
// - Create A2L templates from the XCP server event and memory segment information
// - Read and write calibration variables (CAL)
// - Configure and acquire measurement data (DAQ)
// - List available variables and parameters with regex patterns
// - Execute test sequences
//
// xcpclient --help
//-----------------------------------------------------------------------------

use std::error::Error;
use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use figment::{
    Figment,
    providers::{Format, Toml},
};

use xcpclient::session::{DefaultEvent, DescriptionOptions, MeasurementOptions, ParameterValue, Session, parse_dest_addr};
use xcpclient::xcp_client::{A2lTypeEncoding, XcpTextDecoder};

mod xcp_test_executor;
use xcp_test_executor::test_executor;

//-----------------------------------------------------------------------------
// Command line arguments

use clap::Parser;

// Help text conventions (clap derive):
// - The first paragraph of a doc comment is the short help (-h), the whole doc comment is the long help (--help).
//   Paragraphs are separated by an empty `///` line, the lines of a paragraph are joined, clap wraps the text to the terminal width.
// - Keep the first paragraph to one short sentence, put details, requirements and formats into the following paragraphs.
// - Every option belongs to a help_heading group, except the general options.

#[derive(Parser, Debug)]
#[command(name = "xcpclient")]
#[command(about = concat!("XCP client v", env!("CARGO_PKG_VERSION"), " for testing XCP servers and managing A2L and HEX files"))]
#[command(version)]
#[command(max_term_width = 120)]
struct Args {
    // --config
    /// Load arguments from a TOML config file
    ///
    /// Command-line arguments take precedence.
    #[arg(long, default_value = "", hide_default_value = true, value_name = "FILE")]
    config: String,

    // --log-level
    /// Program flow log level (Off=0, Error=1, Warn=2, Info=3, Debug=4, Trace=5)
    #[arg(long, default_value_t = 3, value_name = "LEVEL")]
    log_level: u8,

    // --verbose
    /// ELF parsing log verbosity level
    #[arg(long, default_value_t = 0, value_name = "LEVEL")]
    verbose: usize,

    // --yes
    /// Automatically confirm EPK mismatch and other safety warnings
    ///
    /// Use in scripts to suppress interactive prompts.
    #[arg(long, short = 'y', default_value_t = false)]
    yes: bool,

    //-------------------------------------------------------------------------
    // Connection

    // --dest-addr
    /// XCP server address (IP address or IP:port)
    ///
    /// If the port is omitted, --port is used.
    #[arg(long, default_value = "127.0.0.1", value_name = "ADDR", help_heading = "Connection")]
    dest_addr: String,

    // --port
    /// XCP server port number, used when --dest-addr has no port
    #[arg(long, default_value_t = 5555, help_heading = "Connection")]
    port: u16,

    // --bind-addr
    /// Local bind address (IP address or IP:port)
    ///
    /// If the port is omitted, the system assigns an available port.
    #[arg(long, default_value = "0.0.0.0", value_name = "ADDR", help_heading = "Connection")]
    bind_addr: String,

    // --baud-rate
    /// Baud rate for XCP communication (only applicable for certain protocols)
    #[arg(long, default_value_t = 115200, help_heading = "Connection")]
    baud_rate: u32,

    // --tcp
    /// Use TCP for XCP communication
    #[arg(long, default_value_t = false, help_heading = "Connection")]
    tcp: bool,

    // --udp
    /// Use UDP for XCP communication
    #[arg(long, default_value_t = false, help_heading = "Connection")]
    udp: bool,

    // --sxi
    /// Use SxI for XCP communication
    #[arg(long, default_value_t = false, help_heading = "Connection")]
    sxi: bool,

    // --connect-mode
    /// XCP connect mode
    #[arg(long, default_value_t = 0, value_name = "MODE", help_heading = "Connection")]
    connect_mode: u8,

    // --offline
    /// Force offline mode, no network communication
    ///
    /// The communication parameters are used to create the A2L file.
    #[arg(long, default_value_t = false, help_heading = "Connection")]
    offline: bool,

    //-------------------------------------------------------------------------
    // A2L file

    // --a2l
    /// Name of the A2L file
    ///
    /// If not specified, the A2L file name is read from the XCP server.
    #[arg(long, default_value = "", hide_default_value = true, value_name = "FILE", help_heading = "A2L file")]
    a2l: String,

    // --upload-a2l
    /// Upload the A2L file from the XCP server
    ///
    /// Requires that the XCP server supports GET_ID A2L upload.
    #[arg(long, default_value_t = false, help_heading = "A2L file")]
    upload_a2l: bool,

    // --create-a2l
    /// Create an A2L file from XCP server information and, with --elf or --upload-elf, from ELF debug information
    ///
    /// Events and memory segments are read from the XCP server, all visible measurement and calibration variables are inserted from
    /// the ELF file if specified with --elf or --upload-elf.
    /// Requires that the XCP server supports the GET_EVENT_INFO and GET_SEGMENT_INFO commands.
    #[arg(long, default_value_t = false, help_heading = "A2L file")]
    create_a2l: bool,

    // --create-a2l-template
    /// Create a minimal A2L template from XCP server event and memory segment information only
    ///
    /// No variables or types are registered, the result is a skeleton A2L file.
    /// Requires that the XCP server supports the GET_EVENT_INFO and GET_SEGMENT_INFO commands.
    #[arg(long, default_value_t = false, help_heading = "A2L file")]
    create_a2l_template: bool,

    // --fix-a2l
    /// Correct event ids and calibration segment numbers of the given A2L file with XCP server information
    ///
    /// The A2L file is rewritten, the original is kept as <file>.a2l.bak.
    /// Only for A2L files created by xcpclient (--elf and --create-a2l), typically generated offline without section registration.
    /// Requires that the XCP server supports the GET_EVENT_INFO and GET_SEGMENT_INFO commands.
    #[arg(long, default_value_t = false, help_heading = "A2L file")]
    fix_a2l: bool,

    // --embed-aml
    /// Embed the XCP A2ML description into a created A2L file, instead of including XCP_104.aml
    ///
    /// Applies to A2L files created by xcpclient (--elf, --create-a2l, --create-a2l-template). The A2L file is self-contained,
    /// XCP_104.aml is not needed next to it. --fix-a2l keeps an embedded A2ML description.
    #[arg(long, default_value_t = false, help_heading = "A2L file")]
    embed_aml: bool,

    //-------------------------------------------------------------------------
    // ELF file

    // --upload-elf
    /// Upload the ELF file from the XCP server
    ///
    /// Requires that the XCP server supports the proprietary GET_ID ELF upload command.
    #[arg(long, default_value_t = false, help_heading = "ELF file")]
    upload_elf: bool,

    // --elf
    /// Create an A2L file from the debug information of this ELF file
    ///
    /// If connected to an XCP server, events and memory segments are read from the XCP server.
    /// ELF files with DWARF debug information only (Linux, QNX, embedded targets), macOS Mach-O executables are not supported.
    #[arg(long, default_value = "", hide_default_value = true, value_name = "FILE", help_heading = "ELF file")]
    elf: String,

    // --elf-unit-limit
    /// Parse only compilation units <= n
    #[arg(long, default_value_t = usize::MAX, value_name = "N", hide_default_value = true, help_heading = "ELF file")]
    elf_unit_limit: usize,

    // --elf-unit-limit-min
    /// Parse only compilation units >= n
    #[arg(long, default_value_t = 0, value_name = "N", help_heading = "ELF file")]
    elf_unit_limit_min: usize,

    // --elf-var-filter
    /// Register only ELF variables whose names match this regex
    ///
    /// If not specified (or empty), all variables are registered.
    #[arg(long, default_value = "", hide_default_value = true, value_name = "REGEX", help_heading = "ELF file")]
    elf_var_filter: String,

    // --elf-skip-no-metadata
    /// Skip ELF variables without metadata (XCP_UNIT, XCP_LIMITS, XCP_COMMENT)
    ///
    /// Only variables that have at least one metadata annotation are included in the A2L output.
    #[arg(long, default_value_t = false, help_heading = "ELF file")]
    elf_skip_no_metadata: bool,

    // --elf-unit-filter
    /// Register only ELF variables from compilation units (source files) whose names match this regex
    ///
    /// If not specified (or empty), variables from all compilation units are registered.
    #[arg(long, default_value = "", hide_default_value = true, value_name = "REGEX", help_heading = "ELF file")]
    elf_unit_filter: String,

    //-------------------------------------------------------------------------
    // Calibration

    // --bin
    /// Pathname of the parameter file (.bin or Intel-HEX .hex) for --upload-bin and --download-bin
    #[arg(long, default_value = "", hide_default_value = true, value_name = "FILE", help_heading = "Calibration")]
    bin: String,

    // --upload-bin
    /// Upload the working page data of all calibration segments into a parameter file
    ///
    /// Requires that the XCP server supports GET_ID A2L upload.
    /// A .bin file is an XCPlite persistence file with the event list and the context list (conv.contexts in the A2L file),
    /// a .hex file contains the segment data only.
    #[arg(long, default_value_t = false, help_heading = "Calibration")]
    upload_bin: bool,

    // --download-bin
    /// Download the working page data of all calibration segments from a parameter file to the target
    #[arg(long, default_value_t = false, help_heading = "Calibration")]
    download_bin: bool,

    //-------------------------------------------------------------------------
    // Measurement

    // --list-mea
    /// List the measurement variables in the A2L file matching this regex
    #[arg(long, default_value = "", hide_default_value = true, value_name = "REGEX", help_heading = "Measurement")]
    list_mea: String,

    // --mea
    /// Measure these variables with DAQ, names separated by space or regular expressions (e.g. ".*")
    #[arg(long, value_delimiter = ' ', num_args = 1.., value_name = "NAME", help_heading = "Measurement")]
    mea: Vec<String>,

    // --default-event
    /// Event for variables without a fixed event, given by event id or event name
    ///
    /// Applies to global variables and static variables in functions without an event trigger.
    /// Used for their DAQ measurement and assigned to them as default event when an A2L file is created from an ELF file.
    /// An event name is looked up in the event list (from the XCP server, the ELF file or the A2L file), xcpclient aborts when it is not found.
    /// If not specified, such variables get no event and can not be measured with xcpclient.
    #[arg(long, value_name = "ID|NAME", help_heading = "Measurement")]
    default_event: Option<DefaultEvent>,

    // --time
    /// Measurement duration in s, 0 means infinite
    #[arg(long, default_value_t = 0, value_name = "S", help_heading = "Measurement")]
    time: u64,

    //-------------------------------------------------------------------------
    // Measurement output files

    // --csv
    /// Save measurement data to a CSV file instead of printing it to the console
    ///
    /// CSV format: time_ns,daq,name,value,unit (one row per measurement sample).
    /// Values are physical values (factor and offset applied), the value of an enum is its label.
    #[arg(long, default_value = "", hide_default_value = true, value_name = "FILE", help_heading = "Measurement output files")]
    csv: String,

    // --mdf
    /// Save measurement data to an ASAM MDF 4.1 file
    ///
    /// One channel group per DAQ list, the payload of the DAQ packets is stored as received.
    /// Values are raw values, the MDF file contains the conversions (factor and offset, enum labels) and units.
    /// May be combined with --csv, --mcap and --perfetto.
    #[arg(long, default_value = "", hide_default_value = true, value_name = "FILE", help_heading = "Measurement output files")]
    mdf: String,

    // --mcap
    /// Save measurement data to an MCAP file (https://mcap.dev)
    ///
    /// One channel per DAQ list, the topic is the event name, the payload of the DAQ packets is stored as received, as ros1 messages.
    /// Values are raw values, MCAP has no conversions or units, they are in the comments of the message definitions and in the channel metadata.
    /// May be combined with --csv, --mdf and --perfetto.
    #[arg(long, default_value = "", hide_default_value = true, value_name = "FILE", help_heading = "Measurement output files")]
    mcap: String,

    // --perfetto
    /// Save measurement data to a trace file for Perfetto (https://ui.perfetto.dev)
    ///
    /// Chrome Trace Event JSON format. Spans (DaqSpan) are slices in a track per context and thread, numeric signals are counter tracks,
    /// enum signals are state tracks, span links (xcp::LinkSpan) are flows (arrows) from the producer span to the consumer span.
    /// The span and link signals of the contexts needed for the trace are added to the measurement, see --trace-auto.
    /// May be combined with --csv, the added signals are not written to the CSV file.
    #[arg(long, default_value = "", hide_default_value = true, value_name = "FILE", help_heading = "Measurement output files")]
    perfetto: String,

    // --trace-auto
    /// Trace signals added to the measurement with --perfetto
    ///
    /// off: none, only hints which signals are missing.
    ///
    /// context: the spans of the contexts of the measured signals.
    ///
    /// links: also the span links of these contexts and, if there are any, the spans of all contexts as possible producers.
    ///
    /// all: the spans and span links of all contexts.
    ///
    /// The context of a signal is the prefix of its name or of the name of its event (<context>.<name>).
    #[arg(long, default_value = "links", value_parser = ["off", "context", "links", "all"], value_name = "LEVEL", help_heading = "Measurement output files")]
    trace_auto: String,

    //-------------------------------------------------------------------------
    // Calibration (continued, clap gathers the options of a heading in declaration order)

    // --list-cal
    /// List the calibration variables in the A2L file matching this regex
    #[arg(long, default_value = "", hide_default_value = true, value_name = "REGEX", help_heading = "Calibration")]
    list_cal: String,

    // --cal
    /// Set a calibration variable to a value
    #[arg(long, value_names = ["NAME", "VALUE"], num_args = 2, help_heading = "Calibration")]
    cal: Vec<String>,

    //-------------------------------------------------------------------------
    // Test

    // --test
    /// Execute a test sequence on the XCP server
    ///
    /// If there is a calibration segment named params (see daq_test, c_demo), a calibration consistency test is performed.
    #[arg(long, default_value_t = false, help_heading = "Test")]
    test: bool,

    //-------------------------------------------------------------------------
    // MCP server

    // --mcp
    /// Run as MCP server on stdin/stdout, offering the xcpclient functions as tools to an AI assistant
    ///
    /// The MCP host starts xcpclient with this option, xcpclient keeps the connection to the target between the tool calls, see docs/MCP.md.
    /// --dest-addr, --port, --udp/--tcp, --bind-addr, --a2l, --elf and --default-event are the defaults of the connect tool.
    /// Log output goes to stderr.
    #[cfg(feature = "mcp")]
    #[arg(long, default_value_t = false, help_heading = "MCP server")]
    mcp: bool,

    // --mcp-read-only
    /// Do not offer the tools which change the target (writing calibration parameters)
    #[cfg(feature = "mcp")]
    #[arg(long, default_value_t = false, requires = "mcp", help_heading = "MCP server")]
    mcp_read_only: bool,

    // --mcp-dir
    /// Directory for the files the MCP server writes on its own
    ///
    /// These are the A2L file uploaded from the target, XCP_104.aml and an A2L file created from an ELF file without a given name.
    /// Default is the cache directory of the user for xcpclient, never the working directory, where an XCPlite application started
    /// in the same directory writes its A2L file.
    #[cfg(feature = "mcp")]
    #[arg(long, default_value = "", hide_default_value = true, value_name = "DIR", requires = "mcp", help_heading = "MCP server")]
    mcp_dir: String,
}

//----------------------------------------------------------------------------------------------
// Logging

#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};

trait ToLogLevelFilter {
    fn to_log_level_filter(self) -> log::LevelFilter;
}

impl ToLogLevelFilter for u8 {
    fn to_log_level_filter(self) -> log::LevelFilter {
        match self {
            0 => log::LevelFilter::Off,
            1 => log::LevelFilter::Error,
            2 => log::LevelFilter::Warn,
            3 => log::LevelFilter::Info,
            4 => log::LevelFilter::Debug,
            5 => log::LevelFilter::Trace,
            _ => log::LevelFilter::Warn,
        }
    }
}

//------------------------------------------------------------------------
// Handle incoming SERV_TEXT data
// Prints the text to the console

#[derive(Debug, Clone, Copy)]
struct ServTextDecoder;

impl ServTextDecoder {
    pub fn new() -> ServTextDecoder {
        ServTextDecoder {}
    }
}

impl XcpTextDecoder for ServTextDecoder {
    // Handle incomming text data from XCP server
    fn decode(&self, data: &[u8]) {
        print!("[SERV_TEXT] ");
        let mut j = 0;
        while j < data.len() {
            print!("{}", data[j] as char);
            j += 1;
        }
    }
}

//------------------------------------------------------------------------
//  XCP client
// Runs the steps selected by the command line arguments with a session, see module session

async fn xcp_client(args: Args, protocol: &'static str, dest_addr: std::net::SocketAddr, local_addr: std::net::SocketAddr, baud_rate: u32) -> Result<(), Box<dyn Error>> {
    // Create the session
    let mut session = Session::new(protocol, dest_addr, local_addr, baud_rate);
    if let Some(event) = &args.default_event {
        info!("Default event {} for variables without a fixed event", event);
    }

    // Execute main logic and capture the result
    let result = xcp_client_steps(&mut session, args, protocol).await;

    // Always disconnect, regardless of whether the logic succeeded or failed
    session.disconnect().await?;

    // Return the result from the main logic (preserving any errors that occurred)
    result
}

async fn xcp_client_steps(session: &mut Session, args: Args, protocol: &'static str) -> Result<(), Box<dyn Error>> {
    // Destructure args
    let Args {
        verbose,
        connect_mode,
        offline,
        a2l,
        upload_a2l,
        create_a2l,
        create_a2l_template,
        fix_a2l,
        embed_aml,
        elf,
        upload_elf,
        elf_unit_limit_min,
        elf_unit_limit,
        elf_var_filter,
        elf_skip_no_metadata,
        elf_unit_filter,
        bin: bin_filename,
        upload_bin,
        download_bin,
        list_cal,
        list_mea,
        mea: measurement_list,
        default_event,
        time,
        cal: cal_args,
        csv,
        mdf,
        mcap,
        perfetto,
        trace_auto,
        yes,
        ..
    } = args;
    let measurement_duration_ms = time * 1000;

    //----------------------------------------------------------------
    // Connect the XCP on ETH server if required
    let go_online = !offline && (protocol == "TCP" || protocol == "UDP" || upload_a2l || upload_elf || !measurement_list.is_empty() || !cal_args.is_empty());
    if go_online {
        session.connect(connect_mode, ServTextDecoder::new()).await?;
    }

    //----------------------------------------------------------------
    // Upload, create or load the A2L file
    let description_options = DescriptionOptions {
        a2l,
        a2l_dir: String::new(), // the current directory
        upload_a2l,
        create_a2l,
        create_a2l_template,
        fix_a2l,
        embed_aml,
        elf,
        upload_elf,
        elf_unit_limit_min,
        elf_unit_limit,
        elf_var_filter,
        elf_unit_filter,
        elf_skip_no_metadata,
        default_event,
        verbose,
    };
    // An EPK mismatch between the A2L file and the target is confirmed with --yes or interactively
    let confirm_epk_mismatch = |_a2l_epk: &str, _ecu_epk: &str| -> Result<(), Box<dyn Error>> {
        if !yes {
            if std::io::stdin().is_terminal() {
                eprint!("Continue anyway? [y/N] ");
                let _ = std::io::stderr().flush();
                let mut input = String::new();
                let _ = std::io::stdin().read_line(&mut input);
                if !input.trim().eq_ignore_ascii_case("y") {
                    return Err("Aborted due to EPK mismatch".into());
                }
            } else {
                return Err("EPK mismatch: aborting. Use --yes to proceed anyway.".into());
            }
        }
        Ok(())
    };
    session.load_description(&description_options, confirm_epk_mismatch).await?;

    // Print all known calibration objects and get their current value
    if !list_cal.is_empty() {
        println!();
        let cal_objects = session.find_parameters(list_cal.as_str());
        info!("===============================================================");
        println!("Calibration variables:");
        if !cal_objects.is_empty() {
            for name in &cal_objects {
                let p = session.read_parameter(name).await?;
                match p.encoding {
                    A2lTypeEncoding::Signed => {
                        print!(" {} {}:{:08X}", p.name, p.addr.ext, p.addr.addr);
                        if let Some(ParameterValue::Signed(v)) = p.value {
                            print!(" ={}", v);
                        }
                    }
                    A2lTypeEncoding::Unsigned => {
                        print!(" {} {}:{:08X} ", p.name, p.addr.ext, p.addr.addr);
                        if let Some(ParameterValue::Unsigned(v)) = p.value {
                            print!(" = {}", v);
                        }
                    }
                    A2lTypeEncoding::Float => {
                        print!(" {} {}:{:08X}", p.name, p.addr.ext, p.addr.addr);
                        if let Some(ParameterValue::Float(v)) = p.value {
                            print!(" = {}", v);
                        }
                    }
                    A2lTypeEncoding::Blob => {
                        print!(" {} = [...]", name);
                    }
                }
                println!();
            }
            println!();
        } else {
            println!(" None");
        }
    }

    // Upload or Download calibration segment data
    // (Note: In XCP, Upload means from target to file, Download means from file to target)
    if upload_bin {
        session.upload_parameters(&bin_filename).await?;
    } else if download_bin {
        session.download_parameters(&bin_filename).await?;
    }

    // Set calibration variable
    if session.is_connected() && !cal_args.is_empty() {
        if cal_args.len() != 2 {
            return Err("Calibration command requires exactly 2 arguments: variable name and value".into());
        }

        // Parse the value as a double
        let var_name = &cal_args[0];
        let value_str = &cal_args[1];
        let value: f64 = value_str.parse().map_err(|_| format!("Failed to parse '{}' as a double value", value_str))?;
        session.write_parameter(var_name, value).await?;
        println!("Ok");
    }

    // Print all known measurement objects
    if !list_mea.is_empty() {
        println!();
        let mea_objects = session.find_measurements(&list_mea);
        info!("===============================================================");
        println!("Measurement variables:");
        if !mea_objects.is_empty() {
            for name in &mea_objects {
                if let Some(description) = session.describe_measurement(name) {
                    println!(" {}", description);
                }
            }
            println!();
        } else {
            println!(" None");
        }
    }

    // Measurement
    if session.is_connected() && !measurement_list.is_empty() {
        // Measurement data output to the given files, to the console if there is none
        let measurement_options = MeasurementOptions {
            csv,
            mdf,
            mcap,
            perfetto,
            trace_auto,
            console: true,
        };
        if session.start_measurement(measurement_list, &measurement_options).await? {
            // Periodic statistics reporter — prints event and data rate once per second.
            // Reported at log level Info, independent of --verbose (which only controls per-sample content dumps).
            let stats_decoder = session.daq_decoder();
            let deadline = if measurement_duration_ms > 0 {
                Some(tokio::time::Instant::now() + std::time::Duration::from_millis(measurement_duration_ms))
            } else {
                println!("Press Ctrl-C to stop measurement...");
                None
            };
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
            ticker.tick().await; // consume the immediate first tick, report rates from t+1s
            let mut last_report = tokio::time::Instant::now();
            let mut last_events = 0usize;
            let mut last_bytes = 0usize;
            let mut last_lost = 0usize;
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        if let Some(ref d) = stats_decoder {
                            let (events, bytes, lost) = {
                                let g = d.lock();
                                (g.get_event_count(), g.get_byte_count(), g.get_lost_count())
                            };
                            let now = tokio::time::Instant::now();
                            let dt = now.duration_since(last_report).as_secs_f64();
                            if dt > 0.0 {
                                if lost > last_lost {
                                    warn!(
                                        "Measurement: {:.0} event/s, {:.3} Mbyte/s, {} packets lost",
                                        (events - last_events) as f64 / dt,
                                        (bytes - last_bytes) as f64 / dt / 1_000_000.0,
                                        lost - last_lost
                                    );
                                } else {
                                    info!(
                                        "Measurement: {:.0} event/s, {:.3} Mbyte/s",
                                        (events - last_events) as f64 / dt,
                                        (bytes - last_bytes) as f64 / dt / 1_000_000.0
                                    );
                                }
                            }
                            last_report = now;
                            last_events = events;
                            last_bytes = bytes;
                            last_lost = lost;
                        }
                    }
                    _ = async {
                        match deadline {
                            Some(dl) => tokio::time::sleep_until(dl).await,
                            None => std::future::pending::<()>().await,
                        }
                    } => {
                        break;
                    }
                    _ = tokio::signal::ctrl_c(), if deadline.is_none() => {
                        println!("\nStopping measurement...");
                        break;
                    }
                }
            }

            // Stop the measurement and print the statistics
            if let Some(summary) = session.stop_measurement().await? {
                summary.log();
            }
        }
    }

    Ok(())
}

//------------------------------------------------------------------------
// Config file support

/// Mirrors all Args fields as Option<T>.
/// Fields present in the TOML file are Some(_); absent fields are None and left unchanged.
#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct ConfigFile {
    log_level: Option<u8>,
    verbose: Option<usize>,
    dest_addr: Option<String>,
    port: Option<u16>,
    bind_addr: Option<String>,
    baud_rate: Option<u32>,
    tcp: Option<bool>,
    udp: Option<bool>,
    sxi: Option<bool>,
    connect_mode: Option<u8>,
    offline: Option<bool>,
    a2l: Option<String>,
    upload_a2l: Option<bool>,
    create_a2l: Option<bool>,
    create_a2l_template: Option<bool>,
    fix_a2l: Option<bool>,
    embed_aml: Option<bool>,
    upload_elf: Option<bool>,
    elf: Option<String>,
    elf_unit_limit: Option<usize>,
    elf_unit_limit_min: Option<usize>,
    elf_var_filter: Option<String>,
    elf_unit_filter: Option<String>,
    bin: Option<String>,
    upload_bin: Option<bool>,
    download_bin: Option<bool>,
    list_mea: Option<String>,
    mea: Option<Vec<String>>,
    default_event: Option<DefaultEvent>,
    time: Option<u64>,
    csv: Option<String>,
    mdf: Option<String>,
    mcap: Option<String>,
    perfetto: Option<String>,
    trace_auto: Option<String>,
    list_cal: Option<String>,
    cal: Option<Vec<String>>,
    elf_skip_no_metadata: Option<bool>,
    test: Option<bool>,
    yes: Option<bool>,
}

/// Apply config file values to args.
/// Only fields that are present in the config file (Some) and were NOT explicitly
/// set on the command line are overwritten.
fn merge_config(matches: &clap::ArgMatches, config: ConfigFile, args: &mut Args) {
    use clap::parser::ValueSource;
    macro_rules! apply {
        ($field:ident) => {
            if let Some(v) = config.$field {
                if matches.value_source(stringify!($field)) != Some(ValueSource::CommandLine) {
                    args.$field = v;
                }
            }
        };
    }
    apply!(log_level);
    apply!(verbose);
    apply!(dest_addr);
    apply!(port);
    apply!(bind_addr);
    apply!(baud_rate);
    apply!(tcp);
    apply!(udp);
    apply!(sxi);
    apply!(connect_mode);
    apply!(offline);
    apply!(a2l);
    apply!(upload_a2l);
    apply!(create_a2l);
    apply!(create_a2l_template);
    apply!(fix_a2l);
    apply!(embed_aml);
    apply!(upload_elf);
    apply!(elf);
    apply!(elf_unit_limit);
    apply!(elf_unit_limit_min);
    apply!(elf_var_filter);
    apply!(elf_unit_filter);
    apply!(bin);
    apply!(upload_bin);
    apply!(download_bin);
    apply!(list_mea);
    apply!(mea);
    // default_event is an Option<DefaultEvent> argument, a value from the config file wraps into Some
    if let Some(v) = config.default_event
        && matches.value_source("default_event") != Some(ValueSource::CommandLine)
    {
        args.default_event = Some(v);
    }
    apply!(time);
    apply!(csv);
    apply!(mdf);
    apply!(mcap);
    apply!(perfetto);
    apply!(trace_auto);
    apply!(list_cal);
    apply!(cal);
    apply!(elf_skip_no_metadata);
    apply!(test);
    apply!(yes);
}

//------------------------------------------------------------------------
// Main function

// Exit status is 1 on any error, so scripts can detect failures
#[tokio::main]
async fn main() -> ExitCode {
    use clap::{CommandFactory, FromArgMatches};

    // Parse command line arguments
    let mut matches = Args::command().get_matches();
    let mut args = Args::from_arg_matches(&mut matches).unwrap();

    // Load and merge config file if --config was specified
    // Logging is not initialized yet, the log level may come from the config file
    if !args.config.is_empty() {
        match Figment::new().merge(Toml::file(&args.config)).extract::<ConfigFile>() {
            Ok(config) => merge_config(&matches, config, &mut args),
            Err(e) => {
                eprintln!("Error: Invalid config file '{}': {}", args.config, e);
                return ExitCode::FAILURE;
            }
        }
    }

    // MCP server mode: stdout is the channel for the MCP messages
    // It is redirected to stderr for everything else, before anything is printed, and the log output goes to stderr
    #[cfg(feature = "mcp")]
    let mcp_output = if args.mcp {
        match xcpclient::mcp::protect_stdout() {
            Ok(output) => Some(output),
            Err(e) => {
                eprintln!("Error: Could not redirect stdout for the MCP server mode: {}", e);
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    #[cfg(feature = "mcp")]
    let log_target = if args.mcp { env_logger::Target::Stderr } else { env_logger::Target::Stdout };
    #[cfg(not(feature = "mcp"))]
    let log_target = env_logger::Target::Stdout;

    // Initialize logging
    let log_level = args.log_level.to_log_level_filter();
    let mut logger = env_logger::Builder::new();
    logger
        .target(log_target)
        .filter_level(log_level)
        .format_timestamp(None)
        .format_module_path(false)
        .format_target(false);
    // In MCP server mode the warnings logged while a tool runs are part of its result, independent of the log level
    #[cfg(feature = "mcp")]
    if args.mcp {
        let _ = xcpclient::log_capture::CaptureLogger::install(logger.build(), log_level);
    } else {
        logger.init();
    }
    #[cfg(not(feature = "mcp"))]
    logger.init();

    info!(
        "xcp_client - version {}.{}.{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH")
    );

    debug!("{:#?}", args);

    // Run the MCP server
    #[cfg(feature = "mcp")]
    if let Some(output) = mcp_output {
        let result = run_mcp(args, output).await;
        if let Err(e) = &result {
            error!("{}", e);
        }
        log::logger().flush();
        // Exit here: the runtime would wait for the blocking read of stdin, which can not be cancelled
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }

    // Run the XCP client or the test executor
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{}", e);
            ExitCode::FAILURE
        }
    }
}

// Run the MCP server on stdin and the given output until the MCP host closes stdin
#[cfg(feature = "mcp")]
async fn run_mcp(args: Args, output: xcpclient::mcp::McpOutput) -> Result<(), Box<dyn Error>> {
    use xcpclient::session::{SessionDefaults, SessionHandle, TransportProtocol, default_file_dir};

    // The options which select an action of the command line are not available, the MCP host selects the actions with tool calls
    let actions = [
        ("--mea", !args.mea.is_empty()),
        ("--time", args.time != 0),
        ("--csv", !args.csv.is_empty()),
        ("--mdf", !args.mdf.is_empty()),
        ("--mcap", !args.mcap.is_empty()),
        ("--perfetto", !args.perfetto.is_empty()),
        ("--cal", !args.cal.is_empty()),
        ("--list-mea", !args.list_mea.is_empty()),
        ("--list-cal", !args.list_cal.is_empty()),
        ("--upload-a2l", args.upload_a2l),
        ("--create-a2l", args.create_a2l),
        ("--create-a2l-template", args.create_a2l_template),
        ("--fix-a2l", args.fix_a2l),
        ("--upload-elf", args.upload_elf),
        ("--bin", !args.bin.is_empty()),
        ("--upload-bin", args.upload_bin),
        ("--download-bin", args.download_bin),
        ("--test", args.test),
        ("--offline", args.offline),
        ("--sxi", args.sxi),
    ];
    if let Some((option, _)) = actions.iter().find(|(_, used)| *used) {
        return Err(format!("{} can not be used with --mcp", option).into());
    }

    // Directory for the files the server writes on its own, an absolute path
    let file_dir = if args.mcp_dir.is_empty() {
        default_file_dir()
    } else {
        std::path::absolute(&args.mcp_dir).map_err(|e| format!("--mcp-dir '{}': {}", args.mcp_dir, e))?
    };
    info!("Files of the MCP server are written to {}", file_dir.display());

    // The target options are the defaults of the connect tool, UDP if no protocol is given
    let defaults = SessionDefaults {
        dest_addr: args.dest_addr,
        port: args.port,
        protocol: if args.tcp {
            Some(TransportProtocol::Tcp)
        } else if args.udp {
            Some(TransportProtocol::Udp)
        } else {
            None
        },
        bind_addr: parse_dest_addr(&args.bind_addr, 0)?,
        connect_mode: args.connect_mode,
        a2l: args.a2l,
        elf: args.elf,
        default_event: args.default_event,
        file_dir,
    };
    let session = SessionHandle::spawn(defaults)?;
    xcpclient::mcp::run_stdio(session, args.mcp_read_only, output).await
}

// Select protocol and addresses, run the XCP client or the test executor
async fn run(args: Args) -> Result<(), Box<dyn Error>> {
    // Protocol, IP addresses, port, baudrate
    let dest_addr: std::net::SocketAddr = parse_dest_addr(&args.dest_addr, args.port)?;
    let local_addr: std::net::SocketAddr = parse_dest_addr(&args.bind_addr, 0)?;
    let mut baud_rate = args.baud_rate;
    let protocol = if args.tcp {
        "TCP"
    } else if args.udp {
        "UDP"
    } else if args.sxi {
        if baud_rate == 0 {
            baud_rate = 115200;
        }
        "SxI"
    } else {
        warn!("No protocol specified, defaulting to TCP");
        "TCP"
    };

    // Run the test executor if --test is specified
    if args.test {
        const TEST_CAL: xcp_test_executor::TestModeCal = xcp_test_executor::TestModeCal::Cal; // Execute calibration tests: Cal or None
        const TEST_DAQ: xcp_test_executor::TestModeDaq = xcp_test_executor::TestModeDaq::DaqCmd; // Execute measurement tests: DaqCmd, Daq or None
        if !test_executor(protocol, dest_addr, local_addr, TEST_CAL, TEST_DAQ).await {
            return Err("XCP test failed".into());
        }
    }
    // Run the XCP client
    else {
        xcp_client(args, protocol, dest_addr, local_addr, baud_rate)
            .await
            .map_err(|e| format!("XCP client error: {}", e))?;
    }

    Ok(())
}
