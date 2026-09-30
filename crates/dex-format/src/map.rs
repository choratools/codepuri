//! DEX map-list representation.

use std::collections::BTreeSet;

use crate::reader::Reader;
use crate::{DexError, RawSpan};

/// A map item type defined by the Android DEX specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MapType {
    /// Header item.
    Header,
    /// String identifier items.
    StringId,
    /// Type identifier items.
    TypeId,
    /// Prototype identifier items.
    ProtoId,
    /// Field identifier items.
    FieldId,
    /// Method identifier items.
    MethodId,
    /// Class definition items.
    ClassDef,
    /// Call-site identifier items added in v038.
    CallSiteId,
    /// Method-handle items added in v038.
    MethodHandle,
    /// Map list.
    MapList,
    /// Type lists.
    TypeList,
    /// Annotation-set reference lists.
    AnnotationSetRefList,
    /// Annotation sets.
    AnnotationSet,
    /// Class data items.
    ClassData,
    /// Code items.
    Code,
    /// String data items.
    StringData,
    /// Debug information items retained as raw bytes.
    DebugInfo,
    /// Annotation items.
    Annotation,
    /// Encoded arrays.
    EncodedArray,
    /// Annotation directories.
    AnnotationsDirectory,
    /// Hidden-API class data retained as raw bytes.
    HiddenApiClassData,
}

/// One entry in the DEX section map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapEntry {
    /// A recognized section type.
    Known {
        /// Recognized section kind.
        kind: MapType,
        /// Item count.
        size: u32,
        /// Physical-container-relative section offset.
        offset: u32,
        /// Exact 12-byte map-item range.
        span: RawSpan,
    },
    /// An extension section not defined by the supported specification.
    Unknown {
        /// Unrecognized numeric map type.
        map_type: u16,
        /// Physical-container-relative section offset.
        offset: u32,
        /// Item count.
        size: u32,
        /// Exact 12-byte map-item range.
        span: RawSpan,
    },
}

/// Parsed `map_list` and its exact byte range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapList {
    /// Map entries in file order.
    pub entries: Vec<MapEntry>,
    /// Exact map-list byte range.
    pub span: RawSpan,
}

impl MapList {
    pub(crate) fn parse(reader: Reader<'_>, offset: u32) -> Result<Self, DexError> {
        let start = usize::try_from(offset)
            .map_err(|_| DexError::malformed(0, "map offset does not fit usize"))?;
        let count = reader.usize_u32(start)?;
        let byte_size = count
            .checked_mul(12)
            .and_then(|size| size.checked_add(4))
            .ok_or_else(|| DexError::malformed(start, "map list size overflow"))?;
        reader.slice(start, byte_size)?;
        let mut entries = Vec::with_capacity(count);
        let mut seen = BTreeSet::new();
        let mut previous_offset = None;
        for index in 0..count {
            let item_offset = start + 4 + index * 12;
            let map_type = reader.u16(item_offset)?;
            if reader.u16(item_offset + 2)? != 0 {
                return Err(DexError::malformed(
                    item_offset + 2,
                    "map unused field is nonzero",
                ));
            }
            if !seen.insert(map_type) {
                return Err(DexError::malformed(item_offset, "duplicate map type"));
            }
            let size = reader.u32(item_offset + 4)?;
            let section_offset = reader.u32(item_offset + 8)?;
            if let Some(previous) = previous_offset
                && section_offset < previous
            {
                return Err(DexError::malformed(
                    item_offset + 8,
                    "map offsets are not ordered",
                ));
            }
            if usize::try_from(section_offset).map_or(true, |value| value > reader.len()) {
                return Err(DexError::malformed(
                    item_offset + 8,
                    "map offset is out of bounds",
                ));
            }
            previous_offset = Some(section_offset);
            let span = RawSpan::from_usize(item_offset, 12)?;
            entries.push(match known_type(map_type) {
                Some(kind) => MapEntry::Known {
                    kind,
                    size,
                    offset: section_offset,
                    span,
                },
                None => MapEntry::Unknown {
                    map_type,
                    offset: section_offset,
                    size,
                    span,
                },
            });
        }
        Ok(Self {
            entries,
            span: RawSpan::from_usize(start, byte_size)?,
        })
    }
}

const fn known_type(value: u16) -> Option<MapType> {
    match value {
        0x0000 => Some(MapType::Header),
        0x0001 => Some(MapType::StringId),
        0x0002 => Some(MapType::TypeId),
        0x0003 => Some(MapType::ProtoId),
        0x0004 => Some(MapType::FieldId),
        0x0005 => Some(MapType::MethodId),
        0x0006 => Some(MapType::ClassDef),
        0x0007 => Some(MapType::CallSiteId),
        0x0008 => Some(MapType::MethodHandle),
        0x1000 => Some(MapType::MapList),
        0x1001 => Some(MapType::TypeList),
        0x1002 => Some(MapType::AnnotationSetRefList),
        0x1003 => Some(MapType::AnnotationSet),
        0x2000 => Some(MapType::ClassData),
        0x2001 => Some(MapType::Code),
        0x2002 => Some(MapType::StringData),
        0x2003 => Some(MapType::DebugInfo),
        0x2004 => Some(MapType::Annotation),
        0x2005 => Some(MapType::EncodedArray),
        0x2006 => Some(MapType::AnnotationsDirectory),
        0xf000 => Some(MapType::HiddenApiClassData),
        _ => None,
    }
}
