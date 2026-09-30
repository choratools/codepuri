//! Panic-free Dalvik instruction decoding.

mod error;
mod insn;
pub mod opcodes;
mod operands;
mod payload;
mod reader;
mod recover;

pub use error::{DecodeError, DecodeReason};
pub use insn::DecodedInsn;
pub use opcodes::{Format, Opcode, OpcodeInfo, SemanticClass, opcode};
pub use recover::{
    DEFAULT_ERROR_LIMIT, DecodeDiagnostic, RecoveredMethod, decode_method_recovering,
};

use dex_format::code::CodeItem;
use operands::Operands;
use reader::CodeUnits;

/// Decodes and validates every instruction in a DEX code item.
///
/// # Errors
/// Returns a [`DecodeError`] for malformed operands, truncation, reserved
/// opcodes, invalid registers, branches, or payload references.
pub fn decode_method(code_item: &CodeItem) -> Result<Vec<DecodedInsn>, DecodeError> {
    validate_raw_size(code_item)?;
    let mut reader = CodeUnits::new(&code_item.insns, code_item.insns_size);
    let mut decoded = Vec::new();
    while reader.remaining() != 0 {
        let offset = reader.position();
        let first = reader.unit(offset, offset, 0)?;
        let opcode_byte = first.to_le_bytes()[0];
        if matches!(
            first,
            payload::PACKED_SWITCH_IDENT | payload::SPARSE_SWITCH_IDENT | payload::ARRAY_DATA_IDENT
        ) {
            let parsed = payload::parse(&reader, offset, offset, opcode_byte, None)?;
            reader.advance(parsed.width, opcode_byte)?;
            decoded.push(DecodedInsn {
                offset,
                opcode: Opcode::Payload {
                    ident: parsed.ident,
                },
                format: parsed.format,
                registers: Vec::new(),
                literal: None,
                pool_index: None,
                secondary_pool_index: None,
                branch_offset: None,
                payload_offset: None,
            });
            continue;
        }
        let info = match opcode(opcode_byte) {
            Opcode::Supported(info) => info,
            Opcode::Unsupported { opcode } => {
                return Err(DecodeError::new(
                    offset,
                    opcode,
                    DecodeReason::UnsupportedOpcode { opcode },
                ));
            }
            Opcode::Payload { ident: _ } => unreachable_payload_variant(offset, opcode_byte)?,
        };
        let width = u32::from(info.format.width().ok_or_else(|| {
            DecodeError::new(
                offset,
                opcode_byte,
                DecodeReason::MalformedPayload {
                    message: "payload format used by an opcode",
                },
            )
        })?);
        reader.advance(width, opcode_byte)?;
        let operands = operands::decode(&reader, offset, first, opcode_byte, info.format)?;
        validate_registers(&operands, code_item.registers_size, offset, opcode_byte)?;
        let payload_offset = validate_control_flow(&reader, &operands, offset, opcode_byte)?;
        decoded.push(DecodedInsn {
            offset,
            opcode: Opcode::Supported(info),
            format: info.format,
            registers: operands.registers,
            literal: operands.literal,
            pool_index: operands.pool_index,
            secondary_pool_index: operands.secondary_pool_index,
            branch_offset: operands.branch_offset,
            payload_offset,
        });
    }
    Ok(decoded)
}

fn validate_raw_size(code_item: &CodeItem) -> Result<(), DecodeError> {
    let actual_units = code_item.insns.len() / 2;
    let exact = code_item.insns.len().is_multiple_of(2)
        && usize::try_from(code_item.insns_size).ok() == Some(actual_units);
    if exact {
        return Ok(());
    }
    let remaining = u32::try_from(actual_units).unwrap_or(u32::MAX);
    Err(DecodeError::new(
        0,
        0,
        DecodeReason::Truncated {
            needed: code_item.insns_size,
            remaining,
        },
    ))
}

fn validate_registers(
    operands: &Operands,
    size: u16,
    offset: u32,
    opcode: u8,
) -> Result<(), DecodeError> {
    for &register in &operands.registers {
        if register >= size {
            return Err(DecodeError::new(
                offset,
                opcode,
                DecodeReason::RegisterOutOfRange {
                    register,
                    registers_size: size,
                },
            ));
        }
    }
    Ok(())
}

fn validate_control_flow(
    reader: &CodeUnits<'_>,
    operands: &Operands,
    offset: u32,
    opcode: u8,
) -> Result<Option<u32>, DecodeError> {
    let Some(relative) = operands.branch_offset else {
        return Ok(None);
    };
    let target = validate_target(offset, i64::from(relative), reader.len(), opcode)?;
    let expected = match opcode {
        0x2b => Some(payload::PACKED_SWITCH_IDENT),
        0x2c => Some(payload::SPARSE_SWITCH_IDENT),
        0x26 => Some(payload::ARRAY_DATA_IDENT),
        _ => None,
    };
    if let Some(ident) = expected {
        let parsed = payload::parse(reader, target, offset, opcode, Some(ident))?;
        payload::validate_switch_targets(reader, &parsed, target, offset, opcode)?;
        Ok(Some(target))
    } else {
        Ok(None)
    }
}

pub(crate) fn validate_target(
    offset: u32,
    relative: i64,
    len: u32,
    opcode: u8,
) -> Result<u32, DecodeError> {
    let target = i64::from(offset) + relative;
    if target < 0 || target >= i64::from(len) {
        return Err(DecodeError::new(
            offset,
            opcode,
            DecodeReason::BranchTargetOutOfRange {
                target,
                insns_size: len,
            },
        ));
    }
    u32::try_from(target).map_err(|_| {
        DecodeError::new(
            offset,
            opcode,
            DecodeReason::BranchTargetOutOfRange {
                target,
                insns_size: len,
            },
        )
    })
}

fn unreachable_payload_variant(offset: u32, opcode: u8) -> Result<OpcodeInfo, DecodeError> {
    Err(DecodeError::new(
        offset,
        opcode,
        DecodeReason::MalformedPayload {
            message: "opcode table returned a payload marker",
        },
    ))
}
