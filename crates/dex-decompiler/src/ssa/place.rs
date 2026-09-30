//! Placement of lifter-synthetic definitions into the chain substrate.
//!
//! The lifter creates single-definition *ephemeral* values outside the
//! instruction stream: literal constants, branch predicates (`Compare`,
//! `Not`), initial reads of never-written registers, and handler inputs.
//! Their defining instructions exist in the arena but own no chain, and
//! managed-ir's verifier requires every arena instruction to be placed.
//!
//! - Operand-free definitions (constants, initial reads, handler inputs)
//!   are placed at the entry chain head: entry dominates everything, so
//!   their uses are always dominated.
//! - Read-bearing predicates execute with their operands' values, so the
//!   primary placement is directly before the earliest use site;
//!   renaming then rewrites their operands to the reaching definition at
//!   that exact position. Multi-use predicates fall back to the head of
//!   the common dominator of their use chains, where renaming's
//!   entry-initial fallback restores dominance.
//! - Zero-use definitions are dead and are removed from the arenas
//!   outright; synthetics that only feed other dead synthetics are
//!   collected iteratively.
//!
//! Arena insertion order of synthetic definitions is a valid topological
//! order (the lifter creates operands before consumers), so hoisted
//! operand-free definitions keep that relative order at entry.

use std::collections::{HashMap, HashSet};

use decomp_core::analysis::{Dominators, dominators};
use decomp_core::cfg::Cfg;
use decomp_core::ids::BlockId;
use managed_ir::insn::ManagedInsn;
use managed_ir::method::InsnArena;
use managed_ir::value::{InsnId, ValueId};

use super::chains::Chains;
use super::uses::{for_each_definition, for_each_use};

/// Outcome of synthetic placement.
#[derive(Debug, Default)]
pub(crate) struct Placement {
    /// Definitions placed at the entry chain head.
    pub hoisted: usize,
    /// Definitions placed at use sites or use-chain heads.
    pub inserted: usize,
    /// Dead definitions removed from the arenas.
    pub dead: Vec<InsnId>,
}

/// Places every unplaced synthetic definition or marks it dead.
pub(crate) fn place_synthetics(
    cfg: &Cfg<usize>,
    chains: &mut Chains,
    insns: &InsnArena,
) -> Placement {
    let mut placed: HashSet<InsnId> = chains.payloads.iter().flatten().copied().collect();
    let unplaced: Vec<InsnId> = insns
        .iter()
        .map(|(id, _)| id)
        .filter(|id| !placed.contains(id))
        .collect();
    let users = value_users(insns);
    let doms = dominators(cfg);
    let mut placement = Placement::default();
    let mut entry_order: Vec<InsnId> = Vec::new();
    let mut alive: HashSet<InsnId> = placed.clone();
    alive.extend(unplaced.iter().copied());
    // Kill fixpoint: an unplaced insn whose definitions nobody (alive)
    // uses is dead — including whole dead chains left unplaced by merge.
    // Counts are recomputed from scratch each round because killing one
    // insn can orphan another.
    loop {
        let mut counts: HashMap<ValueId, usize> = HashMap::new();
        for (insn_id, record) in insns.iter() {
            if !alive.contains(&insn_id) {
                continue;
            }
            for_each_use(record, |value| *counts.entry(value).or_default() += 1);
        }
        let mut changed = false;
        for &insn in &unplaced {
            if !alive.contains(&insn) {
                continue;
            }
            let Some(record) = insns.get(insn) else {
                continue;
            };
            let mut used = false;
            for_each_definition(record, |value| {
                if counts.get(&value).copied().unwrap_or_default() > 0 {
                    used = true;
                }
            });
            if !used {
                alive.remove(&insn);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Placement runs to a fixpoint: a definer may only be insertable once
    // the insn that uses its value has itself been placed (arena order is
    // not dependency order). Each pass places at least one insn or stops.
    loop {
        let mut progressed = false;
        let chain_position = insn_positions(chains, cfg);
        for &insn in &unplaced {
            if placed.contains(&insn) || !alive.contains(&insn) {
                continue;
            }
            let Some(record) = insns.get(insn) else {
                continue;
            };
            let mut reads = false;
            for_each_use(record, |_| reads = true);
            if reads {
                if let Some((chain, position)) = first_use_site(&chain_position, &users, record) {
                    chains.payloads[chain].insert(position, insn);
                    placement.inserted += 1;
                    placed.insert(insn);
                    progressed = true;
                    continue;
                }
                let targets: Vec<BlockId> = for_each_definition_collect(record)
                    .into_iter()
                    .filter_map(|value| users.get(&value))
                    .flatten()
                    .filter_map(|user| {
                        chain_position
                            .get(user)
                            .map(|(chain, _)| mirror(cfg, *chain))
                    })
                    .collect();
                if let Some(block) = common_dominator(&doms, &targets) {
                    let chain = cfg
                        .block(block)
                        .map(|record| *record.payload())
                        .unwrap_or(0);
                    chains.payloads[chain].insert(0, insn);
                    placement.inserted += 1;
                    placed.insert(insn);
                    progressed = true;
                }
            } else {
                entry_order.push(insn);
                placement.hoisted += 1;
                placed.insert(insn);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    let entry_chain = cfg
        .block(cfg.entry())
        .map(|record| *record.payload())
        .unwrap_or(0);
    for insn in entry_order.into_iter().rev() {
        chains.payloads[entry_chain].insert(0, insn);
    }
    // Only killed (unused) insns are dead. An alive insn that resisted
    // placement stays in the arena and is reported by the structural
    // verifier rather than silently dropped.
    placement.dead = Vec::from_iter(
        unplaced
            .iter()
            .copied()
            .filter(|insn| !placed.contains(insn) && !alive.contains(insn)),
    );
    placement.dead.sort();
    placement
}

fn mirror(cfg: &Cfg<usize>, chain: usize) -> BlockId {
    cfg.block_ids()
        .find(|block| {
            cfg.block(*block)
                .is_ok_and(|record| *record.payload() == chain)
        })
        .unwrap_or_else(|| cfg.entry())
}

fn for_each_definition_collect(record: &ManagedInsn) -> Vec<ValueId> {
    let mut defined = Vec::new();
    for_each_definition(record, |value| defined.push(value));
    defined
}

/// Maps every placed insn to its `(chain, position)`.
fn insn_positions(chains: &Chains, cfg: &Cfg<usize>) -> HashMap<InsnId, (usize, usize)> {
    let mut map = HashMap::new();
    for block in cfg.block_ids() {
        let Ok(record) = cfg.block(block) else {
            continue;
        };
        let Some(list) = chains.payloads.get(*record.payload()) else {
            continue;
        };
        for (position, insn) in list.iter().enumerate() {
            map.insert(*insn, (*record.payload(), position));
        }
    }
    map
}

/// Maps every value to the placed insns that use it.
fn value_users(insns: &InsnArena) -> HashMap<ValueId, Vec<InsnId>> {
    let mut users: HashMap<ValueId, Vec<InsnId>> = HashMap::new();
    for (insn_id, record) in insns.iter() {
        for_each_use(record, |value| {
            users.entry(value).or_default().push(insn_id)
        });
    }
    users
}

/// Locates the earliest use site of the record's definitions, expressed as
/// the `(chain, position)` at which the definition should be inserted (so
/// it directly precedes that use).
fn first_use_site(
    chain_position: &HashMap<InsnId, (usize, usize)>,
    users: &HashMap<ValueId, Vec<InsnId>>,
    record: &ManagedInsn,
) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for value in for_each_definition_collect(record) {
        let Some(sites) = users.get(&value) else {
            continue;
        };
        for user in sites {
            let Some((chain, position)) = chain_position.get(user) else {
                continue;
            };
            let better = match best {
                None => true,
                Some((best_chain, best_position)) => {
                    *chain < best_chain || (*chain == best_chain && *position < best_position)
                }
            };
            if better {
                best = Some((*chain, *position));
            }
        }
    }
    best
}

/// Returns the greatest common dominator of the blocks, if any.
fn common_dominator(doms: &Dominators, blocks: &[BlockId]) -> Option<BlockId> {
    let mut candidate = *blocks.first()?;
    for &block in &blocks[1..] {
        candidate = intersect(doms, candidate, block)?;
    }
    Some(candidate)
}

fn intersect(doms: &Dominators, left: BlockId, right: BlockId) -> Option<BlockId> {
    let mut walk = right;
    let mut steps = 0_usize;
    while !doms.dominates(walk, left) {
        walk = doms.immediate_dominator(walk)?;
        steps += 1;
        if steps > 1_000_000 {
            return None;
        }
    }
    Some(walk)
}
