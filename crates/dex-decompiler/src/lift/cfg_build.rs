//! CFG construction over lifted instructions.
//!
//! Every lifted instruction owns exactly one CFG block whose payload is its
//! instruction identity. Normal edges follow the stream: fallthrough after
//! plain instructions, taken/fallthrough after conditionals, switch case
//! edges plus a default fallthrough, nothing after `goto`/`return`/`throw`.
//! Exceptional edges connect potentially throwing guarded instructions to
//! handlers before normal-only synthetic register commits.
//! Payloads never become blocks; unresolvable transfer targets degrade the
//! instruction to a conservative opaque.

use std::collections::{HashMap, HashSet};

use decomp_core::cfg::{Cfg, EdgeKind};
use decomp_core::ids::{BlockId, EdgeId};
use managed_ir::effects::EffectFlags;
use managed_ir::insn::ManagedInsn;
use managed_ir::method::{MethodIr, RegisterForm};
use managed_ir::value::{InsnId, Value, ValueId};

use super::lowering::{LiftedInsn, PendingFlow};
use super::payloads::PayloadData;
use super::values::ValueTable;
use super::{LiftError, LiftedMethod};

/// Builds the CFG, allocates edges, finalizes pending transfers, and
/// validates the final register-form method IR.
pub(crate) fn assemble(
    mut values: ValueTable,
    lifted: Vec<LiftedInsn>,
    payload_map: HashMap<u32, PayloadData>,
    exceptions: Vec<(u32, u32, String)>,
    try_ranges: Vec<(u32, u32)>,
) -> Result<LiftedMethod, LiftError> {
    let mut lifted = lifted;
    if lifted.is_empty() {
        // Abstract-style bodies and all-payload streams still need an entry.
        let id = values.reserve();
        values.insns[id] = ManagedInsn::Return { value: None };
        let mut entry = LiftedInsn::new(0, 0x0e, id);
        entry.pending = Some(PendingFlow::Terminal);
        lifted.push(entry);
    }

    let mut offsets: HashMap<InsnId, u32> = HashMap::with_capacity(lifted.len());
    let mut by_offset: HashMap<u32, BlockId> = HashMap::with_capacity(lifted.len());
    let mut block_ids: Vec<BlockId> = Vec::with_capacity(lifted.len());
    let mut cfg = Cfg::new(lifted[0].insn);
    offsets.insert(lifted[0].insn, lifted[0].offset);
    by_offset.insert(lifted[0].offset, cfg.entry());
    block_ids.push(cfg.entry());
    for entry in &lifted[1..] {
        let block = cfg.add_block(entry.insn);
        offsets.insert(entry.insn, entry.offset);
        by_offset.insert(entry.offset, block);
        block_ids.push(block);
    }
    let throwing: HashSet<_> = lifted
        .iter()
        .filter(|entry| !exceptions.is_empty() && may_throw(entry.opcode))
        .map(|entry| entry.offset)
        .collect();
    let guarded: HashSet<_> = exceptions.iter().map(|(offset, _, _)| *offset).collect();
    for (index, entry) in lifted.iter().enumerate() {
        let mut source = block_ids[index];
        if throwing.contains(&entry.offset) && guarded.contains(&entry.offset) {
            // Exception successors observe the register file before the
            // throwing instruction commits its destination. Ordinary SSA
            // edges observe definitions at their source, so separate the
            // unbound operation result from the normal-only register write.
            let result = match &values.insns[entry.insn] {
                ManagedInsn::Const { result, .. }
                | ManagedInsn::FieldGet { result, .. }
                | ManagedInsn::ArrayGet { result, .. }
                | ManagedInsn::ArrayLength { result, .. }
                | ManagedInsn::NewInstance { result, .. }
                | ManagedInsn::NewArray { result, .. }
                | ManagedInsn::Cast { result, .. }
                | ManagedInsn::InstanceOf { result, .. }
                | ManagedInsn::Arithmetic { result, .. } => Some(*result),
                ManagedInsn::Invoke { result, .. } => *result,
                ManagedInsn::Opaque { writes, .. } if writes.len() == 1 => Some(writes[0]),
                _ => None,
            };
            if let Some(original) = result {
                let commit = values.reserve();
                let ty = values.values[original].ty().clone();
                let temporary = values.values.insert(Value::new(entry.insn, ty.clone()));
                values.values[original] = Value::new(commit, ty);
                match &mut values.insns[entry.insn] {
                    ManagedInsn::Const { result, .. }
                    | ManagedInsn::FieldGet { result, .. }
                    | ManagedInsn::ArrayGet { result, .. }
                    | ManagedInsn::ArrayLength { result, .. }
                    | ManagedInsn::NewInstance { result, .. }
                    | ManagedInsn::NewArray { result, .. }
                    | ManagedInsn::Cast { result, .. }
                    | ManagedInsn::InstanceOf { result, .. }
                    | ManagedInsn::Arithmetic { result, .. } => *result = temporary,
                    ManagedInsn::Invoke { result, .. } => *result = Some(temporary),
                    ManagedInsn::Opaque { writes, .. } => writes[0] = temporary,
                    _ => unreachable!(),
                }
                values.insns[commit] = ManagedInsn::Move {
                    result: original,
                    value: temporary,
                };
                let normal = cfg.add_block(commit);
                cfg.add_edge(source, normal, EdgeKind::Unconditional)?;
                source = normal;
            }
        }
        let fallthrough = block_ids.get(index + 1).copied();
        match &entry.pending {
            Some(PendingFlow::Terminal) => {}
            Some(PendingFlow::Goto(target)) => match by_offset.get(target) {
                Some(&to) => {
                    let edge = cfg.add_edge(source, to, EdgeKind::Unconditional)?;
                    values.insns[entry.insn] = ManagedInsn::Branch {
                        condition: None,
                        then_edge: edge,
                        else_edge: None,
                    };
                }
                None => degrade(&mut values, entry, Vec::new()),
            },
            Some(PendingFlow::Branch { condition, target }) => {
                match by_offset.get(target).copied().zip(fallthrough) {
                    Some((taken, not_taken)) => {
                        let true_edge = cfg.add_edge(source, taken, EdgeKind::BranchTrue)?;
                        let false_edge = cfg.add_edge(source, not_taken, EdgeKind::BranchFalse)?;
                        values.insns[entry.insn] = ManagedInsn::Branch {
                            condition: Some(*condition),
                            then_edge: true_edge,
                            else_edge: Some(false_edge),
                        };
                    }
                    None => degrade(&mut values, entry, vec![*condition]),
                }
            }
            Some(PendingFlow::Switch { key, payload }) => {
                wire_switch(
                    &mut cfg,
                    &mut values,
                    &by_offset,
                    entry,
                    *key,
                    payload.as_ref().and_then(|offset| payload_map.get(offset)),
                    fallthrough,
                )?;
            }
            None => {
                if let Some(next_block) = fallthrough {
                    cfg.add_edge(source, next_block, EdgeKind::Unconditional)?;
                }
            }
        }
    }
    for (source_offset, target, name) in &exceptions {
        if !throwing.contains(source_offset) {
            continue;
        }
        if let (Some(&from), Some(&to)) = (by_offset.get(source_offset), by_offset.get(target)) {
            cfg.add_edge(from, to, EdgeKind::Exception(name.clone()))?;
        }
    }
    cfg.validate()?;
    let width_claims = values.take_width_claims();
    Ok(LiftedMethod {
        ir: MethodIr::<RegisterForm>::from_parts(
            cfg,
            values.insns,
            values.values,
            values.reg_slots,
        ),
        offsets,
        try_ranges,
        width_claims,
    })
}

/// Only suppress edges for opcode families known not to throw. Unknown and
/// resolving/allocating/accessing operations remain conservative.
/// Integer division/remainder (including 2addr and literal forms) are omitted
/// from the non-throwing arithmetic ranges; floating-point division is safe.
fn may_throw(opcode: u8) -> bool {
    !matches!(opcode,
        0x00..=0x19 // nop, moves, returns, raw numeric constants
        | 0x28..=0x3d // branches, switches, ordering comparisons
        | 0x7b..=0x92 | 0x95..=0x9d | 0xa0..=0xb2
        | 0xb5..=0xbd | 0xc0..=0xd2 | 0xd5..=0xda | 0xdd..=0xe2
    )
}

/// Degrades a switch with no usable successors.
fn degrade_ok(values: &mut ValueTable, entry: &LiftedInsn, key: ValueId) -> Result<(), LiftError> {
    degrade(values, entry, vec![key]);
    Ok(())
}

/// Replaces an unresolvable transfer with a conservative opaque.
fn degrade(values: &mut ValueTable, entry: &LiftedInsn, reads: Vec<ValueId>) {
    values.insns[entry.insn] = ManagedInsn::Opaque {
        family_tag: u16::from(entry.opcode),
        reads,
        writes: Vec::new(),
        effects: EffectFlags::TERMINATES | EffectFlags::MAY_THROW,
    };
}

/// Wires one switch: case edges per payload entry plus the default
/// fallthrough; any unresolvable case degrades the whole instruction.
fn wire_switch(
    cfg: &mut Cfg<InsnId>,
    values: &mut ValueTable,
    by_offset: &HashMap<u32, BlockId>,
    entry: &LiftedInsn,
    key: ValueId,
    data: Option<&PayloadData>,
    fallthrough: Option<BlockId>,
) -> Result<(), LiftError> {
    // The source block always exists: blocks are created for every lifted
    // instruction before edge wiring starts.
    let Some(&source) = by_offset.get(&entry.offset) else {
        return Ok(());
    };
    let cases: Vec<(i64, i32)> = match data {
        Some(PayloadData::PackedSwitch { first_key, targets }) => targets
            .iter()
            .enumerate()
            .map(|(index, &relative)| (i64::from(first_key.wrapping_add(index as i32)), relative))
            .collect(),
        Some(PayloadData::SparseSwitch { keys, targets }) => keys
            .iter()
            .copied()
            .zip(targets.iter().copied())
            .map(|(key, relative)| (i64::from(key), relative))
            .collect(),
        _ => Vec::new(),
    };
    if cases.is_empty() {
        return match fallthrough {
            Some(default_block) => {
                let default_edge = cfg.add_edge(source, default_block, EdgeKind::Unconditional)?;
                values.insns[entry.insn] = ManagedInsn::Switch {
                    key,
                    default_edge,
                    cases: Vec::new(),
                };
                Ok(())
            }
            None => degrade_ok(values, entry, key),
        };
    }
    let mut resolved: Vec<(i64, BlockId)> = Vec::with_capacity(cases.len());
    let mut degraded = false;
    for (value, relative) in &cases {
        let target = u32::try_from(i64::from(entry.offset) + i64::from(*relative)).ok();
        match target.and_then(|offset| by_offset.get(&offset)) {
            Some(&block) => resolved.push((*value, block)),
            None => {
                degraded = true;
                break;
            }
        }
    }
    if degraded {
        return degrade_ok(values, entry, key);
    }
    let Some(default_block) = fallthrough else {
        return degrade_ok(values, entry, key);
    };
    let mut managed_cases: Vec<(i64, EdgeId)> = Vec::with_capacity(resolved.len());
    for (value, block) in resolved {
        let edge = cfg.add_edge(source, block, EdgeKind::Switch(value as u64))?;
        managed_cases.push((value, edge));
    }
    let default_edge = cfg.add_edge(source, default_block, EdgeKind::Unconditional)?;
    values.insns[entry.insn] = ManagedInsn::Switch {
        key,
        default_edge,
        cases: managed_cases,
    };
    Ok(())
}
