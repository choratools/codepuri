//! Effect-preserving exception lowering with caller-owned metadata.
//! `plan`/`emit` retain the legacy linear grammar. `plan_region` and
//! `emit_method_region` execute acyclic branching scopes, abrupt handlers,
//! literal catch-all dispatch, proved finally and balanced monitor regions.
//!
//! Integration: the facade implements `ExceptionMetadata`, translating source
//! try ranges to the exact post-SSA block chain (including normal-success
//! commits). Call `plan` with explicit receiver/parameter SSA bindings, then
//! `emit` with a transactional typed renderer. Publish a complete declaration
//! only after all callbacks succeed. `TypedEmitter::instruction` owns ordinary
//! operation validation and checked-exception coverage; this module never
//! resolves DEX indices or invents a finally region.
//!
//! The extended facade adapter supplies an attested, frozen-name JavaHierarchy;
//! Boolean assignability false is never treated as non-shadowing evidence.
#[path = "exception_dispatch.rs"]
pub mod dispatch;

use crate::{JavaResolver, JavaType, branching::Copy};
pub use allocation_proofs::original_allocation_chains;
use decomp_core::{
    cfg::EdgeKind,
    ids::{BlockId, EdgeId},
};
pub use decomp_structure::exceptions as scopes;
use managed_ir::{
    effects::EffectFlags,
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
    value::{InsnId, ValueId},
};
use std::collections::{HashMap, HashSet};

/// Exact source types are resolved by the facade; placeholder CFG labels are
/// NOT class names. Hierarchy proof is required for Throwable and catch order.
pub trait ExceptionMetadata {
    /// Exact post-SSA source scopes.
    fn scopes(&self) -> &[scopes::Scope];
    /// Resolve a frontend caught-type identity in the frozen naming domain.
    fn catch_type(&self, key: &str) -> Option<JavaType>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// Ordered materialization and simultaneous copies.
pub enum Step {
    /// Original instruction emitted once.
    Instruction(InsnId),
    /// Snapshot all sources before destination writes.
    Copies(Vec<Copy>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// Typed catch and validated body.
pub struct Handler {
    /// Exact resolved caught type.
    pub ty: JavaType,
    /// SSA identity bound to the caught Java object.
    pub exception: ValueId,
    /// Handler operations.
    pub steps: Vec<Step>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// Complete effect-preserving exception lowering plan.
pub struct ExceptionPlan {
    /// Hoist ALL definitions. Renderer must initialize locals to typed defaults
    /// for Java definite assignment only; planner proves every semantic read.
    pub locals: Vec<ValueId>,
    /// Operations outside and before the scope.
    pub prefix: Vec<Step>,
    /// Operations within the exact protected scope.
    pub protected: Vec<Step>,
    /// Catch bodies in source dispatch order.
    pub handlers: Vec<Handler>,
    /// Common continuation outside the scope.
    pub suffix: Vec<Step>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// Typed exception-planning failure.
pub enum Error {
    /// Explicit source scope rejected by the structurer.
    Scope(scopes::Error),
    /// Invalid IR identity or metadata.
    Invalid(String),
    /// Operation or proof outside the conservative grammar.
    Unsupported(&'static str),
    /// Semantic read not available on every incoming path.
    Unavailable(ValueId),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exception plan: {self:?}")
    }
}
impl std::error::Error for Error {}
fn invalid(e: impl std::fmt::Display) -> Error {
    Error::Invalid(e.to_string())
}

/// Plan before producing any source. No mutation, type guessing or finally
/// recovery. Incoming handler Phi transfers are performed BEFORE the throwing
/// instruction; normal transfers occur AFTER successful materialization.
pub fn plan(
    ir: &MethodIr<SsaForm>,
    metadata: &(impl ExceptionMetadata + ?Sized),
    bindings: &[ValueId],
    resolver: &impl JavaResolver,
) -> Result<ExceptionPlan, Error> {
    let tree = scopes::structure(ir.cfg(), metadata.scopes()).map_err(Error::Scope)?;
    let mut types = Vec::new();
    for c in &tree.scope.catches {
        let t = metadata
            .catch_type(&c.key)
            .ok_or(Error::Unsupported("unresolved catch type"))?;
        if !matches!(t, JavaType::Class(_))
            || !resolver.assignable(&t, &JavaType::Class("java.lang.Throwable".into()))
        {
            return Err(Error::Unsupported("catch type is not proven Throwable"));
        }
        // Require positive non-shadowing proof in both directions. A boolean
        // assignability false is not proof of unrelated hierarchy.
        for earlier in &types {
            if resolver.assignable(&t, earlier) || !resolver.assignable(earlier, &t) {
                return Err(Error::Unsupported("catch ordering lacks hierarchy proof"));
            }
        }
        types.push(t);
    }
    let mut p = Planner {
        ir,
        copies: HashMap::new(),
        phi: HashSet::new(),
        handler: HashMap::new(),
        bindings: bindings.iter().copied().collect(),
    };
    let mut defs = HashSet::new();
    let mut placed = HashSet::new();
    for b in ir.cfg().block_ids() {
        let (id, insn) = p.insn(b)?;
        if !placed.insert(id) {
            return Err(Error::Unsupported("duplicate instruction placement"));
        }
        for v in defined(insn) {
            if !defs.insert(v) || ir.value(v).map_err(invalid)?.def() != id {
                return Err(Error::Unsupported("invalid SSA definition"));
            }
        }
        if let ManagedInsn::Branch {
            condition,
            then_edge,
            else_edge,
        } = insn
        {
            let outgoing = ir.cfg().block(b).map_err(invalid)?.outgoing();
            if condition.is_some()
                || else_edge.is_some()
                || outgoing != [*then_edge]
                || ir.cfg().edge(*then_edge).map_err(invalid)?.kind() != &EdgeKind::Unconditional
            {
                return Err(Error::Unsupported("branch instruction/edge mismatch"));
            }
        }
        if matches!(insn, ManagedInsn::Return { .. } | ManagedInsn::Throw { .. })
            && !ir.cfg().block(b).map_err(invalid)?.outgoing().is_empty()
        {
            return Err(Error::Unsupported("terminal has successors"));
        }
    }
    let mut initial = HashSet::new();
    for &v in bindings {
        ir.value(v).map_err(invalid)?;
        let synthetic = matches!(ir.instruction(ir.value(v).map_err(invalid)?.def()).map_err(invalid)?,
            ManagedInsn::Opaque { family_tag: 0x8001 | 0x8003, reads, writes, effects }
            if reads.is_empty() && writes.as_slice() == [v] && *effects == EffectFlags::NONE);
        if (defs.contains(&v) && !synthetic) || !initial.insert(v) {
            return Err(Error::Unsupported("invalid parameter binding"));
        }
        defs.remove(&v);
    }
    // Collect every Phi cluster independently, retaining original edge identity.
    for b in ir.cfg().block_ids() {
        if !matches!(p.insn(b)?.1, ManagedInsn::Phi { .. }) || p.phi.contains(&b) {
            continue;
        }
        let incoming = ir.cfg().block(b).map_err(invalid)?.incoming().to_vec();
        // Prefix insertion can allocate a cluster head AFTER its tail. Choose
        // heads by topology, never arena order.
        if let [edge_id] = incoming.as_slice() {
            let edge = ir.cfg().edge(*edge_id).map_err(invalid)?;
            if edge.kind() == &EdgeKind::Unconditional
                && ir.cfg().block(edge.source()).map_err(invalid)?.outgoing() == incoming
                && matches!(p.insn(edge.source())?.1, ManagedInsn::Phi { .. })
            {
                continue;
            }
        }
        let mut current = b;
        while let ManagedInsn::Phi { result, inputs } = p.insn(current)?.1.clone() {
            if inputs.len() != incoming.len()
                || inputs.iter().map(|(e, _)| *e).collect::<HashSet<_>>()
                    != incoming.iter().copied().collect()
            {
                return Err(Error::Unsupported("Phi edge coverage"));
            }
            for &(e, v) in &inputs {
                if ir.value(v).map_err(invalid)?.ty() != ir.value(result).map_err(invalid)?.ty() {
                    return Err(Error::Unsupported("Phi type mismatch"));
                }
                p.copies.entry(e).or_default().push(Copy {
                    destination: result,
                    source: v,
                });
            }
            p.phi.insert(current);
            let out = ir.cfg().block(current).map_err(invalid)?.outgoing();
            if out.len() != 1 {
                break;
            }
            let edge = ir.cfg().edge(out[0]).map_err(invalid)?;
            if edge.kind() != &EdgeKind::Unconditional
                || ir.cfg().block(edge.target()).map_err(invalid)?.incoming() != out
            {
                break;
            }
            current = edge.target();
        }
    }
    for (index, blocks) in tree.handlers.iter().enumerate() {
        let candidates: Vec<_> = blocks
            .iter()
            .filter_map(|b| match p.insn(*b).ok()?.1 {
                ManagedInsn::Opaque {
                    family_tag: 0x8002,
                    reads,
                    writes,
                    effects,
                } if reads.is_empty()
                    && writes.len() == 1
                    && matches!(*effects, EffectFlags::NONE | EffectFlags::MAY_THROW) =>
                {
                    Some((*b, writes[0]))
                }
                _ => None,
            })
            .collect();
        if candidates.len() != 1 {
            return Err(Error::Unsupported(
                "handler requires exactly one TAG_HANDLER",
            ));
        }
        let managed_ir::types::ManagedType::Reference(class) =
            ir.value(candidates[0].1).map_err(invalid)?.ty()
        else {
            return Err(Error::Unsupported("handler value lacks reference type"));
        };
        let expected = JavaType::Class(
            resolver
                .class(*class)
                .ok_or(Error::Unsupported("unresolved handler value type"))?,
        );
        if !resolver.assignable(&types[index], &expected) {
            return Err(Error::Unsupported(
                "catch value incompatible with handler local",
            ));
        }
        p.handler.insert(candidates[0].0, (index, candidates[0].1));
    }
    let mut normal = initial;
    let prefix = p.lower(&tree.prefix, &mut normal, false)?;
    let mut exceptional_states: Vec<Vec<HashSet<ValueId>>> = vec![Vec::new(); tree.handlers.len()];
    let mut protected = Vec::new();
    for &b in &tree.scope.protected {
        let edges: Vec<_> = tree
            .exceptional
            .iter()
            .filter(|e| ir.cfg().edge(**e).is_ok_and(|edge| edge.source() == b))
            .copied()
            .collect();
        let (_, insn) = p.insn(b)?;
        if may_throw(ir, insn) == edges.is_empty() {
            return Err(Error::Unsupported("throwing-site metadata mismatch"));
        }
        for edge in edges {
            let target = ir.cfg().edge(edge).map_err(invalid)?.target();
            let index = tree
                .scope
                .catches
                .iter()
                .position(|c| c.entry == target)
                .ok_or(Error::Unsupported("missing catch target"))?;
            let mut state = normal.clone();
            if let Some(step) = p.transfer(edge, &mut state)? {
                protected.push(step);
            }
            exceptional_states[index].push(state);
        }
        protected.extend(p.lower(&[b], &mut normal, true)?);
    }
    let mut handlers = Vec::new();
    let mut exits = vec![normal];
    for (index, blocks) in tree.handlers.iter().enumerate() {
        let mut states = exceptional_states[index].iter();
        let mut state = states
            .next()
            .ok_or(Error::Unsupported("unreachable catch"))?
            .clone();
        for s in states {
            state.retain(|v| s.contains(v));
        }
        let exception = p
            .handler
            .values()
            .find(|(i, _)| *i == index)
            .ok_or(Error::Unsupported("missing handler binding"))?
            .1;
        let steps = p.lower(blocks, &mut state, false)?;
        handlers.push(Handler {
            ty: types[index].clone(),
            exception,
            steps,
        });
        exits.push(state);
    }
    let mut joined = exits.remove(0);
    for state in exits {
        joined.retain(|v| state.contains(v));
    }
    let suffix = p.lower(&tree.suffix, &mut joined, false)?;
    if !matches!(
        p.insn(
            *tree
                .suffix
                .last()
                .ok_or(Error::Unsupported("empty continuation"))?
        )?
        .1,
        ManagedInsn::Return { .. } | ManagedInsn::Throw { .. }
    ) {
        return Err(Error::Unsupported("missing terminal"));
    }
    Ok(ExceptionPlan {
        locals: ir.values().keys().filter(|v| defs.contains(v)).collect(),
        prefix,
        protected,
        handlers,
        suffix,
    })
}
struct Planner<'a> {
    ir: &'a MethodIr<SsaForm>,
    copies: HashMap<EdgeId, Vec<Copy>>,
    phi: HashSet<BlockId>,
    handler: HashMap<BlockId, (usize, ValueId)>,
    bindings: HashSet<ValueId>,
}
impl Planner<'_> {
    fn insn(&self, b: BlockId) -> Result<(InsnId, &ManagedInsn), Error> {
        let id = *self.ir.cfg().block(b).map_err(invalid)?.payload();
        Ok((id, self.ir.instruction(id).map_err(invalid)?))
    }
    fn transfer(&self, edge: EdgeId, state: &mut HashSet<ValueId>) -> Result<Option<Step>, Error> {
        let Some(copies) = self.copies.get(&edge) else {
            return Ok(None);
        };
        for c in copies {
            if !state.contains(&c.source) {
                return Err(Error::Unavailable(c.source));
            }
        }
        state.extend(copies.iter().map(|c| c.destination));
        Ok(Some(Step::Copies(copies.clone())))
    }
    fn lower(
        &self,
        blocks: &[BlockId],
        state: &mut HashSet<ValueId>,
        _protected: bool,
    ) -> Result<Vec<Step>, Error> {
        let mut steps = Vec::new();
        for &b in blocks {
            let (id, insn) = self.insn(b)?;
            if self.phi.contains(&b) {
                for v in defined(insn) {
                    if !state.contains(&v) {
                        return Err(Error::Unavailable(v));
                    }
                }
            } else if let Some((_, exception)) = self.handler.get(&b) {
                state.insert(*exception);
            } else if matches!(insn, ManagedInsn::Opaque { family_tag: 0x8001 | 0x8003, reads, writes, effects }
                if reads.is_empty() && writes.len() == 1 && self.bindings.contains(&writes[0]) && *effects == EffectFlags::NONE)
            {
            } else {
                if matches!(
                    insn,
                    ManagedInsn::Opaque { .. }
                        | ManagedInsn::Switch { .. }
                        | ManagedInsn::MonitorEnter { .. }
                        | ManagedInsn::MonitorExit { .. }
                ) {
                    return Err(Error::Unsupported("unsupported instruction"));
                }
                for v in reads(insn) {
                    if !state.contains(&v) {
                        return Err(Error::Unavailable(v));
                    }
                }
                state.extend(defined(insn));
                if !matches!(insn, ManagedInsn::Branch { .. }) {
                    steps.push(Step::Instruction(id));
                }
            }
            for &edge in self.ir.cfg().block(b).map_err(invalid)?.outgoing() {
                if self.ir.cfg().edge(edge).map_err(invalid)?.kind() == &EdgeKind::Unconditional
                    && let Some(step) = self.transfer(edge, state)?
                {
                    steps.push(step);
                }
            }
        }
        Ok(steps)
    }
}
// Match the semantic operation retained by the lifter, not merely its IR
// variant. Numeric casts and floating division do not throw synchronously;
// unresolved type evidence stays conservative rather than losing catch edges.
fn may_throw(ir: &MethodIr<SsaForm>, i: &ManagedInsn) -> bool {
    use managed_ir::{
        ops::{ArithOp, Constant},
        types::ManagedType,
    };
    let ty = |v| ir.value(v).ok().map(|value| value.ty());
    match i {
        ManagedInsn::Cast { result, value, .. } => {
            let numeric = |t: Option<&ManagedType>| {
                matches!(
                    t,
                    Some(
                        ManagedType::Byte
                            | ManagedType::Short
                            | ManagedType::Char
                            | ManagedType::Int
                            | ManagedType::Long
                            | ManagedType::Float
                            | ManagedType::Double
                    )
                )
            };
            !(numeric(ty(*result))
                && (numeric(ty(*value)) || matches!(ty(*value), Some(ManagedType::Zero))))
        }
        ManagedInsn::Arithmetic {
            op: ArithOp::Div | ArithOp::Rem,
            result,
            lhs,
            rhs,
        } => !matches!(
            (ty(*result), ty(*lhs), ty(*rhs)),
            (
                Some(ManagedType::Float),
                Some(ManagedType::Float),
                Some(ManagedType::Float)
            ) | (
                Some(ManagedType::Double),
                Some(ManagedType::Double),
                Some(ManagedType::Double)
            )
        ),
        ManagedInsn::Const {
            value: Constant::String(_) | Constant::ClassLiteral(_),
            ..
        } => true,
        ManagedInsn::Opaque { effects, .. } => effects.contains(EffectFlags::MAY_THROW),
        ManagedInsn::Invoke { .. }
        | ManagedInsn::FieldGet { .. }
        | ManagedInsn::FieldPut { .. }
        | ManagedInsn::ArrayGet { .. }
        | ManagedInsn::ArrayPut { .. }
        | ManagedInsn::ArrayLength { .. }
        | ManagedInsn::NewInstance { .. }
        | ManagedInsn::NewArray { .. }
        | ManagedInsn::NewFilledArray { .. }
        | ManagedInsn::FillArray { .. }
        | ManagedInsn::InstanceOf { .. }
        | ManagedInsn::Throw { .. }
        | ManagedInsn::MonitorEnter { .. }
        | ManagedInsn::MonitorExit { .. } => true,
        _ => false,
    }
}
#[cfg(test)]
#[path = "exception_throw_tests.rs"]
mod throw_tests;

fn defined(i: &ManagedInsn) -> Vec<ValueId> {
    use ManagedInsn::*;
    match i {
        Const { result, .. }
        | Move { result, .. }
        | Phi { result, .. }
        | Arithmetic { result, .. }
        | Compare { result, .. }
        | OrderingCompare { result, .. }
        | Neg { result, .. }
        | Not { result, .. }
        | FieldGet { result, .. }
        | ArrayGet { result, .. }
        | ArrayLength { result, .. }
        | NewInstance { result, .. }
        | NewArray { result, .. }
        | Cast { result, .. }
        | InstanceOf { result, .. } => vec![*result],
        Invoke { result, .. } | NewFilledArray { result, .. } => result.iter().copied().collect(),
        Opaque { writes, .. } => writes.clone(),
        _ => vec![],
    }
}
fn reads(i: &ManagedInsn) -> Vec<ValueId> {
    use ManagedInsn::*;
    match i {
        Move { value, .. }
        | Neg { value, .. }
        | Not { value, .. }
        | Cast { value, .. }
        | InstanceOf { value, .. }
        | Throw { value } => vec![*value],
        Arithmetic { lhs, rhs, .. }
        | Compare { lhs, rhs, .. }
        | OrderingCompare { lhs, rhs, .. } => vec![*lhs, *rhs],
        FieldGet { object, .. } => object.iter().copied().collect(),
        FieldPut { object, value, .. } => object.iter().copied().chain([*value]).collect(),
        ArrayGet { array, index, .. } => vec![*array, *index],
        ArrayPut {
            array,
            index,
            value,
        } => vec![*array, *index, *value],
        ArrayLength { array, .. } => vec![*array],
        NewArray { length, .. } => vec![*length],
        NewFilledArray { elements, .. } => elements.clone(),
        FillArray { array, values } => std::iter::once(*array)
            .chain(values.iter().copied())
            .collect(),
        Invoke { args, .. } => args.clone(),
        MonitorEnter { object } | MonitorExit { object } => vec![*object],
        Branch { condition, .. } => condition.iter().copied().collect(),
        Switch { key, .. } => vec![*key],
        Return { value } => value.iter().copied().collect(),
        Opaque { reads, .. } => reads.clone(),
        _ => vec![],
    }
}

#[path = "exception_allocation_proofs.rs"]
mod allocation_proofs;
pub use allocation_proofs::OriginalScope;
#[path = "exception_checked_guards.rs"]
mod checked_guards;

/// Source adapter evidence for the extended exception grammar. The hierarchy
/// must use exactly the resolver's frozen source names, including catch types.
pub trait RegionMetadata {
    /// Immutable original code-range evidence; absent for synthetic metadata.
    fn original_scope(&self) -> Option<&OriginalScope> {
        None
    }
    /// Exact contiguous post-SSA scope.
    fn region_scope(&self) -> &scopes::RegionScope;
    /// Original caught type, or Throwable for a final catch-all entry.
    fn catch_type(&self, key: &str) -> Option<JavaType>;
    /// Attested input/classpath declarations; references alone are insufficient.
    fn hierarchy(&self) -> &crate::type_compatibility::Hierarchy;
    /// True only for the original encoded catch-all entry.
    fn is_catch_all(&self, key: &str) -> bool;
}

/// Prove the narrowly shared empty-handler terminal shape. The ordinary
/// disjoint structurer still validates all scopes, dispatch edges and coverage.
/// A terminal carrying a value, Phi, effect or successor cannot satisfy this.
pub fn shared_void_terminal(ir: &MethodIr<SsaForm>, scope: &scopes::RegionScope) -> bool {
    let Some(terminal) = scope.continuation else {
        return false;
    };
    let Ok(block) = ir.cfg().block(terminal) else {
        return false;
    };
    if scope.catches.is_empty()
        || scope.protected.contains(&terminal)
        || !scope.catches.iter().all(|catch| catch.entry == terminal)
        || !block.outgoing().is_empty()
        || !matches!(
            ir.instruction(*block.payload()),
            Ok(ManagedInsn::Return { value: None })
        )
    {
        return false;
    }
    block.incoming().iter().any(|edge| {
        ir.cfg().edge(*edge).is_ok_and(|edge| {
            !matches!(edge.kind(), EdgeKind::Exception(_))
                && !scope.protected.contains(&edge.source())
        })
    })
}

struct BorrowedMetadata<'a, T: RegionMetadata + ?Sized>(&'a T);
impl<T: RegionMetadata + ?Sized> RegionMetadata for BorrowedMetadata<'_, T> {
    fn original_scope(&self) -> Option<&OriginalScope> {
        self.0.original_scope()
    }
    fn region_scope(&self) -> &scopes::RegionScope {
        self.0.region_scope()
    }
    fn catch_type(&self, key: &str) -> Option<JavaType> {
        self.0.catch_type(key)
    }
    fn hierarchy(&self) -> &crate::type_compatibility::Hierarchy {
        self.0.hierarchy()
    }
    fn is_catch_all(&self, key: &str) -> bool {
        self.0.is_catch_all(key)
    }
}

/// Explicit executable exception regions. Ordinary operations retain their
/// original materialization sites; cleanup relocation requires a proof below.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegionStep {
    /// Ordinary instruction.
    Instruction(InsnId),
    /// Simultaneous nonthrowing edge copies.
    Copies(Vec<Copy>),
    /// Materialized Boolean conditional.
    If {
        /// SSA Boolean, never an inlined expression.
        condition: ValueId,
        /// True arm.
        then_body: Vec<RegionStep>,
        /// False arm.
        else_body: Vec<RegionStep>,
    },
    /// Original typed dispatch, including a literal catch-all when necessary.
    Try {
        /// Exact protected operations.
        body: Vec<RegionStep>,
        /// Original ordered handlers.
        handlers: Vec<RegionHandler>,
    },
    /// Separate original ranges transferring to one common handler.
    SharedHandler {
        /// Exact common caught type and SSA capture identity.
        ty: JavaType,
        /// Blank local assigned only by original catch transfers.
        exception: ValueId,
        /// Normal paths, all abrupt before the common handler.
        body: Vec<RegionStep>,
        /// Original handler operations, placed once outside original ranges.
        handler: Vec<RegionStep>,
    },
    /// Original typed catch transfers forward; it does not execute the handler.
    TryTransfer {
        /// Exact original protected operations and prethrow edge copies.
        body: Vec<RegionStep>,
        /// Original encoded catch type.
        ty: JavaType,
        /// Common original capture identity.
        exception: ValueId,
    },
    /// Proven identical cleanup on every normal exit and captured-object rethrow.
    Finally {
        /// Operations before normal cleanup.
        body: Vec<RegionStep>,
        /// Single original cleanup body, with no invented effects.
        cleanup: Vec<RegionStep>,
    },
    /// Proven balanced, same-object monitor region.
    Synchronized {
        /// Materialized monitor identity.
        object: ValueId,
        /// Protected operations with monitor instructions removed by proof.
        body: Vec<RegionStep>,
    },
}

/// Typed dispatch entry with an abrupt or continuing body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionHandler {
    /// Resolved catch type.
    pub ty: JavaType,
    /// Caught SSA object.
    pub exception: Option<ValueId>,
    /// Executable body; rethrows are retained.
    pub body: Vec<RegionStep>,
}

/// Complete method plan; parent publishes source only after rendering succeeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionPlan {
    /// Hoisted definitions, excluding explicit incoming bindings.
    pub locals: Vec<ValueId>,
    /// Caught-object bindings/identity moves and null-only comparison operands
    /// whose reference type follows from exact metadata/use evidence.
    pub contextual_types: Vec<(ValueId, JavaType)>,
    /// Complete body.
    pub body: Vec<RegionStep>,
}

/// Plan branching protected scopes, abrupt handlers and proved cleanup. A
/// catch-all that fails finally proof remains a real catch(Throwable), never a
/// manufactured finally. Raw monitor instructions cannot escape this planner.
pub fn plan_region(
    ir: &MethodIr<SsaForm>,
    metadata: &(impl RegionMetadata + ?Sized),
    bindings: &[ValueId],
    resolver: &impl JavaResolver,
) -> Result<RegionPlan, Error> {
    plan_region_with_declarations(ir, metadata, bindings, resolver, &HashMap::new())
}

fn plan_region_with_declarations(
    ir: &MethodIr<SsaForm>,
    metadata: &(impl RegionMetadata + ?Sized),
    bindings: &[ValueId],
    resolver: &impl JavaResolver,
    declarations: &HashMap<ValueId, JavaType>,
) -> Result<RegionPlan, Error> {
    use crate::type_compatibility::Compatibility;
    let scope = metadata.region_scope();
    let tree = match scopes::structure_region(ir.cfg(), scope) {
        Ok(tree) => tree,
        Err(_) if shared_void_terminal(ir, scope) => {
            let borrowed = BorrowedMetadata(metadata);
            return plan_disjoint_with_declarations(
                ir,
                &[&borrowed],
                bindings,
                resolver,
                declarations,
            );
        }
        Err(error) => return Err(Error::Scope(error)),
    };
    let mut types = Vec::new();
    for (index, c) in scope.catches.iter().enumerate() {
        let ty = metadata
            .catch_type(&c.key)
            .ok_or(Error::Unsupported("unresolved catch type"))?;
        if !matches!(ty, JavaType::Class(_))
            || metadata
                .hierarchy()
                .assignable(&ty, &JavaType::Class("java.lang.Throwable".into()))
                != Compatibility::Assignable
        {
            return Err(Error::Unsupported("catch type is not proven Throwable"));
        }
        if metadata.is_catch_all(&c.key)
            && (index + 1 != scope.catches.len()
                || ty != JavaType::Class("java.lang.Throwable".into()))
        {
            return Err(Error::Unsupported(
                "catch-all is not last Throwable dispatch",
            ));
        }
        for earlier in &types {
            if metadata.hierarchy().assignable(&ty, earlier) != Compatibility::Incompatible {
                return Err(Error::Unsupported(
                    "shadowed catch or incomplete JavaHierarchy",
                ));
            }
        }
        types.push(ty);
    }
    let (mut p, defs, initial) = prepare_region(ir, bindings, declarations)?;
    let mut handler_values = Vec::new();
    let mut contextual_types = Vec::new();
    // Dalvik if-eqz/if-nez inserts a raw-zero SSA constant. It is provably null
    // when every use is equality against a resolved reference of the same type.
    // Do not pick a type from one use if another use is numeric or ambiguous.
    for (value, info) in ir.values() {
        if !matches!(
            info.ty(),
            managed_ir::types::ManagedType::Zero | managed_ir::types::ManagedType::Null
        ) || !matches!(
            ir.instruction(info.def()).map_err(invalid)?,
            ManagedInsn::Const {
                value: managed_ir::ops::Constant::Zero | managed_ir::ops::Constant::Null,
                ..
            }
        ) {
            continue;
        }
        let mut expected = None;
        let mut valid = true;
        for i in ir
            .instructions()
            .values()
            .filter(|i| reads(i).contains(&value))
        {
            if let ManagedInsn::Compare {
                op: managed_ir::ops::CompareOp::Equal | managed_ir::ops::CompareOp::NotEqual,
                lhs,
                rhs,
                ..
            } = i
            {
                let other = if *lhs == value { *rhs } else { *lhs };
                let managed_ir::types::ManagedType::Reference(class) =
                    ir.value(other).map_err(invalid)?.ty()
                else {
                    valid = false;
                    break;
                };
                let ty = JavaType::Class(
                    resolver
                        .class(*class)
                        .ok_or(Error::Unsupported("unresolved null comparison type"))?,
                );
                if expected.as_ref().is_some_and(|e| e != &ty) {
                    valid = false;
                    break;
                }
                expected = Some(ty);
            } else {
                valid = false;
                break;
            }
        }
        if valid && let Some(ty) = expected {
            contextual_types.push((value, ty));
        }
    }
    for (index, regions) in tree.handlers.iter().enumerate() {
        let mut blocks = Vec::new();
        region_blocks(regions, &mut blocks);
        let candidates: Vec<_> = blocks
            .iter()
            .filter_map(|b| match p.insn(*b).ok()?.1 {
                ManagedInsn::Opaque {
                    family_tag: 0x8002,
                    reads,
                    writes,
                    effects,
                } if reads.is_empty()
                    && writes.len() == 1
                    && matches!(*effects, EffectFlags::NONE | EffectFlags::MAY_THROW) =>
                {
                    Some((*b, writes[0]))
                }
                _ => None,
            })
            .collect();
        match candidates.as_slice() {
            [] => handler_values.push(None),
            [(block, value)] => {
                let expected = match ir.value(*value).map_err(invalid)?.ty() {
                    managed_ir::types::ManagedType::Reference(class) => JavaType::Class(
                        resolver
                            .class(*class)
                            .ok_or(Error::Unsupported("unresolved handler value type"))?,
                    ),
                    managed_ir::types::ManagedType::Unknown => types[index].clone(),
                    _ => return Err(Error::Unsupported("handler value lacks reference type")),
                };
                if metadata.hierarchy().assignable(&types[index], &expected)
                    != Compatibility::Assignable
                {
                    return Err(Error::Unsupported(
                        "catch value incompatible with handler local",
                    ));
                }
                contextual_types.push((*value, expected.clone()));
                // Only direct SSA identity moves inherit this contextual type.
                let mut aliases = HashSet::from([*value]);
                loop {
                    let before = aliases.len();
                    for &b in &blocks {
                        if let ManagedInsn::Move { result, value } = p.insn(b)?.1
                            && aliases.contains(value)
                            && aliases.insert(*result)
                        {
                            match ir.value(*result).map_err(invalid)?.ty() {
                                managed_ir::types::ManagedType::Unknown => {
                                    contextual_types.push((*result, expected.clone()))
                                }
                                managed_ir::types::ManagedType::Reference(class)
                                    if metadata.hierarchy().assignable(
                                        &expected,
                                        &JavaType::Class(resolver.class(*class).ok_or(
                                            Error::Unsupported("unresolved handler alias type"),
                                        )?),
                                    ) == Compatibility::Assignable => {}
                                _ => {
                                    return Err(Error::Unsupported(
                                        "incompatible caught-object move",
                                    ));
                                }
                            }
                        }
                    }
                    if aliases.len() == before {
                        break;
                    }
                }
                p.handler.insert(*block, (index, *value));
                handler_values.push(Some(*value));
            }
            _ => return Err(Error::Unsupported("multiple caught-object bindings")),
        }
    }
    let mut exceptional_states = vec![Vec::new(); tree.handlers.len()];
    let (mut prefix, state) = lower_regions(
        &p,
        &tree.prefix,
        Some(initial),
        None,
        &mut exceptional_states,
    )?;
    let (protected, normal) = lower_regions(
        &p,
        &tree.protected,
        state,
        Some(scope),
        &mut exceptional_states,
    )?;
    let mut exits: Vec<_> = normal.into_iter().collect();
    let mut handlers = Vec::new();
    for (index, regions) in tree.handlers.iter().enumerate() {
        let state = intersect_states(&exceptional_states[index])
            .ok_or(Error::Unsupported("unreachable catch"))?;
        let (body, exit) = lower_regions(&p, regions, Some(state), None, &mut exceptional_states)?;
        exits.extend(exit);
        handlers.push(RegionHandler {
            ty: types[index].clone(),
            exception: handler_values[index],
            body,
        });
    }
    let (mut suffix, end) = lower_regions(
        &p,
        &tree.suffix,
        intersect_states(&exits),
        None,
        &mut exceptional_states,
    )?;
    if end.is_some() {
        return Err(Error::Unsupported("missing terminal"));
    }
    let mut region = RegionStep::Try {
        body: protected,
        handlers,
    };
    // Only this narrow equivalence proof may relocate cleanup into finally.
    recover_cleanup(&p, metadata, &mut prefix, &mut region, &mut suffix)?;
    prefix.push(region);
    prefix.extend(suffix);
    if contains_monitor(ir, &prefix)? {
        return Err(Error::Unsupported("unproved monitor balance"));
    }
    Ok(RegionPlan {
        locals: ir.values().keys().filter(|v| defs.contains(v)).collect(),
        contextual_types,
        body: prefix,
    })
}

/// Lower an acyclic exception-aware tree with independent disjoint scopes.
/// Every handler is numbered globally while exception snapshots stay scope-local.
pub fn plan_disjoint(
    ir: &MethodIr<SsaForm>,
    metadata: &[&dyn RegionMetadata],
    bindings: &[ValueId],
    resolver: &impl JavaResolver,
) -> Result<RegionPlan, Error> {
    plan_disjoint_with_declarations(ir, metadata, bindings, resolver, &HashMap::new())
}

fn plan_disjoint_with_declarations(
    ir: &MethodIr<SsaForm>,
    metadata: &[&dyn RegionMetadata],
    bindings: &[ValueId],
    resolver: &impl JavaResolver,
    declarations: &HashMap<ValueId, JavaType>,
) -> Result<RegionPlan, Error> {
    use crate::type_compatibility::Compatibility;
    let scopes = metadata
        .iter()
        .map(|m| m.region_scope().clone())
        .collect::<Vec<_>>();
    let (structure, shared) = match scopes::structure_disjoint(ir.cfg(), &scopes) {
        Ok(structure) => (structure, false),
        Err(error) => match scopes::structure_disjoint_shared_handlers(ir.cfg(), &scopes) {
            Ok(structure) => (structure, true),
            Err(_) => return Err(Error::Scope(error)),
        },
    };
    let mut types = Vec::new();
    let mut offsets = Vec::new();
    for m in metadata {
        offsets.push(types.len());
        let mut earlier = Vec::new();
        for (index, c) in m.region_scope().catches.iter().enumerate() {
            let ty = m
                .catch_type(&c.key)
                .ok_or(Error::Unsupported("unresolved catch type"))?;
            if !matches!(ty, JavaType::Class(_))
                || m.hierarchy()
                    .assignable(&ty, &JavaType::Class("java.lang.Throwable".into()))
                    != Compatibility::Assignable
            {
                return Err(Error::Unsupported("catch type is not proven Throwable"));
            }
            if m.is_catch_all(&c.key)
                && (index + 1 != m.region_scope().catches.len()
                    || ty != JavaType::Class("java.lang.Throwable".into()))
            {
                return Err(Error::Unsupported(
                    "catch-all is not last Throwable dispatch",
                ));
            }
            for e in &earlier {
                if m.hierarchy().assignable(&ty, e) != Compatibility::Incompatible {
                    return Err(Error::Unsupported(
                        "shadowed catch or incomplete JavaHierarchy",
                    ));
                }
            }
            earlier.push(ty.clone());
            types.push(ty);
        }
    }
    if shared
        && (metadata
            .iter()
            .any(|m| m.is_catch_all(&m.region_scope().catches[0].key))
            || types.iter().any(|ty| ty != &types[0]))
    {
        return Err(Error::Unsupported(
            "inconsistent shared typed catch contract",
        ));
    }
    let tree = scopes::RegionStructure {
        prefix: Vec::new(),
        protected: Vec::new(),
        suffix: Vec::new(),
        exceptional: Vec::new(),
        handlers: if shared {
            structure.scopes[0].handlers.clone()
        } else {
            structure
                .scopes
                .iter()
                .flat_map(|t| t.handlers.clone())
                .collect()
        },
    };
    let (mut p, defs, initial) = prepare_region(ir, bindings, declarations)?;
    let mut handler_values = Vec::new();
    let mut contextual_types = Vec::new();
    // Dalvik if-eqz/if-nez inserts a raw-zero SSA constant. It is provably null
    // when every use is equality against a resolved reference of the same type.
    // Do not pick a type from one use if another use is numeric or ambiguous.
    for (value, info) in ir.values() {
        if !matches!(
            info.ty(),
            managed_ir::types::ManagedType::Zero | managed_ir::types::ManagedType::Null
        ) || !matches!(
            ir.instruction(info.def()).map_err(invalid)?,
            ManagedInsn::Const {
                value: managed_ir::ops::Constant::Zero | managed_ir::ops::Constant::Null,
                ..
            }
        ) {
            continue;
        }
        let mut expected = None;
        let mut valid = true;
        for i in ir
            .instructions()
            .values()
            .filter(|i| reads(i).contains(&value))
        {
            if let ManagedInsn::Compare {
                op: managed_ir::ops::CompareOp::Equal | managed_ir::ops::CompareOp::NotEqual,
                lhs,
                rhs,
                ..
            } = i
            {
                let other = if *lhs == value { *rhs } else { *lhs };
                let managed_ir::types::ManagedType::Reference(class) =
                    ir.value(other).map_err(invalid)?.ty()
                else {
                    valid = false;
                    break;
                };
                let ty = JavaType::Class(
                    resolver
                        .class(*class)
                        .ok_or(Error::Unsupported("unresolved null comparison type"))?,
                );
                if expected.as_ref().is_some_and(|e| e != &ty) {
                    valid = false;
                    break;
                }
                expected = Some(ty);
            } else {
                valid = false;
                break;
            }
        }
        if valid && let Some(ty) = expected {
            contextual_types.push((value, ty));
        }
    }
    for (index, regions) in tree.handlers.iter().enumerate() {
        let mut blocks = Vec::new();
        region_blocks(regions, &mut blocks);
        let candidates: Vec<_> = blocks
            .iter()
            .filter_map(|b| match p.insn(*b).ok()?.1 {
                ManagedInsn::Opaque {
                    family_tag: 0x8002,
                    reads,
                    writes,
                    effects,
                } if reads.is_empty()
                    && writes.len() == 1
                    && matches!(*effects, EffectFlags::NONE | EffectFlags::MAY_THROW) =>
                {
                    Some((*b, writes[0]))
                }
                _ => None,
            })
            .collect();
        match candidates.as_slice() {
            [] => handler_values.push(None),
            [(block, value)] => {
                let expected = match ir.value(*value).map_err(invalid)?.ty() {
                    managed_ir::types::ManagedType::Reference(class) => JavaType::Class(
                        resolver
                            .class(*class)
                            .ok_or(Error::Unsupported("unresolved handler value type"))?,
                    ),
                    managed_ir::types::ManagedType::Unknown => types[index].clone(),
                    _ => return Err(Error::Unsupported("handler value lacks reference type")),
                };
                if metadata[offsets
                    .iter()
                    .rposition(|base| *base <= index)
                    .ok_or(Error::Unsupported("handler scope missing"))?]
                .hierarchy()
                .assignable(&types[index], &expected)
                    != Compatibility::Assignable
                {
                    return Err(Error::Unsupported(
                        "catch value incompatible with handler local",
                    ));
                }
                contextual_types.push((*value, expected.clone()));
                // Only direct SSA identity moves inherit this contextual type.
                let mut aliases = HashSet::from([*value]);
                loop {
                    let before = aliases.len();
                    for &b in &blocks {
                        if let ManagedInsn::Move { result, value } = p.insn(b)?.1
                            && aliases.contains(value)
                            && aliases.insert(*result)
                        {
                            match ir.value(*result).map_err(invalid)?.ty() {
                                managed_ir::types::ManagedType::Unknown => {
                                    contextual_types.push((*result, expected.clone()))
                                }
                                managed_ir::types::ManagedType::Reference(class)
                                    if metadata[offsets
                                        .iter()
                                        .rposition(|base| *base <= index)
                                        .ok_or(Error::Unsupported("handler scope missing"))?]
                                    .hierarchy()
                                    .assignable(
                                        &expected,
                                        &JavaType::Class(resolver.class(*class).ok_or(
                                            Error::Unsupported("unresolved handler alias type"),
                                        )?),
                                    ) == Compatibility::Assignable => {}
                                _ => {
                                    return Err(Error::Unsupported(
                                        "incompatible caught-object move",
                                    ));
                                }
                            }
                        }
                    }
                    if aliases.len() == before {
                        break;
                    }
                }
                p.handler.insert(*block, (index, *value));
                handler_values.push(Some(*value));
            }
            _ => return Err(Error::Unsupported("multiple caught-object bindings")),
        }
    }
    let mut shared_snapshots = Vec::new();
    let (mut body, end) = lower_disjoint(
        &p,
        &structure.body,
        Some(initial),
        &structure,
        &scopes,
        &types,
        &offsets,
        &handler_values,
        shared.then_some(&mut shared_snapshots),
    )?;
    if end.is_some() {
        return Err(Error::Unsupported("missing terminal"));
    }
    let shared_capture = if shared {
        let capture = handler_values
            .first()
            .copied()
            .flatten()
            .ok_or(Error::Unsupported("shared handler lacks original capture"))?;
        let initial = intersect_states(&shared_snapshots)
            .ok_or(Error::Unsupported("unreachable shared handler"))?;
        let (handler, end) = lower_regions(&p, &tree.handlers[0], Some(initial), None, &mut [])?;
        if end.is_some() {
            return Err(Error::Unsupported("shared handler fallthrough"));
        }
        body = vec![RegionStep::SharedHandler {
            ty: types[0].clone(),
            exception: capture,
            body,
            handler,
        }];
        Some(capture)
    } else {
        None
    };
    if contains_monitor(ir, &body)? {
        return Err(Error::Unsupported("unproved monitor balance"));
    }
    Ok(RegionPlan {
        locals: ir
            .values()
            .keys()
            .filter(|v| defs.contains(v) && Some(*v) != shared_capture)
            .collect(),
        contextual_types,
        body,
    })
}
#[allow(clippy::too_many_arguments)]
fn lower_disjoint(
    p: &Planner<'_>,
    regions: &[scopes::DisjointRegion],
    mut state: Option<HashSet<ValueId>>,
    structure: &scopes::DisjointStructure,
    scopes: &[scopes::RegionScope],
    types: &[JavaType],
    offsets: &[usize],
    handler_values: &[Option<ValueId>],
    mut shared_snapshots: Option<&mut Vec<HashSet<ValueId>>>,
) -> Result<RegionFlow, Error> {
    let mut out = Vec::new();
    for region in regions {
        let available = state
            .take()
            .ok_or(Error::Unsupported("instruction after abrupt completion"))?;
        match region {
            scopes::DisjointRegion::Block(b) => {
                let (steps, next) = lower_regions(
                    p,
                    &[scopes::Region::Block(*b)],
                    Some(available),
                    None,
                    &mut [],
                )?;
                out.extend(steps);
                state = next;
            }
            scopes::DisjointRegion::If {
                head,
                then_edge,
                else_edge,
                then_body,
                else_body,
            } => {
                let ManagedInsn::Branch {
                    condition: Some(condition),
                    ..
                } = p.insn(*head)?.1
                else {
                    return Err(Error::Unsupported("conditional region lacks branch"));
                };
                if !available.contains(condition) {
                    return Err(Error::Unavailable(*condition));
                }
                let mut yes = available.clone();
                let mut no = available;
                let mut y = Vec::new();
                let mut n = Vec::new();
                if let Some(Step::Copies(c)) = p.transfer(*then_edge, &mut yes)? {
                    y.push(RegionStep::Copies(c));
                }
                if let Some(Step::Copies(c)) = p.transfer(*else_edge, &mut no)? {
                    n.push(RegionStep::Copies(c));
                }
                let (ys, ye) = lower_disjoint(
                    p,
                    then_body,
                    Some(yes),
                    structure,
                    scopes,
                    types,
                    offsets,
                    handler_values,
                    shared_snapshots.as_deref_mut(),
                )?;
                let (ns, ne) = lower_disjoint(
                    p,
                    else_body,
                    Some(no),
                    structure,
                    scopes,
                    types,
                    offsets,
                    handler_values,
                    shared_snapshots.as_deref_mut(),
                )?;
                y.extend(ys);
                n.extend(ns);
                state = intersect_states(&ye.into_iter().chain(ne).collect::<Vec<_>>());
                out.push(RegionStep::If {
                    condition: *condition,
                    then_body: y,
                    else_body: n,
                });
            }
            scopes::DisjointRegion::Try(i) => {
                let tree = &structure.scopes[*i];
                let scope = &scopes[*i];
                let base = offsets[*i];
                let mut snapshots = vec![Vec::new(); tree.handlers.len()];
                let (body, normal) = lower_regions(
                    p,
                    &tree.protected,
                    Some(available),
                    Some(scope),
                    &mut snapshots,
                )?;
                if let Some(shared) = shared_snapshots.as_deref_mut() {
                    let capture = handler_values
                        .first()
                        .copied()
                        .flatten()
                        .ok_or(Error::Unsupported("shared handler lacks original capture"))?;
                    shared.extend(snapshots.into_iter().flatten());
                    state = normal;
                    out.push(RegionStep::TryTransfer {
                        body,
                        ty: types[0].clone(),
                        exception: capture,
                    });
                    continue;
                }
                let mut exits = normal.into_iter().collect::<Vec<_>>();
                let mut handlers = Vec::new();
                for (index, regions) in tree.handlers.iter().enumerate() {
                    let initial = intersect_states(&snapshots[index])
                        .ok_or(Error::Unsupported("unreachable catch"))?;
                    let (body, exit) = lower_regions(p, regions, Some(initial), None, &mut [])?;
                    exits.extend(exit);
                    handlers.push(RegionHandler {
                        ty: types[base + index].clone(),
                        exception: handler_values[base + index],
                        body,
                    });
                }
                state = intersect_states(&exits);
                out.push(RegionStep::Try { body, handlers });
            }
        }
    }
    Ok((out, state))
}

fn prepare_region<'a>(
    ir: &'a MethodIr<SsaForm>,
    bindings: &[ValueId],
    declarations: &HashMap<ValueId, JavaType>,
) -> Result<(Planner<'a>, HashSet<ValueId>, HashSet<ValueId>), Error> {
    let mut p = Planner {
        ir,
        copies: HashMap::new(),
        phi: HashSet::new(),
        handler: HashMap::new(),
        bindings: bindings.iter().copied().collect(),
    };
    let mut defs = HashSet::new();
    let mut placed = HashSet::new();
    for b in ir.cfg().block_ids() {
        let (id, i) = p.insn(b)?;
        if !placed.insert(id) {
            return Err(Error::Unsupported("duplicate instruction placement"));
        }
        for v in defined(i) {
            if !defs.insert(v) || ir.value(v).map_err(invalid)?.def() != id {
                return Err(Error::Unsupported("invalid SSA definition"));
            }
        }
        let normal: Vec<_> = ir
            .cfg()
            .block(b)
            .map_err(invalid)?
            .outgoing()
            .iter()
            .filter_map(|e| {
                let edge = ir.cfg().edge(*e).ok()?;
                (!matches!(edge.kind(), EdgeKind::Exception(_))).then_some(*e)
            })
            .collect();
        match i {
            ManagedInsn::Branch {
                condition: Some(c),
                then_edge,
                else_edge: Some(no),
            } if ir.value(*c).map_err(invalid)?.ty()
                == &managed_ir::types::ManagedType::Boolean
                && normal.len() == 2
                && normal.contains(then_edge)
                && normal.contains(no)
                && ir.cfg().edge(*then_edge).map_err(invalid)?.kind() == &EdgeKind::BranchTrue
                && ir.cfg().edge(*no).map_err(invalid)?.kind() == &EdgeKind::BranchFalse => {}
            ManagedInsn::Branch {
                condition: None,
                then_edge,
                else_edge: None,
            } if normal == [*then_edge]
                && ir.cfg().edge(*then_edge).map_err(invalid)?.kind()
                    == &EdgeKind::Unconditional => {}
            ManagedInsn::Branch { .. } => {
                return Err(Error::Unsupported("branch instruction/edge mismatch"));
            }
            ManagedInsn::Return { .. } | ManagedInsn::Throw { .. } if !normal.is_empty() => {
                return Err(Error::Unsupported("terminal has normal successors"));
            }
            _ => {}
        }
    }
    let mut initial = HashSet::new();
    for &v in bindings {
        let value = ir.value(v).map_err(invalid)?;
        let synthetic = matches!(ir.instruction(value.def()).map_err(invalid)?, ManagedInsn::Opaque { family_tag: 0x8001 | 0x8003, reads, writes, effects } if reads.is_empty() && writes.as_slice() == [v] && *effects == EffectFlags::NONE);
        if (defs.contains(&v) && !synthetic) || !initial.insert(v) {
            return Err(Error::Unsupported("invalid parameter binding"));
        }
        defs.remove(&v);
    }
    for b in ir.cfg().block_ids() {
        if !matches!(p.insn(b)?.1, ManagedInsn::Phi { .. }) || p.phi.contains(&b) {
            continue;
        }
        let incoming = ir.cfg().block(b).map_err(invalid)?.incoming().to_vec();
        if let [id] = incoming.as_slice() {
            let e = ir.cfg().edge(*id).map_err(invalid)?;
            if e.kind() == &EdgeKind::Unconditional
                && ir.cfg().block(e.source()).map_err(invalid)?.outgoing() == incoming
                && matches!(p.insn(e.source())?.1, ManagedInsn::Phi { .. })
            {
                continue;
            }
        }
        let mut current = b;
        while let ManagedInsn::Phi { result, inputs } = p.insn(current)?.1.clone() {
            if inputs.len() != incoming.len()
                || inputs.iter().map(|(e, _)| *e).collect::<HashSet<_>>()
                    != incoming.iter().copied().collect()
            {
                return Err(Error::Unsupported("Phi edge coverage"));
            }
            for (edge, source) in inputs {
                // This map is produced atomically by checked_phi_plan from
                // every family definition and whole-arena consumer. It changes
                // declaration typing only; edge availability remains mandatory.
                if ir.value(source).map_err(invalid)?.ty()
                    != ir.value(result).map_err(invalid)?.ty()
                    && !declarations.contains_key(&result)
                {
                    return Err(Error::Invalid(format!(
                        "Phi type mismatch at {current:?}: result {result:?} {:?}, input {source:?} {:?}, edge {edge:?}; checked declaration {:?}",
                        ir.value(result).map_err(invalid)?.ty(),
                        ir.value(source).map_err(invalid)?.ty(),
                        declarations.get(&result),
                    )));
                }
                p.copies.entry(edge).or_default().push(Copy {
                    destination: result,
                    source,
                });
            }
            p.phi.insert(current);
            let out = ir.cfg().block(current).map_err(invalid)?.outgoing();
            let [id] = out else {
                break;
            };
            let e = ir.cfg().edge(*id).map_err(invalid)?;
            if e.kind() != &EdgeKind::Unconditional
                || ir.cfg().block(e.target()).map_err(invalid)?.incoming() != out
            {
                break;
            }
            current = e.target();
        }
    }
    Ok((p, defs, initial))
}

fn region_blocks(regions: &[scopes::Region], blocks: &mut Vec<BlockId>) {
    for r in regions {
        match r {
            scopes::Region::Block(b) => blocks.push(*b),
            scopes::Region::If {
                head,
                then_body,
                else_body,
                ..
            } => {
                blocks.push(*head);
                region_blocks(then_body, blocks);
                region_blocks(else_body, blocks);
            }
        }
    }
}

fn intersect_states(states: &[HashSet<ValueId>]) -> Option<HashSet<ValueId>> {
    let mut state = states.first()?.clone();
    for s in &states[1..] {
        state.retain(|v| s.contains(v));
    }
    Some(state)
}

type RegionFlow = (Vec<RegionStep>, Option<HashSet<ValueId>>);
fn lower_regions(
    p: &Planner<'_>,
    regions: &[scopes::Region],
    mut state: Option<HashSet<ValueId>>,
    scope: Option<&scopes::RegionScope>,
    exceptional: &mut [Vec<HashSet<ValueId>>],
) -> Result<RegionFlow, Error> {
    let mut out = Vec::new();
    for region in regions {
        let available = state
            .as_mut()
            .ok_or(Error::Unsupported("instruction after abrupt completion"))?;
        let b = match region {
            scopes::Region::Block(b) => *b,
            scopes::Region::If { head, .. } => *head,
        };
        let (id, insn) = p.insn(b)?;
        let exception_edges: Vec<_> =
            p.ir.cfg()
                .block(b)
                .map_err(invalid)?
                .outgoing()
                .iter()
                .filter_map(|id| {
                    let e = p.ir.cfg().edge(*id).ok()?;
                    matches!(e.kind(), EdgeKind::Exception(_)).then_some((*id, e.target()))
                })
                .collect();
        if let Some(scope) = scope {
            let throwing = may_throw(p.ir, insn)
                || matches!(
                    insn,
                    ManagedInsn::MonitorEnter { .. } | ManagedInsn::MonitorExit { .. }
                );
            if throwing == exception_edges.is_empty() {
                return Err(Error::Unsupported("throwing-site metadata mismatch"));
            }
            for (edge, target) in exception_edges {
                let index = scope
                    .catches
                    .iter()
                    .position(|c| c.entry == target)
                    .ok_or(Error::Unsupported("missing catch target"))?;
                let mut snapshot = available.clone();
                if let Some(Step::Copies(c)) = p.transfer(edge, &mut snapshot)? {
                    out.push(RegionStep::Copies(c));
                }
                exceptional[index].push(snapshot);
            }
        }
        if let scopes::Region::If {
            then_edge,
            else_edge,
            then_body,
            else_body,
            ..
        } = region
        {
            let ManagedInsn::Branch {
                condition: Some(condition),
                ..
            } = insn
            else {
                return Err(Error::Unsupported("conditional region lacks branch"));
            };
            if !available.contains(condition) {
                return Err(Error::Unavailable(*condition));
            }
            let mut yes = available.clone();
            let mut no = available.clone();
            let mut y = Vec::new();
            let mut n = Vec::new();
            if let Some(Step::Copies(c)) = p.transfer(*then_edge, &mut yes)? {
                y.push(RegionStep::Copies(c));
            }
            if let Some(Step::Copies(c)) = p.transfer(*else_edge, &mut no)? {
                n.push(RegionStep::Copies(c));
            }
            let (yb, ys) = lower_regions(p, then_body, Some(yes), scope, exceptional)?;
            let (nb, ns) = lower_regions(p, else_body, Some(no), scope, exceptional)?;
            y.extend(yb);
            n.extend(nb);
            state = intersect_states(&ys.into_iter().chain(ns).collect::<Vec<_>>());
            out.push(RegionStep::If {
                condition: *condition,
                then_body: y,
                else_body: n,
            });
            continue;
        }
        if p.phi.contains(&b) {
            for v in defined(insn) {
                if !available.contains(&v) {
                    return Err(Error::Unavailable(v));
                }
            }
        } else if let Some((_, exception)) = p.handler.get(&b) {
            available.insert(*exception);
        } else if matches!(insn, ManagedInsn::Opaque { family_tag: 0x8001 | 0x8003, reads, writes, effects } if reads.is_empty() && writes.len() == 1 && p.bindings.contains(&writes[0]) && *effects == EffectFlags::NONE)
        {
        } else {
            if matches!(
                insn,
                ManagedInsn::Opaque { .. } | ManagedInsn::Switch { .. }
            ) {
                return Err(Error::Unsupported("unsupported instruction"));
            }
            for v in reads(insn) {
                if !available.contains(&v) {
                    return Err(Error::Unavailable(v));
                }
            }
            available.extend(defined(insn));
            if !matches!(insn, ManagedInsn::Branch { .. }) {
                out.push(RegionStep::Instruction(id));
            }
        }
        for &edge in p.ir.cfg().block(b).map_err(invalid)?.outgoing() {
            if p.ir.cfg().edge(edge).map_err(invalid)?.kind() == &EdgeKind::Unconditional
                && let Some(Step::Copies(c)) = p.transfer(edge, available)?
            {
                out.push(RegionStep::Copies(c));
            }
        }
        if matches!(insn, ManagedInsn::Return { .. } | ManagedInsn::Throw { .. }) {
            state = None;
        }
    }
    Ok((out, state))
}

fn recover_cleanup(
    p: &Planner<'_>,
    metadata: &(impl RegionMetadata + ?Sized),
    prefix: &mut Vec<RegionStep>,
    region: &mut RegionStep,
    suffix: &mut Vec<RegionStep>,
) -> Result<(), Error> {
    let RegionStep::Try { body, handlers } = region else {
        return Ok(());
    };
    // Multiple typed handlers with a cleanup need a nested source scope proof.
    if handlers.len() != 1 || !metadata.is_catch_all(&metadata.region_scope().catches[0].key) {
        return Ok(());
    }
    let handler = &handlers[0];
    let Some(RegionStep::Instruction(rethrow)) = handler.body.last() else {
        return Ok(());
    };
    let Some(mut captured) = handler.exception else {
        return Ok(());
    };
    let mut first = 0;
    while let Some(RegionStep::Instruction(id)) = handler.body.get(first) {
        if let ManagedInsn::Move { result, value } = p.ir.instruction(*id).map_err(invalid)?
            && *value == captured
        {
            captured = *result;
            first += 1;
        } else {
            break;
        }
    }
    if !matches!(p.ir.instruction(*rethrow).map_err(invalid)?, ManagedInsn::Throw { value } if *value == captured)
    {
        return Ok(());
    }
    let cleanup = &handler.body[first..handler.body.len() - 1];
    if cleanup.is_empty() || cleanup.len() > suffix.len() || contains_return(p.ir, body)? {
        return Ok(());
    }
    for (a, b) in cleanup.iter().zip(suffix.iter()) {
        let (RegionStep::Instruction(a), RegionStep::Instruction(b)) = (a, b) else {
            return Ok(());
        };
        let a = p.ir.instruction(*a).map_err(invalid)?;
        let b = p.ir.instruction(*b).map_err(invalid)?;
        // Equality includes method/field identities, argument order and stable
        // SSA operands. Definitions, branches and copies cannot be relocated.
        if a != b
            || !defined(a).is_empty()
            || !matches!(
                a,
                ManagedInsn::Invoke { result: None, .. }
                    | ManagedInsn::FieldPut { .. }
                    | ManagedInsn::MonitorExit { .. }
            )
        {
            return Ok(());
        }
    }
    let cleanup = cleanup.to_vec();
    if let [RegionStep::Instruction(exit)] = cleanup.as_slice()
        && let ManagedInsn::MonitorExit { object } = p.ir.instruction(*exit).map_err(invalid)?
    {
        let Some(RegionStep::Instruction(enter)) = prefix.last() else {
            return Ok(());
        };
        if !matches!(p.ir.instruction(*enter).map_err(invalid)?, ManagedInsn::MonitorEnter { object: acquired } if acquired == object)
        {
            return Ok(());
        }
        let object = *object;
        prefix.pop();
        suffix.drain(..cleanup.len());
        *region = RegionStep::Synchronized {
            object,
            body: body.clone(),
        };
    } else {
        suffix.drain(..cleanup.len());
        *region = RegionStep::Finally {
            body: body.clone(),
            cleanup,
        };
    }
    Ok(())
}

fn contains_monitor(ir: &MethodIr<SsaForm>, steps: &[RegionStep]) -> Result<bool, Error> {
    for step in steps {
        let found = match step {
            RegionStep::Instruction(id) => matches!(
                ir.instruction(*id).map_err(invalid)?,
                ManagedInsn::MonitorEnter { .. } | ManagedInsn::MonitorExit { .. }
            ),
            RegionStep::Copies(_) => false,
            RegionStep::If {
                then_body,
                else_body,
                ..
            } => contains_monitor(ir, then_body)? || contains_monitor(ir, else_body)?,
            RegionStep::Try { body, handlers } => {
                contains_monitor(ir, body)?
                    || handlers
                        .iter()
                        .map(|h| contains_monitor(ir, &h.body))
                        .collect::<Result<Vec<_>, _>>()?
                        .contains(&true)
            }
            RegionStep::Finally { body, cleanup } => {
                contains_monitor(ir, body)? || contains_monitor(ir, cleanup)?
            }
            RegionStep::Synchronized { body, .. } => contains_monitor(ir, body)?,
            RegionStep::TryTransfer { body, .. } => contains_monitor(ir, body)?,
            RegionStep::SharedHandler { body, handler, .. } => {
                contains_monitor(ir, body)? || contains_monitor(ir, handler)?
            }
        };
        if found {
            return Ok(true);
        }
    }
    Ok(false)
}

fn contains_return(ir: &MethodIr<SsaForm>, steps: &[RegionStep]) -> Result<bool, Error> {
    for step in steps {
        match step {
            RegionStep::Instruction(id)
                if matches!(
                    ir.instruction(*id).map_err(invalid)?,
                    ManagedInsn::Return { .. }
                ) =>
            {
                return Ok(true);
            }
            RegionStep::If {
                then_body,
                else_body,
                ..
            } if contains_return(ir, then_body)? || contains_return(ir, else_body)? => {
                return Ok(true);
            }
            _ => {}
        }
    }
    Ok(false)
}

/// Additional transactional rendering callbacks required by RegionPlan. The
/// parent uses its ordinary typed emitter for instructions/copies; condition and
/// monitor callbacks must check Boolean and reference types respectively.
pub trait RegionEmitter: TypedEmitter {
    /// Prepare exact source catch rendering without adding throwable operations.
    fn prepare_exception_scope(
        &mut self,
        body: &[RegionStep],
        types: &[JavaType],
    ) -> Result<(), Self::Error>;

    /// Declare a definitely-assigned captured local and open a forward label.
    fn begin_shared_handler(
        &mut self,
        ty: &JavaType,
        exception: ValueId,
    ) -> Result<(), Self::Error>;
    /// Assign the common capture in its original typed catch and break forward.
    fn transfer_shared_handler(
        &mut self,
        ty: &JavaType,
        exception: ValueId,
    ) -> Result<(), Self::Error>;

    /// Apply proved caught-object types before hoisting any local.
    fn contextual_types(&mut self, types: &[(ValueId, JavaType)]) -> Result<(), Self::Error>;
    /// Open a catch with an optional SSA binding; None means the original
    /// handler discards its caught object.
    fn begin_region_catch(
        &mut self,
        ty: &JavaType,
        exception: Option<ValueId>,
    ) -> Result<(), Self::Error>;
    /// Set checked-exception allowances for the next try only. Finally and
    /// synchronized do not catch checked exceptions and receive an empty list.
    fn catch_allowances(&mut self, types: &[JavaType]) -> Result<(), Self::Error>;
    /// Open an if on the existing SSA Boolean local.
    fn begin_if(&mut self, condition: ValueId) -> Result<(), Self::Error>;
    /// Switch arms.
    fn begin_else(&mut self) -> Result<(), Self::Error>;
    /// Close an if or synchronized body.
    fn end_region(&mut self) -> Result<(), Self::Error>;
    /// Open finally, disabling catch allowances for its instructions.
    fn begin_finally(&mut self) -> Result<(), Self::Error>;
    /// Open synchronized on the materialized reference.
    fn begin_synchronized(&mut self, object: ValueId) -> Result<(), Self::Error>;
}

/// Render a complete extended plan into the parent's transactional buffer.
pub fn emit_region<E: RegionEmitter>(plan: &RegionPlan, emitter: &mut E) -> Result<(), E::Error> {
    emitter.contextual_types(&plan.contextual_types)?;
    emitter.hoist(&plan.locals)?;
    render_regions(&plan.body, emitter)
}
fn render_regions<E: RegionEmitter>(steps: &[RegionStep], e: &mut E) -> Result<(), E::Error> {
    for s in steps {
        match s {
            RegionStep::Instruction(id) => e.instruction(*id)?,
            RegionStep::Copies(c) => e.copies(c)?,
            RegionStep::If {
                condition,
                then_body,
                else_body,
            } => {
                e.begin_if(*condition)?;
                render_regions(then_body, e)?;
                e.begin_else()?;
                render_regions(else_body, e)?;
                e.end_region()?;
            }
            RegionStep::Try { body, handlers } => {
                e.prepare_exception_scope(
                    body,
                    &handlers.iter().map(|h| h.ty.clone()).collect::<Vec<_>>(),
                )?;
                e.catch_allowances(&handlers.iter().map(|h| h.ty.clone()).collect::<Vec<_>>())?;
                e.begin_try()?;
                render_regions(body, e)?;
                for h in handlers {
                    e.begin_region_catch(&h.ty, h.exception)?;
                    render_regions(&h.body, e)?;
                }
                e.end_catches()?;
            }
            RegionStep::SharedHandler {
                ty,
                exception,
                body,
                handler,
            } => {
                e.begin_shared_handler(ty, *exception)?;
                render_regions(body, e)?;
                e.end_region()?;
                render_regions(handler, e)?;
            }
            RegionStep::TryTransfer {
                body,
                ty,
                exception,
            } => {
                e.prepare_exception_scope(body, std::slice::from_ref(ty))?;
                e.catch_allowances(std::slice::from_ref(ty))?;
                e.begin_try()?;
                render_regions(body, e)?;
                e.transfer_shared_handler(ty, *exception)?;
                e.end_catches()?;
            }
            RegionStep::Finally { body, cleanup } => {
                e.prepare_exception_scope(body, &[])?;
                e.catch_allowances(&[])?;
                e.begin_try()?;
                render_regions(body, e)?;
                e.begin_finally()?;
                render_regions(cleanup, e)?;
                e.end_catches()?;
            }
            RegionStep::Synchronized { object, body } => {
                e.catch_allowances(&[])?;
                e.begin_synchronized(*object)?;
                render_regions(body, e)?;
                e.end_region()?;
            }
        }
    }
    Ok(())
}

/// Production transactional declaration entry point for the extended grammar.
/// The parent facade can call this directly without adding rendering callbacks.
pub fn emit_method_region(
    ir: &MethodIr<SsaForm>,
    method: &crate::JavaMethod,
    bindings: &crate::DeclarationBindings,
    resolver: &impl JavaResolver,
    metadata: &(impl RegionMetadata + ?Sized),
) -> crate::SourceResult {
    match emit_region_declaration(ir, method, bindings, resolver, metadata, None) {
        Ok((source, method_name, parameter_names)) => crate::SourceResult::Complete {
            source,
            method_name,
            parameter_names,
        },
        Err(e) => crate::SourceResult::Unsupported(e),
    }
}

/// Transactional declaration emission for disjoint source scopes inside branches.
pub fn emit_method_disjoint(
    ir: &MethodIr<SsaForm>,
    method: &crate::JavaMethod,
    bindings: &crate::DeclarationBindings,
    resolver: &impl JavaResolver,
    metadata: &[&dyn RegionMetadata],
) -> crate::SourceResult {
    let Some(first) = metadata.first() else {
        return crate::SourceResult::Unsupported(crate::Unsupported::Metadata(
            "empty exception scopes".into(),
        ));
    };
    match emit_region_declaration(ir, method, bindings, resolver, *first, Some(metadata)) {
        Ok((source, method_name, parameter_names)) => crate::SourceResult::Complete {
            source,
            method_name,
            parameter_names,
        },
        Err(e) => crate::SourceResult::Unsupported(e),
    }
}

fn emit_region_declaration(
    ir: &MethodIr<SsaForm>,
    m: &crate::JavaMethod,
    b: &crate::DeclarationBindings,
    r: &impl JavaResolver,
    metadata: &(impl RegionMetadata + ?Sized),
    disjoint: Option<&[&dyn RegionMetadata]>,
) -> Result<(String, String, Vec<String>), crate::Unsupported> {
    use crate::{Unsupported, managed_type, reserve_class, reserve_type};
    crate::qualified(&m.owner)?;
    if m.name.starts_with('<')
        || b.parameters.len() != m.parameters.len()
        || (!b.names.is_empty() && b.names.len() != m.parameters.len())
        || (m.is_static && b.receiver.is_some())
    {
        return Err(Unsupported::Metadata(
            "exception declaration binding/signature mismatch".into(),
        ));
    }
    let mut reserved = HashSet::from(["java".to_string()]);
    reserve_class(&mut reserved, &m.owner);
    reserve_type(&mut reserved, &m.return_type);
    for t in &m.parameters {
        reserve_type(&mut reserved, t);
    }
    for v in ir.values().values() {
        if let Ok(t) = managed_type(v.ty(), r) {
            reserve_type(&mut reserved, &t);
        }
    }
    for i in ir.instructions().values() {
        match i {
            ManagedInsn::Invoke { method, .. } => {
                if let Some(t) = r.method(*method) {
                    reserve_class(&mut reserved, &t.owner);
                    reserve_type(&mut reserved, &t.return_type);
                    for p in &t.parameters {
                        reserve_type(&mut reserved, p);
                    }
                }
            }
            ManagedInsn::FieldGet { field, .. } | ManagedInsn::FieldPut { field, .. } => {
                if let Some(f) = r.field(*field) {
                    reserve_class(&mut reserved, &f.owner);
                    reserve_type(&mut reserved, &f.ty);
                }
            }
            ManagedInsn::Cast { target, .. } | ManagedInsn::InstanceOf { target, .. } => {
                if let Some(t) = r.instruction_type(*target) {
                    reserve_type(&mut reserved, &t);
                }
            }
            _ => {}
        }
    }
    if let Some(all) = disjoint {
        for md in all {
            for c in &md.region_scope().catches {
                if let Some(t) = md.catch_type(&c.key) {
                    reserve_type(&mut reserved, &t);
                }
            }
        }
    } else {
        for c in &metadata.region_scope().catches {
            if let Some(t) = metadata.catch_type(&c.key) {
                reserve_type(&mut reserved, &t);
            }
        }
    }
    let scoped_resolver = allocation_proofs::ScopedResolver::new(r);
    let mut e = crate::Emitter {
        allocation_readable: None,
        object_planning: Default::default(),
        ir,
        resolver: &scoped_resolver,
        values: HashMap::new(),
        lines: Vec::new(),
        next: 0,
        parameters: HashSet::new(),
        reserved,
        current: None,
        hoisted: HashSet::new(),
        declarations: match disjoint {
            Some(tables) => dispatch::declarations_for_scopes(ir, &m.return_type, r, tables),
            None => dispatch::declarations_for_scopes(ir, &m.return_type, r, &[metadata]),
        }?,
    };
    let mut names = Vec::new();
    let mut params = Vec::new();
    for (index, (value, ty)) in b.parameters.iter().zip(&m.parameters).enumerate() {
        if *ty == JavaType::Void {
            return Err(Unsupported::Metadata("void parameter".into()));
        }
        let hint = b
            .names
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("p{index}"));
        let mut name = crate::sanitize_identifier(&hint);
        if name
            .strip_prefix('v')
            .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        {
            name.insert_str(0, "p_");
        }
        let base = name.clone();
        let mut suffix = 0;
        while e.reserved.contains(&name) {
            suffix += 1;
            name = format!("{base}_{suffix}");
        }
        e.reserved.insert(name.clone());
        if let Some(v) = value {
            crate::bind(&mut e, *v, name.clone(), ty.clone())?;
        }
        params.push(format!("{} {name}", ty.source()?));
        names.push(name);
    }
    if let Some(v) = b.receiver {
        crate::bind(&mut e, v, "this".into(), JavaType::Class(m.owner.clone()))?;
    }
    let bound: Vec<_> = e.parameters.iter().copied().collect();
    let plan = match disjoint {
        Some(all) => plan_disjoint_with_declarations(ir, all, &bound, r, &e.declarations),
        None => plan_region_with_declarations(ir, metadata, &bound, r, &e.declarations),
    }
    .map_err(|error| {
        if !matches!(&error, Error::Invalid(detail) if detail.starts_with("Phi type mismatch")) {
            return Unsupported::ControlFlow(error.to_string());
        }
        let details = ir
            .instructions()
            .values()
            .filter_map(|insn| {
                let ManagedInsn::Phi { result, .. } = insn else {
                    return None;
                };
                if e.declarations.contains_key(result) {
                    return None;
                }
                let managed_ir::types::ManagedType::Reference(class) = ir.value(*result).ok()?.ty()
                else {
                    return None;
                };
                let declared = JavaType::Class(r.class(*class)?);
                let proof = crate::contextual_locals::checked_phi_plan(
                    ir,
                    *result,
                    &declared,
                    &m.return_type,
                    metadata.hierarchy(),
                    r,
                );
                Some(format!("{result:?} candidate {declared:?}: {proof:?}"))
            })
            .collect::<Vec<_>>();
        Unsupported::ControlFlow(format!("{error}; contextual proofs {details:?}"))
    })?;
    let borrowed_metadata = BorrowedMetadata(metadata);
    let original_scopes: Vec<&dyn RegionMetadata> = match disjoint {
        Some(all) => all.to_vec(),
        None => vec![&borrowed_metadata],
    };
    scoped_resolver.install(allocation_proofs::prove(ir, &original_scopes, r));
    let mut renderer = NativeRegionEmitter {
        inner: crate::exception_emitter::ExceptionEmitter {
            emitter: &mut e,
            method: m,
            caught: Vec::new(),
            protected: false,
        },
        contextual: HashMap::new(),
        shared_label: None,
        guard_scopes: Vec::new(),
        hierarchy: metadata.hierarchy(),
    };
    emit_region(&plan, &mut renderer)?;
    let name = crate::sanitize_identifier(&m.name);
    let throws = if m.throws.is_empty() {
        String::new()
    } else {
        format!(
            " throws {}",
            m.throws
                .iter()
                .map(|n| crate::qualified(n))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        )
    };
    let source = format!(
        "public {}{} {name}({}){throws} {{\n{}\n}}\n",
        if m.is_static { "static " } else { "" },
        m.return_type.source()?,
        params.join(", "),
        e.lines
            .iter()
            .map(|s| format!("    {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    Ok((source, name, names))
}

struct NativeRegionEmitter<'e, 'a, R> {
    inner: crate::exception_emitter::ExceptionEmitter<'e, 'a, R>,
    contextual: HashMap<ValueId, JavaType>,
    shared_label: Option<String>,
    guard_scopes: Vec<Option<Option<String>>>,
    hierarchy: &'a crate::type_compatibility::Hierarchy,
}
impl<R: JavaResolver> TypedEmitter for NativeRegionEmitter<'_, '_, R> {
    type Error = crate::Unsupported;
    fn hoist(&mut self, locals: &[ValueId]) -> Result<(), Self::Error> {
        for &v in locals {
            // Hoisted object slots remain unreadable until the paired call;
            // their declarations preserve scope, never allocation timing.
            let ty = match self.contextual.get(&v) {
                Some(t) => t.clone(),
                None => self.inner.emitter.ty(v)?,
            };
            let e = &mut self.inner.emitter;
            let mut name = format!("v{}", e.next);
            e.next += 1;
            while e.reserved.contains(&name) {
                name = format!("v{}", e.next);
                e.next += 1;
            }
            let default = match ty {
                JavaType::Void => {
                    return Err(crate::Unsupported::Type("void exception local".into()));
                }
                JavaType::Boolean => "false",
                JavaType::Long => "0L",
                JavaType::Float => "0.0f",
                JavaType::Double => "0.0d",
                JavaType::Class(_) | JavaType::Array(_) => "null",
                _ => "0",
            };
            e.lines
                .push(format!("{} {name} = {default};", ty.source()?));
            e.reserved.insert(name.clone());
            e.values.insert(v, (name, ty));
            e.hoisted.insert(v);
        }
        Ok(())
    }
    fn instruction(&mut self, id: InsnId) -> Result<(), Self::Error> {
        if let ManagedInsn::Const {
            result,
            value: managed_ir::ops::Constant::Zero | managed_ir::ops::Constant::Null,
        } = self
            .inner
            .emitter
            .ir
            .instruction(id)
            .map_err(|e| crate::Unsupported::Instruction(e.to_string()))?
            && self.contextual.contains_key(result)
        {
            let (name, ty) = self.inner.emitter.get(*result)?;
            if !matches!(ty, JavaType::Class(_) | JavaType::Array(_)) {
                return Err(crate::Unsupported::Type(
                    "non-reference contextual null".into(),
                ));
            }
            self.inner.emitter.lines.push(format!("{name} = null;"));
            return Ok(());
        }
        if let ManagedInsn::Throw { value } = self
            .inner
            .emitter
            .ir
            .instruction(id)
            .map_err(|e| crate::Unsupported::Instruction(e.to_string()))?
        {
            let (name, ty) = self.inner.emitter.get(*value)?;
            if crate::proven_null_throw(self.inner.emitter.ir, *value) {
                self.inner.emitter.lines.push("throw null;".into());
                return Ok(());
            }
            let r = self.inner.emitter.resolver;
            if !r.assignable(&ty, &JavaType::Class("java.lang.Throwable".into())) {
                return Err(crate::Unsupported::Type(
                    "throw value is not proven Throwable".into(),
                ));
            }
            let unchecked = ["java.lang.RuntimeException", "java.lang.Error"]
                .iter()
                .any(|n| r.assignable(&ty, &JavaType::Class((*n).into())));
            let declared = self
                .inner
                .method
                .throws
                .iter()
                .any(|n| r.assignable(&ty, &JavaType::Class(n.clone())));
            let caught =
                self.inner.protected && self.inner.caught.iter().any(|t| r.assignable(&ty, t));
            if !unchecked && !declared && !caught {
                return Err(crate::Unsupported::Metadata(
                    "handler/body throw checked exceptions are not covered".into(),
                ));
            }
            self.inner.emitter.lines.push(format!("throw {name};"));
            return Ok(());
        }
        if let ManagedInsn::Move { result, value } = self
            .inner
            .emitter
            .ir
            .instruction(id)
            .map_err(|e| crate::Unsupported::Instruction(e.to_string()))?
            && self.contextual.contains_key(result)
        {
            let (destination, expected) = self.inner.emitter.get(*result)?;
            let (source, actual) = self.inner.emitter.get(*value)?;
            self.inner.emitter.compatible(&actual, &expected)?;
            self.inner
                .emitter
                .lines
                .push(format!("{destination} = {source};"));
            return Ok(());
        }
        self.inner.instruction(id)
    }
    fn copies(&mut self, copies: &[Copy]) -> Result<(), Self::Error> {
        self.inner.copies(copies)
    }
    fn begin_try(&mut self) -> Result<(), Self::Error> {
        self.inner.begin_try()
    }
    fn begin_catch(&mut self, ty: &JavaType, exception: ValueId) -> Result<(), Self::Error> {
        self.inner.begin_catch(ty, exception)
    }
    fn end_catches(&mut self) -> Result<(), Self::Error> {
        if let Some(Some(Some(caught))) = self.guard_scopes.pop() {
            self.inner
                .emitter
                .lines
                .push(format!("}} else {{ throw {caught}; }}"));
        }
        self.inner.end_catches()
    }
}
impl<R: JavaResolver> RegionEmitter for NativeRegionEmitter<'_, '_, R> {
    fn prepare_exception_scope(
        &mut self,
        body: &[RegionStep],
        types: &[JavaType],
    ) -> Result<(), Self::Error> {
        let eligible = checked_guards::eligible(
            self.inner.emitter.ir,
            body,
            types,
            self.hierarchy,
            self.inner.emitter.resolver,
        );
        self.guard_scopes.push(eligible.then_some(None));
        Ok(())
    }

    fn begin_shared_handler(
        &mut self,
        ty: &JavaType,
        exception: ValueId,
    ) -> Result<(), Self::Error> {
        if self.shared_label.is_some() {
            return Err(crate::Unsupported::ControlFlow(
                "nested shared handler".into(),
            ));
        }
        let e = &mut self.inner.emitter;
        let mut local = format!("captured{}", e.next);
        e.next += 1;
        while e.reserved.contains(&local) {
            local = format!("captured{}", e.next);
            e.next += 1;
        }
        let mut label = format!("handler{}", e.next);
        e.next += 1;
        while e.reserved.contains(&label) {
            label = format!("handler{}", e.next);
            e.next += 1;
        }
        e.reserved.insert(local.clone());
        e.reserved.insert(label.clone());
        e.lines.push(format!("{} {local};", ty.source()?));
        e.values.insert(exception, (local, ty.clone()));
        e.hoisted.insert(exception);
        e.lines.push(format!("{label}: {{"));
        self.shared_label = Some(label);
        Ok(())
    }
    fn transfer_shared_handler(
        &mut self,
        ty: &JavaType,
        exception: ValueId,
    ) -> Result<(), Self::Error> {
        let label = self.shared_label.as_ref().ok_or_else(|| {
            crate::Unsupported::ControlFlow("missing shared handler label".into())
        })?;
        let (captured, actual) = self.inner.emitter.get(exception)?;
        if actual != *ty {
            return Err(crate::Unsupported::Type(
                "shared capture type mismatch".into(),
            ));
        }
        let e = &mut self.inner.emitter;
        let mut caught = format!("caught{}", e.next);
        e.next += 1;
        while e.reserved.contains(&caught) {
            caught = format!("caught{}", e.next);
            e.next += 1;
        }
        e.reserved.insert(caught.clone());
        let guarded = matches!(self.guard_scopes.last(), Some(Some(_)));
        if guarded {
            e.lines
                .push(format!("}} catch (java.lang.Exception {caught}) {{"));
            e.lines
                .push(format!("if ({caught} instanceof {}) {{", ty.source()?));
            let Some(scope) = self.guard_scopes.last_mut() else {
                return Err(crate::Unsupported::ControlFlow(
                    "missing guarded scope".into(),
                ));
            };
            *scope = Some(Some(caught.clone()));
            e.lines
                .push(format!("{captured} = ({}) {caught};", ty.source()?));
        } else {
            e.lines
                .push(format!("}} catch ({} {caught}) {{", ty.source()?));
            e.lines.push(format!("{captured} = {caught};"));
        }
        e.lines.push(format!("break {label};"));
        self.inner.protected = false;
        Ok(())
    }

    fn contextual_types(&mut self, types: &[(ValueId, JavaType)]) -> Result<(), Self::Error> {
        self.contextual.extend(types.iter().cloned());
        Ok(())
    }
    fn catch_allowances(&mut self, types: &[JavaType]) -> Result<(), Self::Error> {
        self.inner.caught = types.to_vec();
        Ok(())
    }
    fn begin_region_catch(
        &mut self,
        ty: &JavaType,
        exception: Option<ValueId>,
    ) -> Result<(), Self::Error> {
        if matches!(self.guard_scopes.last(), Some(Some(_))) {
            self.inner.protected = false;
            let e = &mut self.inner.emitter;
            let caught = match self.guard_scopes.last().cloned().flatten().flatten() {
                Some(caught) => {
                    e.lines.push(format!(
                        "}} else if ({caught} instanceof {}) {{",
                        ty.source()?
                    ));
                    caught
                }
                None => {
                    let mut caught = format!("caught{}", e.next);
                    e.next += 1;
                    while e.reserved.contains(&caught) {
                        caught = format!("caught{}", e.next);
                        e.next += 1;
                    }
                    e.reserved.insert(caught.clone());
                    e.lines
                        .push(format!("}} catch (java.lang.Exception {caught}) {{"));
                    e.lines
                        .push(format!("if ({caught} instanceof {}) {{", ty.source()?));
                    let Some(scope) = self.guard_scopes.last_mut() else {
                        return Err(crate::Unsupported::ControlFlow(
                            "missing guarded scope".into(),
                        ));
                    };
                    *scope = Some(Some(caught.clone()));
                    caught
                }
            };
            if let Some(value) = exception {
                let (local, expected) = e.get(value)?;
                e.compatible(ty, &expected)?;
                e.lines
                    .push(format!("{local} = ({}) {caught};", ty.source()?));
            }
            return Ok(());
        }
        if let Some(v) = exception {
            return self.inner.begin_catch(ty, v);
        }
        self.inner.protected = false;
        let e = &mut self.inner.emitter;
        let mut name = format!("caught{}", e.next);
        e.next += 1;
        while e.reserved.contains(&name) {
            name = format!("caught{}", e.next);
            e.next += 1;
        }
        e.reserved.insert(name.clone());
        e.lines
            .push(format!("}} catch ({} {name}) {{", ty.source()?));
        Ok(())
    }
    fn begin_if(&mut self, condition: ValueId) -> Result<(), Self::Error> {
        let (name, ty) = self.inner.emitter.get(condition)?;
        if ty != JavaType::Boolean {
            return Err(crate::Unsupported::Type(
                "non-Boolean exception condition".into(),
            ));
        }
        self.inner.emitter.lines.push(format!("if ({name}) {{"));
        Ok(())
    }
    fn begin_else(&mut self) -> Result<(), Self::Error> {
        self.inner.emitter.lines.push("} else {".into());
        Ok(())
    }
    fn end_region(&mut self) -> Result<(), Self::Error> {
        self.inner.emitter.lines.push("}".into());
        Ok(())
    }
    fn begin_finally(&mut self) -> Result<(), Self::Error> {
        self.inner.protected = false;
        self.inner.emitter.lines.push("} finally {".into());
        Ok(())
    }
    fn begin_synchronized(&mut self, object: ValueId) -> Result<(), Self::Error> {
        self.inner.protected = false;
        let (name, ty) = self.inner.emitter.get(object)?;
        if !matches!(ty, JavaType::Class(_) | JavaType::Array(_)) {
            return Err(crate::Unsupported::Type("non-reference monitor".into()));
        }
        self.inner
            .emitter
            .lines
            .push(format!("synchronized ({name}) {{"));
        Ok(())
    }
}

/// Parent integration contract. All callbacks use stable typed SSA locals.
/// `copies` MUST snapshot all sources before assigning any destination, and
/// cannot throw. `instruction` MUST materialize once, assigning hoisted results;
/// it must reject unsupported instructions/types and check checked-exception
/// coverage against the resolved catch types (or declaration throws).
pub trait TypedEmitter {
    /// Parent's transactional rendering failure.
    type Error;
    /// Declare stable locals without moving original initializers.
    fn hoist(&mut self, locals: &[ValueId]) -> Result<(), Self::Error>;
    /// Emit an ordinary instruction with dynamic checked-exception validation.
    fn instruction(&mut self, id: InsnId) -> Result<(), Self::Error>;
    /// Emit nonthrowing simultaneous copies.
    fn copies(&mut self, copies: &[Copy]) -> Result<(), Self::Error>;
    /// Activate the protected catch scope.
    fn begin_try(&mut self) -> Result<(), Self::Error>;
    /// Close preceding try/catch and open this catch, binding `exception` to
    /// the caught Java object, without emitting TAG_HANDLER as an instruction.
    fn begin_catch(&mut self, ty: &JavaType, exception: ValueId) -> Result<(), Self::Error>;
    /// Close the final catch and deactivate its allowances.
    fn end_catches(&mut self) -> Result<(), Self::Error>;
}
/// Emit only into a transactional buffer: discard it if ANY callback fails.
pub fn emit<E: TypedEmitter>(plan: &ExceptionPlan, emitter: &mut E) -> Result<(), E::Error> {
    emitter.hoist(&plan.locals)?;
    render(&plan.prefix, emitter)?;
    emitter.begin_try()?;
    render(&plan.protected, emitter)?;
    for h in &plan.handlers {
        emitter.begin_catch(&h.ty, h.exception)?;
        render(&h.steps, emitter)?;
    }
    emitter.end_catches()?;
    render(&plan.suffix, emitter)
}
fn render<E: TypedEmitter>(steps: &[Step], e: &mut E) -> Result<(), E::Error> {
    for s in steps {
        match s {
            Step::Instruction(id) => e.instruction(*id)?,
            Step::Copies(c) => e.copies(c)?,
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod region_tests {
    use super::*;
    use crate::{
        DeclarationBindings, JavaMethod, SourceResult,
        type_compatibility::{Declaration, Hierarchy},
    };
    use decomp_core::cfg::Cfg;
    use managed_ir::{
        method::{InsnArena, ValueArena},
        ops::{Constant, InvokeKind},
        refs::{ClassRef, FieldRef, MethodRef, StringRef},
        types::ManagedType,
        value::Value,
    };

    struct Metadata {
        scope: scopes::RegionScope,
        hierarchy: Hierarchy,
        all: bool,
    }
    impl RegionMetadata for Metadata {
        fn region_scope(&self) -> &scopes::RegionScope {
            &self.scope
        }
        fn hierarchy(&self) -> &Hierarchy {
            &self.hierarchy
        }
        fn catch_type(&self, key: &str) -> Option<JavaType> {
            Some(JavaType::Class(
                match key {
                    "narrow" => "java.lang.ArithmeticException",
                    "broad" => "java.lang.RuntimeException",
                    _ => "java.lang.Throwable",
                }
                .into(),
            ))
        }
        fn is_catch_all(&self, _: &str) -> bool {
            self.all
        }
    }
    fn hierarchy() -> Hierarchy {
        let mut h = Hierarchy::default();
        for (n, p) in [
            ("java.lang.Throwable", "java.lang.Object"),
            ("java.lang.Exception", "java.lang.Throwable"),
            ("java.lang.RuntimeException", "java.lang.Exception"),
            (
                "java.lang.ArithmeticException",
                "java.lang.RuntimeException",
            ),
            (
                "java.lang.IllegalArgumentException",
                "java.lang.RuntimeException",
            ),
        ] {
            h.insert(Declaration {
                name: n.into(),
                superclass: Some(p.into()),
                interfaces: vec![],
                is_interface: false,
            })
            .unwrap();
        }
        h
    }
    struct Resolver;
    impl JavaResolver for Resolver {
        fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
            hierarchy().assignable(a, b) == crate::type_compatibility::Compatibility::Assignable
        }
        fn class(&self, r: ClassRef) -> Option<String> {
            Some(
                if r.index() == 0 {
                    "java.lang.Object"
                } else {
                    "java.lang.Throwable"
                }
                .into(),
            )
        }
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            Some(JavaMethod {
                owner: "RegionRuntime".into(),
                name: if r.index() == 0 { "work" } else { "cleanup" }.into(),
                is_static: true,
                return_type: if r.index() == 0 {
                    JavaType::Int
                } else {
                    JavaType::Void
                },
                parameters: if r.index() == 0 {
                    vec![JavaType::Int, JavaType::Class("java.lang.Object".into())]
                } else {
                    vec![]
                },
                throws: vec![],
            })
        }
        fn field(&self, _: FieldRef) -> Option<crate::JavaField> {
            None
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
    }
    mod contextual_phi_tests {
        include!("exception_contextual_phi_tests.rs");
    }

    mod allocation_scope_tests {
        include!("exception_allocation_scope_tests.rs");
    }

    mod checked_guard_tests {
        include!("exception_checked_guard_tests.rs");
    }

    mod null_throw_region_tests {
        include!("exception_null_throw_region_tests.rs");
    }

    struct Builder {
        cfg: Cfg<InsnId>,
        insns: InsnArena,
        values: ValueArena,
    }
    impl Builder {
        fn new() -> Self {
            let mut insns = InsnArena::with_key();
            let id = insns.insert(ManagedInsn::Return { value: None });
            Self {
                cfg: Cfg::new(id),
                insns,
                values: ValueArena::with_key(),
            }
        }
        fn block(&mut self) -> BlockId {
            let id = self.insns.insert(ManagedInsn::Return { value: None });
            self.cfg.add_block(id)
        }
        fn value(&mut self, b: BlockId, ty: ManagedType) -> ValueId {
            self.values
                .insert(Value::new(*self.cfg.block(b).unwrap().payload(), ty))
        }
        fn set(&mut self, b: BlockId, i: ManagedInsn) {
            self.insns[*self.cfg.block(b).unwrap().payload()] = i;
        }
        fn edge(&mut self, a: BlockId, b: BlockId, kind: EdgeKind) -> EdgeId {
            self.cfg.add_edge(a, b, kind).unwrap()
        }
        fn link(&mut self, a: BlockId, b: BlockId) -> EdgeId {
            self.edge(a, b, EdgeKind::Unconditional)
        }
        fn incoming(&mut self, ty: ManagedType) -> ValueId {
            let id = self.insns.insert(ManagedInsn::Return { value: None });
            let v = self.values.insert(Value::new(id, ty));
            self.insns[id] = ManagedInsn::Opaque {
                family_tag: 0x8001,
                reads: vec![],
                writes: vec![v],
                effects: EffectFlags::NONE,
            };
            v
        }
        fn finish(self) -> MethodIr<SsaForm> {
            MethodIr::<SsaForm>::from_parts(self.cfg, self.insns, self.values)
        }
    }
    fn cleanup_fixture(monitor: bool) -> (MethodIr<SsaForm>, Metadata, DeclarationBindings) {
        cleanup_fixture_inner(monitor, false)
    }
    fn cleanup_fixture_inner(
        monitor: bool,
        mismatch: bool,
    ) -> (MethodIr<SsaForm>, Metadata, DeclarationBindings) {
        let mut b = Builder::new();
        let mode = b.incoming(ManagedType::Int);
        let object = b.incoming(ManagedType::Reference(ClassRef::new(0)));
        let entry = b.cfg.entry();
        let work = if monitor {
            b.set(entry, ManagedInsn::MonitorEnter { object });
            let w = b.block();
            b.link(entry, w);
            w
        } else {
            entry
        };
        let result = b.value(work, ManagedType::Int);
        b.set(
            work,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(result),
                method: MethodRef::new(0),
                args: vec![mode, object],
            },
        );
        let success = b.block();
        b.link(work, success);
        let operation = if monitor {
            ManagedInsn::MonitorExit { object }
        } else {
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(1),
                args: vec![],
            }
        };
        b.set(
            success,
            if mismatch {
                ManagedInsn::Invoke {
                    kind: InvokeKind::Static,
                    result: None,
                    method: MethodRef::new(2),
                    args: vec![],
                }
            } else {
                operation.clone()
            },
        );
        let ret = b.block();
        b.link(success, ret);
        b.set(
            ret,
            ManagedInsn::Return {
                value: Some(result),
            },
        );
        let tag = b.block();
        let caught = b.value(tag, ManagedType::Unknown);
        b.set(
            tag,
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads: vec![],
                writes: vec![caught],
                effects: EffectFlags::MAY_THROW,
            },
        );
        b.edge(work, tag, EdgeKind::Exception(String::new()));
        let moved = b.block();
        let alias = b.value(moved, ManagedType::Unknown);
        b.set(
            moved,
            ManagedInsn::Move {
                result: alias,
                value: caught,
            },
        );
        b.link(tag, moved);
        let cleanup = b.block();
        b.set(cleanup, operation);
        b.link(moved, cleanup);
        let throw = b.block();
        b.set(throw, ManagedInsn::Throw { value: alias });
        b.link(cleanup, throw);
        let scope = scopes::RegionScope {
            protected: vec![work],
            catches: vec![scopes::Catch {
                key: String::new(),
                entry: tag,
            }],
            continuation: Some(success),
        };
        (
            b.finish(),
            Metadata {
                scope,
                hierarchy: hierarchy(),
                all: true,
            },
            DeclarationBindings {
                parameters: vec![Some(mode), Some(object)],
                ..Default::default()
            },
        )
    }
    fn declaration(name: &str) -> JavaMethod {
        JavaMethod {
            owner: "RegionRuntime".into(),
            name: name.into(),
            is_static: true,
            return_type: JavaType::Int,
            parameters: vec![JavaType::Int, JavaType::Class("java.lang.Object".into())],
            throws: vec!["java.lang.Throwable".into()],
        }
    }

    fn branch_fixture(bad_phi: bool) -> (MethodIr<SsaForm>, Metadata, DeclarationBindings) {
        let mut b = Builder::new();
        let choose = b.incoming(ManagedType::Boolean);
        let mode = b.incoming(ManagedType::Int);
        let object = b.incoming(ManagedType::Reference(ClassRef::new(0)));
        let entry = b.cfg.entry();
        let old = b.value(entry, ManagedType::Int);
        b.set(
            entry,
            ManagedInsn::Const {
                result: old,
                value: Constant::Int(10),
            },
        );
        let zero_node = b.block();
        let zero = b.value(zero_node, ManagedType::Int);
        b.set(
            zero_node,
            ManagedInsn::Const {
                result: zero,
                value: Constant::Int(0),
            },
        );
        b.link(entry, zero_node);
        let head = b.block();
        b.link(zero_node, head);
        let t1 = b.block();
        let r1 = b.value(t1, ManagedType::Int);
        b.set(
            t1,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(r1),
                method: MethodRef::new(0),
                args: vec![zero, object],
            },
        );
        let commit = b.block();
        let committed = b.value(commit, ManagedType::Int);
        b.set(
            commit,
            ManagedInsn::Move {
                result: committed,
                value: r1,
            },
        );
        b.link(t1, commit);
        let t2 = b.block();
        let r2 = b.value(t2, ManagedType::Int);
        b.set(
            t2,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(r2),
                method: MethodRef::new(0),
                args: vec![mode, object],
            },
        );
        b.link(commit, t2);
        let t3 = b.block();
        let r3 = b.value(t3, ManagedType::Int);
        b.set(
            t3,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(r3),
                method: MethodRef::new(0),
                args: vec![mode, object],
            },
        );
        let yes = b.edge(head, t1, EdgeKind::BranchTrue);
        let no = b.edge(head, t3, EdgeKind::BranchFalse);
        b.set(
            head,
            ManagedInsn::Branch {
                condition: Some(choose),
                then_edge: yes,
                else_edge: Some(no),
            },
        );
        let join = b.block();
        let output = b.value(join, ManagedType::Int);
        let e2 = b.link(t2, join);
        let e3 = b.link(t3, join);
        b.set(
            join,
            ManagedInsn::Phi {
                result: output,
                inputs: vec![(e2, r2), (e3, r3)],
            },
        );
        let ret = b.block();
        b.link(join, ret);
        b.set(
            ret,
            ManagedInsn::Return {
                value: Some(output),
            },
        );
        let mut catches = Vec::new();
        for (key, increment) in [("narrow", 100), ("broad", 200)] {
            let phi = b.block();
            let saved = b.value(phi, ManagedType::Int);
            let e1 = b.edge(t1, phi, EdgeKind::Exception(key.into()));
            let e2 = b.edge(t2, phi, EdgeKind::Exception(key.into()));
            let e3 = b.edge(t3, phi, EdgeKind::Exception(key.into()));
            b.set(
                phi,
                ManagedInsn::Phi {
                    result: saved,
                    inputs: vec![
                        (e1, if bad_phi { r1 } else { old }),
                        (e2, committed),
                        (e3, old),
                    ],
                },
            );
            let tag = b.block();
            let caught = b.value(tag, ManagedType::Reference(ClassRef::new(1)));
            b.set(
                tag,
                ManagedInsn::Opaque {
                    family_tag: 0x8002,
                    reads: vec![],
                    writes: vec![caught],
                    effects: EffectFlags::NONE,
                },
            );
            b.link(phi, tag);
            let constant = b.block();
            let add = b.value(constant, ManagedType::Int);
            b.set(
                constant,
                ManagedInsn::Const {
                    result: add,
                    value: Constant::Int(increment),
                },
            );
            b.link(tag, constant);
            let sum = b.block();
            let result = b.value(sum, ManagedType::Int);
            b.set(
                sum,
                ManagedInsn::Arithmetic {
                    op: managed_ir::ops::ArithOp::Add,
                    result,
                    lhs: saved,
                    rhs: add,
                },
            );
            b.link(constant, sum);
            let ret = b.block();
            b.set(
                ret,
                ManagedInsn::Return {
                    value: Some(result),
                },
            );
            b.link(sum, ret);
            catches.push(scopes::Catch {
                key: key.into(),
                entry: phi,
            });
        }
        (
            b.finish(),
            Metadata {
                scope: scopes::RegionScope {
                    protected: vec![head, t1, commit, t2, t3],
                    catches,
                    continuation: Some(join),
                },
                hierarchy: hierarchy(),
                all: false,
            },
            DeclarationBindings {
                parameters: vec![Some(choose), Some(mode), Some(object)],
                ..Default::default()
            },
        )
    }
    fn source(
        ir: &MethodIr<SsaForm>,
        metadata: &Metadata,
        bindings: &DeclarationBindings,
        name: &str,
    ) -> String {
        match emit_method_region(ir, &declaration(name), bindings, &Resolver, metadata) {
            SourceResult::Complete { source, .. } => source,
            result => panic!("{result:?}"),
        }
    }
    fn run_java(source: &str) {
        let root = std::env::temp_dir().join(format!(
            "decomphify-region-runtime-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("RegionRuntime.java"), source).unwrap();
        let version = std::process::Command::new("javac")
            .arg("-version")
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&version.stdout).starts_with("javac 17."),
            "JDK17 required: {version:?}"
        );
        let compilation = std::process::Command::new("javac")
            .arg("RegionRuntime.java")
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            compilation.status.success(),
            "{}\n{source}",
            String::from_utf8_lossy(&compilation.stderr)
        );
        let run = std::process::Command::new("java")
            .args(["-cp", ".", "RegionRuntime"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}\n{source}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert!(String::from_utf8_lossy(&run.stdout).contains("PASS"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn genuine_finally_and_synchronized_compile_execute_return_throw_release() {
        let (ir, metadata, bindings) = cleanup_fixture(false);
        let finally = source(&ir, &metadata, &bindings, "withFinally");
        assert!(finally.contains("finally {"));
        let (ir, metadata, bindings) = cleanup_fixture(true);
        let sync = source(&ir, &metadata, &bindings, "withMonitor");
        assert!(sync.contains("synchronized ("));
        run_java(&format!(
            r#"public class RegionRuntime {{
          static int effects, mode; static Object expectedLock;
          static RuntimeException original = new ArithmeticException("original"), cleanupFailure = new IllegalArgumentException("cleanup");
          static int work(int n,Object lock) {{ if(expectedLock!=null && !Thread.holdsLock(lock)) throw new AssertionError("not held"); if(n!=0) throw original;return 42; }}
          static void cleanup() {{ effects++;if(mode==2) throw cleanupFailure; }}
          {finally}
          {sync}
          public static void main(String[] args) throws Throwable {{
            Object lock=new Object();if(withFinally(0,lock)!=42 || effects!=1) throw new AssertionError("return cleanup");
            try {{withFinally(1,lock);throw new AssertionError();}} catch(RuntimeException e) {{if(e!=original || effects!=2) throw new AssertionError("throw cleanup");}}
            mode=2;try {{withFinally(0,lock);throw new AssertionError();}} catch(RuntimeException e) {{if(e!=cleanupFailure || effects!=3) throw new AssertionError("cleanup override return");}}
            try {{withFinally(1,lock);throw new AssertionError();}} catch(RuntimeException e) {{if(e!=cleanupFailure || effects!=4) throw new AssertionError("cleanup override throw");}}
            expectedLock=lock;if(withMonitor(0,lock)!=42 || Thread.holdsLock(lock)) throw new AssertionError("normal release");
            try {{withMonitor(1,lock);throw new AssertionError();}} catch(RuntimeException e) {{if(e!=original || Thread.holdsLock(lock)) throw new AssertionError("throw release");}}
            Thread t=new Thread(()->{{synchronized(lock) {{}}}});t.start();t.join(2000);if(t.isAlive()) throw new AssertionError("monitor leak");
            try {{withMonitor(0,null);throw new AssertionError();}} catch(NullPointerException expected) {{}}
            System.out.println("PASS finally return/throw/override and synchronized release");
          }}
        }}"#
        ));
    }
    #[test]
    fn catchall_cleanup_mismatch_keeps_literal_handler_and_identity_rethrow() {
        let (ir, metadata, bindings) = cleanup_fixture_inner(false, true);
        let plan = plan_region(
            &ir,
            &metadata,
            &bindings
                .parameters
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>(),
            &Resolver,
        )
        .unwrap();
        assert!(
            matches!(&plan.body[0], RegionStep::Try { .. }),
            "different method identities are not equivalent cleanup"
        );
        let source = source(&ir, &metadata, &bindings, "literalCatchAll");
        assert!(source.contains("catch (java.lang.Throwable"));
        assert!(!source.contains("finally {"));
        assert!(source.contains("throw "));
        run_java(&format!(
            r#"public class RegionRuntime {{
          static RuntimeException original=new ArithmeticException();static int effects;
          static int work(int mode,Object o) {{if(mode!=0) throw original;return 42;}}
          static void cleanup() {{effects++;}}
          {source}
          public static void main(String[] a) throws Throwable {{
            if(literalCatchAll(0,new Object())!=42 || effects!=1) throw new AssertionError();
            try {{literalCatchAll(1,new Object());throw new AssertionError();}} catch(RuntimeException e) {{if(e!=original || effects!=2) throw new AssertionError("handler identity rethrow");}}
            System.out.println("PASS literal catch-all preserves handler rethrow");
          }} }}"#
        ));
    }
    #[test]
    fn branching_protected_prethrow_states_exact_dispatch_and_terminal_handlers() {
        let (ir, metadata, bindings) = branch_fixture(false);
        let mut method = declaration("branch");
        method.parameters.insert(0, JavaType::Boolean);
        let SourceResult::Complete { source, .. } =
            emit_method_region(&ir, &method, &bindings, &Resolver, &metadata)
        else {
            panic!("branch source failed");
        };
        assert!(source.contains("if ("));
        assert!(
            source.find("catch (java.lang.ArithmeticException").unwrap()
                < source.find("catch (java.lang.RuntimeException").unwrap()
        );
        run_java(&format!(
            r#"public class RegionRuntime {{
          static int work(int mode,Object ignored) {{ if(mode==1) throw new ArithmeticException(); if(mode==2) throw new IllegalArgumentException(); return 42; }}
          {source}
          public static void main(String[] a) throws Throwable {{
            Object o=new Object();if(branch(true,0,o)!=42 || branch(false,0,o)!=42 || branch(true,1,o)!=142 || branch(false,1,o)!=110 || branch(true,2,o)!=242 || branch(false,2,o)!=210) throw new AssertionError("branch/prethrow/priority");
            System.out.println("PASS branching scope prethrow snapshots and terminal typed handlers");
          }} }}"#
        ));
        let (ir, metadata, bindings) = branch_fixture(true);
        assert!(matches!(
            plan_region(
                &ir,
                &metadata,
                &bindings
                    .parameters
                    .iter()
                    .flatten()
                    .copied()
                    .collect::<Vec<_>>(),
                &Resolver
            ),
            Err(Error::Unavailable(_))
        ));
    }
    #[test]
    fn unrelated_catches_require_three_way_hierarchy_and_shadowing_rejects() {
        struct Siblings<'a>(&'a Metadata);
        impl RegionMetadata for Siblings<'_> {
            fn region_scope(&self) -> &scopes::RegionScope {
                &self.0.scope
            }
            fn catch_type(&self, key: &str) -> Option<JavaType> {
                if key == "broad" {
                    Some(JavaType::Class("java.lang.IllegalArgumentException".into()))
                } else {
                    self.0.catch_type(key)
                }
            }
            fn hierarchy(&self) -> &Hierarchy {
                &self.0.hierarchy
            }
            fn is_catch_all(&self, _: &str) -> bool {
                false
            }
        }
        let (ir, mut metadata, bindings) = branch_fixture(false);
        let bound = bindings
            .parameters
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let plan = plan_region(&ir, &Siblings(&metadata), &bound, &Resolver).unwrap();
        assert!(matches!(
            &plan.body.last().unwrap(),
            RegionStep::Instruction(_)
        ));
        metadata.scope.catches.reverse();
        assert!(matches!(
            plan_region(&ir, &metadata, &bound, &Resolver),
            Err(Error::Unsupported(
                "shadowed catch or incomplete JavaHierarchy"
            ))
        ));
        metadata.scope.catches.reverse();
        metadata.hierarchy = Hierarchy::default();
        assert!(matches!(
            plan_region(&ir, &Siblings(&metadata), &bound, &Resolver),
            Err(Error::Unsupported("catch type is not proven Throwable"))
        ));
    }
    include!("exception_disjoint_tests.rs");
}
