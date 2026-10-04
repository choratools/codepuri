//! Parent integration: re-export `decomp_structure::loop_control` as
//! `crate::loop_control_regions`, then export this module. The parent supplies
//! its validated instruction/edge lowering through `Lowering` (no duplicated
//! instruction semantics or Phi collection). Render Loop as a labeled
//! `while (true)`, Transfer as labeled break/continue. Render every existing
//! Step with the existing emitter. The condition is an already assigned local.
//!
//! Exact parent wiring:
//! 1. Export `pub mod loop_control` in both libraries and re-export the
//!    structure module as `loop_control_regions` in backend-java.
//! 2. Canonicalize via `structure(ir.cfg())`, compare the entire supplied
//!    `ControlCfg` (including provenance), and call branching `prepare`.
//! 3. Implement Lowering beside Planner, where its private helpers are visible.
//!    Reuse `insn`, `require`, `reads`, `defined`, and `edge`. Split ordinary
//!    block lowering from outgoing-edge lowering: this module owns the latter.
//!    Skip Phi and unconditional Branch instructions; emit Return once and
//!    return false. Conditional Branch is owned by ControlRegion::If.
//! 4. Allocate arena-order locals using the same definition filtering as
//!    branching `finish`, hoisting declarations only. Return a separate plan
//!    with `Vec<ControlStep>`; BranchPlan's `Vec<Step>` cannot hold transfers.
//! 5. Assign a fresh stable Java label per loop header. Emit all selected edge
//!    copies before its labeled transfer, using `schedule_copies`; do not emit
//!    copies again when rendering Transfer. Posttest loops deliberately use
//!    the same general-loop rendering, with the test retained at the tail.
//!
//! Selected real OKcashbag structure evidence (DEX9; offsets in code units):
//! * Operator.isAllowedOperatorChar, method 61256, code item 8060948:
//!   pretest at 5, conditional early return at 9/12, backedge at 15,
//!   exhaustion return at 16; all 17 edges retained.
//! * ANTabBar$Companion.generateViewIdSdk17Under, method 60894,
//!   code item 8037944: branches at 3, 18, 29, retry at 34,
//!   returns at 31 and 38; all 27 edges retained.
//!
//! These were selected from the preserved run-1790850551448556691-2337418
//! report and unavailable-method comments in Operator.java:15 and
//! ANTabBar$Companion.java:20. This is structuring evidence, not a claim that
//! those complete Android source units compile with this standalone module.
//! Function.isValidFunctionName (61184) still rejects a shared-return overlap.
use crate::branching::Step;
use crate::loop_control_regions::{ControlRegion, TransferKind};
use decomp_core::ids::{BlockId, EdgeId};
use managed_ir::value::ValueId;
use std::collections::HashSet;

/// Values definitely available on the current execution path.
pub type Environment = HashSet<ValueId>;
#[derive(Clone, Debug, PartialEq, Eq)]
/// Executable general-loop plan, preserving original instruction positions.
pub enum ControlStep {
    /// Validated ordinary instruction or parallel copies.
    Ordinary(Step),
    /// Selected conditional arm.
    If {
        /// Materialized Boolean local.
        condition: ValueId,
        /// True arm.
        then_steps: Vec<Self>,
        /// False arm.
        else_steps: Vec<Self>,
    },
    /// General natural loop.
    Loop {
        /// Stable original header identity.
        header: BlockId,
        /// Original iteration operations.
        body: Vec<Self>,
    },
    /// Labeled transfer after its original edge copies.
    Transfer {
        /// Original traversed edge.
        edge: EdgeId,
        /// Proven target loop boundary.
        kind: TransferKind,
    },
}
/// Hooks operate only on validated, canonically structured input. The parent
/// must compare the supplied tree with `loop_control_regions::structure` and
/// run branching preparation/definition/branch/Phi validation before calling.
pub trait Lowering {
    /// Transactional parent validation error.
    type Error;
    /// Emit the block's instruction, without ANY outgoing-edge copies.
    /// Phi requires its result available; return checks reads and terminates.
    fn block(
        &mut self,
        block: BlockId,
        env: &mut Environment,
    ) -> Result<(Vec<Step>, bool), Self::Error>;
    /// Check Boolean type and definite availability; emit no instruction.
    fn condition(&mut self, block: BlockId, env: &Environment) -> Result<ValueId, Self::Error>;
    /// Validate all Phi sources against the pre-copy environment, emit one
    /// simultaneous Copies batch, then add its destinations to env.
    fn edge(&mut self, edge: EdgeId, env: &mut Environment) -> Result<Vec<Step>, Self::Error>;
    /// Original outgoing edge of an ordinary, nonterminal block.
    fn outgoing(&mut self, block: BlockId) -> Result<Option<EdgeId>, Self::Error>;
}
/// Lower in original execution order. The boolean says whether control falls
/// through. Break environments, not body fallthrough, determine loop outputs.
pub fn lower<L: Lowering>(
    region: &ControlRegion,
    hooks: &mut L,
    env: &mut Environment,
) -> Result<(Vec<ControlStep>, bool), L::Error> {
    walk(region, hooks, env, &mut Vec::new())
}
struct Frame {
    header: BlockId,
    breaks: Vec<Environment>,
}
fn wrap(steps: Vec<Step>) -> Vec<ControlStep> {
    steps.into_iter().map(ControlStep::Ordinary).collect()
}
fn merge(env: &mut Environment, arms: Vec<Environment>) -> bool {
    let mut arms = arms.into_iter();
    let Some(mut result) = arms.next() else {
        return false;
    };
    for a in arms {
        result.retain(|v| a.contains(v));
    }
    *env = result;
    true
}
#[allow(clippy::expect_used)] // Canonical structure guarantees balanced frames and transfer owners.
fn walk<L: Lowering>(
    r: &ControlRegion,
    h: &mut L,
    env: &mut Environment,
    stack: &mut Vec<Frame>,
) -> Result<(Vec<ControlStep>, bool), L::Error> {
    match r {
        ControlRegion::Block(b) => {
            let (steps, next) = h.block(*b, env)?;
            let mut steps = wrap(steps);
            if next && let Some(e) = h.outgoing(*b)? {
                steps.extend(wrap(h.edge(e, env)?));
            }
            Ok((steps, next))
        }
        ControlRegion::Sequence(rs) => {
            let mut steps = Vec::new();
            for r in rs {
                let (s, next) = walk(r, h, env, stack)?;
                steps.extend(s);
                if !next {
                    return Ok((steps, false));
                }
            }
            Ok((steps, true))
        }
        ControlRegion::If {
            header,
            then_edge,
            then_region,
            else_edge,
            else_region,
        } => {
            let condition = h.condition(*header, env)?;
            let before = env.clone();
            let mut continuing = Vec::new();
            let mut arms = Vec::new();
            for (edge, r) in [(*then_edge, then_region), (*else_edge, else_region)] {
                let mut arm = before.clone();
                let mut steps = wrap(h.edge(edge, &mut arm)?);
                let (child, next) = walk(r, h, &mut arm, stack)?;
                steps.extend(child);
                if next {
                    continuing.push(arm);
                }
                arms.push(steps);
            }
            let next = merge(env, continuing);
            let else_steps = arms.pop().unwrap_or_default();
            let then_steps = arms.pop().unwrap_or_default();
            Ok((
                vec![ControlStep::If {
                    condition,
                    then_steps,
                    else_steps,
                }],
                next,
            ))
        }
        ControlRegion::Loop { header, body, .. } => {
            stack.push(Frame {
                header: *header,
                breaks: Vec::new(),
            });
            let mut body_env = env.clone();
            let (body, _) = walk(body, h, &mut body_env, stack)?;
            let frame = stack.pop().expect("balanced loop frames");
            let next = merge(env, frame.breaks);
            Ok((
                vec![ControlStep::Loop {
                    header: *header,
                    body,
                }],
                next,
            ))
        }
        ControlRegion::Transfer { edge, kind } => {
            // Copies were already emitted by the source Block or selected If
            // arm. Re-emitting here would corrupt cyclic Phi transfers.
            if let TransferKind::Break { header } = kind {
                let frame = stack
                    .iter_mut()
                    .rev()
                    .find(|f| f.header == *header)
                    .expect("canonical transfer owner");
                frame.breaks.push(env.clone());
            }
            Ok((
                vec![ControlStep::Transfer {
                    edge: *edge,
                    kind: *kind,
                }],
                false,
            ))
        }
    }
}
