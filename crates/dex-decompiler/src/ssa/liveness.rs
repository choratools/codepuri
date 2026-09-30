//! Backward liveness for multi-def register variables over fused chains.
//!
//! Only variables that can possibly need a phi (two or more definitions) are
//! tracked, which keeps sets small; this is the standard liveness input for
//! pruned SSA construction. Phi instructions do not exist yet at this point,
//! which is the classic pruned-SSA formulation.

use std::collections::{HashMap, VecDeque};

use decomp_core::analysis::postorder;
use decomp_core::cfg::Cfg;
use decomp_core::ids::BlockId;
use managed_ir::method::InsnArena;

use super::chains::Chains;
use super::uses::{for_each_definition, for_each_use};
use super::vars::Variables;

/// A dense bitset over the phi-candidate variable slots.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Bits(Vec<u64>);

impl Bits {
    fn new(len: usize) -> Self {
        Self(vec![0; len.div_ceil(64)])
    }

    fn insert(&mut self, slot: usize) {
        self.0[slot / 64] |= 1_u64 << (slot % 64);
    }

    fn contains(&self, slot: usize) -> bool {
        self.0[slot / 64] & (1_u64 << (slot % 64)) != 0
    }

    fn subtract(&mut self, other: &Bits) {
        for (slot, mask) in self.0.iter_mut().zip(other.0.iter()) {
            *slot &= !mask;
        }
    }

    fn merge(&mut self, other: &Bits) {
        for (slot, mask) in self.0.iter_mut().zip(other.0.iter()) {
            *slot |= mask;
        }
    }
}

/// The live-in sets of one method, keyed by mirror block.
#[derive(Default)]
pub(crate) struct Liveness {
    live_in: HashMap<BlockId, Bits>,
}

impl Liveness {
    /// Returns whether phi candidate `slot` is live-in at `block`.
    pub fn is_live_in(&self, block: BlockId, slot: usize) -> bool {
        self.live_in
            .get(&block)
            .is_some_and(|bits| bits.contains(slot))
    }
}

/// Computes pruned-SSA liveness over the mirror graph.
pub(crate) fn compute(
    cfg: &Cfg<usize>,
    chains: &Chains,
    insns: &InsnArena,
    vars: &Variables,
    candidates: &[u32],
) -> Liveness {
    let slot_of: HashMap<u32, usize> = candidates
        .iter()
        .copied()
        .enumerate()
        .map(|(slot, var)| (var, slot))
        .collect();
    let order = postorder(cfg);
    let mut facts = Vec::with_capacity(order.len());
    for &block in &order {
        let mut used = Bits::new(candidates.len());
        let mut killed = Bits::new(candidates.len());
        let payload = &chains.payloads[cfg
            .block(block)
            .map(|record| *record.payload())
            .unwrap_or(0)];
        // Upward-exposed uses are reads before the first definition in
        // execution order. A later definition must not kill an earlier read.
        for insn in payload {
            let Some(record) = insns.get(*insn) else {
                continue;
            };
            for_each_use(record, |value| {
                if let Some(slot) = slot_of.get(&vars.var_of(value))
                    && !killed.contains(*slot)
                {
                    used.insert(*slot);
                }
            });
            for_each_definition(record, |value| {
                if let Some(slot) = slot_of.get(&vars.var_of(value)) {
                    killed.insert(*slot);
                }
            });
        }
        facts.push((used, killed));
    }
    let (sets, _) = solve(cfg, &order, &facts, candidates.len());
    Liveness {
        live_in: order.into_iter().zip(sets).collect(),
    }
}

/// Resolve block identities once; the fixed-point loop uses only dense indexes.
/// All outgoing edge kinds participate, including exceptional control flow.
fn solve(
    cfg: &Cfg<usize>,
    order: &[BlockId],
    facts: &[(Bits, Bits)],
    slots: usize,
) -> (Vec<Bits>, usize) {
    let index_of: HashMap<_, _> = order
        .iter()
        .copied()
        .enumerate()
        .map(|(i, b)| (b, i))
        .collect();
    let mut successors = vec![Vec::new(); order.len()];
    let mut predecessors = vec![Vec::new(); order.len()];
    for (source, &block) in order.iter().enumerate() {
        if let Ok(record) = cfg.block(block) {
            for edge in record.outgoing() {
                if let Ok(edge) = cfg.edge(*edge)
                    && let Some(&target) = index_of.get(&edge.target())
                {
                    successors[source].push(target);
                    predecessors[target].push(source);
                }
            }
        }
    }
    let mut live_in = vec![Bits::new(slots); order.len()];
    let mut next = Bits::new(slots);
    let mut queue: VecDeque<_> = (0..order.len()).collect();
    let mut queued = vec![true; order.len()];
    let mut evaluations = 0;
    while let Some(block) = queue.pop_front() {
        queued[block] = false;
        evaluations += 1;
        next.0.fill(0);
        for &successor in &successors[block] {
            next.merge(&live_in[successor]);
        }
        next.subtract(&facts[block].1);
        next.merge(&facts[block].0);
        if next != live_in[block] {
            // Swap buffers rather than allocate a fresh bitset on each visit.
            std::mem::swap(&mut next, &mut live_in[block]);
            for &predecessor in &predecessors[block] {
                if !queued[predecessor] {
                    queued[predecessor] = true;
                    queue.push_back(predecessor);
                }
            }
        }
    }
    (live_in, evaluations)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use decomp_core::cfg::EdgeKind;

    // Independent, scalar implementation of the original whole-graph
    // equations, including its live-out change criterion.
    fn oracle(
        cfg: &Cfg<usize>,
        order: &[BlockId],
        facts: &[(Bits, Bits)],
        slots: usize,
    ) -> (Vec<Bits>, usize) {
        let mut input = vec![vec![false; slots]; order.len()];
        let mut output = input.clone();
        let mut evaluations = 0;
        loop {
            let mut changed = false;
            for (i, &block) in order.iter().enumerate() {
                evaluations += 1;
                let mut out = vec![false; slots];
                for &edge in cfg.block(block).unwrap().outgoing() {
                    let target = cfg.edge(edge).unwrap().target();
                    if let Some(j) = order.iter().position(|&b| b == target) {
                        for s in 0..slots {
                            out[s] |= input[j][s];
                        }
                    }
                }
                let next: Vec<_> = (0..slots)
                    .map(|s| facts[i].0.contains(s) || (out[s] && !facts[i].1.contains(s)))
                    .collect();
                changed |= output[i] != out || input[i] != next;
                output[i] = out;
                input[i] = next;
            }
            if !changed {
                break;
            }
        }
        (
            input
                .into_iter()
                .map(|set| {
                    let mut bits = Bits::new(slots);
                    for (s, live) in set.into_iter().enumerate() {
                        if live {
                            bits.insert(s);
                        }
                    }
                    bits
                })
                .collect(),
            evaluations,
        )
    }

    #[test]
    fn randomized_cfg_matches_original_equations() {
        let mut seed = 0x51a_1e55_u64;
        let mut random = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as usize
        };
        for case in 0..400 {
            let n = 1 + random() % 32;
            let slots = [0, 1, 63, 64, 65, 129][case % 6];
            let mut cfg = Cfg::new(0);
            let mut ids = vec![cfg.entry()];
            // A removed/reused identity ensures indexes are not raw slot IDs.
            let removed = cfg.add_block(usize::MAX);
            cfg.remove_block(removed).unwrap();
            for i in 1..n {
                ids.push(cfg.add_block(i));
            }
            for _ in 0..random() % (n * 4 + 1) {
                let source = random() % n;
                let target = random() % n;
                let kind = match random() % 4 {
                    0 => EdgeKind::Exception("test".into()),
                    1 => EdgeKind::BranchTrue,
                    2 => EdgeKind::Switch(7),
                    _ => EdgeKind::Unconditional,
                };
                cfg.add_edge(ids[source], ids[target], kind).unwrap();
            }
            let order = postorder(&cfg);
            let facts: Vec<_> = order
                .iter()
                .map(|_| {
                    let mut used = Bits::new(slots);
                    let mut killed = Bits::new(slots);
                    for s in 0..slots {
                        if random() % 7 == 0 {
                            used.insert(s);
                        }
                        if random() % 5 == 0 {
                            killed.insert(s);
                        }
                    }
                    (used, killed)
                })
                .collect();
            assert_eq!(
                solve(&cfg, &order, &facts, slots).0,
                oracle(&cfg, &order, &facts, slots).0,
                "case {case}"
            );
        }
    }

    #[test]
    fn upward_exposed_uses_and_exception_edges() {
        use managed_ir::insn::ManagedInsn;
        use managed_ir::method::{RegisterSlot, RegisterSlotArena, ValueArena};
        use managed_ir::types::ManagedType;
        use managed_ir::value::{InsnId, Value};
        use slotmap::Key;

        let mut values = ValueArena::with_key();
        let value = values.insert(Value::new(InsnId::null(), ManagedType::Unknown));
        let mut regs = RegisterSlotArena::with_key();
        let mut reg = RegisterSlot::new(0);
        reg.append(value);
        regs.insert(reg);
        let vars = Variables::build(&regs, &values, &HashMap::new());
        let mut insns = InsnArena::with_key();
        let read = insns.insert(ManagedInsn::Return { value: Some(value) });
        let write = insns.insert(ManagedInsn::Move {
            result: value,
            value,
        });
        // Move reads before it writes, including a same-variable read/write.
        let chains = Chains {
            payloads: vec![vec![], vec![read, write], vec![write, read], vec![read]],
            ..Chains::default()
        };
        let mut cfg = Cfg::new(0);
        let handler = cfg.add_block(1);
        let same_insn = cfg.add_block(2);
        let unreachable = cfg.add_block(3);
        cfg.add_edge(cfg.entry(), handler, EdgeKind::Exception("E".into()))
            .unwrap();
        cfg.add_edge(handler, same_insn, EdgeKind::Unconditional)
            .unwrap();
        let result = compute(&cfg, &chains, &insns, &vars, &[vars.var_of(value)]);
        for block in [cfg.entry(), handler, same_insn] {
            assert!(result.is_live_in(block, 0));
        }
        assert!(!result.is_live_in(unreachable, 0));

        let definition = insns.insert(ManagedInsn::Opaque {
            family_tag: 0,
            reads: vec![],
            writes: vec![value],
            effects: managed_ir::effects::EffectFlags::NONE,
        });
        let chains = Chains {
            payloads: vec![vec![definition, read]],
            ..Chains::default()
        };
        let cfg = Cfg::new(0);
        assert!(
            !compute(&cfg, &chains, &insns, &vars, &[vars.var_of(value)])
                .is_live_in(cfg.entry(), 0)
        );
    }

    #[test]
    fn worklist_reduces_evaluations_on_local_cycle() {
        let mut cfg = Cfg::new(0);
        let ids: Vec<_> = (0..512).map(|i| cfg.add_block(i + 1)).collect();
        for &id in &ids {
            cfg.add_edge(cfg.entry(), id, EdgeKind::Unconditional)
                .unwrap();
        }
        for i in 0..32 {
            cfg.add_edge(ids[i], ids[(i + 1) % 32], EdgeKind::Unconditional)
                .unwrap();
        }
        let order = postorder(&cfg);
        let facts: Vec<_> = order
            .iter()
            .map(|&b| {
                let mut used = Bits::new(65);
                let payload = *cfg.block(b).unwrap().payload();
                if (1..=32).contains(&payload) {
                    used.insert(payload - 1);
                }
                (used, Bits::new(65))
            })
            .collect();
        let start = std::time::Instant::now();
        let (actual, visits) = solve(&cfg, &order, &facts, 65);
        let worklist_time = start.elapsed();
        let start = std::time::Instant::now();
        let (expected, scans) = oracle(&cfg, &order, &facts, 65);
        let oracle_time = start.elapsed();
        assert_eq!(actual, expected);
        assert!(visits * 2 < scans, "worklist={visits}, scans={scans}");
        eprintln!(
            "liveness: worklist {visits} evaluations ({worklist_time:?}), scalar original-equation oracle {scans} ({oracle_time:?})"
        );
    }
}
