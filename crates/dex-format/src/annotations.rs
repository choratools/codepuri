//! DEX annotation items, sets, and directories.

use crate::cursor::Cursor;
use crate::encoded_value::{EncodedAnnotation, parse_annotation};
use crate::reader::Reader;
use crate::{DexError, RawSpan};

/// Annotation visibility policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// Visible only to build tools.
    Build,
    /// Visible at runtime.
    Runtime,
    /// Visible to the runtime system.
    System,
}

/// One annotation item.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationItem {
    /// Visibility policy.
    pub visibility: Visibility,
    /// Encoded annotation body.
    pub annotation: EncodedAnnotation,
    /// Exact item range.
    pub span: RawSpan,
}

/// A set of annotation items.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationSet {
    /// Parsed annotation entries.
    pub entries: Vec<AnnotationItem>,
    /// Exact set range, containing the size and offset array.
    pub span: RawSpan,
}

/// A parameter annotation-set reference list.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationSetRefList {
    /// Annotation sets by parameter; zero offsets become `None`.
    pub entries: Vec<Option<AnnotationSet>>,
    /// Exact reference-list range.
    pub span: RawSpan,
}

/// An annotation set attached to an indexed field or method.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberAnnotations {
    /// Field or method index.
    pub member_idx: u32,
    /// Attached annotation set.
    pub annotations: AnnotationSet,
}

/// Parameter annotations attached to a method.
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterAnnotations {
    /// Method index.
    pub method_idx: u32,
    /// Per-parameter annotation sets.
    pub annotations: AnnotationSetRefList,
}

/// Complete annotations directory referenced by a class definition.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationsDirectory {
    /// Annotations directly attached to the class.
    pub class_annotations: Option<AnnotationSet>,
    /// Field annotations.
    pub field_annotations: Vec<MemberAnnotations>,
    /// Method annotations.
    pub method_annotations: Vec<MemberAnnotations>,
    /// Method parameter annotations.
    pub parameter_annotations: Vec<ParameterAnnotations>,
    /// Exact directory range, excluding referenced sets and items.
    pub span: RawSpan,
}

pub(crate) fn parse_directory(
    reader: Reader<'_>,
    offset: u32,
) -> Result<AnnotationsDirectory, DexError> {
    let start = usize::try_from(offset)
        .map_err(|_| DexError::malformed(0, "annotations offset does not fit usize"))?;
    reader.slice(start, 16)?;
    let class_off = reader.u32(start)?;
    let field_count = reader.usize_u32(start + 4)?;
    let method_count = reader.usize_u32(start + 8)?;
    let parameter_count = reader.usize_u32(start + 12)?;
    let total = field_count
        .checked_add(method_count)
        .and_then(|value| value.checked_add(parameter_count))
        .ok_or_else(|| DexError::malformed(start, "annotation directory count overflow"))?;
    let table_bytes = total
        .checked_mul(8)
        .and_then(|value| value.checked_add(16))
        .ok_or_else(|| DexError::malformed(start, "annotation directory size overflow"))?;
    reader.slice(start, table_bytes)?;
    let mut cursor = start + 16;
    let field_annotations = parse_members(reader, &mut cursor, field_count)?;
    let method_annotations = parse_members(reader, &mut cursor, method_count)?;
    let mut parameter_annotations = Vec::with_capacity(parameter_count);
    for _ in 0..parameter_count {
        let method_idx = reader.u32(cursor)?;
        let list_off = reader.u32(cursor + 4)?;
        parameter_annotations.push(ParameterAnnotations {
            method_idx,
            annotations: parse_set_ref_list(reader, list_off)?,
        });
        cursor += 8;
    }
    Ok(AnnotationsDirectory {
        class_annotations: (class_off != 0)
            .then(|| parse_set(reader, class_off))
            .transpose()?,
        field_annotations,
        method_annotations,
        parameter_annotations,
        span: RawSpan::from_usize(start, table_bytes)?,
    })
}

fn parse_members(
    reader: Reader<'_>,
    cursor: &mut usize,
    count: usize,
) -> Result<Vec<MemberAnnotations>, DexError> {
    let mut members = Vec::with_capacity(count);
    for _ in 0..count {
        let member_idx = reader.u32(*cursor)?;
        let set_off = reader.u32(*cursor + 4)?;
        members.push(MemberAnnotations {
            member_idx,
            annotations: parse_set(reader, set_off)?,
        });
        *cursor += 8;
    }
    Ok(members)
}

fn parse_set(reader: Reader<'_>, offset: u32) -> Result<AnnotationSet, DexError> {
    let start = usize::try_from(offset)
        .map_err(|_| DexError::malformed(0, "annotation set offset does not fit usize"))?;
    let count = reader.usize_u32(start)?;
    let size = count
        .checked_mul(4)
        .and_then(|value| value.checked_add(4))
        .ok_or_else(|| DexError::malformed(start, "annotation set size overflow"))?;
    reader.slice(start, size)?;
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        entries.push(parse_item(reader, reader.u32(start + 4 + index * 4)?)?);
    }
    Ok(AnnotationSet {
        entries,
        span: RawSpan::from_usize(start, size)?,
    })
}

fn parse_set_ref_list(reader: Reader<'_>, offset: u32) -> Result<AnnotationSetRefList, DexError> {
    let start = usize::try_from(offset)
        .map_err(|_| DexError::malformed(0, "annotation ref-list offset does not fit usize"))?;
    let count = reader.usize_u32(start)?;
    let size = count
        .checked_mul(4)
        .and_then(|value| value.checked_add(4))
        .ok_or_else(|| DexError::malformed(start, "annotation ref-list size overflow"))?;
    reader.slice(start, size)?;
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let set_off = reader.u32(start + 4 + index * 4)?;
        entries.push(
            (set_off != 0)
                .then(|| parse_set(reader, set_off))
                .transpose()?,
        );
    }
    Ok(AnnotationSetRefList {
        entries,
        span: RawSpan::from_usize(start, size)?,
    })
}

fn parse_item(reader: Reader<'_>, offset: u32) -> Result<AnnotationItem, DexError> {
    let start = usize::try_from(offset)
        .map_err(|_| DexError::malformed(0, "annotation item offset does not fit usize"))?;
    let visibility = match reader.u8(start)? {
        0 => Visibility::Build,
        1 => Visibility::Runtime,
        2 => Visibility::System,
        _ => return Err(DexError::malformed(start, "invalid annotation visibility")),
    };
    let bytes = reader.slice(0, reader.len())?;
    let mut cursor = Cursor::new(bytes, start + 1);
    let annotation = parse_annotation(&mut cursor, 0)?;
    Ok(AnnotationItem {
        visibility,
        annotation,
        span: RawSpan::from_usize(start, cursor.position() - start)?,
    })
}
