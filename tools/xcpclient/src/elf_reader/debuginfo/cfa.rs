// Get CFA (Canonical Frame Address) information from DWARF debug data in an ELF file
//
// Why this exists: a local variable on the stack has no fixed address. DWARF describes it as an offset relative to the
// "frame base" of its function (DW_OP_fbreg, see evaluate_exprloc in dwarf/attributes.rs), and DW_AT_frame_base says what
// the frame base is, usually DW_OP_call_frame_cfa: the CFA. The CFA is defined by the DWARF standard as the value of the
// stack pointer at the call site, before the call instruction pushed anything, so it is a fixed point of the frame no
// matter how the function moves its stack pointer later on.
//
// The XCPlite event trigger macro (DaqTriggerEvent in inc/xcplib.h) passes a frame address to the target at runtime,
// xcp_get_frame_addr(): __builtin_frame_address(0), on Xtensa __builtin_dwarf_cfa(), minus the constant XCP_FRAME_ADDR_OFFSET
// (0x10000, the same bias which the XCP dynamic address encoding adds back, see register_variables). The target adds the
// variable offsets from the A2L file to it. The frame pointer at the trigger point is not the CFA, so the difference has
// to be known when the A2L addresses are generated: cfa_offset. It is read from the call frame information
// (CFI), which is the unwind table the compiler emits for exception handling and debuggers: for every code address it
// says how to compute the CFA from the current registers, e.g. "CFA = SP + 24" once the function prologue has pushed
// its registers. On Xtensa (ESP32) the trigger macro passes the CFA itself and the offset is 0.
//
// The CFI lives in .eh_frame (the exception handling variant, most common) or .debug_frame (the pure debug variant),
// both with the same structure: CIEs (common information entries, shared initial rules) and FDEs (frame description
// entries, one per function, the rules for its address range as a sequence of "rows"). gimli parses both.
//
// Limitations: the CFA rule is taken from the rows of the whole function, the row valid at the trigger point is not known
// (the address of the trigger is not recorded in the marker), so the largest stack pointer based offset is used, which is
// the state after the prologue in typical code. The DWARF register numbers are architecture specific, the matching in
// parse_fde_for_cfa assumes the numbers of ARM, AArch64 and x86_64 and does not check which architecture the file is for.
// This is a separate DWARF pass with its own section loader because it was written independently of the reader in
// dwarf/mod.rs, both read the same .debug_info

use crate::elf_reader::debuginfo::dwarf::get_low_pc_attribute;
use anyhow::Result;
use gimli::{AttributeValue, BaseAddresses, CieOrFde, DebugFrame, Dwarf, EhFrame, EndianSlice, LittleEndian, Unit, UnwindSection};
use object::{Object, ObjectSection};

/// Represents CFA information for a function
#[derive(Debug, Clone)]
pub struct CfaInfo {
    /// Function name (DW_AT_name, the source name, not mangled)
    pub function: String,
    /// Low PC (start address of function)
    pub low_pc: u64,
    /// High PC (end address of function)
    pub high_pc: u64,
    /// CFA offset from stack pointer (if determinable): CFA = frame address at the trigger + cfa_offset, see module comment
    pub cfa_offset: Option<i64>,
    /// Compilation unit index, the unit and the function name together identify the function (register_event_locations)
    pub unit_idx: usize,
}

// Entry point: collect the CfaInfo of all functions with a name and an address range in the compilation units up to
// unit_idx_limit. Returns the number of functions found. Called from load_elf_dwarf before the variables are read
pub fn get_cfa_from_object(object_file: &object::File<'_>, cfa_info: &mut Vec<CfaInfo>, verbose: usize, unit_idx_limit: usize) -> Result<usize> {
    // Load DWARF sections - this is where all the debug information is stored

    log::debug!("get_cfa_from_object: CFA parser loading DWARF sections...");
    let dwarf = load_dwarf_sections(&object_file)?;

    // Extract function information from DWARF
    log::debug!("get_cfa_from_object: CFA parser extracting function information from DWARF...");
    let n = extract_function_info(&dwarf, &object_file, cfa_info, verbose, unit_idx_limit)?;
    if n == 0 {
        log::warn!("CFA parser: No functions found");
        return Ok(0);
    }
    log::debug!("CFA parser: Found {} functions:", n);
    Ok(n)
}

/// Load DWARF debug sections from the ELF file
///
/// DWARF information is stored in multiple sections:
/// - .debug_info: Contains the main debug information tree
/// - .debug_abbrev: Contains abbreviation tables for .debug_info
/// - .debug_str: Contains string tables
/// - .debug_line: Contains line number information
/// - .debug_frame/.eh_frame: Contains frame unwinding information (CFA data)
fn load_dwarf_sections<'a>(file: &'a object::File<'a>) -> Result<Dwarf<EndianSlice<'a, LittleEndian>>> {
    log::debug!("cfa::load_dwarf_sections");

    // Helper closure to load a section by name
    let load_section = |section_name: &str| -> EndianSlice<'a, LittleEndian> {
        log::debug!("cfa:: loading section: {}", section_name);
        let section_data = file.section_by_name(section_name).and_then(|section| section.data().ok()).unwrap_or(&[]);
        EndianSlice::new(section_data, LittleEndian)
    };

    // Create the DWARF context using the load method
    let dwarf = Dwarf::load(|id| -> Result<EndianSlice<'a, LittleEndian>, gimli::Error> {
        use gimli::SectionId;
        let section_name = match id {
            SectionId::DebugAbbrev => ".debug_abbrev",
            SectionId::DebugAddr => ".debug_addr",
            SectionId::DebugAranges => ".debug_aranges",
            SectionId::DebugInfo => ".debug_info",
            SectionId::DebugLine => ".debug_line",
            SectionId::DebugLineStr => ".debug_line_str",
            SectionId::DebugStr => ".debug_str",
            SectionId::DebugStrOffsets => ".debug_str_offsets",
            SectionId::DebugTypes => ".debug_types",
            SectionId::DebugLoc => ".debug_loc",
            SectionId::DebugLocLists => ".debug_loclists",
            SectionId::DebugRanges => ".debug_ranges",
            SectionId::DebugRngLists => ".debug_rnglists",
            _ => {
                return Ok(EndianSlice::new(&[], LittleEndian));
            }
        };
        Ok(load_section(section_name))
    })?;

    Ok(dwarf)
}

/// Extract function information from DWARF debug data
///
/// This function walks through all compilation units in the DWARF data
/// and extracts information about functions (subprograms in DWARF terminology).
/// For each function, we try to determine its name, address range, and CFA offset.
fn extract_function_info(
    dwarf: &Dwarf<EndianSlice<LittleEndian>>,
    object_file: &object::File,
    functions: &mut Vec<CfaInfo>,
    verbose: usize,
    unit_idx_limit: usize,
) -> Result<usize> {
    log::debug!("cfa::extract_function_info: Extracting function info from DWARF...");

    // Iterate through all compilation units
    // Each source file typically corresponds to one compilation unit
    let mut iter = dwarf.units();
    let mut cu_index = 0;

    while let Some(header) = iter.next()? {
        if cu_index > unit_idx_limit {
            continue;
        }
        log::debug!("\nProcessing Compilation Unit {}:", cu_index);

        // Get the unit from the header
        let unit = dwarf.unit(header)?;

        // Get the root DIE (Debug Information Entry) of this compilation unit
        let mut entries = unit.entries();

        // Skip the compilation unit DIE itself and process its children
        if let Some(entry) = entries.next_dfs()? {
            if entry.tag() == gimli::DW_TAG_compile_unit {
                if let Some(name) = get_string_attribute(dwarf, &unit, &entry, gimli::DW_AT_name)? {
                    log::debug!("Compilation unit name: {}", name);
                }
            }
        }

        // Process all DIEs in this compilation unit
        while let Some(entry) = entries.next_dfs()? {
            // We're looking for subprogram DIEs (functions)
            if entry.tag() == gimli::DW_TAG_subprogram {
                if let Some(func_info) = extract_function_from_die(dwarf, object_file, &unit, &entry, cu_index, verbose)? {
                    log::trace!("  Found function: {} at 0x{:08x}-0x{:08x}", func_info.function, func_info.low_pc, func_info.high_pc);
                    functions.push(func_info);
                }
            }
        }

        cu_index += 1;
    }

    Ok(functions.len())
}

/// Extract function information from a specific DWARF DIE (Debug Information Entry)
///
/// A DIE represents a program entity (variable, function, type, etc.) in DWARF.
/// For functions (DW_TAG_subprogram), we extract:
/// - Function name (DW_AT_name)
/// - Address range (DW_AT_low_pc, DW_AT_high_pc)
/// - Frame base information for CFA calculation
fn extract_function_from_die(
    dwarf: &Dwarf<EndianSlice<LittleEndian>>,
    object_file: &object::File,
    unit: &Unit<EndianSlice<LittleEndian>>,
    entry: &gimli::DebuggingInformationEntry<EndianSlice<LittleEndian>>,
    cu_index: usize,
    verbose: usize,
) -> Result<Option<CfaInfo>> {
    // Get the function name
    let name = match get_string_attribute(dwarf, unit, entry, gimli::DW_AT_name)? {
        Some(name) => name,
        None => {
            return Ok(None);
        }
    };

    // Get the low PC (start address), None for declarations, abstract instances of inlined functions and unresolved address forms
    let Some(low_pc) = get_low_pc_attribute(entry, &name, |index| dwarf.address(unit, index)) else {
        return Ok(None);
    };

    // Get the high PC (end address)
    // High PC can be either an absolute address or an offset from low PC
    let high_pc = match entry.attr_value(gimli::DW_AT_high_pc) {
        Some(AttributeValue::Addr(addr)) => addr,
        Some(AttributeValue::Udata(offset)) => low_pc + offset,
        _ => {
            return Ok(None);
        }
    };

    // Try to determine CFA offset
    // The frame base attribute tells us how to calculate the frame pointer
    let frame_base_offset = extract_frame_base_offset(entry)?;

    // Try to determine CFA offset from CFI (Call Frame Information)
    let cfa_offset = extract_cfa_from_cfi(object_file, low_pc)?;

    // Use CFA offset from CFI, fallback to frame base offset
    let final_cfa_offset = cfa_offset.or(frame_base_offset);

    if verbose >= 5 {
        print!("'{}': ", name);
        print!(" Frame base offset {:?}", frame_base_offset);
        print!(" CFI CFA offset for {:?}", cfa_offset);
        println!("  Final CFA offset {:?}", final_cfa_offset);
    }

    Ok(Some(CfaInfo {
        function: name,
        low_pc,
        high_pc,
        cfa_offset: final_cfa_offset,
        unit_idx: cu_index,
    }))
}

/// Extract a frame base offset from the DW_AT_frame_base attribute of a function DIE
///
/// DW_AT_frame_base is the expression which defines the frame base the local variables are relative to. Usually it is
/// just DW_OP_call_frame_cfa (offset 0, the CFA itself), older or unoptimized code may use a register (DW_OP_breg<n>).
/// Only the constant offset forms are decoded here, the result is the fallback if the CFI gives no CFA rule.
///
/// This is a simplified implementation - real CFA calculation can be quite complex
/// and may vary throughout a function's execution.
fn extract_frame_base_offset(entry: &gimli::DebuggingInformationEntry<EndianSlice<LittleEndian>>) -> Result<Option<i64>> {
    // Look for DW_AT_frame_base attribute
    // This attribute describes how to compute the frame base for local variables
    match entry.attr_value(gimli::DW_AT_frame_base) {
        Some(AttributeValue::Exprloc(expression)) => {
            // The frame base is described by a DWARF expression
            // For simple cases, this might be something like "DW_OP_call_frame_cfa + offset"

            log::debug!("    Frame base expression found (length: {} bytes): {:?}", expression.0.len(), expression.0);

            // Parse the expression to try to extract a constant offset
            // This is a simplified parser - real DWARF expressions can be very complex
            if let Some(offset) = parse_simple_cfa_expression(&expression.0) {
                return Ok(Some(offset));
            }

            // For more complex expressions, we might need to evaluate them
            // This would require a full DWARF expression evaluator
            log::debug!("    Complex frame base expression - cannot extract simple offset");
            Ok(None)
        }
        Some(other) => {
            log::debug!("    Frame base attribute has unexpected type: {:?}", other);
            Ok(None)
        }
        None => {
            log::debug!("    No frame base attribute found");
            Ok(None)
        }
    }
}

/// Extract CFA offset from Call Frame Information (CFI) using pure gimli
///
/// This function parses .eh_frame or .debug_frame sections using gimli
/// to find the CFA (Canonical Frame Address) calculation rule for a function.
fn extract_cfa_from_cfi(file: &object::File, function_address: u64) -> Result<Option<i64>> {
    // Xtensa (ESP32): the trigger macros pass the CFA itself as frame address (__builtin_dwarf_cfa() in xcplib.h) and the
    // DWARF locations of the local variables are relative to the CFA (DW_AT_frame_base DW_OP_call_frame_cfa), no offset is needed
    if file.architecture() == object::Architecture::Xtensa {
        log::debug!("    Xtensa: the frame address is the CFA, CFA offset 0");
        return Ok(Some(0));
    }

    // Try .eh_frame first (more common)
    if let Some(cfa_offset) = parse_eh_frame(file, function_address)? {
        // println!("    Found CFA offset in .eh_frame: {}", cfa_offset);
        return Ok(Some(cfa_offset));
    }

    // Fallback to .debug_frame
    if let Some(cfa_offset) = parse_debug_frame(file, function_address)? {
        // println!("    Found CFA offset in .debug_frame: {}", cfa_offset);
        return Ok(Some(cfa_offset));
    }

    Ok(None)
}

/// Parse .eh_frame section to find CFA offset for a function
fn parse_eh_frame(file: &object::File, function_address: u64) -> Result<Option<i64>> {
    let eh_frame_section = match file.section_by_name(".eh_frame") {
        Some(section) => section,
        None => {
            log::debug!("No .eh_frame section found");
            return Ok(None);
        }
    };

    let eh_frame_data = eh_frame_section.data()?;
    let eh_frame_address = eh_frame_section.address();

    let mut eh_frame = EhFrame::new(eh_frame_data, LittleEndian);
    eh_frame.set_address_size(address_size(file));
    parse_cfi_section(&eh_frame, eh_frame_address, function_address, ".eh_frame")
}

/// Parse .debug_frame section to find CFA offset for a function  
fn parse_debug_frame(file: &object::File, function_address: u64) -> Result<Option<i64>> {
    let debug_frame_section = match file.section_by_name(".debug_frame") {
        Some(section) => section,
        None => {
            log::warn!("    No .debug_frame section found");
            return Ok(None);
        }
    };

    let debug_frame_data = debug_frame_section.data()?;
    let debug_frame_address = debug_frame_section.address();

    log::debug!("    Found .debug_frame section with {} bytes at 0x{:08x}", debug_frame_data.len(), debug_frame_address);

    let mut debug_frame = DebugFrame::new(debug_frame_data, LittleEndian);
    debug_frame.set_address_size(address_size(file));
    parse_cfi_section(&debug_frame, debug_frame_address, function_address, ".debug_frame")
}

/// Address size of the target in bytes. The CIEs of .debug_frame before DWARF 4 do not contain the address size,
/// gimli defaults to the address size of the host, which breaks the parsing of 32 bit targets (ARM, Xtensa) on a 64 bit host
fn address_size(file: &object::File) -> u8 {
    if file.is_64() { 8 } else { 4 }
}

/// Parse a CFI section (either .eh_frame or .debug_frame) using gimli: walk the CIEs and FDEs and find the FDE whose
/// address range contains the function start address. .eh_frame encodes addresses relative to the section, so its base
/// address has to be known (BaseAddresses), .debug_frame uses absolute addresses
fn parse_cfi_section<R: gimli::Reader>(section: &impl UnwindSection<R>, section_address: u64, function_address: u64, section_name: &str) -> Result<Option<i64>> {
    // Set up proper base addresses for CFI parsing
    let mut bases = BaseAddresses::default();

    // For .eh_frame, we need to set the eh_frame base address
    if section_name == ".eh_frame" {
        bases = bases.set_eh_frame(section_address);
    }

    let mut entries = section.entries(&bases);

    while let Some(entry) = entries.next()? {
        match entry {
            CieOrFde::Cie(_) => {
                // Common Information Entry - skip for now
                continue;
            }
            CieOrFde::Fde(partial_fde) => {
                // Frame Description Entry - this contains the function-specific info
                let fde = partial_fde.parse(|_, bases, offset| section.cie_from_offset(bases, offset))?;

                let fde_start = fde.initial_address();
                let fde_end = fde_start + fde.len();

                // Check if this FDE covers our function
                if function_address >= fde_start && function_address < fde_end {
                    // Parse the unwind instructions to get CFA rules
                    return parse_fde_for_cfa(&fde, section, &bases);
                }
            }
        }
    }

    Ok(None)
}

/// Parse Frame Description Entry to extract CFA offset
///
/// fde.rows() replays the unwind instructions of the FDE and yields one row per code range with the rule in effect there,
/// e.g. row 1 (function entry): CFA = SP + 0, row 2 (after push {r4-r7, lr}): CFA = SP + 20, row 3 (after sub sp, #4):
/// CFA = SP + 24. The rule is either register + offset or a full expression (not handled).
/// The register numbers are the DWARF register numbers of the target ABI, which differ per architecture:
/// ARM: 13 = SP, AArch64: 31 = SP, 29 = FP, x86_64: 7 = RSP, 6 = RBP. The largest positive stack pointer offset is taken
/// as the state after the prologue, a switch to a frame pointer based rule ends the search
fn parse_fde_for_cfa<R: gimli::Reader, Section: gimli::UnwindSection<R>>(fde: &gimli::FrameDescriptionEntry<R>, section: &Section, bases: &BaseAddresses) -> Result<Option<i64>> {
    // Create unwind context for parsing the instructions
    let mut ctx = gimli::UnwindContext::new();

    // Initialize unwind table for this FDE
    let mut table = fde.rows(section, bases, &mut ctx)?;

    let mut best_cfa_offset: Option<i64> = None;
    let mut row_count = 0;

    // Iterate through all rows to find the maximum CFA offset
    // This typically corresponds to the function after prologue setup
    while let Some(row) = table.next_row()? {
        row_count += 1;
        let cfa = row.cfa();

        match *cfa {
            gimli::CfaRule::RegisterAndOffset { register, offset } => {
                {
                    log::debug!(
                        "       Row {}: 0x{:08X}-0x{:08X} CFA rule: register {} + offset {}",
                        row_count,
                        row.start_address(),
                        row.end_address(),
                        register.0,
                        offset
                    );
                }

                // For AArch64, register 31 is the stack pointer (SP)
                // For x86_64, register 7 is RSP
                match register.0 {
                    31 => {
                        {
                            log::debug!(" CFA = SP + {} (AArch64)", offset);
                        }
                        // Keep track of the largest positive offset (after prologue)
                        if offset > 0 && (best_cfa_offset.is_none() || offset > best_cfa_offset.unwrap()) {
                            best_cfa_offset = Some(offset);
                        }
                    }
                    7 => {
                        {
                            log::debug!(" CFA = RSP + {} (x86_64)", offset);
                        }
                        // Keep track of the largest positive offset (after prologue)
                        if offset > 0 && (best_cfa_offset.is_none() || offset > best_cfa_offset.unwrap()) {
                            best_cfa_offset = Some(offset);
                        }
                    }
                    29 => {
                        {
                            log::debug!(" CFA = FP + {} (frame pointer based)", offset);
                        }
                        // Frame pointer based CFA - this is typically after prologue
                        // Return the previous SP-based offset if we have one
                        if best_cfa_offset.is_some() {
                            break;
                        }
                    }
                    13 => {
                        {
                            log::debug!(" CFA = SP + {} (ARM)", offset);
                        }
                        // Keep track of the largest positive offset (after prologue)
                        if offset > 0 && (best_cfa_offset.is_none() || offset > best_cfa_offset.unwrap()) {
                            best_cfa_offset = Some(offset);
                        }
                    }
                    _ => {
                        {
                            log::debug!("    CFA uses register {} + {}", register.0, offset);
                        }
                        if offset > 0 && (best_cfa_offset.is_none() || offset > best_cfa_offset.unwrap()) {
                            best_cfa_offset = Some(offset);
                        }
                    }
                }
            }
            gimli::CfaRule::Expression(_) => {
                log::debug!("    Row {}: CFA defined by expression (too complex to parse here)", row_count);
            }
        }
    }

    log::debug!("    Processed {} CFI rows, best CFA offset: {:?}", row_count, best_cfa_offset);

    // Return the best (largest) CFA offset we found, or 0 if none
    Ok(best_cfa_offset.or(Some(0)))
}

/// Parse a simple DWARF expression to extract CFA offset
///
/// This is a very basic parser that handles common cases like:
/// - DW_OP_call_frame_cfa (CFA + 0)
/// - DW_OP_call_frame_cfa + DW_OP_plus_uconst(offset)
/// - DW_OP_fbreg(offset) (frame base register + offset)
///
/// Real DWARF expressions can be much more complex and might require
/// a full expression evaluator.
fn parse_simple_cfa_expression(expr: &[u8]) -> Option<i64> {
    if expr.is_empty() {
        return None;
    }

    match expr[0] {
        // DW_OP_call_frame_cfa = 0x9c
        0x9c => {
            if expr.len() == 1 {
                // Just CFA, offset is 0
                Some(0)
            } else if expr.len() > 1 && expr[1] == 0x23 {
                // DW_OP_plus_uconst = 0x23
                // Try to decode the ULEB128 constant that follows
                if let Some((offset, _)) = decode_uleb128(&expr[2..]) {
                    Some(offset as i64)
                } else {
                    None
                }
            } else {
                // More complex expression
                None
            }
        }
        // DW_OP_fbreg = 0x91
        0x91 => {
            // Frame base register + SLEB128 offset
            if let Some((offset, _)) = decode_sleb128(&expr[1..]) { Some(offset) } else { None }
        }
        _ => None,
    }
}

/// Decode ULEB128 (Unsigned Little Endian Base 128) integer
/// This is a variable-length encoding used in DWARF
fn decode_uleb128(data: &[u8]) -> Option<(u64, usize)> {
    let mut result = 0u64;
    let mut shift = 0;
    let mut i = 0;

    for &byte in data {
        i += 1;
        result |= ((byte & 0x7f) as u64) << shift;

        if byte & 0x80 == 0 {
            return Some((result, i));
        }

        shift += 7;
        if shift >= 64 {
            return None; // Overflow
        }
    }

    None
}

/// Decode SLEB128 (Signed Little Endian Base 128) integer
/// This is a variable-length encoding used in DWARF for signed values
fn decode_sleb128(data: &[u8]) -> Option<(i64, usize)> {
    let mut result = 0i64;
    let mut shift = 0;
    let mut i = 0;
    let mut byte = 0u8;

    for &b in data {
        byte = b;
        i += 1;
        result |= ((byte & 0x7f) as i64) << shift;
        shift += 7;

        if byte & 0x80 == 0 {
            break;
        }

        if shift >= 64 {
            return None; // Overflow
        }
    }

    // Sign extend if necessary
    if shift < 64 && (byte & 0x40) != 0 {
        result |= !0i64 << shift;
    }

    Some((result, i))
}

/// Get a string attribute from a DWARF DIE
///
/// Dwarf::attr_string resolves all string forms: inline (DW_FORM_string), an offset into .debug_str (DW_FORM_strp) and the
/// DWARF 5 index into .debug_str_offsets (DW_FORM_strx, clang). Other forms are reported and treated as no string
fn get_string_attribute(
    dwarf: &Dwarf<EndianSlice<LittleEndian>>,
    unit: &Unit<EndianSlice<LittleEndian>>,
    entry: &gimli::DebuggingInformationEntry<EndianSlice<LittleEndian>>,
    attr: gimli::DwAt,
) -> Result<Option<String>> {
    let Some(attr_value) = entry.attr_value(attr) else {
        return Ok(None);
    };
    match dwarf.attr_string(unit, attr_value) {
        Ok(s) => Ok(Some(s.to_string_lossy().into_owned())),
        Err(e) => {
            log::warn!("CFA parser: unsupported form {:?} of {}: {}", attr_value, attr, e);
            Ok(None)
        }
    }
}
