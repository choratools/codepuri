//! Per-instruction error recovery, modeled on jadx's decoder containment.
//!
//! The strict [`super::decode_method`] aborts a whole method on the first
//! failure. This module instead records a diagnostic per failed instruction,
//! replaces it with a gap, and keeps walking — aborting only after the
//! error budget is exhausted, exactly like jadx (first error tolerated,
//! second error aborts the method).

use dex_format::code::CodeItem;

use super::DecodeError;
use super::DecodeReason;
use super::DecodedInsn;
use super::opcode;
use super::operands;
use super::payload;
use super::reader::CodeUnits;

/// Default per-method error budget: two failures abort the walk, same as
/// jadx's decoder containment.
pub const DEFAULT_ERROR_LIMIT: usize = 2;

/// One recovered decode failure, retained as data instead of aborting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeDiagnostic {
    /// Offset of the failing instruction, in 16-bit code units.
    pub offset: u32,
    /// Opcode byte of the failing instruction.
    pub opcode: u8,
    /// Typed failure reason.
    pub reason: DecodeReason,
}

impl DecodeDiagnostic {
    fn from_error(error: &DecodeError) -> Self {
        Self {
            offset: error.offset,
            opcode: error.opcode,
            reason: error.reason.clone(),
        }
    }
}

/// Outcome of a recovering decode of one method.
#[derive(Debug, Clone, Default)]
pub struct RecoveredMethod {
    /// Successfully decoded instructions, in offset order. Failed units are
    /// absent, leaving gaps exactly like jadx's sparse instruction array.
    pub insns: Vec<DecodedInsn>,
    /// Failures recovered during the walk, capped by the error limit.
    pub errors: Vec<DecodeDiagnostic>,
    /// True when the error limit was reached and the walk stopped early.
    pub aborted: bool,
    /// Number of diagnostics that counted toward the error budget. Recovery
    /// semantics follow jadx: register-range and unreferenced-payload noise
    /// is recorded but never aborts the walk, because neither affects Java
    /// semantics.
    pub budget_used: usize,
}

impl RecoveredMethod {
    /// True when the walk recorded no failures at all.
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Decodes with per-instruction recovery: failed units become diagnostics
/// plus a gap, and the walk aborts only after `error_limit` budgeted
/// failures. Register-range violations and payload-ident lookalikes are
/// recorded but never budgeted.
pub fn decode_method_recovering(code_item: &CodeItem, error_limit: usize) -> RecoveredMethod {
    let mut out = RecoveredMethod::default();
    let available = code_item.insns.len() / 2;
    let size_exact = code_item.insns.len().is_multiple_of(2)
        && usize::try_from(code_item.insns_size).ok() == Some(available);
    let mut reader = if size_exact {
        CodeUnits::new(&code_item.insns, code_item.insns_size)
    } else {
        let clamped = code_item
            .insns_size
            .min(u32::try_from(available).unwrap_or(u32::MAX));
        out.errors.push(DecodeDiagnostic {
            offset: 0,
            opcode: 0,
            reason: DecodeReason::Truncated {
                needed: code_item.insns_size,
                remaining: clamped,
            },
        });
        CodeUnits::new(&code_item.insns, clamped)
    };
    while reader.remaining() != 0 {
        if out.budget_used >= error_limit {
            out.aborted = true;
            break;
        }
        let offset = reader.position();
        let first = match reader.unit(offset, offset, 0) {
            Ok(unit) => unit,
            Err(error) => {
                out.errors.push(DecodeDiagnostic::from_error(&error));
                break;
            }
        };
        let opcode_byte = first.to_le_bytes()[0];
        if matches!(
            first,
            payload::PACKED_SWITCH_IDENT | payload::SPARSE_SWITCH_IDENT | payload::ARRAY_DATA_IDENT
        ) {
            match payload::parse(&reader, offset, offset, opcode_byte, None) {
                Ok(parsed) => {
                    if reader.advance(parsed.width, opcode_byte).is_err() {
                        record_truncated(&mut out, offset, opcode_byte);
                        break;
                    }
                    out.insns.push(DecodedInsn {
                        offset,
                        opcode: super::Opcode::Payload {
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
                }
                Err(error) => {
                    record(&mut out, error, false);
                    drop_one(&mut reader, opcode_byte);
                }
            }
            continue;
        }
        let info = match opcode(opcode_byte) {
            super::Opcode::Supported(info) => info,
            super::Opcode::Unsupported { opcode } => {
                record(
                    &mut out,
                    DecodeError::new(offset, opcode, DecodeReason::UnsupportedOpcode { opcode }),
                    true,
                );
                drop_one(&mut reader, opcode_byte);
                continue;
            }
            super::Opcode::Payload { .. } => {
                record(
                    &mut out,
                    malformed(
                        offset,
                        opcode_byte,
                        "opcode table returned a payload marker",
                    ),
                    false,
                );
                drop_one(&mut reader, opcode_byte);
                continue;
            }
        };
        let Some(width) = info.format.width() else {
            record(
                &mut out,
                malformed(offset, opcode_byte, "payload format used by an opcode"),
                false,
            );
            drop_one(&mut reader, opcode_byte);
            continue;
        };
        let width = u32::from(width);
        if reader.advance(width, opcode_byte).is_err() {
            record_truncated(&mut out, offset, opcode_byte);
            break;
        }
        let operands = match operands::decode(&reader, offset, first, opcode_byte, info.format) {
            Ok(decoded) => decoded,
            Err(error) => {
                record(&mut out, error, true);
                continue;
            }
        };
        // jadx parity: register-range violations never abort or drop the
        // instruction — the register file bound is irrelevant to Java
        // semantics, so the insn is kept with its declared indices.
        if let Err(error) =
            check_registers(&operands, code_item.registers_size, offset, opcode_byte)
        {
            record(&mut out, error, false);
        }
        let payload_offset = match check_control_flow(&reader, &operands, offset, opcode_byte) {
            Ok(bound) => bound,
            Err(error) => {
                record(&mut out, error, false);
                None
            }
        };
        out.insns.push(DecodedInsn {
            offset,
            opcode: super::Opcode::Supported(info),
            format: info.format,
            registers: operands.registers,
            literal: operands.literal,
            pool_index: operands.pool_index,
            secondary_pool_index: operands.secondary_pool_index,
            branch_offset: operands.branch_offset,
            payload_offset,
        });
    }
    out
}

fn record(out: &mut RecoveredMethod, error: DecodeError, budgeted: bool) {
    out.errors.push(DecodeDiagnostic::from_error(&error));
    if budgeted {
        out.budget_used += 1;
    }
}

fn record_truncated(out: &mut RecoveredMethod, offset: u32, opcode_byte: u8) {
    record(
        out,
        DecodeError::new(
            offset,
            opcode_byte,
            DecodeReason::MalformedPayload {
                message: "extent past end of instruction stream",
            },
        ),
        false,
    );
}

fn malformed(offset: u32, opcode_byte: u8, message: &'static str) -> DecodeError {
    DecodeError::new(
        offset,
        opcode_byte,
        DecodeReason::MalformedPayload { message },
    )
}

fn drop_one(reader: &mut CodeUnits<'_>, opcode_byte: u8) {
    let _ = reader.advance(1, opcode_byte);
}

fn check_registers(
    operands: &operands::Operands,
    size: u16,
    offset: u32,
    opcode_byte: u8,
) -> Result<(), DecodeError> {
    for &register in &operands.registers {
        if register >= size {
            return Err(DecodeError::new(
                offset,
                opcode_byte,
                DecodeReason::RegisterOutOfRange {
                    register,
                    registers_size: size,
                },
            ));
        }
    }
    Ok(())
}

fn check_control_flow(
    reader: &CodeUnits<'_>,
    operands: &operands::Operands,
    offset: u32,
    opcode_byte: u8,
) -> Result<Option<u32>, DecodeError> {
    let Some(relative) = operands.branch_offset else {
        return Ok(None);
    };
    let target = super::validate_target(offset, i64::from(relative), reader.len(), opcode_byte)?;
    let expected = match opcode_byte {
        0x2b => Some(payload::PACKED_SWITCH_IDENT),
        0x2c => Some(payload::SPARSE_SWITCH_IDENT),
        0x26 => Some(payload::ARRAY_DATA_IDENT),
        _ => None,
    };
    if let Some(ident) = expected {
        let parsed = payload::parse(reader, target, offset, opcode_byte, Some(ident))?;
        payload::validate_switch_targets(reader, &parsed, target, offset, opcode_byte)?;
        return Ok(Some(target));
    }
    Ok(None)
}
