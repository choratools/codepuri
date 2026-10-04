//! Brute-force validation of published SSA shape against independent
//! reaching-definition and dominance oracles.
//!
//! Included as a module by `ssa_properties.rs`; walked standalone this
//! file compiles to nothing, hence the dead-code allowance. Uses its own
//! tiny def/use visitors (managed-ir's walkers are private) and avoids
//! importing the verifier's own analyses, so a bug in the construction
//! cannot hide behind a matching bug in its mirror.
//!
//! Checks:
//! 1. cluster shape: consecutive phis select over the same head incoming
//!    edges, with no branching or additional entry inside the cluster;
//! 2. phi inputs actually reach along their own incoming edge (may-reach
//!    dataflow, with the phi result killed and re-defined at its block);
//! 3. every use is dominated by its definition — instruction uses by a
//!    graph BFS that deletes the def block, phi inputs by dominance of the
//!    incoming edge's source.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use decomp_core::cfg::Cfg;
use decomp_core::ids::{BlockId, EdgeId};
use managed_ir::insn::ManagedInsn;
use managed_ir::value::{InsnId, ValueId};

use dex_decompiler::SsaMethod;

/// Visits every value defined by the instruction.
fn for_each_definition(insn: &ManagedInsn, mut visit: impl FnMut(ValueId)) {
    match insn {
        ManagedInsn::Const { result, .. }
        | ManagedInsn::Move { result, .. }
        | ManagedInsn::Phi { result, .. }
        | ManagedInsn::Neg { result, .. }
        | ManagedInsn::Not { result, .. }
        | ManagedInsn::FieldGet { result, .. }
        | ManagedInsn::ArrayGet { result, .. }
        | ManagedInsn::ArrayLength { result, .. }
        | ManagedInsn::NewInstance { result, .. }
        | ManagedInsn::NewArray { result, .. }
        | ManagedInsn::Cast { result, .. }
        | ManagedInsn::InstanceOf { result, .. }
        | ManagedInsn::Arithmetic { result, .. }
        | ManagedInsn::Compare { result, .. }
        | ManagedInsn::OrderingCompare { result, .. } => visit(*result),
        ManagedInsn::Invoke { result, .. } | ManagedInsn::NewFilledArray { result, .. } => {
            if let Some(value) = result {
                visit(*value);
            }
        }
        ManagedInsn::Opaque { writes, .. } => {
            for value in writes {
                visit(*value);
            }
        }
        ManagedInsn::FieldPut { .. }
        | ManagedInsn::ArrayPut { .. }
        | ManagedInsn::FillArray { .. }
        | ManagedInsn::MonitorEnter { .. }
        | ManagedInsn::MonitorExit { .. }
        | ManagedInsn::Branch { .. }
        | ManagedInsn::Switch { .. }
        | ManagedInsn::Return { .. }
        | ManagedInsn::Throw { .. } => {}
    }
}

/// Visits every value used by the instruction.
fn for_each_use(insn: &ManagedInsn, mut visit: impl FnMut(ValueId)) {
    match insn {
        ManagedInsn::Const { .. } | ManagedInsn::NewInstance { .. } => {}
        ManagedInsn::Move { value, .. }
        | ManagedInsn::Neg { value, .. }
        | ManagedInsn::Not { value, .. }
        | ManagedInsn::Cast { value, .. }
        | ManagedInsn::InstanceOf { value, .. }
        | ManagedInsn::Throw { value } => visit(*value),
        ManagedInsn::Phi { inputs, .. } => {
            for (_, value) in inputs {
                visit(*value);
            }
        }
        ManagedInsn::Arithmetic { lhs, rhs, .. }
        | ManagedInsn::Compare { lhs, rhs, .. }
        | ManagedInsn::OrderingCompare { lhs, rhs, .. } => {
            visit(*lhs);
            visit(*rhs);
        }
        ManagedInsn::FieldGet { object, .. } => {
            if let Some(value) = object {
                visit(*value);
            }
        }
        ManagedInsn::FieldPut { object, value, .. } => {
            if let Some(value) = object {
                visit(*value);
            }
            visit(*value);
        }
        ManagedInsn::ArrayGet { array, index, .. } => {
            visit(*array);
            visit(*index);
        }
        ManagedInsn::ArrayPut {
            array,
            index,
            value,
        } => {
            visit(*array);
            visit(*index);
            visit(*value);
        }
        ManagedInsn::ArrayLength { array, .. }
        | ManagedInsn::NewArray { length: array, .. }
        | ManagedInsn::MonitorEnter { object: array }
        | ManagedInsn::MonitorExit { object: array }
        | ManagedInsn::Switch { key: array, .. } => visit(*array),
        ManagedInsn::FillArray { array, values } => {
            visit(*array);
            for value in values {
                visit(*value);
            }
        }
        ManagedInsn::NewFilledArray { elements: args, .. }
        | ManagedInsn::Invoke { args, .. }
        | ManagedInsn::Opaque { reads: args, .. } => {
            for value in args {
                visit(*value);
            }
        }
        ManagedInsn::Branch { condition, .. } | ManagedInsn::Return { value: condition } => {
            if let Some(value) = condition {
                visit(*value);
            }
        }
    }
}

fn payload_of(ssa: &SsaMethod, block: BlockId) -> Option<InsnId> {
    ssa.ir()
        .cfg()
        .block(block)
        .ok()
        .map(|record| *record.payload())
}

fn insn_of(ssa: &SsaMethod, insn: InsnId) -> Option<&ManagedInsn> {
    ssa.ir().instructions().get(insn)
}

/// Whether `block` is reachable from the entry without passing through
/// `avoid`; `dominates` exploits this to report entry-domination.
fn reaches_without(ssa: &SsaMethod, avoid: BlockId, block: BlockId) -> bool {
    let cfg: &Cfg<InsnId> = ssa.ir().cfg();
    let entry = cfg.entry();
    let mut seen: BTreeSet<BlockId> = BTreeSet::new();
    let mut queue = VecDeque::from([entry]);
    while let Some(cursor) = queue.pop_front() {
        if cursor == avoid || !seen.insert(cursor) {
            continue;
        }
        if cursor == block {
            return true;
        }
        let Ok(record) = cfg.block(cursor) else {
            continue;
        };
        for edge in record.outgoing() {
            let Ok(target) = cfg.edge(*edge) else {
                continue;
            };
            queue.push_back(target.target());
        }
    }
    false
}

/// Whether `dom` dominates `block`: every entry-to-block path passes
/// through `dom`.
pub(crate) fn dominates(ssa: &SsaMethod, dom: BlockId, block: BlockId) -> bool {
    if dom == block {
        return true;
    }
    !reaches_without(ssa, dom, block)
}

/// Runs every structural and brute-force oracle over the published IR.
pub(crate) fn check(ssa: &SsaMethod) -> Result<(), String> {
    let cfg: &Cfg<InsnId> = ssa.ir().cfg();
    let mut homes: BTreeMap<ValueId, u16> = BTreeMap::new();
    for (_, slot) in ssa.ir().register_slots().iter() {
        for value in slot.values() {
            homes
                .entry(*value)
                .and_modify(|home| *home = (*home).min(slot.index()))
                .or_insert(slot.index());
        }
    }
    let mut defs: BTreeMap<ValueId, BlockId> = BTreeMap::new();
    let mut kill_var: BTreeMap<BlockId, u16> = BTreeMap::new();
    let mut block_defs: BTreeMap<BlockId, BTreeSet<ValueId>> = BTreeMap::new();
    for block in cfg.block_ids() {
        let Some(insn) = payload_of(ssa, block) else {
            return Err(format!("block {block:?} has no payload"));
        };
        let Some(record) = insn_of(ssa, insn) else {
            return Err(format!("block {block:?} payload {insn:?} missing"));
        };
        for_each_definition(record, |value| {
            defs.insert(value, block);
            block_defs.entry(block).or_default().insert(value);
            if let ManagedInsn::Phi { inputs, .. } = record
                && let Some(var) = inputs
                    .iter()
                    .filter_map(|(_, value)| homes.get(value))
                    .next()
            {
                kill_var.insert(block, *var);
            }
        });
    }

    // 1. Block shape and phi edge identity.
    let mut phis: BTreeMap<BlockId, ValueId> = BTreeMap::new();
    for block in cfg.block_ids() {
        let Some(insn) = payload_of(ssa, block) else {
            continue;
        };
        let Some(ManagedInsn::Phi { result, inputs }) = insn_of(ssa, insn) else {
            continue;
        };
        if phis.contains_key(&block) {
            return Err(format!("block {block:?} holds more than one phi"));
        }
        phis.insert(block, *result);
        let mut head = block;
        let mut visited = BTreeSet::new();
        while visited.insert(head) {
            let record = cfg.block(head).map_err(|error| error.to_string())?;
            let [edge] = record.incoming() else { break };
            let edge = cfg.edge(*edge).map_err(|error| error.to_string())?;
            let pred = cfg
                .block(edge.source())
                .map_err(|error| error.to_string())?;
            if edge.kind() != &decomp_core::cfg::EdgeKind::Unconditional
                || pred.outgoing().len() != 1
                || !matches!(insn_of(ssa, *pred.payload()), Some(ManagedInsn::Phi { .. }))
            {
                break;
            }
            head = edge.source();
        }
        let record = cfg.block(head).map_err(|error| error.to_string())?;
        if record.incoming().len() < 2 {
            return Err(format!(
                "phi {result:?} at {block:?} sits on {} predecessor(s)",
                record.incoming().len()
            ));
        }
        let expected: BTreeSet<EdgeId> = record.incoming().iter().copied().collect();
        let actual: BTreeSet<EdgeId> = inputs.iter().map(|(edge, _)| *edge).collect();
        if expected != actual {
            return Err(format!(
                "phi {result:?} at {block:?} inputs {actual:?} do not match incoming {expected:?}"
            ));
        }
    }

    // 2. May-reach dataflow; then every phi input must reach along its own
    //    edge (its own result excepted: loop-header self reference).
    let mut reach: BTreeMap<BlockId, BTreeSet<ValueId>> = cfg
        .block_ids()
        .map(|block| (block, BTreeSet::new()))
        .collect();
    let order = decomp_core::analysis::postorder(cfg);
    let mut changed = true;
    while changed {
        changed = false;
        for block in &order {
            let mut incoming: BTreeSet<ValueId> = BTreeSet::new();
            let Ok(record) = cfg.block(*block) else {
                continue;
            };
            for edge in record.incoming() {
                let Ok(source) = cfg.edge(*edge) else {
                    continue;
                };
                if let Some(out) = reach.get(&source.source()) {
                    incoming.extend(out.iter().copied());
                }
            }
            if let Some(var) = kill_var.get(block) {
                incoming.retain(|value| homes.get(value) != Some(var));
            }
            if let Some(produced) = block_defs.get(block) {
                incoming.extend(produced.iter().copied());
            }
            if reach.get(block) != Some(&incoming) {
                reach.insert(*block, incoming);
                changed = true;
            }
        }
    }
    for (&block, &result) in &phis {
        let Some(insn) = payload_of(ssa, block) else {
            continue;
        };
        let Some(ManagedInsn::Phi { inputs, .. }) = insn_of(ssa, insn) else {
            continue;
        };
        for (edge, value) in inputs {
            let Ok(source_record) = cfg.edge(*edge) else {
                return Err(format!("phi {result:?} references missing edge {edge:?}"));
            };
            let source = source_record.source();
            let Some(out) = reach.get(&source) else {
                continue;
            };
            if *value != result && !out.contains(value) {
                return Err(format!(
                    "phi {result:?} input {value:?} does not reach along {edge:?} from {source:?}"
                ));
            }
        }
    }

    // 3. Brute-force dominance for every use.
    for block in cfg.block_ids() {
        let Some(insn) = payload_of(ssa, block) else {
            continue;
        };
        let Some(record) = insn_of(ssa, insn) else {
            continue;
        };
        let is_phi = matches!(record, ManagedInsn::Phi { .. });
        if !is_phi {
            for_each_use(record, |value| {
                if let Some(def_block) = defs.get(&value) {
                    let ok = *def_block != block && dominates(ssa, *def_block, block);
                    if !ok {
                        panic!(
                            "definition of {value:?} at {def_block:?} does not dominate its use in {insn:?} at {block:?}"
                        );
                    }
                } else {
                    panic!("use of {value:?} in {insn:?} has no definition");
                }
            });
        }
        if let ManagedInsn::Phi { inputs, .. } = record {
            for (edge, value) in inputs {
                let Ok(edge_record) = cfg.edge(*edge) else {
                    continue;
                };
                let Some(def_block) = defs.get(value) else {
                    return Err(format!("phi input {value:?} has no definition"));
                };
                let source = edge_record.source();
                let ok = *def_block == source || dominates(ssa, *def_block, source);
                if !ok {
                    return Err(format!(
                        "phi input {value:?} defined at {def_block:?} does not dominate edge {edge:?} source {source:?}"
                    ));
                }
            }
        }
    }
    Ok(())
}
