//! DEX class definitions and delta-encoded class data.

use crate::annotations::{AnnotationsDirectory, parse_directory};
use crate::code::{CodeItem, parse_code};
use crate::cursor::Cursor;
use crate::encoded_value::{EncodedValue, parse_encoded_array};
use crate::header::SectionHeader;
use crate::ids::{TypeList, parse_type_list};
use crate::reader::{Reader, checked_table};
use crate::{DexError, RawSpan};

/// A delta-encoded field declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedField {
    /// Absolute field identifier index.
    pub field_idx: u32,
    /// DEX access flags.
    pub access_flags: u32,
}

/// A delta-encoded method declaration and optional code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedMethod {
    /// Absolute method identifier index.
    pub method_idx: u32,
    /// DEX access flags.
    pub access_flags: u32,
    /// Physical code-item offset, or zero.
    pub code_off: u32,
    /// Parsed method code when `code_off` is nonzero.
    pub code: Option<CodeItem>,
}

/// The variable-width data attached to one class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassData {
    /// Static fields.
    pub static_fields: Vec<EncodedField>,
    /// Instance fields.
    pub instance_fields: Vec<EncodedField>,
    /// Direct methods.
    pub direct_methods: Vec<EncodedMethod>,
    /// Virtual methods.
    pub virtual_methods: Vec<EncodedMethod>,
    /// Exact class-data byte range.
    pub span: RawSpan,
}

/// A fixed-width class definition and all referenced class-owned data.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassDef {
    /// Defined class type index.
    pub class_idx: u32,
    /// DEX access flags.
    pub access_flags: u32,
    /// Superclass type index, or `u32::MAX`.
    pub superclass_idx: u32,
    /// Interface type-list offset, or zero.
    pub interfaces_off: u32,
    /// Parsed interfaces.
    pub interfaces: Option<TypeList>,
    /// Source-file string index, or `u32::MAX`.
    pub source_file_idx: u32,
    /// Annotation-directory offset, or zero.
    pub annotations_off: u32,
    /// Parsed class annotations.
    pub annotations: Option<AnnotationsDirectory>,
    /// Class-data offset, or zero.
    pub class_data_off: u32,
    /// Parsed class data.
    pub class_data: Option<ClassData>,
    /// Static-values encoded-array offset, or zero.
    pub static_values_off: u32,
    /// Parsed static values.
    pub static_values: Option<Vec<EncodedValue>>,
    /// Exact fixed-width class definition range.
    pub span: RawSpan,
}

pub(crate) fn parse_classes(
    reader: Reader<'_>,
    section: SectionHeader,
) -> Result<Vec<ClassDef>, DexError> {
    let start = checked_table(reader, section.offset, section.size, 32, "class_defs")?;
    let count = usize::try_from(section.size)
        .map_err(|_| DexError::malformed(start, "class count does not fit usize"))?;
    let bytes = reader.slice(0, reader.len())?;
    let mut classes = Vec::with_capacity(count);
    for index in 0..count {
        let offset = start + index * 32;
        let interfaces_off = reader.u32(offset + 12)?;
        let annotations_off = reader.u32(offset + 20)?;
        let class_data_off = reader.u32(offset + 24)?;
        let static_values_off = reader.u32(offset + 28)?;
        classes.push(ClassDef {
            class_idx: reader.u32(offset)?,
            access_flags: reader.u32(offset + 4)?,
            superclass_idx: reader.u32(offset + 8)?,
            interfaces_off,
            interfaces: optional(interfaces_off, || parse_type_list(reader, interfaces_off))?,
            source_file_idx: reader.u32(offset + 16)?,
            annotations_off,
            annotations: optional(annotations_off, || parse_directory(reader, annotations_off))?,
            class_data_off,
            class_data: optional(class_data_off, || {
                parse_class_data(reader, bytes, class_data_off)
            })?,
            static_values_off,
            static_values: optional(static_values_off, || {
                parse_encoded_array(bytes, static_values_off).map(|value| value.0)
            })?,
            span: RawSpan::from_usize(offset, 32)?,
        });
    }
    Ok(classes)
}

fn parse_class_data(reader: Reader<'_>, bytes: &[u8], offset: u32) -> Result<ClassData, DexError> {
    let start = usize::try_from(offset)
        .map_err(|_| DexError::malformed(0, "class data offset does not fit usize"))?;
    let mut cursor = Cursor::new(bytes, start);
    let static_count = read_count(&mut cursor, start)?;
    let instance_count = read_count(&mut cursor, start)?;
    let direct_count = read_count(&mut cursor, start)?;
    let virtual_count = read_count(&mut cursor, start)?;
    let direct_values = direct_count
        .checked_mul(3)
        .ok_or_else(|| DexError::malformed(start, "direct method count overflow"))?;
    let virtual_values = virtual_count
        .checked_mul(3)
        .ok_or_else(|| DexError::malformed(start, "virtual method count overflow"))?;
    let minimum = static_count
        .checked_add(instance_count)
        .and_then(|value| value.checked_add(direct_values))
        .and_then(|value| value.checked_add(virtual_values))
        .ok_or_else(|| DexError::malformed(start, "class data count overflow"))?;
    if minimum > cursor.remaining() {
        return Err(DexError::malformed(start, "class data counts exceed input"));
    }
    let static_fields = parse_fields(&mut cursor, static_count)?;
    let instance_fields = parse_fields(&mut cursor, instance_count)?;
    let direct_methods = parse_methods(reader, &mut cursor, direct_count)?;
    let virtual_methods = parse_methods(reader, &mut cursor, virtual_count)?;
    Ok(ClassData {
        static_fields,
        instance_fields,
        direct_methods,
        virtual_methods,
        span: RawSpan::from_usize(start, cursor.position() - start)?,
    })
}

fn read_count(cursor: &mut Cursor<'_>, start: usize) -> Result<usize, DexError> {
    usize::try_from(cursor.uleb()?)
        .map_err(|_| DexError::malformed(start, "class data count does not fit usize"))
}

fn parse_fields(cursor: &mut Cursor<'_>, count: usize) -> Result<Vec<EncodedField>, DexError> {
    let mut fields = Vec::with_capacity(count);
    let mut field_idx = 0_u32;
    for _ in 0..count {
        field_idx = field_idx
            .checked_add(cursor.uleb()?)
            .ok_or_else(|| DexError::malformed(cursor.position(), "field index overflow"))?;
        fields.push(EncodedField {
            field_idx,
            access_flags: cursor.uleb()?,
        });
    }
    Ok(fields)
}

fn parse_methods(
    reader: Reader<'_>,
    cursor: &mut Cursor<'_>,
    count: usize,
) -> Result<Vec<EncodedMethod>, DexError> {
    let mut methods = Vec::with_capacity(count);
    let mut method_idx = 0_u32;
    for _ in 0..count {
        method_idx = method_idx
            .checked_add(cursor.uleb()?)
            .ok_or_else(|| DexError::malformed(cursor.position(), "method index overflow"))?;
        let access_flags = cursor.uleb()?;
        let code_off = cursor.uleb()?;
        methods.push(EncodedMethod {
            method_idx,
            access_flags,
            code_off,
            code: optional(code_off, || parse_code(reader, code_off))?,
        });
    }
    Ok(methods)
}

fn optional<T>(
    offset: u32,
    parse: impl FnOnce() -> Result<T, DexError>,
) -> Result<Option<T>, DexError> {
    (offset != 0).then(parse).transpose()
}
