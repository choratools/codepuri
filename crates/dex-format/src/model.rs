//! Aggregate DEX model and checked index accessors.

use std::sync::Arc;

use crate::class::{ClassDef, parse_classes};
use crate::header::Header;
use crate::ids::{
    CallSiteId, FieldId, MethodHandle, MethodId, ProtoId, TypeId, parse_call_sites,
    parse_field_ids, parse_method_handles, parse_method_ids, parse_proto_ids, parse_type_ids,
};
use crate::map::{MapEntry, MapList, MapType};
use crate::reader::Reader;
use crate::strings::{StringData, StringId, parse_strings};
use crate::{DexError, RawSpan};

/// A raw section extent inferred from consecutive ordered map entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSection {
    /// Recognized section kind, or the unknown numeric type.
    pub kind: RawSectionKind,
    /// Item count recorded in the map.
    pub item_count: u32,
    /// Raw range from this section offset to the next mapped section.
    pub span: RawSpan,
}

/// Identity of a retained raw section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawSectionKind {
    /// A section defined by the supported DEX specification.
    Known(MapType),
    /// An unrecognized extension section.
    Unknown(u16),
}

/// One logical DEX file backed by retained physical-container bytes.
///
/// Losslessness is provided by `raw_bytes`, each typed item's `RawSpan`, and
/// `raw_sections`, which includes recognized and unknown map sections.
#[derive(Debug, Clone)]
pub struct DexFile {
    raw: Arc<[u8]>,
    /// Parsed header.
    pub header: Header,
    /// Parsed map list, including explicit unknown entries.
    pub map: MapList,
    /// Raw section extents inferred from the ordered map.
    pub raw_sections: Vec<RawSection>,
    /// String identifier table.
    pub string_ids: Vec<StringId>,
    /// Decoded string data corresponding to `string_ids`.
    pub strings: Vec<StringData>,
    /// Type identifier table.
    pub type_ids: Vec<TypeId>,
    /// Prototype identifier table.
    pub proto_ids: Vec<ProtoId>,
    /// Field identifier table.
    pub field_ids: Vec<FieldId>,
    /// Method identifier table.
    pub method_ids: Vec<MethodId>,
    /// v038+ call-site identifiers.
    pub call_site_ids: Vec<CallSiteId>,
    /// v038+ method handles.
    pub method_handles: Vec<MethodHandle>,
    /// Class definitions and class-owned data.
    pub classes: Vec<ClassDef>,
}

impl DexFile {
    /// Parses the first logical DEX from a complete physical container.
    ///
    /// Use [`DexFile::from_container`] to obtain every logical v041 DEX.
    ///
    /// # Errors
    /// Returns a typed error when any referenced byte range or format rule is invalid.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DexError> {
        let raw: Arc<[u8]> = Arc::from(bytes);
        Self::parse_at(raw, 0)
    }

    /// Parses all logical files in a standard or v041 DEX container.
    ///
    /// # Errors
    /// Returns a typed error when a logical header, section, or container boundary is invalid.
    pub fn from_container(bytes: &[u8]) -> Result<Vec<Self>, DexError> {
        let raw: Arc<[u8]> = Arc::from(bytes);
        let mut files = Vec::new();
        let mut base = 0_usize;
        loop {
            let file = Self::parse_at(Arc::clone(&raw), base)?;
            let version = file.header.version;
            let step = usize::try_from(file.header.file_size)
                .map_err(|_| DexError::malformed(base + 32, "logical size does not fit usize"))?;
            if step == 0 {
                return Err(DexError::malformed(base + 32, "logical size is zero"));
            }
            files.push(file);
            if version != 41 {
                return Ok(files);
            }
            base = base
                .checked_add(step)
                .ok_or_else(|| DexError::malformed(base, "next header offset overflow"))?;
            if base == raw.len() {
                return Ok(files);
            }
            if base > raw.len() || base & 3 != 0 {
                return Err(DexError::malformed(base, "invalid next v041 header offset"));
            }
        }
    }

    /// Returns the exact retained physical container bytes.
    #[must_use]
    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw
    }

    /// Resolves a checked string index.
    ///
    /// # Errors
    /// Returns `OutOfBounds` when the index does not exist.
    pub fn string(&self, index: u32) -> Result<&str, DexError> {
        checked(&self.strings, index, "string").map(|value| value.value.as_str())
    }

    /// Resolves a checked type identifier index.
    ///
    /// # Errors
    /// Returns `OutOfBounds` when the index does not exist.
    pub fn type_id(&self, index: u32) -> Result<&TypeId, DexError> {
        checked(&self.type_ids, index, "type")
    }

    /// Resolves a checked prototype identifier index.
    ///
    /// # Errors
    /// Returns `OutOfBounds` when the index does not exist.
    pub fn proto_id(&self, index: u32) -> Result<&ProtoId, DexError> {
        checked(&self.proto_ids, index, "prototype")
    }

    /// Resolves a checked field identifier index.
    ///
    /// # Errors
    /// Returns `OutOfBounds` when the index does not exist.
    pub fn field_id(&self, index: u32) -> Result<&FieldId, DexError> {
        checked(&self.field_ids, index, "field")
    }

    /// Resolves a checked method identifier index.
    ///
    /// # Errors
    /// Returns `OutOfBounds` when the index does not exist.
    pub fn method_id(&self, index: u32) -> Result<&MethodId, DexError> {
        checked(&self.method_ids, index, "method")
    }

    fn parse_at(raw: Arc<[u8]>, base: usize) -> Result<Self, DexError> {
        let (header, endian) = Header::parse(&raw, base)?;
        let reader = Reader::new(&raw, endian);
        let map = MapList::parse(reader, header.map_off)?;
        let raw_sections = raw_sections(&map, reader.len())?;
        let (string_ids, strings) = parse_strings(reader, header.string_ids)?;
        let type_ids = parse_type_ids(reader, header.type_ids)?;
        let proto_ids = parse_proto_ids(reader, header.proto_ids)?;
        let field_ids = parse_field_ids(reader, header.field_ids)?;
        let method_ids = parse_method_ids(reader, header.method_ids)?;
        let (call_site_ids, method_handles) = parse_modern_ids(reader, &map)?;
        let classes = parse_classes(reader, header.class_defs)?;
        let file = Self {
            raw,
            header,
            map,
            raw_sections,
            string_ids,
            strings,
            type_ids,
            proto_ids,
            field_ids,
            method_ids,
            call_site_ids,
            method_handles,
            classes,
        };
        file.validate_indices()?;
        Ok(file)
    }

    fn validate_indices(&self) -> Result<(), DexError> {
        for item in &self.type_ids {
            self.string(item.descriptor_idx)?;
        }
        for item in &self.proto_ids {
            self.string(item.shorty_idx)?;
            self.type_id(item.return_type_idx)?;
            if let Some(parameters) = &item.parameters {
                for index in &parameters.types {
                    self.type_id(u32::from(*index))?;
                }
            }
        }
        for item in &self.field_ids {
            self.type_id(u32::from(item.class_idx))?;
            self.type_id(u32::from(item.type_idx))?;
            self.string(item.name_idx)?;
        }
        for item in &self.method_ids {
            self.type_id(u32::from(item.class_idx))?;
            self.proto_id(u32::from(item.proto_idx))?;
            self.string(item.name_idx)?;
        }
        for class in &self.classes {
            self.type_id(class.class_idx)?;
            if class.superclass_idx != u32::MAX {
                self.type_id(class.superclass_idx)?;
            }
            if class.source_file_idx != u32::MAX {
                self.string(class.source_file_idx)?;
            }
        }
        Ok(())
    }
}

fn checked<'a, T>(values: &'a [T], index: u32, name: &str) -> Result<&'a T, DexError> {
    let offset = usize::try_from(index)
        .map_err(|_| DexError::malformed(0, format!("{name} index does not fit usize")))?;
    values.get(offset).ok_or_else(|| {
        DexError::malformed(offset, format!("{name} index {index} is out of bounds"))
    })
}

fn parse_modern_ids(
    reader: Reader<'_>,
    map: &MapList,
) -> Result<(Vec<CallSiteId>, Vec<MethodHandle>), DexError> {
    let mut call_sites = Vec::new();
    let mut handles = Vec::new();
    for entry in &map.entries {
        if let MapEntry::Known {
            kind, size, offset, ..
        } = entry
        {
            match kind {
                MapType::CallSiteId => call_sites = parse_call_sites(reader, *offset, *size)?,
                MapType::MethodHandle => handles = parse_method_handles(reader, *offset, *size)?,
                _ => {}
            }
        }
    }
    Ok((call_sites, handles))
}

fn raw_sections(map: &MapList, len: usize) -> Result<Vec<RawSection>, DexError> {
    let mut output = Vec::with_capacity(map.entries.len());
    for (index, entry) in map.entries.iter().enumerate() {
        let (kind, count, offset) = match entry {
            MapEntry::Known {
                kind, size, offset, ..
            } => (RawSectionKind::Known(*kind), *size, *offset),
            MapEntry::Unknown {
                map_type,
                size,
                offset,
                ..
            } => (RawSectionKind::Unknown(*map_type), *size, *offset),
        };
        let start = usize::try_from(offset)
            .map_err(|_| DexError::malformed(0, "section offset does not fit usize"))?;
        let end = map.entries.get(index + 1).map_or(len, entry_offset);
        if end < start || end > len {
            return Err(DexError::malformed(start, "invalid raw section extent"));
        }
        output.push(RawSection {
            kind,
            item_count: count,
            span: RawSpan::from_usize(start, end - start)?,
        });
    }
    Ok(output)
}

fn entry_offset(entry: &MapEntry) -> usize {
    let offset = match entry {
        MapEntry::Known { offset, .. } | MapEntry::Unknown { offset, .. } => *offset,
    };
    match usize::try_from(offset) {
        Ok(value) => value,
        Err(_) => usize::MAX,
    }
}
