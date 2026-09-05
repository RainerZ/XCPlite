//--------------------------------------------------------------------------------------------------------------------------------------------------
// Module dwarf
// Implements DebugDataReader, UnitList and functions to read DWARF debug information from ELF files
// Read ELF files and extract debug information
// Taken from Github repository a2ltool by DanielT

use indexmap::IndexMap;
use std::ffi::OsStr;
use std::ops::Index;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
};

type SliceType<'a> = EndianSlice<'a, RunTimeEndian>;

use object::read::{ObjectSection, ObjectSymbol};
use object::{Endianness, Object};

use gimli::{Abbreviations, DebuggingInformationEntry, Dwarf, UnitHeader};
use gimli::{EndianSlice, RunTimeEndian};

use crate::elf_reader::debuginfo::cfa::{CfaInfo, get_cfa_from_object};
use crate::elf_reader::debuginfo::{DbgDataType, DebugData, TypeInfo, VarInfo};

mod attributes;
use attributes::{get_abstract_origin_attribute, get_linkage_name_attribute, get_location_attribute, get_name_attribute, get_specification_attribute, get_typeref_attribute};

mod typereader;

pub(crate) struct UnitList<'a> {
    list: Vec<(UnitHeader<SliceType<'a>>, gimli::Abbreviations)>,
}

struct DebugDataReader<'elffile> {
    dwarf: Dwarf<EndianSlice<'elffile, RunTimeEndian>>,
    verbose: usize,
    units: UnitList<'elffile>,
    unit_names: Vec<Option<String>>,
    endian: Endianness,
    sections: HashMap<String, (u64, u64)>,
    cfa_info: Vec<CfaInfo>,
    epk_string: Option<String>,
    epk_addr: u64,
    symbol_addresses: HashMap<String, u64>,
    global_symbol_names: HashSet<String>,  // names of the symbols with global (or weak) binding
    xcp_meta_data: Option<(u64, Vec<u8>)>, // (section_base_addr, raw_bytes)
    is_little_endian: bool,
    scope_parent: HashMap<usize, usize>, // .debug_info offset of a named type or scope -> offset of its enclosing scope (namespace, struct, class, union or function)
}

// Create DebugData
// Load and validate ELF/DWARF input, then collect and return parsed DebugData.
// This function constructs a temporary DebugDataReader that owns parser state
// (units, transient names, symbol table cache) and finalizes it into DebugData.
pub(crate) fn load_elf_dwarf(filename: &OsStr, verbose: usize, unit_idx_limit: usize) -> Result<DebugData, String> {
    log::debug!("load_elf_dwarf: {}", filename.to_string_lossy());

    // open the file and mmap its content
    let filedata = load_filedata(filename)?;

    // load the elf file using the object crate
    let elffile = load_elf_file(&filename.to_string_lossy(), &filedata, verbose)?;

    // print symbol table
    if verbose >= 1 {
        println!("\nSymbol table:");
        for symbol in elffile.symbols() {
            let Ok(name) = symbol.name() else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            println!("  `{:?}`: addr={:x}, {:?}", name, symbol.address(), symbol);
        }
    }

    // verify that the elf file contains DWARF debug info
    if !elffile.sections().any(|section| section.name() == Ok(".debug_info")) {
        log::error!("DWARF .debug_info section not found");
        return Err(format!(
            "Error: {} does not contain DWARF2+ debug info. The section .debug_info is missing.",
            filename.to_string_lossy()
        ));
    }

    // load the DWARF sections from the elf file
    let dwarf = load_dwarf_sections(&elffile)?;

    // verify that the dwarf data is valid
    if !verify_dwarf_compile_units(&dwarf) {
        return Err(format!(
            "Error: {} does not contain DWARF2+ debug info - zero compile units contain debug info.",
            filename.to_string_lossy()
        ));
    }

    // get the elf sections for DebugDataReader
    let sections = get_elf_sections(&elffile);

    // read the EPK string and address from the xcp_epk ELF section
    let epk_section = elffile.section_by_name("xcp_epk");
    let epk_addr: u64 = epk_section.as_ref().map_or(0, |s| s.address());
    let epk_string: Option<String> = epk_section
        .and_then(|s| s.data().ok())
        .and_then(|data| std::ffi::CStr::from_bytes_until_nul(data).ok())
        .map(|cs| cs.to_string_lossy().into_owned());
    if let Some(ref epk) = epk_string {
        log::debug!("EPK string read from xcp_epk section: '{}' at address 0x{:08X}", epk, epk_addr);
    }

    // read the xcp_meta section raw bytes for metadata (XCP_UNIT / XCP_LIMITS annotations)
    let xcp_meta_section = elffile.section_by_name("xcp_meta");
    let xcp_meta_data: Option<(u64, Vec<u8>)> = xcp_meta_section.and_then(|s| {
        let addr = s.address();
        s.data().ok().map(|data| (addr, data.to_vec()))
    });
    if let Some((addr, ref data)) = xcp_meta_data {
        log::debug!("XCP metadata section (xcp_meta) found at address 0x{:08X}, {} bytes", addr, data.len());
    } else {
        log::debug!("XCP metadata section (xcp_meta) not found in ELF file");
    }
    let is_little_endian = elffile.endianness() == Endianness::Little;

    // get CFA information for DebugDataReader
    let mut cfa_info = Vec::new();
    let res = get_cfa_from_object(&elffile, &mut cfa_info, verbose, unit_idx_limit);
    match res {
        Ok(cfa) => {
            if cfa > 0 {
                log::debug!("CFA data found in {cfa} functions");
            } else {
                log::warn!("CFA data not found");
            }
        }
        Err(err) => {
            log::error!("CFA parser error: {err}");
        }
    }

    // create the debug data reader
    log::debug!("Creating debug data reader");
    let dbg_reader = DebugDataReader {
        dwarf,
        verbose,
        units: UnitList::new(),
        unit_names: Vec::new(),
        endian: elffile.endianness(),
        sections,
        cfa_info,
        epk_string,
        epk_addr,
        symbol_addresses: get_symbol_addresses(&elffile),
        global_symbol_names: get_global_symbol_names(&elffile),
        xcp_meta_data,
        is_little_endian,
        scope_parent: HashMap::new(),
    };
    log::debug!("Reading debug info entries");
    Ok(dbg_reader.collect_debug_data(unit_idx_limit))
}

// open a file and mmap its content
fn load_filedata(filename: &OsStr) -> Result<memmap2::Mmap, String> {
    let file = match File::open(filename) {
        Ok(file) => file,
        Err(error) => {
            return Err(format!("Error: could not open file {}: {error}", filename.to_string_lossy()));
        }
    };

    match unsafe { memmap2::Mmap::map(&file) } {
        Ok(mmap) => Ok(mmap),
        Err(err) => Err(format!("Error: Failed to map file '{}': {err}", filename.to_string_lossy())),
    }
}

// read the headers and sections of an elf/object file
fn load_elf_file<'data>(filename: &str, filedata: &'data [u8], verbose: usize) -> Result<object::read::File<'data>, String> {
    log::debug!("load_elf_file: {}", filename);
    match object::File::parse(filedata) {
        Ok(object_file) => {
            if verbose >= 1 {
                println!("\nParsed object file file: {}", filename);
                println!("ELF file format: {:?}", object_file.format());
                println!("Architecture: {:?}", object_file.architecture());
                println!("Endianness: {:?}", object_file.endianness());
                println!("\nSections:");
                for section in object_file.sections() {
                    let kind = section.kind();
                    println!(
                        "  Name: {:<20} Addr: 0x{:08x} Size: {} bytes Kind: {:?} ",
                        section.name().unwrap_or("<unknown>"),
                        section.address(),
                        section.size(),
                        kind
                    );
                }
                println!("\n");
            }

            Ok(object_file)
        }
        Err(err) => Err(format!("Error: Failed to parse file '{filename}': {err}")),
    }
}

fn get_elf_sections(elffile: &object::read::File) -> HashMap<String, (u64, u64)> {
    log::debug!("get_elf_sections: Creating ELF sections map for debug data (only size!=0 and addr!=0)");
    let mut map = HashMap::new();
    for section in elffile.sections() {
        let addr = section.address();
        let size = section.size();
        if addr != 0
            && size != 0
            && let Ok(name) = section.name()
        {
            map.insert(name.to_string(), (addr, addr + size));
            log::trace!("elf section: {} addr={addr:x}, size={size:x}", name);
        }
    }

    map
}

fn get_symbol_addresses(elffile: &object::read::File) -> HashMap<String, u64> {
    let mut map = HashMap::new();
    for symbol in elffile.symbols() {
        let Ok(name) = symbol.name() else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let addr = symbol.address();
        if addr != 0 {
            map.insert(name.to_string(), addr);
        }
    }
    map
}

// Names of the symbols with global (or weak) binding, all other symbols are local to their compilation unit (static variables and functions)
fn get_global_symbol_names(elffile: &object::read::File) -> HashSet<String> {
    elffile
        .symbols()
        .filter(|symbol| symbol.is_global())
        .filter_map(|symbol| symbol.name().ok().filter(|name| !name.is_empty()).map(str::to_string))
        .collect()
}

// load the DWARF debug info from the .debug_<xyz> sections
fn load_dwarf_sections<'data>(elffile: &object::read::File<'data>) -> Result<gimli::Dwarf<SliceType<'data>>, String> {
    log::debug!("load_dwarf_sections");
    // Dwarf::load takes two closures / functions and uses them to load all the required debug sections
    let loader = |section: gimli::SectionId| get_file_section_reader(elffile, section.name());
    gimli::Dwarf::load(loader)
}

// verify that the dwarf data is valid
fn verify_dwarf_compile_units(dwarf: &gimli::Dwarf<SliceType>) -> bool {
    let mut units_iter = dwarf.debug_info.units();
    let mut units_count = 0;
    while let Ok(Some(_)) = units_iter.next() {
        units_count += 1;
    }

    log::debug!("DWARF compile units: {}", units_count);
    units_count > 0
}

// get a section from the elf file.
// returns a slice referencing the section data if it exists, or an empty slice otherwise
fn get_file_section_reader<'data>(elffile: &object::read::File<'data>, section_name: &str) -> Result<SliceType<'data>, String> {
    if let Some(dbginfo) = elffile.section_by_name(section_name) {
        match dbginfo.data() {
            Ok(val) => Ok(EndianSlice::new(val, get_endian(elffile))),
            Err(e) => Err(e.to_string()),
        }
    } else {
        Ok(EndianSlice::new(&[], get_endian(elffile)))
    }
}

// get the endianity of the elf file
fn get_endian(elffile: &object::read::File) -> RunTimeEndian {
    if elffile.is_little_endian() { RunTimeEndian::Little } else { RunTimeEndian::Big }
}

impl DebugDataReader<'_> {
    // Get the address of a symbol by its exact name
    // local_only: only symbols with local binding (static variables) are considered
    fn symbol_address(&self, symbol_name: &str, local_only: bool) -> Option<u64> {
        if local_only && self.global_symbol_names.contains(symbol_name) {
            return None;
        }
        self.symbol_addresses.get(symbol_name).copied()
    }

    // Get the address of the only symbol whose (mangled) name ends with the variable name, e.g. _ZZ4mainE7counter for the static variable counter in main
    fn resolve_address_by_unique_suffix(&self, var_name: &str, local_only: bool) -> Option<u64> {
        // Very short names are too ambiguous in mangled symbols.
        if var_name.len() < 4 {
            return None;
        }

        let mut matches = self.symbol_addresses.iter().filter_map(|(symbol_name, addr)| {
            if *addr != 0 && symbol_name.ends_with(var_name) && !(local_only && self.global_symbol_names.contains(symbol_name)) {
                Some(*addr)
            } else {
                None
            }
        });

        let first = matches.next()?;
        if matches.next().is_none() { Some(first) } else { None }
    }

    // Resolve the address of a variable without location attribute from the symbol table: by linkage name, by name,
    // by the mangled name of a variable in a namespace or class scope, or by a unique name suffix
    // local_only: the variable is local to a function, only symbols with local binding (static variables) are considered,
    // a global symbol with the same name belongs to a different variable
    // scopes: the namespaces and classes the variable is defined in (outermost first), used for the mangled name
    fn resolve_address_from_symbols(
        &self,
        entry: &DebuggingInformationEntry<SliceType, usize>,
        unit: &UnitHeader<SliceType>,
        var_name: &str,
        local_only: bool,
        scopes: &[String],
    ) -> Option<u64> {
        if let Ok(linkage_name) = get_linkage_name_attribute(entry, &self.dwarf, unit)
            && let Some(addr) = self.symbol_address(&linkage_name, local_only)
        {
            return Some(addr);
        }
        if let Some(addr) = self.symbol_address(var_name, local_only) {
            return Some(addr);
        }
        // GCC emits no linkage name for variables with internal linkage in a namespace (static const in a namespace, e.g. the XCP_COMMENT
        // metadata markers), and the mangled symbol name of a namespace scope variable (_ZN13motor_controlL5inputE) does not end with the variable name
        if !local_only && !scopes.is_empty() {
            for mangled in itanium_mangled_names(scopes, var_name) {
                if let Some(addr) = self.symbol_address(&mangled, local_only) {
                    return Some(addr);
                }
            }
        }
        self.resolve_address_by_unique_suffix(var_name, local_only)
    }

    // Traverse DWARF entries and finalize collected parser state into DebugData.
    fn collect_debug_data(mut self, unit_idx_limit: usize) -> DebugData {
        let variables = self.load_variables(unit_idx_limit);
        let (types, typenames) = self.load_types(&variables);
        let qualified_type_names = self.load_qualified_type_names(&types, &typenames);
        let varname_list: Vec<&String> = variables.keys().collect();
        let demangled_names = demangle_cpp_varnames(&varname_list);
        let unit_names = std::mem::take(&mut self.unit_names);

        DebugData {
            variables,
            types,
            typenames,
            qualified_type_names,
            demangled_names,
            unit_names,
            sections: self.sections,
            symbol_addresses: self.symbol_addresses,
            cfa_info: self.cfa_info,
            epk_string: self.epk_string,
            epk_addr: self.epk_addr,
            xcp_meta_data: self.xcp_meta_data,
            is_little_endian: self.is_little_endian,
        }
    }

    // load all variables from the dwarf data
    fn load_variables(&mut self, unit_idx_limit: usize) -> IndexMap<String, Vec<VarInfo>> {
        let mut variables = IndexMap::<String, Vec<VarInfo>>::new();

        let mut iter = self.dwarf.debug_info.units();
        while let Ok(Some(unit)) = iter.next() {
            // get the abbreviations for the unit
            let Ok(abbreviations) = unit.abbreviations(&self.dwarf.debug_abbrev) else {
                let offset = unit.offset().to_debug_info_offset(&unit).unwrap_or(gimli::DebugInfoOffset(0)).0;
                log::warn!("Failed to get abbreviations for unit @{offset:x}");
                continue;
            };

            // store the unit for later reference
            self.units.add(unit, abbreviations);
            let unit_idx = self.units.list.len() - 1;
            if unit_idx > unit_idx_limit {
                break;
            }
            let (unit, abbreviations) = &self.units[unit_idx];

            // The root of the tree inside of a unit is always a DW_TAG_compile_unit or DW_TAG_partial_unit.
            // The global variables are among the immediate children of the unit; static variables
            // in functions are declared inside of DW_TAG_subprogram[/DW_TAG_lexical_block]*.
            // We can easily find all of them by using depth-first traversal of the tree
            let mut entries_cursor = unit.entries(abbreviations);
            if let Ok(Some(entry)) = entries_cursor.next_dfs()
                && (entry.tag() == gimli::constants::DW_TAG_compile_unit || entry.tag() == gimli::constants::DW_TAG_partial_unit)
            {
                // @@@@ warn if unit name is missing
                let unit_name = match get_name_attribute(entry, &self.dwarf, unit) {
                    Ok(name) => {
                        log::trace!("unit name: {}", &name);
                        Some(name)
                    }
                    Err(e) => {
                        log::warn!("Failed to get unit name: {}", e);
                        None
                    }
                };
                self.unit_names.push(unit_name);
            }

            // traverse all entries in depth-first order
            // context holds the tag, the name (namespaces and functions only) and the .debug_info offset of the ancestors of the current entry
            let mut context: Vec<(gimli::DwTag, Option<String>, usize)> = Vec::new();
            while let Ok(Some(entry)) = entries_cursor.next_dfs() {
                let depth = entry.depth();
                debug_assert!(depth >= 1);
                context.truncate((depth - 1) as usize);
                let tag = entry.tag();
                let offset = entry.offset().to_debug_info_offset(unit).map_or(0, |o| o.0);
                // It's essential to only get those names that might actually be needed.
                // Getting all names unconditionally doubled the runtime of the program
                // as a result of countless useless string allocations and deallocations.
                if tag == gimli::constants::DW_TAG_namespace || tag == gimli::constants::DW_TAG_subprogram {
                    context.push((tag, get_name_attribute(entry, &self.dwarf, unit).ok(), offset));
                } else {
                    context.push((tag, None, offset));
                }
                debug_assert_eq!(depth as usize, context.len());

                // Remember the enclosing scope of named types, of nested scopes and of variables declared in a namespace or class,
                // to qualify the names of types which are defined in a namespace, class or function (motor_control::Input, Controller::Params)
                // and to find the scope of a variable definition which refers to its declaration (DW_AT_specification).
                // Only offsets are stored here, the scope names are resolved in load_qualified_type_names for the few types which need them.
                if (is_scope_tag(tag) || is_type_tag(tag) || tag == gimli::constants::DW_TAG_variable)
                    && let Some((parent_tag, _, parent_offset)) = context[..context.len() - 1].iter().rev().find(|(t, _, _)| is_scope_tag(*t))
                    && (tag != gimli::constants::DW_TAG_variable || *parent_tag != gimli::constants::DW_TAG_subprogram)
                {
                    self.scope_parent.insert(offset, *parent_offset);
                }

                if entry.tag() == gimli::constants::DW_TAG_variable {
                    // Get variable information
                    let (function, namespaces) = get_varinfo_from_context(&context);
                    match self.get_variable(entry, unit, abbreviations, function.is_some(), &namespaces) {
                        Ok((name, typeref, address)) => {
                            let var_infos = variables.entry(name).or_default();
                            // GCC describes a namespace scope (or static member) variable with a declaration entry inside the namespace
                            // and a definition entry at compilation unit level (DW_AT_specification). Both resolve to the same address,
                            // the variable is kept once with the namespaces of the declaration
                            if function.is_none()
                                && address.0 == 0
                                && address.1 != 0
                                && let Some(existing) = var_infos.iter_mut().find(|v| v.unit_idx == unit_idx && v.address == address && v.function.is_none())
                            {
                                if existing.namespaces.is_empty() {
                                    existing.namespaces = namespaces;
                                }
                            } else {
                                var_infos.push(VarInfo {
                                    address, // may be 0 for local variables
                                    typeref,
                                    unit_idx,
                                    function,
                                    namespaces,
                                });
                            }
                        }
                        Err(errmsg) => {
                            let offset = entry.offset().to_debug_info_offset(unit).unwrap_or(gimli::DebugInfoOffset(0)).0;
                            log::warn!("Could not load variable @{offset:x}: {errmsg}");
                        }
                    }
                }
            }
        }

        variables
    }

    // Determine the scope qualified names of the struct and class types whose name is used by different types in different scopes.
    // The DWARF name of a type is its unqualified name (DW_AT_name "Input" for motor_control::Input), the enclosing scopes
    // (namespaces, classes, functions) recorded during the traversal in load_variables tell such types apart.
    // The result maps a type reference to its qualified name ("motor_control.Input") and contains only the types which need
    // qualification: the name is used in more than one scope and the type has a scope. All other types keep their plain name.
    fn load_qualified_type_names(&self, types: &HashMap<usize, TypeInfo>, typenames: &HashMap<String, Vec<usize>>) -> HashMap<usize, String> {
        // Scopes of all struct/class types, outermost first. The type of a variable or member may be a const or volatile
        // qualified type, the name and the scope are the ones of the named type behind the qualifier
        let mut scopes: HashMap<usize, Vec<String>> = HashMap::new();
        for (offset, typeinfo) in types {
            if matches!(typeinfo.datatype, DbgDataType::Struct { .. })
                && let Some(named_offset) = self.get_named_type_offset(*offset)
            {
                scopes.insert(*offset, self.get_scope_path(named_offset));
            }
        }

        // Type names which are used by struct/class types in more than one scope
        let mut ambiguous: HashSet<&String> = HashSet::new();
        for (type_name, type_refs) in typenames {
            let mut distinct: Vec<&[String]> = Vec::new();
            for type_ref in type_refs {
                if let Some(scope) = scopes.get(type_ref)
                    && !distinct.contains(&scope.as_slice())
                {
                    distinct.push(scope.as_slice());
                }
            }
            if distinct.len() > 1 {
                log::debug!(
                    "Type name '{}' is used by {} different struct/class types, the typedef names are qualified with their scope",
                    type_name,
                    distinct.len()
                );
                ambiguous.insert(type_name);
            }
        }

        // Qualified names for the types with an ambiguous name and a scope
        let mut qualified_type_names = HashMap::new();
        for (offset, scope) in &scopes {
            if scope.is_empty() {
                continue;
            }
            if let Some(type_name) = types.get(offset).and_then(|t| t.name.as_ref())
                && ambiguous.contains(type_name)
            {
                let qualified = format!("{}.{}", scope.join("."), type_name);
                log::trace!("type {} @{offset:x} is qualified as {}", type_name, qualified);
                qualified_type_names.insert(*offset, qualified);
            }
        }
        qualified_type_names
    }

    // Read tag, name and type reference of the debug info entry at the given .debug_info offset
    fn read_entry(&self, dbginfo_offset: usize) -> Option<(gimli::DwTag, Option<String>, Option<usize>)> {
        let unit_idx = self.units.get_unit(dbginfo_offset)?;
        let (unit, abbrev) = &self.units[unit_idx];
        let unit_offset = gimli::DebugInfoOffset(dbginfo_offset).to_unit_offset(unit)?;
        let mut entries_tree = unit.entries_tree(abbrev, Some(unit_offset)).ok()?;
        let node = entries_tree.root().ok()?;
        let entry = node.entry();
        let name = get_name_attribute(entry, &self.dwarf, unit).ok();
        let typeref = get_typeref_attribute(entry, unit).ok();
        Some((entry.tag(), name, typeref))
    }

    // Follow const, volatile and similar qualifiers to the named type definition (struct, class, union, enum or typedef)
    fn get_named_type_offset(&self, dbginfo_offset: usize) -> Option<usize> {
        let mut offset = dbginfo_offset;
        for _ in 0..16 {
            // limited number of qualifier levels, to be safe with malformed debug info
            let (tag, _, typeref) = self.read_entry(offset)?;
            if is_type_tag(tag) {
                return Some(offset);
            }
            match tag {
                gimli::constants::DW_TAG_const_type
                | gimli::constants::DW_TAG_volatile_type
                | gimli::constants::DW_TAG_packed_type
                | gimli::constants::DW_TAG_restrict_type
                | gimli::constants::DW_TAG_immutable_type
                | gimli::constants::DW_TAG_atomic_type => offset = typeref?,
                _ => return None,
            }
        }
        None
    }

    // Get the names of the scopes enclosing a debug info entry, outermost scope first
    // Anonymous scopes (unnamed namespaces, anonymous structs) are skipped
    fn get_scope_path(&self, dbginfo_offset: usize) -> Vec<String> {
        let mut path = Vec::new();
        let mut offset = dbginfo_offset;
        // a parent always precedes its children in .debug_info, so the chain of parents terminates
        while let Some(parent) = self.scope_parent.get(&offset) {
            if let Some((_, Some(name), _)) = self.read_entry(*parent) {
                path.push(name);
            }
            offset = *parent;
        }
        path.reverse();
        path
    }

    // Return global variable information
    // an entry of the type DW_TAG_variable only describes a global variable if there is a name, a type and an address
    // this function tries to get all three and returns them
    // returns None if the entry does not describe a global variable
    /*
        fn get_global_variable(
            &self,
            entry: &DebuggingInformationEntry<SliceType, usize>,
            unit: &UnitHeader<SliceType>,
            abbrev: &gimli::Abbreviations,
        ) -> Result<Option<(String, usize, u64)>, String> {
            match get_location_attribute(self, entry, unit.encoding(), &self.units.list.len() - 1) {
                Some((addr_ext, addr)) => {
                    // if debugging information entry A has a DW_AT_specification or DW_AT_abstract_origin attribute
                    // pointing to another debugging information entry B, any attributes of B are considered to be part of A.
                    if let Some(specification_entry) = get_specification_attribute(entry, unit, abbrev) {
                        // the entry refers to a specification, which contains the name and type reference
                        let name = get_name_attribute(&specification_entry, &self.dwarf, unit)?;
                        let typeref = get_typeref_attribute(&specification_entry, unit)?;
                        Ok(Some((name, typeref, addr)))
                    } else if let Some(abstract_origin_entry) = get_abstract_origin_attribute(entry, unit, abbrev) {
                        // the entry refers to an abstract origin, which should also be considered when getting the name and type ref
                        let name = get_name_attribute(entry, &self.dwarf, unit).or_else(|_| get_name_attribute(&abstract_origin_entry, &self.dwarf, unit))?;
                        let typeref = get_typeref_attribute(entry, unit).or_else(|_| get_typeref_attribute(&abstract_origin_entry, unit))?;
                        Ok(Some((name, typeref, addr)))
                    } else {
                        // usual case: there is no specification or abstract origin and all info is part of this entry
                        let name = get_name_attribute(entry, &self.dwarf, unit)?;
                        let typeref = get_typeref_attribute(entry, unit)?;
                        Ok(Some((name, typeref, addr)))
                    }
                }
                None => {
                    // it's a local variable, skip, no error
                    Ok(None)
                }
            }
        }
    */

    // @@@@ xcp_client: Get all variables, including local variables
    // Return variable information
    // returns name, type reference and address
    // address may be 0 if a local variable is requested
    // A missing address is resolved from the symbol table if possible (declarations of global variables, static variables without location)
    // local: the variable is local to a function, only symbols with local binding are considered to resolve the address
    // namespaces: the namespaces the entry is nested in (outermost first)
    fn get_variable<'a>(
        &self,
        entry: &DebuggingInformationEntry<SliceType<'a>, usize>,
        unit: &UnitHeader<SliceType<'a>>,
        abbrev: &gimli::Abbreviations,
        local: bool,
        namespaces: &[String],
    ) -> Result<(String, usize, (u8, u64)), String> {
        // if debugging information entry A has a DW_AT_specification or DW_AT_abstract_origin attribute
        // pointing to another debugging information entry B, any attributes of B are considered to be part of A.
        if let Some(specification_entry) = get_specification_attribute(entry, unit, abbrev) {
            // the entry refers to a specification, which contains the name and type reference
            let name = get_name_attribute(&specification_entry, &self.dwarf, unit)?;
            log::debug!("get_variable '{}':", name);
            let typeref = get_typeref_attribute(&specification_entry, unit)?;
            let mut address = get_location_attribute(self, entry, unit.encoding(), &self.units.list.len() - 1).unwrap_or((0u8, 0u64));
            // The definition entry is at compilation unit level, the scope of the variable is the one of its declaration (specification)
            let specification_scopes = specification_entry
                .offset()
                .to_debug_info_offset(unit)
                .map(|o| self.get_scope_path(o.0))
                .unwrap_or_default();
            if address == (0u8, 0u64)
                && let Some(sym_addr) = self
                    .resolve_address_from_symbols(entry, unit, &name, local, namespaces)
                    .or_else(|| self.resolve_address_from_symbols(&specification_entry, unit, &name, local, &specification_scopes))
            {
                address = (0u8, sym_addr);
            }
            if address.0 >= 0x80 {
                log::debug!("  {} is a register, tls or has unknown location", name);
            } else if address.1 == 0 {
                log::debug!("  {} has no address", name);
            }
            Ok((name, typeref, address))
        } else if let Some(abstract_origin_entry) = get_abstract_origin_attribute(entry, unit, abbrev) {
            // the entry refers to an abstract origin, which should also be considered when getting the name and type ref
            let name = get_name_attribute(entry, &self.dwarf, unit).or_else(|_| get_name_attribute(&abstract_origin_entry, &self.dwarf, unit))?;
            log::debug!("'{}':", name);
            let typeref = get_typeref_attribute(entry, unit).or_else(|_| get_typeref_attribute(&abstract_origin_entry, unit))?;
            let mut address = get_location_attribute(self, entry, unit.encoding(), &self.units.list.len() - 1).unwrap_or((0u8, 0u64));
            if address == (0u8, 0u64)
                && let Some(sym_addr) = self
                    .resolve_address_from_symbols(entry, unit, &name, local, namespaces)
                    .or_else(|| self.resolve_address_from_symbols(&abstract_origin_entry, unit, &name, local, namespaces))
            {
                address = (0u8, sym_addr);
            }
            if address.0 >= 0x80 {
                log::debug!("  {} is a register, tls or has unknown location", name);
            } else if address.1 == 0 {
                log::debug!("  {} has no address", name);
            }
            Ok((name, typeref, address))
        } else {
            // usual case: there is no specification or abstract origin and all info is part of this entry
            let name = get_name_attribute(entry, &self.dwarf, unit)?;
            log::debug!("'{}':", name);
            let typeref = get_typeref_attribute(entry, unit)?;
            let mut address = get_location_attribute(self, entry, unit.encoding(), &self.units.list.len() - 1).unwrap_or((0u8, 0u64));
            if address == (0u8, 0u64)
                && let Some(sym_addr) = self.resolve_address_from_symbols(entry, unit, &name, local, namespaces)
            {
                address = (0u8, sym_addr);
            }
            if address.0 >= 0x80 {
                log::debug!("  {} is a register, tls or has unknown location", name);
            } else if address.1 == 0 {
                log::debug!(". {} has no address", name);
            }
            Ok((name, typeref, address))
        }
    }
}

// Tags of debug info entries which are a named scope for the types and variables nested inside of them
fn is_scope_tag(tag: gimli::DwTag) -> bool {
    matches!(
        tag,
        gimli::constants::DW_TAG_namespace
            | gimli::constants::DW_TAG_structure_type
            | gimli::constants::DW_TAG_class_type
            | gimli::constants::DW_TAG_union_type
            | gimli::constants::DW_TAG_subprogram
    )
}

// Tags of debug info entries which define a named type
fn is_type_tag(tag: gimli::DwTag) -> bool {
    matches!(
        tag,
        gimli::constants::DW_TAG_structure_type
            | gimli::constants::DW_TAG_class_type
            | gimli::constants::DW_TAG_union_type
            | gimli::constants::DW_TAG_enumeration_type
            | gimli::constants::DW_TAG_typedef
    )
}

// Mangled names (Itanium C++ ABI) of a variable in nested namespaces or classes, with external and with internal linkage:
// motor_control::input -> _ZN13motor_control5inputE, static motor_control::input -> _ZN13motor_controlL5inputE
fn itanium_mangled_names(scopes: &[String], name: &str) -> [String; 2] {
    let mut prefix = String::from("_ZN");
    for scope in scopes {
        prefix.push_str(&format!("{}{}", scope.len(), scope));
    }
    [format!("{prefix}{}{name}E", name.len()), format!("{prefix}L{}{name}E", name.len())]
}

// Get the innermost enclosing function and the enclosing namespaces (outermost first) of a variable from the traversal context
fn get_varinfo_from_context(context: &[(gimli::DwTag, Option<String>, usize)]) -> (Option<String>, Vec<String>) {
    let function = context
        .iter()
        .rev()
        .find(|(tag, _, _)| *tag == gimli::constants::DW_TAG_subprogram)
        .and_then(|(_, name, _)| name.clone());
    let namespaces: Vec<String> = context
        .iter()
        .filter_map(|(tag, ns, _)| (*tag == gimli::constants::DW_TAG_namespace).then(|| ns.clone()).flatten())
        .collect();
    (function, namespaces)
}

fn demangle_cpp_varnames(input: &[&String]) -> HashMap<String, String> {
    let mut demangled_symbols = HashMap::<String, String>::new();
    let demangle_opts = cpp_demangle::DemangleOptions::new().no_params().no_return_type();
    for varname in input {
        // some really simple strings can be processed by the demangler, e.g "c" -> "const", which is wrong here.
        // by only processing symbols that start with _Z (variables in classes/namespaces) this problem is avoided
        if varname.starts_with("_Z")
            && let Ok(sym) = cpp_demangle::Symbol::new(*varname)
        {
            // exclude useless demangled names like "typeinfo for std::type_info" or "{vtable(std::type_info)}"
            if let Ok(demangled) = sym.demangle_with_options(&demangle_opts)
                && !demangled.contains(' ')
                && !demangled.starts_with("{vtable")
            {
                demangled_symbols.insert(demangled, (*varname).clone());
            }
        }
    }

    demangled_symbols
}

// UnitList holds a list of all UnitHeaders in the Dwarf data for convenient access
impl<'a> UnitList<'a> {
    fn new() -> Self {
        Self { list: Vec::new() }
    }

    fn add(&mut self, unit: UnitHeader<SliceType<'a>>, abbrev: Abbreviations) {
        self.list.push((unit, abbrev));
    }

    fn get_unit(&self, itemoffset: usize) -> Option<usize> {
        for (idx, (unit, _)) in self.list.iter().enumerate() {
            let unitoffset = unit.offset().to_debug_info_offset(unit).unwrap().0;
            if unitoffset < itemoffset && unitoffset + unit.length_including_self() > itemoffset {
                return Some(idx);
            }
        }

        None
    }
}

impl<'a> Index<usize> for UnitList<'a> {
    type Output = (UnitHeader<SliceType<'a>>, gimli::Abbreviations);

    fn index(&self, idx: usize) -> &Self::Output {
        &self.list[idx]
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // C++ type test fixture, see fixtures/cpp_types.cpp
    static ELF_FILE_NAMES: [&str; 1] = [concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/cpp_types.elf")];

    #[test]
    fn test_itanium_mangled_names() {
        let namespace = ["motor_control".to_string()];
        assert_eq!(itanium_mangled_names(&namespace, "input"), ["_ZN13motor_control5inputE", "_ZN13motor_controlL5inputE"]);
        let nested = ["diagnostics".to_string(), "detail".to_string()];
        assert_eq!(itanium_mangled_names(&nested, "input")[0], "_ZN11diagnostics6detail5inputE");
    }

    // Qualified names of struct types whose name is used in different scopes: namespaces, nested namespaces, enclosing classes
    // and const/volatile qualified variables, see fixtures/cpp_namespaces.cpp
    #[test]
    fn test_load_qualified_type_names() {
        let filename = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/cpp_namespaces.elf");
        let debugdata = DebugData::load_dwarf(OsStr::new(filename), 0, usize::MAX).unwrap();
        let type_name_of = |varinfo: &VarInfo| -> String {
            let type_info = debugdata.types.get(&varinfo.typeref).expect("type of variable");
            debugdata.get_type_name(type_info).expect("type name").to_string()
        };
        let variable = |name: &str| &debugdata.variables.get(name).unwrap_or_else(|| panic!("variable {name}"))[0];

        // The three variables named "input" are in different namespaces and have types of different scopes
        let inputs = debugdata.variables.get("input").expect("variables named input");
        assert_eq!(inputs.len(), 3);
        let mut namespaces: Vec<Vec<String>> = inputs.iter().map(|v| v.namespaces.clone()).collect();
        namespaces.sort();
        assert_eq!(namespaces, vec![vec!["diagnostics", "detail"], vec!["motor_control"], vec!["valve_control"]]);
        let mut type_names: Vec<String> = inputs.iter().map(type_name_of).collect();
        type_names.sort();
        assert_eq!(type_names, vec!["diagnostics.detail.Input", "motor_control.Input", "valve_control.Input"]);

        // A type from another namespace than the variable, and const/volatile qualified variables
        assert_eq!(variable("last_motor_input").namespaces, vec!["diagnostics"]);
        assert_eq!(type_name_of(variable("last_motor_input")), "motor_control.Input");
        assert_eq!(type_name_of(variable("volatile_motor_input")), "motor_control.Input");
        assert_eq!(type_name_of(variable("const_valve_input")), "valve_control.Input");

        // A type with a unique name keeps its plain name, a type without scope keeps its plain name even if the name is ambiguous
        assert_eq!(type_name_of(variable("output")), "Output");
        let mut config_names: Vec<String> = debugdata.variables.get("config").unwrap().iter().map(type_name_of).collect();
        config_names.sort();
        assert_eq!(config_names, vec!["Config", "valve_control.Config"]);

        // The scope of a struct type nested in a class is the class
        let motor_controller = debugdata.types.get(&variable("motor_controller").typeref).unwrap();
        assert_eq!(debugdata.get_type_name(motor_controller), Some("MotorController"));
        let DbgDataType::Struct { members, .. } = &motor_controller.datatype else {
            panic!("MotorController is not a struct");
        };
        let DbgDataType::TypeRef(params_ref, _) = members.get("params").unwrap().0.datatype else {
            panic!("MotorController.params is not a type reference");
        };
        assert_eq!(debugdata.get_type_name(debugdata.types.get(&params_ref).unwrap()), Some("MotorController.Params"));
    }

    #[test]
    fn test_load_data() {
        for filename in ELF_FILE_NAMES {
            let debugdata = DebugData::load_dwarf(OsStr::new(filename), 1, usize::MAX).unwrap();
            // 14 globals in cpp_types.cpp, compilers may add a few more (e.g. static members)
            assert!(debugdata.variables.len() >= 14, "only {} variables found", debugdata.variables.len());
            assert!(debugdata.variables.get("g_sink").is_some());

            for (_, varinfo) in &debugdata.variables {
                assert!(debugdata.types.contains_key(&varinfo[0].typeref));
            }

            let datatype_of = |name: &str| -> &DbgDataType {
                let varinfo = debugdata.variables.get(name).unwrap_or_else(|| panic!("variable {name} not found"));
                &debugdata.types.get(&varinfo[0].typeref).unwrap().datatype
            };
            assert!(matches!(datatype_of("g_plain"), DbgDataType::Struct { is_class: false, .. }));
            assert!(matches!(datatype_of("g_pubclass"), DbgDataType::Struct { is_class: true, .. }));
            assert!(matches!(datatype_of("g_bigenum"), DbgDataType::Enum { signed: true, .. }));

            /*
            if let TypeInfo {
                datatype: DbgDataType::Class { inheritance, members, .. },
                ..
            } = typeinfo
            {
                assert!(inheritance.contains_key("base1"));
                assert!(inheritance.contains_key("base2"));
                assert!(matches!(
                    members.get("ss"),
                    Some((
                        TypeInfo {
                            datatype: DbgDataType::Sint16,
                            ..
                        },
                        _
                    ))
                ));
                assert!(matches!(
                    members.get("base1_var"),
                    Some((
                        TypeInfo {
                            datatype: DbgDataType::Sint32,
                            ..
                        },
                        _
                    ))
                ));
                assert!(matches!(
                    members.get("base2var"),
                    Some((
                        TypeInfo {
                            datatype: DbgDataType::Sint32,
                            ..
                        },
                        _
                    ))
                ));
            }

            let varinfo = debugdata.variables.get("class2").unwrap();
            let typeinfo = debugdata.types.get(&varinfo[0].typeref).unwrap();
            assert!(matches!(
                typeinfo,
                TypeInfo {
                    datatype: DbgDataType::Class { .. },
                    ..
                }
            ));

            let varinfo = debugdata.variables.get("class3").unwrap();
            let typeinfo = debugdata.types.get(&varinfo[0].typeref).unwrap();
            assert!(matches!(
                typeinfo,
                TypeInfo {
                    datatype: DbgDataType::Class { .. },
                    ..
                }
            ));

            let varinfo = debugdata.variables.get("class4").unwrap();
            let typeinfo = debugdata.types.get(&varinfo[0].typeref).unwrap();
            assert!(matches!(
                typeinfo,
                TypeInfo {
                    datatype: DbgDataType::Class { .. },
                    ..
                }
            ));

            let varinfo = debugdata.variables.get("staticvar").unwrap();
            let typeinfo = debugdata.types.get(&varinfo[0].typeref).unwrap();
            assert!(matches!(
                typeinfo,
                TypeInfo {
                    datatype: DbgDataType::Sint32,
                    ..
                }
            ));

            let varinfo = debugdata.variables.get("structvar").unwrap();
            let typeinfo = debugdata.types.get(&varinfo[0].typeref).unwrap();
            assert!(matches!(
                typeinfo,
                TypeInfo {
                    datatype: DbgDataType::Struct { .. },
                    ..
                }
            ));

            let varinfo = debugdata.variables.get("bitfield").unwrap();
            let typeinfo = debugdata.types.get(&varinfo[0].typeref).unwrap();
            assert!(matches!(
                typeinfo,
                TypeInfo {
                    datatype: DbgDataType::Struct { .. },
                    ..
                }
            ));
            if let TypeInfo {
                datatype: DbgDataType::Struct { members, .. },
                ..
            } = typeinfo
            {
                assert!(matches!(
                    members.get("var"),
                    Some((
                        TypeInfo {
                            datatype: DbgDataType::Bitfield { bit_offset: 0, bit_size: 5, .. },
                            ..
                        },
                        0
                    ))
                ));
                assert!(matches!(
                    members.get("var2"),
                    Some((
                        TypeInfo {
                            datatype: DbgDataType::Bitfield { bit_offset: 5, bit_size: 5, .. },
                            ..
                        },
                        0
                    ))
                ));
                assert!(matches!(
                    members.get("var3"),
                    Some((
                        TypeInfo {
                            datatype: DbgDataType::Bitfield { bit_offset: 0, bit_size: 23, .. },
                            ..
                        },
                        4
                    ))
                ));
                assert!(matches!(
                    members.get("var4"),
                    Some((
                        TypeInfo {
                            datatype: DbgDataType::Bitfield { bit_offset: 23, bit_size: 1, .. },
                            ..
                        },
                        4
                    ))
                ));
            }
            let varinfo = debugdata.variables.get("enum_var1").unwrap();
            let typeinfo = debugdata.types.get(&varinfo[0].typeref).unwrap();
            assert!(matches!(
                typeinfo,
                TypeInfo {
                    datatype: DbgDataType::Enum { .. },
                    ..
                }
            ));
            let varinfo = debugdata.variables.get("enum_var2").unwrap();
            let typeinfo = debugdata.types.get(&varinfo[0].typeref).unwrap();
            assert!(matches!(
                typeinfo,
                TypeInfo {
                    datatype: DbgDataType::Enum { .. },
                    ..
                }
            ));
            let varinfo = debugdata.variables.get("enum_var3").unwrap();
            let typeinfo = debugdata.types.get(&varinfo[0].typeref).unwrap();
            assert!(matches!(
                typeinfo,
                TypeInfo {
                    datatype: DbgDataType::Enum { .. },
                    ..
                }
            ));

            let varinfo = debugdata.variables.get("var_array").unwrap();
            let typeinfo = debugdata.types.get(&varinfo[0].typeref).unwrap();
            let DbgDataType::Array { size, dim, arraytype, .. } = &typeinfo.datatype else {
                panic!("Expected array type, got {:?}", typeinfo.datatype);
            };
            assert_eq!(*size, 33);
            assert_eq!(dim.len(), 1);
            assert_eq!(dim[0], 33);
            assert!(matches!(arraytype.datatype, DbgDataType::Uint8));

            let varinfo = debugdata.variables.get("var_multidim").unwrap();
            let typeinfo = debugdata.types.get(&varinfo[0].typeref).unwrap();
            let DbgDataType::Array { dim, arraytype, .. } = &typeinfo.datatype else {
                panic!("Expected array type, got {:?}", typeinfo.datatype);
            };
            assert_eq!(dim.len(), 3);
            assert_eq!(dim, &[10, 3, 7]);
            assert!(matches!(arraytype.datatype, DbgDataType::Float));
            */
        }
    }
}
