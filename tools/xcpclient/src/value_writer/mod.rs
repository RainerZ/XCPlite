//--------------------------------------------------------------------------------------------------------------------------------------------------
// Module value_writer — The current values of a running measurement
//
// A DaqWriter which writes no file. It keeps the last sample of each DAQ list and, for a time window, statistics of selected
// signals. It answers "what is the value now" and "how did the signal behave in the last seconds" for a measurement which runs
// in the background (MCP server mode: xcp_read, xcp_sample, see docs/MCP.md).
//
// The values are read through a ValueStore, a handle which is shared with the writer. The writer is called in the receive task
// of the XCP client for every DAQ packet, so it does as little as possible there:
//   - Last sample: the payload of the DAQ packet is copied, it is decoded only when a value is asked for
//   - Window statistics: the first element of each selected signal is decoded for minimum, maximum and mean, the payload of
//     the first and the last sample of the window is copied. Signals which are not selected cost nothing

use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::daq_writer::{DaqLayout, DaqSample, DaqWriter};
use crate::xcp_client::{A2lConversion, A2lType, A2lValue};

//--------------------------------------------------------------------------------------------------------------------------------------------------
// Signals and samples

// A signal of the measurement: where it is in the payload of its DAQ list and how it is decoded
struct Signal {
    name: String,
    daq: usize,
    offset: usize,
    a2l_type: A2lType, // type of one element
    element_count: usize,
    conversion: A2lConversion,
    event_name: Option<String>,
}

impl Signal {
    fn size(&self) -> usize {
        self.a2l_type.size * self.element_count
    }

    // Physical value of the first element
    fn decode_first(&self, data: &[u8]) -> Option<A2lValue> {
        self.a2l_type.decode(data.get(self.offset..)?).map(|raw| self.conversion.convert(raw))
    }

    // Physical values of all elements
    fn decode(&self, data: &[u8]) -> Option<Vec<A2lValue>> {
        let data = data.get(self.offset..self.offset + self.size())?;
        data.chunks_exact(self.a2l_type.size.max(1))
            .map(|bytes| self.a2l_type.decode(bytes).map(|raw| self.conversion.convert(raw)))
            .collect()
    }
}

// The last sample of a DAQ list
struct LastSample {
    data: Vec<u8>,
    received: Instant,
}

// Statistics of a signal in a window
#[derive(Default)]
struct Statistics {
    count: u64,
    // Of the first element, if it is a number
    numeric_count: u64,
    min: f64,
    max: f64,
    sum: f64,
    // Payload of the first and the last sample of the DAQ list in the window
    first: Vec<u8>,
    last: Vec<u8>,
}

struct Window {
    started: Instant,
    // Signal index and statistics, for each DAQ list
    daq_statistics: Vec<Vec<(usize, Statistics)>>,
}

#[derive(Default)]
struct Store {
    running: bool,
    signals: Vec<Signal>,
    by_name: HashMap<String, usize>,
    last: Vec<Option<LastSample>>, // for each DAQ list
    window: Option<Window>,
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
// ValueStore

/// The last value of a signal of the running measurement
#[derive(Debug, Clone, PartialEq)]
pub enum LastValue {
    /// The signal is not part of the running measurement, or no measurement is running
    NotMeasured,
    /// No sample of the signal was received since the measurement start, with the name of its event
    NoSample { event_name: Option<String> },
    /// The physical values of all elements with the unit, and the time since the sample was received
    Value { values: Vec<A2lValue>, unit: String, age: Duration },
}

/// Statistics of a signal in a time window
#[derive(Debug, Clone, PartialEq)]
pub struct WindowStatistics {
    pub name: String,
    pub unit: String,
    pub event_name: Option<String>,
    /// Number of samples in the window
    pub count: u64,
    /// Minimum, maximum and mean of the first element, None if there was no sample or the signal is not a number (enum)
    pub min_max_mean: Option<(f64, f64, f64)>,
    /// Physical values of all elements of the first and of the last sample, None if there was no sample
    pub first: Option<Vec<A2lValue>>,
    pub last: Option<Vec<A2lValue>>,
}

/// Handle to the values of the running measurement, shared with the ValueWriter of the measurement
#[derive(Clone, Default)]
pub struct ValueStore(Arc<Mutex<Store>>);

impl ValueStore {
    pub fn new() -> ValueStore {
        ValueStore::default()
    }

    /// A writer for the next measurement, it fills this store
    pub fn writer(&self) -> ValueWriter {
        ValueWriter(self.clone())
    }

    /// A measurement with this store's writer is running
    pub fn is_running(&self) -> bool {
        self.0.lock().running
    }

    /// Number of signals and DAQ lists of the running measurement
    pub fn counts(&self) -> (usize, usize) {
        let store = self.0.lock();
        if store.running { (store.signals.len(), store.last.len()) } else { (0, 0) }
    }

    /// The signal is part of the running measurement
    pub fn contains(&self, name: &str) -> bool {
        let store = self.0.lock();
        store.running && store.by_name.contains_key(name)
    }

    /// The last value of a signal
    pub fn last_value(&self, name: &str) -> LastValue {
        let store = self.0.lock();
        let Some(signal) = store.by_name.get(name).filter(|_| store.running).map(|&i| &store.signals[i]) else {
            return LastValue::NotMeasured;
        };
        match store.last[signal.daq]
            .as_ref()
            .and_then(|sample| signal.decode(&sample.data).map(|values| (values, sample.received)))
        {
            Some((values, received)) => LastValue::Value {
                values,
                unit: signal.conversion.unit().to_string(),
                age: received.elapsed(),
            },
            None => LastValue::NoSample {
                event_name: signal.event_name.clone(),
            },
        }
    }

    /// Time since the newest sample of any DAQ list was received, None if there was none since the measurement start
    pub fn last_sample_age(&self) -> Option<Duration> {
        let store = self.0.lock();
        store.last.iter().flatten().map(|sample| sample.received.elapsed()).min()
    }

    /// Start a statistics window for the given signals, a window which is open is replaced
    /// Returns the names which are not part of the running measurement
    pub fn begin_window(&self, names: &[String]) -> Vec<String> {
        let mut store = self.0.lock();
        let mut daq_statistics: Vec<Vec<(usize, Statistics)>> = (0..store.last.len()).map(|_| Vec::new()).collect();
        let mut not_measured = Vec::new();
        for name in names {
            match store.by_name.get(name).filter(|_| store.running) {
                Some(&index) => {
                    let daq = store.signals[index].daq;
                    if !daq_statistics[daq].iter().any(|(i, _)| *i == index) {
                        daq_statistics[daq].push((index, Statistics::default()));
                    }
                }
                None => not_measured.push(name.clone()),
            }
        }
        store.window = Some(Window {
            started: Instant::now(),
            daq_statistics,
        });
        not_measured
    }

    /// End the statistics window
    /// Returns the duration of the window and the statistics of its signals, in the order of the measurement
    pub fn end_window(&self) -> (Duration, Vec<WindowStatistics>) {
        let mut store = self.0.lock();
        let Some(window) = store.window.take() else {
            return (Duration::ZERO, Vec::new());
        };
        let mut result: Vec<(usize, WindowStatistics)> = Vec::new();
        for (index, statistics) in window.daq_statistics.into_iter().flatten() {
            let signal = &store.signals[index];
            let has_samples = statistics.count > 0;
            result.push((
                index,
                WindowStatistics {
                    name: signal.name.clone(),
                    unit: signal.conversion.unit().to_string(),
                    event_name: signal.event_name.clone(),
                    count: statistics.count,
                    min_max_mean: (statistics.numeric_count > 0).then(|| (statistics.min, statistics.max, statistics.sum / statistics.numeric_count as f64)),
                    first: has_samples.then(|| signal.decode(&statistics.first)).flatten(),
                    last: has_samples.then(|| signal.decode(&statistics.last)).flatten(),
                },
            ));
        }
        result.sort_by_key(|(index, _)| *index);
        (window.started.elapsed(), result.into_iter().map(|(_, statistics)| statistics).collect())
    }
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
// ValueWriter

/// DaqWriter which fills a ValueStore
pub struct ValueWriter(ValueStore);

impl DaqWriter for ValueWriter {
    fn name(&self) -> &'static str {
        "Value"
    }

    fn start(&mut self, layout: &DaqLayout, _start_ns: u64) -> io::Result<()> {
        let mut store = self.0.0.lock();
        store.signals.clear();
        store.by_name.clear();
        for (daq, daq_list) in layout.daq_lists.iter().enumerate() {
            for entry in &daq_list.entries {
                let index = store.signals.len();
                store.by_name.insert(entry.name.clone(), index);
                store.signals.push(Signal {
                    name: entry.name.clone(),
                    daq,
                    offset: entry.offset as usize,
                    a2l_type: entry.a2l_type,
                    element_count: entry.element_count(),
                    conversion: entry.conversion.clone(),
                    event_name: daq_list.event_name.clone(),
                });
            }
        }
        store.last = (0..layout.daq_lists.len()).map(|_| None).collect();
        store.window = None;
        store.running = true;
        Ok(())
    }

    fn write(&mut self, sample: &DaqSample) -> io::Result<()> {
        let mut store = self.0.0.lock();
        let store = &mut *store;
        let daq = sample.daq as usize;
        let Some(last) = store.last.get_mut(daq) else {
            return Ok(());
        };

        // Keep the payload of the last sample, the buffer is reused
        match last {
            Some(last) => {
                last.data.clear();
                last.data.extend_from_slice(sample.data);
                last.received = Instant::now();
            }
            None => {
                *last = Some(LastSample {
                    data: sample.data.to_vec(),
                    received: Instant::now(),
                })
            }
        }

        // Statistics of the signals of this DAQ list which are in the window
        if let Some(window) = &mut store.window {
            for (index, statistics) in &mut window.daq_statistics[daq] {
                if statistics.count == 0 {
                    statistics.first.extend_from_slice(sample.data);
                }
                statistics.count += 1;
                statistics.last.clear();
                statistics.last.extend_from_slice(sample.data);
                if let Some(value) = store.signals[*index].decode_first(sample.data).and_then(|value| value.as_f64()) {
                    if statistics.numeric_count == 0 {
                        statistics.min = value;
                        statistics.max = value;
                    } else {
                        statistics.min = statistics.min.min(value);
                        statistics.max = statistics.max.max(value);
                    }
                    statistics.sum += value;
                    statistics.numeric_count += 1;
                }
            }
        }
        Ok(())
    }

    fn stop(&mut self, _stop_ns: u64) -> io::Result<()> {
        // The values of a measurement which is over are not served any more
        let mut store = self.0.0.lock();
        store.running = false;
        store.window = None;
        Ok(())
    }
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xcp_client::{A2lAddr, A2lTypeEncoding, OdtEntry, XcpDaqList};

    fn entry(name: &str, size: usize, encoding: A2lTypeEncoding, dim: [u16; 2], offset: u16, conversion: A2lConversion) -> OdtEntry {
        OdtEntry {
            name: name.to_string(),
            a2l_type: A2lType { size, encoding },
            dim,
            a2l_addr: A2lAddr { ext: 0, addr: 0, event: Some(0) },
            conversion,
            offset,
        }
    }

    // DAQ list 0 (event task): counter u16, voltage u16 with factor 0.5 and unit, array u8[4]. DAQ list 1 (event slow): state enum
    fn layout() -> DaqLayout {
        let none = || A2lConversion::new(None, None, "");
        let task = XcpDaqList {
            event_id: 0,
            event_name: Some("task".to_string()),
            entries: vec![
                entry("counter", 2, A2lTypeEncoding::Unsigned, [1, 1], 0, none()),
                entry("voltage", 2, A2lTypeEncoding::Unsigned, [1, 1], 2, A2lConversion::new(Some(0.5), Some(0.0), "Volt")),
                entry("array", 1, A2lTypeEncoding::Unsigned, [4, 1], 4, none()),
            ],
            odt_sizes: Vec::new(),
            size: 8,
        };
        let slow = XcpDaqList {
            event_id: 1,
            event_name: Some("slow".to_string()),
            entries: vec![entry("state", 1, A2lTypeEncoding::Unsigned, [1, 1], 0, none())],
            odt_sizes: Vec::new(),
            size: 1,
        };
        DaqLayout::new(4, 1, vec![task, slow])
    }

    fn write(writer: &mut ValueWriter, layout: &DaqLayout, daq: u16, data: &[u8]) {
        let sample = DaqSample {
            daq,
            daq_list: &layout.daq_lists[daq as usize],
            lost: 0,
            timestamp: 0,
            time_ns: 0,
            packet: data,
            data,
        };
        writer.write(&sample).unwrap();
    }

    fn task_sample(counter: u16, voltage: u16, array: [u8; 4]) -> Vec<u8> {
        let mut data = counter.to_le_bytes().to_vec();
        data.extend_from_slice(&voltage.to_le_bytes());
        data.extend_from_slice(&array);
        data
    }

    #[test]
    fn test_last_value() {
        let store = ValueStore::new();
        let layout = layout();
        let mut writer = store.writer();
        // No measurement
        assert_eq!(store.last_value("counter"), LastValue::NotMeasured);
        assert!(!store.is_running());

        writer.start(&layout, 0).unwrap();
        assert!(store.is_running());
        assert_eq!(store.counts(), (4, 2));
        assert!(store.contains("voltage") && !store.contains("other"));
        // No sample yet
        assert_eq!(
            store.last_value("counter"),
            LastValue::NoSample {
                event_name: Some("task".to_string())
            }
        );
        assert_eq!(store.last_value("other"), LastValue::NotMeasured);
        assert!(store.last_sample_age().is_none());

        write(&mut writer, &layout, 0, &task_sample(7, 20, [1, 2, 3, 4]));
        write(&mut writer, &layout, 0, &task_sample(8, 24, [5, 6, 7, 8]));
        let LastValue::Value { values, unit, age } = store.last_value("counter") else {
            panic!("no value")
        };
        assert_eq!((values, unit.as_str()), (vec![A2lValue::Unsigned(8)], ""));
        assert!(age < Duration::from_secs(1));
        // Physical value with unit
        let LastValue::Value { values, unit, .. } = store.last_value("voltage") else {
            panic!("no value")
        };
        assert_eq!((values, unit.as_str()), (vec![A2lValue::Float(12.0)], "Volt"));
        // All elements of an array
        let LastValue::Value { values, .. } = store.last_value("array") else {
            panic!("no value")
        };
        assert_eq!(values, [5u64, 6, 7, 8].map(A2lValue::Unsigned).to_vec());
        // The other DAQ list has no sample yet
        assert_eq!(
            store.last_value("state"),
            LastValue::NoSample {
                event_name: Some("slow".to_string())
            }
        );
        assert!(store.last_sample_age().is_some());

        // After the stop the values are not served any more
        writer.stop(0).unwrap();
        assert_eq!(store.last_value("counter"), LastValue::NotMeasured);
        assert_eq!(store.counts(), (0, 0));
    }

    #[test]
    fn test_window_statistics() {
        let store = ValueStore::new();
        let layout = layout();
        let mut writer = store.writer();
        writer.start(&layout, 0).unwrap();

        // A sample before the window does not count
        write(&mut writer, &layout, 0, &task_sample(100, 200, [0; 4]));
        let not_measured = store.begin_window(&["voltage".to_string(), "counter".to_string(), "state".to_string(), "other".to_string()]);
        assert_eq!(not_measured, ["other"]);
        for (counter, voltage) in [(1u16, 10u16), (2, 30), (3, 20)] {
            write(&mut writer, &layout, 0, &task_sample(counter, voltage, [counter as u8; 4]));
        }
        let (duration, statistics) = store.end_window();
        assert!(duration < Duration::from_secs(1));
        // In the order of the measurement, not of the request
        assert_eq!(statistics.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["counter", "voltage", "state"]);

        let counter = &statistics[0];
        assert_eq!(counter.count, 3);
        assert_eq!(counter.min_max_mean, Some((1.0, 3.0, 2.0)));
        assert_eq!(counter.first, Some(vec![A2lValue::Unsigned(1)]));
        assert_eq!(counter.last, Some(vec![A2lValue::Unsigned(3)]));

        // Physical values
        let voltage = &statistics[1];
        assert_eq!(voltage.unit, "Volt");
        assert_eq!(voltage.min_max_mean, Some((5.0, 15.0, 10.0)));
        assert_eq!(voltage.last, Some(vec![A2lValue::Float(10.0)]));

        // A signal without a sample in the window, with the event to name in the message
        let state = &statistics[2];
        assert_eq!((state.count, state.min_max_mean, &state.first, &state.last), (0, None, &None, &None));
        assert_eq!(state.event_name.as_deref(), Some("slow"));

        // The window is closed
        assert_eq!(store.end_window().1.len(), 0);
        // Samples after the window do not change anything, the last value is still served
        write(&mut writer, &layout, 0, &task_sample(9, 9, [0; 4]));
        assert!(matches!(store.last_value("counter"), LastValue::Value { .. }));
    }

    // A new measurement with other signals replaces the signals of the measurement before
    #[test]
    fn test_restart() {
        let store = ValueStore::new();
        let layout1 = layout();
        let mut writer = store.writer();
        writer.start(&layout1, 0).unwrap();
        write(&mut writer, &layout1, 0, &task_sample(1, 2, [0; 4]));
        writer.stop(0).unwrap();

        let layout2 = DaqLayout::new(
            4,
            1,
            vec![XcpDaqList {
                event_id: 0,
                event_name: None,
                entries: vec![entry("other", 1, A2lTypeEncoding::Unsigned, [1, 1], 0, A2lConversion::new(None, None, ""))],
                odt_sizes: Vec::new(),
                size: 1,
            }],
        );
        let mut writer = store.writer();
        writer.start(&layout2, 0).unwrap();
        assert_eq!(store.counts(), (1, 1));
        assert_eq!(store.last_value("counter"), LastValue::NotMeasured);
        assert_eq!(store.last_value("other"), LastValue::NoSample { event_name: None });
        // A sample of a DAQ list which does not exist is ignored
        write(&mut writer, &layout1, 1, &[1]);
    }
}
