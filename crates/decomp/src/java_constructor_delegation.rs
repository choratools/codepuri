//! Receiver-bound identity attestation, separate from source/effect validation.
use super::*;
use decomp_core::{cfg::EdgeKind, ids::BlockId};
use managed_ir::value::ValueId;
use std::collections::{HashMap, HashSet};

struct ReceiverProof<'a> {
    ir: &'a MethodIr<SsaForm>,
    receiver: ValueId,
    definitions: HashMap<managed_ir::value::InsnId, BlockId>,
    dominance: decomp_core::analysis::Dominators,
    memo: HashMap<ValueId, bool>,
    active: HashSet<ValueId>,
}
impl<'a> ReceiverProof<'a> {
    fn new(ir: &'a MethodIr<SsaForm>, receiver: ValueId) -> Option<Self> {
        ir.cfg().validate().ok()?;
        let root = ir.value(receiver).ok()?;
        if !matches!(ir.instruction(root.def()).ok()?, ManagedInsn::Opaque {reads,writes,effects,..}
            if reads.is_empty() && writes == &[receiver] && *effects == managed_ir::effects::EffectFlags::NONE)
        {
            return None;
        }
        let mut definitions = HashMap::new();
        for block in ir.cfg().block_ids() {
            if definitions
                .insert(*ir.cfg().block(block).ok()?.payload(), block)
                .is_some()
            {
                return None;
            }
        }
        Some(Self {
            ir,
            receiver,
            definitions,
            dominance: decomp_core::analysis::dominators(ir.cfg()),
            memo: HashMap::new(),
            active: HashSet::new(),
        })
    }
    fn available(&self, value: ValueId, before: BlockId) -> bool {
        value == self.receiver
            || self
                .ir
                .value(value)
                .ok()
                .and_then(|v| self.definitions.get(&v.def()))
                .is_some_and(|&definition| self.dominance.dominates(definition, before))
    }
    fn copy(&mut self, value: ValueId) -> bool {
        if value == self.receiver {
            return true;
        }
        if let Some(&answer) = self.memo.get(&value) {
            return answer;
        }
        if self.active.len() >= 128 || !self.active.insert(value) {
            return false;
        }
        let answer = self.copy_inner(value);
        self.active.remove(&value);
        self.memo.insert(value, answer);
        answer
    }
    fn copy_inner(&mut self, value: ValueId) -> bool {
        let Some(record) = self.ir.value(value).ok() else {
            return false;
        };
        let Some(&block) = self.definitions.get(&record.def()) else {
            return false;
        };
        match self.ir.instruction(record.def()) {
            Ok(ManagedInsn::Move {
                result,
                value: source,
            }) if *result == value => self.available(*source, block) && self.copy(*source),
            Ok(ManagedInsn::Phi { result, inputs }) if *result == value && !inputs.is_empty() => {
                let inputs = inputs.clone();
                let mut head = block;
                let mut seen = HashSet::new();
                loop {
                    if !seen.insert(head) {
                        return false;
                    }
                    let Ok(node) = self.ir.cfg().block(head) else {
                        return false;
                    };
                    if node.outgoing().len() != 1
                        || !self
                            .ir
                            .cfg()
                            .edge(node.outgoing()[0])
                            .is_ok_and(|e| e.kind() == &EdgeKind::Unconditional)
                    {
                        return false;
                    }
                    let [edge] = node.incoming() else {
                        break;
                    };
                    let Ok(edge) = self.ir.cfg().edge(*edge) else {
                        return false;
                    };
                    let Ok(parent) = self.ir.cfg().block(edge.source()) else {
                        return false;
                    };
                    if edge.kind() == &EdgeKind::Unconditional
                        && matches!(
                            self.ir.instruction(*parent.payload()),
                            Ok(ManagedInsn::Phi { .. })
                        )
                    {
                        head = edge.source();
                    } else {
                        break;
                    }
                }
                let Ok(node) = self.ir.cfg().block(head) else {
                    return false;
                };
                if inputs.len() != node.incoming().len() {
                    return false;
                }
                let mut keys = HashSet::new();
                for (edge, source) in inputs {
                    if !keys.insert(edge) || !node.incoming().contains(&edge) {
                        return false;
                    }
                    let Ok(edge) = self.ir.cfg().edge(edge) else {
                        return false;
                    };
                    if matches!(edge.kind(), EdgeKind::Exception(_))
                        || edge.target() != head
                        || !self.available(source, edge.source())
                        || !self.copy(source)
                    {
                        return false;
                    }
                }
                true
            }
            _ => false,
        }
    }
}

enum Prefix {
    Target(MethodRef),
    Abrupt,
    Next(Vec<BlockId>),
}

pub(super) fn attest(
    ir: &MethodIr<SsaForm>,
    receiver: ValueId,
    owner: &str,
    resolve: impl Fn(MethodRef) -> Option<MethodKey>,
) -> Option<MethodRef> {
    let mut receiver_proof = ReceiverProof::new(ir, receiver)?;
    let entry = ir.cfg().entry();
    let mut pending = vec![entry];
    let mut nodes = HashMap::new();
    while let Some(block) = pending.pop() {
        if nodes.contains_key(&block) {
            continue;
        }
        if nodes.len() >= 4096 {
            return None;
        }
        let node = ir.cfg().block(block).ok()?;
        if node.outgoing().iter().any(|edge| {
            ir.cfg()
                .edge(*edge)
                .map_or(true, |edge| matches!(edge.kind(), EdgeKind::Exception(_)))
        }) {
            return None;
        }
        let insn = ir.instruction(*node.payload()).ok()?;
        let prefix = match insn {
            ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                method,
                args,
                result: None,
            } if args
                .first()
                .is_some_and(|value| receiver_proof.copy(*value)) =>
            {
                let key = resolve(*method)?;
                if key.owner != owner || key.name != "<init>" || key.return_type != "V" {
                    return None;
                }
                Prefix::Target(*method)
            }
            ManagedInsn::Throw { .. } if node.outgoing().is_empty() => Prefix::Abrupt,
            ManagedInsn::Return { .. } | ManagedInsn::Throw { .. } => return None,
            _ => {
                let edges = node.outgoing();
                match insn {
                    ManagedInsn::Branch {
                        condition: Some(_),
                        then_edge,
                        else_edge: Some(other),
                    } if edges.len() == 2
                        && then_edge != other
                        && edges.contains(then_edge)
                        && edges.contains(other)
                        && *ir.cfg().edge(*then_edge).ok()?.kind() == EdgeKind::BranchTrue
                        && *ir.cfg().edge(*other).ok()?.kind() == EdgeKind::BranchFalse => {}
                    ManagedInsn::Branch {
                        condition: None,
                        then_edge,
                        else_edge: None,
                    } if edges == [*then_edge]
                        && *ir.cfg().edge(*then_edge).ok()?.kind() == EdgeKind::Unconditional => {}
                    ManagedInsn::Branch { .. } | ManagedInsn::Switch { .. } => return None,
                    _ if edges.len() == 1
                        && *ir.cfg().edge(edges[0]).ok()?.kind() == EdgeKind::Unconditional => {}
                    _ => return None,
                }
                let next = edges
                    .iter()
                    .map(|edge| ir.cfg().edge(*edge).ok().map(|e| e.target()))
                    .collect::<Option<Vec<_>>>()?;
                pending.extend(next.iter().copied());
                Prefix::Next(next)
            }
        };
        nodes.insert(block, prefix);
    }
    // Topological validation rejects every cycle, including one with an exit.
    let mut incoming: HashMap<_, usize> = nodes.keys().map(|block| (*block, 0)).collect();
    for prefix in nodes.values() {
        if let Prefix::Next(next) = prefix {
            for block in next {
                *incoming.get_mut(block)? += 1;
            }
        }
    }
    let mut ready: Vec<_> = incoming
        .iter()
        .filter_map(|(block, count)| (*count == 0).then_some(*block))
        .collect();
    let mut visited = 0;
    let mut target = None;
    while let Some(block) = ready.pop() {
        visited += 1;
        match nodes.get(&block)? {
            Prefix::Target(method) => {
                if target.is_some_and(|previous| previous != *method) {
                    return None;
                }
                target = Some(*method);
            }
            Prefix::Abrupt => {}
            Prefix::Next(next) => {
                for child in next {
                    let count = incoming.get_mut(child)?;
                    *count -= 1;
                    if *count == 0 {
                        ready.push(*child);
                    }
                }
            }
        }
    }
    (visited == nodes.len()).then_some(target).flatten()
}

#[cfg(test)]
#[path = "java_constructor_delegation_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "java_constructor_delegation_phi_tests.rs"]
mod phi_tests;
