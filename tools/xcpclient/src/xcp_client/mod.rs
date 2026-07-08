//--------------------------------------------------------------------------------------------------------------------------------------------------
// Module xcp_client
// Simplified, quick and dirty implementation of an UDP XCP client for integration testing

#![allow(dead_code)] // because of all the unused XCP definitions

#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};

use parking_lot::Mutex;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::mpsc;
use tokio::time::Duration;

pub mod xcp;
use xcp::*;

mod types;
pub use types::*;

mod decoder;
pub use decoder::*;

mod transport;
mod protocol;
mod a2l;
mod cal;
mod daq;

//--------------------------------------------------------------------------------------------------------------------------------------------------
// XCP Parameters

pub const CMD_TIMEOUT: Duration = Duration::from_secs(3);



//--------------------------------------------------------------------------------------------------------------------------------------------------
// Type to control the receive task sent over the receive task control channel

#[derive(Debug, Copy, Clone)]
pub struct XcpTaskControl {
    running: bool,
    connected: bool,
}

impl XcpTaskControl {
    #[allow(clippy::new_without_default)]
    pub fn new() -> XcpTaskControl {
        XcpTaskControl { running: false, connected: false }
    }
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
// Socket abstraction for UDP and TCP

#[derive(Debug)]
enum XcpSocket {
    Udp(Arc<UdpSocket>),
    Tcp(Arc<TcpStream>),
}

impl XcpSocket {
    async fn send_to(&self, buf: &[u8], addr: SocketAddr) -> Result<usize, std::io::Error> {
        match self {
            XcpSocket::Udp(udp_socket) => {
                // On macOS, sendto() on a UDP socket returns EHOSTUNREACH immediately when
                // the ARP cache is empty, instead of queuing the packet like Linux does.
                // Retry with backoff to allow ARP resolution to complete.
                let mut delay_ms = 50u64;
                loop {
                    match udp_socket.send_to(buf, addr).await {
                        Ok(n) => return Ok(n),
                        Err(e) if e.kind() == std::io::ErrorKind::HostUnreachable && delay_ms <= 400 => {
                            debug!("send_to: EHOSTUNREACH, retrying after {}ms (ARP not yet resolved)", delay_ms);
                            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                            delay_ms *= 2;
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            XcpSocket::Tcp(tcp_stream) => {
                // But for now, let's revert to the working approach:
                let mut pos = 0;
                while pos < buf.len() {
                    match tcp_stream.try_write(&buf[pos..]) {
                        Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "write zero bytes")),
                        Ok(n) => pos += n,
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            tcp_stream.writable().await?;
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok(buf.len())
            }
        }
    }
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
//--------------------------------------------------------------------------------------------------------------------------------------------------
//--------------------------------------------------------------------------------------------------------------------------------------------------
// XcpClient

/// XCP client
pub struct XcpClient {
    protocol: &'static str,
    baud_rate: u32,
    bind_addr: SocketAddr,
    dest_addr: SocketAddr,

    // Information from connect and get_comm_mode_info commands
    pub resources: u8,
    pub comm_mode_basic: u8,
    pub max_cto_size: u8,
    pub max_dto_size: u16,
    pub protocol_version: u16,
    pub transport_layer_version: u16,
    pub comm_mode_optional: u8,
    pub driver_version: u8,
    pub max_segments: u8,
    pub freeze_supported: bool,
    pub max_events: u16,

    pub registry: Option<xcp_registry::Registry>,

    timestamp_resolution_ns: u64,
    daq_header_size: u8,

    socket: Option<XcpSocket>,
    receive_task: Option<tokio::task::JoinHandle<()>>,
    rx_cmd_resp: Option<mpsc::Receiver<Vec<u8>>>,
    tx_task_control: Option<mpsc::Sender<XcpTaskControl>>,
    task_control: XcpTaskControl,
    daq_decoder: Option<Arc<Mutex<dyn XcpDaqDecoder>>>,
    ctr: u16,

    calibration_object_list: Vec<XcpClientCalibrationObject>,
    measurement_object_list: Vec<XcpClientMeasurementObject>,
}

impl XcpClient {
    //------------------------------------------------------------------------
    // new
    //
    #[allow(clippy::type_complexity)]
    pub fn new(protocol: &'static str, dest_addr: SocketAddr, bind_addr: SocketAddr, baud_rate: u32) -> XcpClient {
        XcpClient {
            protocol,
            bind_addr,
            dest_addr,
            baud_rate,
            socket: None,
            receive_task: None,
            rx_cmd_resp: None,
            tx_task_control: None,
            task_control: XcpTaskControl::new(),
            daq_decoder: None,
            ctr: 0,
            resources: 0,
            comm_mode_basic: 0,
            comm_mode_optional: 0,
            driver_version: 0,
            max_cto_size: 0,
            max_dto_size: 0,
            max_segments: 0,
            max_events: 0,
            freeze_supported: false,
            protocol_version: 0,
            transport_layer_version: 0,
            timestamp_resolution_ns: 1,
            daq_header_size: 4,
            registry: None,
            calibration_object_list: Vec::new(),
            measurement_object_list: Vec::new(),
        }
    }

    pub fn set_registry(&mut self, registry: xcp_registry::Registry) {
        self.registry = Some(registry);
    }
}
