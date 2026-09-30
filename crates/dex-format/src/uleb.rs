//! Checked LEB128 and Modified UTF-8 codecs.

use crate::DexError;

/// Reads a DEX `uleb128` value and returns the value and bytes consumed.
pub fn read_uleb128(bytes: &[u8], offset: usize) -> Result<(u32, usize), DexError> {
    let (value, consumed) = read_unsigned(bytes, offset, 5, 32)?;
    let value =
        u32::try_from(value).map_err(|_| DexError::malformed(offset, "uleb128 exceeds u32"))?;
    Ok((value, consumed))
}

/// Reads a 64-bit unsigned LEB128 value.
pub fn read_uleb128_u64(bytes: &[u8], offset: usize) -> Result<(u64, usize), DexError> {
    read_unsigned(bytes, offset, 10, 64)
}

/// Reads a DEX `sleb128` value as an `i32`.
pub fn read_sleb128(bytes: &[u8], offset: usize) -> Result<(i32, usize), DexError> {
    let (value, consumed) = read_signed(bytes, offset, 5, 32)?;
    let value =
        i32::try_from(value).map_err(|_| DexError::malformed(offset, "sleb128 exceeds i32"))?;
    Ok((value, consumed))
}

/// Reads a signed 64-bit LEB128 value.
pub fn read_sleb128_i64(bytes: &[u8], offset: usize) -> Result<(i64, usize), DexError> {
    read_signed(bytes, offset, 10, 64)
}

/// Encodes a `u32` as canonical unsigned LEB128.
pub fn encode_uleb128(value: u32) -> Vec<u8> {
    encode_uleb128_u64(u64::from(value))
}

/// Encodes a `u64` as canonical unsigned LEB128.
pub fn encode_uleb128_u64(mut value: u64) -> Vec<u8> {
    let mut output = Vec::with_capacity(10);
    loop {
        let mut byte = (value & 0x7f).to_le_bytes()[0];
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            return output;
        }
    }
}

/// Decodes `utf16_size` code units of DEX Modified UTF-8.
///
/// The returned byte count excludes a string-data terminator.
pub fn decode_mutf8(
    bytes: &[u8],
    offset: usize,
    utf16_size: u32,
) -> Result<(String, usize), DexError> {
    let target = usize::try_from(utf16_size)
        .map_err(|_| DexError::malformed(offset, "MUTF-8 length does not fit usize"))?;
    let mut units = Vec::with_capacity(target);
    let mut cursor = offset;
    while units.len() < target {
        let first = byte(bytes, cursor)?;
        let (unit, width) = match first {
            0x01..=0x7f => (u16::from(first), 1),
            0xc0..=0xdf => decode_two(bytes, cursor, first)?,
            0xe0..=0xef => decode_three(bytes, cursor, first)?,
            _ => return Err(DexError::malformed(cursor, "invalid MUTF-8 leading byte")),
        };
        units.push(unit);
        cursor = cursor
            .checked_add(width)
            .ok_or_else(|| DexError::malformed(cursor, "MUTF-8 offset overflow"))?;
    }
    let decoded = String::from_utf16_lossy(&units);
    Ok((decoded, cursor - offset))
}

/// Encodes a Rust string as DEX Modified UTF-8 without a trailing zero byte.
pub fn encode_mutf8(value: &str) -> Vec<u8> {
    let mut output = Vec::with_capacity(value.len());
    for unit in value.encode_utf16() {
        match unit {
            0 => output.extend_from_slice(&[0xc0, 0x80]),
            0x01..=0x7f => output.push(unit.to_le_bytes()[0]),
            0x80..=0x07ff => {
                output.push(0xc0 | (unit >> 6).to_le_bytes()[0]);
                output.push(0x80 | (unit & 0x3f).to_le_bytes()[0]);
            }
            _ => {
                output.push(0xe0 | (unit >> 12).to_le_bytes()[0]);
                output.push(0x80 | ((unit >> 6) & 0x3f).to_le_bytes()[0]);
                output.push(0x80 | (unit & 0x3f).to_le_bytes()[0]);
            }
        }
    }
    output
}

fn read_unsigned(
    bytes: &[u8],
    offset: usize,
    max_bytes: usize,
    bits: u32,
) -> Result<(u64, usize), DexError> {
    let mut value = 0_u64;
    for index in 0..max_bytes {
        let current = byte(bytes, offset + index)?;
        let shift = u32::try_from(index * 7)
            .map_err(|_| DexError::malformed(offset, "LEB128 shift overflow"))?;
        let payload = u64::from(current & 0x7f);
        if shift >= bits || (shift + 7 > bits && payload >= (1_u64 << (bits - shift))) {
            return Err(DexError::malformed(offset, "uleb128 overflow"));
        }
        value |= payload << shift;
        if current & 0x80 == 0 {
            return Ok((value, index + 1));
        }
    }
    Err(DexError::malformed(offset, "unterminated uleb128"))
}

fn read_signed(
    bytes: &[u8],
    offset: usize,
    max_bytes: usize,
    bits: u32,
) -> Result<(i64, usize), DexError> {
    let mut value = 0_u64;
    for index in 0..max_bytes {
        let current = byte(bytes, offset + index)?;
        let shift = u32::try_from(index * 7)
            .map_err(|_| DexError::malformed(offset, "LEB128 shift overflow"))?;
        value |= u64::from(current & 0x7f) << shift;
        if current & 0x80 == 0 {
            let used = shift + 7;
            if used < 64 && current & 0x40 != 0 {
                value |= u64::MAX << used;
            }
            let signed = i64::from_ne_bytes(value.to_ne_bytes());
            let minimum = -(1_i128 << (bits - 1));
            let maximum = (1_i128 << (bits - 1)) - 1;
            if i128::from(signed) < minimum || i128::from(signed) > maximum {
                return Err(DexError::malformed(offset, "sleb128 overflow"));
            }
            return Ok((signed, index + 1));
        }
    }
    Err(DexError::malformed(offset, "unterminated sleb128"))
}

fn byte(bytes: &[u8], offset: usize) -> Result<u8, DexError> {
    bytes.get(offset).copied().ok_or(DexError::OutOfBounds {
        offset,
        size: 1,
        len: bytes.len(),
    })
}

fn continuation(bytes: &[u8], offset: usize) -> Result<u8, DexError> {
    let value = byte(bytes, offset)?;
    if value & 0xc0 != 0x80 {
        return Err(DexError::malformed(offset, "invalid MUTF-8 continuation"));
    }
    Ok(value & 0x3f)
}

fn decode_two(bytes: &[u8], offset: usize, first: u8) -> Result<(u16, usize), DexError> {
    let second = continuation(bytes, offset + 1)?;
    let unit = (u16::from(first & 0x1f) << 6) | u16::from(second);
    if unit < 0x80 && unit != 0 {
        return Err(DexError::malformed(offset, "overlong MUTF-8 sequence"));
    }
    Ok((unit, 2))
}

fn decode_three(bytes: &[u8], offset: usize, first: u8) -> Result<(u16, usize), DexError> {
    let second = continuation(bytes, offset + 1)?;
    let third = continuation(bytes, offset + 2)?;
    let unit = (u16::from(first & 0x0f) << 12) | (u16::from(second) << 6) | u16::from(third);
    if unit < 0x800 {
        return Err(DexError::malformed(offset, "overlong MUTF-8 sequence"));
    }
    Ok((unit, 3))
}
