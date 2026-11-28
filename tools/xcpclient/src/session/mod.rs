//-----------------------------------------------------------------------------
// Module session
// A session with an XCP server (target ECU)
//
// A session owns the XCP client and the state of the connection: the target identification, the description of the target
// (registry, loaded from an A2L file or created from an ELF file) and the running measurement.
// It offers the steps xcpclient is made of, one method per step:
//
//   connect               Connect and identify the target (GET_ID)
//   load_description      Upload, create or load the A2L file, see module description
//   find_parameters, read_parameter, write_parameter, upload_parameters, download_parameters
//   find_measurements, describe_measurement, start_measurement, stop_measurement
//   disconnect
//
// The command line (main.rs) calls these steps in a fixed order, controlled by its arguments. Decisions which need the user
// (EPK mismatch) are callbacks, output to the console is left to the caller.

use std::error::Error;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};
use parking_lot::Mutex;

use crate::bin_reader;
use crate::console_writer::ConsoleWriter;
use crate::csv_writer::CsvWriter;
use crate::daq_writer::{DaqDecoder, DaqWriter};
use crate::mcap_writer::McapWriter;
use crate::mdf_writer::MdfWriter;
use crate::perfetto_writer::{self, PerfettoWriter, TraceAuto};
use crate::xcp_client::*;

mod default_event;
pub use default_event::DefaultEvent;

mod description;
pub use description::DescriptionOptions;

//------------------------------------------------------------------------
// Calibration parameters

/// Value of a calibration parameter, in the encoding of its A2L type
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParameterValue {
    Signed(i64),
    Unsigned(u64),
    Float(f64),
}

/// A calibration parameter with its current value
#[derive(Debug, Clone)]
pub struct ParameterInfo {
    pub name: String,
    pub addr: A2lAddr,
    pub encoding: A2lTypeEncoding,
    /// None if not connected or if the parameter is not a basic type value (blob)
    pub value: Option<ParameterValue>,
}

//------------------------------------------------------------------------
// Measurement

/// Outputs of a measurement, an empty file name means no output in this format
#[derive(Debug, Clone, Default)]
pub struct MeasurementOptions {
    /// CSV file
    pub csv: String,
    /// ASAM MDF 4.1 file
    pub mdf: String,
    /// MCAP file
    pub mcap: String,
    /// Perfetto trace file (Chrome Trace Event JSON format)
    pub perfetto: String,
    /// Trace signals added to the measurement with a Perfetto trace file: off, context, links, all
    pub trace_auto: String,
    /// Print the measurement data to the console (stdout), if there is no output file
    pub console: bool,
}

/// Statistics of a measurement, available after its stop
#[derive(Debug, Clone, Copy)]
pub struct MeasurementSummary {
    /// Duration from start to stop in us
    pub elapsed_us: u128,
    pub event_count: usize,
    pub byte_count: usize,
    pub packet_count: usize,
    pub lost_count: usize,
    pub dropped_count: usize,
}

impl MeasurementSummary {
    /// Log the statistics, warnings for lost packets and dropped samples
    pub fn log(&self) {
        info!(
            "Measurement done, {} events, {:.0} event/s, {:.3} Mbytes/s, {} packets, {} packets lost",
            self.event_count,
            self.event_count as f64 * 1_000_000.0 / self.elapsed_us as f64,
            self.byte_count as f64 / self.elapsed_us as f64,
            self.packet_count,
            self.lost_count
        );
        if self.lost_count > 0 {
            warn!(
                "{} of {} DAQ packets lost ({:.3}%), in the transmit queue of the XCP server or in the UDP receive buffer",
                self.lost_count,
                self.packet_count + self.lost_count,
                self.lost_count as f64 * 100.0 / (self.packet_count + self.lost_count) as f64
            );
        }
        if self.dropped_count > 0 {
            warn!(
                "{} incomplete samples of DAQ lists with more than one ODT dropped, an ODT was lost or out of sequence",
                self.dropped_count
            );
        }
    }
}

// Create the writers for the measurement data output, without an output file the data is printed to the console if enabled
// contexts are the known contexts and trace_signals the signals added to the measurement for a Perfetto trace (see add_trace_signals)
fn create_daq_writers(options: &MeasurementOptions, ecu_name: &str, contexts: &[String], trace_signals: &[String]) -> Result<Vec<Box<dyn DaqWriter>>, Box<dyn Error>> {
    let mut writers: Vec<Box<dyn DaqWriter>> = Vec::new();
    if !options.csv.is_empty() {
        let writer = CsvWriter::create(&options.csv).map_err(|e| format!("Failed to create CSV file '{}': {}", options.csv, e))?;
        info!("CSV output to file: {}", options.csv);
        // The trace signals are not written to the CSV file
        writers.push(Box::new(writer.with_skip(trace_signals)));
    }
    if !options.mdf.is_empty() {
        let writer = MdfWriter::create(&options.mdf).map_err(|e| format!("Failed to create MDF file '{}': {}", options.mdf, e))?;
        info!("MDF output to file: {}", options.mdf);
        writers.push(Box::new(writer.with_ecu_name(ecu_name)));
    }
    if !options.mcap.is_empty() {
        let writer = McapWriter::create(&options.mcap).map_err(|e| format!("Failed to create MCAP file '{}': {}", options.mcap, e))?;
        info!("MCAP output to file: {}", options.mcap);
        writers.push(Box::new(writer.with_ecu_name(ecu_name)));
    }
    if !options.perfetto.is_empty() {
        let mut writer = PerfettoWriter::create(&options.perfetto).map_err(|e| format!("Failed to create Perfetto trace file '{}': {}", options.perfetto, e))?;
        info!("Perfetto trace output to file: {}", options.perfetto);
        writer.set_ecu_name(ecu_name);
        writer.set_contexts(contexts);
        writers.push(Box::new(writer));
    }
    if writers.is_empty() && options.console {
        writers.push(Box::new(ConsoleWriter::stdout()));
    }
    Ok(writers)
}

// Trace signals for a Perfetto trace
// Adds the span and link signals needed for the trace to the measurement list, at the level given with trace_auto
// Returns the measurement list, the known contexts (each is a process in the Perfetto trace) and the added signals
fn add_trace_signals(xcp_client: &XcpClient, trace_auto: &str, mut list: Vec<String>) -> (Vec<String>, Vec<String>, Vec<String>) {
    let level: TraceAuto = trace_auto.parse().unwrap_or(TraceAuto::Links);
    let measurements: Vec<(String, Option<String>)> = xcp_client
        .find_measurements(".*")
        .into_iter()
        .map(|name| {
            let event = xcp_client.measurement_event_name(&name);
            (name, event)
        })
        .collect();
    let contexts = perfetto_writer::find_contexts(measurements.iter().map(|(n, _)| n.as_str()));
    if contexts.is_empty() {
        return (list, contexts, Vec::new());
    }

    let added = perfetto_writer::trace_signals(level, &measurements, &list);
    if level == TraceAuto::Off {
        let missing = perfetto_writer::trace_signals(TraceAuto::Links, &measurements, &list);
        if !missing.is_empty() {
            warn!(
                "Perfetto trace: the spans and span links of the contexts are not measured, add them with --trace-auto links or measure them: {}",
                missing.join(" ")
            );
        }
    } else if !added.is_empty() {
        info!("Perfetto trace: {} trace signals added (--trace-auto {}): {}", added.len(), trace_auto, added.join(" "));
    }
    list.extend(added.iter().cloned());
    (list, contexts, added)
}

//------------------------------------------------------------------------
// Session

/// A session with an XCP server
pub struct Session {
    protocol: &'static str,
    dest_addr: SocketAddr,
    baud_rate: u32,

    xcp_client: XcpClient,

    // DAQ decoder, the measurement data writers are set at measurement start
    daq_decoder: Option<Arc<Mutex<DaqDecoder>>>,

    // Target ECU name (from GET_ID)
    ecu_name: String,
    // A2L name (from GET_ID)
    a2l_name: String,
    // EPK reported by the target, empty when offline or unsupported
    ecu_epk: String,

    // A2L file, set by load_description
    a2l_path: PathBuf,
    // Calibration segment relative addressing mode
    segment_relative: bool,

    // Start time of the running measurement
    measurement_start: Option<tokio::time::Instant>,
}

impl Session {
    /// Create a session, not connected
    /// protocol is "TCP", "UDP" or "SxI"
    pub fn new(protocol: &'static str, dest_addr: SocketAddr, bind_addr: SocketAddr, baud_rate: u32) -> Session {
        Session {
            protocol,
            dest_addr,
            baud_rate,
            xcp_client: XcpClient::new(protocol, dest_addr, bind_addr, baud_rate),
            daq_decoder: None,
            ecu_name: String::new(),
            a2l_name: String::new(),
            ecu_epk: String::new(),
            a2l_path: PathBuf::new(),
            segment_relative: false,
            measurement_start: None,
        }
    }

    /// Name of the target ECU from GET_ID, empty if not connected
    pub fn ecu_name(&self) -> &str {
        &self.ecu_name
    }

    /// EPK reported by the target, empty if not connected or not supported by the target
    pub fn ecu_epk(&self) -> &str {
        &self.ecu_epk
    }

    /// The A2L file used, valid after load_description
    pub fn a2l_path(&self) -> &std::path::Path {
        &self.a2l_path
    }

    pub fn is_connected(&mut self) -> bool {
        self.xcp_client.is_connected()
    }

    //------------------------------------------------------------------------
    // Connect, disconnect

    /// Connect to the XCP server and read the target identification: ECU name, A2L name and EPK
    /// text_decoder gets the SERV_TEXT messages of the target
    pub async fn connect<T>(&mut self, connect_mode: u8, text_decoder: T) -> Result<(), Box<dyn Error>>
    where
        T: XcpTextDecoder + Send + 'static,
    {
        // Connect to the XCP server
        // Print protocol information
        info!("XCP Connect using {}", self.protocol);
        let daq_decoder = Arc::new(Mutex::new(DaqDecoder::new()));
        self.daq_decoder = Some(Arc::clone(&daq_decoder));
        match self.xcp_client.connect(connect_mode, Arc::clone(&daq_decoder), text_decoder).await {
            Ok(_) => {
                info!("Connected to XCP server at {}", self.dest_addr);
            }
            Err(e) => {
                error!("Failed to connect to XCP server at {}, Error: {}", self.dest_addr, e);
                return Err("Failed to connect to XCP server".into());
            }
        }
        self.xcp_client.log_connect_info();
        assert!((self.xcp_client.comm_mode_basic() & 0x07) == 0); // Address granularity != 1 and motorola format not supported

        info!("Reading target ECU information via XCP GET_ID commands:");

        // Get target ECU name
        let res = self.xcp_client.get_id(xcp::IDT_ASCII).await;
        match res {
            Ok((_, Some(id))) => {
                self.ecu_name = id;
                info!("  GET_ID IDT_ASCII = {}", self.ecu_name);
            }
            Err(e) => {
                error!("GET_ID IDT_ASCII failed, Error: {}", e);
            }
            _ => {
                panic!("Empty string");
            }
        };

        // Get A2L name
        let res = self.xcp_client.get_id(xcp::IDT_ASAM_NAME).await;
        match res {
            Ok((_, Some(id))) => {
                self.a2l_name = id;
                info!("  GET_ID IDT_ASAM_NAME = {}", self.a2l_name);
            }
            Err(e) => {
                error!("GET_ID IDT_ASAM_NAME failed, Error: {}", e);
            }
            _ => {
                panic!("Empty string");
            }
        };

        // Get EPK
        let res = self.xcp_client.get_id(xcp::IDT_ASAM_EPK).await;
        match res {
            Ok((_, Some(id))) => {
                info!("  GET_ID IDT_EPK = {}", id);
                self.ecu_epk = id;
            }
            Err(e) => {
                warn!("GET_ID IDT_ASAM_EPK failed, Error: {}", e);
            }
            _ => {
                panic!("Empty string");
            }
        };

        Ok(())
    }

    /// Disconnect from the XCP server, does nothing if not connected
    pub async fn disconnect(&mut self) -> Result<(), Box<dyn Error>> {
        if self.xcp_client.is_connected() {
            self.xcp_client.disconnect().await?;
            info!("XCP Disconnected");
        }
        Ok(())
    }

    //------------------------------------------------------------------------
    // Calibration parameters

    /// Names of the calibration parameters matching a regular expression
    pub fn find_parameters(&self, pattern: &str) -> Vec<String> {
        self.xcp_client.find_characteristics(pattern)
    }

    /// A calibration parameter by name, with its current value uploaded from the target if connected
    pub async fn read_parameter(&mut self, name: &str) -> Result<ParameterInfo, Box<dyn Error>> {
        let h: XcpCalibrationObjectHandle = self.xcp_client.create_calibration_object(name).await?;
        let connected = self.xcp_client.is_connected();
        let (name, addr, encoding) = {
            let o = self.xcp_client.get_calibration_object(h);
            (o.get_name().to_string(), o.get_a2l_addr(), o.get_a2l_type().encoding)
        };
        let value = if connected {
            match encoding {
                A2lTypeEncoding::Signed => Some(ParameterValue::Signed(self.xcp_client.get_value_i64(h))),
                A2lTypeEncoding::Unsigned => Some(ParameterValue::Unsigned(self.xcp_client.get_value_u64(h))),
                A2lTypeEncoding::Float => Some(ParameterValue::Float(self.xcp_client.get_value_f64(h))),
                A2lTypeEncoding::Blob => None,
            }
        } else {
            None
        };
        Ok(ParameterInfo { name, addr, encoding, value })
    }

    /// Set a calibration parameter to a value
    pub async fn write_parameter(&mut self, name: &str, value: f64) -> Result<(), Box<dyn Error>> {
        info!("Setting calibration variable '{}' to {}", name, value);

        // Create calibration object
        let handle = self
            .xcp_client
            .create_calibration_object(name)
            .await
            .map_err(|e| format!("Failed to create calibration object for '{}': {}", name, e))?;

        // Set the value
        self.xcp_client
            .set_value_f64(handle, value)
            .await
            .map_err(|e| format!("Failed to set value for '{}': {}", name, e))?;

        info!("Successfully set '{}' = {}", name, value);
        Ok(())
    }

    // Path of the file for calibration segment data, built from the ECU name if no file name is given
    fn parameter_file_path(&mut self, filename: &str) -> Result<PathBuf, Box<dyn Error>> {
        let path = if filename.is_empty() {
            // No filename given, build HEX file path from ECU name
            std::path::Path::new(&self.ecu_name).with_extension("hex")
        } else {
            std::path::Path::new(filename).to_path_buf()
        };
        if !self.xcp_client.is_connected() {
            return Err("XCP client not connected, cannot upload or download calibration segment data".into());
        }
        Ok(path)
    }

    /// Upload the working page data of all calibration segments from the target and store it into a file (.hex or .bin)
    /// (Note: In XCP, Upload means from target to file, Download means from file to target)
    pub async fn upload_parameters(&mut self, filename: &str) -> Result<(), Box<dyn Error>> {
        let path = self.parameter_file_path(filename)?;
        // The context list of the target is the enum conversion table conv.contexts in the A2L file
        let contexts = std::fs::read_to_string(&self.a2l_path)
            .ok()
            .and_then(|a2l_text| bin_reader::parse_a2l_context_table(&a2l_text))
            .unwrap_or_default();
        if !contexts.is_empty() {
            info!("{} contexts found in A2L file {}", contexts.len(), self.a2l_path.display());
        }
        self.xcp_client.save_calibration_segments_to_file(&path, &contexts).await
    }

    /// Download the working page data of all calibration segments from a file (.hex or .bin) to the target
    pub async fn download_parameters(&mut self, filename: &str) -> Result<(), Box<dyn Error>> {
        let path = self.parameter_file_path(filename)?;
        self.xcp_client.load_calibration_segments_from_file(&path).await
    }

    //------------------------------------------------------------------------
    // Measurement

    /// Names of the measurable objects matching a regular expression
    pub fn find_measurements(&self, pattern: &str) -> Vec<String> {
        self.xcp_client.find_measurements(pattern)
    }

    /// Description of a measurable object: name, address, event and type, None if it can not be measured
    pub fn describe_measurement(&self, name: &str) -> Option<String> {
        self.xcp_client.describe_measurement(name)
    }

    /// Start a measurement
    /// measurement_list is a list of names or, with one element, a regular expression
    /// Returns false if there is nothing to measure, the measurement is not started then
    pub async fn start_measurement(&mut self, measurement_list: Vec<String>, options: &MeasurementOptions) -> Result<bool, Box<dyn Error>> {
        // Create list of measurement variable names
        let list = if measurement_list.len() == 1 {
            // Regular expression
            self.xcp_client.find_measurements(measurement_list[0].as_str())
        } else {
            // Just a list of names
            measurement_list
        };
        // Trace signals for a Perfetto trace
        let (list, contexts, trace_signals) = if options.perfetto.is_empty() {
            (list, Vec::new(), Vec::new())
        } else {
            add_trace_signals(&self.xcp_client, &options.trace_auto, list)
        };
        if list.is_empty() {
            warn!("No measurement variables found");
            return Ok(false);
        }

        // Create measurement objects for all names in the list
        // Multi dimensional objects not supported yet
        info!("===============================================================");
        info!("Measurement list:");
        for name in &list {
            if let Some(o) = self.xcp_client.create_measurement_object(name) {
                info!(r#"  {}: {}"#, o.0, name);
            }
        }

        // Measurement data output
        let writers = create_daq_writers(options, &self.ecu_name, &contexts, &trace_signals)?;
        if let Some(daq_decoder) = &self.daq_decoder {
            daq_decoder.lock().set_writers(writers);
        }

        // Measure
        // 32 bit DAQ timestamp will overflow after 4.2s
        self.measurement_start = Some(tokio::time::Instant::now());
        self.xcp_client.start_measurement().await?;
        Ok(true)
    }

    /// The DAQ decoder with the counters of the running measurement (events, bytes, lost packets)
    pub fn daq_decoder(&mut self) -> Option<Arc<Mutex<dyn XcpDaqDecoder>>> {
        self.xcp_client.get_daq_decoder()
    }

    /// Stop the measurement, complete the output files
    /// Returns the statistics of the measurement
    pub async fn stop_measurement(&mut self) -> Result<Option<MeasurementSummary>, Box<dyn Error>> {
        self.xcp_client.stop_measurement().await?;
        let elapsed_us = self.measurement_start.take().map(|t| t.elapsed().as_micros()).unwrap_or(0);

        // Statistics from DAQ decoder
        let summary = self.xcp_client.get_daq_decoder().map(|daq_decoder| {
            let daq_decoder = daq_decoder.lock();
            MeasurementSummary {
                elapsed_us,
                event_count: daq_decoder.get_event_count(),
                byte_count: daq_decoder.get_byte_count(),
                packet_count: daq_decoder.get_packet_count(),
                lost_count: daq_decoder.get_lost_count(),
                dropped_count: daq_decoder.get_dropped_count(),
            }
        });
        Ok(summary)
    }
}
