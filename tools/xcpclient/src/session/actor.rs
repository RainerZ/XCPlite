//-----------------------------------------------------------------------------
// Module session::actor
// The session actor: the one XCP session of a long running process (MCP server mode)
//
// The actor is a task on its own thread which owns the session (Session) and its state. It takes one command at a time from a
// channel and answers it, so the XCP commands of concurrent callers are serialized without locks. The callers hold a
// SessionHandle, which can be cloned and used from any thread or tokio runtime.
//
// The commands are the operations of the MCP tools (docs/MCP.md): status, connect, disconnect, list, read, write, and the
// measurement operations sample, measure start and measure stop in the module measure, and the A2L and parameter file
// operations in the module files.
// A reply is a text for the reader of the tool result (a language model), Ok for a result and Err for an error message which
// says what to do next.
//
// State of the actor:
//   disconnected   No session. After a lost connection the reason is kept for status
//   connected      Session connected, no description of the target (A2L) loaded
//   ready          Session connected, description loaded
//   measuring      A measurement into files runs in the background (recording), started by measure start
//
// A lost connection is detected when a command fails: the actor then checks with GET_STATUS whether the XCP server still
// responds. There is no keep-alive and no automatic reconnect, see docs/MCP.md.
// A panic while a command is executed is caught, the session is dropped and the caller gets an error.

use std::collections::VecDeque;
use std::future::Future;
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::FutureExt;
#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};

use super::{DefaultEvent, DescriptionSource, EpkCheck, ParameterInfo, ParameterValue, Session, TargetInfo};
use crate::value_writer::{LastValue, ValueStore};
use crate::xcp_client::{A2lTypeEncoding, A2lValue, XcpMeasurementInfo, XcpTextDecoder};

mod measure;
pub use measure::{MeasureRequest, SAMPLE_DURATION_DEFAULT_MS, SAMPLE_DURATION_MAX_MS, SAMPLE_NAMES_MAX, SampleRequest};
use measure::{PendingSample, Recording};

mod files;
use files::warnings_text;
pub use files::{CreateA2lRequest, LoadA2lRequest};

/// The A2ML description of XCP, an A2L file uploaded from an XCPlite target includes it by name
const XCP_104_AML: &str = include_str!("../../XCP_104.aml");
const XCP_104_AML_NAME: &str = "XCP_104.aml";

/// Number of SERV_TEXT lines of the target kept for status
const SERV_TEXT_LINES: usize = 20;

/// Limits of the list and read operations
pub const LIST_LIMIT_DEFAULT: usize = 50;
pub const LIST_LIMIT_MAX: usize = 200;
pub const READ_NAMES_MAX: usize = 50;
/// Number of array elements shown for a value
const ARRAY_ELEMENTS_SHOWN: usize = 16;

//------------------------------------------------------------------------
// Requests

/// Transport protocol of XCP on Ethernet
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransportProtocol {
    Udp,
    Tcp,
}

impl TransportProtocol {
    /// Protocol name as the session and the XCP client use it
    pub fn as_str(self) -> &'static str {
        match self {
            TransportProtocol::Udp => "UDP",
            TransportProtocol::Tcp => "TCP",
        }
    }
}

/// Defaults for the connect request, from the command line and the config file
#[derive(Debug, Clone)]
pub struct SessionDefaults {
    /// Address of the target: IP address, IP:port or host name
    pub dest_addr: String,
    pub port: u16,
    /// None: UDP
    pub protocol: Option<TransportProtocol>,
    pub bind_addr: SocketAddr,
    pub connect_mode: u8,
    /// A2L file to load instead of an upload from the target, empty: none
    pub a2l: String,
    /// ELF file to create the A2L file from, empty: none
    pub elf: String,
    pub default_event: Option<DefaultEvent>,
    /// Directory for the files the session writes on its own: an uploaded A2L file, XCP_104.aml, an A2L file created from an
    /// ELF file without a given name. It is created when needed, see default_file_dir
    pub file_dir: PathBuf,
}

/// The default directory for the files the session writes on its own: the cache directory of the user for xcpclient
/// (macOS ~/Library/Caches/xcpclient, Windows %LOCALAPPDATA%\xcpclient\cache, others $XDG_CACHE_HOME/xcpclient or ~/.cache/xcpclient),
/// the temporary directory if there is none
/// A directory of its own, so a session can never overwrite files in the working directory, where an XCPlite application
/// started in the same directory writes its A2L file
pub fn default_file_dir() -> PathBuf {
    let env_dir = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from);
    let cache_dir = if cfg!(target_os = "macos") {
        env_dir("HOME").map(|home| home.join("Library").join("Caches").join("xcpclient"))
    } else if cfg!(windows) {
        env_dir("LOCALAPPDATA").map(|dir| dir.join("xcpclient").join("cache"))
    } else {
        env_dir("XDG_CACHE_HOME")
            .or_else(|| env_dir("HOME").map(|home| home.join(".cache")))
            .map(|dir| dir.join("xcpclient"))
    };
    cache_dir.unwrap_or_else(|| std::env::temp_dir().join("xcpclient"))
}

/// Connect to a target and load its description, None: the default
#[derive(Debug, Clone, Default)]
pub struct ConnectRequest {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub protocol: Option<TransportProtocol>,
    pub a2l: Option<String>,
    pub elf: Option<String>,
    /// Event id or name for variables without a fixed event
    pub default_event: Option<String>,
    /// Use an A2L file with an EPK which differs from the EPK of the target
    pub accept_epk_mismatch: bool,
}

impl From<&ConnectRequest> for LoadA2lRequest {
    /// The part of a connect request which is about the description of the target
    fn from(request: &ConnectRequest) -> LoadA2lRequest {
        LoadA2lRequest {
            a2l: request.a2l.clone(),
            elf: request.elf.clone(),
            fix: false,
            default_event: request.default_event.clone(),
            accept_epk_mismatch: request.accept_epk_mismatch,
        }
    }
}

/// List the objects matching a regular expression, one page
#[derive(Debug, Clone)]
pub struct ListRequest {
    pub pattern: String,
    pub limit: usize,
    pub offset: usize,
}

type ReplyTx = oneshot::Sender<Result<String, String>>;

enum Command {
    Status(ReplyTx),
    Connect(ConnectRequest, ReplyTx),
    Disconnect(ReplyTx),
    ListMeasurements(ListRequest, ReplyTx),
    ListParameters(ListRequest, ReplyTx),
    Read(Vec<String>, ReplyTx),
    Write(String, f64, ReplyTx),
    Sample(SampleRequest, ReplyTx),
    MeasureStart(MeasureRequest, ReplyTx),
    MeasureStop(ReplyTx),
    LoadA2l(LoadA2lRequest, ReplyTx),
    CreateA2l(CreateA2lRequest, ReplyTx),
    SaveParameters(Option<String>, ReplyTx),
    LoadParameters(String, ReplyTx),
    Shutdown(oneshot::Sender<()>),
}

//------------------------------------------------------------------------
// SessionHandle

/// Handle to the session actor
#[derive(Clone)]
pub struct SessionHandle {
    tx: mpsc::Sender<Command>,
}

impl SessionHandle {
    /// Start the session actor on its own thread, not connected
    pub fn spawn(defaults: SessionDefaults) -> std::io::Result<SessionHandle> {
        let (tx, rx) = mpsc::channel(16);
        std::thread::Builder::new().name("xcp-session".into()).spawn(move || {
            // The futures of the XCP client are not Send (Box<dyn Error>), the actor has its own single threaded runtime
            match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime.block_on(SessionActor::new(defaults).run(rx)),
                Err(e) => error!("Session actor: could not create the tokio runtime: {}", e),
            }
        })?;
        Ok(SessionHandle { tx })
    }

    async fn request(&self, command: impl FnOnce(ReplyTx) -> Command) -> Result<String, String> {
        const TERMINATED: &str = "The XCP session task of xcpclient has terminated, restart the xcpclient MCP server";
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx.send(command(reply_tx)).await.map_err(|_| TERMINATED.to_string())?;
        reply_rx.await.map_err(|_| TERMINATED.to_string())?
    }

    /// State of the session: connection, target, description
    pub async fn status(&self) -> Result<String, String> {
        self.request(Command::Status).await
    }

    /// Connect to a target and load its description
    pub async fn connect(&self, request: ConnectRequest) -> Result<String, String> {
        self.request(|reply| Command::Connect(request, reply)).await
    }

    pub async fn disconnect(&self) -> Result<String, String> {
        self.request(Command::Disconnect).await
    }

    /// List the measurable objects matching a regular expression
    pub async fn list_measurements(&self, request: ListRequest) -> Result<String, String> {
        self.request(|reply| Command::ListMeasurements(request, reply)).await
    }

    /// List the calibration parameters matching a regular expression with their current values
    pub async fn list_parameters(&self, request: ListRequest) -> Result<String, String> {
        self.request(|reply| Command::ListParameters(request, reply)).await
    }

    /// Read the current values of parameters and measurement variables
    pub async fn read(&self, names: Vec<String>) -> Result<String, String> {
        self.request(|reply| Command::Read(names, reply)).await
    }

    /// Set a calibration parameter
    pub async fn write(&self, name: String, value: f64) -> Result<String, String> {
        self.request(|reply| Command::Write(name, value, reply)).await
    }

    /// Measure signals for a time window and return statistics, the reply comes when the window is over
    pub async fn sample(&self, request: SampleRequest) -> Result<String, String> {
        self.request(|reply| Command::Sample(request, reply)).await
    }

    /// Start a measurement into files, it runs in the background until it is stopped or its duration is over
    pub async fn measure_start(&self, request: MeasureRequest) -> Result<String, String> {
        self.request(|reply| Command::MeasureStart(request, reply)).await
    }

    /// Stop the measurement started with measure_start
    pub async fn measure_stop(&self) -> Result<String, String> {
        self.request(Command::MeasureStop).await
    }

    /// Load or replace the description of the connected target
    pub async fn load_a2l(&self, request: LoadA2lRequest) -> Result<String, String> {
        self.request(|reply| Command::LoadA2l(request, reply)).await
    }

    /// Create an A2L file from an ELF file, the description of the session is not changed
    pub async fn create_a2l(&self, request: CreateA2lRequest) -> Result<String, String> {
        self.request(|reply| Command::CreateA2l(request, reply)).await
    }

    /// Save the calibration segments of the target into a file, None: a file named after the target and the time
    pub async fn save_parameters(&self, file: Option<String>) -> Result<String, String> {
        self.request(|reply| Command::SaveParameters(file, reply)).await
    }

    /// Load a parameter file into the calibration segments of the target
    pub async fn load_parameters(&self, file: String) -> Result<String, String> {
        self.request(|reply| Command::LoadParameters(file, reply)).await
    }

    /// Stop a running measurement, disconnect and stop the actor, returns when it is done
    pub async fn shutdown(&self) {
        let (done_tx, done_rx) = oneshot::channel();
        if self.tx.send(Command::Shutdown(done_tx)).await.is_ok() {
            let _ = done_rx.await;
        }
    }
}

//------------------------------------------------------------------------
// SERV_TEXT of the target

#[derive(Default)]
struct ServText {
    lines: VecDeque<String>,
    partial: String,
}

/// Text decoder which keeps the last lines of the SERV_TEXT messages of the target
struct ServTextBuffer(Arc<Mutex<ServText>>);

impl XcpTextDecoder for ServTextBuffer {
    fn decode(&self, data: &[u8]) {
        let mut text = self.0.lock();
        for &c in data {
            match c {
                b'\n' => {
                    let line = std::mem::take(&mut text.partial);
                    if text.lines.len() >= SERV_TEXT_LINES {
                        text.lines.pop_front();
                    }
                    text.lines.push_back(line);
                }
                0 | b'\r' => {}
                // Text from the target is data, keep printable characters only and limit the line length
                c if text.partial.len() < 200 => text.partial.push(if c.is_ascii_graphic() || c == b' ' { c as char } else { '?' }),
                _ => {}
            }
        }
    }
}

//------------------------------------------------------------------------
// Text formatting

/// A duration in a readable unit: "12 s", "4 min", "2 h 5 min"
pub fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s < 120 {
        format!("{} s", s)
    } else if s < 2 * 3600 {
        format!("{} min", s / 60)
    } else {
        format!("{} h {} min", s / 3600, (s % 3600) / 60)
    }
}

/// The age of a sample: "3 ms", "12 s"
fn format_age(age: Duration) -> String {
    if age < Duration::from_secs(1) {
        format!("{} ms", age.as_millis())
    } else {
        format_duration(age)
    }
}

/// A number with at most 6 significant digits, for statistics: "999.5", "0.301", "1.5e-7"
fn format_significant(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return v.to_string();
    }
    let magnitude = v.abs().log10().floor() as i32;
    if !(-5..15).contains(&magnitude) {
        return format!("{:.5e}", v);
    }
    let text = format!("{:.*}", (5 - magnitude).clamp(0, 9) as usize, v);
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

/// A file size: "512 bytes", "88 KB", "12.3 MB"
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} bytes", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// A number without the digits of a very large or very small f64 (limits like f64::MAX)
fn format_number(v: f64) -> String {
    if v != 0.0 && (v.abs() >= 1e15 || v.abs() < 1e-6) {
        format!("{:e}", v)
    } else {
        format!("{}", v)
    }
}

/// Type name of a basic type: u16, i32, f64
fn type_name(encoding: A2lTypeEncoding, size: usize) -> String {
    match encoding {
        A2lTypeEncoding::Unsigned => format!("u{}", size * 8),
        A2lTypeEncoding::Signed => format!("i{}", size * 8),
        A2lTypeEncoding::Float => format!("f{}", size * 8),
        A2lTypeEncoding::Blob => format!("{} bytes", size),
    }
}

fn parameter_value(p: &ParameterInfo) -> String {
    match p.value {
        Some(ParameterValue::Signed(v)) => v.to_string(),
        Some(ParameterValue::Unsigned(v)) => v.to_string(),
        // The shortest decimal representation of a f32 ("0.301", not "0.3009999990463257")
        Some(ParameterValue::Float(v)) if p.size == 4 => (v as f32).to_string(),
        Some(ParameterValue::Float(v)) => v.to_string(),
        None => "?".to_string(),
    }
}

fn parameter_limits(p: &ParameterInfo) -> String {
    format!("{}..{}", format_number(p.limits.lower), format_number(p.limits.upper))
}

/// One line of the parameter list: name, type, value and limits
fn parameter_line(p: &ParameterInfo) -> String {
    format!("{}  {} = {}  limits {}", p.name, type_name(p.encoding, p.size), parameter_value(p), parameter_limits(p))
}

/// One line of the measurement list: name, type with dimensions, unit and event
fn measurement_line(m: &XcpMeasurementInfo) -> String {
    let mut line = format!("{}  {}", m.name, type_name(m.a2l_type.encoding, m.a2l_type.size));
    if m.dim[1] > 1 {
        line.push_str(&format!("[{}x{}]", m.dim[0], m.dim[1]));
    } else if m.dim[0] > 1 {
        line.push_str(&format!("[{}]", m.dim[0]));
    }
    if !m.unit.is_empty() {
        line.push_str(&format!("  unit {}", m.unit));
    }
    match (&m.event_name, m.a2l_addr.event) {
        (Some(name), _) => line.push_str(&format!("  event {}", name)),
        (None, Some(id)) => line.push_str(&format!("  event {}", id)),
        (None, None) => line.push_str("  no event, needs default_event"),
    }
    if m.characteristic {
        line.push_str("  (parameter)");
    }
    line
}

/// The values of an object with unit: "17.5 Volt", "[0 1 2 ...] (16 of 32 elements)"
fn format_values(values: &[A2lValue], unit: &str) -> String {
    let mut text = if values.len() == 1 {
        values[0].to_string()
    } else {
        let shown: Vec<String> = values.iter().take(ARRAY_ELEMENTS_SHOWN).map(|v| v.to_string()).collect();
        if values.len() > ARRAY_ELEMENTS_SHOWN {
            format!("[{} ...] ({} of {} elements)", shown.join(" "), ARRAY_ELEMENTS_SHOWN, values.len())
        } else {
            format!("[{}]", shown.join(" "))
        }
    };
    // The unit of an enum is its label list, the value is the label then
    if !unit.is_empty() && !values.iter().any(|v| v.as_label().is_some()) {
        text.push(' ');
        text.push_str(unit);
    }
    text
}

/// Header line of a list page
fn list_header(shown: usize, total: usize, what: &str, request: &ListRequest) -> String {
    let mut header = format!("{} of {} {} matching '{}'", shown, total, what, request.pattern);
    if request.offset > 0 {
        header.push_str(&format!(", from offset {}", request.offset));
    }
    if request.offset + shown < total {
        header.push_str(&format!(", next page: offset {}. Use a more specific pattern to get fewer", request.offset + shown));
    }
    header
}

const NOT_CONNECTED: &str = "Not connected. Call xcp_connect first.";
const NO_DESCRIPTION: &str = "Connected, but no description of the target (A2L) is loaded. Call xcp_connect again with an A2L file (a2l) or an ELF file (elf).";

//------------------------------------------------------------------------
// SessionActor

struct SessionActor {
    defaults: SessionDefaults,
    session: Option<Session>,
    /// Why the last connection ended unexpectedly and when, shown by status
    last_disconnect: Option<(String, Instant)>,
    serv_text: Arc<Mutex<ServText>>,
    /// The values of the running measurement, filled by a ValueWriter
    values: ValueStore,
    /// The measurement into files which runs in the background
    recording: Option<Recording>,
    /// The sample window which is open, its reply is sent when it is over
    sample: Option<PendingSample>,
    /// Summary of the last measurement into files, shown by status
    last_measurement: Option<String>,
}

type CommandFuture<'a> = Pin<Box<dyn Future<Output = Result<String, String>> + 'a>>;

impl SessionActor {
    fn new(defaults: SessionDefaults) -> SessionActor {
        SessionActor {
            defaults,
            session: None,
            last_disconnect: None,
            serv_text: Arc::new(Mutex::new(ServText::default())),
            values: ValueStore::new(),
            recording: None,
            sample: None,
            last_measurement: None,
        }
    }

    async fn run(mut self, mut rx: mpsc::Receiver<Command>) {
        debug!("Session actor started");
        loop {
            // The next command, or the time when a sample window or a timed measurement is over
            let wakeup = self.next_wakeup();
            tokio::select! {
                command = rx.recv() => {
                    match command {
                        Some(Command::Shutdown(done)) => {
                            self.shutdown().await;
                            let _ = done.send(());
                            debug!("Session actor stopped");
                            return;
                        }
                        Some(command) => self.execute(command).await,
                        // All handles are dropped
                        None => break,
                    }
                }
                _ = tokio::time::sleep_until(wakeup.unwrap_or_else(tokio::time::Instant::now)), if wakeup.is_some() => {
                    if let Err(panic) = AssertUnwindSafe(self.on_wakeup()).catch_unwind().await {
                        self.reset_after_panic(panic);
                    }
                }
            }
        }
        self.shutdown().await;
        debug!("Session actor stopped, no handles left");
    }

    // A panic while a command was executed: the session and all it was doing is dropped
    // Returns the error message for the caller
    fn reset_after_panic(&mut self, panic: Box<dyn std::any::Any + Send>) -> String {
        let message = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown".to_string());
        error!("Session actor: panic while executing a command: {}", message);
        let reason = format!("Internal error in xcpclient ({})", message);
        self.session = None;
        self.recording = None;
        self.last_disconnect = Some((reason.clone(), Instant::now()));
        let message = format!("{}. The XCP session was reset. Call xcp_connect to connect again.", reason);
        if let Some(sample) = self.sample.take() {
            let _ = sample.reply.send(Err(message.clone()));
        }
        message
    }

    // Execute a command and send the reply
    // A panic is caught: the session is dropped, the caller gets an error
    async fn execute(&mut self, command: Command) {
        // A sample window is answered when it is over, the reply is kept until then
        let command = match command {
            Command::Sample(request, reply) => {
                match AssertUnwindSafe(self.sample_start(request)).catch_unwind().await {
                    Ok(Ok(window)) => self.sample = Some(PendingSample { reply, window }),
                    Ok(Err(e)) => {
                        let _ = reply.send(Err(e));
                    }
                    Err(panic) => {
                        let _ = reply.send(Err(self.reset_after_panic(panic)));
                    }
                }
                return;
            }
            command => command,
        };
        // While a sample window is open, the measurement and the connection are not changed
        if let Some(sample) = &self.sample {
            let busy = matches!(
                &command,
                Command::Connect(..) | Command::Disconnect(_) | Command::MeasureStart(..) | Command::MeasureStop(_) | Command::LoadA2l(..) | Command::CreateA2l(..)
            );
            if busy {
                let message = format!(
                    "xcp_sample is running for another {:.1} s, call this tool again when it is done.",
                    sample.remaining().as_secs_f64()
                );
                match command {
                    Command::Connect(_, reply)
                    | Command::Disconnect(reply)
                    | Command::MeasureStart(_, reply)
                    | Command::MeasureStop(reply)
                    | Command::LoadA2l(_, reply)
                    | Command::CreateA2l(_, reply) => {
                        let _ = reply.send(Err(message));
                    }
                    _ => {}
                }
                return;
            }
        }
        let (reply, result) = {
            let (reply, future): (ReplyTx, CommandFuture) = match command {
                Command::Status(reply) => (reply, Box::pin(async { Ok(self.status()) })),
                Command::Connect(request, reply) => (reply, Box::pin(self.connect(request))),
                Command::Disconnect(reply) => (reply, Box::pin(self.disconnect())),
                Command::ListMeasurements(request, reply) => (reply, Box::pin(self.list_measurements(request))),
                Command::ListParameters(request, reply) => (reply, Box::pin(self.list_parameters(request))),
                Command::Read(names, reply) => (reply, Box::pin(self.read(names))),
                Command::Write(name, value, reply) => (reply, Box::pin(self.write(name, value))),
                Command::MeasureStart(request, reply) => (reply, Box::pin(self.measure_start(request))),
                Command::MeasureStop(reply) => (reply, Box::pin(self.measure_stop())),
                Command::LoadA2l(request, reply) => (reply, Box::pin(self.load_a2l(request))),
                Command::CreateA2l(request, reply) => (reply, Box::pin(self.create_a2l(request))),
                Command::SaveParameters(file, reply) => (reply, Box::pin(self.save_parameters(file))),
                Command::LoadParameters(file, reply) => (reply, Box::pin(self.load_parameters(file))),
                Command::Sample(..) | Command::Shutdown(_) => return,
            };
            (reply, AssertUnwindSafe(future).catch_unwind().await)
        };
        let result = match result {
            Ok(result) => result,
            Err(panic) => Err(self.reset_after_panic(panic)),
        };
        let _ = reply.send(result);
    }

    // Stop what is running and disconnect
    async fn shutdown(&mut self) {
        if self.sample.is_some() {
            self.sample_finish(Some("xcpclient is shut down")).await;
        }
        if self.recording.is_some() {
            let _ = self.recording_finish("xcpclient was shut down").await;
        }
        if let Some(mut session) = self.session.take() {
            if let Err(e) = session.disconnect().await {
                warn!("Disconnect failed: {}", e);
            }
        }
    }

    // The session was found not to respond anymore: drop it and keep the reason
    // A running measurement is completed locally, its files are closed
    // Returns the error message for the caller
    fn connection_lost(&mut self, error: &str) -> String {
        let mut session = self.session.take();
        let target = session.as_ref().map(|s| s.target_info().name_and_address()).unwrap_or_default();
        let reason = format!("Connection to {} lost ({})", target, error);
        warn!("{}", reason);
        self.last_disconnect = Some((reason.clone(), Instant::now()));
        let mut message = format!("{}. Call xcp_connect to reconnect.", reason);
        if let Some(session) = session.as_mut() {
            let progress = session.measurement_progress();
            session.abort_measurement();
            if let Some(recording) = self.recording.take() {
                let summary = recording.summary_text(progress, "the connection was lost");
                message = format!("{}\n{}", message, summary);
                self.last_measurement = Some(summary);
            }
        }
        if let Some(sample) = self.sample.take() {
            let _ = sample.reply.send(Err(message.clone()));
        }
        message
    }

    //------------------------------------------------------------------------
    // Status

    // Lines about the connected target and its description
    fn target_lines(&self, session: &Session) -> Vec<String> {
        let target: TargetInfo = session.target_info();
        let description = session.description();
        let mut lines = Vec::new();
        lines.push(format!("Target: {}, connected for {}", target.name_and_address(), format_duration(target.connected_for)));
        let epk = if target.epk.is_empty() { "none".to_string() } else { target.epk.clone() };
        let file = match description.map(|d| d.source) {
            Some(DescriptionSource::Created) => "ELF file",
            _ => "A2L file",
        };
        lines.push(match description.map(|d| d.epk_check) {
            Some(EpkCheck::Match) => format!("EPK: {}, matches the {}", epk, file),
            Some(EpkCheck::MismatchAccepted) => format!("EPK: {}, DIFFERS from the EPK of the {} (accepted), addresses may be wrong", epk, file),
            Some(EpkCheck::NoEpkInA2l) => format!("EPK: {}, the {} has no EPK to compare", epk, file),
            _ => format!("EPK: {}", epk),
        });
        lines.push(format!("XCP: {}", target.xcp));
        lines.push(match description {
            Some(description) => format!("Description: {}", description),
            None => "Description: none".to_string(),
        });
        lines
    }

    fn status(&self) -> String {
        let mut lines = Vec::new();
        match &self.session {
            None => {
                lines.push("State: disconnected".to_string());
                if let Some((reason, time)) = &self.last_disconnect {
                    lines.push(format!("{}, {} ago", reason, format_duration(time.elapsed())));
                }
                let protocol = self.defaults.protocol.unwrap_or(TransportProtocol::Udp).as_str();
                lines.push(format!(
                    "Default target of xcp_connect: {} port {} ({})",
                    self.defaults.dest_addr, self.defaults.port, protocol
                ));
            }
            Some(session) => {
                if self.recording.is_some() {
                    lines.push("State: measuring".to_string());
                } else if session.description().is_some() {
                    lines.push("State: ready".to_string());
                } else {
                    lines.push("State: connected, no description of the target loaded".to_string());
                }
                lines.extend(self.target_lines(session));
                if session.description().is_none() {
                    lines.push("Call xcp_connect again with an A2L file (a2l) or an ELF file (elf) to load a description.".to_string());
                }
                if let Some(recording) = &self.recording {
                    lines.push(format!(
                        "Measurement: {}",
                        recording.progress_text(session.measurement_progress(), self.values.last_sample_age())
                    ));
                }
            }
        }
        if self.recording.is_none()
            && let Some(last_measurement) = &self.last_measurement
        {
            lines.push(format!("Last measurement: {}", last_measurement));
        }
        let serv_text = self.serv_text.lock();
        if !serv_text.lines.is_empty() {
            lines.push("Messages of the target (SERV_TEXT):".to_string());
            lines.extend(serv_text.lines.iter().map(|line| format!("  {}", line)));
        }
        lines.join("\n")
    }

    //------------------------------------------------------------------------
    // Connect, disconnect

    // Address of the target from the request and the defaults: IP address, IP:port or host name
    async fn resolve_dest_addr(&self, request: &ConnectRequest) -> Result<SocketAddr, String> {
        let host = request.host.as_deref().unwrap_or(&self.defaults.dest_addr).trim();
        let default_port = request.port.unwrap_or(self.defaults.port);
        let mut dest_addr = match super::parse_dest_addr(host, default_port) {
            Ok(addr) => addr,
            Err(_) => {
                // Not an IP address, resolve the host name
                let target = if host.contains(':') { host.to_string() } else { format!("{}:{}", host, default_port) };
                let mut addrs: Vec<SocketAddr> = tokio::net::lookup_host(&target)
                    .await
                    .map_err(|e| format!("Invalid target address '{}': {}. Expected an IP address, IP:port or a host name.", host, e))?
                    .collect();
                // XCP on Ethernet with IPv4 first
                addrs.sort_by_key(|a| !a.is_ipv4());
                *addrs.first().ok_or_else(|| format!("Host name '{}' has no address", host))?
            }
        };
        // A port given on its own wins over a port in the address
        if let Some(port) = request.port {
            dest_addr.set_port(port);
        }
        Ok(dest_addr)
    }

    async fn connect(&mut self, request: ConnectRequest) -> Result<String, String> {
        let protocol = request.protocol.or(self.defaults.protocol).unwrap_or(TransportProtocol::Udp);
        let dest_addr = self.resolve_dest_addr(&request).await?;

        if let Some(session) = &self.session {
            // Connected: the same target is not connected again, another target is an error
            let target = session.target_info();
            if target.dest_addr != dest_addr || target.protocol != protocol.as_str() {
                return Err(format!(
                    "Already connected to {}. Call xcp_disconnect first to connect to {} ({}).",
                    target.name_and_address(),
                    dest_addr,
                    protocol.as_str()
                ));
            }
            // A description is loaded again only if the request asks for it
            if session.description().is_some() && request.a2l.is_none() && request.elf.is_none() && !request.accept_epk_mismatch {
                return Ok(format!("Already connected, nothing changed.\n{}", self.status()));
            }
            if self.recording.is_some() {
                return Err("A measurement is running with the loaded description. Stop it with xcp_measure_stop first.".to_string());
            }
        } else {
            let mut session = Session::new(protocol.as_str(), dest_addr, self.defaults.bind_addr, 0);
            if let Err(e) = session.connect(self.defaults.connect_mode, ServTextBuffer(Arc::clone(&self.serv_text))).await {
                let _ = session.disconnect().await;
                let e = e.to_string();
                return Err(format!(
                    "Could not connect to {} ({}): {}. Is the application running, are address, port and protocol (udp or tcp) right?",
                    dest_addr,
                    protocol.as_str(),
                    e.strip_prefix("Failed to connect to XCP server: ").unwrap_or(&e)
                ));
            }
            self.session = Some(session);
            self.last_disconnect = None;
        }

        // Load the description of the target
        let warnings = self.load_description(&LoadA2lRequest::from(&request), "xcp_connect").await?;
        Ok(format!("Connected.\n{}{}", self.status(), warnings_text(&warnings)))
    }

    async fn disconnect(&mut self) -> Result<String, String> {
        // A running measurement is stopped first, its summary is part of the result
        let mut measurement = String::new();
        if self.recording.is_some() {
            measurement = match self.recording_finish("stopped by xcp_disconnect").await {
                Ok(summary) => format!("\n{}", summary),
                // The connection is lost, there is nothing left to disconnect
                Err(e) => return Err(e),
            };
        }
        self.last_disconnect = None;
        match self.session.take() {
            None => Ok("Not connected, nothing to do.".to_string()),
            Some(mut session) => {
                let target = session.target_info().name_and_address();
                if let Err(e) = session.disconnect().await {
                    warn!("Disconnect failed: {}", e);
                }
                Ok(format!("Disconnected from {}.{}", target, measurement))
            }
        }
    }

    //------------------------------------------------------------------------
    // List

    // The session with a loaded description, or the error which says what is missing
    fn ready(session: &mut Option<Session>) -> Result<&mut Session, String> {
        match session {
            None => Err(NOT_CONNECTED.to_string()),
            Some(session) if session.description().is_none() => Err(NO_DESCRIPTION.to_string()),
            Some(session) => Ok(session),
        }
    }

    // Check and normalize a list request
    fn check_list_request(mut request: ListRequest) -> Result<ListRequest, String> {
        if request.pattern.is_empty() {
            request.pattern = ".".to_string();
        }
        regex::Regex::new(&request.pattern).map_err(|e| format!("'{}' is not a valid regular expression: {}", request.pattern, e))?;
        request.limit = request.limit.clamp(1, LIST_LIMIT_MAX);
        Ok(request)
    }

    async fn list_measurements(&mut self, request: ListRequest) -> Result<String, String> {
        let request = Self::check_list_request(request)?;
        let session = Self::ready(&mut self.session)?;
        let names = session.find_measurements(&request.pattern);
        let page: Vec<&String> = names.iter().skip(request.offset).take(request.limit).collect();
        let mut lines = vec![list_header(page.len(), names.len(), "signals", &request)];
        for name in page {
            if let Some(info) = session.measurement_info(name) {
                lines.push(measurement_line(&info));
            }
        }
        Ok(lines.join("\n"))
    }

    async fn list_parameters(&mut self, request: ListRequest) -> Result<String, String> {
        let request = Self::check_list_request(request)?;
        let session = Self::ready(&mut self.session)?;
        let names = session.find_parameters(&request.pattern);
        let page: Vec<&String> = names.iter().skip(request.offset).take(request.limit).collect();
        let mut lines = vec![list_header(page.len(), names.len(), "parameters", &request)];
        let mut lost = None;
        for name in page {
            match session.read_parameter(name).await {
                Ok(parameter) => lines.push(parameter_line(&parameter)),
                Err(e) => {
                    if !session.check_connection().await {
                        lost = Some(e.to_string());
                        break;
                    }
                    lines.push(format!("{}: {}", name, e));
                }
            }
        }
        if let Some(e) = lost {
            return Err(self.connection_lost(&e));
        }
        Ok(lines.join("\n"))
    }

    //------------------------------------------------------------------------
    // Read, write

    // Names which contain the last part of a name which was not found, as a hint
    fn similar_names(session: &Session, name: &str) -> String {
        let part = name.rsplit('.').next().unwrap_or(name).to_lowercase();
        if part.is_empty() {
            return String::new();
        }
        let mut names = session.find_parameters(".");
        names.extend(session.find_measurements("."));
        names.sort();
        names.dedup();
        let similar: Vec<String> = names.into_iter().filter(|n| n.to_lowercase().contains(&part)).take(5).collect();
        if similar.is_empty() {
            String::new()
        } else {
            format!(" Similar: {}", similar.join(", "))
        }
    }

    async fn read(&mut self, names: Vec<String>) -> Result<String, String> {
        if names.is_empty() {
            return Err("No names given".to_string());
        }
        if names.len() > READ_NAMES_MAX {
            return Err(format!("{} names given, at most {} can be read with one call", names.len(), READ_NAMES_MAX));
        }
        let session = Self::ready(&mut self.session)?;
        let mut lines = Vec::new();
        let mut ok = 0;
        let mut lost = None;
        for name in &names {
            // A signal of the running measurement: its last sample, without a command to the target
            let no_sample = match self.values.last_value(name) {
                LastValue::Value { values, unit, age } => {
                    ok += 1;
                    lines.push(format!("{} = {}  (measured, sample {} old)", name, format_values(&values, &unit), format_age(age)));
                    continue;
                }
                LastValue::NoSample { event_name } => Some(format!(
                    "no sample since the measurement start, its event{} was not triggered, and ",
                    event_name.map(|name| format!(" '{}'", name)).unwrap_or_default()
                )),
                LastValue::NotMeasured => None,
            };
            let result = match session.is_parameter(name) {
                None => {
                    lines.push(format!("{}: not found.{}", name, Self::similar_names(session, name)));
                    continue;
                }
                // A parameter: its value as it is written with xcp_write
                Some(true) => session
                    .read_parameter(name)
                    .await
                    .map(|p| format!("{} = {}  (parameter, limits {})", p.name, parameter_value(&p), parameter_limits(&p))),
                // A measurement variable: polled, the physical value with unit
                Some(false) => session
                    .read_value(name)
                    .await
                    .map(|(values, unit)| format!("{} = {}  (polled)", name, format_values(&values, &unit))),
            };
            match result {
                Ok(line) => {
                    ok += 1;
                    lines.push(line);
                }
                Err(e) => {
                    if !session.check_connection().await {
                        lost = Some(e.to_string());
                        break;
                    }
                    lines.push(format!("{}: {}can not be read by upload ({})", name, no_sample.unwrap_or_default(), e));
                }
            }
        }
        if let Some(e) = lost {
            return Err(self.connection_lost(&e));
        }
        let text = lines.join("\n");
        if ok > 0 { Ok(text) } else { Err(text) }
    }

    async fn write(&mut self, name: String, value: f64) -> Result<String, String> {
        let session = Self::ready(&mut self.session)?;
        match session.is_parameter(&name) {
            None => return Err(format!("{} not found, nothing written.{}", name, Self::similar_names(session, &name))),
            Some(false) => return Err(format!("{} is a measurement variable, not a calibration parameter. Only parameters can be written.", name)),
            Some(true) => {}
        }
        if !value.is_finite() {
            return Err(format!("{}: {} is not a valid value, nothing written", name, value));
        }
        match Self::write_parameter(session, &name, value).await {
            Ok(text) => Ok(text),
            Err(WriteError::Refused(text)) => Err(text),
            Err(WriteError::Failed(e)) => {
                if !session.check_connection().await {
                    return Err(self.connection_lost(&e));
                }
                Err(format!("{} not written: {}", name, e))
            }
        }
    }

    // Read the old value, check the limits, write and read back
    async fn write_parameter(session: &mut Session, name: &str, value: f64) -> Result<String, WriteError> {
        let failed = |e: Box<dyn std::error::Error>| WriteError::Failed(e.to_string());
        let old = session.read_parameter(name).await.map_err(failed)?;
        if value < old.limits.lower || value > old.limits.upper {
            return Err(WriteError::Refused(format!(
                "{}: value {} exceeds the limits {}, not written",
                old.name,
                format_number(value),
                parameter_limits(&old)
            )));
        }
        session.write_parameter(name, value).await.map_err(failed)?;
        let new = session.read_parameter(name).await.map_err(failed)?;
        Ok(format!(
            "{}: {} -> {}  ({}, limits {})",
            new.name,
            parameter_value(&old),
            parameter_value(&new),
            type_name(new.encoding, new.size),
            parameter_limits(&new)
        ))
    }
}

// Why a parameter was not written
enum WriteError {
    /// Refused before anything was sent to the target, with the message for the caller
    Refused(String),
    /// An XCP command failed
    Failed(String),
}

//------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xcp_client::{A2lAddr, A2lLimits, A2lType};

    fn defaults() -> SessionDefaults {
        SessionDefaults {
            dest_addr: "127.0.0.1".to_string(),
            port: 5555,
            protocol: None,
            bind_addr: "0.0.0.0:0".parse().unwrap(),
            connect_mode: 0,
            a2l: String::new(),
            elf: String::new(),
            default_event: None,
            file_dir: std::env::temp_dir().join("xcpclient-test"),
        }
    }

    // The directory for the files of the session is a directory of its own, not the working directory
    #[test]
    fn test_default_file_dir() {
        let dir = default_file_dir();
        assert!(dir.is_absolute(), "{}", dir.display());
        assert!(dir.components().any(|c| c.as_os_str() == "xcpclient"), "{}", dir.display());
        assert_ne!(dir, std::env::current_dir().unwrap());
    }

    // The A2L file of a connect without a file name goes to the directory of the session, a given name is used as it is
    #[test]
    fn test_description_options() {
        let actor = SessionActor::new(defaults());
        let file_dir = actor.defaults.file_dir.to_string_lossy().into_owned();
        let upload = actor.description_options(&LoadA2lRequest::default()).unwrap();
        assert!(upload.upload_a2l && upload.a2l.is_empty());
        assert_eq!(upload.a2l_dir, file_dir);
        let load = actor
            .description_options(&LoadA2lRequest {
                a2l: Some("my.a2l".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert!(!load.upload_a2l && !load.create_a2l);
        assert_eq!(load.a2l, "my.a2l");
        let create = actor
            .description_options(&LoadA2lRequest {
                elf: Some("app.elf".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert!(create.create_a2l && create.a2l.is_empty());
        assert_eq!((create.elf.as_str(), create.a2l_dir.as_str()), ("app.elf", file_dir.as_str()));
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(12)), "12 s");
        assert_eq!(format_duration(Duration::from_secs(4 * 60 + 10)), "4 min");
        assert_eq!(format_duration(Duration::from_secs(2 * 3600 + 5 * 60)), "2 h 5 min");
    }

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(65535.0), "65535");
        assert_eq!(format_number(-0.5), "-0.5");
        assert_eq!(format_number(0.0), "0");
        assert_eq!(format_number(f64::MAX), "1.7976931348623157e308");
        assert_eq!(format_number(1e-9), "1e-9");
    }

    #[test]
    fn test_parameter_line() {
        let p = ParameterInfo {
            name: "params.counter_max".to_string(),
            addr: A2lAddr {
                ext: 0,
                addr: 0x80010004,
                event: None,
            },
            encoding: A2lTypeEncoding::Unsigned,
            size: 2,
            limits: A2lLimits { lower: 0.0, upper: 65535.0 },
            value: Some(ParameterValue::Unsigned(1024)),
        };
        assert_eq!(parameter_line(&p), "params.counter_max  u16 = 1024  limits 0..65535");
    }

    #[test]
    fn test_measurement_line() {
        let mut m = XcpMeasurementInfo {
            name: "SigGen1.input".to_string(),
            a2l_addr: A2lAddr {
                ext: 2,
                addr: 0x10,
                event: Some(3),
            },
            a2l_type: A2lType {
                size: 8,
                encoding: A2lTypeEncoding::Float,
            },
            dim: [1, 1],
            unit: "Volt".to_string(),
            event_name: Some("SigGen1.clip".to_string()),
            characteristic: false,
        };
        assert_eq!(measurement_line(&m), "SigGen1.input  f64  unit Volt  event SigGen1.clip");
        m.dim = [8, 4];
        m.unit.clear();
        m.event_name = None;
        m.a2l_addr.event = None;
        assert_eq!(measurement_line(&m), "SigGen1.input  f64[8x4]  no event, needs default_event");
    }

    #[test]
    fn test_format_values() {
        assert_eq!(format_values(&[A2lValue::Float(17.5)], "Volt"), "17.5 Volt");
        assert_eq!(format_values(&[A2lValue::Label("RUNNING".to_string())], "0:IDLE,1:RUNNING"), "RUNNING");
        let array: Vec<A2lValue> = (0..32).map(A2lValue::Unsigned).collect();
        assert_eq!(format_values(&array[..3], ""), "[0 1 2]");
        assert_eq!(format_values(&array, ""), "[0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 ...] (16 of 32 elements)");
    }

    #[test]
    fn test_list_header() {
        let mut request = ListRequest {
            pattern: "counter".to_string(),
            limit: 50,
            offset: 0,
        };
        assert_eq!(list_header(3, 3, "parameters", &request), "3 of 3 parameters matching 'counter'");
        assert_eq!(
            list_header(50, 1234, "signals", &request),
            "50 of 1234 signals matching 'counter', next page: offset 50. Use a more specific pattern to get fewer"
        );
        request.offset = 1200;
        assert_eq!(list_header(34, 1234, "signals", &request), "34 of 1234 signals matching 'counter', from offset 1200");
    }

    #[test]
    fn test_serv_text_buffer() {
        let text = Arc::new(Mutex::new(ServText::default()));
        let decoder = ServTextBuffer(Arc::clone(&text));
        decoder.decode(b"hello ");
        decoder.decode(b"world\nsecond\x07 line\r\npartial");
        let lines: Vec<String> = text.lock().lines.iter().cloned().collect();
        assert_eq!(lines, vec!["hello world".to_string(), "second? line".to_string()]);
        for i in 0..100 {
            decoder.decode(format!("line {}\n", i).as_bytes());
        }
        assert_eq!(text.lock().lines.len(), SERV_TEXT_LINES);
        assert_eq!(text.lock().lines.back().unwrap(), "line 99");
    }

    // The actor answers without a connection: status, and errors which say what to do
    #[tokio::test]
    async fn test_actor_not_connected() {
        let session = SessionHandle::spawn(defaults()).unwrap();
        let status = session.status().await.unwrap();
        assert!(status.starts_with("State: disconnected"), "{}", status);
        assert!(status.contains("Default target of xcp_connect: 127.0.0.1 port 5555 (UDP)"), "{}", status);
        assert_eq!(session.read(vec!["x".to_string()]).await.unwrap_err(), NOT_CONNECTED);
        assert_eq!(session.write("x".to_string(), 1.0).await.unwrap_err(), NOT_CONNECTED);
        let request = ListRequest {
            pattern: ".".to_string(),
            limit: 10,
            offset: 0,
        };
        assert_eq!(session.list_parameters(request.clone()).await.unwrap_err(), NOT_CONNECTED);
        assert_eq!(session.list_measurements(request).await.unwrap_err(), NOT_CONNECTED);
        assert_eq!(session.disconnect().await.unwrap(), "Not connected, nothing to do.");
        session.shutdown().await;
        // The actor is gone
        assert!(session.status().await.is_err());
    }

    // An invalid regular expression is refused before the state is looked at
    #[tokio::test]
    async fn test_actor_invalid_pattern() {
        let session = SessionHandle::spawn(defaults()).unwrap();
        let request = ListRequest {
            pattern: "(".to_string(),
            limit: 10,
            offset: 0,
        };
        let e = session.list_parameters(request).await.unwrap_err();
        assert!(e.contains("not a valid regular expression"), "{}", e);
        session.shutdown().await;
    }

    // A target which refuses the connection is an error with a hint, the state stays disconnected
    #[tokio::test]
    async fn test_actor_connect_refused() {
        // A TCP port nobody listens on: bind a listener to get a free port, then close it
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let session = SessionHandle::spawn(defaults()).unwrap();
        let request = ConnectRequest {
            host: Some("127.0.0.1".to_string()),
            port: Some(port),
            protocol: Some(TransportProtocol::Tcp),
            ..Default::default()
        };
        let e = session.connect(request).await.unwrap_err();
        assert!(e.starts_with(&format!("Could not connect to 127.0.0.1:{} (TCP)", port)), "{}", e);
        assert!(session.status().await.unwrap().starts_with("State: disconnected"));
        let e = session
            .connect(ConnectRequest {
                host: Some("not an address".to_string()),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(e.contains("Invalid target address"), "{}", e);
        session.shutdown().await;
    }
}
