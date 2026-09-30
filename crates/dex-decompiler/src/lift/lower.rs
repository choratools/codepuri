//! Stream walking: fusion, payload folding, and per-instruction lowering.

use std::collections::HashMap;

use dex_format::code::CodeItem;
use managed_ir::insn::ManagedInsn;
use managed_ir::ops::Constant;
use managed_ir::types::ManagedType;
use managed_ir::value::ValueId;

use super::cfg_build::assemble;
use super::lowering::LiftedInsn;
use super::payloads::{self, MAX_FILL_VALUES, PayloadData};
use super::values::ValueTable;
use super::{LiftError, LiftedMethod};
use crate::decode::{DecodedInsn, Opcode, RecoveredMethod};

/// Lifts a recovered instruction stream into managed IR with a CFG.
///
/// Normalization performed while walking:
/// - `invoke-*` / `filled-new-array` fuse a directly following
///   `move-result[-wide|object]` into their result binding;
/// - `packed-switch` / `sparse-switch` payloads are resolved at edge
///   allocation time and `fill-array-data` payloads are folded into
///   [`ManagedInsn::FillArray`](managed_ir::insn::ManagedInsn) constants;
/// - register pairs `(vN, vN+1)` share one atomic category-2 value.
pub(crate) fn lift_stream(
    recovered: &RecoveredMethod,
    code: &CodeItem,
    ctx: &dyn crate::infer::TypeContext,
) -> Result<LiftedMethod, LiftError> {
    let payload_map = payloads::index_payloads(code, recovered);
    let exceptions = exception_edges(code);
    let try_ranges = code
        .tries
        .iter()
        .map(|item| {
            (
                item.start_addr,
                item.start_addr.saturating_add(u32::from(item.insn_count)),
            )
        })
        .collect();
    let mut values = ValueTable::new();
    let mut handler_values: HashMap<u32, ValueId> = HashMap::new();
    let mut lifted: Vec<LiftedInsn> = Vec::new();
    let stream = &recovered.insns;
    let mut index = 0;
    while index < stream.len() {
        let insn = &stream[index];
        index += 1;
        let Opcode::Supported(info) = insn.opcode else {
            continue;
        };
        if info.opcode == 0x00 {
            continue;
        }
        let fused = fusion_result(stream, index, insn, info.opcode);
        if fused.is_some() {
            index += 1;
        }
        let fill_values = if info.opcode == 0x26 {
            fill_elements(&mut values, &payload_map, insn.payload_offset)
        } else {
            None
        };
        let handler_input = if info.opcode == 0x0d {
            Some(handler_value(&mut values, &mut handler_values, insn.offset))
        } else {
            None
        };
        let id = values.reserve();
        let mut invoke_args = None;
        if matches!(info.opcode, 0x6e..=0x72 | 0x74..=0x78) {
            let mut params = Vec::new();
            if ctx.method_params(insn.pool_index.unwrap_or_default(), &mut params) {
                let mut args = Vec::new();
                let mut word = 0;
                if !matches!(info.opcode, 0x71 | 0x77) {
                    if let Some(reg) = insn.registers.first() {
                        args.push(values.read_typed(*reg, false));
                    }
                    word = 1;
                }
                for ty in params {
                    let wide = ty.category() == Some(managed_ir::types::ValueCategory::Two);
                    if let Some(reg) = insn.registers.get(word) {
                        let value = values.read_typed(*reg, wide);
                        args.push(value);
                        if wide && insn.registers.get(word + 1) == reg.checked_add(1).as_ref() {
                            args.push(value);
                        }
                    }
                    word += if wide { 2 } else { 1 };
                }
                if args.len() == insn.registers.len() {
                    invoke_args = Some(args);
                }
            }
        }
        let mut lowered = super::insn_map::build(
            &mut values,
            id,
            super::insn_map::Context {
                insn,
                fused,
                handler_input,
                fill_values,
            },
        );
        if let Some(args) = invoke_args
            && let super::insn_map::Built::Managed(ManagedInsn::Invoke { args: actual, .. }) =
                &mut lowered
        {
            *actual = args;
        }
        let mut entry = LiftedInsn::new(insn.offset, info.opcode, id);
        match lowered {
            super::insn_map::Built::Managed(managed) => {
                // Returns and throws terminate the block: without an
                // explicit terminal marker the CFG builder would wire a
                // bogus fallthrough edge into unreachable code.
                let terminal = matches!(
                    managed,
                    ManagedInsn::Return { .. } | ManagedInsn::Throw { .. }
                );
                values.insns[id] = managed;
                if terminal {
                    entry.pending = Some(super::lowering::PendingFlow::Terminal);
                }
            }
            super::insn_map::Built::Pending(pending) => entry.pending = Some(pending),
        }
        lifted.push(entry);
    }
    assemble(values, lifted, payload_map, exceptions, try_ranges)
}

/// Returns the fused `move-result` destination and width when the
/// instruction directly following `insn` is a compatible result move at the
/// adjacent offset.
fn fusion_result(
    stream: &[DecodedInsn],
    next: usize,
    insn: &DecodedInsn,
    byte: u8,
) -> Option<super::lowering::FusedResult> {
    let following = stream.get(next)?;
    let width = u32::from(insn.format.width()?);
    if following.offset != insn.offset.checked_add(width)? {
        return None;
    }
    let Opcode::Supported(info) = following.opcode else {
        return None;
    };
    let compatible = match byte {
        0x24 | 0x25 => info.opcode == 0x0c,
        _ => matches!(info.opcode, 0x0a..=0x0c),
    };
    compatible.then(|| super::lowering::FusedResult {
        dest: following.registers.first().copied().unwrap_or(0),
        wide: info.opcode == 0x0b,
    })
}

/// Returns the synthetic exception value for a handler entry address.
fn handler_value(
    values: &mut ValueTable,
    handler_values: &mut HashMap<u32, ValueId>,
    address: u32,
) -> ValueId {
    match handler_values.get(&address) {
        Some(&value) => value,
        None => {
            let value = values.handler_value();
            handler_values.insert(address, value);
            value
        }
    }
}

/// Folds an `array-data` payload into constant values, or `None` when the
/// payload is missing, not array data, or beyond the fold budget.
fn fill_elements(
    values: &mut ValueTable,
    payload_map: &HashMap<u32, PayloadData>,
    payload_offset: Option<u32>,
) -> Option<Vec<ValueId>> {
    let PayloadData::ArrayData {
        element_width,
        data,
    } = payload_map.get(&payload_offset?)?
    else {
        return None;
    };
    let width = usize::try_from(*element_width).ok()?;
    if width == 0 || data.len() % width != 0 {
        return None;
    }
    let count = data.len() / width;
    if u64::try_from(count).ok()? > u64::from(MAX_FILL_VALUES) {
        return None;
    }
    Some(
        (0..count)
            .map(|index| {
                values.const_value(
                    element_constant(&data[index * width..index * width + width]),
                    ManagedType::Unknown,
                )
            })
            .collect(),
    )
}

fn element_constant(bytes: &[u8]) -> Constant {
    match bytes.len() {
        1 => Constant::Int(i32::from(bytes[0])),
        2 => Constant::Int(i32::from(u16::from_le_bytes([bytes[0], bytes[1]]))),
        4 => Constant::Int(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        _ => Constant::Long(i64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])),
    }
}

/// Expands try ranges into per-instruction exceptional edges.
///
/// Returns `(source offset, handler address, exception name)` triples; typed
/// handlers are named by their type index and catch-all handlers by the
/// empty string.
fn exception_edges(code: &CodeItem) -> Vec<(u32, u32, String)> {
    let mut edges = Vec::new();
    for try_item in &code.tries {
        let Some(handler) = code
            .handlers
            .iter()
            .find(|item| item.relative_offset == u32::from(try_item.handler_off))
        else {
            continue;
        };
        let end = try_item
            .start_addr
            .saturating_add(u32::from(try_item.insn_count));
        let mut offset = try_item.start_addr;
        while offset < end {
            for caught in &handler.handlers {
                edges.push((offset, caught.address, caught.type_idx.to_string()));
            }
            if let Some(catch_all) = handler.catch_all_addr {
                edges.push((offset, catch_all, String::new()));
            }
            offset += 1;
        }
    }
    edges
}
