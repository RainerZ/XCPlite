//-----------------------------------------------------------------------------
// Module session::actor::measure
// The measurement operations of the session actor
//
//   sample          Measure signals for a time window of some seconds and return statistics. The reply is sent when the window
//                   is over, the actor serves other commands in between (PendingSample)
//   measure start   Start a measurement into files, it runs in the background (Recording) until measure stop or until its
//                   duration is over
//   measure stop    Stop it and return the statistics
//
// XCP has one DAQ setup, which can not be changed while it runs: there is one measurement at a time. A sample window uses the
// running recording, if there is one, and can then only cover its signals. Without a recording it starts a measurement of its
// own and stops it at the end of the window.
//
// Every measurement of the actor has a ValueWriter (module value_writer), which keeps the last samples and the statistics of a
// window. Measurement data never passes the actor, the receive task of the XCP client hands it to the writers.

use std::path::Path;
use std::time::{Duration, Instant};

#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};

use super::{ReplyTx, SessionActor, format_age, format_duration, format_significant, format_size, format_values};
use crate::session::{MeasurementOptions, MeasurementSummary, Session};
use crate::value_writer::WindowStatistics;
use crate::xcp_client::A2lValue;

/// Limits of a sample window
pub const SAMPLE_NAMES_MAX: usize = 20;
pub const SAMPLE_DURATION_DEFAULT_MS: u64 = 1000;
pub const SAMPLE_DURATION_MAX_MS: u64 = 30000;
const SAMPLE_DURATION_MIN_MS: u64 = 100;

/// While a sample window is open, the actor wakes up at least this often to notice a caller which has given up
const SAMPLE_POLL: Duration = Duration::from_millis(250);

//------------------------------------------------------------------------
// Requests

/// Measure signals for a time window and return statistics
#[derive(Debug, Clone)]
pub struct SampleRequest {
    /// Exact names of the signals
    pub names: Vec<String>,
    /// Length of the window, None: SAMPLE_DURATION_DEFAULT_MS
    pub duration_ms: Option<u64>,
}

/// Start a measurement into files
#[derive(Debug, Clone, Default)]
pub struct MeasureRequest {
    /// Exact names of the signals, or
    pub names: Vec<String>,
    /// a regular expression which selects them
    pub pattern: Option<String>,
    /// Stop after this time, 0: until measure stop
    pub duration_s: u64,
    /// Output files, None: no output in this format. All None: an MDF file with a name built from the target name and the time
    pub mdf: Option<String>,
    pub mcap: Option<String>,
    pub csv: Option<String>,
    pub perfetto: Option<String>,
    /// Trace signals added with a Perfetto trace file: off, context, links, all. None: links
    pub trace_auto: Option<String>,
}

//------------------------------------------------------------------------
// Recording: a measurement into files which runs in the background

pub(super) struct Recording {
    signals: usize,
    daq_lists: usize,
    files: Vec<String>,
    started: Instant,
    /// Stop after this time, None: until measure stop
    duration: Option<Duration>,
}

impl Recording {
    // The files with their sizes: "a.mf4 (88 KB), b.csv (1.2 MB)"
    fn files_text(&self) -> String {
        let files: Vec<String> = self
            .files
            .iter()
            .map(|file| match std::fs::metadata(file) {
                Ok(metadata) => format!("{} ({})", file, format_size(metadata.len())),
                Err(_) => file.clone(),
            })
            .collect();
        files.join(", ")
    }

    /// One line about the running measurement, for status
    pub(super) fn progress_text(&self, progress: Option<MeasurementSummary>, last_sample_age: Option<Duration>) -> String {
        let mut text = format!(
            "{} in {}, running {:.1} s",
            count_text(self.signals, "signal"),
            count_text(self.daq_lists, "DAQ list"),
            self.started.elapsed().as_secs_f64()
        );
        if let Some(duration) = self.duration {
            text.push_str(&format!(" of {} s", duration.as_secs()));
        }
        if let Some(progress) = progress {
            text.push_str(&format!(", {}", counters_text(&progress)));
        }
        match last_sample_age {
            Some(age) => text.push_str(&format!(", last sample {} ago", format_age(age))),
            None => text.push_str(", no sample received yet"),
        }
        format!("{}, files: {}", text, self.files_text())
    }

    /// The result of the measurement which is over, with the reason why it ended
    pub(super) fn summary_text(&self, summary: Option<MeasurementSummary>, reason: &str) -> String {
        let mut text = format!(
            "{} measured for {:.1} s ({})",
            count_text(self.signals, "signal"),
            summary.map(|s| s.elapsed_us as f64 / 1_000_000.0).unwrap_or_else(|| self.started.elapsed().as_secs_f64()),
            reason
        );
        if let Some(summary) = summary {
            text.push_str(&format!(": {}", counters_text(&summary)));
            if summary.dropped_count > 0 {
                text.push_str(&format!(", {} incomplete samples dropped", summary.dropped_count));
            }
        }
        format!("{}. Files: {}", text, self.files_text())
    }
}

// A number with its noun in singular or plural: "1 signal", "9 signals"
fn count_text(count: usize, noun: &str) -> String {
    if count == 1 { format!("1 {}", noun) } else { format!("{} {}s", count, noun) }
}

// Events, rates and lost packets of a measurement: "12400 events, 1000 event/s, 0.012 MByte/s, 0 packets lost"
fn counters_text(summary: &MeasurementSummary) -> String {
    let seconds = (summary.elapsed_us as f64 / 1_000_000.0).max(0.001);
    let mut text = format!(
        "{} events, {:.0} event/s, {:.3} MByte/s",
        summary.event_count,
        summary.event_count as f64 / seconds,
        summary.byte_count as f64 / seconds / 1_000_000.0
    );
    if summary.lost_count > 0 {
        let total = summary.packet_count + summary.lost_count;
        text.push_str(&format!(
            ", {} of {} packets lost ({:.3} %)",
            summary.lost_count,
            total,
            summary.lost_count as f64 * 100.0 / total as f64
        ));
    } else {
        text.push_str(", 0 packets lost");
    }
    text
}

//------------------------------------------------------------------------
// PendingSample: a sample window which is open

pub(super) struct SampleWindow {
    started: Instant,
    duration: Duration,
    /// The measurement was started for this window and is stopped at its end
    own_measurement: bool,
    /// Lost packets of the measurement at the start of the window
    lost_before: usize,
    /// Lines about the requested names which are not in the window
    skipped: Vec<String>,
}

pub(super) struct PendingSample {
    /// The reply of the sample command, sent when the window is over
    pub(super) reply: ReplyTx,
    pub(super) window: SampleWindow,
}

impl PendingSample {
    /// Time until the window is over
    pub(super) fn remaining(&self) -> Duration {
        self.window.duration.saturating_sub(self.window.started.elapsed())
    }
}

// One line of the sample result: count, rate, minimum, maximum, mean and last value
fn statistics_line(statistics: &WindowStatistics, seconds: f64) -> String {
    if statistics.count == 0 {
        let event = statistics.event_name.as_ref().map(|name| format!(" '{}'", name)).unwrap_or_default();
        return format!("{}: no sample in {:.1} s, its event{} was not triggered", statistics.name, seconds, event);
    }
    let rate = statistics.count as f64 / seconds.max(0.001);
    let rate = if rate >= 100.0 { format!("{:.0}", rate) } else { format!("{:.1}", rate) };
    let mut line = format!("{}  {} samples, {}/s", statistics.name, statistics.count, rate);
    if let Some((min, max, mean)) = statistics.min_max_mean {
        line.push_str(&format!(
            "  min {}  max {}  mean {}",
            format_significant(min),
            format_significant(max),
            format_significant(mean)
        ));
    }
    if let Some(last) = &statistics.last {
        // A single number with the digits of the statistics, anything else as it is read
        let last = match last.as_slice() {
            [A2lValue::Float(value)] => format!("{} {}", format_significant(*value), statistics.unit).trim_end().to_string(),
            _ => format_values(last, &statistics.unit),
        };
        line.push_str(&format!("  last {}", last));
    }
    line
}

//------------------------------------------------------------------------
// Signal selection

// The signals which can be measured out of the given names, and a line for each which can not, with the reason
fn select_signals(session: &Session, names: &[String]) -> (Vec<String>, Vec<String>) {
    let mut list: Vec<String> = Vec::new();
    let mut skipped = Vec::new();
    for name in names {
        if list.contains(name) {
            continue;
        }
        match session.measurement_info(name) {
            Some(info) if info.a2l_addr.event.is_some() => list.push(name.clone()),
            Some(_) => skipped.push(format!(
                "{}: has no event, it can not be measured. Give the event which samples it as default_event to xcp_connect, or read it with xcp_read.",
                name
            )),
            None => match session.is_parameter(name) {
                None => skipped.push(format!("{}: not found.{}", name, SessionActor::similar_names(session, name))),
                Some(true) => skipped.push(format!(
                    "{}: is a calibration parameter without event, it can not be measured. Read it with xcp_read.",
                    name
                )),
                Some(false) => skipped.push(format!("{}: can not be measured.", name)),
            },
        }
    }
    (list, skipped)
}

impl SessionActor {
    //------------------------------------------------------------------------
    // Timers

    /// The time of the next wakeup without a command: the end of the sample window or of a timed measurement
    pub(super) fn next_wakeup(&self) -> Option<tokio::time::Instant> {
        let mut wakeup: Option<Duration> = None;
        if let Some(sample) = &self.sample {
            wakeup = Some(sample.remaining().min(SAMPLE_POLL));
        }
        if let Some(Recording {
            started,
            duration: Some(duration),
            ..
        }) = &self.recording
        {
            let remaining = duration.saturating_sub(started.elapsed());
            wakeup = Some(wakeup.map_or(remaining, |w| w.min(remaining)));
        }
        wakeup.map(|w| tokio::time::Instant::now() + w)
    }

    /// A wakeup time is reached
    pub(super) async fn on_wakeup(&mut self) {
        if let Some(sample) = &self.sample {
            if sample.reply.is_closed() {
                // The caller has given up (cancelled tool call)
                self.sample_finish(Some("the call was cancelled")).await;
            } else if sample.remaining().is_zero() {
                self.sample_finish(None).await;
            }
        }
        // The duration of the measurement is over, its summary is kept for status
        if self.sample.is_none()
            && let Some(Recording {
                started,
                duration: Some(duration),
                ..
            }) = &self.recording
            && started.elapsed() >= *duration
        {
            let _ = self.recording_finish("the requested duration is over").await;
        }
    }

    //------------------------------------------------------------------------
    // Sample

    /// Open a sample window
    /// Uses the running recording, or starts a measurement of the signals
    pub(super) async fn sample_start(&mut self, request: SampleRequest) -> Result<SampleWindow, String> {
        if request.names.is_empty() {
            return Err("No names given".to_string());
        }
        if request.names.len() > SAMPLE_NAMES_MAX {
            return Err(format!("{} names given, at most {} can be sampled with one call", request.names.len(), SAMPLE_NAMES_MAX));
        }
        if let Some(sample) = &self.sample {
            return Err(format!(
                "xcp_sample is already running for another {:.1} s, call it again when it is done.",
                sample.remaining().as_secs_f64()
            ));
        }
        let duration = Duration::from_millis(
            request
                .duration_ms
                .unwrap_or(SAMPLE_DURATION_DEFAULT_MS)
                .clamp(SAMPLE_DURATION_MIN_MS, SAMPLE_DURATION_MAX_MS),
        );
        let session = Self::ready(&mut self.session)?;

        let (own_measurement, skipped) = if self.recording.is_some() {
            // The DAQ setup of the running measurement can not be changed, only its signals can be sampled
            let not_measured = self.values.begin_window(&request.names);
            let skipped: Vec<String> = not_measured
                .iter()
                .map(|name| {
                    format!(
                        "{}: not part of the running measurement. Stop it with xcp_measure_stop or sample it after the measurement.",
                        name
                    )
                })
                .collect();
            if not_measured.len() == request.names.len() {
                self.values.end_window();
                return Err(skipped.join("\n"));
            }
            (false, skipped)
        } else {
            let (list, skipped) = select_signals(session, &request.names);
            if list.is_empty() {
                return Err(skipped.join("\n"));
            }
            let writer = Box::new(self.values.writer());
            match session.start_measurement_of(list.clone(), &MeasurementOptions::default(), vec![writer]).await {
                Ok(true) => {}
                Ok(false) => return Err(format!("None of the signals can be measured.\n{}", skipped.join("\n")).trim_end().to_string()),
                Err(e) => {
                    let e = e.to_string();
                    if !session.check_connection().await {
                        return Err(self.connection_lost(&e));
                    }
                    return Err(format!("The measurement could not be started: {}", e));
                }
            }
            self.values.begin_window(&list);
            (true, skipped)
        };
        let lost_before = self.session.as_ref().and_then(|session| session.measurement_progress()).map(|p| p.lost_count).unwrap_or(0);
        Ok(SampleWindow {
            started: Instant::now(),
            duration,
            own_measurement,
            lost_before,
            skipped,
        })
    }

    /// Close the sample window and send its reply: the statistics, or an error with the reason if it is cancelled
    pub(super) async fn sample_finish(&mut self, cancelled: Option<&str>) {
        let Some(PendingSample { reply, window }) = self.sample.take() else {
            return;
        };
        let (duration, statistics) = self.values.end_window();
        let lost = self
            .session
            .as_ref()
            .and_then(|session| session.measurement_progress())
            .map(|p| p.lost_count.saturating_sub(window.lost_before))
            .unwrap_or(0);

        // Stop the measurement which was started for this window
        if window.own_measurement
            && let Some(session) = self.session.as_mut()
            && let Err(e) = session.stop_measurement().await
        {
            let e = e.to_string();
            if !session.check_connection().await {
                let _ = reply.send(Err(self.connection_lost(&e)));
                return;
            }
            warn!("Stop of the sample measurement failed: {}", e);
        }
        if let Some(reason) = cancelled {
            let _ = reply.send(Err(format!("xcp_sample was cancelled: {}", reason)));
            return;
        }

        let seconds = duration.as_secs_f64();
        let sampled = statistics.iter().filter(|s| s.count > 0).count();
        let mut lines = vec![format!("Sampled {} for {:.1} s, {} packets lost", count_text(statistics.len(), "signal"), seconds, lost)];
        lines.extend(statistics.iter().map(|s| statistics_line(s, seconds)));
        lines.extend(window.skipped);
        let text = lines.join("\n");
        let _ = reply.send(if sampled > 0 { Ok(text) } else { Err(text) });
    }

    //------------------------------------------------------------------------
    // Measure start, stop

    pub(super) async fn measure_start(&mut self, request: MeasureRequest) -> Result<String, String> {
        if let Some(recording) = &self.recording {
            return Err(format!(
                "A measurement is already running ({} signals, {} s). Stop it with xcp_measure_stop first.",
                recording.signals,
                recording.started.elapsed().as_secs()
            ));
        }
        let session = Self::ready(&mut self.session)?;

        // The signals: exact names or a regular expression
        let (list, skipped) = match (&request.pattern, request.names.is_empty()) {
            (Some(_), false) => return Err("Give either names or pattern, not both".to_string()),
            (None, true) => return Err("No signals given: give their names, or a pattern (regular expression, \".\" for all)".to_string()),
            (None, false) => select_signals(session, &request.names),
            (Some(pattern), true) => {
                regex::Regex::new(pattern).map_err(|e| format!("'{}' is not a valid regular expression: {}", pattern, e))?;
                let names = session.find_measurements(pattern);
                let (list, skipped) = select_signals(session, &names);
                // Not one line for each signal without event
                let skipped = if skipped.is_empty() {
                    Vec::new()
                } else {
                    vec![format!("{} signals matching '{}' have no event and are not measured.", skipped.len(), pattern)]
                };
                if list.is_empty() && skipped.is_empty() {
                    return Err(format!("No signals match '{}'", pattern));
                }
                (list, skipped)
            }
        };
        if list.is_empty() {
            return Err(format!("Nothing to measure.\n{}", skipped.join("\n")));
        }

        // The output files: as given, or an MDF file with a name built from the target name and the time
        let mut options = MeasurementOptions {
            csv: request.csv.unwrap_or_default(),
            mdf: request.mdf.unwrap_or_default(),
            mcap: request.mcap.unwrap_or_default(),
            perfetto: request.perfetto.unwrap_or_default(),
            trace_auto: request.trace_auto.unwrap_or_else(|| "links".to_string()),
            console: false,
        };
        if !["off", "context", "links", "all"].contains(&options.trace_auto.as_str()) {
            return Err(format!("trace_auto '{}' is not one of off, context, links, all", options.trace_auto));
        }
        if options.csv.is_empty() && options.mdf.is_empty() && options.mcap.is_empty() && options.perfetto.is_empty() {
            let target = session.ecu_name();
            options.mdf = format!(
                "{}_{}.mf4",
                if target.is_empty() { "measurement" } else { target },
                chrono::Local::now().format("%Y%m%d_%H%M%S")
            );
        }
        let files: Vec<String> = [&options.mdf, &options.mcap, &options.csv, &options.perfetto]
            .into_iter()
            .filter(|file| !file.is_empty())
            .cloned()
            .collect();
        // A measurement never overwrites a file
        if let Some(file) = files.iter().find(|file| Path::new(file).exists()) {
            return Err(format!("The file '{}' exists, a measurement does not overwrite files. Give another file name.", file));
        }

        let writer = Box::new(self.values.writer());
        match session.start_measurement_of(list, &options, vec![writer]).await {
            Ok(true) => {}
            Ok(false) => return Err("None of the signals can be measured.".to_string()),
            Err(e) => {
                let e = e.to_string();
                if !session.check_connection().await {
                    return Err(self.connection_lost(&e));
                }
                return Err(format!("The measurement could not be started: {}", e));
            }
        }

        let (signals, daq_lists) = self.values.counts();
        let duration = (request.duration_s > 0).then(|| Duration::from_secs(request.duration_s));
        let mut lines = vec![format!(
            "Measurement started: {} in {}, {}.",
            count_text(signals, "signal"),
            count_text(daq_lists, "DAQ list"),
            match duration {
                Some(duration) => format!("for {}, it stops by itself", format_duration(duration)),
                None => "until xcp_measure_stop".to_string(),
            }
        )];
        lines.push(format!("Files: {}", files.join(", ")));
        lines.extend(skipped);
        self.last_measurement = None;
        self.recording = Some(Recording {
            signals,
            daq_lists,
            files,
            started: Instant::now(),
            duration,
        });
        Ok(lines.join("\n"))
    }

    pub(super) async fn measure_stop(&mut self) -> Result<String, String> {
        if self.recording.is_none() {
            return Err(match &self.last_measurement {
                Some(last) => format!("No measurement is running. The last one: {}", last),
                None => "No measurement is running.".to_string(),
            });
        }
        self.recording_finish("stopped by xcp_measure_stop").await
    }

    /// Stop the recording and complete its files
    /// Returns its summary, which is also kept for status. An error if the connection is lost, the files are completed all the same
    pub(super) async fn recording_finish(&mut self, reason: &str) -> Result<String, String> {
        let Some(recording) = self.recording.take() else {
            return Err("No measurement is running.".to_string());
        };
        let Some(session) = self.session.as_mut() else {
            return Err(super::NOT_CONNECTED.to_string());
        };
        let progress = session.measurement_progress();
        match session.stop_measurement().await {
            Ok(summary) => {
                let text = recording.summary_text(summary.or(progress), reason);
                info!("Measurement: {}", text);
                self.last_measurement = Some(text.clone());
                Ok(text)
            }
            Err(e) => {
                // The files are completed, the stop command failed
                let e = e.to_string();
                let text = recording.summary_text(progress, reason);
                self.last_measurement = Some(text.clone());
                if !session.check_connection().await {
                    return Err(format!("{}\n{}", self.connection_lost(&e), text));
                }
                Ok(format!("{}\nThe stop command failed: {}", text, e))
            }
        }
    }
}

//------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(lost_count: usize) -> MeasurementSummary {
        MeasurementSummary {
            elapsed_us: 12_400_000,
            event_count: 12400,
            byte_count: 148_800,
            packet_count: 12400 - lost_count,
            lost_count,
            dropped_count: 0,
        }
    }

    #[test]
    fn test_counters_text() {
        assert_eq!(counters_text(&summary(0)), "12400 events, 1000 event/s, 0.012 MByte/s, 0 packets lost");
        assert_eq!(
            counters_text(&summary(124)),
            "12400 events, 1000 event/s, 0.012 MByte/s, 124 of 12400 packets lost (1.000 %)"
        );
    }

    #[test]
    fn test_recording_text() {
        let recording = Recording {
            signals: 4,
            daq_lists: 2,
            files: vec!["no_such_file.mf4".to_string()],
            started: Instant::now(),
            duration: Some(Duration::from_secs(60)),
        };
        assert_eq!(
            recording.summary_text(Some(summary(0)), "stopped by xcp_measure_stop"),
            "4 signals measured for 12.4 s (stopped by xcp_measure_stop): 12400 events, 1000 event/s, 0.012 MByte/s, 0 packets lost. Files: no_such_file.mf4"
        );
        let progress = recording.progress_text(Some(summary(0)), Some(Duration::from_millis(3)));
        assert!(progress.starts_with("4 signals in 2 DAQ lists, running 0.0 s of 60 s, 12400 events"), "{}", progress);
        assert!(progress.ends_with(", last sample 3 ms ago, files: no_such_file.mf4"), "{}", progress);
        assert!(recording.progress_text(None, None).contains("no sample received yet"));
    }

    #[test]
    fn test_statistics_line() {
        let mut statistics = WindowStatistics {
            name: "SigGen1.input".to_string(),
            unit: "Volt".to_string(),
            event_name: Some("SigGen1.clip".to_string()),
            count: 120000,
            min_max_mean: Some((-17.514199949, 17.514199949, 0.0021)),
            first: Some(vec![A2lValue::Float(1.0)]),
            last: Some(vec![A2lValue::Float(3.07)]),
        };
        assert_eq!(
            statistics_line(&statistics, 3.0),
            "SigGen1.input  120000 samples, 40000/s  min -17.5142  max 17.5142  mean 0.0021  last 3.07 Volt"
        );
        // A slow signal, and an enum without numbers
        statistics.count = 5;
        statistics.min_max_mean = None;
        statistics.last = Some(vec![A2lValue::Label("RUNNING".to_string())]);
        assert_eq!(statistics_line(&statistics, 3.0), "SigGen1.input  5 samples, 1.7/s  last RUNNING");
        // No sample
        statistics.count = 0;
        assert_eq!(
            statistics_line(&statistics, 3.0),
            "SigGen1.input: no sample in 3.0 s, its event 'SigGen1.clip' was not triggered"
        );
    }
}
