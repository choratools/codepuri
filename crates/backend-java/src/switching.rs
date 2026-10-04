//! Primitive Java switch lowering, independent of the parent instruction emitter.
//!
//! Include with `pub mod switching;`. Lower a canonical extended switch with
//! `plan`, hoist its Phi destinations with the parent's other SSA locals, then
//! call `emit`. The region callback emits ordinary instructions, nested regions,
//! and outgoing-edge copies in their original order. Dispatch copies are owned
//! here, not by that callback. All value names must denote materialized locals.
//! The temporary callback must allocate a fresh, collision-free Java identifier.
//!
//! Parent integration points:
//! - At the structured-method path, use `structure_extended` for acyclic switch
//!   methods and recurse over `ExtendedRegion` instead of calling the branch-only
//!   planner, which intentionally rejects switches.
//! - At `ExtendedRegion::Switch`, call `plan` with the PRE-dispatch availability
//!   set. Materialize the selector's defining instruction in its original block;
//!   never inline that instruction into either generated switch.
//! - Allocate declarations for all CFG definitions at method scope. Render Phi
//!   blocks as availability checks, not Java assignments. This module assigns
//!   entry Phis on dispatch edges; the parent's ordinary edge lowering assigns
//!   join Phis on non-dispatch edges, snapshotting all sources first.
//! - For each body callback, start with the pre-dispatch environment plus the
//!   destinations assigned on every dispatch edge selecting that target. Lower
//!   its region recursively; merge only continuing arm environments by
//!   intersection before lowering the enclosing sequence's join.
//! - Pass the parent's stable-local lookup, Java type lookup, and fresh-name
//!   allocator to `emit`. Return `ArmSource::continues = true` for an empty body
//!   or a body reaching the excluded join, and false for a terminal body.
//! - Translate `SwitchError` into the parent's unsupported-source diagnostic;
//!   propagate callback errors, never publish a partial source fragment.

use decomp_core::{
    cfg::EdgeKind,
    ids::{BlockId, EdgeId},
};
use decomp_structure::{
    StructureError,
    extended::{ExtendedRegion, SwitchArm},
};
use managed_ir::{
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
    types::ManagedType,
    value::ValueId,
};
use std::collections::HashSet;

/// Simultaneous assignment on one original dispatch edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Copy {
    /// Stable Phi result local.
    pub destination: ValueId,
    /// Stable incoming local, read before assignments in this batch.
    pub source: ValueId,
}
/// One original dispatch edge; labels use signed Java int semantics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dispatch {
    /// Original CFG identity, including parallel edges to the same target.
    pub edge: EdgeId,
    /// None denotes the original default edge.
    pub label: Option<i32>,
    /// Cluster-ordered parallel assignments.
    pub copies: Vec<Copy>,
}
/// Grouped bodies plus edge-specific entry assignments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepSwitch {
    /// Materialized primitive int selector.
    pub key: ValueId,
    /// Original edge-specific entry actions.
    pub dispatch: Vec<Dispatch>,
    /// Canonical distinct-target bodies; the join is excluded.
    pub arms: Vec<SwitchArm>,
}
/// Invalid input never produces a partial source fragment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwitchError {
    /// Original typed structuring failure.
    Structure(StructureError),
    /// Supplied region is not a canonical switch in this method.
    RegionMismatch,
    /// Stale or inconsistent arena identity.
    InvalidIr(String),
    /// Instruction cases/default disagree with CFG edges or Java int range.
    InvalidEdges(BlockId),
    /// Phi cluster does not exactly cover its head's incoming edges.
    InvalidPhi(BlockId),
    /// Selector or incoming Phi type is incompatible.
    TypeMismatch(ValueId),
    /// Selector or copy source is not defined before dispatch.
    Unavailable(ValueId),
}
impl std::fmt::Display for SwitchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Java switch: {self:?}")
    }
}
impl std::error::Error for SwitchError {}
fn invalid(e: impl std::fmt::Display) -> SwitchError {
    SwitchError::InvalidIr(e.to_string())
}

fn contains(root: &ExtendedRegion, wanted: &ExtendedRegion) -> bool {
    if root == wanted {
        return true;
    }
    match root {
        ExtendedRegion::Sequence(rs) => rs.iter().any(|r| contains(r, wanted)),
        ExtendedRegion::If {
            then_region,
            else_region,
            ..
        } => contains(then_region, wanted) || contains(else_region, wanted),
        ExtendedRegion::Switch { arms, .. } => arms.iter().any(|a| contains(&a.region, wanted)),
        ExtendedRegion::Block(_) => false,
    }
}

/// Validate a canonical switch and collect dispatch-edge Phi clusters. The
/// availability predicate is queried before any destination is assigned.
/// Other incoming edges of a shared join remain the parent lowerer's obligation.
pub fn plan(
    ir: &MethodIr<SsaForm>,
    region: &ExtendedRegion,
    available: impl Fn(ValueId) -> bool,
) -> Result<StepSwitch, SwitchError> {
    let canonical =
        decomp_structure::extended::structure_extended(ir.cfg()).map_err(SwitchError::Structure)?;
    if !contains(&canonical.root, region) {
        return Err(SwitchError::RegionMismatch);
    }
    let ExtendedRegion::Switch { header, arms, .. } = region else {
        return Err(SwitchError::RegionMismatch);
    };
    let cfg = ir.cfg();
    let insn = ir
        .instruction(*cfg.block(*header).map_err(invalid)?.payload())
        .map_err(invalid)?;
    let ManagedInsn::Switch {
        key,
        default_edge,
        cases,
    } = insn
    else {
        return Err(SwitchError::InvalidEdges(*header));
    };
    if ir.value(*key).map_err(invalid)?.ty() != &ManagedType::Int {
        return Err(SwitchError::TypeMismatch(*key));
    }
    if !available(*key) {
        return Err(SwitchError::Unavailable(*key));
    }
    let mut seen = HashSet::new();
    let mut labels = HashSet::new();
    let mut dispatch = Vec::new();
    for (label, id) in cases
        .iter()
        .map(|(v, e)| (Some(*v), *e))
        .chain(std::iter::once((None, *default_edge)))
    {
        let edge = cfg.edge(id).map_err(invalid)?;
        let java_label = label
            .map(i32::try_from)
            .transpose()
            .map_err(|_| SwitchError::InvalidEdges(*header))?;
        let kind_ok = match label {
            Some(v) => edge.kind() == &EdgeKind::Switch(v as u64) && labels.insert(v),
            None => edge.kind() == &EdgeKind::Unconditional,
        };
        if edge.source() != *header || !kind_ok || !seen.insert(id) {
            return Err(SwitchError::InvalidEdges(*header));
        }
        let head = edge.target();
        let incoming = cfg.block(head).map_err(invalid)?.incoming();
        let expected: HashSet<_> = incoming.iter().copied().collect();
        let mut member = head;
        let mut copies = Vec::new();
        loop {
            let instruction_id = *cfg.block(member).map_err(invalid)?.payload();
            let ManagedInsn::Phi { result, inputs } =
                ir.instruction(instruction_id).map_err(invalid)?
            else {
                break;
            };
            if ir.value(*result).map_err(invalid)?.def() != instruction_id {
                return Err(SwitchError::InvalidPhi(member));
            }
            let mut input_edges = HashSet::new();
            if inputs.len() != expected.len() {
                return Err(SwitchError::InvalidPhi(member));
            }
            for &(e, source) in inputs {
                if !expected.contains(&e) || !input_edges.insert(e) {
                    return Err(SwitchError::InvalidPhi(member));
                }
                if ir.value(source).map_err(invalid)?.ty()
                    != ir.value(*result).map_err(invalid)?.ty()
                {
                    return Err(SwitchError::TypeMismatch(*result));
                }
                if e == id {
                    if !available(source) {
                        return Err(SwitchError::Unavailable(source));
                    }
                    copies.push(Copy {
                        destination: *result,
                        source,
                    });
                }
            }
            let outgoing = cfg.block(member).map_err(invalid)?.outgoing();
            if outgoing.len() != 1 {
                return Err(SwitchError::InvalidPhi(member));
            }
            let next = cfg.edge(outgoing[0]).map_err(invalid)?;
            if next.kind() != &EdgeKind::Unconditional {
                return Err(SwitchError::InvalidPhi(member));
            }
            if cfg.block(next.target()).map_err(invalid)?.incoming().len() != 1 {
                break;
            }
            member = next.target();
        }
        dispatch.push(Dispatch {
            edge: id,
            label: java_label,
            copies,
        });
    }
    if seen.len() != cfg.block(*header).map_err(invalid)?.outgoing().len() {
        return Err(SwitchError::InvalidEdges(*header));
    }
    Ok(StepSwitch {
        key: *key,
        dispatch,
        arms: arms.clone(),
    })
}

/// Parent callback result: `continues` means the body reaches the switch join.
/// A continuing arm gets a break; a returning/throwing arm must not get one.
pub struct ArmSource {
    /// Complete recursively emitted body, without case labels or outer braces.
    pub source: String,
    /// Whether normal execution reaches the enclosing switch join.
    pub continues: bool,
}

/// Render a classic int switch. A separate entry switch preserves different
/// Phi inputs on labels sharing a body, without duplicating side effects. Both
/// switches read the same stable key local. Bodies are braced; no fallthrough
/// is invented. Overlapping/fallthrough CFGs retain typed structurer errors.
pub fn emit<E>(
    plan: &StepSwitch,
    mut name: impl FnMut(ValueId) -> Result<String, E>,
    mut ty: impl FnMut(ValueId) -> Result<String, E>,
    mut temporary: impl FnMut() -> String,
    mut body: impl FnMut(&ExtendedRegion) -> Result<ArmSource, E>,
) -> Result<String, E> {
    let key = name(plan.key)?;
    let mut out = String::new();
    if plan.dispatch.iter().any(|d| !d.copies.is_empty()) {
        out.push_str(&format!("switch ({key}) {{\n"));
        for entry in &plan.dispatch {
            match entry.label {
                Some(v) => out.push_str(&format!("case {v}: {{\n")),
                None => out.push_str("default: {\n"),
            }
            let mut saved = Vec::new();
            for copy in &entry.copies {
                let temp = temporary();
                out.push_str(&format!(
                    "{} {temp} = {};\n",
                    ty(copy.source)?,
                    name(copy.source)?
                ));
                saved.push(temp);
            }
            for (copy, temp) in entry.copies.iter().zip(saved) {
                out.push_str(&format!("{} = {temp};\n", name(copy.destination)?));
            }
            out.push_str("break;\n}\n");
        }
        out.push_str("}\n");
    }
    out.push_str(&format!("switch ({key}) {{\n"));
    for arm in &plan.arms {
        for label in &arm.labels {
            out.push_str(&format!("case {}:\n", *label as i64));
        }
        if arm.is_default {
            out.push_str("default:\n");
        }
        out.push_str("{\n");
        let rendered = body(&arm.region)?;
        out.push_str(&rendered.source);
        out.push('\n');
        if rendered.continues {
            out.push_str("break;\n");
        }
        out.push_str("}\n");
    }
    out.push_str("}\n");
    Ok(out)
}
