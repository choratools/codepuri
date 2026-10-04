//! Stage 2.4b: pruned Cytron SSA construction over fused chains.
//!
//! Pipeline per method: place lifter-synthetic definitions (entry hoist or
//! use-chain insertion, see [`place`]), build the variable model from
//! register histories, compute pruned liveness over multi-def register
//! variables, place phis at live dominance frontiers, rename via iterative
//! dominator-tree DFS (rewriting every operand to its reaching definition
//! and completing per-edge phi inputs), eliminate trivial phis, then
//! materialize the published `MethodIr<SsaForm>` (one block per
//! instruction, as managed-ir's contract requires) and run the structural
//! verifier on every method. Type inference runs over the verified
//! structure afterwards; see [`crate::infer`].
//!
//! Reads with no reaching definition fall back to per-variable
//! *entry-initial* definitions hoisted to the entry chain, which preserves
//! use-dominates-def for registers that are undefined on some paths while
//! keeping Dalvik's "read an untouched register" behavior representable.

mod analysis;
mod chains;
mod initial_context;
mod liveness;
mod materialize;
#[cfg(test)]
mod performance;
mod phis;
mod place;
mod rename;
pub(crate) mod uses;
mod vars;

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Caller-owned opt-in pass timings. Completed stages remain available on error.
#[derive(Debug, Default)]
pub struct SsaTimings {
    /// Mirror construction and shared dominance analyses.
    pub analysis: Duration,
    /// Synthetic placement and dead-definition cleanup.
    pub placement: Duration,
    /// Variable model and liveness.
    pub liveness: Duration,
    /// Pruned phi placement.
    pub phi_placement: Duration,
    /// Renaming and dead-value cleanup.
    pub rename: Duration,
    /// Trivial phi cleanup.
    pub phi_cleanup: Duration,
    /// Published CFG materialization and validation.
    pub materialize: Duration,
    /// Type inference and literal interpretation.
    pub inference: Duration,
    /// Structural SSA verification.
    pub verification: Duration,
}

struct Timer<'a> {
    output: Option<&'a mut SsaTimings>,
    start: Option<Instant>,
}

impl Timer<'_> {
    fn mark(&mut self, field: impl FnOnce(&mut SsaTimings) -> &mut Duration) {
        if let (Some(output), Some(start)) = (self.output.as_deref_mut(), self.start) {
            let now = Instant::now();
            *field(output) = now.duration_since(start);
            self.start = Some(now);
        }
    }
}

use decomp_core::diagnostics::Diagnostic;
use decomp_core::ids::BlockId;
use managed_ir::method::{MethodIr, SsaForm};
use managed_ir::types::ManagedType;
use managed_ir::value::{InsnId, Value, ValueId};

use crate::infer::{self, TypeContext};
use crate::merge::MergedMethod;

/// Why SSA construction could not complete.
#[derive(Debug, Clone, Eq, PartialEq, thiserror::Error)]
pub enum SsaError {
    /// Chain fusion failed before SSA construction started.
    #[error("merge failed: {0}")]
    Merge(#[from] crate::merge::MergeError),
    /// A structural invariant was violated during construction.
    #[error("ssa construction violated an invariant: {0}")]
    Structure(String),
    /// The published CFG failed validation.
    #[error("cfg invalid: {0}")]
    Cfg(#[from] decomp_core::cfg::CfgError),
    /// The structural verifier rejected the constructed SSA form.
    #[error("ssa verification failed")]
    Verification(Vec<Diagnostic>),
}

/// Counters describing one SSA transformation.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub struct SsaStats {
    /// One-instruction blocks before fusion.
    pub lifted_blocks: usize,
    /// Fused chains after merge and dead-chain pruning.
    pub merged_blocks: usize,
    /// Redundant `goto` instructions removed by fusion.
    pub dropped_gotos: usize,
    /// Entry-unreachable chains removed before SSA.
    pub dead_blocks: usize,
    /// Instructions inside removed dead chains.
    pub dead_insns: usize,
    /// Synthetic definitions placed at the entry chain head.
    pub hoisted_defs: usize,
    /// Synthetic predicates placed at use-chain heads.
    pub inserted_defs: usize,
    /// Unplaced dead real instructions and unused synthetic definitions removed.
    pub dead_defs: usize,
    /// Phis placed before elimination.
    pub phis_placed: usize,
    /// Trivial phis eliminated.
    pub phis_eliminated: usize,
    /// Entry-initial fallback definitions created by renaming.
    pub entry_initials: usize,
    /// Final one-instruction-per-block blocks.
    pub ssa_blocks: usize,
}

/// A method in verified SSA form with DEX offset provenance.
#[derive(Debug)]
pub struct SsaMethod {
    initial_registers: HashMap<ValueId, Vec<u16>>,
    ir: MethodIr<SsaForm>,
    offsets: HashMap<InsnId, u32>,
    diagnostics: Vec<Diagnostic>,
    stats: SsaStats,
}

impl SsaMethod {
    /// Physical words attested for synthetic entry definitions in this IR only.
    pub fn initial_registers(&self) -> &HashMap<ValueId, Vec<u16>> {
        &self.initial_registers
    }
    /// Returns the published SSA-form method IR.
    pub const fn ir(&self) -> &MethodIr<SsaForm> {
        &self.ir
    }

    /// Returns the DEX code-unit offset of an instruction, if any.
    ///
    /// Phi instructions and synthetic definitions have no offset.
    pub fn offset(&self, insn: InsnId) -> Option<u32> {
        self.offsets.get(&insn).copied()
    }

    /// Returns the inference diagnostics (warnings and reported conflicts).
    pub const fn diagnostics(&self) -> &Vec<Diagnostic> {
        &self.diagnostics
    }

    /// Returns the transformation counters.
    pub const fn stats(&self) -> &SsaStats {
        &self.stats
    }
}

/// Transforms a merged method into verified SSA form without descriptor
/// context.
///
/// # Errors
/// Returns [`SsaError`] on structural failure or verifier rejection.
pub fn to_ssa(merged: MergedMethod) -> Result<SsaMethod, SsaError> {
    to_ssa_with(merged, &infer::NoTypes)
}

/// Transforms a merged method into verified SSA form and infers types.
///
/// # Errors
/// Returns [`SsaError`] on structural failure or verifier rejection.
pub fn to_ssa_with(merged: MergedMethod, ctx: &dyn TypeContext) -> Result<SsaMethod, SsaError> {
    build_ssa(merged, ctx, None)
}

/// Builds SSA with caller-controlled stage timings, without library output.
///
/// # Errors
/// Returns the same structural and verification errors as [`to_ssa_with`].
pub fn to_ssa_with_timings(
    merged: MergedMethod,
    ctx: &dyn TypeContext,
    timings: &mut SsaTimings,
) -> Result<SsaMethod, SsaError> {
    *timings = SsaTimings::default();
    build_ssa(merged, ctx, Some(timings))
}

fn build_ssa(
    merged: MergedMethod,
    ctx: &dyn TypeContext,
    timings: Option<&mut SsaTimings>,
) -> Result<SsaMethod, SsaError> {
    let mut timer = Timer {
        start: timings.as_ref().map(|_| Instant::now()),
        output: timings,
    };
    let mut stats = SsaStats {
        lifted_blocks: merged.lifted_blocks(),
        dropped_gotos: merged.dropped_gotos(),
        dead_blocks: merged.dead_blocks(),
        dead_insns: merged.dead_insns(),
        ..SsaStats::default()
    };
    let mut chains = chains::Chains::from_merged(&merged);
    stats.merged_blocks = chains.payloads.len();
    let mut insns = merged.insns;
    chains.add_invocation_entry(&mut insns);
    let mut values = merged.values;
    let reg_slots = merged.reg_slots;
    let width_claims = merged.width_claims;
    let mut offsets = merged.offsets;
    let mut seeds = HashMap::new();
    for slot in reg_slots.values() {
        for value in slot.values() {
            if let Some(record) = values.get(*value)
                && matches!(
                    insns.get(record.def()),
                    Some(managed_ir::insn::ManagedInsn::Opaque {
                        family_tag: 0x8001,
                        ..
                    })
                )
            {
                let wide = width_claims.get(value) == Some(&managed_ir::types::ValueCategory::Two);
                if let Some(ty) = ctx.initial_register_type(slot.index(), wide) {
                    seeds.insert(*value, ty.clone());
                    values[*value] = Value::new(record.def(), ty);
                }
            }
        }
    }

    let mirror = chains.analysis_cfg();
    let mirror_blocks: Vec<BlockId> = mirror.block_ids().collect();
    let analysis = analysis::Analysis::new(&mirror);
    timer.mark(|t| &mut t.analysis);

    let placement = place::place_synthetics(&analysis, &mut chains, &insns, &offsets);
    stats.hoisted_defs = placement.hoisted;
    stats.inserted_defs = placement.inserted;
    stats.dead_defs = placement.dead.len();
    let mut dead_values = Vec::new();
    for insn in &placement.dead {
        if let Some(record) = insns.get(*insn) {
            uses::for_each_definition(record, |value| {
                dead_values.push(value);
            });
        }
        insns.remove(*insn);
        offsets.remove(insn);
    }
    timer.mark(|t| &mut t.placement);
    let mut vars = vars::Variables::build(&reg_slots, &values, &width_claims);
    let candidates = vars.multi_def_vars();
    let live = liveness::compute(&mirror, &chains, &insns, &vars, &candidates);
    timer.mark(|t| &mut t.liveness);

    let mut phis = phis::place(
        &analysis,
        &chains,
        &mut insns,
        &mut values,
        &vars,
        &candidates,
        &live,
    );
    stats.phis_placed = phis.len();
    for phi in phis.all() {
        vars.bind(phi.result, phi.var);
    }
    timer.mark(|t| &mut t.phi_placement);

    let renamed = rename::rename(
        &analysis,
        &mut chains,
        &mut insns,
        &mut values,
        &vars,
        &mut phis,
    );
    // File-order lifter operands can still name an unreachable definition.
    // Keep its register lineage until renaming resolves the live reaching
    // value; only then remove the orphaned semantic value.
    for value in dead_values {
        values.remove(value);
    }
    let reg_slots = {
        let mut scrubbed = managed_ir::method::RegisterSlotArena::with_key();
        for (_, slot) in reg_slots.iter() {
            let mut copy = managed_ir::method::RegisterSlot::new(slot.index());
            for value in slot.values() {
                if values.get(*value).is_some() {
                    copy.append(*value);
                }
            }
            for (var, value) in &renamed.entry_initials {
                if vars.register(*var) == Some(slot.index()) {
                    if let Some(ty) = ctx.initial_register_type(slot.index(), vars.wide(*var)) {
                        seeds.insert(*value, ty.clone());
                        let def = values[*value].def();
                        values[*value] = Value::new(def, ty);
                    }
                    copy.append(*value);
                } else if vars.wide(*var)
                    && vars.register(*var).and_then(|r| r.checked_add(1)) == Some(slot.index())
                {
                    copy.append(*value);
                }
            }
            scrubbed.insert(copy);
        }
        scrubbed
    };
    stats.entry_initials = renamed.entry_initials.len();
    let mut initial_insns: Vec<(u32, InsnId)> = renamed
        .entry_initials
        .iter()
        .map(|(var, value)| {
            (
                *var,
                values
                    .get(*value)
                    .map(|record| record.def())
                    .unwrap_or_default(),
            )
        })
        .collect();
    initial_insns.sort_unstable_by_key(|(var, _)| *var);
    let initial_ids: Vec<InsnId> = initial_insns.into_iter().map(|(_, insn)| insn).collect();
    chains.payloads[0].splice(0..0, initial_ids);
    timer.mark(|t| &mut t.rename);

    stats.phis_eliminated = phis::eliminate(&analysis, &chains, &mut insns, &mut values, &mut phis);
    timer.mark(|t| &mut t.phi_cleanup);

    let mirror_edge_index = chains::edge_index_map(&mirror, chains.edges.len());
    let cfg = materialize::materialize(
        chains,
        &mirror_blocks,
        &mirror_edge_index,
        &mut insns,
        &phis,
        &mut offsets,
    )
    .map_err(|error| SsaError::Structure(error.to_string()))?;
    stats.ssa_blocks = cfg.block_count();
    cfg.validate()?;
    timer.mark(|t| &mut t.materialize);

    let mut types: HashMap<ValueId, ManagedType> = values
        .iter()
        .map(|(id, record)| (id, record.ty().clone()))
        .collect();
    let seeded_context = initial_context::InitialContext { base: ctx, seeds };
    let diagnostics = infer::infer_types(&cfg, &insns, &mut types, &offsets, &seeded_context);

    // Publish the inferred interpretation of raw DEX constant bits. This is
    // a literal representation change, never a numeric conversion or cast.
    for (_, insn) in insns.iter_mut() {
        if let managed_ir::insn::ManagedInsn::Const { result, value } = insn {
            use managed_ir::ops::Constant;
            match (types.get(result), *value) {
                (Some(ManagedType::Boolean), Constant::Int(0)) => *value = Constant::Boolean(false),
                (Some(ManagedType::Boolean), Constant::Int(1)) => *value = Constant::Boolean(true),
                (Some(ManagedType::Float), Constant::Int(bits)) => {
                    *value = Constant::FloatBits(bits as u32)
                }
                (Some(ManagedType::Float), Constant::Zero) => *value = Constant::FloatBits(0),
                (Some(ManagedType::Double), Constant::Long(bits)) => {
                    *value = Constant::DoubleBits(bits as u64)
                }
                _ => {}
            }
        }
    }

    let updates: Vec<(ValueId, ManagedType)> =
        types.iter().map(|(id, ty)| (*id, ty.clone())).collect();
    for (id, ty) in updates {
        if let Some(record) = values.get(id) {
            let def = record.def();
            values[id] = Value::new(def, ty);
        }
    }
    let ir = MethodIr::<SsaForm>::from_parts_with_registers(cfg, insns, values, reg_slots);
    timer.mark(|t| &mut t.inference);
    let findings = managed_ir::verify::SsaVerifier::new().verify(&ir);
    timer.mark(|t| &mut t.verification);
    if !findings.is_empty() {
        if std::env::var_os("DUMP_SSA_FAIL").is_some() {
            eprintln!("TYPE DIAGNOSTICS {diagnostics:?}");
            for (id, record) in ir.values().iter() {
                eprintln!(
                    "VALUE {id:?} = {:?} def={:?} slots={:?}",
                    record.ty(),
                    record.def(),
                    ir.register_slots()
                        .iter()
                        .filter(|(_, slot)| slot.values().contains(&id))
                        .map(|(_, slot)| slot.index())
                        .collect::<Vec<_>>()
                );
            }
            for (id, record) in ir.instructions() {
                eprintln!("INSN {id:?} off={:?} = {record:?}", offsets.get(&id));
            }
        }
        return Err(SsaError::Verification(findings));
    }
    let mut initial_registers = HashMap::<ValueId, Vec<u16>>::new();
    for slot in ir.register_slots().values() {
        for value in slot.values() {
            if let Ok(record) = ir.value(*value)
                && matches!(
                    ir.instruction(record.def()),
                    Ok(managed_ir::insn::ManagedInsn::Opaque {
                        family_tag: 0x8001 | 0x8003,
                        ..
                    })
                )
            {
                initial_registers
                    .entry(*value)
                    .or_default()
                    .push(slot.index());
            }
        }
    }
    for words in initial_registers.values_mut() {
        words.sort_unstable();
        words.dedup();
    }
    Ok(SsaMethod {
        initial_registers,
        ir,
        offsets,
        diagnostics,
        stats,
    })
}
