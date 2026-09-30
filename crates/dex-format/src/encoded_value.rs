//! DEX encoded arrays, annotations, and values.

use crate::cursor::Cursor;
use crate::{DexError, RawSpan};

const MAX_NESTING: usize = 256;

/// A named element in an encoded annotation.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationElement {
    /// String index containing the element name.
    pub name_idx: u32,
    /// Element value.
    pub value: EncodedValue,
}

/// An annotation body embedded in an encoded value or annotation item.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedAnnotation {
    /// Annotation type index.
    pub type_idx: u32,
    /// Named annotation elements.
    pub elements: Vec<AnnotationElement>,
    /// Exact encoded byte range.
    pub span: RawSpan,
}

/// A typed encoded value and its exact source range.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedValue {
    /// Decoded value.
    pub value: EncodedValueKind,
    /// Exact encoded byte range, including the header byte.
    pub span: RawSpan,
}

/// Every value kind defined by the DEX encoded-value format.
#[derive(Debug, Clone, PartialEq)]
pub enum EncodedValueKind {
    /// Signed byte.
    Byte(i8),
    /// Signed short.
    Short(i16),
    /// Unsigned UTF-16 code unit.
    Char(u16),
    /// Signed integer.
    Int(i32),
    /// Signed long.
    Long(i64),
    /// IEEE-754 single precision value.
    Float(f32),
    /// IEEE-754 double precision value.
    Double(f64),
    /// Prototype index.
    MethodType(u32),
    /// Method-handle index.
    MethodHandle(u32),
    /// String index.
    String(u32),
    /// Type index.
    Type(u32),
    /// Field index.
    Field(u32),
    /// Method index.
    Method(u32),
    /// Enum field index.
    Enum(u32),
    /// Nested encoded array.
    Array(Vec<EncodedValue>),
    /// Nested encoded annotation.
    Annotation(EncodedAnnotation),
    /// Null value.
    Null,
    /// Boolean encoded entirely in `value_arg`.
    Boolean(bool),
}

/// Parses an encoded array item at a physical byte offset.
pub fn parse_encoded_array(
    bytes: &[u8],
    offset: u32,
) -> Result<(Vec<EncodedValue>, RawSpan), DexError> {
    let start = usize::try_from(offset)
        .map_err(|_| DexError::malformed(0, "encoded array offset does not fit usize"))?;
    let mut cursor = Cursor::new(bytes, start);
    let values = parse_array(&mut cursor, 0)?;
    Ok((
        values,
        RawSpan::from_usize(start, cursor.position() - start)?,
    ))
}

pub(crate) fn parse_value(cursor: &mut Cursor<'_>, depth: usize) -> Result<EncodedValue, DexError> {
    if depth > MAX_NESTING {
        return Err(DexError::malformed(
            cursor.position(),
            "encoded-value nesting is too deep",
        ));
    }
    let start = cursor.position();
    let header = cursor.u8()?;
    let value_type = header & 0x1f;
    let value_arg = header >> 5;
    let value = match value_type {
        0x00 => EncodedValueKind::Byte(narrow_signed(cursor, value_arg, 1)?),
        0x02 => EncodedValueKind::Short(narrow_signed(cursor, value_arg, 2)?),
        0x03 => EncodedValueKind::Char(narrow_unsigned(cursor, value_arg, 2)?),
        0x04 => EncodedValueKind::Int(narrow_signed(cursor, value_arg, 4)?),
        0x06 => EncodedValueKind::Long(read_signed(cursor, value_arg, 8)?),
        0x10 => {
            let bits = u32::try_from(read_float_bits(cursor, value_arg, 4)?)
                .map_err(|_| DexError::malformed(start, "float bits exceed u32"))?;
            EncodedValueKind::Float(f32::from_bits(bits))
        }
        0x11 => EncodedValueKind::Double(f64::from_bits(read_float_bits(cursor, value_arg, 8)?)),
        0x15 => EncodedValueKind::MethodType(read_index(cursor, value_arg)?),
        0x16 => EncodedValueKind::MethodHandle(read_index(cursor, value_arg)?),
        0x17 => EncodedValueKind::String(read_index(cursor, value_arg)?),
        0x18 => EncodedValueKind::Type(read_index(cursor, value_arg)?),
        0x19 => EncodedValueKind::Field(read_index(cursor, value_arg)?),
        0x1a => EncodedValueKind::Method(read_index(cursor, value_arg)?),
        0x1b => EncodedValueKind::Enum(read_index(cursor, value_arg)?),
        0x1c if value_arg == 0 => EncodedValueKind::Array(parse_array(cursor, depth + 1)?),
        0x1d if value_arg == 0 => {
            EncodedValueKind::Annotation(parse_annotation(cursor, depth + 1)?)
        }
        0x1e if value_arg == 0 => EncodedValueKind::Null,
        0x1f if value_arg <= 1 => EncodedValueKind::Boolean(value_arg == 1),
        0x1c..=0x1f => return Err(DexError::malformed(start, "invalid encoded-value argument")),
        _ => return Err(DexError::malformed(start, "unknown encoded-value type")),
    };
    Ok(EncodedValue {
        value,
        span: RawSpan::from_usize(start, cursor.position() - start)?,
    })
}

pub(crate) fn parse_annotation(
    cursor: &mut Cursor<'_>,
    depth: usize,
) -> Result<EncodedAnnotation, DexError> {
    if depth > MAX_NESTING {
        return Err(DexError::malformed(
            cursor.position(),
            "annotation nesting is too deep",
        ));
    }
    let start = cursor.position();
    let type_idx = cursor.uleb()?;
    let count = usize::try_from(cursor.uleb()?)
        .map_err(|_| DexError::malformed(start, "annotation size does not fit usize"))?;
    if count > cursor.remaining() / 2 {
        return Err(DexError::malformed(
            start,
            "annotation element count exceeds input",
        ));
    }
    let mut elements = Vec::with_capacity(count);
    for _ in 0..count {
        elements.push(AnnotationElement {
            name_idx: cursor.uleb()?,
            value: parse_value(cursor, depth + 1)?,
        });
    }
    Ok(EncodedAnnotation {
        type_idx,
        elements,
        span: RawSpan::from_usize(start, cursor.position() - start)?,
    })
}

fn parse_array(cursor: &mut Cursor<'_>, depth: usize) -> Result<Vec<EncodedValue>, DexError> {
    let start = cursor.position();
    let count = usize::try_from(cursor.uleb()?)
        .map_err(|_| DexError::malformed(start, "encoded array size does not fit usize"))?;
    if count > cursor.remaining() {
        return Err(DexError::malformed(
            start,
            "encoded array count exceeds input",
        ));
    }
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(parse_value(cursor, depth + 1)?);
    }
    Ok(values)
}

fn read_index(cursor: &mut Cursor<'_>, value_arg: u8) -> Result<u32, DexError> {
    let value = read_unsigned(cursor, value_arg, 4)?;
    u32::try_from(value).map_err(|_| DexError::malformed(cursor.position(), "index exceeds u32"))
}

fn narrow_signed<T>(cursor: &mut Cursor<'_>, value_arg: u8, maximum: usize) -> Result<T, DexError>
where
    T: TryFrom<i64>,
{
    T::try_from(read_signed(cursor, value_arg, maximum)?)
        .map_err(|_| DexError::malformed(cursor.position(), "signed encoded value overflow"))
}

fn narrow_unsigned<T>(cursor: &mut Cursor<'_>, value_arg: u8, maximum: usize) -> Result<T, DexError>
where
    T: TryFrom<u64>,
{
    T::try_from(read_unsigned(cursor, value_arg, maximum)?)
        .map_err(|_| DexError::malformed(cursor.position(), "unsigned encoded value overflow"))
}

fn read_unsigned(cursor: &mut Cursor<'_>, value_arg: u8, maximum: usize) -> Result<u64, DexError> {
    let count = usize::from(value_arg) + 1;
    if count > maximum {
        return Err(DexError::malformed(
            cursor.position(),
            "encoded value is too wide",
        ));
    }
    let mut value = 0_u64;
    for (index, byte) in cursor.bytes(count)?.iter().copied().enumerate() {
        value |= u64::from(byte) << (index * 8);
    }
    Ok(value)
}

fn read_signed(cursor: &mut Cursor<'_>, value_arg: u8, maximum: usize) -> Result<i64, DexError> {
    let count = usize::from(value_arg) + 1;
    let value = read_unsigned(cursor, value_arg, maximum)?;
    let shift = 64 - count * 8;
    Ok(i64::from_ne_bytes((value << shift).to_ne_bytes()) >> shift)
}

fn read_float_bits(
    cursor: &mut Cursor<'_>,
    value_arg: u8,
    maximum: usize,
) -> Result<u64, DexError> {
    let count = usize::from(value_arg) + 1;
    let value = read_unsigned(cursor, value_arg, maximum)?;
    Ok(value << ((maximum - count) * 8))
}
