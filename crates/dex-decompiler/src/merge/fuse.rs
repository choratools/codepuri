//! Chain-fusion worklist over the lifted one-instruction CFG.

use std::collections::{HashMap, HashSet, VecDeque};

use decomp_core::cfg::{Cfg, EdgeKind};
use decomp_core::ids::{BlockId, EdgeId};
use managed_ir::insn::ManagedInsn;
use managed_ir::method::InsnArena;
use managed_ir::value::InsnId;

use super::{
    MergeError, MergedMethod, clone_arenas, clone_reg_slots, is_hard_terminator, is_pure_goto,
    try_masks,
};
use crate::lift::LiftedMethod;

/// Per-block facts snapshot of the lifted CFG.
struct Facts {
    entry: BlockId,
    insn_of: HashMap<BlockId, InsnId>,
    out_edges: HashMap<BlockId, Vec<EdgeId>>,
    in_edges: HashMap<BlockId, Vec<EdgeId>>,
    edges: HashMap<EdgeId, (BlockId, BlockId, EdgeKind)>,
    edge_order: Vec<EdgeId>,
    try_mask: HashMap<BlockId, u64>,
    blocks: Vec<BlockId>,
}

/// Fusion result: chain order plus the seam links used while fusing.
struct Fused {
    chains: Vec<Vec<BlockId>>,
    next: HashMap<BlockId, BlockId>,
}

impl Facts {
    fn snapshot(lifted: &LiftedMethod) -> Result<Self, MergeError> {
        let cfg = lifted.ir().cfg();
        let mut insn_of = HashMap::with_capacity(cfg.block_count());
        let mut offset_of = HashMap::with_capacity(cfg.block_count());
        let mut blocks = Vec::with_capacity(cfg.block_count());
        for block in cfg.block_ids() {
            let payload = *cfg
                .block(block)
                .map_err(|_| MergeError::MissingBlock(block))?
                .payload();
            insn_of.insert(block, payload);
            offset_of.insert(block, lifted.offset(payload).unwrap_or(u32::MAX));
            blocks.push(block);
        }
        let mut out_edges: HashMap<BlockId, Vec<EdgeId>> = HashMap::new();
        let mut in_edges: HashMap<BlockId, Vec<EdgeId>> = HashMap::new();
        let mut edges = HashMap::with_capacity(cfg.edge_count());
        let mut edge_order = Vec::with_capacity(cfg.edge_count());
        for edge in cfg.edge_ids() {
            edge_order.push(edge);
            let record = cfg.edge(edge).map_err(|_| MergeError::MissingEdge(edge))?;
            out_edges.entry(record.source()).or_default().push(edge);
            in_edges.entry(record.target()).or_default().push(edge);
            edges.insert(
                edge,
                (record.source(), record.target(), record.kind().clone()),
            );
        }
        let try_mask = try_masks(lifted, &offset_of);
        Ok(Self {
            entry: cfg.entry(),
            insn_of,
            out_edges,
            in_edges,
            edges,
            edge_order,
            try_mask,
            blocks,
        })
    }

    /// Returns the sole unconditional successor, or `None`.
    fn single_uncond_out(&self, block: BlockId) -> Option<BlockId> {
        let edges = self.out_edges.get(&block)?;
        if edges.len() != 1 {
            return None;
        }
        let (source, target, kind) = self.edges.get(&edges[0])?;
        if *source != block || !matches!(kind, EdgeKind::Unconditional) {
            return None;
        }
        Some(*target)
    }
}

/// Union-find chains: `next`/`prev` are the seam links, `root` compresses
/// chain membership, `tail_of` maps a chain head to its current tail.
struct Chains {
    next: HashMap<BlockId, BlockId>,
    prev: HashMap<BlockId, BlockId>,
    root: HashMap<BlockId, BlockId>,
    tail_of: HashMap<BlockId, BlockId>,
}

impl Chains {
    fn new(blocks: &[BlockId]) -> Self {
        let mut root = HashMap::with_capacity(blocks.len());
        let mut tail_of = HashMap::with_capacity(blocks.len());
        for &block in blocks {
            root.insert(block, block);
            tail_of.insert(block, block);
        }
        Self {
            next: HashMap::new(),
            prev: HashMap::new(),
            root,
            tail_of,
        }
    }

    fn find(&mut self, block: BlockId) -> BlockId {
        let mut cursor = block;
        while self.root[&cursor] != cursor {
            cursor = self.root[&cursor];
        }
        let head = cursor;
        cursor = block;
        while self.root[&cursor] != head {
            let parent = self.root[&cursor];
            self.root.insert(cursor, head);
            cursor = parent;
        }
        head
    }

    fn link(&mut self, tail: BlockId, head: BlockId) {
        let tail_head = self.find(tail);
        let head_head = self.find(head);
        let new_tail = self.tail_of[&head_head];
        self.next.insert(tail, head);
        self.prev.insert(head, tail);
        self.root.insert(head_head, tail_head);
        self.tail_of.insert(tail_head, new_tail);
    }
}

/// Revalidates every fusion precondition against current chain state.
fn legal(
    facts: &Facts,
    chains: &mut Chains,
    arena: &InsnArena,
    tail: BlockId,
    head: BlockId,
) -> bool {
    // Invocation supplies an implicit incoming path to the original entry.
    // One explicit backedge is therefore not a single-entry seam: absorbing
    // entry into that predecessor would execute a loop suffix on first entry.
    // Retain an entry goto too, so every merged entry starts with the exact
    // original payload; other pure goto seams remain eligible for fusion.
    if head == facts.entry
        || tail == facts.entry && arena.get(facts.insn_of[&tail]).is_some_and(is_pure_goto)
    {
        return false;
    }
    if chains.next.contains_key(&tail) || chains.prev.contains_key(&head) {
        return false;
    }
    if chains.find(tail) == chains.find(head) {
        return false;
    }
    if facts
        .in_edges
        .get(&head)
        .is_none_or(|edges| edges.len() != 1)
    {
        return false;
    }
    if facts.single_uncond_out(tail) != Some(head) {
        return false;
    }
    if facts.try_mask.get(&tail) != facts.try_mask.get(&head) {
        return false;
    }
    arena
        .get(facts.insn_of[&tail])
        .is_some_and(|insn| !is_hard_terminator(insn))
}

/// Runs the fusion worklist and returns the fused chains in stable order.
fn fuse_chains(facts: &Facts, arena: &InsnArena) -> Fused {
    let mut chains = Chains::new(&facts.blocks);
    let mut ready: VecDeque<(BlockId, BlockId)> = VecDeque::new();
    for &block in &facts.blocks {
        if let Some(target) = facts.single_uncond_out(block) {
            ready.push_back((block, target));
        }
    }
    while let Some((tail, head)) = ready.pop_front() {
        if !legal(facts, &mut chains, arena, tail, head) {
            continue;
        }
        let root = chains.find(tail);
        chains.link(tail, head);
        let new_tail = chains.tail_of[&root];
        if let Some(target) = facts.single_uncond_out(new_tail) {
            ready.push_back((new_tail, target));
        }
    }
    let chain_list: Vec<Vec<BlockId>> = facts
        .blocks
        .iter()
        .copied()
        .filter(|block| !chains.prev.contains_key(block))
        .map(|head| {
            let mut chain = Vec::new();
            let mut cursor = Some(head);
            while let Some(block) = cursor {
                chain.push(block);
                cursor = chains.next.get(&block).copied();
            }
            chain
        })
        .collect();
    Fused {
        chains: chain_list,
        next: chains.next,
    }
}

/// Builds the fused-chain graph from the lifted method.
pub(super) fn fuse(lifted: LiftedMethod) -> Result<MergedMethod, MergeError> {
    let facts = Facts::snapshot(&lifted)?;
    let (mut insns, values) = clone_arenas(lifted.ir());
    let reg_slots = clone_reg_slots(lifted.ir());
    let fused = fuse_chains(&facts, &insns);
    let dropped = count_drops(&fused.chains, &insns, &facts);

    let payloads: Vec<Vec<InsnId>> = fused
        .chains
        .iter()
        .map(|chain| chain_payload(chain, &fused.next, &insns, &facts))
        .collect();
    let entry_chain = fused
        .chains
        .iter()
        .position(|chain| chain.first() == Some(&facts.entry))
        .ok_or(MergeError::MissingBlock(facts.entry))?;
    let entry_payload = payloads
        .get(entry_chain)
        .filter(|payload| payload.first() == Some(&facts.insn_of[&facts.entry]))
        .ok_or(MergeError::MissingBlock(facts.entry))?
        .clone();
    let mut block_map: HashMap<BlockId, BlockId> = HashMap::with_capacity(fused.chains.len());
    let mut cfg: Cfg<Vec<InsnId>> = Cfg::new(entry_payload);
    block_map.insert(facts.entry, cfg.entry());
    for (index, payload) in payloads.into_iter().enumerate() {
        if index == entry_chain {
            continue;
        }
        let block = cfg.add_block(payload);
        block_map.insert(fused.chains[index][0], block);
    }

    let chain_of: HashMap<BlockId, usize> = fused
        .chains
        .iter()
        .enumerate()
        .flat_map(|(index, chain)| chain.iter().map(move |block| (*block, index)))
        .collect();
    let mut edge_remap: HashMap<EdgeId, EdgeId> = HashMap::with_capacity(facts.edges.len());
    // Retain source arena edge order, including exceptional priority and
    // distinct switch edges sharing a target. HashMap iteration here changes
    // successor scheduling and downstream dispatch state/source numbering.
    for edge in &facts.edge_order {
        let (source, target, kind) = facts
            .edges
            .get(edge)
            .ok_or(MergeError::MissingEdge(*edge))?;
        if fused.next.get(source) == Some(target) {
            continue;
        }
        let source_chain = *chain_of
            .get(source)
            .ok_or(MergeError::MissingBlock(*source))?;
        let target_chain = *chain_of
            .get(target)
            .ok_or(MergeError::MissingBlock(*target))?;
        let new_edge = cfg.add_edge(
            block_map[&fused.chains[source_chain][0]],
            block_map[&fused.chains[target_chain][0]],
            kind.clone(),
        )?;
        edge_remap.insert(*edge, new_edge);
    }
    remap_terminators(&mut insns, &fused, &facts, &edge_remap)?;
    let (dead_blocks, dead_insns) = prune_dead(&mut cfg);
    Ok(MergedMethod {
        cfg,
        insns,
        values,
        reg_slots,
        offsets: lifted_offset_map(&lifted),
        width_claims: lifted.into_width_claims(),
        lifted_blocks: facts.blocks.len(),
        dropped_gotos: dropped,
        dead_blocks,
        dead_insns,
    })
}

fn lifted_offset_map(lifted: &LiftedMethod) -> HashMap<InsnId, u32> {
    let mut offsets = HashMap::with_capacity(lifted.ir().instructions().len());
    for (id, _) in lifted.ir().instructions() {
        if let Some(offset) = lifted.offset(id) {
            offsets.insert(id, offset);
        }
    }
    offsets
}

/// Counts redundant gotos removed at chain seams.
fn count_drops(chains: &[Vec<BlockId>], insns: &InsnArena, facts: &Facts) -> usize {
    chains
        .iter()
        .flat_map(|chain| chain.windows(2))
        .filter(|pair| insns.get(facts.insn_of[&pair[0]]).is_some_and(is_pure_goto))
        .count()
}

/// Materializes one chain payload, dropping seam gotos.
fn chain_payload(
    chain: &[BlockId],
    next: &HashMap<BlockId, BlockId>,
    insns: &InsnArena,
    facts: &Facts,
) -> Vec<InsnId> {
    let mut payload = Vec::with_capacity(chain.len());
    for block in chain {
        let id = facts.insn_of[block];
        if next.contains_key(block) && insns.get(id).is_some_and(is_pure_goto) {
            continue;
        }
        payload.push(id);
    }
    payload
}

/// Rewrites surviving terminator edge references to fused edge identities.
fn remap_terminators(
    insns: &mut InsnArena,
    fused: &Fused,
    facts: &Facts,
    edge_remap: &HashMap<EdgeId, EdgeId>,
) -> Result<(), MergeError> {
    for chain in &fused.chains {
        for block in chain {
            let insn_id = facts.insn_of[block];
            let seam = fused.next.contains_key(block);
            let pure = insns.get(insn_id).is_some_and(is_pure_goto);
            if seam && pure {
                continue;
            }
            if let Some(insn) = insns.get_mut(insn_id) {
                remap_insn(insn, edge_remap)?;
            }
        }
    }
    Ok(())
}

fn remap_insn(
    insn: &mut ManagedInsn,
    edge_remap: &HashMap<EdgeId, EdgeId>,
) -> Result<(), MergeError> {
    let resolve = |edge: &EdgeId| {
        edge_remap
            .get(edge)
            .copied()
            .ok_or(MergeError::MissingEdge(*edge))
    };
    match insn {
        ManagedInsn::Branch {
            then_edge,
            else_edge,
            ..
        } => {
            *then_edge = resolve(then_edge)?;
            if let Some(edge) = else_edge {
                *edge = resolve(edge)?;
            }
        }
        ManagedInsn::Switch {
            default_edge,
            cases,
            ..
        } => {
            *default_edge = resolve(default_edge)?;
            for (_, edge) in cases {
                *edge = resolve(edge)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Removes entry-unreachable dead chains, returning removed block and
/// instruction counts. Removal is infallible by construction: dead blocks
/// exist and are never the entry.
fn prune_dead(cfg: &mut Cfg<Vec<InsnId>>) -> (usize, usize) {
    let mut reachable: HashSet<BlockId> = HashSet::new();
    let mut stack = vec![cfg.entry()];
    while let Some(block) = stack.pop() {
        if !reachable.insert(block) {
            continue;
        }
        if let Ok(record) = cfg.block(block) {
            for edge in record.outgoing() {
                if let Ok(target) = cfg.edge(*edge) {
                    stack.push(target.target());
                }
            }
        }
    }
    let dead: Vec<BlockId> = cfg
        .block_ids()
        .filter(|block| !reachable.contains(block))
        .collect();
    let mut dead_insns = 0_usize;
    let mut dead_blocks = 0_usize;
    for block in dead {
        let size = cfg.block(block).map_or(0, |record| record.payload().len());
        if cfg.remove_block(block).is_ok() {
            dead_blocks += 1;
            dead_insns += size;
        }
    }
    (dead_blocks, dead_insns)
}
