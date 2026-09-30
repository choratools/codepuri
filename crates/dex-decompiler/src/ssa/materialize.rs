//! Final materialization: the chain substrate becomes a published
//! `MethodIr<SsaForm>`.
//!
//! managed-ir's contract requires each CFG block to carry exactly one
//! `InsnId`, so every surviving phi and instruction gets its own block: a
//! join chain's phis occupy consecutive leading blocks, chain instructions
//! follow in order, and the chain's terminator edges leave the last block.
//! The join's incoming edges enter the phi cluster head; every phi in the
//! cluster selects over those same edges. Edge identities are remapped once,
//! here: phi inputs (keyed by mirror edge identities) and terminators
//! (keyed by merged edge identities) both resolve through the chain edge
//! index to final edge identities. The instruction arena is patched in
//! place, so all identities are stable and value provenance survives
//! untouched.

use std::collections::{HashMap, HashSet};

use decomp_core::cfg::{Cfg, EdgeKind};
use decomp_core::ids::{BlockId, EdgeId};
use managed_ir::insn::ManagedInsn;
use managed_ir::method::InsnArena;
use managed_ir::value::InsnId;

use super::chains::Chains;
use super::phis::Phis;

/// Why materialization could not complete.
#[derive(Debug, Clone, Eq, PartialEq, thiserror::Error)]
pub(crate) enum MaterializeError {
    /// A chain lost every instruction, which fusion prevents.
    #[error("chain {0} has no instructions")]
    EmptyChain(usize),
    /// A phi input edge was never filled by renaming.
    #[error("phi {0:?} has an unfilled input")]
    UnfilledPhi(InsnId),
    /// A terminator references an edge identity missing from the maps.
    #[error("instruction {0:?} references missing edge {1:?}")]
    StaleEdge(InsnId, EdgeId),
    /// A control-flow graph mutation failed.
    #[error("cfg mutation failed: {0}")]
    Cfg(#[from] decomp_core::cfg::CfgError),
}

/// Builds the final one-instruction-per-block CFG, patches phi and
/// terminator edges in place, and filters the offset provenance.
pub(crate) fn materialize(
    chains: Chains,
    mirror_blocks: &[BlockId],
    mirror_edge_index: &HashMap<EdgeId, usize>,
    insns: &mut InsnArena,
    phis: &Phis,
    offsets: &mut HashMap<InsnId, u32>,
) -> Result<Cfg<InsnId>, MaterializeError> {
    let mut sequence_of: Vec<Vec<InsnId>> = Vec::with_capacity(chains.payloads.len());
    for (index, payload) in chains.payloads.iter().enumerate() {
        let mut sequence: Vec<InsnId> = mirror_blocks
            .get(index)
            .map(|block| phis.at(*block))
            .unwrap_or(&[])
            .iter()
            .map(|phi| phi.insn)
            .collect();
        sequence.extend(payload.iter().copied());
        if sequence.is_empty() {
            return Err(MaterializeError::EmptyChain(index));
        }
        sequence_of.push(sequence);
    }
    let mut first_of = vec![BlockId::default(); sequence_of.len()];
    let mut last_of = vec![BlockId::default(); sequence_of.len()];
    let mut built: Cfg<InsnId> = Cfg::new(sequence_of[0][0]);
    {
        let mut previous = built.entry();
        for insn in sequence_of[0].iter().skip(1) {
            let created = built.add_block(*insn);
            built.add_edge(previous, created, EdgeKind::Unconditional)?;
            previous = created;
        }
        first_of[0] = built.entry();
        last_of[0] = previous;
    }
    for (index, sequence) in sequence_of.iter().enumerate().skip(1) {
        let mut previous: Option<BlockId> = None;
        let mut head: Option<BlockId> = None;
        for insn in sequence {
            let created = built.add_block(*insn);
            if let Some(prev) = previous {
                built.add_edge(prev, created, EdgeKind::Unconditional)?;
            }
            head.get_or_insert(created);
            previous = Some(created);
        }
        first_of[index] = head.ok_or(MaterializeError::EmptyChain(index))?;
        last_of[index] = previous.ok_or(MaterializeError::EmptyChain(index))?;
    }
    let mut final_edges: Vec<EdgeId> = Vec::with_capacity(chains.edges.len());
    for (source, target, kind) in &chains.edges {
        let from = last_of[*source];
        let to = first_of[*target];
        final_edges.push(built.add_edge(from, to, kind.clone())?);
    }
    patch_phis(insns, phis, mirror_edge_index, &final_edges)?;
    patch_terminators(insns, &chains.merged_edges, &final_edges)?;
    let placed: HashSet<InsnId> = built
        .block_ids()
        .filter_map(|block| built.block(block).ok().map(|record| *record.payload()))
        .collect();
    offsets.retain(|insn, _| placed.contains(insn));
    Ok(built)
}

fn patch_phis(
    insns: &mut InsnArena,
    phis: &Phis,
    mirror_edge_index: &HashMap<EdgeId, usize>,
    final_edges: &[EdgeId],
) -> Result<(), MaterializeError> {
    for phi in phis.all() {
        let mut inputs = Vec::with_capacity(phi.inputs.len());
        for (mirror_edge, value) in &phi.inputs {
            let index = mirror_edge_index
                .get(mirror_edge)
                .ok_or(MaterializeError::StaleEdge(phi.insn, *mirror_edge))?;
            let value = value.ok_or(MaterializeError::UnfilledPhi(phi.insn))?;
            inputs.push((final_edges[*index], value));
        }
        insns[phi.insn] = ManagedInsn::Phi {
            result: phi.result,
            inputs,
        };
    }
    Ok(())
}

fn patch_terminators(
    insns: &mut InsnArena,
    merged_edges: &[EdgeId],
    final_edges: &[EdgeId],
) -> Result<(), MaterializeError> {
    let remap: HashMap<EdgeId, EdgeId> = merged_edges
        .iter()
        .zip(final_edges.iter())
        .map(|(merged, final_edge)| (*merged, *final_edge))
        .collect();
    for (insn_id, insn) in insns.iter_mut() {
        if matches!(insn, ManagedInsn::Phi { .. }) {
            continue;
        }
        let resolve = |edge: &EdgeId| -> Result<EdgeId, MaterializeError> {
            remap
                .get(edge)
                .copied()
                .ok_or(MaterializeError::StaleEdge(insn_id, *edge))
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
    }
    Ok(())
}
