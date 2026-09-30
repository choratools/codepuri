//! DEX string identifier and string-data items.

use crate::header::SectionHeader;
use crate::reader::{Reader, checked_table};
use crate::uleb::{decode_mutf8, read_uleb128};
use crate::{DexError, RawSpan};

/// A string identifier pointing to a string-data item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringId {
    /// Physical offset of the string-data item.
    pub string_data_off: u32,
    /// Exact identifier byte range.
    pub span: RawSpan,
}

/// A decoded string with its exact encoded byte range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringData {
    /// Decoded Unicode scalar view; isolated UTF-16 surrogates become U+FFFD.
    pub value: String,
    /// Declared UTF-16 code-unit count.
    pub utf16_size: u32,
    /// Full item range including length and zero terminator.
    pub span: RawSpan,
}

pub(crate) fn parse_strings(
    reader: Reader<'_>,
    section: SectionHeader,
) -> Result<(Vec<StringId>, Vec<StringData>), DexError> {
    let start = checked_table(reader, section.offset, section.size, 4, "string_ids")?;
    let count = usize::try_from(section.size)
        .map_err(|_| DexError::malformed(start, "string count does not fit usize"))?;
    let mut ids = Vec::with_capacity(count);
    let mut data = Vec::with_capacity(count);
    for index in 0..count {
        let item_offset = start + index * 4;
        let string_data_off = reader.u32(item_offset)?;
        ids.push(StringId {
            string_data_off,
            span: RawSpan::from_usize(item_offset, 4)?,
        });
        data.push(parse_string_data(reader, string_data_off)?);
    }
    Ok((ids, data))
}

fn parse_string_data(reader: Reader<'_>, offset: u32) -> Result<StringData, DexError> {
    let start = usize::try_from(offset)
        .map_err(|_| DexError::malformed(0, "string data offset does not fit usize"))?;
    let (utf16_size, prefix_size) = read_uleb128(reader.slice(0, reader.len())?, start)?;
    let data_offset = start
        .checked_add(prefix_size)
        .ok_or_else(|| DexError::malformed(start, "string data offset overflow"))?;
    if usize::try_from(utf16_size).map_or(true, |size| size > reader.len() - data_offset) {
        return Err(DexError::malformed(
            start,
            "UTF-16 size exceeds remaining input",
        ));
    }
    let (value, encoded_size) =
        decode_mutf8(reader.slice(0, reader.len())?, data_offset, utf16_size)?;
    let terminator = data_offset
        .checked_add(encoded_size)
        .ok_or_else(|| DexError::malformed(data_offset, "string terminator offset overflow"))?;
    if reader.u8(terminator)? != 0 {
        return Err(DexError::malformed(terminator, "missing string terminator"));
    }
    let size = prefix_size
        .checked_add(encoded_size)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| DexError::malformed(start, "string item size overflow"))?;
    Ok(StringData {
        value,
        utf16_size,
        span: RawSpan::from_usize(start, size)?,
    })
}
