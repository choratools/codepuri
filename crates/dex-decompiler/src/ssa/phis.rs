//! Phi placement (pruned Cytron) and iterated trivial-phi elimination.
//!
//! Phis are placed at dominance-frontier blocks of multi-def register
//! variables where the variable is live-in. Each phi owns a fresh
//! `ValueId`; its per-edge inputs start unfilled and are completed by the
//! renaming pass. Elimination removes phis whose non-self inputs collapse
//! to a single value, rewriting every operand that referenced them.
//! Replacement cycles are detected and left untouched: they cannot arise
//! under pruned SSA and are kept structurally valid rather than
//! mis-rewritten.
//!
//! Multiple variables at a join are represented by a leading phi cluster;
//! every member selects over the incoming edges of the cluster head.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use decomp_core::cfg::Cfg;
use decomp_core::ids::{BlockId, EdgeId};
use managed_ir::insn::ManagedInsn;
use managed_ir::method::{InsnArena, ValueArena};
use managed_ir::types::ManagedType;
use managed_ir::value::{InsnId, Value, ValueId};

use super::chains::Chains;
use super::uses::{for_each_definition, for_each_use, for_each_use_mut};
use super::vars::Variables;

const PHI_PLACEHOLDER: ManagedInsn = ManagedInsn::Return { value: None };

/// One phi under construction at a join block.
#[derive(Debug)]
pub(crate) struct PhiDef {
    /// Arena slot of the phi instruction.
    pub insn: InsnId,
    /// The merged value the phi defines.
    pub result: ValueId,
    /// The variable being merged.
    pub var: u32,
    /// One input per incoming edge; filled by renaming.
    pub inputs: Vec<(EdgeId, Option<ValueId>)>,
    input_index: HashMap<EdgeId, usize>,
}

impl PhiDef {
    /// Records the value flowing along one predecessor edge.
    pub fn fill(&mut self, edge: EdgeId, value: ValueId) {
        if let Some(&index) = self.input_index.get(&edge) {
            self.inputs[index].1 = Some(value);
        }
    }
}

/// Phi definitions keyed by their join block.
#[derive(Debug, Default)]
pub(crate) struct Phis {
    by_block: BTreeMap<BlockId, Vec<PhiDef>>,
    results: HashSet<ValueId>,
}

impl Phis {
    /// Returns the phis of one block, in placement order.
    pub fn at(&self, block: BlockId) -> &[PhiDef] {
        self.by_block.get(&block).map_or(&[][..], Vec::as_slice)
    }

    /// Returns the phis of one block mutably.
    pub fn at_mut(&mut self, block: BlockId) -> &mut [PhiDef] {
        self.by_block
            .get_mut(&block)
            .map_or(&mut [][..], Vec::as_mut_slice)
    }

    /// Returns the number of phis placed.
    pub fn len(&self) -> usize {
        self.by_block.values().map(Vec::len).sum()
    }

    /// Iterates every phi in deterministic placement order.
    pub fn all(&self) -> impl Iterator<Item = &PhiDef> {
        self.by_block.values().flatten()
    }

    fn block_mut(&mut self, block: BlockId) -> &mut Vec<PhiDef> {
        self.by_block.entry(block).or_default()
    }
}

/// Inserts pruned phis for every multi-def register variable.
pub(crate) fn place(
    analysis: &super::analysis::Analysis<'_>,
    chains: &super::chains::Chains,
    insns: &mut InsnArena,
    values: &mut ValueArena,
    vars: &Variables,
    candidates: &[u32],
    live: &super::liveness::Liveness,
) -> Phis {
    let cfg = analysis.cfg;
    let insn_block: HashMap<InsnId, BlockId> = cfg
        .block_ids()
        .flat_map(|block| {
            let chain = cfg
                .block(block)
                .map(|record| *record.payload())
                .unwrap_or(usize::MAX);
            chains
                .payloads
                .get(chain)
                .into_iter()
                .flatten()
                .map(move |insn| (*insn, block))
        })
        .collect();
    let frontier = &analysis.frontier;
    let mut phis = Phis::default();
    let mut placed: HashMap<BlockId, HashSet<u32>> = HashMap::new();
    let mut pending: VecDeque<BlockId> = VecDeque::new();
    for (slot, &var) in candidates.iter().enumerate() {
        pending.clear();
        for value in vars.definitions(var) {
            if let Some(record) = values.get(*value)
                && let Some(block) = insn_block.get(&record.def())
            {
                pending.push_back(*block);
            }
        }
        while let Some(block) = pending.pop_front() {
            for &join in frontier.get(&block).into_iter().flatten() {
                // A phi merges distinct incoming values; a block with a
                // single predecessor has nothing to merge (degenerate
                // frontier cases such as one-predecessor entry chains).
                if cfg
                    .block(join)
                    .map(|record| record.incoming().len())
                    .unwrap_or(0)
                    < 2
                {
                    continue;
                }
                let seen = placed.entry(join).or_default();
                if seen.contains(&var) {
                    continue;
                }
                if !live.is_live_in(join, slot) {
                    continue;
                }
                seen.insert(var);
                let phi = new_phi(cfg, insns, values, var, join);
                phis.results.insert(phi.result);
                phis.block_mut(join).push(phi);
                pending.push_back(join);
            }
        }
    }
    phis
}

fn new_phi(
    cfg: &Cfg<usize>,
    insns: &mut InsnArena,
    values: &mut ValueArena,
    var: u32,
    block: BlockId,
) -> PhiDef {
    let insn = insns.insert(PHI_PLACEHOLDER);
    let result = values.insert(Value::new(insn, ManagedType::Unknown));
    let inputs = cfg
        .block(block)
        .map(|record| {
            record
                .incoming()
                .iter()
                .map(|edge| (*edge, None))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let input_index = inputs
        .iter()
        .enumerate()
        .map(|(index, (edge, _))| (*edge, index))
        .collect();
    PhiDef {
        insn,
        result,
        var,
        inputs,
        input_index,
    }
}

/// Eliminates phis whose inputs collapse to one distinct non-self value,
/// provided the replacement dominates every use site of the phi (the
/// classic trivial-phi caveat: a collapse through a loop-header or
/// bypassed join can otherwise break use-dominates-def).
///
/// Returns the number of eliminated phis.
pub(crate) fn eliminate(
    analysis: &super::analysis::Analysis<'_>,
    chains: &Chains,
    insns: &mut InsnArena,
    values: &mut ValueArena,
    phis: &mut Phis,
) -> usize {
    if phis.by_block.is_empty() {
        return 0;
    }
    let by_result: HashMap<ValueId, (InsnId, Vec<ValueId>)> = phis
        .by_block
        .values()
        .flatten()
        .map(|phi| {
            (
                phi.result,
                (
                    phi.insn,
                    phi.inputs
                        .iter()
                        .filter_map(|(_, value)| *value)
                        .filter(|value| value != &phi.result)
                        .collect(),
                ),
            )
        })
        .collect();
    let mut replacement: HashMap<ValueId, ValueId> = HashMap::new();
    // A separate compressed resolution index leaves candidate links intact
    // for the dominance gate below.
    let mut canonical = HashMap::new();
    let mut dependents: HashMap<ValueId, HashSet<ValueId>> = HashMap::new();
    let mut pending: VecDeque<_> = phis.all().map(|phi| phi.result).collect();
    let mut queued: HashSet<_> = pending.iter().copied().collect();
    while let Some(result) = pending.pop_front() {
        queued.remove(&result);
        let Some((_, inputs)) = by_result.get(&result) else {
            continue;
        };
        let mut resolved = HashSet::new();
        for input in inputs {
            let target = resolve_compressed(&mut canonical, *input);
            dependents.entry(target).or_default().insert(result);
            resolved.insert(target);
        }
        if resolved.len() == 1 {
            let target = *resolved.iter().next().unwrap_or(&result);
            if target != result && replacement.get(&result) != Some(&target) {
                replacement.insert(result, target);
                canonical.insert(result, target);
                for dependent in dependents.get(&result).into_iter().flatten() {
                    if queued.insert(*dependent) {
                        pending.push_back(*dependent);
                    }
                }
            }
        }
    }
    for value in cyclic_keys(&replacement) {
        replacement.remove(&value);
    }
    if !dominance_safe(analysis, chains, insns, phis, &mut replacement) {
        return 0;
    }
    if replacement.is_empty() {
        return 0;
    }
    // Flatten only after cycle rejection and dominance filtering: compression
    // must not hide a cycle or a link that the safety check needs to reject.
    for value in replacement.keys().copied().collect::<Vec<_>>() {
        resolve_compressed(&mut replacement, value);
    }
    for (_, insn) in insns.iter_mut() {
        if matches!(insn, ManagedInsn::Phi { .. }) {
            continue;
        }
        for_each_use_mut(insn, &mut |value: &mut ValueId| {
            *value = resolve(&replacement, *value);
        });
    }
    for block in phis.by_block.values_mut() {
        for phi in block.iter_mut() {
            for (_, value) in &mut phi.inputs {
                if let Some(value) = value {
                    *value = resolve(&replacement, *value);
                }
            }
        }
    }
    let mut removed = 0;
    for (result, (insn, _)) in &by_result {
        if replacement.contains_key(result) {
            values.remove(*result);
            insns.remove(*insn);
            removed += 1;
        }
    }
    for block in phis.by_block.values_mut() {
        block.retain(|phi| !replacement.contains_key(&phi.result));
    }
    phis.by_block.retain(|_, block| !block.is_empty());
    for result in replacement.keys() {
        phis.results.remove(result);
    }
    removed
}

/// Reads the flattened replacement graph after cycle and dominance filtering.
fn resolve(replacement: &HashMap<ValueId, ValueId>, value: ValueId) -> ValueId {
    replacement.get(&value).copied().unwrap_or(value)
}

fn resolve_compressed(replacement: &mut HashMap<ValueId, ValueId>, value: ValueId) -> ValueId {
    let mut path = Vec::new();
    let mut cursor = value;
    while let Some(next) = replacement.get(&cursor).copied() {
        path.push(cursor);
        cursor = next;
        if path.len() > replacement.len() {
            return value;
        }
    }
    for member in path {
        replacement.insert(member, cursor);
    }
    cursor
}

/// Returns replacement keys that participate in a follow cycle.
fn cyclic_keys(replacement: &HashMap<ValueId, ValueId>) -> HashSet<ValueId> {
    let mut status: HashMap<ValueId, bool> = HashMap::new();
    for start in replacement.keys().copied() {
        if status.contains_key(&start) {
            continue;
        }
        let mut path = Vec::new();
        let mut seen = HashSet::new();
        let mut cursor = start;
        let cyclic = loop {
            if let Some(cyclic) = status.get(&cursor) {
                break *cyclic;
            }
            if !seen.insert(cursor) {
                break true;
            }
            let Some(next) = replacement.get(&cursor) else {
                break false;
            };
            path.push(cursor);
            cursor = *next;
        };
        for value in path {
            status.insert(value, cyclic);
        }
    }
    status
        .into_iter()
        .filter_map(|(value, cyclic)| cyclic.then_some(value))
        .collect()
}

/// A use or definition site: phis and payload insns share one position
/// space per chain, with phis first and phi-edge uses at the chain end.
#[derive(Clone, Copy, Eq, PartialEq)]
struct Site {
    chain: usize,
    position: usize,
}

const CHAIN_END: usize = usize::MAX;

/// Filters the replacement map down to dominance-safe entries, returning
/// whether anything survived.
fn dominance_safe(
    analysis: &super::analysis::Analysis<'_>,
    chains: &Chains,
    insns: &InsnArena,
    phis: &Phis,
    replacement: &mut HashMap<ValueId, ValueId>,
) -> bool {
    let cfg = analysis.cfg;
    let intervals = dominance_intervals(analysis);
    let phi_counts: HashMap<usize, usize> = phis
        .by_block
        .iter()
        .map(|(block, list)| {
            (
                cfg.block(*block)
                    .map(|record| *record.payload())
                    .unwrap_or(usize::MAX),
                list.len(),
            )
        })
        .collect();
    let mut def_sites: HashMap<ValueId, Site> = HashMap::new();
    for (block, list) in phis.by_block.iter() {
        let chain = cfg
            .block(*block)
            .map(|record| *record.payload())
            .unwrap_or(usize::MAX);
        for (index, phi) in list.iter().enumerate() {
            def_sites.insert(
                phi.result,
                Site {
                    chain,
                    position: index,
                },
            );
        }
    }
    for (chain, payload) in chains.payloads.iter().enumerate() {
        let offset = phi_counts.get(&chain).copied().unwrap_or(0);
        for (position, insn) in payload.iter().enumerate() {
            if let Some(record) = def_value(insns, *insn) {
                def_sites.entry(record).or_insert(Site {
                    chain,
                    position: offset + position,
                });
            }
        }
    }
    let mut uses_of: HashMap<ValueId, Vec<Site>> = HashMap::new();
    for (chain, payload) in chains.payloads.iter().enumerate() {
        let offset = phi_counts.get(&chain).copied().unwrap_or(0);
        for (position, insn) in payload.iter().enumerate() {
            if let Some(record) = insns.get(*insn) {
                for_each_use(record, |value| {
                    uses_of.entry(value).or_default().push(Site {
                        chain,
                        position: offset + position,
                    });
                });
            }
        }
    }
    for (block, list) in phis.by_block.iter() {
        let chain = cfg
            .block(*block)
            .map(|record| *record.payload())
            .unwrap_or(usize::MAX);
        for phi in list {
            for (edge, value) in &phi.inputs {
                let Some(value) = value else { continue };
                let pred = cfg
                    .edge(*edge)
                    .ok()
                    .and_then(|record| cfg.block(record.source()).ok())
                    .map(|record| *record.payload())
                    .unwrap_or(chain);
                uses_of.entry(*value).or_default().push(Site {
                    chain: pred,
                    position: CHAIN_END,
                });
            }
        }
    }
    // Sites are immutable and filtering never changes any direct target.
    // Each candidate therefore needs exactly one safety check.
    for result in replacement.keys().copied().collect::<Vec<_>>() {
        let Some(target) = replacement.get(&result).copied() else {
            continue;
        };
        let Some(target_site) = def_sites.get(&target) else {
            replacement.remove(&result);
            continue;
        };
        let safe = uses_of.get(&result).is_none_or(|sites| {
            sites
                .iter()
                .all(|site| dominates(analysis, &intervals, *target_site, *site))
        });
        if !safe {
            replacement.remove(&result);
        }
    }
    !replacement.is_empty()
}

fn def_value(insns: &InsnArena, insn: InsnId) -> Option<ValueId> {
    let record = insns.get(insn)?;
    let mut defined = None;
    for_each_definition(record, |value| defined = Some(value));
    defined
}

/// Returns whether a definition site dominates a use site. Phi-edge uses
/// sit at the end of the predecessor chain, so any definition inside that
/// chain dominates them.
fn dominance_intervals(
    analysis: &super::analysis::Analysis<'_>,
) -> HashMap<BlockId, (usize, usize)> {
    let mut children: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
    for block in analysis.cfg.block_ids() {
        if let Some(parent) = analysis.doms.immediate_dominator(block) {
            children.entry(parent).or_default().push(block);
        }
    }
    let mut intervals = HashMap::new();
    let mut clock = 0;
    let mut pending = vec![(analysis.cfg.entry(), false)];
    while let Some((block, exiting)) = pending.pop() {
        if exiting {
            if let Some(interval) = intervals.get_mut(&block) {
                let (_, end) = interval;
                *end = clock;
            }
        } else {
            intervals.insert(block, (clock, clock));
            clock += 1;
            pending.push((block, true));
            for child in children.get(&block).into_iter().flatten().rev() {
                pending.push((*child, false));
            }
        }
    }
    intervals
}

fn dominates(
    analysis: &super::analysis::Analysis<'_>,
    intervals: &HashMap<BlockId, (usize, usize)>,
    def: Site,
    use_site: Site,
) -> bool {
    if def.chain == use_site.chain {
        return def.position < use_site.position;
    }
    let Some(def_block) = analysis.block_of_chain(def.chain) else {
        return false;
    };
    let Some(use_block) = analysis.block_of_chain(use_site.chain) else {
        return false;
    };
    match (intervals.get(&def_block), intervals.get(&use_block)) {
        (Some(&(start, end)), Some(&(position, _))) => start <= position && position < end,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn trivial_phi_with_bypassed_definition_is_not_eliminated() {
        let mut cfg = Cfg::new(0);
        let left = cfg.add_block(1);
        let right = cfg.add_block(2);
        let join = cfg.add_block(3);
        for target in [left, right] {
            cfg.add_edge(
                cfg.entry(),
                target,
                decomp_core::cfg::EdgeKind::Unconditional,
            )
            .unwrap();
            cfg.add_edge(target, join, decomp_core::cfg::EdgeKind::Unconditional)
                .unwrap();
        }
        let mut insns = InsnArena::with_key();
        let mut values = ValueArena::with_key();
        let constant = insns.insert(PHI_PLACEHOLDER);
        let value = values.insert(Value::new(constant, ManagedType::Int));
        insns[constant] = ManagedInsn::Const {
            result: value,
            value: managed_ir::ops::Constant::Int(1),
        };
        let mut phi = new_phi(&cfg, &mut insns, &mut values, 0, join);
        for (_, input) in &mut phi.inputs {
            *input = Some(value);
        }
        let result = phi.result;
        let ret = insns.insert(ManagedInsn::Return {
            value: Some(result),
        });
        let chains = Chains {
            payloads: vec![vec![], vec![constant], vec![], vec![ret]],
            ..Chains::default()
        };
        let mut phis = Phis::default();
        phis.results.insert(result);
        phis.block_mut(join).push(phi);
        let analysis = super::super::analysis::Analysis::new(&cfg);
        assert_eq!(
            eliminate(&analysis, &chains, &mut insns, &mut values, &mut phis),
            0
        );
        assert!(values.contains_key(result));
        assert_eq!(phis.len(), 1);
        assert!(
            matches!(insns[ret], ManagedInsn::Return { value: Some(value) } if value == result)
        );
    }

    #[test]
    fn high_indegree_fill_preserves_edge_order_and_unknown_edge() {
        let mut cfg = Cfg::new(0);
        let join = cfg.add_block(1);
        let mut edges = Vec::new();
        for _ in 0..4096 {
            edges.push(
                cfg.add_edge(cfg.entry(), join, decomp_core::cfg::EdgeKind::Unconditional)
                    .unwrap(),
            );
        }
        let unknown = cfg
            .add_edge(join, cfg.entry(), decomp_core::cfg::EdgeKind::Unconditional)
            .unwrap();
        let mut insns = InsnArena::with_key();
        let mut values = ValueArena::with_key();
        let mut phi = new_phi(&cfg, &mut insns, &mut values, 0, join);
        for edge in edges.iter().rev() {
            phi.fill(*edge, phi.result);
        }
        phi.fill(unknown, phi.result);
        assert_eq!(phi.inputs.len(), edges.len());
        assert_eq!(phi.input_index.len(), edges.len());
        let mut scan_comparisons = 0;
        for (index, edge) in edges.iter().enumerate() {
            let scanned = phi.inputs.iter().position(|(candidate, _)| {
                scan_comparisons += 1;
                candidate == edge
            });
            assert_eq!(scanned, Some(index));
            assert_eq!(phi.input_index[edge], index);
            assert_eq!(phi.inputs[index], (*edge, Some(phi.result)));
        }
        assert_eq!(scan_comparisons, 8_390_656);
        assert_eq!(edges.len(), 4096);
    }

    #[test]
    fn dominance_intervals_match_parent_walk_including_unreachable_blocks() {
        let mut cfg = Cfg::new(0);
        let left = cfg.add_block(1);
        let right = cfg.add_block(2);
        let join = cfg.add_block(3);
        cfg.add_block(4);
        for (source, target) in [
            (cfg.entry(), left),
            (cfg.entry(), right),
            (left, join),
            (right, join),
            (join, left),
        ] {
            cfg.add_edge(source, target, decomp_core::cfg::EdgeKind::Unconditional)
                .unwrap();
        }
        let analysis = super::super::analysis::Analysis::new(&cfg);
        let intervals = dominance_intervals(&analysis);
        for def in cfg.block_ids() {
            for use_block in cfg.block_ids() {
                let indexed = match (intervals.get(&def), intervals.get(&use_block)) {
                    (Some(&(start, end)), Some(&(position, _))) => {
                        start <= position && position < end
                    }
                    _ => false,
                };
                assert_eq!(indexed, analysis.doms.dominates(def, use_block));
            }
        }
    }

    #[test]
    fn reverse_phi_dependencies_collapse_and_cycles_survive() {
        let mut insns = InsnArena::with_key();
        let mut values = ValueArena::with_key();
        let constant = insns.insert(PHI_PLACEHOLDER);
        let initial = values.insert(Value::new(constant, ManagedType::Int));
        insns[constant] = ManagedInsn::Const {
            result: initial,
            value: managed_ir::ops::Constant::Int(1),
        };
        let mut chains = Chains {
            payloads: vec![vec![constant]],
            ..Chains::default()
        };
        let mut cfg = chains.analysis_cfg();
        let edge = cfg
            .add_edge(
                cfg.entry(),
                cfg.entry(),
                decomp_core::cfg::EdgeKind::Unconditional,
            )
            .unwrap();
        let mut phis = Phis::default();
        let mut results = Vec::new();
        for _ in 0..2048 {
            let phi = new_phi(&cfg, &mut insns, &mut values, 0, cfg.entry());
            results.push(phi.result);
            phis.results.insert(phi.result);
            phis.block_mut(cfg.entry()).push(phi);
        }
        for (index, phi) in phis.at_mut(cfg.entry()).iter_mut().enumerate() {
            phi.inputs = vec![(
                edge,
                Some(results.get(index + 1).copied().unwrap_or(initial)),
            )];
        }
        let ret = insns.insert(ManagedInsn::Return {
            value: Some(results[0]),
        });
        chains.payloads[0].push(ret);
        let analysis = super::super::analysis::Analysis::new(&cfg);
        assert_eq!(
            eliminate(&analysis, &chains, &mut insns, &mut values, &mut phis),
            2048
        );
        assert!(
            matches!(insns[ret], ManagedInsn::Return { value: Some(value) } if value == initial)
        );
        let replacement = HashMap::from([
            (results[0], results[1]),
            (results[1], results[0]),
            (results[2], results[0]),
            (results[3], initial),
        ]);
        assert_eq!(
            cyclic_keys(&replacement),
            HashSet::from([results[0], results[1], results[2]])
        );
    }
}
