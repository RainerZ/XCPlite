//--------------------------------------------------------------------------------------------------------------------------------------------------
// decoder.rs — Decoder traits used by the receive task

#![allow(dead_code)]

use super::types::{A2lAddr, A2lType};

//--------------------------------------------------------------------------------------------------------------------------------------------------
// OdtEntry — describes one DAQ signal entry in an ODT

/// DAQ information
/// Describes a single ODT entry
#[derive(Debug)]
pub struct OdtEntry {
    pub name: String,
    pub a2l_type: A2lType,
    pub a2l_addr: A2lAddr,
    pub offset: u16, // offset from data start, not including daq header and timestamp
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
    /// Decoding information: ODT entry table and 64 bit start timestamp
    fn start(&mut self, odt_entries: Vec<Vec<OdtEntry>>, timestamp_raw64: u64);

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
}
