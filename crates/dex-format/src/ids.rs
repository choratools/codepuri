//! DEX identifier tables and type lists.

use crate::header::SectionHeader;
use crate::reader::{Reader, checked_table};
use crate::{DexError, RawSpan};

/// A type identifier referencing a descriptor string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeId {
    /// Descriptor string index.
    pub descriptor_idx: u32,
    /// Exact item range.
    pub span: RawSpan,
}

/// A method prototype identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtoId {
    /// Short-form descriptor string index.
    pub shorty_idx: u32,
    /// Return type index.
    pub return_type_idx: u32,
    /// Parameter type-list offset, or zero.
    pub parameters_off: u32,
    /// Parsed parameter types, when present.
    pub parameters: Option<TypeList>,
    /// Exact item range.
    pub span: RawSpan,
}

/// A field identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldId {
    /// Defining class type index.
    pub class_idx: u16,
    /// Field type index.
    pub type_idx: u16,
    /// Field name string index.
    pub name_idx: u32,
    /// Exact item range.
    pub span: RawSpan,
}

/// A method identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodId {
    /// Defining class type index.
    pub class_idx: u16,
    /// Prototype index.
    pub proto_idx: u16,
    /// Method name string index.
    pub name_idx: u32,
    /// Exact item range.
    pub span: RawSpan,
}

/// A list of type indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeList {
    /// Type indices in declaration order.
    pub types: Vec<u16>,
    /// Item range, excluding alignment after the final entry.
    pub span: RawSpan,
}

/// A v038+ call-site identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSiteId {
    /// Encoded-array offset describing the call site.
    pub call_site_off: u32,
    /// Exact item range.
    pub span: RawSpan,
}

/// A v038+ method handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodHandle {
    /// Handle-kind numeric value from the specification.
    pub handle_type: u16,
    /// Field or method identifier selected by `handle_type`.
    pub field_or_method_idx: u16,
    /// Exact item range.
    pub span: RawSpan,
}

pub(crate) fn parse_type_ids(
    reader: Reader<'_>,
    section: SectionHeader,
) -> Result<Vec<TypeId>, DexError> {
    let start = checked_table(reader, section.offset, section.size, 4, "type_ids")?;
    collect(section.size, start, 4, |offset| {
        Ok(TypeId {
            descriptor_idx: reader.u32(offset)?,
            span: RawSpan::from_usize(offset, 4)?,
        })
    })
}

pub(crate) fn parse_proto_ids(
    reader: Reader<'_>,
    section: SectionHeader,
) -> Result<Vec<ProtoId>, DexError> {
    let start = checked_table(reader, section.offset, section.size, 12, "proto_ids")?;
    collect(section.size, start, 12, |offset| {
        let parameters_off = reader.u32(offset + 8)?;
        Ok(ProtoId {
            shorty_idx: reader.u32(offset)?,
            return_type_idx: reader.u32(offset + 4)?,
            parameters_off,
            parameters: (parameters_off != 0)
                .then(|| parse_type_list(reader, parameters_off))
                .transpose()?,
            span: RawSpan::from_usize(offset, 12)?,
        })
    })
}

pub(crate) fn parse_field_ids(
    reader: Reader<'_>,
    section: SectionHeader,
) -> Result<Vec<FieldId>, DexError> {
    let start = checked_table(reader, section.offset, section.size, 8, "field_ids")?;
    collect(section.size, start, 8, |offset| {
        Ok(FieldId {
            class_idx: reader.u16(offset)?,
            type_idx: reader.u16(offset + 2)?,
            name_idx: reader.u32(offset + 4)?,
            span: RawSpan::from_usize(offset, 8)?,
        })
    })
}

pub(crate) fn parse_method_ids(
    reader: Reader<'_>,
    section: SectionHeader,
) -> Result<Vec<MethodId>, DexError> {
    let start = checked_table(reader, section.offset, section.size, 8, "method_ids")?;
    collect(section.size, start, 8, |offset| {
        Ok(MethodId {
            class_idx: reader.u16(offset)?,
            proto_idx: reader.u16(offset + 2)?,
            name_idx: reader.u32(offset + 4)?,
            span: RawSpan::from_usize(offset, 8)?,
        })
    })
}

pub(crate) fn parse_call_sites(
    reader: Reader<'_>,
    offset: u32,
    size: u32,
) -> Result<Vec<CallSiteId>, DexError> {
    let start = checked_table(reader, offset, size, 4, "call_site_ids")?;
    collect(size, start, 4, |item_offset| {
        Ok(CallSiteId {
            call_site_off: reader.u32(item_offset)?,
            span: RawSpan::from_usize(item_offset, 4)?,
        })
    })
}

pub(crate) fn parse_method_handles(
    reader: Reader<'_>,
    offset: u32,
    size: u32,
) -> Result<Vec<MethodHandle>, DexError> {
    let start = checked_table(reader, offset, size, 8, "method_handles")?;
    collect(size, start, 8, |item_offset| {
        if reader.u16(item_offset + 2)? != 0 || reader.u16(item_offset + 6)? != 0 {
            return Err(DexError::malformed(
                item_offset,
                "method handle unused field is nonzero",
            ));
        }
        let handle_type = reader.u16(item_offset)?;
        if handle_type > 8 {
            return Err(DexError::malformed(
                item_offset,
                "invalid method handle type",
            ));
        }
        Ok(MethodHandle {
            handle_type,
            field_or_method_idx: reader.u16(item_offset + 4)?,
            span: RawSpan::from_usize(item_offset, 8)?,
        })
    })
}

pub(crate) fn parse_type_list(reader: Reader<'_>, offset: u32) -> Result<TypeList, DexError> {
    let start = usize::try_from(offset)
        .map_err(|_| DexError::malformed(0, "type list offset does not fit usize"))?;
    let count = reader.usize_u32(start)?;
    let bytes = count
        .checked_mul(2)
        .and_then(|size| size.checked_add(4))
        .ok_or_else(|| DexError::malformed(start, "type list size overflow"))?;
    reader.slice(start, bytes)?;
    let mut types = Vec::with_capacity(count);
    for index in 0..count {
        types.push(reader.u16(start + 4 + index * 2)?);
    }
    Ok(TypeList {
        types,
        span: RawSpan::from_usize(start, bytes)?,
    })
}

fn collect<T>(
    count: u32,
    start: usize,
    stride: usize,
    mut parse: impl FnMut(usize) -> Result<T, DexError>,
) -> Result<Vec<T>, DexError> {
    let count = usize::try_from(count)
        .map_err(|_| DexError::malformed(start, "table count does not fit usize"))?;
    let mut output = Vec::with_capacity(count);
    for index in 0..count {
        output.push(parse(start + index * stride)?);
    }
    Ok(output)
}
