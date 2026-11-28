//-----------------------------------------------------------------------------
// Module log_capture
// Capture of warnings and errors of the log, to return them with the result of an operation
//
// xcpclient reports what it finds on the way as log messages: an A2L file which differs from the target, variables of an ELF
// file which can not be described, a section which is missing. On the command line the user reads them in the log. In MCP
// server mode the reader of a tool result (a language model) does not see the log, so the warnings and errors logged while an
// operation runs are captured and appended to its result.
//
// CaptureLogger wraps the logger of the application. While a capture is open (begin, end), it keeps the messages of level
// Warn and Error, independent of the log level of the wrapped logger. There is one capture for the process.
//
// An ELF file has hundreds of variables which can not be described, each with a warning of the ELF reader:
//   <unit>:<line>: '<name>': <reason> <details>
//   Local variable '<name>' in function '<function>' <reason>
//   Variable '<name>' <reason>: <details>
// Warnings of these forms are not kept one by one: they are counted by reason, with a few examples, and reported as one line
// for each reason.

use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

/// Maximum number of messages kept by a capture, more are counted
const CAPTURE_MAX: usize = 30;

/// Number of examples kept for the warnings about variables with the same reason
const EXAMPLES_MAX: usize = 3;

// Warnings about variables with the same reason
struct VariableWarnings {
    reason: String,
    count: usize,
    examples: Vec<String>,
}

struct Capture {
    messages: Vec<String>,
    dropped: usize,
    variables: Vec<VariableWarnings>,
}

static CAPTURE: Mutex<Option<Capture>> = Mutex::new(None);

/// Start to capture warnings and errors, a capture which is open is discarded
pub fn begin() {
    let capture = Capture {
        messages: Vec::new(),
        dropped: 0,
        variables: Vec::new(),
    };
    *CAPTURE.lock().unwrap_or_else(|e| e.into_inner()) = Some(capture);
}

/// End the capture
/// Returns the captured messages, "Warning: ..." and "Error: ...", in the order they were logged. The same message is kept
/// once. Warnings about variables follow as one line for each reason, with the number of variables and examples.
/// If there were more messages than the capture keeps, the last line says how many are missing
pub fn end() -> Vec<String> {
    let Some(mut capture) = CAPTURE.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return Vec::new();
    };
    capture.variables.sort_by(|a, b| b.count.cmp(&a.count));
    for variables in &capture.variables {
        capture.messages.push(format!(
            "Warning: {} {} not described, {}: {}{}",
            variables.count,
            if variables.count == 1 { "variable" } else { "variables" },
            variables.reason,
            variables.examples.join(", "),
            if variables.count > variables.examples.len() { ", ..." } else { "" }
        ));
    }
    if capture.dropped > 0 {
        capture.messages.push(format!("... and {} more warnings, see the log of xcpclient", capture.dropped));
    }
    capture.messages
}

// A warning about a variable
// Returns the variable and the reason without its details, which is the same for all variables with this problem
fn variable_warning(message: &str) -> Option<(String, String)> {
    located_variable_warning(message).or_else(|| named_variable_warning(message))
}

// "Local variable '<name>' in function '<function>' <reason>" and "Variable '<name>' <reason>: <details>"
// Returns the variable ("'<name>' in <function>") and the reason up to a colon
fn named_variable_warning(message: &str) -> Option<(String, String)> {
    let rest = message.strip_prefix("Local variable '").or_else(|| message.strip_prefix("Variable '"))?;
    let (name, rest) = rest.split_once("' ")?;
    let (variable, reason) = match rest.strip_prefix("in function ") {
        Some(rest) => {
            // The function name is quoted or printed as an Option
            let (function, reason) = rest.split_once(' ')?;
            let function = function.trim_start_matches("Some(").trim_end_matches(')').trim_matches(['\'', '"']);
            (format!("'{}' in {}", name, function), reason)
        }
        None => (format!("'{}'", name), rest),
    };
    let reason = reason.split(':').next().unwrap_or(reason).trim();
    if reason.is_empty() {
        return None;
    }
    Some((variable, reason.to_string()))
}

// "<unit>:<line>: '<name>': <reason> <details>"
// Returns the variable ("<unit>:<line> '<name>'") and the reason: the text up to the first bracket, colon or equal sign,
// without a value at its end, numbers replaced
fn located_variable_warning(message: &str) -> Option<(String, String)> {
    let (location, rest) = message.split_once(": '")?;
    let (name, reason) = rest.split_once("': ")?;
    if location.is_empty() || location.contains(' ') || name.contains(' ') {
        return None;
    }
    let reason = reason.split(['(', '[', '{', '=', ':']).next().unwrap_or(reason).trim();
    // A value at the end: "location is a register Register(2)" is cut to "location is a register Register"
    let reason = match reason.rsplit_once(' ') {
        Some((text, last)) if last.starts_with(|c: char| c.is_ascii_uppercase()) => text,
        _ => reason,
    };
    let mut generic = String::with_capacity(reason.len());
    for c in reason.chars() {
        if !c.is_ascii_digit() {
            generic.push(c);
        } else if !generic.ends_with('N') {
            generic.push('N');
        }
    }
    if generic.is_empty() {
        return None;
    }
    Some((format!("{} '{}'", location, name), generic))
}

// Keep a message if a capture is open
fn capture(level: Level, message: String) {
    let mut guard = CAPTURE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(capture) = guard.as_mut() else {
        return;
    };
    // A warning about a variable is counted with the others of its reason
    if level == Level::Warn
        && let Some((variable, reason)) = variable_warning(&message)
    {
        match capture.variables.iter_mut().find(|v| v.reason == reason) {
            Some(variables) => {
                variables.count += 1;
                if variables.examples.len() < EXAMPLES_MAX {
                    variables.examples.push(variable);
                }
            }
            None => capture.variables.push(VariableWarnings {
                reason,
                count: 1,
                examples: vec![variable],
            }),
        }
        return;
    }
    let message = format!("{}: {}", if level == Level::Error { "Error" } else { "Warning" }, message);
    if capture.messages.contains(&message) {
        return;
    }
    if capture.messages.len() < CAPTURE_MAX {
        capture.messages.push(message);
    } else {
        capture.dropped += 1;
    }
}

/// A logger which captures warnings and errors and passes everything on to the logger it wraps
pub struct CaptureLogger<L: Log> {
    inner: L,
}

impl<L: Log + 'static> CaptureLogger<L> {
    /// Install a logger wrapped into a CaptureLogger as the logger of the application
    /// max_level is the log level of the wrapped logger, warnings and errors reach the capture also if it is lower
    pub fn install(inner: L, max_level: LevelFilter) -> Result<(), log::SetLoggerError> {
        log::set_boxed_logger(Box::new(CaptureLogger { inner }))?;
        log::set_max_level(max_level.max(LevelFilter::Warn));
        Ok(())
    }
}

impl<L: Log> Log for CaptureLogger<L> {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Warn || self.inner.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        if record.level() <= Level::Warn {
            capture(record.level(), record.args().to_string());
        }
        if self.inner.enabled(record.metadata()) {
            self.inner.log(record);
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

//------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variable_warning() {
        let warning = |message: &str| variable_warning(message).map(|(variable, reason)| format!("{} | {}", variable, reason));
        assert_eq!(
            warning("main_c:333: 'p_counter_control': location is a register Register(2)").as_deref(),
            Some("main_c:333 'p_counter_control' | location is a register")
        );
        assert_eq!(
            warning("xcplite_c: 'len': location is a constant value Generic(0)").as_deref(),
            Some("xcplite_c 'len' | location is a constant value")
        );
        assert_eq!(
            warning("xcplite_c:2963: 'res': expression not evaluated, unsupported, eval_result=RequiresRegister { register: Register(0) }").as_deref(),
            Some("xcplite_c:2963 'res' | expression not evaluated, unsupported, eval_result")
        );
        assert_eq!(
            warning("xcplite_c:3405: 'uuid': only a slice of the variable has a location (DW_OP_piece), not measurable: Piece { size_in_bits: Some(64) }").as_deref(),
            Some("xcplite_c:3405 'uuid' | only a slice of the variable has a location")
        );
        // The warnings about variables without a source location
        assert_eq!(
            warning("Local variable 'len' in function 'safe_strnlen' not registered, it has no location in the debug information, not implemented yet").as_deref(),
            Some("'len' in safe_strnlen | not registered, it has no location in the debug information, not implemented yet")
        );
        assert_eq!(
            warning("Local variable 'ts' in function Some(\"clockInit\") skipped, could not find event for dyn addressing mode").as_deref(),
            Some("'ts' in clockInit | skipped, could not find event for dyn addressing mode")
        );
        assert_eq!(
            warning("Variable 'heap_struct' has unsupported type: Pointer(...)").as_deref(),
            Some("'heap_struct' | has unsupported type")
        );
        // Other messages with quotes and colons
        assert_eq!(warning("No xcp_meta section found, skipping metadata registration"), None);
        assert_eq!(
            warning("EPK mismatch: A2L file 'other.a2l' has EPK 'V9.9', target reports EPK 'V2.3'. Addresses may be wrong."),
            None
        );
        assert_eq!(warning("Event id 0 of event 'foo' is already used by event 'calc_power'"), None);
        assert_eq!(warning("New event 'foo' found, created with undefined event id 0xffff"), None);
    }

    // The capture is one for the process, one test covers it
    #[test]
    fn test_capture() {
        // Nothing is kept without a capture
        capture(Level::Warn, "before".to_string());
        assert!(end().is_empty());

        begin();
        capture(Level::Warn, "first".to_string());
        capture(Level::Error, "second".to_string());
        capture(Level::Warn, "first".to_string());
        assert_eq!(end(), ["Warning: first", "Error: second"]);
        assert!(end().is_empty());

        // Warnings about variables are counted by reason
        begin();
        capture(Level::Warn, "main_c:333: 'p_counter_control': location is a register Register(2)".to_string());
        capture(Level::Warn, "No xcp_meta section found".to_string());
        capture(Level::Warn, "main_c:346: 'p_params': location is a register Register(0)".to_string());
        capture(Level::Warn, "xcplite_c: 'len': location is a constant value Generic(0)".to_string());
        for i in 0..10 {
            capture(Level::Warn, format!("xcplite_c:{}: 'v{}': location is a register Register({})", i, i, i));
        }
        capture(
            Level::Warn,
            "main_c:244: 'test_struct2': the variable is split into 7 slices (DW_OP_piece), not measurable: [Piece { size_in_bits: Some(16) }]".to_string(),
        );
        // An error about a variable is kept as it is
        capture(Level::Error, "main_c:1: 'x': location is a register Register(1)".to_string());
        assert_eq!(
            end(),
            [
                "Warning: No xcp_meta section found",
                "Error: main_c:1: 'x': location is a register Register(1)",
                "Warning: 12 variables not described, location is a register: main_c:333 'p_counter_control', main_c:346 'p_params', xcplite_c:0 'v0', ...",
                "Warning: 1 variable not described, location is a constant value: xcplite_c 'len'",
                "Warning: 1 variable not described, the variable is split into N slices: main_c:244 'test_struct2'",
            ]
        );

        // More messages than the capture keeps
        begin();
        for i in 0..CAPTURE_MAX + 5 {
            capture(Level::Warn, format!("message {}", i));
        }
        let messages = end();
        assert_eq!(messages.len(), CAPTURE_MAX + 1);
        assert_eq!(messages.last().unwrap(), "... and 5 more warnings, see the log of xcpclient");
    }
}
