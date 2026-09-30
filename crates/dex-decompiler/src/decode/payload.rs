//! Dalvik switch and array-data payload parsing.

use super::reader::CodeUnits;
use super::{DecodeError, DecodeReason, Format};

pub(crate) const PACKED_SWITCH_IDENT: u16 = 0x0100;
pub(crate) const SPARSE_SWITCH_IDENT: u16 = 0x0200;
pub(crate) const ARRAY_DATA_IDENT: u16 = 0x0300;

pub(crate) struct ParsedPayload {
    pub(crate) ident: u16,
    pub(crate) format: Format,
    pub(crate) width: u32,
    pub(crate) size: u32,
}

pub(crate) fn parse(
    reader: &CodeUnits<'_>,
    offset: u32,
    insn: u32,
    opcode: u8,
    expected: Option<u16>,
) -> Result<ParsedPayload, DecodeError> {
    if !offset.is_multiple_of(2) {
        return Err(DecodeError::new(
            insn,
            opcode,
            DecodeReason::MalformedPayload {
                message: "payload is not 32-bit aligned",
            },
        ));
    }
    let ident = reader.unit(offset, insn, opcode)?;
    if let Some(required) = expected
        && ident != required
    {
        return Err(DecodeError::new(
            insn,
            opcode,
            DecodeReason::BadPayloadIdent {
                expected: required,
                found: ident,
            },
        ));
    }
    let (format, size, width) = match ident {
        PACKED_SWITCH_IDENT => {
            let size = u32::from(reader.unit(add(offset, 1, insn, opcode)?, insn, opcode)?);
            (
                Format::PackedSwitchPayload,
                size,
                checked_width(4, size, 2, insn, opcode)?,
            )
        }
        SPARSE_SWITCH_IDENT => {
            let size = u32::from(reader.unit(add(offset, 1, insn, opcode)?, insn, opcode)?);
            (
                Format::SparseSwitchPayload,
                size,
                checked_width(2, size, 4, insn, opcode)?,
            )
        }
        ARRAY_DATA_IDENT => {
            let element_width =
                u32::from(reader.unit(add(offset, 1, insn, opcode)?, insn, opcode)?);
            if !matches!(element_width, 1 | 2 | 4 | 8) {
                return Err(DecodeError::new(
                    insn,
                    opcode,
                    DecodeReason::MalformedPayload {
                        message: "array element width is not 1, 2, 4, or 8",
                    },
                ));
            }
            let size = read_u32(reader, add(offset, 2, insn, opcode)?, insn, opcode)?;
            let bytes = element_width.checked_mul(size).ok_or_else(|| {
                DecodeError::new(
                    insn,
                    opcode,
                    DecodeReason::MalformedPayload {
                        message: "array payload byte size overflows",
                    },
                )
            })?;
            let data_units = bytes.checked_add(1).map(|value| value / 2).ok_or_else(|| {
                DecodeError::new(
                    insn,
                    opcode,
                    DecodeReason::MalformedPayload {
                        message: "array payload width overflows",
                    },
                )
            })?;
            (
                Format::ArrayDataPayload,
                size,
                4_u32.checked_add(data_units).ok_or_else(|| {
                    DecodeError::new(
                        insn,
                        opcode,
                        DecodeReason::MalformedPayload {
                            message: "array payload width overflows",
                        },
                    )
                })?,
            )
        }
        found => {
            return Err(DecodeError::new(
                insn,
                opcode,
                DecodeReason::BadPayloadIdent {
                    expected: expected.unwrap_or(PACKED_SWITCH_IDENT),
                    found,
                },
            ));
        }
    };
    let end = offset.checked_add(width).filter(|end| *end <= reader.len());
    if end.is_none() {
        return Err(DecodeError::new(
            insn,
            opcode,
            DecodeReason::Truncated {
                needed: width,
                remaining: reader.len().saturating_sub(offset),
            },
        ));
    }
    Ok(ParsedPayload {
        ident,
        format,
        width,
        size,
    })
}

pub(crate) fn validate_switch_targets(
    reader: &CodeUnits<'_>,
    payload: &ParsedPayload,
    payload_offset: u32,
    switch_offset: u32,
    opcode: u8,
) -> Result<(), DecodeError> {
    let target_start = match payload.ident {
        PACKED_SWITCH_IDENT => add(payload_offset, 4, switch_offset, opcode)?,
        SPARSE_SWITCH_IDENT => add(payload_offset, 2 + payload.size * 2, switch_offset, opcode)?,
        _ => return Ok(()),
    };
    for index in 0..payload.size {
        let relative = i64::from(read_i32(
            reader,
            add(target_start, index * 2, switch_offset, opcode)?,
            switch_offset,
            opcode,
        )?);
        super::validate_target(switch_offset, relative, reader.len(), opcode)?;
    }
    Ok(())
}

pub(crate) fn read_i32(
    reader: &CodeUnits<'_>,
    offset: u32,
    insn: u32,
    opcode: u8,
) -> Result<i32, DecodeError> {
    Ok(i32::from_le_bytes(
        read_u32(reader, offset, insn, opcode)?.to_le_bytes(),
    ))
}

pub(crate) fn read_u32(
    reader: &CodeUnits<'_>,
    offset: u32,
    insn: u32,
    opcode: u8,
) -> Result<u32, DecodeError> {
    let low = u32::from(reader.unit(offset, insn, opcode)?);
    let high = u32::from(reader.unit(add(offset, 1, insn, opcode)?, insn, opcode)?);
    Ok(low | high << 16)
}

fn checked_width(
    base: u32,
    size: u32,
    stride: u32,
    insn: u32,
    opcode: u8,
) -> Result<u32, DecodeError> {
    size.checked_mul(stride)
        .and_then(|body| base.checked_add(body))
        .ok_or_else(|| {
            DecodeError::new(
                insn,
                opcode,
                DecodeReason::MalformedPayload {
                    message: "payload width overflows",
                },
            )
        })
}

fn add(offset: u32, delta: u32, insn: u32, opcode: u8) -> Result<u32, DecodeError> {
    offset.checked_add(delta).ok_or_else(|| {
        DecodeError::new(
            insn,
            opcode,
            DecodeReason::MalformedPayload {
                message: "payload offset overflows",
            },
        )
    })
}
