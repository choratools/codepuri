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

use decomp_core::analysis::{Dominators, dominance_frontier, dominators};
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
}

impl PhiDef {
    /// Records the value flowing along one predecessor edge.
    pub fn fill(&mut self, edge: EdgeId, value: ValueId) {
        for (candidate, input) in &mut self.inputs {
            if *candidate == edge {
                *input = Some(value);
                return;
            }
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
    cfg: &Cfg<usize>,
    chains: &super::chains::Chains,
    insns: &mut InsnArena,
    values: &mut ValueArena,
    vars: &Variables,
    candidates: &[u32],
    live: &super::liveness::Liveness,
) -> Phis {
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
    let frontier = dominance_frontier(cfg, &dominators(cfg));
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
    PhiDef {
        insn,
        result,
        var,
        inputs,
    }
}

/// Eliminates phis whose inputs collapse to one distinct non-self value,
/// provided the replacement dominates every use site of the phi (the
/// classic trivial-phi caveat: a collapse through a loop-header or
/// bypassed join can otherwise break use-dominates-def).
///
/// Returns the number of eliminated phis.
pub(crate) fn eliminate(
    cfg: &Cfg<usize>,
    chains: &Chains,
    insns: &mut InsnArena,
    values: &mut ValueArena,
    phis: &mut Phis,
) -> usize {
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
    let mut changed = true;
    while changed {
        changed = false;
        for (result, (_, inputs)) in &by_result {
            let resolved: HashSet<ValueId> = inputs
                .iter()
                .map(|value| resolve(&replacement, *value))
                .collect();
            if resolved.len() == 1 {
                let target = *resolved.iter().next().unwrap_or(result);
                if target != *result && replacement.get(result) != Some(&target) {
                    replacement.insert(*result, target);
                    changed = true;
                }
            }
        }
    }
    for value in cyclic_keys(&replacement) {
        replacement.remove(&value);
    }
    if !dominance_safe(cfg, chains, insns, phis, &mut replacement) {
        return 0;
    }
    if replacement.is_empty() {
        return 0;
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

/// Follows replacement links to their fixpoint; bounded by the map size.
fn resolve(replacement: &HashMap<ValueId, ValueId>, value: ValueId) -> ValueId {
    let mut cursor = value;
    let mut guard = 0_usize;
    while let Some(next) = replacement.get(&cursor) {
        cursor = *next;
        guard += 1;
        if guard > replacement.len() {
            return value;
        }
    }
    cursor
}

/// Returns replacement keys that participate in a follow cycle.
fn cyclic_keys(replacement: &HashMap<ValueId, ValueId>) -> HashSet<ValueId> {
    replacement
        .keys()
        .copied()
        .filter(|start| {
            let mut seen = HashSet::new();
            let mut cursor = *start;
            loop {
                if !seen.insert(cursor) {
                    return true;
                }
                match replacement.get(&cursor) {
                    Some(next) => cursor = *next,
                    None => return false,
                }
            }
        })
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
    cfg: &Cfg<usize>,
    chains: &Chains,
    insns: &InsnArena,
    phis: &Phis,
    replacement: &mut HashMap<ValueId, ValueId>,
) -> bool {
    let doms = dominators(cfg);
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
    loop {
        let mut changed = false;
        for result in replacement.keys().copied().collect::<Vec<_>>() {
            let Some(target) = replacement.get(&result).copied() else {
                continue;
            };
            let Some(target_site) = def_sites.get(&target) else {
                replacement.remove(&result);
                changed = true;
                continue;
            };
            let safe = uses_of.get(&result).is_none_or(|sites| {
                sites
                    .iter()
                    .all(|site| dominates(&doms, cfg, *target_site, *site))
            });
            if !safe {
                replacement.remove(&result);
                changed = true;
            }
        }
        if !changed {
            return !replacement.is_empty();
        }
    }
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
fn dominates(doms: &Dominators, cfg: &Cfg<usize>, def: Site, use_site: Site) -> bool {
    if def.chain == use_site.chain {
        return def.position < use_site.position;
    }
    let Some(def_block) = block_of_chain(cfg, def.chain) else {
        return false;
    };
    let Some(use_block) = block_of_chain(cfg, use_site.chain) else {
        return false;
    };
    doms.dominates(def_block, use_block)
}

fn block_of_chain(cfg: &Cfg<usize>, chain: usize) -> Option<BlockId> {
    cfg.block_ids().find(|block| {
        cfg.block(*block)
            .is_ok_and(|record| *record.payload() == chain)
    })
}
