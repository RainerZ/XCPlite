//--------------------------------------------------------------------------------------------------------------------------------------------------
// types.rs — A2L address/type descriptors and calibration/measurement object wrappers

#![allow(dead_code)]

use xcp_registry::McValueType;

use super::XcpClient;

//--------------------------------------------------------------------------------------------------------------------------------------------------
// A2L address

#[derive(Debug, Clone, Copy)]
pub struct A2lAddr {
    pub ext: u8,
    pub addr: u32,
    pub event: Option<u16>,
}

impl std::fmt::Display for A2lAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if let Some(event) = self.event {
            write!(f, "{}:0x{:08X} event {}", self.ext, self.addr, event)
        } else {
            write!(f, "{}:0x{:08X}", self.ext, self.addr)
        }
    }
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
// A2L type encoding and descriptor

#[derive(Debug, Clone, Copy)]
pub enum A2lTypeEncoding {
    Signed,
    Unsigned,
    Float,
    Blob,
}

impl From<&McValueType> for A2lTypeEncoding {
    fn from(value_type: &McValueType) -> A2lTypeEncoding {
        match value_type {
            McValueType::Bool | McValueType::Ubyte | McValueType::Uword | McValueType::Ulong | McValueType::Ulonglong => A2lTypeEncoding::Unsigned,
            McValueType::Sbyte | McValueType::Sword | McValueType::Slong | McValueType::Slonglong => A2lTypeEncoding::Signed,
            McValueType::Float32Ieee | McValueType::Float64Ieee => A2lTypeEncoding::Float,
            _ => A2lTypeEncoding::Blob,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct A2lType {
    pub size: usize,
    pub encoding: A2lTypeEncoding,
}

impl std::fmt::Display for A2lType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.encoding {
            A2lTypeEncoding::Signed => write!(f, "{} byte signed", self.size),
            A2lTypeEncoding::Unsigned => write!(f, "{} byte unsigned", self.size),
            A2lTypeEncoding::Float => write!(f, "{} byte float", self.size),
            A2lTypeEncoding::Blob => write!(f, "{} byte blob", self.size),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct A2lLimits {
    pub lower: f64,
    pub upper: f64,
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
// Calibration object

#[derive(Debug, Clone, Copy)]
pub struct XcpCalibrationObjectHandle(pub(super) usize);

impl XcpCalibrationObjectHandle {
    pub fn get_name(self, xcp_client: &mut XcpClient) -> &str {
        xcp_client.get_calibration_object(self).get_name()
    }
    pub fn get_a2l_addr(self, xcp_client: &mut XcpClient) -> A2lAddr {
        xcp_client.get_calibration_object(self).get_a2l_addr()
    }
    pub fn get_a2l_type(self, xcp_client: &mut XcpClient) -> A2lType {
        xcp_client.get_calibration_object(self).get_a2l_type()
    }
}

#[derive(Debug)]
pub struct XcpClientCalibrationObject {
    pub(super) name: String,
    pub(super) a2l_addr: A2lAddr,
    pub(super) get_type: A2lType,
    pub(super) a2l_limits: A2lLimits,
    pub(super) value: Vec<u8>,
}

impl XcpClientCalibrationObject {
    pub fn new(name: &str, a2l_addr: A2lAddr, get_type: A2lType, a2l_limits: A2lLimits) -> XcpClientCalibrationObject {
        XcpClientCalibrationObject {
            name: name.to_string(),
            a2l_addr,
            get_type,
            a2l_limits,
            value: Vec::new(),
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }
    pub fn get_a2l_type(&self) -> A2lType {
        self.get_type
    }
    pub fn get_a2l_addr(&self) -> A2lAddr {
        self.a2l_addr
    }
    pub fn set_value(&mut self, bytes: &[u8]) {
        self.value = bytes.to_vec();
    }
    pub fn get_value(&mut self) -> &[u8] {
        &self.value
    }
    pub fn get_value_u64(&self) -> u64 {
        let mut value = 0u64;
        for i in (0..self.get_type.size).rev() {
            value <<= 8;
            value += self.value[i] as u64;
        }
        value
    }
    pub fn get_value_i64(&self) -> i64 {
        let size = self.get_type.size;
        let mut value: i64 = if self.value[size - 1] & 0x80 != 0 { -1 } else { 0 };
        for i in (0..size).rev() {
            value <<= 8;
            assert!(value & 0xFF == 0);
            value |= self.value[i] as i64;
        }
        value
    }
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
// Measurement object

#[derive(Debug, Copy, Clone)]
pub struct XcpMeasurementObjectHandle(pub usize);

impl XcpMeasurementObjectHandle {
    pub fn get_name(self, xcp_client: &mut XcpClient) -> &str {
        xcp_client.get_measurement_object(self).get_name()
    }
    pub fn get_a2l_addr(self, xcp_client: &mut XcpClient) -> A2lAddr {
        xcp_client.get_measurement_object(self).get_a2l_addr()
    }
    pub fn get_a2l_type(self, xcp_client: &mut XcpClient) -> A2lType {
        xcp_client.get_measurement_object(self).get_a2l_type()
    }
}

#[derive(Debug, Clone)]
pub struct XcpClientMeasurementObject {
    pub(super) name: String,
    pub a2l_addr: A2lAddr,
    pub a2l_type: A2lType,
    pub daq: u16,
    pub odt: u8,
    pub offset: u16,
}

impl XcpClientMeasurementObject {
    pub fn new(name: &str, a2l_addr: A2lAddr, a2l_type: A2lType) -> XcpClientMeasurementObject {
        XcpClientMeasurementObject {
            name: name.to_string(),
            a2l_addr,
            a2l_type,
            daq: 0,
            odt: 0,
            offset: 0,
        }
    }
    pub fn get_name(&self) -> &str {
        &self.name
    }
    pub fn get_a2l_addr(&self) -> A2lAddr {
        self.a2l_addr
    }
    pub fn get_a2l_type(&self) -> A2lType {
        self.a2l_type
    }
}
