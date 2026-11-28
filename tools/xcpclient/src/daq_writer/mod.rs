//--------------------------------------------------------------------------------------------------------------------------------------------------
// Module daq_writer — Output of DAQ measurement data
//
// The DAQ decoder (DaqDecoder) receives the DAQ packets from the XCP client receive task. It parses the DAQ header, extends the
// 32 bit timestamps to 64 bit, checks the packets and keeps the statistics. Each packet is handed over unchanged to a list of
// writers (DaqWriter), one for each output format: console, CSV, Perfetto, ...
//
// A writer gets the binary layout of the DAQ packets at measurement start (DaqLayout) and then each packet as it was received
// (DaqSample): the complete packet with DAQ header and timestamp, and the payload with the signals. A writer may decode the signals
// (OdtEntry::decode) or store the packets as they are, a binary file format which describes the layout needs no copy or rearrangement.
//
// Adding an output format
//   - Implement DaqWriter in a module <format>_writer
//   - Writer specific settings (file name, ECU name, ...) are parameters of its constructor
//   - Create it in create_daq_writers in main.rs
//
// DAQ packet layout
//
//   header_size 4 (DAQ_HDR_ODT_FIL_DAQW):  ODT (u8), fill (u8), DAQ (u16), timestamp (u32), payload
//   header_size 2 (DAQ_HDR_ODT_DAQB):      ODT (u8), DAQ (u8),              timestamp (u32), payload
//
//   All values little endian. The payload has the signals of the DAQ list (XcpDaqList::entries) at their offsets, its size is
//   XcpDaqList::size. The transport layer may add fill bytes, the packet may be longer than the layout says.
//
//   A DAQ list with a large signal may have more than one ODT (XcpDaqList::odt_sizes), the packets of ODT 1.. have no timestamp.
//   The decoder reassembles the packets of a sample: the writers get one DaqSample with the DAQ header and the timestamp of ODT 0
//   and the payload of all ODTs, without fill bytes. A sample with a lost ODT is dropped, the decoder resynchronizes with the
//   next ODT 0.

use std::io;

#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};

use crate::xcp_client::{DAQ_TIMESTAMP_SIZE, DaqTimestampExtender, XcpDaqDecoder, XcpDaqList};

//--------------------------------------------------------------------------------------------------------------------------------------------------
// DaqLayout

/// Binary layout of the DAQ packets of a measurement
#[derive(Debug)]
pub struct DaqLayout {
    /// Size of the DAQ header: 2 (ODT, DAQ as u8) or 4 (ODT, fill, DAQ as u16)
    pub header_size: usize,
    /// Size of the raw timestamp behind the DAQ header
    pub timestamp_size: usize,
    /// ns per raw timestamp tick
    pub timestamp_resolution_ns: u64,
    /// DAQ lists, the index is the DAQ list number in the DAQ header
    pub daq_lists: Vec<XcpDaqList>,
}

impl DaqLayout {
    pub fn new(header_size: usize, timestamp_resolution_ns: u64, daq_lists: Vec<XcpDaqList>) -> DaqLayout {
        DaqLayout {
            header_size,
            timestamp_size: DAQ_TIMESTAMP_SIZE,
            timestamp_resolution_ns,
            daq_lists,
        }
    }

    /// Offset of the raw timestamp in a DAQ packet
    pub fn timestamp_offset(&self) -> usize {
        self.header_size
    }

    /// Offset of the payload in a DAQ packet, the ODT entry offsets are relative to it
    pub fn payload_offset(&self) -> usize {
        self.header_size + self.timestamp_size
    }

    /// Size of the DAQ packet of a DAQ list, without fill bytes of the transport layer
    pub fn packet_size(&self, daq: usize) -> usize {
        self.payload_offset() + self.daq_lists[daq].size
    }
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
// DaqSample

/// A DAQ packet, a sample of a DAQ list
#[derive(Debug)]
pub struct DaqSample<'a> {
    /// DAQ list number
    pub daq: u16,
    /// DAQ list
    pub daq_list: &'a XcpDaqList,
    /// Packets lost before this one (transport layer counter)
    pub lost: u32,
    /// 64 bit timestamp in raw ticks, extended from the 32 bit raw timestamp in the packet
    pub timestamp: u64,
    /// 64 bit timestamp in ns
    pub time_ns: u64,
    /// The complete DAQ packet as received, with DAQ header, raw timestamp and maybe fill bytes, see DaqLayout
    pub packet: &'a [u8],
    /// The payload of the DAQ packet, without DAQ header and timestamp
    pub data: &'a [u8],
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
// DaqWriter

/// Output of DAQ measurement data in one format
///
/// Called from the receive task of the XCP client, with the DAQ decoder locked. A writer which returns an error is removed, the
/// measurement goes on with the others.
pub trait DaqWriter: Send {
    /// Name of the output format, for log messages
    fn name(&self) -> &'static str;

    /// Measurement start
    /// layout is the binary layout of the DAQ packets, start_ns is the timestamp of the measurement start in ns
    fn start(&mut self, layout: &DaqLayout, start_ns: u64) -> io::Result<()>;

    /// A DAQ packet
    fn write(&mut self, sample: &DaqSample) -> io::Result<()>;

    /// Measurement stop, stop_ns is the newest timestamp of the measurement in ns
    /// Completes and flushes the output
    fn stop(&mut self, stop_ns: u64) -> io::Result<()>;
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
// DaqDecoder

/// Receives the DAQ packets from the XCP client and hands them to the writers
pub struct DaqDecoder {
    writers: Vec<Box<dyn DaqWriter>>,
    layout: Option<DaqLayout>,
    timestamp_resolution: u64,
    header_size: u8,
    event_count: usize,
    byte_count: usize,
    lost_count: usize,                        // DAQ packets lost, from the gaps of the transport layer counter
    packet_count: usize,                      // DAQ packets received
    dropped_count: usize,                     // Incomplete samples of DAQ lists with more than one ODT dropped
    assembly: Vec<Option<DaqAssembly>>,       // Sample of each DAQ list with more than one ODT, which is being reassembled
    daq_timestamp: Vec<u64>,                  // Last timestamp of each DAQ list, to check for monotonic timestamps
    timestamp_extender: DaqTimestampExtender, // 32 to 64 bit timestamp extension, shared by all DAQ lists
    last_timestamp_ns: u64,                   // Newest timestamp of all DAQ lists
}

impl DaqDecoder {
    pub fn new() -> DaqDecoder {
        DaqDecoder {
            writers: Vec::new(),
            layout: None,
            timestamp_resolution: 1,
            header_size: 4,
            event_count: 0,
            byte_count: 0,
            lost_count: 0,
            packet_count: 0,
            dropped_count: 0,
            assembly: Vec::new(),
            daq_timestamp: Vec::new(),
            timestamp_extender: DaqTimestampExtender::new(0),
            last_timestamp_ns: 0,
        }
    }

    /// Set the writers for the next measurement, before the measurement start
    pub fn set_writers(&mut self, writers: Vec<Box<dyn DaqWriter>>) {
        self.writers = writers;
    }

    // Parse the DAQ header, returns DAQ list number and ODT number
    fn parse_header(&self, buf: &[u8]) -> Option<(u16, u8)> {
        let header_size = self.header_size as usize;
        if buf.len() < header_size {
            return None;
        }
        let daq = if header_size == 4 { u16::from_le_bytes([buf[2], buf[3]]) } else { buf[1] as u16 };
        Some((daq, buf[0]))
    }
}

/// A sample of a DAQ list with more than one ODT, which is being reassembled
struct DaqAssembly {
    packet: Vec<u8>, // DAQ header and timestamp of ODT 0, then the payload of the ODTs received so far
    next_odt: usize, // Next ODT expected
    lost: u32,       // Packets lost before ODT 0
    timestamp: u64,  // 64 bit timestamp in raw ticks
}

// Call a writer function for all writers, a writer which fails is removed
fn for_each_writer(writers: &mut Vec<Box<dyn DaqWriter>>, mut f: impl FnMut(&mut dyn DaqWriter) -> io::Result<()>) {
    writers.retain_mut(|w| match f(w.as_mut()) {
        Ok(()) => true,
        Err(e) => {
            error!("{} output failed, stopped: {}", w.name(), e);
            false
        }
    });
}

impl Default for DaqDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl XcpDaqDecoder for DaqDecoder {
    // Set timestamp resolution and DAQ header size, called at connect
    fn set_daq_properties(&mut self, timestamp_resolution: u64, daq_header_size: u8) {
        self.header_size = daq_header_size;
        self.timestamp_resolution = timestamp_resolution.max(1);
    }

    // Measurement start
    fn start(&mut self, daq_lists: Vec<XcpDaqList>, timestamp: u64) {
        let layout = DaqLayout::new(self.header_size as usize, self.timestamp_resolution, daq_lists);
        self.last_timestamp_ns = timestamp * self.timestamp_resolution;
        self.daq_timestamp = vec![timestamp; layout.daq_lists.len()];
        self.event_count = 0;
        self.byte_count = 0;
        self.lost_count = 0;
        self.packet_count = 0;
        self.dropped_count = 0;
        self.assembly = (0..layout.daq_lists.len()).map(|_| None).collect();
        self.timestamp_extender = DaqTimestampExtender::new(timestamp);
        let start_ns = self.last_timestamp_ns;
        for_each_writer(&mut self.writers, |w| w.start(&layout, start_ns));
        self.layout = Some(layout);
    }

    // Measurement stop
    fn stop(&mut self) {
        let stop_ns = self.last_timestamp_ns;
        for_each_writer(&mut self.writers, |w| w.stop(stop_ns));
    }

    // DAQ packet
    fn decode(&mut self, lost: u32, buf: &[u8]) {
        let Some(layout) = &self.layout else {
            return;
        };
        self.lost_count += lost as usize;

        // Only DAQ lists and ODTs of the measurement setup are expected, ignore anything else
        let (daq, odt) = match self.parse_header(buf) {
            Some((daq, odt)) if (daq as usize) < layout.daq_lists.len() && (odt as usize) < layout.daq_lists[daq as usize].odt_count() => (daq, odt as usize),
            Some((daq, odt)) => {
                warn!(
                    "Unexpected DAQ packet ignored: daq={}, odt={}, len={}, {} DAQ lists configured",
                    daq,
                    odt,
                    buf.len(),
                    layout.daq_lists.len()
                );
                return;
            }
            None => {
                warn!("DAQ packet too short ignored: len={}", buf.len());
                return;
            }
        };
        self.packet_count += 1;
        let daq_list = &layout.daq_lists[daq as usize];
        let header_size = layout.header_size;
        let payload_offset = layout.payload_offset();

        // A lost packet may belong to any DAQ list, the samples which are being reassembled are incomplete
        if lost > 0 {
            for assembly in self.assembly.iter_mut().filter(|a| a.is_some()) {
                *assembly = None;
                self.dropped_count += 1;
            }
        }

        // ODT 0: timestamp, a single ODT is a complete sample
        // ODT 1..: append to the sample which is being reassembled, if it is the next ODT, else the sample is incomplete
        let assembly = if odt == 0 {
            if self.assembly[daq as usize].take().is_some() {
                self.dropped_count += 1; // ODT 0 before the last ODT of the previous sample
            }
            let packet_size = payload_offset + daq_list.odt_size(0);
            if buf.len() < packet_size {
                warn!("DAQ packet too short ignored: daq={}, odt=0, len={}", daq, buf.len());
                return;
            }
            let ts = &buf[header_size..header_size + DAQ_TIMESTAMP_SIZE];
            let timestamp_raw = u32::from_le_bytes([ts[0], ts[1], ts[2], ts[3]]);

            // Extend to a 64 bit timestamp
            // The extension uses the newest timestamp of all DAQ lists as reference, the order is checked for each DAQ list on its own
            let t_last = self.daq_timestamp[daq as usize];
            let t = self.timestamp_extender.extend(timestamp_raw);
            if t < t_last {
                warn!("Timestamp of daq {} declining {} -> {}", daq, t_last, t);
            }
            self.daq_timestamp[daq as usize] = t;
            if daq_list.odt_count() == 1 {
                None // Complete sample in buf
            } else {
                let mut packet = Vec::with_capacity(payload_offset + daq_list.size);
                packet.extend_from_slice(&buf[..packet_size]);
                self.assembly[daq as usize] = Some(DaqAssembly {
                    packet,
                    next_odt: 1,
                    lost,
                    timestamp: t,
                });
                return;
            }
        } else {
            let Some(mut assembly) = self.assembly[daq as usize].take() else {
                return; // ODT of a sample which is incomplete, wait for the next ODT 0
            };
            let size = daq_list.odt_size(odt);
            if assembly.next_odt != odt || buf.len() < header_size + size {
                self.dropped_count += 1; // Out of sequence or too short
                return;
            }
            assembly.packet.extend_from_slice(&buf[header_size..header_size + size]);
            assembly.next_odt += 1;
            if assembly.next_odt < daq_list.odt_count() {
                self.assembly[daq as usize] = Some(assembly);
                return;
            }
            Some(assembly) // Complete
        };

        let (packet, lost, t) = match &assembly {
            Some(a) => (a.packet.as_slice(), a.lost, a.timestamp),
            None => (buf, lost, self.daq_timestamp[daq as usize]),
        };
        let time_ns = t * self.timestamp_resolution;
        self.last_timestamp_ns = self.last_timestamp_ns.max(time_ns);
        let sample = DaqSample {
            daq,
            daq_list,
            lost,
            timestamp: t,
            time_ns,
            packet,
            data: &packet[payload_offset..],
        };
        self.byte_count += sample.data.len(); // overall payload byte count
        self.event_count += 1; // overall count of complete samples

        for_each_writer(&mut self.writers, |w| w.write(&sample));
    }

    fn get_byte_count(&self) -> usize {
        self.byte_count
    }

    fn get_event_count(&self) -> usize {
        self.event_count
    }

    fn get_lost_count(&self) -> usize {
        self.lost_count
    }

    fn get_packet_count(&self) -> usize {
        self.packet_count
    }

    fn get_dropped_count(&self) -> usize {
        self.dropped_count
    }
}

//--------------------------------------------------------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xcp_client::{A2lAddr, A2lConversion, A2lType, A2lTypeEncoding, OdtEntry};
    use std::sync::{Arc, Mutex};

    // Records what it gets
    #[derive(Default)]
    struct Recorder {
        log: Arc<Mutex<Vec<String>>>,
        fail_after: Option<usize>,
    }

    impl DaqWriter for Recorder {
        fn name(&self) -> &'static str {
            "Recorder"
        }
        fn start(&mut self, layout: &DaqLayout, start_ns: u64) -> io::Result<()> {
            self.log
                .lock()
                .unwrap()
                .push(format!("start {} {} {}", layout.payload_offset(), layout.packet_size(0), start_ns));
            Ok(())
        }
        fn write(&mut self, s: &DaqSample) -> io::Result<()> {
            let mut log = self.log.lock().unwrap();
            if self.fail_after.is_some_and(|n| log.len() > n) {
                return Err(io::Error::other("full"));
            }
            let value = s.daq_list.entries[0].decode(s.data).unwrap();
            log.push(format!("write {} {} {} {} {}", s.daq, s.lost, s.time_ns, s.packet.len(), value));
            Ok(())
        }
        fn stop(&mut self, stop_ns: u64) -> io::Result<()> {
            self.log.lock().unwrap().push(format!("stop {}", stop_ns));
            Ok(())
        }
    }

    fn daq_list() -> XcpDaqList {
        XcpDaqList {
            event_id: 1,
            event_name: None,
            entries: vec![OdtEntry {
                name: "x".to_string(),
                a2l_type: A2lType {
                    size: 2,
                    encoding: A2lTypeEncoding::Unsigned,
                },
                dim: [1, 1],
                a2l_addr: A2lAddr { ext: 0, addr: 0, event: Some(1) },
                conversion: A2lConversion::default(),
                offset: 0,
            }],
            odt_sizes: Vec::new(),
            size: 2,
        }
    }

    // ODT 0, fill 0xAA, DAQ 0, timestamp, payload, 2 fill bytes of the transport layer
    fn packet(timestamp: u32, value: u16) -> Vec<u8> {
        let mut p = vec![0, 0xAA, 0, 0];
        p.extend(timestamp.to_le_bytes());
        p.extend(value.to_le_bytes());
        p.extend([0, 0]);
        p
    }

    #[test]
    fn test_daq_decoder() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut d = DaqDecoder::new();
        d.set_daq_properties(10, 4);
        d.set_writers(vec![Box::new(Recorder {
            log: Arc::clone(&log),
            fail_after: None,
        })]);
        d.start(vec![daq_list()], 0x1_0000_0000);
        d.decode(0, &packet(5, 42));
        d.decode(3, &packet(7, 43));
        d.decode(0, &[1, 0xAA, 0, 0, 0, 0, 0, 0]); // ODT 1, ignored
        d.decode(0, &[0, 0xAA, 1, 0, 0, 0, 0, 0, 0, 0]); // DAQ 1, ignored
        d.decode(0, &[0, 0xAA, 0, 0, 0]); // too short, ignored
        d.stop();
        assert_eq!(
            *log.lock().unwrap(),
            ["start 8 10 42949672960", "write 0 0 42949673010 12 42", "write 0 3 42949673030 12 43", "stop 42949673030"]
        );
        assert_eq!(d.get_event_count(), 2);
        assert_eq!(d.get_byte_count(), 8);
        assert_eq!(d.get_lost_count(), 3);

        // A new measurement resets the lost count
        d.start(vec![daq_list()], 0);
        assert_eq!(d.get_lost_count(), 0);
    }

    #[test]
    fn test_daq_decoder_failing_writer() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut d = DaqDecoder::new();
        d.set_daq_properties(1, 4);
        d.set_writers(vec![Box::new(Recorder {
            log: Arc::clone(&log),
            fail_after: Some(1),
        })]);
        d.start(vec![daq_list()], 0);
        for i in 0..4 {
            d.decode(0, &packet(i, i as u16));
        }
        d.stop();
        // Removed after the first error, no stop
        assert_eq!(*log.lock().unwrap(), ["start 8 10 0", "write 0 0 0 12 0"]);
        assert_eq!(d.get_event_count(), 4);
    }

    // Records the payload of each sample: DAQ list, lost, time_ns, payload
    type PayloadLog = Arc<Mutex<Vec<(u16, u32, u64, Vec<u8>)>>>;
    struct PayloadRecorder {
        log: PayloadLog,
    }

    impl DaqWriter for PayloadRecorder {
        fn name(&self) -> &'static str {
            "PayloadRecorder"
        }
        fn start(&mut self, _layout: &DaqLayout, _start_ns: u64) -> io::Result<()> {
            Ok(())
        }
        fn write(&mut self, s: &DaqSample) -> io::Result<()> {
            if s.daq_list.odt_count() > 1 {
                assert_eq!(s.packet.len(), 8 + s.daq_list.size); // Reassembled: DAQ header and timestamp of ODT 0, payload without fill bytes
            }
            self.log.lock().unwrap().push((s.daq, s.lost, s.time_ns, s.data.to_vec()));
            Ok(())
        }
        fn stop(&mut self, _stop_ns: u64) -> io::Result<()> {
            Ok(())
        }
    }

    // DAQ packet of DAQ list daq, ODT odt, with a timestamp in ODT 0, 2 fill bytes of the transport layer
    fn odt_packet(daq: u16, odt: u8, timestamp: u32, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![odt, 0xAA];
        p.extend(daq.to_le_bytes());
        if odt == 0 {
            p.extend(timestamp.to_le_bytes());
        }
        p.extend(payload);
        p.extend([0, 0]);
        p
    }

    // A DAQ list with 3 ODTs (4, 6 and 2 bytes) is reassembled to one sample, a sample with a lost ODT is dropped
    #[test]
    fn test_daq_decoder_more_odts() {
        let image = XcpDaqList {
            event_id: 1,
            event_name: None,
            entries: vec![OdtEntry {
                name: "image".to_string(),
                a2l_type: A2lType {
                    size: 1,
                    encoding: A2lTypeEncoding::Unsigned,
                },
                dim: [12, 1],
                a2l_addr: A2lAddr { ext: 0, addr: 0, event: Some(1) },
                conversion: A2lConversion::default(),
                offset: 0,
            }],
            odt_sizes: vec![4, 6, 2],
            size: 12,
        };
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut d = DaqDecoder::new();
        d.set_daq_properties(1, 4);
        d.set_writers(vec![Box::new(PayloadRecorder { log: Arc::clone(&log) })]);
        d.start(vec![daq_list(), image], 0);

        // Complete sample, interleaved with a sample of the DAQ list with one ODT
        d.decode(2, &odt_packet(1, 0, 10, &[0, 1, 2, 3]));
        d.decode(0, &odt_packet(1, 1, 0, &[4, 5, 6, 7, 8, 9]));
        d.decode(0, &odt_packet(0, 0, 11, &[42, 0]));
        d.decode(0, &odt_packet(1, 2, 0, &[10, 11]));
        // Lost packet: the sample is incomplete, the rest of it is ignored
        d.decode(0, &odt_packet(1, 0, 20, &[0, 1, 2, 3]));
        d.decode(1, &odt_packet(1, 2, 0, &[10, 11]));
        // Out of sequence: ODT 2 instead of ODT 1
        d.decode(0, &odt_packet(1, 0, 30, &[0, 1, 2, 3]));
        d.decode(0, &odt_packet(1, 2, 0, &[10, 11]));
        // ODT 0 before the last ODT of the previous sample, then complete
        d.decode(0, &odt_packet(1, 0, 40, &[0, 1, 2, 3]));
        d.decode(0, &odt_packet(1, 0, 50, &[9, 9, 9, 9]));
        d.decode(0, &odt_packet(1, 1, 0, &[8, 8, 8, 8, 8, 8]));
        d.decode(0, &odt_packet(1, 2, 0, &[7, 7]));
        // ODT number of a DAQ list with less ODTs, ignored
        d.decode(0, &odt_packet(1, 3, 0, &[0]));
        d.stop();

        assert_eq!(
            *log.lock().unwrap(),
            [
                (0, 0, 11, vec![42, 0, 0, 0]),
                (1, 2, 10, (0..12).collect::<Vec<u8>>()),
                (1, 0, 50, vec![9, 9, 9, 9, 8, 8, 8, 8, 8, 8, 7, 7]),
            ]
        );
        assert_eq!(d.get_event_count(), 3);
        assert_eq!(d.get_packet_count(), 12);
        assert_eq!(d.get_dropped_count(), 3);
        assert_eq!(d.get_lost_count(), 3);
        assert_eq!(d.get_byte_count(), 4 + 12 + 12);
    }
}
