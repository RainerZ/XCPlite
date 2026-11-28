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

use xcpclient::session::{DefaultEvent, DescriptionOptions, MeasurementOptions, ParameterValue, Session};
use xcpclient::xcp_client::{A2lTypeEncoding, XcpTextDecoder};

mod xcp_test_executor;
use xcp_test_executor::test_executor;

//-----------------------------------------------------------------------------
// Command line arguments

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "xcpclient")]
#[command(about = concat!("XCP client v", env!("CARGO_PKG_VERSION"), " for testing XCP servers and managing A2L and HEX files"))]
#[command(long_about = concat!("XCP client v", env!("CARGO_PKG_VERSION"), " for testing XCP servers and managing A2L and HEX files"))]
#[command(version)]
struct Args {
    // --config
    /// Load arguments from a TOML config file. Command-line arguments take precedence.
    #[arg(long, default_value = "")]
    config: String,

    // -l --log-level
    /// Program flow log level (Off=0, Error=1, Warn=2, Info=3, Debug=4, Trace=5)
    #[arg(long, default_value_t = 3)]
    log_level: u8,

    // -v --verbose
    /// ELF parsing log verbosity level
    #[arg(long, default_value_t = 0)]
    verbose: usize,

    // -d --dest_addr
    /// XCP server address (IP address or IP:port). If port is omitted, uses --port parameter.
    #[arg(long, default_value = "127.0.0.1")]
    dest_addr: String,

    // --port
    /// XCP server port number (used when --dest-addr doesn't include port).
    #[arg(long, default_value_t = 5555)]
    port: u16,

    // --bind-addr
    /// Bind address (IP address or IP:port). If port is omitted, system assigns an available port.
    #[arg(long, default_value = "0.0.0.0")]
    bind_addr: String,

    // --baud-rate
    /// Baud rate for XCP communication (only applicable for certain protocols).
    #[arg(long, default_value_t = 115200)]
    baud_rate: u32,

    // --tcp
    /// Use TCP for XCP communication..
    #[arg(long, default_value_t = false)]
    tcp: bool,
    // --udp
    /// Use UDP for XCP communication
    #[arg(long, default_value_t = false)]
    udp: bool,
    // --sxi
    /// Use SxI for XCP communication
    #[arg(long, default_value_t = false)]
    sxi: bool,

    // --connect-mode
    /// XCP connect mode
    #[arg(long, default_value_t = 0)]
    connect_mode: u8,

    // --offline
    /// Force offline mode (no network communication), communication parameters are used to create A2L file.
    #[arg(long, default_value_t = false)]
    offline: bool,

    // --a2l
    /// Specify and overide the name of the A2L file name.
    /// If not specified, The A2L file name is read from the XCP server.
    #[arg(long, default_value = "")]
    a2l: String,

    // --upload-a2l
    /// Upload A2L file from XCP server.
    /// Requires that the XCP server supports GET_ID A2L upload.
    #[arg(long, default_value_t = false)]
    upload_a2l: bool,

    // --create-a2l
    /// Build an A2L file template from XCP server information about events and memory segments.
    /// Requires that the XCP server supports the GET_EVENT_INFO and GET_SEGMENT_INFO commands.
    /// Insert all visible measurement and calibration variables from ELF file if specified with --elf or --upload-elf.
    #[arg(long, default_value_t = false)]
    create_a2l: bool,

    // --create-a2l-template
    /// Build a minimal A2L template from XCP server event and memory segment information only.
    /// No variables or types are registered; the result is a skeleton A2L file.
    /// Requires that the XCP server supports the GET_EVENT_INFO and GET_SEGMENT_INFO commands.
    #[arg(long, default_value_t = false)]
    create_a2l_template: bool,

    // --fix-a2l
    /// Correct the event ids and calibration segment numbers of the given A2L file with XCP server information and rewrite it, the original is kept as <file>.a2l.bak.
    /// Only for A2L files created by xcpclient (--elf and --create-a2l), typically generated offline without section registration.
    /// Requires that the XCP server supports the GET_EVENT_INFO and GET_SEGMENT_INFO commands.
    #[arg(long, default_value_t = false)]
    fix_a2l: bool,

    // --embed-aml
    /// Embed the XCP A2ML description into an A2L file created by xcpclient (--elf, --create-a2l, --create-a2l-template), instead of including XCP_104.aml.
    /// The A2L file is self-contained, XCP_104.aml is not needed next to it. --fix-a2l keeps an embedded A2ML description.
    #[arg(long, default_value_t = false)]
    embed_aml: bool,

    // --upload-elf
    /// Upload ELF file from XCP server.
    /// Requires that the XCP server supports proprietary GET_ID ELF upload command.
    #[arg(long, default_value_t = false)]
    upload_elf: bool,

    // ---elf
    /// Specify the name of an ELF file, create an A2L file from ELF debug information.
    /// If connected to a XCP server, events and memory segments will be extracted from the XCP server.
    /// ELF files with DWARF debug information only (Linux, QNX, embedded targets), macOS Mach-O executables are not supported.
    #[arg(long, default_value = "")]
    elf: String,

    // --elf-unit-limit
    /// Parse only compilations units <= n.
    #[arg(long, default_value_t = usize::MAX)]
    elf_unit_limit: usize,

    // --elf-unit-limit-min
    /// Parse only compilations units >= n.
    #[arg(long, default_value_t = 0)]
    elf_unit_limit_min: usize,

    // --elf-var-filter
    /// Regex pattern to filter variable names when registering from an ELF file.
    /// Only variables whose names match the pattern are included in the A2L output.
    /// If not specified (or empty), all variables are registered.
    #[arg(long, default_value = "")]
    elf_var_filter: String,

    // --elf-skip-no-metadata
    /// Skip variables without any metadata (XCP_UNIT / XCP_LIMITS / XCP_COMMENT) when creating an A2L file from an ELF file.
    /// Only variables that have at least one metadata annotation are included in the A2L output.
    #[arg(long, default_value_t = false)]
    elf_skip_no_metadata: bool,

    // --elf-unit-filter
    /// Regex pattern to filter variables by their compilation unit (source file) name.
    /// Only variables defined in compilation units whose name matches are included in the A2L output.
    /// If not specified (or empty), variables from all compilation units are registered.
    #[arg(long, default_value = "")]
    elf_unit_filter: String,

    // --bin
    /// Specify the pathname of a binary file (Intel-HEX) for calibration parameter segment data
    #[arg(long, default_value = "")]
    bin: String,

    // --upload-bin
    /// Upload all calibration segments working page data from target and store into a binary file.
    /// Requires that the XCP server supports GET_ID A2L upload.
    /// A .bin file is an XCPlite persistence file with the event list and the context list (conv.contexts in the A2L file), a .hex file contains the segment data only.
    #[arg(long, default_value_t = false)]
    upload_bin: bool,

    // --download-bin
    /// Download all calibration segments working page data in a binary file to the target.
    #[arg(long, default_value_t = false)]
    download_bin: bool,

    // --list-mea
    /// Lists all specified measurement variables (regex) found in the A2L file.
    #[arg(long, default_value = "")]
    list_mea: String,

    // --mea
    /// Specify variable names for DAQ measurement (list), may be list of names separated by space or single regular expressions (e.g. ".*").
    #[arg(long, value_delimiter = ' ', num_args = 1..)]
    mea: Vec<String>,

    // --default-event
    /// Event for variables without a fixed event (global variables and static variables in functions without an event trigger), given by event id or event name.
    /// Used for their DAQ measurement and assigned to them as default event when an A2L file is created from an ELF file.
    /// An event name is looked up in the event list (from the XCP server, the ELF file or the A2L file), xcpclient aborts when it is not found.
    /// If not specified, such variables get no event and can not be measured with xcpclient.
    #[arg(long, value_name = "ID|NAME")]
    default_event: Option<DefaultEvent>,

    // --time
    /// Time limit measurement duration to n s. 0 means infinite.
    #[arg(long, default_value_t = 0)]
    time: u64,

    // --csv
    /// Save measurement data to a CSV file. If not specified, data is printed to the console.
    /// CSV format: time_ns,daq,name,value,unit  (one row per measurement sample).
    /// Values are physical values (factor and offset applied), the value of an enum is its label.
    #[arg(long, default_value = "")]
    csv: String,

    // --mdf
    /// Save measurement data to an ASAM MDF 4.1 file. One channel group per DAQ list, the payload of the DAQ packets is stored as received.
    /// Values are raw values, the MDF file contains the conversions (factor and offset, enum labels) and units.
    /// May be combined with --csv and --perfetto.
    #[arg(long, default_value = "", value_name = "FILE")]
    mdf: String,

    // --mcap
    /// Save measurement data to an MCAP file (https://mcap.dev). One channel per DAQ list, the topic is the event name, the payload of the
    /// DAQ packets is stored as received, as ros1 messages. Values are raw values, MCAP has no conversions or units, they are in the
    /// comments of the message definitions and in the channel metadata. May be combined with --csv, --mdf and --perfetto.
    #[arg(long, default_value = "", value_name = "FILE")]
    mcap: String,

    // --perfetto
    /// Save measurement data to a trace file for Perfetto (Chrome Trace Event JSON format, open it with https://ui.perfetto.dev).
    /// Spans (DaqSpan) are slices in a track per context and thread, numeric signals are counter tracks, enum signals are state tracks,
    /// span links (xcp::LinkSpan) are flows (arrows) from the producer span to the consumer span.
    /// The span and link signals of the contexts needed for the trace are added to the measurement, see --trace-auto.
    /// May be combined with --csv, the added signals are not written to the CSV file.
    #[arg(long, default_value = "", value_name = "FILE")]
    perfetto: String,

    // --trace-auto
    /// Trace signals added to the measurement with --perfetto: off (none, only hints which signals are missing),
    /// context (the spans of the contexts of the measured signals), links (also the span links of these contexts and, if there are any,
    /// the spans of all contexts as possible producers), all (the spans and span links of all contexts).
    /// The context of a signal is the prefix of its name or of the name of its event (<context>.<name>).
    #[arg(long, default_value = "links", value_parser = ["off", "context", "links", "all"], value_name = "LEVEL")]
    trace_auto: String,

    // --list-cal
    /// Lists all specified calibration variables (regex) found in the A2L file
    #[arg(long, default_value = "")]
    list_cal: String,

    // --cal
    /// Set calibration variable to a value (format: "variable_name value").
    #[arg(long, value_names = ["NAME", "VALUE"], num_args = 2)]
    cal: Vec<String>,

    /// --test
    /// Execute a test sequence on the XCP server.
    /// If there is a calibration segment named params (see daq_test, c_demo), a calibration consistency test will be performed.
    #[arg(long, default_value_t = false)]
    test: bool,

    // --yes
    /// Automatically confirm EPK mismatch and other safety warnings. Use in scripts to suppress interactive prompts.
    #[arg(long, short = 'y', default_value_t = false)]
    yes: bool,
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
    let confirm_epk_mismatch = || -> Result<(), Box<dyn Error>> {
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
// Helper function to parse destination address with flexible port handling

fn parse_dest_addr(dest_addr: &str, default_port: u16) -> Result<std::net::SocketAddr, Box<dyn Error>> {
    // Try to parse as a complete socket address first (IP:port)
    if let Ok(addr) = dest_addr.parse::<std::net::SocketAddr>() {
        return Ok(addr);
    }

    // If that fails, try to parse as just an IP address and add the default port
    if let Ok(ip) = dest_addr.parse::<std::net::IpAddr>() {
        return Ok(std::net::SocketAddr::new(ip, default_port));
    }

    // If both fail, return an error
    Err(format!("Invalid destination address: '{}'. Expected format: 'IP' or 'IP:port'", dest_addr).into())
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

    // Initialize logging
    let log_level = args.log_level.to_log_level_filter();
    env_logger::Builder::new()
        .target(env_logger::Target::Stdout)
        .filter_level(log_level)
        .format_timestamp(None)
        .format_module_path(false)
        .format_target(false)
        .init();

    info!(
        "xcp_client - version {}.{}.{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH")
    );

    debug!("{:#?}", args);

    // Run the XCP client or the test executor
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{}", e);
            ExitCode::FAILURE
        }
    }
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
