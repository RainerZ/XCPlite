//-----------------------------------------------------------------------------
// Library crate xcpclient
// xcpclient is a library crate that provides an XCP on ETH client implementation for integration testing
// The command line tool xcpclient (main.rs) is built on it

// This crate is a library
#![crate_type = "lib"]
// The library crate is named "xcpclient"
#![crate_name = "xcpclient"]

// XCP client, ELF/DWARF reader, XCPlite persistence file reader
pub mod bin_reader;
pub mod elf_reader;
pub mod xcp_client;

// Correction of the event ids and calibration segment numbers of an A2L file
pub mod a2l_fix;

// Measurement data output
pub mod console_writer;
pub mod csv_writer;
pub mod daq_writer;
pub mod mcap_writer;
pub mod mdf_writer;
pub mod perfetto_writer;

// A session with an XCP server: the steps xcpclient is made of
pub mod session;
