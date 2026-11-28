//--------------------------------------------------------------------------------------------------------------------------------------------------
// a2l.rs — ELF/A2L file upload and registry population

#![allow(dead_code)]

#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};

use std::error::Error;
use std::io::Write;

use xcp_registry::McEvent;

use super::*;

/// Path of the temporary file an upload is written to: the file name with .part appended
fn partial_upload_path(path: &std::path::Path) -> std::path::PathBuf {
    let mut file_name = path.file_name().unwrap_or_default().to_os_string();
    file_name.push(".part");
    path.with_file_name(file_name)
}

impl XcpClient {
    //-------------------------------------------------------------------------------------------------
    // Upload of a file

    // Upload file_size bytes from the MTA of the XCP server into a file
    // The data is written to a temporary file, which gets its name when the upload is complete. A file with the name of the
    // uploaded file is not touched before: it may be the file the XCP server is reading for this upload (an XCPlite application
    // in the same directory), and a failed upload does not leave an empty or truncated file
    async fn upload_to_file(&mut self, path: &std::path::Path, file_size: u32) -> Result<(), Box<dyn Error>> {
        let partial_path = partial_upload_path(path);
        let result: Result<(), Box<dyn Error>> = async {
            let file = std::fs::File::create(&partial_path)?;
            let mut writer = std::io::BufWriter::new(file);
            let mut size = file_size;
            while size > 0 {
                let n = if size >= self.max_cto_size as u32 { self.max_cto_size - 1 } else { size as u8 };
                size -= n as u32;
                let data = self.upload(n).await?;
                trace!("xcp_client.upload: {} bytes = {:?}", data.len(), data);
                writer.write_all(&data[1..=n as usize])?;
            }
            writer.flush()?;
            Ok(())
        }
        .await;
        match result {
            Ok(()) => {
                std::fs::rename(&partial_path, path)?;
                Ok(())
            }
            Err(e) => {
                let _ = std::fs::remove_file(&partial_path);
                Err(e)
            }
        }
    }

    //-------------------------------------------------------------------------------------------------
    // ELF upload

    pub async fn upload_elf_file<P: AsRef<std::path::Path>>(&mut self, elf_path: &P) -> Result<(), Box<dyn Error>> {
        // Send XCP GET_ID IDT_VECTOR_ELF_UPLOAD command to set MTA
        let (file_size, _) = self.get_id(IDT_VECTOR_ELF_UPLOAD).await?;
        if file_size == 0 {
            error!("ELF file not available, GET_ID returned size 0");
            return Err(Box::new(XcpError::new(ERROR_GENERIC, CC_GET_ID)) as Box<dyn Error>);
        }

        // Check if the ELF file already exists and warn about overwriting
        if elf_path.as_ref().exists() {
            info!("ELF file {} already exists, replacing it", elf_path.as_ref().display());
        }

        // Upload the ELF file
        info!("Upload ELF to {}", elf_path.as_ref().display());
        self.upload_to_file(elf_path.as_ref(), file_size).await?;
        debug!("ELF upload completed, {} bytes loaded", file_size);

        Ok(())
    }

    //-------------------------------------------------------------------------------------------------
    // A2L upload

    pub async fn upload_a2l_file<P: AsRef<std::path::Path>>(&mut self, a2l_path: &P) -> Result<(), Box<dyn Error>> {
        // Send XCP GET_ID 4 command to set MTA
        let (file_size, _) = self.get_id(IDT_ASAM_UPLOAD).await?;
        if file_size == 0 {
            error!("A2L file not available, GET_ID 4 returned size 0");
            return Err(Box::new(XcpError::new(ERROR_GENERIC, CC_GET_ID)) as Box<dyn Error>);
        }

        // Check if the A2L file already exists and warn about overwriting
        if a2l_path.as_ref().exists() {
            info!("A2L file {} already exists, replacing it", a2l_path.as_ref().display());
        }

        // Upload the A2L file
        info!("Upload A2L to {}", a2l_path.as_ref().display());
        self.upload_to_file(a2l_path.as_ref(), file_size).await?;
        debug!("A2L upload completed, {} bytes loaded", file_size);

        Ok(())
    }

    // Get the A2L via XCP upload and GET_ID4 (IDT_ASAM_UPLOAD) and load it into the registry
    pub async fn upload_a2l_into_registry<P: AsRef<std::path::Path>>(&mut self, a2l_path: &P, reg: &mut xcp_registry::Registry) -> Result<(), Box<dyn Error>> {
        // Upload the A2L file
        self.upload_a2l_file(&a2l_path).await?;

        // Load the A2L file into the registry
        // @@@@ TODO xcp_client does not support arrays, instances and typedefs yet, flatten the registry and mangle the names
        reg.load_a2l(&a2l_path, true, true, true, true)?;
        info!(
            " A2L file contains {} instances, {} events and {} calibration segments",
            reg.instance_list.len(),
            reg.event_list.len(),
            reg.cal_seg_list.len()
        );
        Ok(())
    }

    // Get the A2L via XCP upload and GET_ID4 (IDT_ASAM_UPLOAD) and load it into the registry
    pub fn load_a2l_file_into_registry<P: AsRef<std::path::Path>>(&mut self, a2l_path: &P, reg: &mut xcp_registry::Registry) -> Result<(), Box<dyn Error>> {
        // Load the A2L file into the registry
        // @@@@ TODO xcp_client does not support arrays, instances and typedefs yet, flatten the registry and mangle the names
        reg.load_a2l(&a2l_path, true, true, true, true)?;
        info!(
            " A2L file contains {} instances, {} events and {} calibration segments",
            reg.instance_list.len(),
            reg.event_list.len(),
            reg.cal_seg_list.len()
        );
        Ok(())
    }

    pub fn get_epk(&self) -> Option<&str> {
        self.registry.as_ref().map(|r| r.application.get_version())
    }

    //------------------------------------------------------------------------
    // Get event and segment information from XCP server and add to registry

    pub async fn get_event_segment_info(&mut self, reg: &mut xcp_registry::Registry) -> Result<(), Box<dyn Error>> {
        info!("Reading event and segment information from connected XCP server:");

        // Get event information
        for i in 0..self.max_events {
            let name = self.get_daq_event_info(i).await?;
            info!(" Event {}: {}", i, name);
            reg.event_list
                .add_event(McEvent::new(name, 0, i, 0))
                .map_err(|e| format!("Event {} of the XCP server can not be registered: {}", i, e))?;
        }

        // Get segment information
        for i in 0..self.max_segments {
            let (addr_ext, addr, length, name) = self.get_segment_info(i).await?;
            info!(" Segment {}: {} addr={}:0x{:08X} length={} ", i, name, addr_ext, addr, length);

            // Segment relative addressing is ignored, all addresses are treated as raw A2L addr_ext/addr
            reg.cal_seg_list
                .add_cal_seg_by_addr(name, Some(i), addr_ext, addr, length as u32)
                .map_err(|e| format!("Calibration segment {} of the XCP server can not be registered: {}", i, e))?;
        }

        Ok(())
    }

    //------------------------------------------------------------------------
    // Registry
    // Get a list of available measurement and calibration object names from registry matching a regular expression

    pub fn get_registry(&self) -> &xcp_registry::Registry {
        self.registry.as_ref().unwrap()
    }

    /// Number of measurable objects and of calibration objects in a registry
    pub fn count_objects(reg: &xcp_registry::Registry) -> (usize, usize) {
        let measurements = reg.instance_list.into_iter().filter(|i| daq::is_measurable(i)).count();
        let parameters = reg
            .instance_list
            .into_iter()
            .filter(|i| i.object_type() == xcp_registry::McObjectType::Characteristic)
            .count();
        (measurements, parameters)
    }

    /// The registry, an error if no description of the target is loaded
    pub(super) fn registry(&self) -> Result<&xcp_registry::Registry, Box<dyn Error>> {
        self.registry.as_ref().ok_or_else(|| "No A2L description of the target loaded".into())
    }

    /// Names of the calibration objects matching a regular expression, none if no description of the target is loaded
    pub fn find_characteristics(&self, expr: &str) -> Vec<String> {
        let Some(registry) = self.registry.as_ref() else {
            return Vec::new();
        };
        registry.instance_list.find_instances_regex(expr, xcp_registry::McObjectType::Characteristic, None)
    }

    /// Names of the measurable instances matching a regular expression: MEASUREMENTs and CHARACTERISTICs with an event, see daq::is_measurable
    /// Arrays are included, an invalid regular expression matches nothing
    pub fn find_measurements(&self, expr: &str) -> Vec<String> {
        let Some(registry) = self.registry.as_ref() else {
            return Vec::new();
        };
        let Ok(regex) = regex::Regex::new(expr) else {
            return Vec::new();
        };
        registry
            .instance_list
            .into_iter()
            .filter(|i| daq::is_measurable(i) && regex.is_match(i.name.as_str()))
            .map(|i| i.name.to_string())
            .collect()
    }
}

//--------------------------------------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // The temporary file of an upload is next to the file, the name of the file is kept with its dots
    #[test]
    fn test_partial_upload_path() {
        assert_eq!(partial_upload_path(Path::new("hello_xcp_V2.3.a2l")), Path::new("hello_xcp_V2.3.a2l.part"));
        assert_eq!(partial_upload_path(Path::new("dir.d/app.elf")), Path::new("dir.d/app.elf.part"));
    }

    // An upload which fails does not touch an existing file with the name of the uploaded file and leaves no temporary file
    #[tokio::test]
    async fn test_failed_upload_keeps_existing_file() {
        let dir = std::env::temp_dir().join(format!("xcpclient-test-upload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hello_xcp_V2.3.a2l");
        std::fs::write(&path, "content").unwrap();

        // Not connected, the first UPLOAD command fails
        let addr: SocketAddr = "127.0.0.1:5555".parse().unwrap();
        let mut client = XcpClient::new("UDP", addr, addr, 0);
        client.max_cto_size = 248;
        assert!(client.upload_to_file(&path, 1000).await.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "content");
        assert!(!partial_upload_path(&path).exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
