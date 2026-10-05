//! Structured, effect-preserving branch/Phi lowering, independent of rendering.
//!
//! Integration must hoist declarations for `locals` (not their initializers),
//! materialize each ordinary instruction exactly once at its planned position,
//! and render every `Copies` by first saving ALL sources into fresh temporaries
//! and then assigning ALL destinations. Never inline an SSA definition into a
//! condition or copy. Parameter values are supplied explicitly to `plan`.

use decomp_core::{
    cfg::EdgeKind,
    ids::{BlockId, EdgeId},
};
use decomp_structure::{Region, StructureError};
use managed_ir::{
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
    types::ManagedType,
    value::{InsnId, ValueId},
};
use std::collections::{HashMap, HashSet};

/// A simultaneous assignment of stable SSA locals on an original CFG edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Copy {
    /// Phi result's stable local.
    pub destination: ValueId,
    /// Incoming value's stable local, read before any assignment in the batch.
    pub source: ValueId,
}

/// A concrete parallel-copy schedule. Slots are batch-local temporary identities;
/// the renderer must allocate a fresh Java local for each slot in every batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyAction {
    /// Declare a temporary with the source local's type and save its value.
    Snapshot {
        /// Batch-local temporary index.
        slot: usize,
        /// Stable source local.
        source: ValueId,
    },
    /// Assign a Phi local from a previously saved temporary.
    Assign {
        /// Stable destination local.
        destination: ValueId,
        /// Batch-local temporary index.
        slot: usize,
    },
}

/// Lower simultaneous copies to ordered snapshot/assignment actions. Saving all
/// sources first preserves swaps, fan-out and arbitrary cycles without moving
/// or duplicating the effects that originally produced those source values.
pub fn schedule_copies(copies: &[Copy]) -> Vec<CopyAction> {
    copies
        .iter()
        .enumerate()
        .map(|(slot, c)| CopyAction::Snapshot {
            slot,
            source: c.source,
        })
        .chain(
            copies
                .iter()
                .enumerate()
                .map(|(slot, c)| CopyAction::Assign {
                    destination: c.destination,
                    slot,
                }),
        )
        .collect()
}

/// Ordered operations; braces around arms are required when rendering Java.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Integral dispatch with edge-specific entry copies and grouped bodies.
    Switch {
        /// Validated original dispatch.
        dispatch: crate::switching::StepSwitch,
        /// Arm operations and continuation status in dispatch arm order.
        bodies: Vec<(Vec<Step>, bool)>,
    },
    /// Pretest loop with effectful evaluation on every iteration.
    While {
        /// Instructions evaluated before each test.
        evaluation: Vec<Step>,
        /// Materialized Boolean condition.
        condition: ValueId,
        /// Body edge polarity.
        body_on_true: bool,
        /// Body operations and backedge copies.
        body_steps: Vec<Step>,
        /// Normal-exit edge copies.
        exit_steps: Vec<Step>,
    },
    /// Delegate to the existing typed instruction emitter, using assignments
    /// into hoisted locals instead of block-scoped declarations.
    Instruction(InsnId),
    /// Snapshot all sources, then assign all destinations, even for cycles.
    Copies {
        /// Original edge selecting this batch.
        edge: EdgeId,
        /// Assignments in deterministic cluster order.
        copies: Vec<Copy>,
    },
    /// Read the already materialized Boolean local once. Edge copies occur
    /// inside the selected arm, including an otherwise empty arm.
    If {
        /// Already defined Boolean value.
        condition: ValueId,
        /// True-edge operations.
        then_steps: Vec<Step>,
        /// False-edge operations.
        else_steps: Vec<Step>,
    },
}

/// Complete plan, with deterministic arena-order local allocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchPlan {
    /// Every CFG-defined value, including Phi results. Bindings are excluded.
    pub locals: Vec<ValueId>,
    /// Complete ordered body, excluding local declarations.
    pub steps: Vec<Step>,
}

/// No partial plan is returned for unsupported or inconsistent input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// Unsupported or invalid CFG, including unsupported loops and exceptions.
    Structure(StructureError),
    /// The supplied Region is not the canonical structure of this graph.
    RegionMismatch,
    /// Stale identity, duplicate placement/definition, or invalid binding.
    InvalidIr(String),
    /// Instruction and outgoing edge semantics disagree.
    InvalidBranch(BlockId),
    /// Phi inputs do not exactly cover the cluster head's incoming edges.
    InvalidPhi(BlockId),
    /// Value is not definitely defined on this execution path.
    Unavailable(ValueId),
    /// Non-Boolean condition or mismatched Phi source/result types.
    TypeMismatch(ValueId),
    /// Unsupported terminal operation. Other unsupported instructions are
    /// rejected by the parent's ordinary typed instruction renderer.
    UnsupportedInstruction(InsnId),
}
impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unsupported Java branch plan: {self:?}")
    }
}
impl std::error::Error for PlanError {}

/// Validate and lower a caller-supplied structured Region. Unsupported cycles, switches and
/// exception edges retain the structurer's typed errors. The parent still owns
/// resolver/type compatibility checks for ordinary instructions and bindings.
pub fn plan(
    ir: &MethodIr<SsaForm>,
    region: &Region,
    bindings: &[ValueId],
) -> Result<BranchPlan, PlanError> {
    plan_checked(ir, region, bindings, &HashSet::new())
}
/// Structured plan with parent-proven atomic phi families.
pub fn plan_checked(
    ir: &MethodIr<SsaForm>,
    region: &Region,
    bindings: &[ValueId],
    declarations: &HashSet<ValueId>,
) -> Result<BranchPlan, PlanError> {
    let structured = decomp_structure::structure(ir.cfg()).map_err(PlanError::Structure)?;
    if &structured.root != region {
        return Err(PlanError::RegionMismatch);
    }
    lower_canonical(ir, region, bindings, declarations)
}

// Construct and consume the canonical region against the same immutable IR.
// Unlike the public caller-supplied-region API, there is no snapshot to attest
// by rebuilding the graph a second time. SSA/edge/availability checks remain.
pub(crate) fn plan_canonical_checked(
    ir: &MethodIr<SsaForm>,
    bindings: &[ValueId],
    declarations: &HashSet<ValueId>,
) -> Result<BranchPlan, PlanError> {
    let structured = decomp_structure::structure(ir.cfg()).map_err(PlanError::Structure)?;
    lower_canonical(ir, &structured.root, bindings, declarations)
}

fn lower_canonical(
    ir: &MethodIr<SsaForm>,
    region: &Region,
    bindings: &[ValueId],
    declarations: &HashSet<ValueId>,
) -> Result<BranchPlan, PlanError> {
    let (p, definitions, mut available) = prepare_checked(ir, bindings, declarations)?;
    let (steps, _) = p.lower(region, &mut available, &mut Vec::new())?;
    Ok(finish(ir, definitions, steps))
}

/// Production lowering for acyclic integral switches and nested conditionals.
pub fn plan_extended(
    ir: &MethodIr<SsaForm>,
    region: &decomp_structure::ExtendedRegion,
    bindings: &[ValueId],
) -> Result<BranchPlan, PlanError> {
    plan_extended_checked(ir, region, bindings, &HashSet::new())
}
/// Switch plan with parent-proven atomic phi families.
pub fn plan_extended_checked(
    ir: &MethodIr<SsaForm>,
    region: &decomp_structure::ExtendedRegion,
    bindings: &[ValueId],
    declarations: &HashSet<ValueId>,
) -> Result<BranchPlan, PlanError> {
    let structured =
        decomp_structure::structure_extended(ir.cfg()).map_err(PlanError::Structure)?;
    if &structured.root != region {
        return Err(PlanError::RegionMismatch);
    }
    lower_extended_canonical(ir, region, bindings, declarations)
}

pub(crate) fn plan_extended_canonical_checked(
    ir: &MethodIr<SsaForm>,
    bindings: &[ValueId],
    declarations: &HashSet<ValueId>,
) -> Result<BranchPlan, PlanError> {
    let structured =
        decomp_structure::structure_extended(ir.cfg()).map_err(PlanError::Structure)?;
    lower_extended_canonical(ir, &structured.root, bindings, declarations)
}

fn lower_extended_canonical(
    ir: &MethodIr<SsaForm>,
    region: &decomp_structure::ExtendedRegion,
    bindings: &[ValueId],
    declarations: &HashSet<ValueId>,
) -> Result<BranchPlan, PlanError> {
    let (p, definitions, mut available) = prepare_checked(ir, bindings, declarations)?;
    let (steps, _) = p.lower_extended(region, &mut available)?;
    Ok(finish(ir, definitions, steps))
}

fn finish(ir: &MethodIr<SsaForm>, definitions: HashSet<ValueId>, steps: Vec<Step>) -> BranchPlan {
    let locals = ir
        .values()
        .keys()
        .filter(|v| definitions.contains(v) && hoistable_definition(ir, *v))
        .collect();
    BranchPlan { locals, steps }
}

type Prepared<'a> = (Planner<'a>, HashSet<ValueId>, HashSet<ValueId>);
/// Validated single-placement acyclic plan with original edge-copy batches.
pub struct ForwardPlan {
    /// Method-scope declarations, without initializers.
    pub locals: Vec<ValueId>,
    /// Canonical original control-flow snapshot.
    pub regions: decomp_structure::extended::ForwardRegions,
    /// Edge-specific simultaneous copies.
    pub copies: HashMap<EdgeId, Vec<Copy>>,
}

/// Validate definitions and availability on every original forward path.
pub fn plan_forward(
    ir: &MethodIr<SsaForm>,
    bindings: &[ValueId],
) -> Result<ForwardPlan, PlanError> {
    plan_forward_checked(ir, bindings, &HashSet::new())
}

/// The parent has checked whole-family declarations and original input coercions.
pub fn plan_forward_checked(
    ir: &MethodIr<SsaForm>,
    bindings: &[ValueId],
    declarations: &HashSet<ValueId>,
) -> Result<ForwardPlan, PlanError> {
    use decomp_structure::extended::{FlowStructuredCfg, structure_flow};
    let FlowStructuredCfg::Shared(regions) =
        structure_flow(ir.cfg()).map_err(PlanError::Structure)?
    else {
        return Err(PlanError::RegionMismatch);
    };
    let (planner, definitions, initial) = prepare_checked(ir, bindings, declarations)?;
    let mut incoming: HashMap<BlockId, HashSet<ValueId>> = HashMap::new();
    incoming.insert(regions.entry, initial);
    for block in &regions.blocks {
        let mut env = incoming
            .remove(&block.block)
            .ok_or(PlanError::InvalidBranch(block.block))?;
        let (_, insn) = planner.insn(block.block)?;
        if let ManagedInsn::Phi { result, .. } = insn {
            require(&env, *result)?;
        } else {
            for value in reads(insn) {
                require(&env, value)?;
            }
            env.extend(defined(insn));
        }
        for &edge in &block.outgoing {
            let mut selected = env.clone();
            planner.edge(edge, &mut selected, &mut Vec::new(), &mut Vec::new())?;
            let target = ir.cfg().edge(edge).map_err(invalid)?.target();
            incoming
                .entry(target)
                .and_modify(|values| values.retain(|v| selected.contains(v)))
                .or_insert(selected);
        }
    }
    Ok(ForwardPlan {
        locals: finish(ir, definitions, Vec::new()).locals,
        regions,
        copies: planner.copies,
    })
}

/// Exact ordinary-CFG plan with SSA dominance and original edge-copy proof.
/// Concrete locals may be initialized for Java definite assignment only after
/// this validation; those initial values are never read on a valid CFG path.
pub struct DispatchPlan {
    /// Original CFG-defined values, excluding genuine incoming bindings.
    pub locals: Vec<ValueId>,
    /// Canonical state schedule preserving every original normal edge.
    pub regions: decomp_structure::dispatch::DispatchPlan,
    /// Original simultaneous Phi transfers.
    pub copies: HashMap<EdgeId, Vec<Copy>>,
}

/// Validate arbitrary normal flow without requiring a structured region tree.
pub fn plan_dispatch_checked(
    ir: &MethodIr<SsaForm>,
    bindings: &[ValueId],
    declarations: &HashSet<ValueId>,
) -> Result<DispatchPlan, PlanError> {
    let regions = decomp_structure::dispatch::plan(ir.cfg()).map_err(PlanError::Structure)?;
    let diagnostics = managed_ir::verify::SsaVerifier::new().verify(ir);
    if !diagnostics.is_empty() {
        return Err(PlanError::InvalidIr(format!(
            "dispatch SSA: {diagnostics:?}"
        )));
    }
    let (planner, definitions, initial) = prepare_checked(ir, bindings, declarations)?;
    let materializable: HashSet<_> = definitions.union(&initial).copied().collect();
    for state in &regions.states {
        let (id, insn) = planner.insn(state.block)?;
        if matches!(
            insn,
            ManagedInsn::MonitorEnter { .. } | ManagedInsn::MonitorExit { .. }
        ) {
            return Err(PlanError::UnsupportedInstruction(id));
        }
        for value in reads(insn) {
            require(&materializable, value)?;
        }
    }
    for copies in planner.copies.values() {
        for copy in copies {
            require(&materializable, copy.source)?;
            require(&materializable, copy.destination)?;
        }
    }
    Ok(DispatchPlan {
        locals: finish(ir, definitions, Vec::new()).locals,
        regions,
        copies: planner.copies,
    })
}
/// Canonically validate and lower general natural-loop transfers.
pub fn plan_control(
    ir: &MethodIr<SsaForm>,
    bindings: &[ValueId],
) -> Result<(Vec<ValueId>, Vec<crate::loop_control::ControlStep>), PlanError> {
    let structured =
        crate::loop_control_regions::structure(ir.cfg()).map_err(PlanError::Structure)?;
    let (mut planner, definitions, mut available) = prepare(ir, bindings)?;
    let (steps, _) = crate::loop_control::lower(&structured.root, &mut planner, &mut available)?;
    Ok((finish(ir, definitions, Vec::new()).locals, steps))
}

impl crate::loop_control::Lowering for Planner<'_> {
    type Error = PlanError;
    fn block(
        &mut self,
        block: BlockId,
        env: &mut HashSet<ValueId>,
    ) -> Result<(Vec<Step>, bool), PlanError> {
        let (id, insn) = self.insn(block)?;
        let mut steps = Vec::new();
        match insn {
            ManagedInsn::Phi { result, .. } => require(env, *result)?,
            ManagedInsn::Branch {
                condition: None, ..
            } => {}
            _ => {
                for value in reads(insn) {
                    require(env, value)?;
                }
                steps.push(Step::Instruction(id));
                env.extend(defined(insn));
            }
        }
        Ok((
            steps,
            !matches!(insn, ManagedInsn::Return { .. } | ManagedInsn::Throw { .. }),
        ))
    }
    fn condition(&mut self, block: BlockId, env: &HashSet<ValueId>) -> Result<ValueId, PlanError> {
        let (_, insn) = self.insn(block)?;
        let ManagedInsn::Branch {
            condition: Some(value),
            ..
        } = insn
        else {
            return Err(PlanError::InvalidBranch(block));
        };
        require(env, *value)?;
        Ok(*value)
    }
    fn edge(&mut self, edge: EdgeId, env: &mut HashSet<ValueId>) -> Result<Vec<Step>, PlanError> {
        let mut steps = Vec::new();
        Planner::edge(self, edge, env, &mut Vec::new(), &mut steps)?;
        Ok(steps)
    }
    fn outgoing(&mut self, block: BlockId) -> Result<Option<EdgeId>, PlanError> {
        let outgoing = self.ir.cfg().block(block).map_err(invalid)?.outgoing();
        if outgoing.len() > 1 {
            return Err(PlanError::InvalidBranch(block));
        }
        Ok(outgoing.first().copied())
    }
}

fn prepare<'a>(ir: &'a MethodIr<SsaForm>, bindings: &[ValueId]) -> Result<Prepared<'a>, PlanError> {
    prepare_checked(ir, bindings, &HashSet::new())
}

fn prepare_checked<'a>(
    ir: &'a MethodIr<SsaForm>,
    bindings: &[ValueId],
    declarations: &HashSet<ValueId>,
) -> Result<Prepared<'a>, PlanError> {
    // The implicit external entry has no original predecessor edge selecting
    // a Phi input. Backedges cannot initialize its first execution.
    let entry_id = *ir.cfg().block(ir.cfg().entry()).map_err(invalid)?.payload();
    if matches!(
        ir.instruction(entry_id).map_err(invalid)?,
        ManagedInsn::Phi { .. }
    ) {
        return Err(PlanError::InvalidPhi(ir.cfg().entry()));
    }
    let mut p = Planner {
        ir,
        copies: HashMap::new(),
    };
    let mut placed = HashSet::new();
    let mut definitions = HashSet::new();
    for block in ir.cfg().block_ids() {
        let id = *ir.cfg().block(block).map_err(invalid)?.payload();
        if !placed.insert(id) {
            return Err(PlanError::InvalidIr(
                "duplicate instruction placement".into(),
            ));
        }
        let insn = ir.instruction(id).map_err(invalid)?;
        for v in defined(insn) {
            if !definitions.insert(v) || ir.value(v).map_err(invalid)?.def() != id {
                return Err(PlanError::InvalidIr("invalid definition identity".into()));
            }
        }
        p.validate_block(block, insn)?;
    }
    let mut available = HashSet::new();
    for &v in bindings {
        let value = ir.value(v).map_err(invalid)?;
        let synthetic = matches!(ir.instruction(value.def()).map_err(invalid)?,
            ManagedInsn::Opaque { family_tag: 0x8001 | 0x8003, reads, writes, effects }
            if reads.is_empty() && writes.as_slice() == [v] && *effects == managed_ir::effects::EffectFlags::NONE);
        if (definitions.contains(&v) && !synthetic) || !available.insert(v) {
            return Err(PlanError::InvalidIr(
                "duplicate or CFG-defined binding".into(),
            ));
        }
        // Genuine synthetic incoming definitions can be materialized as CFG
        // nodes by the frontend, but are not Java local redefinitions.
        if synthetic {
            definitions.remove(&v);
        }
    }
    p.collect_phis(declarations)?;
    Ok((p, definitions, available))
}

fn invalid(e: impl std::fmt::Display) -> PlanError {
    PlanError::InvalidIr(e.to_string())
}

fn hoistable_definition(ir: &MethodIr<SsaForm>, value: ValueId) -> bool {
    // A stable source slot can outlive a labelled block. Allocation slots are
    // kept unreadable by the emitter until the exact constructor initializes it.
    ir.value(value).is_ok()
}

struct Planner<'a> {
    ir: &'a MethodIr<SsaForm>,
    copies: HashMap<EdgeId, Vec<Copy>>,
}
impl Planner<'_> {
    fn lower_extended(
        &self,
        region: &decomp_structure::ExtendedRegion,
        available: &mut HashSet<ValueId>,
    ) -> Result<(Vec<Step>, bool), PlanError> {
        use decomp_structure::ExtendedRegion as R;
        match region {
            R::Block(b) => self.lower(&Region::Block(*b), available, &mut Vec::new()),
            R::Sequence(rs) => {
                let mut steps = Vec::new();
                for r in rs {
                    let (child, next) = self.lower_extended(r, available)?;
                    steps.extend(child);
                    if !next {
                        return Ok((steps, false));
                    }
                }
                Ok((steps, true))
            }
            R::If {
                header,
                then_region,
                else_region,
                ..
            } => {
                let (_, insn) = self.insn(*header)?;
                let ManagedInsn::Branch {
                    condition: Some(condition),
                    then_edge,
                    else_edge: Some(else_edge),
                } = insn
                else {
                    return Err(PlanError::InvalidBranch(*header));
                };
                require(available, *condition)?;
                let before = available.clone();
                let mut arms = Vec::new();
                let mut continuing = Vec::new();
                for (edge, arm) in [(*then_edge, then_region), (*else_edge, else_region)] {
                    let mut env = before.clone();
                    let mut steps = Vec::new();
                    self.edge(edge, &mut env, &mut Vec::new(), &mut steps)?;
                    let (child, next) = self.lower_extended(arm, &mut env)?;
                    steps.extend(child);
                    arms.push(steps);
                    if next {
                        continuing.push(env);
                    }
                }
                let next = merge_continuing(available, continuing);
                let else_steps = arms.pop().ok_or(PlanError::InvalidBranch(*header))?;
                let then_steps = arms.pop().ok_or(PlanError::InvalidBranch(*header))?;
                Ok((
                    vec![Step::If {
                        condition: *condition,
                        then_steps,
                        else_steps,
                    }],
                    next,
                ))
            }
            R::Switch { .. } => {
                let dispatch = crate::switching::plan(self.ir, region, |v| available.contains(&v))
                    .map_err(|err| PlanError::InvalidIr(err.to_string()))?;
                let before = available.clone();
                let mut bodies = Vec::new();
                let mut continuing = Vec::new();
                for arm in &dispatch.arms {
                    let mut entries = Vec::new();
                    for entry in &dispatch.dispatch {
                        if self.ir.cfg().edge(entry.edge).map_err(invalid)?.target() == arm.target {
                            let mut env = before.clone();
                            for copy in &entry.copies {
                                env.insert(copy.destination);
                            }
                            entries.push(env);
                        }
                    }
                    let mut env = before.clone();
                    if !merge_continuing(&mut env, entries) {
                        return Err(PlanError::InvalidIr(
                            "switch arm has no dispatch edge".into(),
                        ));
                    }
                    let (steps, next) = self.lower_extended(&arm.region, &mut env)?;
                    bodies.push((steps, next));
                    if next {
                        continuing.push(env);
                    }
                }
                let next = merge_continuing(available, continuing);
                Ok((vec![Step::Switch { dispatch, bodies }], next))
            }
        }
    }
    fn insn(&self, block: BlockId) -> Result<(InsnId, &ManagedInsn), PlanError> {
        let id = *self.ir.cfg().block(block).map_err(invalid)?.payload();
        Ok((id, self.ir.instruction(id).map_err(invalid)?))
    }

    fn validate_block(&self, block: BlockId, insn: &ManagedInsn) -> Result<(), PlanError> {
        let cfg = self.ir.cfg();
        let outgoing = cfg.block(block).map_err(invalid)?.outgoing();
        match insn {
            ManagedInsn::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                let valid = if let Some(condition) = condition {
                    if self.ir.value(*condition).map_err(invalid)?.ty() != &ManagedType::Boolean {
                        return Err(PlanError::TypeMismatch(*condition));
                    }
                    outgoing.len() == 2
                        && outgoing.contains(then_edge)
                        && else_edge.is_some_and(|e| outgoing.contains(&e) && e != *then_edge)
                        && cfg.edge(*then_edge).map_err(invalid)?.kind() == &EdgeKind::BranchTrue
                        && cfg
                            .edge(else_edge.ok_or(PlanError::InvalidBranch(block))?)
                            .map_err(invalid)?
                            .kind()
                            == &EdgeKind::BranchFalse
                } else {
                    outgoing == [*then_edge]
                        && else_edge.is_none()
                        && cfg.edge(*then_edge).map_err(invalid)?.kind() == &EdgeKind::Unconditional
                };
                if !valid {
                    return Err(PlanError::InvalidBranch(block));
                }
            }
            ManagedInsn::Return { .. } | ManagedInsn::Throw { .. } => {
                if !outgoing.is_empty() {
                    return Err(PlanError::InvalidBranch(block));
                }
            }
            ManagedInsn::Switch {
                key,
                default_edge,
                cases,
            } => {
                if self.ir.value(*key).map_err(invalid)?.ty() != &ManagedType::Int {
                    return Err(PlanError::TypeMismatch(*key));
                }
                let mut seen = HashSet::new();
                let mut labels = HashSet::new();
                for (label, id) in cases
                    .iter()
                    .map(|(v, e)| (Some(*v), *e))
                    .chain(std::iter::once((None, *default_edge)))
                {
                    let edge = cfg.edge(id).map_err(invalid)?;
                    let valid = match label {
                        Some(v) => {
                            i32::try_from(v).is_ok()
                                && labels.insert(v)
                                && edge.kind() == &EdgeKind::Switch(v as u64)
                        }
                        None => edge.kind() == &EdgeKind::Unconditional,
                    };
                    if !valid || edge.source() != block || !seen.insert(id) {
                        return Err(PlanError::InvalidBranch(block));
                    }
                }
                if seen.len() != outgoing.len() {
                    return Err(PlanError::InvalidBranch(block));
                }
            }
            _ => {
                if outgoing.len() != 1
                    || cfg.edge(outgoing[0]).map_err(invalid)?.kind() != &EdgeKind::Unconditional
                {
                    return Err(PlanError::InvalidBranch(block));
                }
            }
        }
        Ok(())
    }

    fn collect_phis(&mut self, declarations: &HashSet<ValueId>) -> Result<(), PlanError> {
        let cfg = self.ir.cfg();
        let mut processed = HashSet::new();
        for block in cfg.block_ids() {
            if processed.contains(&block) {
                continue;
            }
            if !matches!(self.insn(block)?.1, ManagedInsn::Phi { .. }) {
                continue;
            }
            // Walk back to the cluster head independent of arena/block order.
            let mut head = block;
            let mut walked = HashSet::new();
            loop {
                if !walked.insert(head) {
                    return Err(PlanError::InvalidPhi(head));
                }
                let incoming = cfg.block(head).map_err(invalid)?.incoming();
                if incoming.len() != 1 {
                    break;
                }
                let edge = cfg.edge(incoming[0]).map_err(invalid)?;
                let source = cfg.block(edge.source()).map_err(invalid)?;
                if edge.kind() != &EdgeKind::Unconditional
                    || source.outgoing().len() != 1
                    || !matches!(self.insn(edge.source())?.1, ManagedInsn::Phi { .. })
                {
                    break;
                }
                head = edge.source();
            }
            let incoming = cfg.block(head).map_err(invalid)?.incoming();
            let incoming_set: HashSet<_> = incoming.iter().copied().collect();
            if incoming.is_empty() {
                return Err(PlanError::InvalidPhi(head));
            }
            let mut member = head;
            while let ManagedInsn::Phi { result, inputs } = self
                .ir
                .instruction(*cfg.block(member).map_err(invalid)?.payload())
                .map_err(invalid)?
            {
                if !processed.insert(member) {
                    return Err(PlanError::InvalidPhi(member));
                }
                let result = *result;
                let mut seen = HashSet::new();
                if inputs.len() != incoming.len() {
                    return Err(PlanError::InvalidPhi(member));
                }
                for &(edge, source) in inputs {
                    if !incoming_set.contains(&edge) || !seen.insert(edge) {
                        return Err(PlanError::InvalidPhi(member));
                    }
                    let destination_ty = self.ir.value(result).map_err(invalid)?.ty();
                    if !declarations.contains(&result)
                        && destination_ty != self.ir.value(source).map_err(invalid)?.ty()
                    {
                        return Err(PlanError::TypeMismatch(result));
                    }
                    self.copies.entry(edge).or_default().push(Copy {
                        destination: result,
                        source,
                    });
                }
                let outgoing = cfg.block(member).map_err(invalid)?.outgoing();
                if outgoing.len() != 1 {
                    break;
                }
                let edge = cfg.edge(outgoing[0]).map_err(invalid)?;
                let target = cfg.block(edge.target()).map_err(invalid)?;
                if edge.kind() != &EdgeKind::Unconditional || target.incoming().len() != 1 {
                    break;
                }
                member = edge.target();
            }
        }
        Ok(())
    }

    fn edge(
        &self,
        edge: EdgeId,
        available: &mut HashSet<ValueId>,
        added: &mut Vec<ValueId>,
        steps: &mut Vec<Step>,
    ) -> Result<(), PlanError> {
        if let Some(copies) = self.copies.get(&edge) {
            // Read against the PRE-copy environment, never the partial result.
            for copy in copies {
                require(available, copy.source)?;
            }
            for copy in copies {
                if available.insert(copy.destination) {
                    added.push(copy.destination);
                }
            }
            steps.push(Step::Copies {
                edge,
                copies: copies.clone(),
            });
        }
        Ok(())
    }

    fn lower(
        &self,
        region: &Region,
        available: &mut HashSet<ValueId>,
        added: &mut Vec<ValueId>,
    ) -> Result<(Vec<Step>, bool), PlanError> {
        let mut steps = Vec::new();
        match region {
            Region::While {
                evaluation,
                condition: block,
                body_on_true,
                body,
                ..
            } => {
                let (evaluation_steps, next) = self.lower(evaluation, available, added)?;
                if !next {
                    return Err(PlanError::InvalidBranch(*block));
                }
                let (_, insn) = self.insn(*block)?;
                let ManagedInsn::Branch {
                    condition: Some(condition),
                    then_edge,
                    else_edge: Some(else_edge),
                } = insn
                else {
                    return Err(PlanError::InvalidBranch(*block));
                };
                require(available, *condition)?;
                let (body_edge, exit_edge) = if *body_on_true {
                    (*then_edge, *else_edge)
                } else {
                    (*else_edge, *then_edge)
                };
                // The body may execute zero times. Only the header environment
                // and the normal-exit copies are available after this loop.
                let checkpoint = added.len();
                let mut body_steps = Vec::new();
                self.edge(body_edge, available, added, &mut body_steps)?;
                let (lowered_body, next) = self.lower(body, available, added)?;
                if !next {
                    return Err(PlanError::InvalidBranch(*block));
                }
                body_steps.extend(lowered_body);
                for value in added.split_off(checkpoint) {
                    available.remove(&value);
                }
                let mut exit_steps = Vec::new();
                self.edge(exit_edge, available, added, &mut exit_steps)?;
                steps.push(Step::While {
                    evaluation: evaluation_steps,
                    condition: *condition,
                    body_on_true: *body_on_true,
                    body_steps,
                    exit_steps,
                });
            }
            Region::Sequence(regions) => {
                for child in regions {
                    let (child_steps, next) = self.lower(child, available, added)?;
                    steps.extend(child_steps);
                    if !next {
                        return Ok((steps, false));
                    }
                }
            }
            Region::Block(block) => {
                let (id, insn) = self.insn(*block)?;
                match insn {
                    ManagedInsn::Phi { result, .. } => require(available, *result)?,
                    ManagedInsn::Branch {
                        condition: None, ..
                    } => {}
                    _ => {
                        for v in reads(insn) {
                            require(available, v)?;
                        }
                        steps.push(Step::Instruction(id));
                        for value in defined(insn) {
                            if available.insert(value) {
                                added.push(value);
                            }
                        }
                    }
                }
                if matches!(insn, ManagedInsn::Return { .. } | ManagedInsn::Throw { .. }) {
                    return Ok((steps, false));
                }
                let outgoing = self.ir.cfg().block(*block).map_err(invalid)?.outgoing();
                for &edge in outgoing {
                    self.edge(edge, available, added, &mut steps)?;
                }
            }
            Region::If {
                header,
                then_region,
                else_region,
                ..
            } => {
                let (_, insn) = self.insn(*header)?;
                let ManagedInsn::Branch {
                    condition: Some(condition),
                    then_edge,
                    else_edge: Some(else_edge),
                } = insn
                else {
                    return Err(PlanError::InvalidBranch(*header));
                };
                require(available, *condition)?;
                // Each arm only adds definitions. Roll back its delta rather
                // than copying all definitions preceding this conditional.
                let checkpoint = added.len();
                let mut then_steps = Vec::new();
                let mut else_steps = Vec::new();
                self.edge(*then_edge, available, added, &mut then_steps)?;
                let (a_steps, a) = self.lower(then_region, available, added)?;
                let a_added = added.split_off(checkpoint);
                for value in &a_added {
                    available.remove(value);
                }
                self.edge(*else_edge, available, added, &mut else_steps)?;
                let (b_steps, b) = self.lower(else_region, available, added)?;
                let b_added = added.split_off(checkpoint);
                let keep: Vec<_> = match (a, b) {
                    (true, true) => a_added
                        .into_iter()
                        .filter(|v| available.contains(v))
                        .collect(),
                    (true, false) => a_added,
                    (false, true) => b_added.clone(),
                    (false, false) => Vec::new(),
                };
                for value in b_added {
                    available.remove(&value);
                }
                for value in keep {
                    available.insert(value);
                    added.push(value);
                }
                then_steps.extend(a_steps);
                else_steps.extend(b_steps);
                steps.push(Step::If {
                    condition: *condition,
                    then_steps,
                    else_steps,
                });
                return Ok((steps, a || b));
            }
        }
        Ok((steps, true))
    }
}

fn require(available: &HashSet<ValueId>, value: ValueId) -> Result<(), PlanError> {
    if available.contains(&value) {
        Ok(())
    } else {
        Err(PlanError::Unavailable(value))
    }
}

fn merge_continuing(available: &mut HashSet<ValueId>, environments: Vec<HashSet<ValueId>>) -> bool {
    let mut iter = environments.into_iter();
    let Some(mut merged) = iter.next() else {
        return false;
    };
    for env in iter {
        merged.retain(|v| env.contains(v));
    }
    *available = merged;
    true
}

fn defined(insn: &ManagedInsn) -> Vec<ValueId> {
    match insn {
        ManagedInsn::Const { result, .. }
        | ManagedInsn::Move { result, .. }
        | ManagedInsn::Phi { result, .. }
        | ManagedInsn::Arithmetic { result, .. }
        | ManagedInsn::Compare { result, .. }
        | ManagedInsn::OrderingCompare { result, .. }
        | ManagedInsn::Neg { result, .. }
        | ManagedInsn::Not { result, .. }
        | ManagedInsn::FieldGet { result, .. }
        | ManagedInsn::ArrayGet { result, .. }
        | ManagedInsn::ArrayLength { result, .. }
        | ManagedInsn::NewInstance { result, .. }
        | ManagedInsn::NewArray { result, .. }
        | ManagedInsn::Cast { result, .. }
        | ManagedInsn::InstanceOf { result, .. } => vec![*result],
        ManagedInsn::Invoke { result, .. } | ManagedInsn::NewFilledArray { result, .. } => {
            result.iter().copied().collect()
        }
        ManagedInsn::Opaque { writes, .. } => writes.clone(),
        _ => Vec::new(),
    }
}
fn reads(insn: &ManagedInsn) -> Vec<ValueId> {
    match insn {
        ManagedInsn::Move { value, .. }
        | ManagedInsn::Neg { value, .. }
        | ManagedInsn::Not { value, .. }
        | ManagedInsn::Cast { value, .. }
        | ManagedInsn::InstanceOf { value, .. }
        | ManagedInsn::Throw { value } => vec![*value],
        ManagedInsn::Arithmetic { lhs, rhs, .. }
        | ManagedInsn::Compare { lhs, rhs, .. }
        | ManagedInsn::OrderingCompare { lhs, rhs, .. } => {
            vec![*lhs, *rhs]
        }
        ManagedInsn::FieldGet { object, .. } => object.iter().copied().collect(),
        ManagedInsn::FieldPut { object, value, .. } => {
            object.iter().copied().chain([*value]).collect()
        }
        ManagedInsn::ArrayGet { array, index, .. } => vec![*array, *index],
        ManagedInsn::ArrayPut {
            array,
            index,
            value,
        } => vec![*array, *index, *value],
        ManagedInsn::ArrayLength { array, .. } => vec![*array],
        ManagedInsn::NewArray { length, .. } => vec![*length],
        ManagedInsn::NewFilledArray { elements, .. } => elements.clone(),
        ManagedInsn::FillArray { array, values } => std::iter::once(*array)
            .chain(values.iter().copied())
            .collect(),
        ManagedInsn::Invoke { args, .. } => args.clone(),
        ManagedInsn::MonitorEnter { object } | ManagedInsn::MonitorExit { object } => vec![*object],
        ManagedInsn::Branch { condition, .. } => condition.iter().copied().collect(),
        ManagedInsn::Switch { key, .. } => vec![*key],
        ManagedInsn::Return { value } => value.iter().copied().collect(),
        ManagedInsn::Opaque { reads, .. } => reads.clone(),
        ManagedInsn::Const { .. } | ManagedInsn::Phi { .. } | ManagedInsn::NewInstance { .. } => {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod canonical_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use decomp_core::cfg::Cfg;
    use managed_ir::{
        method::{InsnArena, ValueArena},
        value::Value,
    };

    #[test]
    fn canonical_entrypoints_match_untrusted_region_validation() {
        for ty in [ManagedType::Boolean, ManagedType::Int] {
            let mut insns = InsnArena::with_key();
            let entry = insns.insert(ManagedInsn::Return { value: None });
            let parameter = insns.insert(ManagedInsn::Return { value: None });
            let mut values = ValueArena::with_key();
            let condition = values.insert(Value::new(parameter, ty));
            let mut cfg = Cfg::new(entry);
            let left = cfg.add_block(insns.insert(ManagedInsn::Return { value: None }));
            let right = cfg.add_block(insns.insert(ManagedInsn::Return { value: None }));
            let then_edge = cfg
                .add_edge(cfg.entry(), left, EdgeKind::BranchTrue)
                .unwrap();
            let else_edge = cfg
                .add_edge(cfg.entry(), right, EdgeKind::BranchFalse)
                .unwrap();
            insns[entry] = ManagedInsn::Branch {
                condition: Some(condition),
                then_edge,
                else_edge: Some(else_edge),
            };
            let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
            for bindings in [vec![condition], vec![]] {
                let declarations = HashSet::new();
                let region = decomp_structure::structure(ir.cfg()).unwrap().root;
                let old = plan_checked(&ir, &region, &bindings, &declarations);
                let new = plan_canonical_checked(&ir, &bindings, &declarations);
                assert_eq!(format!("{old:?}"), format!("{new:?}"));
                let extended = decomp_structure::structure_extended(ir.cfg()).unwrap().root;
                let old = plan_extended_checked(&ir, &extended, &bindings, &declarations);
                let new = plan_extended_canonical_checked(&ir, &bindings, &declarations);
                assert_eq!(format!("{old:?}"), format!("{new:?}"));
            }
            let wrong = Region::Sequence(vec![]);
            assert!(matches!(
                plan_checked(&ir, &wrong, &[condition], &HashSet::new()),
                Err(PlanError::RegionMismatch)
            ));
        }
    }
}
