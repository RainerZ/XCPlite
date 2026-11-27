//--------------------------------------------------------------------------------------------------------------------------------------------------
// Module console_writer — Prints DAQ measurement data to the console (xcpclient --mea without output file)
//
// One line per DAQ packet with event, DAQ list, lost packets, timestamp and the time since the last packet of the DAQ list,
// then one line per signal with its physical value and unit.

use std::io::{self, Write};

#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};

use crate::daq_writer::{DaqLayout, DaqSample, DaqWriter};

// Number of array elements printed
const CONSOLE_ARRAY_ELEMENTS: usize = 16;

pub struct ConsoleWriter<W: Write + Send> {
    writer: W,
    last_time_ns: Vec<u64>, // Last timestamp of each DAQ list
}

impl ConsoleWriter<io::Stdout> {
    pub fn stdout() -> Self {
        ConsoleWriter::new(io::stdout())
    }
}

impl<W: Write + Send> ConsoleWriter<W> {
    pub fn new(writer: W) -> Self {
        ConsoleWriter { writer, last_time_ns: Vec::new() }
    }
}

impl<W: Write + Send> DaqWriter for ConsoleWriter<W> {
    fn name(&self) -> &'static str {
        "Console"
    }

    fn start(&mut self, layout: &DaqLayout, start_ns: u64) -> io::Result<()> {
        self.last_time_ns = vec![start_ns; layout.daq_lists.len()];
        Ok(())
    }

    fn write(&mut self, s: &DaqSample) -> io::Result<()> {
        let last = self.last_time_ns.get_mut(s.daq as usize).ok_or_else(|| io::Error::other("unknown DAQ list"))?;
        let delta_us = s.time_ns.saturating_sub(*last) / 1000;
        *last = s.time_ns;

        let w = &mut self.writer;
        match &s.daq_list.event_name {
            Some(name) => writeln!(
                w,
                "EVENT: {} (id={}, daq={}), lost={}, t={}ns (+{}us)",
                name, s.daq_list.event_id, s.daq, s.lost, s.time_ns, delta_us
            )?,
            None => writeln!(w, "EVENT: id={}, daq={}, lost={}, t={}ns (+{}us)", s.daq_list.event_id, s.daq, s.lost, s.time_ns, delta_us)?,
        }
        for entry in &s.daq_list.entries {
            // Physical value: factor and offset applied, the label of an enum value
            // Array: the first CONSOLE_ARRAY_ELEMENTS elements in memory order (x fastest)
            let value = if entry.is_array() {
                entry.decode_array(s.data).map(|values| {
                    let mut text = values.iter().take(CONSOLE_ARRAY_ELEMENTS).map(|v| v.to_string()).collect::<Vec<_>>().join(" ");
                    if values.len() > CONSOLE_ARRAY_ELEMENTS {
                        text.push_str(&format!(" ... ({} elements)", values.len()));
                    }
                    format!("[{}]", text)
                })
            } else {
                entry.decode(s.data).map(|value| value.to_string())
            };
            let value = value.unwrap_or_else(|| {
                warn!("{}: {} not supported", entry.name, entry.a2l_type);
                String::new()
            });
            let unit = entry.conversion.unit();
            if unit.is_empty() {
                writeln!(w, " {} = {}", entry.name, value)?;
            } else {
                writeln!(w, " {} = {} {}", entry.name, value, unit)?;
            }
        }
        Ok(())
    }

    fn stop(&mut self, _stop_ns: u64) -> io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xcp_client::{A2lAddr, A2lConversion, A2lType, A2lTypeEncoding, OdtEntry, XcpDaqList};

    // An array is printed with its first CONSOLE_ARRAY_ELEMENTS elements
    #[test]
    fn test_console_writer_array() {
        let entry = |name: &str, dim: [u16; 2], offset: u16| OdtEntry {
            name: name.to_string(),
            a2l_type: A2lType {
                size: 1,
                encoding: A2lTypeEncoding::Unsigned,
            },
            dim,
            a2l_addr: A2lAddr { ext: 0, addr: 0, event: Some(0) },
            conversion: A2lConversion::default(),
            offset,
        };
        let layout = DaqLayout::new(
            4,
            1,
            vec![XcpDaqList {
                event_id: 3,
                event_name: Some("mainloop".to_string()),
                entries: vec![entry("short", [3, 1], 0), entry("long", [5, 4], 3)],
                size: 23,
            }],
        );
        let mut w = ConsoleWriter::new(Vec::new());
        w.start(&layout, 0).unwrap();
        let data: Vec<u8> = (0..23).collect();
        let sample = DaqSample {
            daq: 0,
            daq_list: &layout.daq_lists[0],
            lost: 0,
            timestamp: 1000,
            time_ns: 1000,
            packet: &data,
            data: &data,
        };
        w.write(&sample).unwrap();
        let text = String::from_utf8(w.writer).unwrap();
        let lines: Vec<&str> = text.lines().skip(1).collect();
        assert_eq!(lines, [" short = [0 1 2]", " long = [3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 ... (20 elements)]"]);
    }
}
