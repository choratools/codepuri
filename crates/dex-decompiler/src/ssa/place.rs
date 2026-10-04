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

use std::collections::{HashMap, HashSet, VecDeque};

use decomp_core::analysis::Dominators;
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
    analysis: &super::analysis::Analysis<'_>,
    chains: &mut Chains,
    insns: &InsnArena,
    offsets: &HashMap<InsnId, u32>,
) -> Placement {
    let cfg = analysis.cfg;
    let mut placed: HashSet<InsnId> = chains.payloads.iter().flatten().copied().collect();
    let unplaced: Vec<InsnId> = insns
        .iter()
        .map(|(id, _)| id)
        .filter(|id| !placed.contains(id))
        .collect();
    // Absence from the CFG is not synthetic provenance: merge also leaves
    // unreachable real instructions in the arena. Normal-success commits are
    // Moves without offsets, but are CFG-owned, never standalone synthetics.
    let dead_real: HashSet<InsnId> = unplaced
        .iter()
        .copied()
        .filter(|id| {
            offsets.contains_key(id)
                || !matches!(
                    insns.get(*id),
                    Some(
                        ManagedInsn::Const { .. }
                            | ManagedInsn::Compare { .. }
                            | ManagedInsn::Not { .. }
                    ) | Some(ManagedInsn::Opaque {
                        family_tag: 0x8001 | 0x8002,
                        ..
                    })
                )
        })
        .collect();
    let users = value_users(insns);
    let doms = &analysis.doms;
    let mut placement = Placement::default();
    let mut entry_order: Vec<InsnId> = Vec::new();
    let mut alive: HashSet<InsnId> = placed.clone();
    alive.extend(
        unplaced
            .iter()
            .copied()
            .filter(|id| !dead_real.contains(id)),
    );
    let mut counts: HashMap<ValueId, usize> = HashMap::new();
    let mut definers = HashMap::new();
    for (id, record) in insns.iter() {
        for_each_definition(record, |value| {
            definers.insert(value, id);
        });
        if alive.contains(&id) {
            for_each_use(record, |value| *counts.entry(value).or_default() += 1);
        }
    }
    let mut pending: VecDeque<_> = unplaced.iter().copied().collect();
    while let Some(id) = pending.pop_front() {
        if !alive.contains(&id) || placed.contains(&id) {
            continue;
        }
        let Some(record) = insns.get(id) else {
            continue;
        };
        // Only discard pure synthetic definitions. Dead real instructions
        // were excluded above using provenance, even when they have effects.
        if matches!(record, ManagedInsn::Opaque { family_tag, effects, .. } if *family_tag != 0x8002 && *effects != managed_ir::effects::EffectFlags::NONE)
        {
            continue;
        }
        let mut used = false;
        for_each_definition(record, |value| {
            used |= counts.get(&value).copied().unwrap_or(0) != 0
        });
        if used {
            continue;
        }
        alive.remove(&id);
        for_each_use(record, |value| {
            if let Some(count) = counts.get_mut(&value) {
                *count -= 1;
                if *count == 0
                    && let Some(definer) = definers.get(&value)
                {
                    pending.push_back(*definer);
                }
            }
        });
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
            if let ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads,
                writes,
                ..
            } = record
            {
                // A caught object is supplied at handler entry, not at method
                // entry. Prove the unique offset-bearing move-exception use
                // and exceptional incoming chain before materializing it there.
                if reads.is_empty()
                    && writes.len() == 1
                    && let Some(sites) = users.get(&writes[0])
                    && sites.len() == 1
                    && let Some((chain, position)) =
                        current_position(chains, &chain_position, sites[0])
                    && offsets.contains_key(&sites[0])
                    && matches!(insns.get(sites[0]), Some(ManagedInsn::Move { value, .. }) if *value == writes[0])
                    && cfg.block(mirror(cfg, chain)).is_ok_and(|b| {
                        !b.incoming().is_empty()
                            && b.incoming().iter().all(|e| {
                                cfg.edge(*e).is_ok_and(|e| {
                                    matches!(e.kind(), decomp_core::cfg::EdgeKind::Exception(_))
                                })
                            })
                    })
                {
                    chains.payloads[chain].insert(position, insn);
                    placement.inserted += 1;
                    placed.insert(insn);
                    progressed = true;
                }
                continue;
            }
            let mut reads = false;
            for_each_use(record, |_| reads = true);
            if reads {
                if let Some((chain, position)) =
                    first_use_site(chains, &chain_position, &users, record)
                {
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
                if let Some(block) = common_dominator(doms, &targets) {
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
    chains.payloads[entry_chain].splice(0..0, entry_order);
    // Dead real code and killed (unused) synthetics are removed. An alive
    // synthetic that resisted
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
    chains: &Chains,
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
            let Some((chain, position)) = current_position(chains, chain_position, *user) else {
                continue;
            };
            let better = match best {
                None => true,
                Some((best_chain, best_position)) => {
                    chain < best_chain || (chain == best_chain && position < best_position)
                }
            };
            if better {
                best = Some((chain, position));
            }
        }
    }
    best
}

/// Placements earlier in this pass can shift an instruction inside its chain.
/// Chain ownership stays fixed, but the cached position is only a hint. Locate
/// the actual use before inserting a predicate, so it observes every preceding
/// register definition (including a cmp after a newly placed handler input).
fn current_position(
    chains: &Chains,
    positions: &HashMap<InsnId, (usize, usize)>,
    instruction: InsnId,
) -> Option<(usize, usize)> {
    let &(chain, hint) = positions.get(&instruction)?;
    let payload = chains.payloads.get(chain)?;
    let position = if payload.get(hint) == Some(&instruction) {
        hint
    } else {
        payload.iter().position(|id| *id == instruction)?
    };
    Some((chain, position))
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use managed_ir::{
        effects::EffectFlags, method::ValueArena, ops::Constant, types::ManagedType, value::Value,
    };

    #[test]
    fn reverse_dead_dependencies_and_effectful_roots() {
        let mut insns = InsnArena::with_key();
        let mut values = ValueArena::with_key();
        let mut ids = Vec::new();
        let mut results = Vec::new();
        // Reserve consumers before their definers: arena order must not be
        // treated as topological order by the deletion worklist.
        for _ in 0..2048 {
            let id = insns.insert(ManagedInsn::Return { value: None });
            results.push(values.insert(Value::new(id, ManagedType::Boolean)));
            ids.push(id);
        }
        for i in 0..ids.len() - 1 {
            insns[ids[i]] = ManagedInsn::Not {
                result: results[i],
                value: results[i + 1],
            };
        }
        insns[*ids.last().unwrap()] = ManagedInsn::Const {
            result: *results.last().unwrap(),
            value: Constant::Int(1),
        };
        let root = insns.insert(ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![],
            effects: EffectFlags::NONE,
        });
        let effect = insns.insert(ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![],
            effects: EffectFlags::MAY_THROW,
        });
        let ret = insns.insert(ManagedInsn::Return { value: None });
        let mut chains = Chains {
            payloads: vec![vec![ret]],
            ..Chains::default()
        };
        let cfg = chains.analysis_cfg();
        let analysis = super::super::analysis::Analysis::new(&cfg);
        let placement = place_synthetics(&analysis, &mut chains, &insns, &HashMap::new());
        assert_eq!(placement.dead.len(), ids.len() + 1);
        assert!(placement.dead.contains(&root));
        assert!(!placement.dead.contains(&effect));
        assert!(chains.payloads[0].contains(&effect));
    }
}
