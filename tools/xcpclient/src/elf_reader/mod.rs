//--------------------------------------------------------------------------------------------------------------------------------------------------
// Module elf_reader
// Defines and implements ElfReader
// Read ELF files and extract debug information with DebugData (see copyright notice below)
// ElfReader provides functions to fill a XCP registry with events, segments, variables and metadata

// Based on Github repository a2ltool by DanielT: https://github.com/DanielT/a2ltool

/*
Note on V2.1.10:
Updated to typereader.rs from a2ltool v3.4.1 (commit 0b61aa5, 2026-08-04).
The Class variant is gone.
Struct now carries is_class and inheritance, and the size and Display code follow.
The two Class match arms from the previous fix are collapsed into the Struct arms, and a new test asserts that base members arrive for all four struct/class inheritance combinations.
*/

#![allow(clippy::collapsible_else_if)]

use indexmap::IndexMap;
use regex::Regex;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::ffi::OsStr;

#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};

use xcp_registry::{McAddress, McDimType, McEvent, McIdentifier, McObjectQualifier, McObjectType, McSupportData, McValueType, Registry, RegistryError};

/*
Which information can be detected from ELF/DWARF:
    - Events:
        name, compilation unit, function name and CFA offset, but index is unknown
    - Memory segment name, type (naming convention name = reference page), address, length, but number is unknown
    - Variables:
        variable name, typename, absolute address, frame offset, compilation unit, function name, namespace
        static variables in functions get the correct event
        local variables on stack get the correct CFA
        name, type, compilation unit, namespace, location (register or stack)
    - Types:
        typedefs, structs, enums
        basic types: int8/16/32/64, uint8/16/32/64, float, double
        arrays 1D and 2D
        pointers (as ulong or ulonglong)

Key benefits:
    - Instance names get prefixed with function name if local stack or static variables
    - All instances get the correct fixed event id, if there is one in their scope, otherwise default event id is 0
    - Event compilation unit, function and CFA is detected to enable local variable access

Tools:
    dwarfdump --debug-info <filename>
    dwarfdump --debug-info --name <varname> <filename>
    objdump -h  <filename>
    objdump --syms <filename>

Limitations:
    - With -o1 most stack variables are in registers, have to be manually spilled to stack or captured
    - Segment numbers and event index are not constant expressions, need to be read by XCP (current solution) or from the binary persistence file from the target

Possible future improvements:
    - Thread load addressing mode
    - C++ support,  this addressing support, namespaces
    - Measurement of variables and function parameters in registers
    - Just in time compilation of variable access expressions
*/

// Dwarf reader
// This module contains modified code adapted from https://github.com/DanielT/a2ltool
// Original code licensed under MIT/Apache-2.0
// Copyright (c) DanielT
mod debuginfo;
use debuginfo::{DbgDataType, DebugData, TypeInfo, VarInfo};

//------------------------------------------------------------------------
//  ELF reader and A2L creator

pub(crate) struct ElfReader {
    pub(crate) debug_data: DebugData,
    typedefs: RefCell<TypedefNames>, // Bookkeeping for the typedef names of the struct/class types registered so far
}

// Bookkeeping for the typedef names of struct and class types
// A2L has one flat name space for typedefs, but the DWARF type name is the unqualified name, so different types
// may have the same name (motor_control::Input and valve_control::Input, MotorController::Params and ValveController::Params)
#[derive(Default)]
struct TypedefNames {
    ambiguous: HashSet<String>,          // Type names used by struct/class types in different scopes, these types get scope qualified typedef names
    signatures: HashMap<String, String>, // Typedef name -> content signature of the registered typedef, to detect different types with the same name
}

// Find the type names which are used by struct/class types in different scopes (namespaces, classes or functions)
fn find_ambiguous_type_names(debug_data: &DebugData) -> HashSet<String> {
    let mut ambiguous = HashSet::new();
    for (type_name, type_refs) in &debug_data.typenames {
        let mut scopes: Vec<&[String]> = Vec::new();
        for type_ref in type_refs {
            if let Some(type_info) = debug_data.types.get(type_ref)
                && matches!(type_info.datatype, DbgDataType::Struct { .. })
            {
                let scope: &[String] = debug_data.type_scopes.get(type_ref).map_or(&[], |s| s.as_slice());
                if !scopes.contains(&scope) {
                    scopes.push(scope);
                }
            }
        }
        if scopes.len() > 1 {
            debug!(
                "Type name '{}' is used by {} different struct/class types, typedef names are qualified with their scope",
                type_name,
                scopes.len()
            );
            ambiguous.insert(type_name.clone());
        }
    }
    ambiguous
}

impl ElfReader {
    // Load debug information from the ELF file
    pub fn new(file_name: &str, verbose: usize, unit_idx_limit: usize) -> Option<ElfReader> {
        info!("Loading debug information from ELF file: {}", file_name);
        let debug_data = DebugData::load_dwarf(OsStr::new(file_name), verbose, unit_idx_limit);
        match debug_data {
            Ok(debug_data) => Some(ElfReader::from_debug_data(debug_data)),
            Err(e) => {
                error!("Failed to load debug info from '{}': {}", file_name, e);
                None
            }
        }
    }

    // Create the ELF reader from loaded debug information
    fn from_debug_data(debug_data: DebugData) -> ElfReader {
        let ambiguous = find_ambiguous_type_names(&debug_data);
        ElfReader {
            debug_data,
            typedefs: RefCell::new(TypedefNames {
                ambiguous,
                signatures: HashMap::new(),
            }),
        }
    }

    // Get the McValueType for a given TypeInfo, which can be a basic type, pointer or array
    fn get_value_type(&self, reg: &mut Registry, type_info: &TypeInfo, object_type: McObjectType) -> McValueType {
        let type_size = type_info.get_size();
        match &type_info.datatype {
            DbgDataType::Uint8 => McValueType::Ubyte,
            DbgDataType::Uint16 => McValueType::Uword,
            DbgDataType::Uint32 => McValueType::Ulong,
            DbgDataType::Uint64 => McValueType::Ulonglong,
            DbgDataType::Sint8 => McValueType::Sbyte,
            DbgDataType::Sint16 => McValueType::Sword,
            DbgDataType::Sint32 => McValueType::Slong,
            DbgDataType::Sint64 => McValueType::Slonglong,
            DbgDataType::Float => McValueType::Float32Ieee,
            DbgDataType::Double => McValueType::Float64Ieee,
            DbgDataType::Struct { size, members, .. } => {
                if type_info.name.is_some() {
                    // Register a typedef for the struct/class type (reused if it already exists) and reference it by its unique typedef name.
                    // Inherited members of structs and classes are already flattened into `members` by the DWARF reader.
                    McValueType::new_typedef(self.register_struct(reg, object_type, type_info, *size as usize, members))
                } else {
                    warn!("Struct/class type without name in get_value_type");
                    McValueType::Ubyte
                }
            }
            DbgDataType::Enum { size, signed, enumerators } => McValueType::from_integer_size(*size as usize, *signed),

            DbgDataType::TypeRef(typeref, size) => {
                if let Some(typeinfo) = self.debug_data.types.get(typeref) {
                    self.get_value_type(reg, typeinfo, object_type)
                } else {
                    error!("TypeRef {} to unknown in get_field_type", typeref);
                    McValueType::Ubyte
                }
            }

            DbgDataType::Pointer(size, _pointee) => {
                if *size == 4 {
                    McValueType::Ulong
                } else if *size == 8 {
                    McValueType::Ulonglong
                } else {
                    warn!("Unsupported pointer size {} in get_field_type", size);
                    McValueType::Ulonglong
                }
            }

            // These types are not a supported value type (arrays are handled in get_dim_type)
            // DbgDataType::Bitfield | DbgDataType::Union | DbgDataType::FuncPtr | DbgDataType::Other | DbgDataType::Array =>
            _ => {
                warn!("Unsupported type in get_field_type: {:?}", &type_info.datatype);
                //assert!(false, "Unsupported type in get_field_type: {:?}", &type_info.datatype);
                McValueType::Ubyte
            }
        }
    }

    // Get the dimension type for a variable, which is used to determine the number of elements and dimensions for arrays
    fn get_dim_type(&self, reg: &mut Registry, type_info: &TypeInfo, object_type: McObjectType) -> McDimType {
        let type_size = type_info.get_size();
        match &type_info.datatype {
            DbgDataType::Array { arraytype, dim, stride, size } => {
                assert!(dim.len() != 0);
                let elem_type = self.get_value_type(reg, arraytype, object_type);
                if dim.len() > 2 {
                    warn!("Only 1D and 2D arrays supported, got {}D", dim.len());
                    McDimType::new(McValueType::Ubyte, 1, 1)
                } else if dim.len() == 1 {
                    McDimType::new(elem_type, dim[0] as u16, 1)
                } else {
                    McDimType::new(elem_type, dim[0] as u16, dim[1] as u16)
                }
            }
            _ => McDimType::new(self.get_value_type(reg, type_info, object_type), 1, 1),
        }
    }

    // Register a struct/class type as typedef in the registry, including its members, and return the typedef name to reference it.
    // Nested struct/class members are registered recursively.
    // A2L has one flat name space for typedefs, but the DWARF type name is unqualified, so the typedef name is determined as follows:
    // - The typedef is named after the type, the identifier is sanitized (e.g. "TplStruct<short unsigned int>" -> "TplStruct_short_unsigned_int_").
    // - If struct/class types with this name exist in different scopes, the name is qualified with the enclosing namespaces, classes or
    //   functions of the type (motor_control::Input -> "motor_control.Input", MotorController::Params -> "MotorController.Params").
    // - A typedef with the same name and the same content is reused: the same type used by several variables or fields, or
    //   the same type defined again in another compilation unit (the DWARF of every compilation unit has its own copy of a type).
    // - If the name is still in use for a typedef with different content, a numeric suffix is appended ("state_1"). This happens for
    //   different types with the same name and without scope (file local struct types in different C files), and for a type
    //   which is used for measurement and for calibration variables (TYPEDEF_MEASUREMENT vs. TYPEDEF_CHARACTERISTIC components).
    fn register_struct(&self, reg: &mut Registry, object_type: McObjectType, type_info: &TypeInfo, size: usize, members: &IndexMap<String, (TypeInfo, u64)>) -> McIdentifier {
        let type_name = type_info.name.as_deref().unwrap_or("");
        let location = || self.debug_data.make_simple_unit_name(type_info.unit_idx).unwrap_or_else(|| type_info.unit_idx.to_string());

        // Resolve the member types first, this may recursively register nested typedefs
        let first_nested_index = reg.typedef_list.len();
        let mut fields: Vec<(&String, McDimType, u16)> = Vec::with_capacity(members.len());
        for (field_name, (field_type_info, field_offset)) in members {
            let Ok(offset) = u16::try_from(*field_offset) else {
                warn!("Field '{}.{}' skipped, offset {} exceeds the supported range", type_name, field_name, field_offset);
                continue;
            };
            fields.push((field_name, self.get_dim_type(reg, field_type_info, object_type), offset));
        }

        // Content signature: typedefs may share a name only if size, object type and all fields (name, type, dimensions, offset) are identical
        let signature = format!("{size} {object_type:?} {fields:?}");

        // Scope qualified name for ambiguous type names, e.g. "motor_control.Input" for motor_control::Input
        let base_name = match self.debug_data.type_scopes.get(&type_info.dbginfo_offset) {
            Some(scope) if self.typedefs.borrow().ambiguous.contains(type_name) => format!("{}.{}", scope.join("."), type_name),
            _ => type_name.to_string(),
        };

        // Find the typedef with this content or a free name: base_name, base_name_1, base_name_2, ...
        let mut candidate = base_name.clone();
        let mut suffix = 0;
        let type_id = loop {
            let type_id = McIdentifier::from(candidate.clone());
            match self.typedefs.borrow().signatures.get(type_id.as_str()) {
                Some(existing) if *existing == signature => return type_id, // already registered
                Some(_) => {}                                               // used by a typedef with different content
                None if reg.typedef_list.find_typedef(type_id.as_str()).is_none() => break type_id,
                None => {} // used by a typedef which was not registered from the ELF file
            }
            suffix += 1;
            candidate = format!("{base_name}_{suffix}");
        };
        if suffix > 0 {
            warn!(
                "Struct/class type '{}' in {} has a different definition or object type than the existing typedef '{}', registered as typedef '{}'",
                type_name,
                location(),
                base_name,
                type_id
            );
        } else if base_name != type_name {
            info!(
                "Struct/class type '{}' in {} registered as typedef '{}', the type name is used in different scopes",
                type_name,
                location(),
                type_id
            );
        }

        // Register the typedef and its fields
        if let Err(e) = reg.add_typedef(type_id, size) {
            error!("Failed to register typedef '{}' for struct/class type '{}': {}", type_id, type_name, e);
            return type_id;
        }
        for (field_name, field_dim_type, offset) in fields {
            if let Err(e) = reg.add_typedef_field(type_id.as_str(), field_name.clone(), field_dim_type, McSupportData::new(object_type), offset) {
                error!("Failed to register field '{}.{}': {}", type_id, field_name, e);
            }
        }
        // Keep the typedef in front of its nested typedefs in the registry, this is the order of the typedefs in the A2L file
        reg.typedef_list[first_nested_index..].rotate_right(1);
        self.typedefs.borrow_mut().signatures.insert(type_id.to_string(), signature);
        type_id
    }

    // Find the addressing mode marker variable (naming convention "XCPLITE__<signature>") and return the signature, if found
    // (CASDD, ACSDD, ...)
    pub fn get_target_signature(&self) -> Option<&str> {
        // Iterate over variables and look for XCPlite addressing mode marker
        for (var_name, var_infos) in &self.debug_data.variables {
            if !var_name.starts_with("XCPLITE__") {
                continue;
            }
            if let Some(signature) = var_name.strip_prefix("XCPLITE__") {
                return Some(signature);
            }
        }
        return None;
    }

    // Get the EPK string and address from debug_data and set it in the registry application version information, if available
    pub fn register_epk_addr_info(&self, reg: &mut Registry, verbose: usize) {
        info!("===============================================================");
        if self.debug_data.epk_addr > 0 {
            info!("EPK segment memory section found at address = 0x{:08X}", self.debug_data.epk_addr);
            let epk = self.debug_data.epk_string.clone().unwrap_or_else(|| "<unknown>".to_string());
            info!("EPK string: '{}'", epk);
            reg.application.set_version(epk, self.debug_data.epk_addr.try_into().unwrap());
        } else {
            warn!("EPK segment memory section not found in ELF file");
        }
    }

    // Register segments from segment creation markers (calseg__name) found in the code
    pub fn register_segments(&self, reg: &mut Registry, seg_relative: bool, verbose: usize) -> Result<(), Box<dyn Error>> {
        info!("===============================================================");
        info!(
            "Registering segment information {}:",
            if !seg_relative { "(absolute addressing mode)" } else { "(relative addressing mode)" }
        );
        info!("===============================================================");

        // Step 1
        // Iterate over all variables and look for segment definition markers, which are created by the CalSegCreate or CalBlkCreate macros
        // Naming convention is "calseg__<name>" or "calblk__<name>"
        // Sort the vector by address to ensure the segments are processed in the order they are defined in the code
        // Index in the vector is now the segment number
        let mut seg_definitions: Vec<(String, &Vec<VarInfo>, u64, Option<u8>)> = Vec::new();
        for (var_name, var_infos) in &self.debug_data.variables {
            let is_calseg = var_name.starts_with("calseg__");
            let is_calblk = var_name.starts_with("calblk__");
            if is_calseg || is_calblk {
                let (seg_name, seg_number) = if is_calseg {
                    (var_name.strip_prefix("calseg__").unwrap_or(var_name), Some(0))
                } else {
                    (var_name.strip_prefix("calblk__").unwrap_or(var_name), None)
                };
                let mut seg_descr_addr = var_infos[0].address.1;
                if seg_name == "epk" {
                    // EPK segment is a special case, it has always index = 0
                    seg_descr_addr = 0;
                }
                assert!(var_infos.len() == 1);
                seg_definitions.push((seg_name.to_string(), var_infos, seg_descr_addr, seg_number));
            }
        }
        seg_definitions.sort_by_key(|x| x.2);
        // Calculate the segment numbers for calseg, calblk doues not have a number
        let mut seg_number: u8 = 0;
        for i in 0..seg_definitions.len() {
            if let Some(0) = seg_definitions[i].3 {
                seg_definitions[i].3 = Some(seg_number);
                seg_number += 1;
            }
        }

        // Print the found segment definition markers
        if verbose >= 1 {
            println!("Found {} segment definition marker variables:", seg_definitions.len());
            for (seg_index, (var_name, var_infos, var_address, seg_number)) in seg_definitions.iter().enumerate() {
                println!("{}: '{}' - number={:?}, addr={:08X}'", seg_index, var_name, seg_number, var_address);
                if verbose >= 2 {
                    let var_info = &var_infos[0];
                    let function_name = if let Some(f) = var_info.function.as_ref() { f.as_str() } else { "" };
                    let unit_idx = var_info.unit_idx;
                    let unit_name = if let Some(name) = self.debug_data.make_simple_unit_name(unit_idx) {
                        name
                    } else {
                        format!("{unit_idx}")
                    };
                    println!("  found in {}:'{}'", unit_name, function_name);
                }
            }
        }

        // Step 2
        // Iterate over the segment definitions and register the segments in the registry
        for (seg_index, (seg_name, var_infos, var_address, seg_number)) in seg_definitions.iter().enumerate() {
            let var_info = &var_infos[0];
            let seg_length: u16;
            let seg_addr: u64;

            // Special case for EPK segment, which does not have a reference page variable, but the segment address and length may be stored in the debug data from the EPK section
            if seg_name == "epk" {
                if let Some(epk_str) = self.debug_data.epk_string.as_ref() {
                    seg_length = epk_str.len().try_into().expect("EPK string length exceeds 64K");
                    seg_addr = self.debug_data.epk_addr;
                } else {
                    error!("No EPK segment memory section in ELF file, segment '{}' skipped", seg_name);
                    continue; // skip this variable
                }
            }
            // Not epk segment
            else {
                // Lookup the reference page variable (by naming convention: same as segment name!) information
                // This may be ambigous, so we use some heuristics to select the right variable
                // @@@@ TODO use the commandline compilation unit filter here
                let seg_var_info = if let Some(x) = self.debug_data.variables.get(seg_name) {
                    let mut valid_candidates: Vec<_> = x.iter().filter(|var_info| var_info.address.0 == 0 && var_info.address.1 != 0).collect();
                    if valid_candidates.len() > 1 {
                        let same_unit_candidates: Vec<_> = valid_candidates.iter().copied().filter(|candidate| candidate.unit_idx == var_info.unit_idx).collect();
                        if same_unit_candidates.len() == 1 {
                            valid_candidates = same_unit_candidates;
                        }
                    }
                    if valid_candidates.len() != 1 {
                        error!(
                            "Calibration segment reference page variable '{}' has {} usable definitions, expected 1 ({} total DWARF entries)",
                            seg_name,
                            valid_candidates.len(),
                            x.len()
                        );
                        if verbose >= 1 {
                            for candidate in x {
                                let unit_name = self.debug_data.make_simple_unit_name(candidate.unit_idx).unwrap_or_else(|| candidate.unit_idx.to_string());
                                let function_name = candidate.function.as_deref().unwrap_or("<global>");
                                println!(
                                    "  candidate in {}:'{}', addr_class={}, addr=0x{:08X}",
                                    unit_name, function_name, candidate.address.0, candidate.address.1
                                );
                            }
                        }
                        continue;
                    }
                    valid_candidates[0]
                } else {
                    error!("Could not find calibration segment reference page variable '{}'", seg_name);
                    continue;
                };

                // Determine segment length
                seg_length = {
                    if let Some(type_info) = self.debug_data.types.get(&seg_var_info.typeref) {
                        println!(
                            "Calibration segment '{}' type information found, type={}, size = {}",
                            seg_name,
                            type_info.name.as_ref().map_or("<unnamed>", |s| s.as_str()),
                            type_info.get_size()
                        );
                        if verbose >= 2 {
                            println!("  type = {}", type_info);
                        }
                        type_info.get_size().try_into().expect("segment size exceeds 64K")
                    } else {
                        error!("Could not determine length type for segment {}", seg_name);
                        0
                    }
                };

                // Determine segment address
                // @@@@ TODO: handle signed relative encoding
                seg_addr = seg_var_info.address.1;
                if !(seg_length > 0 && seg_addr > 0 && seg_var_info.address.0 == 0) {
                    error!(
                        "Calibration segment from cal_<name> '{}' not found, has invalid address {:#x} or size {:#x}, skipped",
                        seg_name, seg_addr, seg_length
                    );
                    continue; // skip this variable
                }

                info!(
                    "Calibration segment '{}' default page variable found in debug data: Address = {:#x}, Size = {:#x}",
                    seg_name, seg_addr, seg_length
                );
            } // not EPK segment

            // Find the segment by name in the registry
            if let Some(reg_seg) = reg.cal_seg_list.find_cal_seg(seg_name) {
                info!("Calibration segment '{}' {}:0x{:08X} found in registry", seg_name, reg_seg.addr_ext, reg_seg.addr);
                // Segment relative addressing mode
                if reg_seg.addr == 0x80000000 + ((reg_seg.index as u32) << 16) {
                    info!("  with segment relative addressing");
                    // Check if length matches
                    if reg_seg.size == seg_length as u32 {
                        reg_seg.set_mem_addr(seg_addr);
                        info!("  matches existing registry entry");
                    } else {
                        warn!("Calibration segment '{}' length does not match existing registry entry", seg_name);
                    }
                }
                // Segment absolute addressing mode
                else {
                    // Check if address and length match
                    if reg_seg.addr as u64 != seg_addr {
                        warn!(
                            "Calibration segment '{}' address does not match existing registry entry, reg = {:08X} vs. {:08X}",
                            seg_name, reg_seg.addr, seg_addr
                        );
                    } else if reg_seg.size != seg_length as u32 {
                        warn!(
                            "Calibration segment '{}' length does not match existing registry entry, reg = {} vs. {}",
                            seg_name, reg_seg.size, seg_length
                        );
                    } else {
                        info!("Calibration segment '{}' matches existing registry entry", seg_name);
                    }
                } // absolute addressing mode
            }
            // already existing
            //
            // If not existing, create the segment
            // Use segment relative or absolute addressing mode
            else {
                info!("Calibration segment '{}' not yet defined in registry", seg_name);

                if seg_relative {
                    // Add in segment relative addressing mode
                    let res = reg.cal_seg_list.add_cal_seg(seg_name.to_string(), *seg_number, seg_length as u32);
                    if let Err(e) = res {
                        error!("Failed to add calibration segment '{}': {}", seg_name, e);
                        continue;
                    }
                } else {
                    // Absolute addressing mode
                    if seg_addr >= 0xFFFFFFFF {
                        error!(
                            "Calibration segment '{}' has 64 bit address {:#x}, which does not fit the 32 bit XCP address range",
                            seg_name, seg_addr
                        );
                        continue; // skip 
                    }
                    if seg_index >= 255 {
                        error!("Too many calibration segments, segment index {} does not fit in u8 for segment '{}'", seg_index, seg_name);
                        continue; // skip
                    }
                    if seg_length == 0 {
                        error!("Calibration segment '{}' has zero length, skipped", seg_name);
                        continue; // skip
                    }
                    let res = reg
                        .cal_seg_list
                        .add_cal_seg_by_addr(seg_name.to_string(), *seg_number, 0, seg_addr as u32, seg_length as u32);
                    if let Err(e) = res {
                        error!("Failed to add calibration segment '{}': {}", seg_name, e);
                        continue;
                    }
                }

                // Set memory address for later lookup of potential calibration variables in this segment
                let new_seg = reg.cal_seg_list.find_cal_seg(seg_name).unwrap();
                new_seg.set_mem_addr(seg_addr);

                info!(
                    "Created segment {}: '{}':  addr = 0x{:08X}, size = {}, mem_addr = 0x{:08X}",
                    seg_index, seg_name, new_seg.addr, new_seg.size, new_seg.mem_addr
                );
            } // not already existing
        } // for
        Ok(())
    }

    // Register events from event creation markers (evt__name) in the code
    pub fn register_events(&self, reg: &mut Registry, verbose: usize) -> Result<(), Box<dyn Error>> {
        info!("===============================================================");
        info!("Registering event information:");
        info!("===============================================================");

        // Get the address range of the XCP event descriptor memory section (start is 0 if not found)
        let xcp_event_section_addr = self.debug_data.get_event_section_addr();
        let xcp_event_section_end = self
            .debug_data
            .sections
            .get("xcp_evts")
            .map(|(_, end)| *end)
            .or_else(|| self.debug_data.symbol_addresses.get("__stop_xcp_evts").copied());

        // Placeholder ids for events whose id can not be determined from the event descriptor section
        // They must be unique in the registry, counting down from 0xFFFF keeps them out of the range of real event ids
        // The ids are corrected from the XCP server event information when connected (see --fix-a2l)
        let mut next_undefined_event_id: u16 = 0xFFFF;

        // Location "unit:function" of a marker variable for messages
        let location = |v: &VarInfo| -> String {
            let unit_name = self.debug_data.make_simple_unit_name(v.unit_idx).unwrap_or_else(|| v.unit_idx.to_string());
            format!("{}:{}", unit_name, v.function.as_deref().unwrap_or(""))
        };

        // Iterate over variables
        for (var_name, var_infos) in &self.debug_data.variables {
            // Skip standard library variables and system/compiler internals (__<name>)s
            // Skip global XCP variables (gXCP.. and gA2L..)
            if var_name.starts_with("__") || var_name.starts_with("gXcp") || var_name.starts_with("gA2l") {
                continue;
            }

            // Event definitions (by markers from DaqCreateEvent macro)
            // (thread local) static evt__<name>, name is event name
            if let Some(evt_name) = var_name.strip_prefix("evt__") {
                let Some(first) = var_infos.first() else {
                    continue;
                };

                // The DaqCreateEvent macro emits one event descriptor per call site. If the same event is created in several
                // functions or compilation units, the target creates the event once for the first descriptor in the section
                // (XcpInit scans the section in address order), so the definition with the lowest address is used here as well
                let var_info = var_infos.iter().filter(|v| v.address.1 != 0).min_by_key(|v| v.address.1).unwrap_or(first);
                info!(
                    "Event definition for event '{}' found in {}, addr = {:#x}",
                    evt_name,
                    location(var_info),
                    var_info.address.1
                );
                if var_infos.len() > 1 {
                    let others: Vec<String> = var_infos.iter().filter(|v| !std::ptr::eq(*v, var_info)).map(|v| location(v)).collect();
                    warn!(
                        "Event '{}' is defined {} times, using the definition in {} (also defined in {})",
                        evt_name,
                        var_infos.len(),
                        location(var_info),
                        others.join(", ")
                    );
                }

                // Skip if the event already exists in the registry (e.g. from the XCP server event information)
                if reg.event_list.find_event(evt_name, 0).is_some() {
                    continue;
                }

                // Determine the event id from the position of the event descriptor in the event descriptor section
                let addr = var_info.address.1;
                let mut event_id: Option<u16> = None;
                if xcp_event_section_addr > 0 && addr >= xcp_event_section_addr && xcp_event_section_end.is_none_or(|end| addr < end) {
                    let id = ((addr - xcp_event_section_addr) / 16) as u16; // @@@@ size of tXcpEventDescriptor hardcoded
                    if let Some(other) = reg.event_list.find_event_id(id) {
                        warn!("Event id {} of event '{}' is already used by event '{}'", id, evt_name, other.get_name());
                    } else {
                        event_id = Some(id);
                    }
                } else if xcp_event_section_addr > 0 {
                    warn!("Event definition marker of event '{}' at {:#x} is outside the event descriptor section", evt_name, addr);
                }

                match event_id {
                    Some(id) => {
                        reg.event_list.add_event(McEvent::new(evt_name.to_string(), 0, id, 0))?;
                        info!("New event '{}' found: event id = {}", evt_name, id);
                    }
                    None => {
                        // Use a unique placeholder id, it has to be corrected later from the XCP server event information
                        let mut id = next_undefined_event_id;
                        while reg.event_list.find_event_id(id).is_some() {
                            id = id.saturating_sub(1);
                        }
                        next_undefined_event_id = id.saturating_sub(1);
                        reg.event_list.add_event(McEvent::new(evt_name.to_string(), 0, id, 0))?;
                        warn!(
                            "New event '{}' found, created with undefined event id {:#06x}, correct it with the XCP server event information",
                            evt_name, id
                        );
                    }
                }
            }
        }
        Ok(())
    }

    // Find event triggers in the code and register their location (compilation unit, function, CFA offset)
    pub fn register_event_locations(&self, reg: &mut Registry, verbose: usize) -> Result<(), Box<dyn Error>> {
        info!("===============================================================");
        info!("Registering event locations:");
        info!("===============================================================");

        // Iterate over variables
        for (var_name, var_infos) in &self.debug_data.variables {
            // Skip standard library variables and system/compiler internals (__<name>)s
            // Skip global XCP variables (gXCP.. and gA2L..)
            if var_name.starts_with("__") || var_name.starts_with("gXcp") || var_name.starts_with("gA2l") {
                continue;
            }

            // trg__<event_name> (thread local static, name is event name)
            // Event definitions (thread local static variables)
            if var_name.starts_with("trg__") {
                // One trigger location per event is expected, the location is used to resolve stack relative variables
                if var_infos.len() > 1 {
                    warn!(
                        "Event trigger marker '{}' is defined {} times, only the first definition is used to locate stack variables",
                        var_name,
                        var_infos.len()
                    );
                }
                let Some(var_info) = var_infos.first() else {
                    continue;
                };

                // Get the event name from format  "trg__<tag>__<eventname>" prefix
                let s = var_name.strip_prefix("trg__").unwrap_or("unnamed");
                let mut parts = s.split("__");
                let evt_mode = parts.next().unwrap_or("");
                let evt_name = parts.next().unwrap_or("");

                let evt_unit_idx = var_infos[0].unit_idx;
                let evt_unit_name = if let Some(name) = self.debug_data.make_simple_unit_name(evt_unit_idx) {
                    name
                } else {
                    format!("{evt_unit_idx}")
                };
                let evt_function = if let Some(f) = var_info.function.as_ref() { f.as_str() } else { "" };
                info!(
                    "  Event {} trigger found in {}:{}, address resolver mode {}",
                    evt_name, evt_unit_name, evt_function, evt_mode
                );

                // Find the event in the registry
                if let Some(_evt) = reg.event_list.find_event(evt_name, 0) {
                    // Try to lookup the canonical stack frame address offset from the function name
                    let mut evt_cfa: i32 = 0;
                    for cfa_info in self.debug_data.cfa_info.iter() {
                        if cfa_info.unit_idx == evt_unit_idx && cfa_info.function == evt_function {
                            if let Some(x) = cfa_info.cfa_offset {
                                evt_cfa = x as i32;
                            } else {
                                warn!("Could not determine CFA offset for function '{}'", evt_function);
                            }
                            break;
                        }
                    }

                    if verbose >= 1 {
                        println!("  Event '{}' trigger in function '{}', cfa = {}", evt_name, evt_function, evt_cfa);
                    }

                    // Store the unit and function name and canonical stack frame address offset for this event trigger
                    match reg.event_list.set_event_location(evt_name, evt_unit_idx, evt_function, evt_cfa) {
                        Ok(_) => {}
                        Err(e) => {
                            error!("Failed to set event location for event '{}': {}", evt_name, e);
                        }
                    }
                } else {
                    error!("Event '{}' for trigger not found in registry", evt_name);
                }
                continue; // skip this variable
            }
        }
        Ok(())
    }

    // Register variables from the ELF debug information into the registry
    pub fn register_variables(
        &self,
        reg: &mut Registry,
        seg_relative: bool,
        verbose: usize,
        unit_idx_limit: usize,
        name_filter: &str,
        unit_filter: &str,
    ) -> Result<(), Box<dyn Error>> {
        // Load debug information from the ELF file
        info!("===============================================================");
        info!("Registering variables:");
        info!("===============================================================");

        // Compile name filter regex if specified
        let name_regex: Option<Regex> = if name_filter.is_empty() {
            None
        } else {
            match Regex::new(name_filter) {
                Ok(re) => {
                    info!("Variable name filter: '{}'", name_filter);
                    Some(re)
                }
                Err(e) => {
                    return Err(format!("Invalid --elf-var-filter regex '{}': {}", name_filter, e).into());
                }
            }
        };

        // Compile compilation unit filter regex if specified
        let unit_regex: Option<Regex> = if unit_filter.is_empty() {
            None
        } else {
            match Regex::new(unit_filter) {
                Ok(re) => {
                    info!("Compilation unit filter: '{}'", unit_filter);
                    Some(re)
                }
                Err(e) => {
                    return Err(format!("Invalid --elf-unit-filter regex '{}': {}", unit_filter, e).into());
                }
            }
        };

        // Iterate over variables
        for (var_name, var_infos) in &self.debug_data.variables {
            // Skip standard library variables and system/compiler internals (__<name>)
            // Skip global XCP variables (gXCP.. and gA2L..) and special marker variables (calseg__, evt__, trg__, xcp_meta__)
            if var_name.starts_with("__")
                || var_name.starts_with("gXcp")
                || var_name.starts_with("gA2l")
                || var_name.starts_with("calseg__")
                || var_name.starts_with("calblk__")
                || var_name.starts_with("evt__")
                || var_name.starts_with("trg__")
                || var_name.starts_with("xcp_meta__")
            {
                continue;
            }

            // Apply name filter
            if let Some(ref re) = name_regex {
                if !re.is_match(var_name) {
                    continue;
                }
            }

            if var_infos.is_empty() {
                warn!("Variable '{}' has no variable info", var_name);
            }

            let mut a2l_name = var_name.to_string();
            let mut xcp_event_id: Option<u16>;
            // @@@@ TODO: Behaviour changed, check what this affect, async event 0 (OPTION_DAQ_ASYNC_EVENT) is now optional, does not work with section registered events (e.g. FreeRTOS)
            // Previous: default event id is 0, which is the async event in transmit thread
            // Current: default event id is None, meaning no event assigned

            // daq__<event_name>__<var_name> (local scope static variables)
            // Check for captured variables with format "daq__<event_name>__<var_name>"
            // @@@@ TODO: Check if this is correct and up to date with the current event handling logic
            /*
            if var_name.starts_with("daq__") {
                // remove the "daq__" prefix
                let new_name = var_name.strip_prefix("daq__").unwrap_or(var_name);
                // get event name and variable name
                let mut parts = new_name.split("__");
                let event_name = parts.next().unwrap_or("");
                let var_name = parts.next().unwrap_or("");
                // Find the event in the registry
                if let Some(id) = reg.event_list.find_event(event_name, 0) {
                    xcp_event_id = Some(id.id);
                    if event_name.len() > 0 {
                        a2l_name = format!("{}.{}", event_name, var_name);
                    } else {
                        a2l_name = var_name.to_string();
                    }
                } else {
                    warn!("Event '{}' for captured variable '{}' not found in registry", event_name, var_name);
                    continue; // skip this variable
                }
            }
            */

            // Count variables with this name in compilation unit 0
            let count = var_infos.iter().filter(|v| v.unit_idx <= unit_idx_limit).count();
            // Count the distinct global or static variables with this name by their address
            // (declarations of the same variable in several compilation units resolve to the same address, local variables have no address)
            let mut addresses: Vec<u64> = var_infos
                .iter()
                .filter(|v| v.unit_idx <= unit_idx_limit && v.address.0 == 0 && v.address.1 != 0)
                .map(|v| v.address.1)
                .collect();
            addresses.sort_unstable();
            addresses.dedup();
            let defined_count = addresses.len();

            // Process all variables with this name in different scopes and namespaces
            for var_info in var_infos {
                // @@@@ TODO: Create only variables from specified compilation unit
                if var_info.unit_idx > unit_idx_limit {
                    continue;
                }

                // Apply compilation unit filter
                if let Some(ref re) = unit_regex {
                    let cu_name = self.debug_data.make_simple_unit_name(var_info.unit_idx).unwrap_or_else(|| format!("{}", var_info.unit_idx));
                    if !re.is_match(&cu_name) {
                        continue;
                    }
                }

                let var_function = var_info.function.as_ref().map(|f| f.as_str());

                // Address encoder
                let mem_addr_ext: u8 = var_info.address.0;
                let mem_addr: u64 = 
                
                // Encode absolute addressing mode
                if mem_addr_ext == 0 {
                    if var_info.address.1 == 0 {
                        debug!("Variable '{}' not registered, no address", var_name);
                        continue; // skip this variable
                    } 
                    else if var_info.address.1 >= 0xFFFFFFFF {
                        warn!(
                            "Global variable '{}' not registered, address {:#x} out of the 32 bit XCP address range",
                            var_name, var_info.address.1
                        );
                        continue; // skip this variable
                    } 
                    else {
                        // Find an event triggered in the function
                        if let Some(var_function_name) = var_function {
                            if let Some(event) = reg.event_list.find_event_by_location(var_info.unit_idx, var_function_name) {
                                xcp_event_id = Some(event.id);
                                info!("Static variable '{}' local to function '{:?}', event id = {}", var_name, var_function, event.id);
                            } else {
                                info!("Static variable '{}' local to function '{:?}', no event associated, no event found in this function", var_name, var_function);
                                xcp_event_id = None;
                            }
                            
                        } else {
                            info!("Global variable '{}', no event associated", var_name);
                            xcp_event_id = None;
                        }

                        // Multiple variables with this name: local static variables are prefixed with the function name,
                        // global variables defined in several namespaces with their namespace (motor_control.input, valve_control.input)
                        if count > 1 {
                            if let Some(f) = var_function {
                                a2l_name = format!("{}.{}", f, var_name);
                            } else if defined_count > 1 && !var_info.namespaces.is_empty() {
                                a2l_name = format!("{}.{}", var_info.namespaces.join("."), var_name);
                            } else {
                                a2l_name = var_name.to_string();
                            }
                        }
                        var_info.address.1

                    }
                }

                // Encode stack relative addressing mode
                else if mem_addr_ext == 2 {
                    // Find an event id for this local variable
                    let var_function_name = var_function.expect("Local variable function name is missing for relative addressing mode");
                    if let Some(event) = reg.event_list.find_event_by_location(var_info.unit_idx, var_function_name) {
                        // Set the event id for this function
                        // Prefix the variable with the function name
                        xcp_event_id = Some(event.id);
                        let cfa: i64 = event.cfa as i64;
                        if let Some(f) = var_function {
                            a2l_name = format!("{}.{}", f, var_name);
                        } else {
                            a2l_name = var_name.to_string();
                        }
                        info!(
                            "Local variable '{}' in function '{:?}', event id = {:?}, dwarf_offset = {} cfa = {}",
                            var_name,
                            var_function,
                            xcp_event_id,
                            (var_info.address.1 as i64 - 0x80000000) ,
                            cfa
                        );

                        // @@@@ TODO: Create functions instead of constants for relative address encoding
                        // Encode dyn addressing mode A2L/XCP address from offset and event id
                        let offset: i64 = var_info.address.1 as i64 - 0x80000000 + cfa;
                        if offset < -(McAddress::XCP_ADDR_EXT_DYN_OFFSET_OFFSET as i64)
                            || offset > (McAddress::XCP_ADDR_EXT_DYN_OFFSET_MASK as i64 - McAddress::XCP_ADDR_EXT_DYN_OFFSET_OFFSET as i64)
                        {
                            warn!(
                                "Local variable '{}' skipped, has offset {} which does not fit the XCP dynamic addressing mode range",
                                var_name, offset
                            );
                            continue; // skip this variable
                        }

                        (((offset + McAddress::XCP_ADDR_EXT_DYN_OFFSET_OFFSET as i64) as u64) & McAddress::XCP_ADDR_EXT_DYN_OFFSET_MASK as u64)
                            | ((event.id as u64) << McAddress::XCP_ADDR_EXT_DYN_OFFSET_BITS)
                    } else {
                        warn!("Local variable '{}' in function {:?} skipped, could not find event for dyn addressing mode", var_name, var_function);
                        continue; // skip this variable
                    }
                }

                // @@@@ TODO: Handle other address extensions
                else {
                    debug!("Variable '{}' skipped, has unsupported address extension {:#x}", var_name, mem_addr_ext);
                    continue; // skip this variable
                };

                // Check if the absolute address is in a calibration segment or block
                // For segments with segment relative and absolute addressing mode, we always need to check with the memory address of the segment, not the a2l address
                let seg_name = reg.cal_seg_list.find_cal_seg_by_mem_address(mem_addr);
                let (object_type, mc_addr) = if let Some(seg_name) = seg_name {
                    let seg = reg.cal_seg_list.find_cal_seg(&seg_name).unwrap();
                    let offset: u16 = (mem_addr - seg.mem_addr).try_into().unwrap();
                    // Address extension of characteristics in memory segments is always 0, hardcoded here
                    // @@@@ NOTE: This might change in the future
                    (McObjectType::Characteristic, McAddress::new_a2l(seg.addr + offset as u32, 0))
                } else {
                    // Create a McAddress with event id, mem_addr is relative or absolute
                    // @@@@ TODO: Not implemented dependency on target addressing scheme
                    // Address extension might be 0, 1, 2 depending on the target addressing scheme
                    let addr_ext = if seg_relative && mem_addr_ext == 0 {
                        1 // set to absolute addressing mode
                    } else {
                        mem_addr_ext
                    };
                    if let Some(xcp_event_id) = xcp_event_id {
                        (McObjectType::Measurement, McAddress::new_a2l_with_event(xcp_event_id, mem_addr as u32, addr_ext))
                    } else {
                        (McObjectType::Measurement, McAddress::new_a2l(mem_addr as u32, addr_ext))
                    }
                };

                // Register measurement variable if possible
                if let Some(type_info) = self.debug_data.types.get(&var_info.typeref) {
                    // Register supported variable types in the registry
                    let type_size = type_info.get_size();
                    let type_name = &type_info.name;
                    match &type_info.datatype {
                        DbgDataType::Uint8
                        | DbgDataType::Uint16
                        | DbgDataType::Uint32
                        | DbgDataType::Uint64
                        | DbgDataType::Sint8
                        | DbgDataType::Sint16
                        | DbgDataType::Sint32
                        | DbgDataType::Sint64
                        | DbgDataType::Float
                        | DbgDataType::Double
                        | DbgDataType::Array { .. }
                        | DbgDataType::Struct { .. } => {
                            if verbose >= 2 {
                                print!(
                                    "  Add {} instance for {}: addr = {}:0x{:08x}",
                                    if object_type == McObjectType::Characteristic { "characteristic" } else { "measurement" },
                                    a2l_name,
                                    mem_addr_ext,
                                    mem_addr
                                );
                                if verbose >= 3 {
                                    println!(" type = {}", type_info);
                                } else {
                                    println!();
                                }
                            }
                            let dim_type = self.get_dim_type(reg, type_info, object_type);
                            let res = reg.instance_list.add_instance(a2l_name.clone(), dim_type, McSupportData::new(object_type), mc_addr);
                            match res {
                                Ok(_) => {
                                    if verbose >= 1 {
                                        println!(
                                            "  Registered variable '{}' type_name = '{}', size = {}, event_id = {:?}",
                                            a2l_name,
                                            type_name.as_ref().unwrap_or(&"<unnamed>".to_string()),
                                            type_size,
                                            xcp_event_id
                                        );
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to register variable '{}': {}", a2l_name, e);
                                }
                            }
                        }
                        // Special case for enum types, which are represented as integer types with enumerators described as special unit format "value "NAME" value "NAME" ...".
                        // We convert the enumerators to a unit string and store it in the McSupportData for the instance.
                        DbgDataType::Enum { size, signed, enumerators } => {
                            if verbose >= 2 {
                                print!(
                                    "  Add {} instance for enum {}: addr = {}:0x{:08x}, size = {}, signed = {}, enumerators = {:?}",
                                    if object_type == McObjectType::Characteristic { "characteristic" } else { "measurement" },
                                    a2l_name,
                                    mem_addr_ext,
                                    mem_addr,
                                    size,
                                    signed,
                                    enumerators
                                );
                                if verbose >= 3 {
                                    println!(" type = {}", type_info);
                                } else {
                                    println!();
                                }
                            }
                            let dim_type = self.get_dim_type(reg, type_info, object_type);
                            let unit_string = enumerators_to_unit_string(enumerators);
                            let mc_support_data = if let Some(unit_str) = unit_string {
                                McSupportData::new(object_type).set_unit(unit_str)
                            } else {
                                warn!("Enum variable '{}' has no enumerators, no conversion table generated", a2l_name);
                                McSupportData::new(object_type)
                            };
                            let res = reg.instance_list.add_instance(a2l_name.clone(), dim_type, mc_support_data, mc_addr);
                            match res {
                                Ok(_) => {
                                    if verbose >= 1 {
                                        println!(
                                            "Registered enum variable '{}' with type '{}', size = {}, event id = {:?}, unit = {:?}",
                                            a2l_name,
                                            type_name.as_ref().unwrap_or(&"<unnamed>".to_string()),
                                            type_size,
                                            xcp_event_id,
                                            enumerators_to_unit_string(enumerators)
                                        );
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to register variable '{}': {}", a2l_name, e);
                                }
                            }
                        }

                        _ => {
                            warn!("Variable '{}' has unsupported type: {}", var_name, type_info);
                        }
                    }
                } else {
                    warn!("TypeRef {} of variable '{}' not found in debug info", var_info.typeref, var_name);
                }
            }
        } // var_infos
        Ok(())
    }

    /// Read XCP_UNIT / XCP_LIMITS / XCP_COMMENT metadata from the xcp_meta ELF section
    /// and apply them to already-registered instances in the registry.
    /// Must be called after register_variables.
    pub fn register_metadata(&self, reg: &mut Registry, verbose: usize) -> Result<(), Box<dyn Error>> {
        info!("===============================================================");
        info!("Registering metadata from xcp_meta section:");
        info!("===============================================================");

        // Get meta_base_addr and meta_end
        let (meta_base_addr, meta_data) = match &self.debug_data.xcp_meta_data {
            Some(data) => data,
            None => {
                info!("No xcp_meta section found, skipping metadata registration");
                return Ok(());
            }
        };
        let meta_end = meta_base_addr + meta_data.len() as u64;
        let is_le = self.debug_data.is_little_endian;
        assert!(is_le, "Big endian is not supported for meta data registration");

        // Search for metadata variables (xcp_meta__<kind>__<base_name>) in the debug data
        // Add meta data to the registry instances
        for (var_name, var_infos) in &self.debug_data.variables {
            // Only process metadata variables
            let Some(rest) = var_name.strip_prefix("xcp_meta__") else {
                continue;
            };
            let Some((kind, base_name)) = rest.split_once("__") else {
                warn!("Unexpected xcp_meta__ variable name format: '{}'", var_name);
                continue;
            };
            if var_infos.is_empty() {
                continue;
            }

            // Every marker with this name is processed separately: markers without scope prefix in different namespaces or functions
            // (XCP_COMMENT(input, ...) in namespace motor_control and in namespace valve_control) share the DWARF name.
            // A declaration entry without address (GCC declaration/definition pairs) is skipped.
            let markers: Vec<&VarInfo> = var_infos.iter().filter(|v| v.address.0 == 0 && v.address.1 != 0).collect();
            if markers.is_empty() {
                warn!("Metadata variable '{}' address is 0", var_name);
                continue;
            }
            for marker in markers {
                // Get the section offset of the metadata variable
                let var_addr = marker.address.1;
                if var_addr < *meta_base_addr || var_addr >= meta_end {
                    warn!("Metadata variable '{}' address 0x{:08X} is outside xcp_meta section", var_name, var_addr);
                    continue;
                }
                let offset = (var_addr - meta_base_addr) as usize;

                // Decode base_name: __ is the path separator, e.g. "params__delay_us" means
                // instance "params", field "delay_us".  Replace all __ with . to get the dot path.
                let dot_path = base_name.replace("__", ".");

                // The name is looked up qualified with the scope of the metadata marker first, then unqualified:
                // a marker in the same namespace as the variable (XCP_COMMENT(input, ...) in namespace motor_control) refers to motor_control.input,
                // a marker in the same function as a local variable (XCP_COMMENT(counter, ...) in foo) refers to foo.counter.
                // The unqualified lookup keeps markers with an explicit prefix (foo__counter) and markers at file scope working.
                let scope = match &marker.function {
                    Some(function) => function.clone(),
                    None => marker.namespaces.join("."),
                };
                let mut candidates: Vec<String> = Vec::with_capacity(2);
                if !scope.is_empty() {
                    candidates.push(format!("{scope}.{dot_path}"));
                }
                candidates.push(dot_path);

                let mut applied = false;
                for path in &candidates {
                    // Path A — typedef field metadata (instance + dot-separated field path)
                    // Applies when base_name contains __, i.e. it encodes a struct field reference.
                    // The instance name may contain dots itself (namespace qualified instances like motor_control.input), so every
                    // split into instance name and field path is tried, the longest instance name first.
                    // Uses set_instance_field_support_data which walks the typedef tree.
                    for (split, _) in path.rmatch_indices('.') {
                        let (instance_name, field_path) = (&path[..split], &path[split + 1..]);
                        if apply_field_metadata(reg, var_name, kind, instance_name, field_path, meta_data, offset, is_le, verbose) {
                            applied = true;
                            break;
                        }
                    }

                    // Path B — direct instance metadata (simple variable or flattened typedef)
                    // Matches instances whose A2L name equals the path or ends with ".{path}".
                    // The path already has . separators so it matches both "delay_us" and "params.delay_us".
                    let escaped = path.replace('.', "\\.");
                    let pattern = format!(r"^(.*\.)?{}$", escaped);
                    let names: Vec<String> = reg.instance_list.find_instances_regex(&pattern, McObjectType::Unspecified, None);
                    for name in &names {
                        if let Some(inst) = reg.instance_list.get_instance_mut(name, None) {
                            apply_instance_metadata(inst, kind, meta_data, offset, is_le);
                            applied = true;
                            if verbose >= 1 {
                                println!("  Metadata {} {} applied to instance '{}'", kind, var_name, name);
                            }
                        }
                    }
                    if applied {
                        break;
                    }
                }

                if !applied {
                    warn!("Metadata '{}': no matching registry entry for '{}'", var_name, candidates.join("' or '"));
                }
            }
        }

        Ok(())
    }
}

// Convert an enumerators vec to the XCP/A2L COMPU_VTAB string format: `value "NAME" value "NAME" ...`
fn enumerators_to_unit_string(enumerators: &[(String, i64)]) -> Option<String> {
    if enumerators.is_empty() {
        return None;
    }
    let parts: Vec<String> = enumerators.iter().map(|(name, value)| format!(r#"{} "{}""#, value, name)).collect();
    Some(parts.join(" "))
}

// Read a null-terminated UTF-8 string from a byte slice at a given offset
fn read_cstr_at(data: &[u8], offset: usize) -> Option<String> {
    if offset >= data.len() {
        return None;
    }
    let end = data[offset..].iter().position(|&b| b == 0).map(|p| offset + p).unwrap_or(data.len());
    String::from_utf8(data[offset..end].to_vec()).ok()
}

// Path A helper: apply metadata to a typedef field via set_instance_field_support_data.
// Returns true if the metadata was successfully applied.
fn apply_field_metadata(
    reg: &mut Registry,
    var_name: &str,
    kind: &str,
    instance_name: &str,
    field_path: &str,
    meta_data: &[u8],
    offset: usize,
    is_le: bool,
    verbose: usize,
) -> bool {
    let support_data = match kind {
        "unit" | "comment" => {
            let Some(value) = read_cstr_at(meta_data, offset) else {
                warn!("Failed to read string for metadata variable '{}'", var_name);
                return false;
            };
            let sd = McSupportData::new(McObjectType::Unspecified);
            if kind == "unit" { sd.set_unit(value) } else { sd.set_comment(value) }
        }
        "min" | "max" => {
            if offset + 8 > meta_data.len() {
                warn!("Not enough bytes for f64 at offset {} in xcp_meta for '{}'", offset, var_name);
                return false;
            }
            let bytes: [u8; 8] = meta_data[offset..offset + 8].try_into().unwrap();
            let value = if is_le { f64::from_le_bytes(bytes) } else { f64::from_be_bytes(bytes) };
            let sd = McSupportData::new(McObjectType::Unspecified);
            if kind == "min" { sd.set_min(Some(value)) } else { sd.set_max(Some(value)) }
        }
        "read_write" => {
            let sd = McSupportData::new(McObjectType::Unspecified);
            sd.set_read_write()
        }
        _ => {
            warn!("Unknown metadata kind '{}' in variable '{}'", kind, var_name);
            return false;
        }
    };

    match reg.set_instance_field_support_data(instance_name, field_path, support_data) {
        Ok(()) => {
            if verbose >= 1 {
                println!("  Metadata {} applied to typedef field '{}.{}'", var_name, instance_name, field_path);
            }
            true
        }
        Err(RegistryError::NotFound(_)) => false, // no such instance or field — not an error, Path B will try
        Err(e) => {
            warn!("Metadata '{}': set_instance_field_support_data failed: {}", var_name, e);
            false
        }
    }
}

// Path B helper: apply metadata directly to an McInstance's mc_support_data.
fn apply_instance_metadata(inst: &mut xcp_registry::McInstance, kind: &str, meta_data: &[u8], offset: usize, is_le: bool) {
    match kind {
        "read_write" => {
            inst.mc_support_data.update_qualifier(McObjectQualifier::ReadWrite);
        }
        "unit" | "comment" => {
            if let Some(value) = read_cstr_at(meta_data, offset) {
                if kind == "unit" {
                    inst.mc_support_data.update_unit(value);
                } else {
                    inst.mc_support_data.update_comment(value);
                }
            }
        }
        "min" | "max" => {
            if offset + 8 <= meta_data.len() {
                let bytes: [u8; 8] = meta_data[offset..offset + 8].try_into().unwrap();
                let value = if is_le { f64::from_le_bytes(bytes) } else { f64::from_be_bytes(bytes) };
                if kind == "min" {
                    inst.mc_support_data.update_min(Some(value));
                } else {
                    inst.mc_support_data.update_max(Some(value));
                }
            }
        }
        _ => {}
    }
}

//------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod test {
    use super::*;

    // C++ type test fixture, see fixtures/cpp_types.cpp (GCC 12.3 arm-none-eabi, DWARF 5)
    const CPP_TYPES_ELF: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/cpp_types.elf");
    // C++ namespace test fixture, see fixtures/cpp_namespaces.cpp (GCC 12.3 arm-none-eabi, DWARF 5)
    const CPP_NAMESPACES_ELF: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/cpp_namespaces.elf");

    // Load a fixture ELF file and register all its variables
    fn load_fixture(elf_file: &str) -> (ElfReader, Registry) {
        let elf_reader = ElfReader::new(elf_file, 0, usize::MAX).unwrap_or_else(|| panic!("failed to load {elf_file}"));
        let mut reg = Registry::new();
        elf_reader.register_variables(&mut reg, false, 0, usize::MAX, "", "").expect("register_variables failed");
        (elf_reader, reg)
    }

    fn load_cpp_types() -> Registry {
        load_fixture(CPP_TYPES_ELF).1
    }

    // Name of the typedef referenced by a measurement instance
    fn instance_typedef(reg: &Registry, var_name: &str) -> &'static str {
        reg.instance_list
            .get_instance(var_name, McObjectType::Measurement, None)
            .unwrap_or_else(|| panic!("instance '{var_name}' not registered"))
            .get_typedef_name()
            .unwrap_or_else(|| panic!("instance '{var_name}' does not reference a typedef"))
    }

    // Names and offsets of the fields of a typedef
    fn typedef_fields(reg: &Registry, typedef_name: &str) -> Vec<(&'static str, u16)> {
        let typedef = reg
            .typedef_list
            .find_typedef(typedef_name)
            .unwrap_or_else(|| panic!("typedef '{typedef_name}' not registered"));
        typedef.fields.iter().map(|f| (f.get_name(), f.offset)).collect()
    }

    // Struct types with the same name in different namespaces get namespace qualified typedef names and the instances reference them,
    // the same type used by several variables is registered once
    #[test]
    fn test_register_same_named_types_in_namespaces() {
        let (_, reg) = load_fixture(CPP_NAMESPACES_ELF);

        // different size
        assert_eq!(instance_typedef(&reg, "motor_control.input"), "motor_control.Input");
        assert_eq!(instance_typedef(&reg, "valve_control.input"), "valve_control.Input");
        assert_eq!(reg.typedef_list.find_typedef("motor_control.Input").unwrap().size, 4);
        assert_eq!(reg.typedef_list.find_typedef("valve_control.Input").unwrap().size, 8);
        assert_eq!(typedef_fields(&reg, "motor_control.Input"), vec![("speed", 0)]);
        assert_eq!(typedef_fields(&reg, "valve_control.Input"), vec![("flow", 0), ("pressure", 4)]);
        assert!(reg.typedef_list.find_typedef("Input").is_none());

        // same size, different member names
        assert_eq!(instance_typedef(&reg, "motor_control.state"), "motor_control.State");
        assert_eq!(instance_typedef(&reg, "valve_control.state"), "valve_control.State");
        assert_eq!(typedef_fields(&reg, "motor_control.State"), vec![("rpm", 0)]);
        assert_eq!(typedef_fields(&reg, "valve_control.State"), vec![("position", 0)]);

        // nested namespace, and the same type used by a variable in another namespace
        assert_eq!(instance_typedef(&reg, "diagnostics.detail.input"), "diagnostics.detail.Input");
        assert_eq!(typedef_fields(&reg, "diagnostics.detail.Input"), vec![("raw", 0)]);
        assert_eq!(instance_typedef(&reg, "last_motor_input"), "motor_control.Input");
        assert_eq!(reg.typedef_list.iter().filter(|t| t.name.as_str().ends_with("Input")).count(), 3);
    }

    // Struct types with the same name nested in different classes get class qualified typedef names
    #[test]
    fn test_register_same_named_nested_types() {
        let (_, reg) = load_fixture(CPP_NAMESPACES_ELF);

        let motor = reg.typedef_list.find_typedef(instance_typedef(&reg, "motor_controller")).unwrap();
        assert_eq!(motor.find_field("params").unwrap().get_typedef_name(), Some("MotorController.Params"));
        let valve = reg.typedef_list.find_typedef(instance_typedef(&reg, "valve_controller")).unwrap();
        assert_eq!(valve.find_field("params").unwrap().get_typedef_name(), Some("ValveController.Params"));
        assert_eq!(typedef_fields(&reg, "MotorController.Params"), vec![("gain", 0)]);
        assert_eq!(typedef_fields(&reg, "ValveController.Params"), vec![("gain", 0), ("offset", 4)]);
        assert!(reg.typedef_list.find_typedef("Params").is_none());
    }

    // Types with a unique name keep their plain name, also inside a namespace.
    // A type outside of any namespace keeps its plain name when a namespaced type has the same name.
    #[test]
    fn test_register_unique_type_names_unchanged() {
        let (_, reg) = load_fixture(CPP_NAMESPACES_ELF);

        assert_eq!(instance_typedef(&reg, "output"), "Output");
        assert_eq!(instance_typedef(&reg, "config"), "Config");
        assert_eq!(instance_typedef(&reg, "valve_control.config"), "valve_control.Config");
        assert_eq!(reg.typedef_list.find_typedef("Config").unwrap().size, 4);
        assert_eq!(reg.typedef_list.find_typedef("valve_control.Config").unwrap().size, 8);
    }

    // Empty debug data for hand-made test cases
    fn empty_debug_data() -> DebugData {
        DebugData {
            variables: IndexMap::new(),
            types: HashMap::new(),
            typenames: HashMap::new(),
            type_scopes: HashMap::new(),
            demangled_names: HashMap::new(),
            unit_names: vec![Some("main.c".to_string())],
            sections: HashMap::new(),
            symbol_addresses: HashMap::new(),
            cfa_info: Vec::new(),
            epk_string: None,
            epk_addr: 0,
            xcp_meta_data: None,
            is_little_endian: true,
        }
    }

    // Debug data with two struct types named "state" with different members and without scope (e.g. file local types in two C files)
    fn elf_reader_with_conflicting_types() -> ElfReader {
        let make_struct = |offset: usize, unit_idx: usize, member: &str| {
            let member_type = TypeInfo {
                name: Some("uint32_t".to_string()),
                unit_idx,
                datatype: DbgDataType::Uint32,
                dbginfo_offset: offset + 1,
            };
            TypeInfo {
                name: Some("state".to_string()),
                unit_idx,
                datatype: DbgDataType::Struct {
                    size: 4,
                    is_class: false,
                    inheritance: IndexMap::new(),
                    members: IndexMap::from([(member.to_string(), (member_type, 0u64))]),
                },
                dbginfo_offset: offset,
            }
        };
        let var_info = |address: u64, typeref: usize, unit_idx: usize| VarInfo {
            address: (0, address),
            typeref,
            unit_idx,
            function: None,
            namespaces: Vec::new(),
        };
        let mut debug_data = empty_debug_data();
        debug_data.unit_names = vec![Some("a.c".to_string()), Some("b.c".to_string())];
        debug_data.types.insert(0x10, make_struct(0x10, 0, "a"));
        debug_data.types.insert(0x20, make_struct(0x20, 1, "b"));
        debug_data.typenames.insert("state".to_string(), vec![0x10, 0x20]);
        debug_data.variables.insert("state_a".to_string(), vec![var_info(0x1000, 0x10, 0)]);
        debug_data.variables.insert("state_b".to_string(), vec![var_info(0x2000, 0x20, 1)]);
        ElfReader::from_debug_data(debug_data)
    }

    // Different types with the same name and without scope get a numeric suffix,
    // a type used for measurement and for calibration variables gets two typedefs
    #[test]
    fn test_register_conflicting_typedef_names() {
        let elf = elf_reader_with_conflicting_types();
        let mut reg = Registry::new();
        elf.register_variables(&mut reg, false, 0, usize::MAX, "", "").unwrap();
        assert_eq!(instance_typedef(&reg, "state_a"), "state");
        assert_eq!(instance_typedef(&reg, "state_b"), "state_1");
        assert_eq!(typedef_fields(&reg, "state"), vec![("a", 0)]);
        assert_eq!(typedef_fields(&reg, "state_1"), vec![("b", 0)]);

        // The same type registered again for a measurement reuses the typedef
        let type_info = elf.debug_data.types.get(&0x10).unwrap();
        assert_eq!(
            elf.get_dim_type(&mut reg, type_info, McObjectType::Measurement).value_type,
            McValueType::new_typedef("state")
        );
        // Registered for a characteristic it is a different typedef (TYPEDEF_CHARACTERISTIC instead of TYPEDEF_MEASUREMENT components)
        assert_eq!(
            elf.get_dim_type(&mut reg, type_info, McObjectType::Characteristic).value_type,
            McValueType::new_typedef("state_2")
        );
        let field = reg.typedef_list.find_typedef("state_2").unwrap().find_field("a").unwrap();
        assert_eq!(field.get_mc_support_data().get_object_type(), McObjectType::Characteristic);
        assert_eq!(reg.typedef_list.len(), 3);
    }

    // Metadata markers for a namespace qualified instance (XCP_COMMENT(motor_control__input, ...)) and for a field of its typedef
    #[test]
    fn test_register_metadata_namespaced_instance() {
        let meta_base: u64 = 0xF000;
        let meta: Vec<u8> = b"Motor input\0rpm\0".to_vec(); // comment at offset 0, unit at offset 12
        let marker = |addr: u64| {
            vec![VarInfo {
                address: (0, addr),
                typeref: 0,
                unit_idx: 0,
                function: None,
                namespaces: vec!["motor_control".to_string()],
            }]
        };
        let mut debug_data = empty_debug_data();
        debug_data.xcp_meta_data = Some((meta_base, meta));
        debug_data.variables.insert("xcp_meta__comment__motor_control__input".to_string(), marker(meta_base));
        debug_data
            .variables
            .insert("xcp_meta__unit__motor_control__input__speed".to_string(), marker(meta_base + 12));
        let elf = ElfReader::from_debug_data(debug_data);

        let mut reg = Registry::new();
        reg.add_typedef("motor_control.Input", 4).unwrap();
        reg.add_typedef_field(
            "motor_control.Input",
            "speed",
            McDimType::new(McValueType::Slong, 1, 1),
            McSupportData::new(McObjectType::Measurement),
            0,
        )
        .unwrap();
        reg.instance_list
            .add_instance(
                "motor_control.input",
                McDimType::new(McValueType::new_typedef("motor_control.Input"), 1, 1),
                McSupportData::new(McObjectType::Measurement),
                McAddress::new_a2l(0x30388, 0),
            )
            .unwrap();
        elf.register_metadata(&mut reg, 0).unwrap();

        let instance = reg.instance_list.get_instance("motor_control.input", McObjectType::Measurement, None).unwrap();
        assert_eq!(instance.comment(), "Motor input");
        let field = reg.typedef_list.find_typedef("motor_control.Input").unwrap().find_field("speed").unwrap();
        assert_eq!(field.get_mc_support_data().get_unit(), "rpm");
    }

    // Metadata markers without scope prefix and with the same DWARF name in different namespaces are applied to their own instance each
    #[test]
    fn test_register_metadata_same_marker_name_in_namespaces() {
        let meta_base: u64 = 0xF000;
        let meta: Vec<u8> = b"Motor input\0Valve input\0".to_vec(); // offsets 0 and 12
        let marker = |addr: u64, namespace: &str| VarInfo {
            address: (0, addr),
            typeref: 0,
            unit_idx: 0,
            function: None,
            namespaces: vec![namespace.to_string()],
        };
        let mut debug_data = empty_debug_data();
        debug_data.xcp_meta_data = Some((meta_base, meta));
        debug_data.variables.insert(
            "xcp_meta__comment__input".to_string(),
            vec![marker(meta_base, "motor_control"), marker(meta_base + 12, "valve_control")],
        );
        let elf = ElfReader::from_debug_data(debug_data);

        let mut reg = Registry::new();
        let measurement = || McSupportData::new(McObjectType::Measurement);
        let ulong = || McDimType::new(McValueType::Ulong, 1, 1);
        reg.instance_list
            .add_instance("motor_control.input", ulong(), measurement(), McAddress::new_a2l(0x30388, 0))
            .unwrap();
        reg.instance_list
            .add_instance("valve_control.input", ulong(), measurement(), McAddress::new_a2l(0x30390, 0))
            .unwrap();
        elf.register_metadata(&mut reg, 0).unwrap();

        let comment = |name: &str| reg.instance_list.get_instance(name, McObjectType::Measurement, None).unwrap().comment();
        assert_eq!(comment("motor_control.input"), "Motor input");
        assert_eq!(comment("valve_control.input"), "Valve input");
    }

    // Metadata markers without scope prefix: a marker in the same namespace as the variable refers to the namespace qualified instance,
    // a marker in the same function as a local variable refers to the function qualified instance and not to other variables with this name
    #[test]
    fn test_register_metadata_marker_scope() {
        let meta_base: u64 = 0xF000;
        let meta: Vec<u8> = b"Motor input\0rpm\0Counter in foo\0".to_vec(); // offsets 0, 12 and 16
        let marker = |addr: u64, function: Option<&str>, namespaces: &[&str]| {
            vec![VarInfo {
                address: (0, addr),
                typeref: 0,
                unit_idx: 0,
                function: function.map(str::to_string),
                namespaces: namespaces.iter().map(|s| s.to_string()).collect(),
            }]
        };
        let mut debug_data = empty_debug_data();
        debug_data.xcp_meta_data = Some((meta_base, meta));
        debug_data
            .variables
            .insert("xcp_meta__comment__input".to_string(), marker(meta_base, None, &["motor_control"]));
        debug_data
            .variables
            .insert("xcp_meta__unit__input__speed".to_string(), marker(meta_base + 12, None, &["motor_control"]));
        debug_data
            .variables
            .insert("xcp_meta__comment__counter".to_string(), marker(meta_base + 16, Some("foo"), &[]));
        let elf = ElfReader::from_debug_data(debug_data);

        let mut reg = Registry::new();
        reg.add_typedef("motor_control.Input", 4).unwrap();
        reg.add_typedef_field(
            "motor_control.Input",
            "speed",
            McDimType::new(McValueType::Slong, 1, 1),
            McSupportData::new(McObjectType::Measurement),
            0,
        )
        .unwrap();
        let measurement = || McSupportData::new(McObjectType::Measurement);
        let input_type = || McDimType::new(McValueType::new_typedef("motor_control.Input"), 1, 1);
        let uword = || McDimType::new(McValueType::Uword, 1, 1);
        reg.instance_list
            .add_instance("motor_control.input", input_type(), measurement(), McAddress::new_a2l(0x30388, 0))
            .unwrap();
        reg.instance_list
            .add_instance("valve_control.input", uword(), measurement(), McAddress::new_a2l(0x30390, 0))
            .unwrap();
        reg.instance_list
            .add_instance("foo.counter", uword(), measurement(), McAddress::new_a2l(0x30400, 0))
            .unwrap();
        reg.instance_list
            .add_instance("main.counter", uword(), measurement(), McAddress::new_a2l(0x30402, 0))
            .unwrap();
        elf.register_metadata(&mut reg, 0).unwrap();

        let comment = |name: &str| reg.instance_list.get_instance(name, McObjectType::Measurement, None).unwrap().comment();
        assert_eq!(comment("motor_control.input"), "Motor input");
        assert_eq!(comment("valve_control.input"), "");
        assert_eq!(comment("foo.counter"), "Counter in foo");
        assert_eq!(comment("main.counter"), "");
        let field = reg.typedef_list.find_typedef("motor_control.Input").unwrap().find_field("speed").unwrap();
        assert_eq!(field.get_mc_support_data().get_unit(), "rpm");
    }

    // Variables and struct members of C++ class type are registered like structs
    #[test]
    fn test_register_class_types() {
        let reg = load_cpp_types();

        for (var_name, typedef_name) in [
            ("g_pubclass", "PubClass"),
            ("g_tpl_class", "TplClass_long_unsigned_int_"),
            ("g_derived_cc", "DerivedCC"),
            ("g_derived_cs", "DerivedCS"),
            ("g_derived_ss", "DerivedSS"),
            ("g_derived_sc", "DerivedSC"),
        ] {
            let inst = reg
                .instance_list
                .get_instance(var_name, McObjectType::Measurement, None)
                .unwrap_or_else(|| panic!("instance '{var_name}' not registered"));
            assert_eq!(inst.dim_type.value_type, McValueType::new_typedef(typedef_name), "{var_name}");
        }

        let pub_class = reg.typedef_list.find_typedef("PubClass").expect("typedef PubClass");
        assert_eq!(pub_class.size, 8);
        assert_eq!(pub_class.find_field("x").map(|f| f.offset), Some(0));
        assert_eq!(pub_class.find_field("y").map(|f| f.offset), Some(4));

        // Inherited members of a class derived from a class are flattened by the DWARF reader
        let derived_cc = reg.typedef_list.find_typedef("DerivedCC").expect("typedef DerivedCC");
        assert_eq!(derived_cc.find_field("cbase_a").map(|f| f.offset), Some(0));
        assert_eq!(derived_cc.find_field("cderived_b").map(|f| f.offset), Some(4));

        // A class typed struct member references the class typedef instead of degrading to UBYTE
        let outer = reg.typedef_list.find_typedef("Outer").expect("typedef Outer");
        let inner_class = outer.find_field("inner_class").expect("field Outer.inner_class");
        assert_eq!(inner_class.dim_type.value_type, McValueType::new_typedef("PubClass"));
        assert_eq!(inner_class.offset, 0);
    }

    // Base class members are flattened into the derived type for all struct/class combinations
    #[test]
    fn test_register_inherited_members() {
        let reg = load_cpp_types();

        for (typedef_name, base_member, derived_member) in [
            ("DerivedSS", "base_a", "derived_b"),
            ("DerivedCC", "cbase_a", "cderived_b"),
            ("DerivedCS", "base_a", "cs_b"),
            ("DerivedSC", "cbase_a", "sc_b"),
        ] {
            let typedef = reg
                .typedef_list
                .find_typedef(typedef_name)
                .unwrap_or_else(|| panic!("typedef '{typedef_name}' not registered"));
            assert_eq!(typedef.size, 8, "{typedef_name}");
            assert_eq!(typedef.fields.len(), 2, "{typedef_name} member count");
            assert_eq!(typedef.find_field(base_member).map(|f| f.offset), Some(0), "{typedef_name}.{base_member}");
            assert_eq!(typedef.find_field(derived_member).map(|f| f.offset), Some(4), "{typedef_name}.{derived_member}");
        }
    }

    // Typedef names which are sanitized (template instantiations) get their members and a matching reference
    #[test]
    fn test_register_template_struct_members() {
        let reg = load_cpp_types();

        for typedef_name in ["TplStruct_short_unsigned_int_", "TplStruct_float_", "TplClass_long_unsigned_int_"] {
            let typedef = reg
                .typedef_list
                .find_typedef(typedef_name)
                .unwrap_or_else(|| panic!("typedef '{typedef_name}' not registered"));
            assert_eq!(typedef.size, 8, "{typedef_name}");
            assert_eq!(typedef.fields.len(), 2, "{typedef_name} has no members");
            assert_eq!(typedef.find_field("value").map(|f| f.offset), Some(0), "{typedef_name}.value");
            assert_eq!(typedef.find_field("count").map(|f| f.offset), Some(4), "{typedef_name}.count");
        }

        let outer = reg.typedef_list.find_typedef("Outer").unwrap();
        let inner_tpl = outer.find_field("inner_tpl").expect("field Outer.inner_tpl");
        assert_eq!(inner_tpl.dim_type.value_type, McValueType::new_typedef("TplStruct_short_unsigned_int_"));
        assert_eq!(inner_tpl.offset, 8);
    }

    // Build an ElfReader from hand-made debug data containing only event definition (evt__) and trigger (trg__) marker variables
    fn elf_reader_with_markers(markers: &[(&str, u64, &str)], event_section: Option<(u64, u64)>) -> ElfReader {
        let mut debug_data = empty_debug_data();
        for (name, addr, function) in markers {
            debug_data.variables.entry(name.to_string()).or_default().push(VarInfo {
                address: (0, *addr),
                typeref: 0,
                unit_idx: 0,
                function: Some(function.to_string()),
                namespaces: Vec::new(),
            });
        }
        if let Some(range) = event_section {
            debug_data.sections.insert("xcp_evts".to_string(), range);
        }
        ElfReader::from_debug_data(debug_data)
    }

    // An event created in several functions has several definition markers, the first descriptor in the section wins,
    // duplicate trigger markers must not panic either
    #[test]
    fn test_register_events_duplicate_definitions() {
        let elf = elf_reader_with_markers(
            &[
                ("evt__foo", 0x1010, "task_b"),
                ("evt__foo", 0x1000, "task_a"),
                ("evt__bar", 0x1020, "main"),
                ("trg__AAS__foo", 0x2000, "task_a"),
                ("trg__AAS__foo", 0x2004, "task_b"),
            ],
            Some((0x1000, 0x1030)),
        );
        let mut reg = Registry::new();
        elf.register_events(&mut reg, 0).unwrap();
        assert_eq!(reg.event_list.find_event("foo", 0).unwrap().get_id(), 0);
        assert_eq!(reg.event_list.find_event("bar", 0).unwrap().get_id(), 2);
        assert!(reg.event_list.find_event_id(1).is_none());
        elf.register_event_locations(&mut reg, 0).unwrap();
        assert!(reg.event_list.find_event_by_location(0, "task_a").is_some());
    }

    // Without an event descriptor section every event gets a unique placeholder id (previously all got 0xFFFF and the second one panicked)
    #[test]
    fn test_register_events_without_descriptor_section() {
        let elf = elf_reader_with_markers(&[("evt__foo", 0x1000, "main"), ("evt__bar", 0x1010, "main"), ("evt__baz", 0, "main")], None);
        let mut reg = Registry::new();
        elf.register_events(&mut reg, 0).unwrap();
        let ids: Vec<u16> = ["foo", "bar", "baz"].iter().map(|n| reg.event_list.find_event(n, 0).unwrap().get_id()).collect();
        assert_eq!(ids, vec![0xFFFF, 0xFFFE, 0xFFFD]);
    }

    // Markers outside the descriptor section, without address or with an id which is already taken get placeholder ids
    #[test]
    fn test_register_events_marker_outside_section() {
        let elf = elf_reader_with_markers(
            &[("evt__foo", 0x1000, "main"), ("evt__out", 0x5000, "main"), ("evt__zero", 0, "main")],
            Some((0x1000, 0x1010)),
        );
        let mut reg = Registry::new();
        reg.event_list.add_event(McEvent::new("srv", 0, 0, 0)).unwrap(); // id 0 is taken, e.g. by the XCP server event information
        elf.register_events(&mut reg, 0).unwrap();
        assert_eq!(reg.event_list.find_event("foo", 0).unwrap().get_id(), 0xFFFF);
        assert_eq!(reg.event_list.find_event("out", 0).unwrap().get_id(), 0xFFFE);
        assert_eq!(reg.event_list.find_event("zero", 0).unwrap().get_id(), 0xFFFD);
    }
}
