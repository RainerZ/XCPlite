//-----------------------------------------------------------------------------
// Module session::description
// The description of the target: the registry with events, calibration segments, measurement and calibration objects
//
// Session::load_description gets it from one of three sources:
//   - Upload of the A2L file from the XCP server
//   - Creation of an A2L file from the event and segment information of the XCP server and from an ELF file
//   - A local A2L file, checked against the EPK and the event and segment information of the XCP server, optionally corrected

use std::collections::HashMap;
use std::error::Error;
use std::net::Ipv4Addr;

#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};
use xcp_registry::{McA2mlMode, McEvent, Registry};

use super::{DefaultEvent, Session};
use crate::a2l_fix;
use crate::elf_reader::{ElfFilter, ElfReader};

//------------------------------------------------------------------------
// DescriptionOptions

/// Where the description of the target comes from and how it is created, see the command line options of the same names
#[derive(Debug, Clone)]
pub struct DescriptionOptions {
    /// Name of the A2L file, overrides the name from the XCP server
    pub a2l: String,
    /// Upload the A2L file from the XCP server
    pub upload_a2l: bool,
    /// Create an A2L file from XCP server information and the ELF file
    pub create_a2l: bool,
    /// Create an A2L file with events and calibration segments only
    pub create_a2l_template: bool,
    /// Correct the event ids and calibration segment numbers of the A2L file with XCP server information
    pub fix_a2l: bool,
    /// Embed the A2ML description into a created A2L file
    pub embed_aml: bool,
    /// Name of the ELF file
    pub elf: String,
    /// Upload the ELF file from the XCP server
    pub upload_elf: bool,
    /// Parse only compilation units >= n
    pub elf_unit_limit_min: usize,
    /// Parse only compilation units <= n
    pub elf_unit_limit: usize,
    /// Regular expression to filter the variable names
    pub elf_var_filter: String,
    /// Regular expression to filter the compilation unit names
    pub elf_unit_filter: String,
    /// Skip variables without metadata
    pub elf_skip_no_metadata: bool,
    /// Event for variables without a fixed event
    pub default_event: Option<DefaultEvent>,
    /// Verbosity of the ELF reader
    pub verbose: usize,
}

impl Default for DescriptionOptions {
    fn default() -> Self {
        DescriptionOptions {
            a2l: String::new(),
            upload_a2l: false,
            create_a2l: false,
            create_a2l_template: false,
            fix_a2l: false,
            embed_aml: false,
            elf: String::new(),
            upload_elf: false,
            elf_unit_limit_min: 0,
            elf_unit_limit: usize::MAX,
            elf_var_filter: String::new(),
            elf_unit_filter: String::new(),
            elf_skip_no_metadata: false,
            default_event: None,
            verbose: 0,
        }
    }
}

//------------------------------------------------------------------------
// Session

impl Session {
    /// Load the description of the target into the XCP client: upload the A2L file, create it or load it from a file
    /// Works connected and, for the creation from an ELF file and the load from a file, not connected
    /// confirm_epk_mismatch is called when the EPK of a loaded A2L file differs from the EPK of the target, an error aborts
    pub async fn load_description<F>(&mut self, options: &DescriptionOptions, confirm_epk_mismatch: F) -> Result<(), Box<dyn Error>>
    where
        F: Fn() -> Result<(), Box<dyn Error>>,
    {
        // Default event id, resolved from the default event once the registry event list is complete
        let mut default_event_id: Option<u16> = None;

        //----------------------------------------------------------------
        // Create a new empty A2L registry
        let mut reg = Registry::new();
        if !self.ecu_name.is_empty() {
            reg.application.set_info(self.ecu_name.clone(), "-", 0);
        }
        reg.set_flatten_typedefs_mode(false);
        reg.set_prefix_names_mode(false);
        reg.set_a2ml_mode(if options.embed_aml { McA2mlMode::Embed } else { McA2mlMode::Include });

        // Set A2L file path to the given A2L file name or to GET_ID IDT_ASAM_NAME or IDT_ASCII
        let a2l_filename = options.a2l.clone();
        let (a2l_path, a2l_asam_name) = if !a2l_filename.is_empty() {
            log::info!("Using A2L file name from command line argument: {}", a2l_filename);
            let a2l_filename = if !a2l_filename.ends_with(".a2l") {
                // Add .a2l extension
                format!("{}.a2l", a2l_filename)
            } else {
                a2l_filename
            };
            (std::path::Path::new(&a2l_filename).to_path_buf(), false)
        } else if !self.a2l_name.is_empty() {
            log::info!("Using A2L file name from XCP server GET_ID ASAM_NAME: {}", self.a2l_name);
            let a2l_filename = format!("{}.a2l", self.a2l_name);
            (std::path::Path::new(&a2l_filename).to_path_buf(), true) // from GET_ID ASAM_NAME
        } else if !self.ecu_name.is_empty() {
            log::info!("Using A2L file name from XCP server GET_ID ASCII: {}", self.ecu_name);
            let a2l_filename = format!("{}.a2l", self.ecu_name);
            (std::path::Path::new(&a2l_filename).to_path_buf(), false) // from GET_ID ASCII
        } else {
            return Err("^No A2L file name specified, use --a2l commandline parameter".into());
        };
        self.a2l_path = a2l_path;
        info!("A2L path: {}", self.a2l_path.display());

        //----------------------------------------------------------------
        // Upload A2L
        // From XCP server and load it into the registry
        if options.upload_a2l {
            self.upload_a2l(&mut reg, a2l_asam_name).await?;
        }
        //
        //----------------------------------------------------------------
        // Create A2L file
        // If an ELF file is available, create an A2L file from the XCP server information and the ELF file
        // If option create_a2l is specified and no ELF file, create a A2L template from XCP server information
        else if options.create_a2l || options.create_a2l_template || !options.elf.is_empty() || options.upload_elf {
            default_event_id = self.create_a2l(&mut reg, options).await?;
        }
        //----------------------------------------------------------------
        // Load A2L from local file
        // If not upload or create option load A2L from specified file into reg
        else {
            self.load_a2l(&mut reg, options, &confirm_epk_mismatch).await?;
        }

        // Resolve the default event against the final event list, if not done yet, and use it for the measurement of variables without a fixed event
        if let Some(event) = &options.default_event
            && default_event_id.is_none()
        {
            default_event_id = Some(event.resolve(&reg)?);
        }
        if let (Some(DefaultEvent::Name(name)), Some(id)) = (&options.default_event, default_event_id) {
            info!("Default event '{}' resolved to event id {}", name, id);
        }
        self.xcp_client.set_default_event(default_event_id);

        // Assign the new registry to xcp_client
        self.xcp_client.set_registry(reg);

        // Check the status of all calibration segments and goto working page
        if self.xcp_client.is_connected() {
            self.xcp_client.init_calibration_segments().await?;
        }

        Ok(())
    }

    //------------------------------------------------------------------------
    // Upload the A2L file from the XCP server and load it into the registry

    async fn upload_a2l(&mut self, reg: &mut Registry, a2l_asam_name: bool) -> Result<(), Box<dyn Error>> {
        // Don't overwrite existing A2L file generated by a XCP demo running in the same directory
        if self.a2l_path.exists() && a2l_asam_name && self.a2l_path == std::path::Path::new(&self.a2l_name).with_extension("a2l") {
            // Generate a different file name for the uploaded A2L file
            let new_a2l_path = self
                .a2l_path
                .with_file_name(format!("{}_upload", self.a2l_path.file_stem().unwrap_or_default().to_string_lossy()))
                .with_extension("a2l");
            warn!(
                "The A2L file '{}' already exists and matches the XCP server A2L file name (from GET_ID ASAM_NAME), uploading to {}",
                self.a2l_path.display(),
                new_a2l_path.display()
            );
            self.a2l_path = new_a2l_path;
        }

        // Upload A2L file
        let res = self.xcp_client.upload_a2l_into_registry(&self.a2l_path, reg).await;
        if let Err(e) = res {
            error!("A2L upload failed, Error: {}", e);
            return Err("A2L upload failed".into());
        }
        Ok(())
    }

    //------------------------------------------------------------------------
    // Create an A2L file
    // Read segment and event information obtained from the XCP server into registry
    // Add measurement and calibration variables from ELF file if specified
    // Addressing scheme may be XCPLITE__ACSDD or XCPLITE__CASDD depending on the target configuration
    // Returns the default event id, if it was resolved for the registration of the variables

    async fn create_a2l(&mut self, reg: &mut Registry, options: &DescriptionOptions) -> Result<Option<u16>, Box<dyn Error>> {
        let elf_filename = &options.elf;
        let upload_elf = options.upload_elf;
        let create_a2l_template = options.create_a2l_template;
        let verbose = options.verbose;
        let mut default_event_id: Option<u16> = None;

        let mode = if create_a2l_template {
            if !elf_filename.is_empty() || upload_elf {
                "target XCP event/segment and ELF/DWARF event/segment information (template, no variables), online mode"
            } else {
                "target XCP event/segment information only (template, no variables), online mode"
            }
        } else if self.xcp_client.is_connected() {
            if !elf_filename.is_empty() || upload_elf {
                "target XCP event/segment and ELF/DWARF variable and type information, online mode"
            } else {
                "target XCP event/segment information only, online mode"
            }
        } else {
            "ELF/DWARF information only,  offline mode"
        };
        info!("Generate A2L file {} with {} ", self.a2l_path.display(), mode);

        // Set registry XCP transport layer information for A2L file
        // TCP or UDP: set IP address and port number
        if self.protocol == "TCP" || self.protocol == "UDP" {
            let addr = self.dest_addr.ip();
            let ipv4_addr = match addr {
                std::net::IpAddr::V4(v4) => v4,
                std::net::IpAddr::V6(_) => Ipv4Addr::new(127, 0, 0, 1),
            };
            let port: u16 = self.dest_addr.port();
            reg.set_xcp_eth_params(self.protocol, ipv4_addr, port);
        }
        // SxI protocol: set baud rate
        else if self.protocol == "SxI" {
            reg.set_xcp_sxi_params(self.baud_rate);
        }

        // If there is an ECU online, get event and segment information via XCP
        if self.xcp_client.is_connected() {
            self.xcp_client.get_event_segment_info(reg).await?;
        }

        // Read ELF/DWARF information for events, segments and (unless a template is created) variables
        // Events and calibration segments found in the ELF file, must match the XCP server information if present
        // If not, they are created, but with dummy event id and segment number, which has to be fixed later !!!
        if !elf_filename.is_empty() || upload_elf {
            // Upload ELF file
            if upload_elf {
                if elf_filename.is_empty() {
                    error!("ELF file name must be specified with --elf when using --upload-elf");
                    return Err("ELF file name not specified".into());
                }
                info!("Upload ELF file: {}", elf_filename);
                let res = self.xcp_client.upload_elf_file(elf_filename).await;
                if let Err(e) = res {
                    error!("ELF upload failed, Error: {}", e);
                    return Err("ELF upload failed".into());
                }
            }

            // The compilation unit and variable name selection, built and validated once here and then carried by the ELF reader
            let elf_filter = ElfFilter::new(options.elf_unit_limit_min, options.elf_unit_limit, &options.elf_unit_filter, &options.elf_var_filter)?;

            // Read ELF file and DWARF debug information, compilation unit number may be limited to reduce processing time and memory needed
            info!("Reading ELF file: {}", elf_filename);
            let elf_reader = ElfReader::new(elf_filename, verbose, elf_filter).map_err(|e| format!("Failed to read ELF file '{}': {}", elf_filename, e))?;
            elf_reader.log_compilers();

            if verbose > 0 {
                elf_reader.debug_data.print_debug_info(verbose);
            }

            // Detect addressing scheme for calibration segments
            // Get target signature (CASDD, ACSDD, ...) from ELF file if available
            // true - addr_ext==0 is segment relative addressing, addr_ext==1 is absolute addressing
            // false - addr_ext==0 is absolute addressing, addr_ext==1 is segment relative addressing
            info!("===============================================================");
            self.segment_relative = {
                let signature = elf_reader.get_target_signature();
                if let Some(sig) = signature {
                    info!("Target signature found: {}", sig);
                    if sig.starts_with("C") { true } else { false }
                } else {
                    warn!("No target signature found in ELF file");
                    false
                }
            };
            let segment_relative = self.segment_relative;
            info!(
                "Using {} addressing for calibration segments",
                if segment_relative { "segment relative" } else { "absolute" }
            );

            // Register events from event creation markers (evt__name) in the code
            elf_reader.register_events(reg, verbose)?;

            // Register segments from segment creation markers (calseg__name) found in the code
            elf_reader.register_segments(reg, segment_relative, verbose)?;

            // Get the EPK string from debug_data and set it in the registry application version information, if available
            // After the segments, ADDR_EPK depends on the addressing mode and the epk segment
            elf_reader.register_epk_addr_info(reg, segment_relative, verbose);

            // Find event triggers in the code by their trigger markers (trg__mode__name) and register their location (compilation unit, function, CFA offset)
            elf_reader.register_event_locations(reg, verbose)?;

            // Register all accessible variables and their types
            // Skipped when a template is created; events and segments are still registered above
            if !create_a2l_template {
                // The event list is complete now (XCP server and ELF file), resolve the default event name to its id
                if let Some(event) = &options.default_event {
                    default_event_id = Some(event.resolve(reg)?);
                }
                elf_reader.register_variables(reg, segment_relative, verbose, default_event_id)?;
                // Register the captured local variables of the event triggers (DaqTriggerEventCapture)
                elf_reader.register_captures(reg, verbose)?;
                // Apply metadata (XCP_UNIT / XCP_LIMITS / XCP_COMMENT) from the xcp_meta ELF section
                elf_reader.register_metadata(reg, verbose)?;
                // Optionally remove all variables without any metadata (XCP_UNIT / XCP_LIMITS / XCP_COMMENT) from the registry
                if options.elf_skip_no_metadata {
                    let before = reg.instance_list.len();
                    reg.instance_list.retain(|inst| inst.get_mc_support_data().has_metadata());
                    let removed = before - reg.instance_list.len();
                    if verbose > 0 || removed > 0 {
                        info!("Removed {} variables without metadata ({} remaining)", removed, reg.instance_list.len());
                    }
                }
            }
        }

        // Write the registry to A2L file
        if !self.a2l_path.as_os_str().is_empty() {
            info!("===============================================================");
            if self.a2l_path.exists() {
                warn!("Overwriting existing A2L file: {}", self.a2l_path.display());
            }
            if self.ecu_name.is_empty() {
                self.ecu_name = "project_name".into();
            }
            let title_info = format!("Created by xcp_client with {} - {}", mode, chrono::Utc::now().format("%Y-%m-%d %H:%M:%S"));
            reg.write_a2l(
                &self.a2l_path,
                title_info.as_str(),
                &self.ecu_name,
                "",
                &self.ecu_name,
                if self.segment_relative { "XCPLITE__CASDD" } else { "XCPLITE__ACSDD" },
                true,
            )
            .map_err(|e| format!("Could not write A2L file '{}': {}", self.a2l_path.display(), e))?;
            info!("Created A2L with file: {} {}", self.a2l_path.display(), mode);
        }

        // The A2L file keeps the typedef structure, but xcp_client supports only basic type instances (see load_a2l_file_into_registry):
        // flatten the registry like loading an A2L file does, so listing, calibration and measurement work the same as with a loaded A2L file
        reg.flatten_typedefs();

        Ok(default_event_id)
    }

    //------------------------------------------------------------------------
    // Load the A2L file into the registry
    // Check the EPK and the event and calibration segment information against the XCP server
    // If fix_a2l is specified, correct the A2L file with the XCP server information, otherwise just warn about differences

    async fn load_a2l<F>(&mut self, reg: &mut Registry, options: &DescriptionOptions, confirm_epk_mismatch: &F) -> Result<(), Box<dyn Error>>
    where
        F: Fn() -> Result<(), Box<dyn Error>>,
    {
        info!("===============================================================");
        info!("Load A2L file: {}", self.a2l_path.display());
        self.xcp_client
            .load_a2l_file_into_registry(&self.a2l_path, reg)
            .map_err(|e| format!("Could not load A2L file '{}': {}", self.a2l_path.display(), e))?;

        // Check the A2L EPK against the EPK reported by the target.
        // The EPK is the firmware version string the A2L was generated from,
        // so a mismatch means this A2L describes different firmware and its
        // addresses may point at the wrong variables.
        if !self.ecu_epk.is_empty() {
            let a2l_epk = reg.application.get_version();
            if a2l_epk.is_empty() {
                warn!("A2L file {} has no EPK, cannot verify it matches the target", self.a2l_path.display());
            } else if a2l_epk != self.ecu_epk {
                error!(
                    "EPK mismatch: A2L file '{}' has EPK '{}', target reports EPK '{}'. Addresses may be wrong.",
                    self.a2l_path.display(),
                    a2l_epk,
                    self.ecu_epk
                );
                confirm_epk_mismatch()?;
            } else {
                info!("EPK '{}' of A2L file and target match", self.ecu_epk);
            }
        }

        let mut event_mapping: HashMap<u16, u16> = HashMap::new();
        let mut seg_mapping: HashMap<u16, u16> = HashMap::new();
        let mut missing_event_ids = 0;

        // Load the event and calibration segment information from target into a temporary registry and check if the given A2L file needs to be corrected
        let mut tmp_reg = Registry::new();
        self.xcp_client.get_event_segment_info(&mut tmp_reg).await?;

        // Check events
        if self.xcp_client.max_events() == 0 {
            warn!("XCP server does not support get event info, skipping event check");
        } else {
            for event in &tmp_reg.event_list {
                if let Some(e) = reg.event_list.find_event(event.get_name(), 0) {
                    if e.get_id() != event.get_id() {
                        warn!(
                            "Event id of '{}' differs, A2L file {} has id {}, target has id {}",
                            event.get_name(),
                            self.a2l_path.display(),
                            e.get_id(),
                            event.get_id()
                        );

                        // Create event mapping information in a hash map
                        event_mapping.insert(e.get_id(), event.get_id());
                    }
                } else {
                    warn!("Event '{}' missing in A2L file {}", event.get_name(), self.a2l_path.display());
                    missing_event_ids += 1;
                }
            }
        }

        // Check calibration segments
        if self.xcp_client.max_segments() == 0 {
            warn!("XCP server does not support get segment info, skipping calibration segment check");
        } else {
            for seg in &tmp_reg.cal_seg_list {
                if let Some(s) = reg.cal_seg_list.find_cal_seg(seg.get_name()) {
                    if s.get_index() != seg.get_index() {
                        warn!(
                            "Calibration segment index of '{}' differs, A2L file {} has index {}, target has index {}",
                            seg.get_name(),
                            self.a2l_path.display(),
                            s.get_index(),
                            seg.get_index()
                        );

                        // Create segment mapping information in a hash map
                        seg_mapping.insert(s.get_index(), seg.get_index());
                    }
                } else {
                    error!("Calibration segment '{}' missing in A2L file {}", seg.get_name(), self.a2l_path.display());
                }
            }
        }

        // Fix the registry used in this run and rewrite the A2L file
        // In the event list and calibration segment list, and in the addresses of all instances with XCPlite event relative
        // or calibration segment relative addressing, which contain the event id or segment number
        if options.fix_a2l {
            // Only an A2L file generated by xcpclient is corrected, the event ids and segment numbers of an A2L file generated offline
            // from the ELF file without section registration. Any other A2L file is rejected, it might lose information when rewritten
            let a2l_text = std::fs::read_to_string(&self.a2l_path).map_err(|e| format!("Could not read A2L file '{}': {}", self.a2l_path.display(), e))?;
            if !a2l_fix::is_created_by_xcpclient(&a2l_text) {
                return Err(format!(
                    "--fix-a2l: A2L file '{}' was not created by xcpclient, only A2L files created with --elf and --create-a2l can be corrected. \
                     Create the A2L file with --elf and --create-a2l while connected to the XCP server instead",
                    self.a2l_path.display()
                )
                .into());
            }
            if !event_mapping.is_empty() {
                info!("Event mapping information:");
                for (k, v) in &event_mapping {
                    info!("  {} -> {}", k, v);
                }
            }
            if !seg_mapping.is_empty() {
                info!("Calibration segment mapping information:");
                for (k, v) in &seg_mapping {
                    info!("  {} -> {}", k, v);
                }
            }
            let project_no = a2l_fix::get_project_no(&a2l_text);
            a2l_fix::apply_mappings(reg, &event_mapping, &seg_mapping, project_no.as_deref());

            if missing_event_ids > 0 {
                if missing_event_ids == 1 {
                    // Add a dummy event
                    match reg.event_list.add_event(McEvent::new("async", 0, 0, 0)) {
                        Ok(_) => {
                            warn!("XCPlite async event added");
                        }
                        Err(e) => warn!("Failed to add event 'async', Error: {}", e),
                    }
                } else {
                    warn!("A2L file {} is missing {} event ids, please correct manually", self.a2l_path.display(), missing_event_ids);
                }
            }

            if event_mapping.is_empty() && seg_mapping.is_empty() {
                info!("Event ids and calibration segment numbers of A2L file {} match the target", self.a2l_path.display());
            } else {
                let tl_params = match self.dest_addr.ip() {
                    std::net::IpAddr::V4(addr) if self.protocol == "TCP" || self.protocol == "UDP" => Some((self.protocol, addr, self.dest_addr.port())),
                    _ => None,
                };
                a2l_fix::rewrite_a2l_file(&self.a2l_path, &event_mapping, &seg_mapping, tl_params, options.embed_aml)
                    .map_err(|e| format!("Could not rewrite A2L file '{}': {}", self.a2l_path.display(), e))?;
            }
        } else if !event_mapping.is_empty() || !seg_mapping.is_empty() {
            warn!("A2L file {} differs from target, use automatic correction (--fix-a2l)", self.a2l_path.display());
            info!("events: {:?}", event_mapping);
            info!("segments: {:?}", seg_mapping);
        }

        Ok(())
    }
}
