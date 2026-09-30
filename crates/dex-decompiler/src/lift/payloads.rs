//! Resolution of switch and array-data payloads referenced by lifted
//! instructions.
//!
//! Payloads live in the decoded stream as [`Opcode::Payload`] pseudo
//! instructions without operand data, so this module re-reads their bodies
//! from the raw code units and bounds-checks every access.

use std::collections::HashMap;

use dex_format::code::CodeItem;

use crate::decode::{Opcode, RecoveredMethod};

/// Maximum number of folded `fill-array-data` elements; larger payloads
/// degrade the referencing instruction to a conservative opaque.
pub(crate) const MAX_FILL_VALUES: u32 = 4096;

/// Parsed body of one payload pseudo-instruction.
#[derive(Debug, Clone)]
pub(crate) enum PayloadData {
    /// `packed-switch` payload: contiguous keys from `first_key`.
    PackedSwitch {
        /// Key of the first case.
        first_key: i32,
        /// Case targets relative to the referencing switch instruction.
        targets: Vec<i32>,
    },
    /// `sparse-switch` payload: parallel sorted keys and targets.
    SparseSwitch {
        /// Case keys in encoded order.
        keys: Vec<i32>,
        /// Case targets relative to the referencing switch instruction.
        targets: Vec<i32>,
    },
    /// `array-data` payload with raw little-endian element bytes.
    ArrayData {
        /// Element width in bytes (1, 2, 4, or 8).
        element_width: u32,
        /// Raw element bytes.
        data: Vec<u8>,
    },
}

/// Byte-order helpers over the raw instruction array.
struct Units<'a> {
    bytes: &'a [u8],
}

impl Units<'_> {
    fn unit(&self, index: u32) -> Option<u16> {
        let start = usize::try_from(index.checked_mul(2)?).ok()?;
        let end = start.checked_add(2)?;
        let slice = self.bytes.get(start..end)?;
        Some(u16::from_le_bytes([slice[0], slice[1]]))
    }

    fn i32_at(&self, index: u32) -> Option<i32> {
        let low = self.unit(index)?;
        let high = self.unit(index.checked_add(1)?)?;
        Some(i32::from_le_bytes([
            low.to_le_bytes()[0],
            low.to_le_bytes()[1],
            high.to_le_bytes()[0],
            high.to_le_bytes()[1],
        ]))
    }
}

/// Indexes every payload pseudo-instruction in the recovered stream by its
/// code-unit offset.
pub(crate) fn index_payloads(
    code: &CodeItem,
    recovered: &RecoveredMethod,
) -> HashMap<u32, PayloadData> {
    let units = Units { bytes: &code.insns };
    let mut payloads = HashMap::new();
    for insn in &recovered.insns {
        let Opcode::Payload { ident } = insn.opcode else {
            continue;
        };
        if let Some(data) = parse_payload(&units, insn.offset, ident) {
            payloads.insert(insn.offset, data);
        }
    }
    payloads
}

fn parse_payload(units: &Units<'_>, offset: u32, ident: u16) -> Option<PayloadData> {
    match ident {
        0x0100 => {
            let size = u32::from(units.unit(offset.checked_add(1)?)?);
            let first_key = units.i32_at(offset.checked_add(2)?)?;
            let mut targets = Vec::new();
            for index in 0..size {
                targets.push(units.i32_at(offset.checked_add(4)?.checked_add(index * 2)?)?);
            }
            Some(PayloadData::PackedSwitch { first_key, targets })
        }
        0x0200 => {
            let size = u32::from(units.unit(offset.checked_add(1)?)?);
            let mut keys = Vec::new();
            let mut targets = Vec::new();
            for index in 0..size {
                keys.push(units.i32_at(offset.checked_add(2)?.checked_add(index * 2)?)?);
            }
            let target_base = offset.checked_add(2)?.checked_add(size.checked_mul(2)?)?;
            for index in 0..size {
                targets.push(units.i32_at(target_base.checked_add(index * 2)?)?);
            }
            (keys.len() == targets.len()).then_some(PayloadData::SparseSwitch { keys, targets })
        }
        0x0300 => {
            let element_width = u32::from(units.unit(offset.checked_add(1)?)?);
            let size = units.i32_at(offset.checked_add(2)?)?;
            let size = u32::try_from(size).ok()?;
            let byte_len = element_width.checked_mul(size)?;
            let byte_start = usize::try_from(offset.checked_add(4)?.checked_mul(2)?).ok()?;
            let byte_end = byte_start.checked_add(usize::try_from(byte_len).ok()?)?;
            (byte_end <= units.bytes.len()).then(|| PayloadData::ArrayData {
                element_width,
                data: units.bytes[byte_start..byte_end].to_vec(),
            })
        }
        _ => None,
    }
}
