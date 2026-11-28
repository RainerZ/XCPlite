//-----------------------------------------------------------------------------
// Module session::actor::files
// The file operations of the session actor: A2L files and parameter files
//
//   load A2L          Load or replace the description of the connected target: from an A2L file, created from an ELF file,
//                     or uploaded from the target
//   create A2L        Create an A2L file from an ELF file, without using it: offline, or with the event and segment
//                     information of the connected target
//   save parameters   Upload the calibration segments of the target into a file (.hex or .bin)
//   load parameters   Download a file (.hex) into the calibration segments of the target
//
// Files the user names are relative to the working directory. A file is not replaced without being told so: a parameter file
// is never overwritten, an A2L file only with overwrite.
// The warnings and errors logged while an operation runs are appended to its result (module log_capture): what xcpclient
// finds in an A2L or ELF file is reported as log messages.

use std::path::Path;

#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};

use super::{NOT_CONNECTED, SessionActor, TransportProtocol, format_size};
use crate::log_capture;
use crate::session::{DefaultEvent, DescriptionInfo, DescriptionOptions, DescriptionSource, EpkCheck, Session, a2l_file_path, parse_dest_addr};

//------------------------------------------------------------------------
// Requests

/// Load or replace the description of the connected target
/// A2L file given: load it. ELF file given: create the A2L file from it. Neither: the default of the session, else upload
#[derive(Debug, Clone, Default)]
pub struct LoadA2lRequest {
    pub a2l: Option<String>,
    pub elf: Option<String>,
    /// Correct the event ids and calibration segment numbers of the A2L file with those of the target and rewrite it
    pub fix: bool,
    /// Event id or name for variables without a fixed event
    pub default_event: Option<String>,
    /// Use a file with an EPK which differs from the EPK of the target
    pub accept_epk_mismatch: bool,
}

/// Create an A2L file from an ELF file
#[derive(Debug, Clone, Default)]
pub struct CreateA2lRequest {
    pub elf: String,
    /// Name of the A2L file to write
    pub a2l: String,
    /// Events and calibration segments only, no variables
    pub template: bool,
    /// Embed the A2ML description instead of including XCP_104.aml
    pub embed_aml: bool,
    /// Regular expressions to select variables by name and by compilation unit
    pub var_filter: Option<String>,
    pub unit_filter: Option<String>,
    /// Only variables with metadata (unit, limits, comment)
    pub skip_no_metadata: bool,
    /// Event id or name for variables without a fixed event
    pub default_event: Option<String>,
    /// Do not use the event and segment information of the connected target
    pub offline: bool,
    /// Use an ELF file with an EPK which differs from the EPK of the connected target
    pub accept_epk_mismatch: bool,
    /// Replace an existing A2L file
    pub overwrite: bool,
}

//------------------------------------------------------------------------
// Texts

// The captured warnings as lines of a result, empty if there are none
pub(super) fn warnings_text(warnings: &[String]) -> String {
    if warnings.is_empty() { String::new() } else { format!("\n{}", warnings.join("\n")) }
}

// The captured messages which belong into a result
// Not those which say what the tool was asked to do. With the error of a failed operation: not the error itself, it is the
// message of the result already
fn relevant_warnings(warnings: Vec<String>, error: Option<&str>) -> Vec<String> {
    warnings
        .into_iter()
        .filter(|w| !w.starts_with("Warning: Overwriting existing A2L file"))
        .filter(|w| error.is_none_or(|e| !w.ends_with(e) && !w.starts_with("Error: EPK mismatch")))
        .collect()
}

// An A2L file of XCPlite includes the A2ML description of XCP by name, it has to be next to the A2L file
// Write it into the directory of the A2L file, if it is not there. Returns a note for the result if it was written
fn provide_aml(a2l_path: &Path) -> Option<String> {
    let directory = a2l_path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let aml_path = directory.join(super::XCP_104_AML_NAME);
    if aml_path.exists() {
        return None;
    }
    match std::fs::write(&aml_path, super::XCP_104_AML) {
        Ok(()) => {
            info!("{} written, it is included by A2L files of XCPlite", aml_path.display());
            Some(format!("{} written, the A2L file includes it.", aml_path.display()))
        }
        Err(e) => {
            warn!("Could not write {}: {}", aml_path.display(), e);
            None
        }
    }
}

// What the EPK check of a created A2L file found, as a sentence, empty if there was nothing to compare
fn epk_check_text(info: &DescriptionInfo) -> &'static str {
    match info.epk_check {
        EpkCheck::Match => " The EPK of the ELF file matches the target.",
        EpkCheck::MismatchAccepted => " The EPK of the ELF file DIFFERS from the EPK of the target (accepted), addresses may be wrong for this target.",
        EpkCheck::NoEpkInA2l => " The ELF file has no EPK, it could not be compared with the target.",
        EpkCheck::NotChecked => "",
    }
}

// A file name with one of the given extensions, compared without case
fn has_extension(file: &str, extensions: &[&str]) -> bool {
    Path::new(file)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| extensions.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

impl SessionActor {
    //------------------------------------------------------------------------
    // Description of the connected target

    // The options to load the description, from the request and the defaults
    // A2L file given: load it. ELF file given: create the A2L file from it. Neither: upload the A2L file from the target
    pub(super) fn description_options(&self, request: &LoadA2lRequest) -> Result<DescriptionOptions, String> {
        let default_event = match &request.default_event {
            Some(event) => Some(event.parse::<DefaultEvent>()?),
            None => self.defaults.default_event.clone(),
        };
        let not_empty = |s: &String| !s.is_empty();
        let explicit = request.a2l.is_some() || request.elf.is_some();
        let a2l = request
            .a2l
            .clone()
            .or_else(|| if explicit { None } else { Some(self.defaults.a2l.clone()) })
            .filter(not_empty);
        let elf = request
            .elf
            .clone()
            .or_else(|| if explicit { None } else { Some(self.defaults.elf.clone()) })
            .filter(not_empty);
        // An A2L file which gets the name of the target is written to the directory of the session, not to the working directory
        let mut options = DescriptionOptions {
            default_event,
            a2l_dir: self.defaults.file_dir.to_string_lossy().into_owned(),
            ..Default::default()
        };
        if let Some(elf) = elf {
            options.elf = elf;
            options.create_a2l = true;
            options.a2l = a2l.unwrap_or_default();
        } else if let Some(a2l) = a2l {
            options.a2l = a2l;
            options.fix_a2l = request.fix;
        } else {
            options.upload_a2l = true;
        }
        Ok(options)
    }

    // The decision about an EPK mismatch between a file and the target: accepted by the request, or an error which says how
    // to accept it. file is what has the other EPK ("A2L file", "ELF file"), tool is the tool to call again
    fn confirm_epk_mismatch(accept: bool, file: &'static str, tool: &'static str) -> impl Fn(&str, &str) -> Result<(), Box<dyn std::error::Error>> {
        move |file_epk: &str, ecu_epk: &str| {
            if accept {
                Ok(())
            } else {
                Err(format!(
                    "EPK mismatch: the {} has EPK '{}', the target reports EPK '{}'. It describes another software version, addresses may be wrong. \
                     Ask the user. To use this file anyway, call {} again with accept_epk_mismatch = true",
                    file, file_epk, ecu_epk, tool
                )
                .into())
            }
        }
    }

    // Load the description into the connected session
    // Returns the warnings logged on the way
    // If it fails, the connection is checked: the error is then the one of the lost connection
    pub(super) async fn load_description(&mut self, request: &LoadA2lRequest, tool: &'static str) -> Result<Vec<String>, String> {
        let options = self.description_options(request)?;
        if options.upload_a2l || options.create_a2l {
            // The A2L file is written: with the name of the target into the directory of the session, or to the given name
            let file_dir = &self.defaults.file_dir;
            std::fs::create_dir_all(file_dir).map_err(|e| format!("Could not create the directory '{}' for the A2L file: {}", file_dir.display(), e))?;
            if options.a2l.is_empty() {
                provide_aml(&file_dir.join("a2l"));
            } else {
                provide_aml(&a2l_file_path(&options.a2l));
            }
        }
        let confirm = Self::confirm_epk_mismatch(request.accept_epk_mismatch, if options.create_a2l { "ELF file" } else { "A2L file" }, tool);
        let Some(session) = self.session.as_mut() else {
            return Err(NOT_CONNECTED.to_string());
        };
        log_capture::begin();
        let result = session.load_description(&options, confirm).await;
        let warnings = log_capture::end();
        match result {
            Ok(()) => Ok(relevant_warnings(warnings, None)),
            Err(e) => {
                let e = e.to_string();
                // The connection may be the reason
                if !session.check_connection().await {
                    return Err(self.connection_lost(&e));
                }
                // The warnings before the error explain it
                let warnings = relevant_warnings(warnings, Some(&e));
                Err(format!(
                    "The description of the target could not be loaded: {}{}\n\nCurrent state:\n{}",
                    e,
                    warnings_text(&warnings),
                    self.status()
                ))
            }
        }
    }

    pub(super) async fn load_a2l(&mut self, request: LoadA2lRequest) -> Result<String, String> {
        if self.session.is_none() {
            return Err(NOT_CONNECTED.to_string());
        }
        if self.recording.is_some() {
            return Err("A measurement is running with the loaded description. Stop it with xcp_measure_stop first.".to_string());
        }
        let warnings = self.load_description(&request, "xcp_load_a2l").await?;
        Ok(format!("Description loaded.\n{}{}", self.status(), warnings_text(&warnings)))
    }

    //------------------------------------------------------------------------
    // Create an A2L file

    pub(super) async fn create_a2l(&mut self, request: CreateA2lRequest) -> Result<String, String> {
        if request.elf.is_empty() || request.a2l.is_empty() {
            return Err("Give the ELF file (elf) and the name of the A2L file to write (a2l)".to_string());
        }
        if !Path::new(&request.elf).is_file() {
            let directory = std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default();
            return Err(format!(
                "ELF file '{}' not found. File names are relative to the working directory {}",
                request.elf, directory
            ));
        }
        let a2l_path = a2l_file_path(&request.a2l);
        if a2l_path.exists() && !request.overwrite {
            return Err(format!(
                "The file '{}' exists. To replace it, call xcp_create_a2l again with overwrite = true.",
                a2l_path.display()
            ));
        }
        let default_event = match &request.default_event {
            Some(event) => Some(event.parse::<DefaultEvent>()?),
            None => self.defaults.default_event.clone(),
        };
        let options = DescriptionOptions {
            a2l: request.a2l.clone(),
            elf: request.elf.clone(),
            create_a2l: !request.template,
            create_a2l_template: request.template,
            embed_aml: request.embed_aml,
            elf_var_filter: request.var_filter.clone().unwrap_or_default(),
            elf_unit_filter: request.unit_filter.clone().unwrap_or_default(),
            elf_skip_no_metadata: request.skip_no_metadata,
            default_event,
            ..Default::default()
        };

        // The A2ML description the A2L file includes, if it is not embedded
        let aml_note = if request.embed_aml { None } else { provide_aml(&a2l_path) };

        // With the event and segment information of the connected target, or offline
        let online = self.session.is_some() && !request.offline;
        log_capture::begin();
        let result = match self.session.as_mut().filter(|_| online) {
            Some(session) => {
                let confirm = Self::confirm_epk_mismatch(request.accept_epk_mismatch, "ELF file", "xcp_create_a2l");
                session.create_a2l_file(&options, confirm).await
            }
            None => {
                // A session which is not connected, its target address is written to the A2L file as the address of the XCP server
                let protocol = self.defaults.protocol.unwrap_or(TransportProtocol::Udp);
                let dest_addr = parse_dest_addr(&self.defaults.dest_addr, self.defaults.port).unwrap_or_else(|_| ([127, 0, 0, 1], self.defaults.port).into());
                let mut offline = Session::new(protocol.as_str(), dest_addr, self.defaults.bind_addr, 0);
                offline.create_a2l_file(&options, |_: &str, _: &str| Ok(())).await
            }
        };
        let warnings = log_capture::end();

        match result {
            Ok(info) => {
                let how = match (&self.session, online) {
                    (Some(session), true) => format!("with the events and calibration segments of {}", session.target_info().name_and_address()),
                    _ => "offline, from the ELF file only".to_string(),
                };
                debug_assert!(info.source == DescriptionSource::Created);
                Ok(format!(
                    "A2L file {} created from {} ({}){}: {} events, {} calibration segments, {} signals, {} parameters.{}{}{}",
                    info.a2l_path.display(),
                    request.elf,
                    how,
                    if request.template { ", template without variables" } else { "" },
                    info.events,
                    info.segments,
                    info.measurements,
                    info.parameters,
                    epk_check_text(&info),
                    aml_note.map(|note| format!(" {}", note)).unwrap_or_default(),
                    warnings_text(&relevant_warnings(warnings, None))
                ))
            }
            Err(e) => {
                let e = e.to_string();
                if online
                    && let Some(session) = self.session.as_mut()
                    && !session.check_connection().await
                {
                    return Err(self.connection_lost(&e));
                }
                Err(format!("The A2L file could not be created: {}{}", e, warnings_text(&relevant_warnings(warnings, Some(&e)))))
            }
        }
    }

    //------------------------------------------------------------------------
    // Parameter files

    pub(super) async fn save_parameters(&mut self, file: Option<String>) -> Result<String, String> {
        let session = Self::ready(&mut self.session)?;
        // The file: as given, or a HEX file with a name built from the target name and the time
        let file = match file.filter(|file| !file.is_empty()) {
            Some(file) => file,
            None => {
                let target = session.ecu_name();
                format!(
                    "{}_{}.hex",
                    if target.is_empty() { "parameters" } else { target },
                    chrono::Local::now().format("%Y%m%d_%H%M%S")
                )
            }
        };
        if !has_extension(&file, &["hex", "bin"]) {
            return Err(format!(
                "The file '{}' has no known extension. Use .hex (Intel-HEX with the data of the calibration segments) or .bin (XCPlite persistence file).",
                file
            ));
        }
        // A parameter file is never overwritten
        if Path::new(&file).exists() {
            return Err(format!("The file '{}' exists, a parameter file is not overwritten. Give another file name.", file));
        }
        match session.upload_parameters(&file).await {
            Ok(segments) => {
                let size = std::fs::metadata(&file).map(|m| format!(" ({})", format_size(m.len()))).unwrap_or_default();
                Ok(format!("{} calibration segments saved to {}{}.", segments, file, size))
            }
            Err(e) => {
                let e = e.to_string();
                if !session.check_connection().await {
                    return Err(self.connection_lost(&e));
                }
                Err(format!("The parameters could not be saved: {}", e))
            }
        }
    }

    pub(super) async fn load_parameters(&mut self, file: String) -> Result<String, String> {
        let session = Self::ready(&mut self.session)?;
        if has_extension(&file, &["bin"]) {
            return Err(format!(
                "'{}' is an XCPlite persistence file (.bin). Only Intel-HEX files (.hex) can be loaded into the target.",
                file
            ));
        }
        if !Path::new(&file).is_file() {
            let directory = std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default();
            return Err(format!("File '{}' not found. File names are relative to the working directory {}", file, directory));
        }
        let total = session.description().map(|d| d.segments).unwrap_or(0);
        log_capture::begin();
        let result = session.download_parameters(&file).await;
        let warnings = log_capture::end();
        match result {
            Ok(0) => Err(format!(
                "Nothing loaded: the file '{}' has no data for the calibration segments of this target.{}",
                file,
                warnings_text(&warnings)
            )),
            Ok(segments) => Ok(format!(
                "{} of {} calibration segments loaded from {} into the working page of the target.{}",
                segments,
                total,
                file,
                warnings_text(&warnings)
            )),
            Err(e) => {
                let e = e.to_string();
                if !session.check_connection().await {
                    return Err(self.connection_lost(&e));
                }
                Err(format!(
                    "The parameters could not be loaded: {}. Segments before the error may be loaded already.{}",
                    e,
                    warnings_text(&relevant_warnings(warnings, Some(&e)))
                ))
            }
        }
    }
}

//------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_extension() {
        assert!(has_extension("a.hex", &["hex", "bin"]));
        assert!(has_extension("dir.d/hello_xcp_V2.3.HEX", &["hex", "bin"]));
        assert!(has_extension("a.bin", &["hex", "bin"]));
        assert!(!has_extension("a.bin", &["hex"]));
        assert!(!has_extension("hello_xcp_V2.3", &["hex", "bin"]));
        assert!(!has_extension("hex", &["hex", "bin"]));
    }

    #[test]
    fn test_relevant_warnings() {
        let warnings = || {
            vec![
                "Warning: No xcp_meta section found".to_string(),
                "Warning: Overwriting existing A2L file: x.a2l".to_string(),
                "Error: EPK mismatch: A2L file 'x.a2l' has EPK 'a', target reports EPK 'b'. Addresses may be wrong.".to_string(),
                "Error: Could not load x".to_string(),
            ]
        };
        // What the tool was asked to do is no warning
        assert_eq!(relevant_warnings(warnings(), None).len(), 3);
        // The error of the result is not repeated
        assert_eq!(relevant_warnings(warnings(), Some("Could not load x")), ["Warning: No xcp_meta section found"]);
    }

    #[test]
    fn test_warnings_text() {
        assert_eq!(warnings_text(&[]), "");
        assert_eq!(warnings_text(&["Warning: a".to_string(), "Error: b".to_string()]), "\nWarning: a\nError: b");
    }
}
