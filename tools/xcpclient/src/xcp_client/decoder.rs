//--------------------------------------------------------------------------------------------------------------------------------------------------
// decoder.rs — Decoder traits used by the receive task

#![allow(dead_code)]

use super::types::{A2lAddr, A2lConversion, A2lType, A2lValue};

//--------------------------------------------------------------------------------------------------------------------------------------------------
// DAQ packet layout

/// Size of the DAQ timestamp in bytes, behind the DAQ header in the first ODT of a DAQ list
/// xcpclient uses one ODT per DAQ list, so every DAQ packet has a timestamp
pub const DAQ_TIMESTAMP_SIZE: usize = 4;

//--------------------------------------------------------------------------------------------------------------------------------------------------
// OdtEntry — describes one DAQ signal entry in an ODT

/// DAQ information
/// Describes a single ODT entry
#[derive(Debug)]
pub struct OdtEntry {
    pub name: String,
    pub a2l_type: A2lType, // type of one element
    pub dim: [u16; 2],     // array dimensions x, y (MATRIX_DIM), [1, 1] for a scalar, the elements are stored x fastest
    pub a2l_addr: A2lAddr,
    pub conversion: A2lConversion, // conversion from raw to physical value, for each element
    pub offset: u16,               // offset from data start, not including daq header and timestamp
}

impl OdtEntry {
    /// Number of elements, 1 for a scalar
    pub fn element_count(&self) -> usize {
        self.dim[0] as usize * self.dim[1] as usize
    }

    /// The entry is an array (more than one element)
    pub fn is_array(&self) -> bool {
        self.element_count() > 1
    }

    /// Size of the entry in bytes, all elements of an array
    pub fn size(&self) -> usize {
        self.a2l_type.size * self.element_count()
    }

    /// Dimensions as text: "16" for a one dimensional array, "2048x1536" (x, y) for a two dimensional array
    pub fn dim_text(&self) -> String {
        if self.dim[1] > 1 {
            format!("{}x{}", self.dim[0], self.dim[1])
        } else {
            self.dim[0].to_string()
        }
    }

    /// Decode the raw value of this entry, the first element of an array, from the DAQ data (without daq header and timestamp)
    /// Returns None for a blob or when the data is too short
    pub fn decode_raw(&self, data: &[u8]) -> Option<A2lValue> {
        self.a2l_type.decode(data.get(self.offset as usize..)?)
    }

    /// Decode the physical value of this entry, the first element of an array, from the DAQ data (without daq header and timestamp)
    /// Returns None for a blob or when the data is too short
    pub fn decode(&self, data: &[u8]) -> Option<A2lValue> {
        self.decode_raw(data).map(|raw| self.conversion.convert(raw))
    }

    /// Decode the physical values of all elements, in memory order (x fastest), from the DAQ data (without daq header and timestamp)
    /// Returns None for a blob or when the data is too short
    pub fn decode_array(&self, data: &[u8]) -> Option<Vec<A2lValue>> {
        let data = data.get(self.offset as usize..self.offset as usize + self.size())?;
        data.chunks_exact(self.a2l_type.size)
            .map(|bytes| self.a2l_type.decode(bytes).map(|raw| self.conversion.convert(raw)))
            .collect()
    }
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
// XcpDaqList — describes one DAQ list

/// DAQ list information
/// A DAQ list is attached to exactly one event and has one or more ODTs
/// The DAQ packet of the first ODT is: DAQ header, timestamp (DAQ_TIMESTAMP_SIZE), payload, the following ODTs have no timestamp,
/// the transport layer may add fill bytes. The payload of the DAQ list is the payload of all ODTs (size bytes).
#[derive(Debug)]
pub struct XcpDaqList {
    pub event_id: u16,              // id of the event the DAQ list is attached to
    pub event_name: Option<String>, // unique name of the event (with instance index), None if the event is not in the registry event list
    pub entries: Vec<OdtEntry>,     // signals of the DAQ list, offsets are relative to the payload of all ODTs
    pub odt_sizes: Vec<usize>,      // payload size of each ODT, empty for one ODT with size bytes
    pub size: usize,                // payload size in bytes of all ODTs, without DAQ header and timestamp
}

impl XcpDaqList {
    /// Number of ODTs
    pub fn odt_count(&self) -> usize {
        self.odt_sizes.len().max(1)
    }

    /// Payload size of ODT odt, without DAQ header and timestamp
    pub fn odt_size(&self, odt: usize) -> usize {
        if self.odt_sizes.is_empty() { self.size } else { self.odt_sizes[odt] }
    }
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
// DaqTimestampExtender — extends the 32 bit raw DAQ timestamps of all DAQ lists to 64 bit

/// Extends 32 bit raw DAQ timestamps to 64 bit
///
/// There is one reference for all DAQ lists, the newest timestamp seen so far. A raw timestamp is placed at the position
/// nearest to the reference, by the signed 32 bit difference to the lower half of the reference.
/// The order of the timestamps is guaranteed only within a DAQ list, a sample of another DAQ list may be older than the
/// reference. It gets a negative difference and is placed behind the reference, it does not count as a wraparound.
///
/// Precondition: Every timestamp is within half the 32 bit range (2.147s with 1ns ticks) of the newest timestamp seen before.
/// So at least one sample of any DAQ list is needed in this time, a DAQ list on its own may be silent for any time.
#[derive(Debug, Clone, Copy, Default)]
pub struct DaqTimestampExtender {
    reference: u64,
}

impl DaqTimestampExtender {
    /// Create with the 64 bit raw timestamp of the measurement start as reference
    pub fn new(start_raw64: u64) -> DaqTimestampExtender {
        DaqTimestampExtender { reference: start_raw64 }
    }

    /// Extend a 32 bit raw timestamp to 64 bit
    pub fn extend(&mut self, timestamp_raw: u32) -> u64 {
        let delta = timestamp_raw.wrapping_sub(self.reference as u32) as i32;
        let t = if delta >= 0 {
            self.reference + delta as u64
        } else {
            self.reference.saturating_sub(delta.unsigned_abs() as u64)
        };
        if t > self.reference {
            self.reference = t;
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_daq_timestamp_extender() {
        // Monotonic timestamps of one DAQ list, with wraparound
        let mut e = DaqTimestampExtender::new(0x1_FFFF_FF00);
        assert_eq!(e.extend(0xFFFF_FF10), 0x1_FFFF_FF10);
        assert_eq!(e.extend(0x0000_0010), 0x2_0000_0010); // wraparound
        assert_eq!(e.extend(0x0000_0020), 0x2_0000_0020);

        // A sample of another DAQ list, taken before the wraparound, arrives after it
        assert_eq!(e.extend(0xFFFF_FFF0), 0x1_FFFF_FFF0);
        // The reference has not moved back
        assert_eq!(e.extend(0x0000_0030), 0x2_0000_0030);

        // Equal timestamps
        assert_eq!(e.extend(0x0000_0030), 0x2_0000_0030);

        // A DAQ list which has been silent for many wraparounds, the other DAQ lists kept the reference up to date
        let mut e = DaqTimestampExtender::new(0);
        let mut t: u64 = 0;
        for _ in 0..20 {
            t += 0x4000_0000; // a quarter of the 32 bit range
            assert_eq!(e.extend(t as u32), t);
        }
        assert_eq!(e.extend((t - 5) as u32), t - 5); // silent DAQ list, slightly older than the reference
        assert_eq!(e.extend((t + 5) as u32), t + 5);

        // Limits of the window: just below half the range ahead and half the range behind
        let mut e = DaqTimestampExtender::new(0x5_0000_0000);
        assert_eq!(e.extend(0x7FFF_FFFF), 0x5_7FFF_FFFF);
        let mut e = DaqTimestampExtender::new(0x5_0000_0000);
        assert_eq!(e.extend(0x8000_0000), 0x4_8000_0000);

        // First samples are older than a small start reference, no underflow
        let mut e = DaqTimestampExtender::new(0x10);
        assert_eq!(e.extend(0xFFFF_FFF0), 0);
        assert_eq!(e.extend(0x20), 0x20);
    }
    // Array: element count, size, dimensions text and decoding of all elements with the conversion of each element
    #[test]
    fn test_odt_entry_decode_array() {
        use crate::xcp_client::types::A2lTypeEncoding;
        let entry = OdtEntry {
            name: "map".to_string(),
            a2l_type: A2lType {
                size: 2,
                encoding: A2lTypeEncoding::Unsigned,
            },
            dim: [3, 2],
            a2l_addr: A2lAddr { ext: 0, addr: 0, event: Some(0) },
            conversion: A2lConversion::new(Some(0.5), None, ""),
            offset: 1,
        };
        assert!(entry.is_array());
        assert_eq!((entry.element_count(), entry.size(), entry.dim_text()), (6, 12, "3x2".to_string()));
        let data: Vec<u8> = [0xFFu8].into_iter().chain((1u16..=6).flat_map(|v| v.to_le_bytes())).collect();
        let values: Vec<f64> = entry.decode_array(&data).unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        assert_eq!(values, [0.5, 1.0, 1.5, 2.0, 2.5, 3.0]);
        assert_eq!(entry.decode(&data).unwrap().as_f64(), Some(0.5)); // First element
        assert!(entry.decode_array(&data[..12]).is_none()); // Too short
        let entry = OdtEntry { dim: [16, 1], ..entry };
        assert_eq!(entry.dim_text(), "16");
        let entry = OdtEntry { dim: [1, 1], ..entry };
        assert!(!entry.is_array());
    }
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
// XcpTextDecoder — receives XCP service-text messages from the server

pub trait XcpTextDecoder {
    /// Handle incomming SERV_TEXT data from XCP server
    fn decode(&self, data: &[u8]) {
        print!("[SERV_TEXT] ");
        let mut j = 0;
        while j < data.len() {
            if data[j] == 0 {
                break;
            }
            print!("{}", data[j] as char);
            j += 1;
        }
    }
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
// XcpDaqDecoder — receives raw DAQ packets and decodes them into measurement values

pub trait XcpDaqDecoder {
    /// Handle incomming DAQ packet from XCP server
    /// Transport layer header has been stripped
    fn decode(&mut self, lost: u32, data: &[u8]);

    /// Measurement start
    /// Decoding information: DAQ lists (event and ODT entry table, index is the DAQ list number) and 64 bit start timestamp
    fn start(&mut self, daq_lists: Vec<XcpDaqList>, timestamp_raw64: u64);

    /// Measurement stop
    fn stop(&mut self) {}

    /// Set measurement timestamp resolution in ns per raw timestamp tick and DAQ header size (2 (ODTB/DAQB or 4 (ODTB,_,DAQW))
    fn set_daq_properties(&mut self, timestamp_resolution: u64, daq_header_size: u8);

    /// Get the event count
    fn get_event_count(&self) -> usize {
        0
    }

    /// Get the byte count
    fn get_byte_count(&self) -> usize {
        0
    }

    /// Get the number of lost DAQ packets, from the gaps of the transport layer counter
    fn get_lost_count(&self) -> usize {
        0
    }

    /// Get the number of DAQ packets received, a sample of a DAQ list with more than one ODT has more than one packet
    fn get_packet_count(&self) -> usize {
        0
    }

    /// Get the number of incomplete samples of DAQ lists with more than one ODT, dropped because an ODT was lost
    fn get_dropped_count(&self) -> usize {
        0
    }
}
