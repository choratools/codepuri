//! Parent integration for single-placement acyclic shared tails.
//!
//! Register this module in the parent backend. Call `structure_flow` after the
//! ordinary branch planner reports overlap. `Strict` still uses the ordinary
//! planner. For `Shared`, supply validated instruction/terminator rendering via
//! `ForwardEmitter`. Hoist SSA locals, never their initializers. The emitter must
//! perform edge-specific parallel Phi copies using `branching::schedule_copies`.
//! The control skeleton uses only forward labelled breaks, never a state machine.

use decomp_core::{
    cfg::EdgeKind,
    ids::{BlockId, EdgeId},
};
use decomp_structure::extended::ForwardRegions;
use std::collections::HashMap;
use std::fmt::Write;

/// Family-specific, validated operations needed by forward control emission.
pub trait ForwardEmitter {
    /// Render instructions before the terminator, in original order. SSA
    /// definitions assign hoisted locals. Do not include branch/goto/return.
    fn instructions(&mut self, block: BlockId) -> Result<String, String>;
    /// Render the original terminal (return or throw). It must not fall through.
    fn terminal(&mut self, block: BlockId) -> Result<String, String>;
    /// Read an already-materialized Boolean local; do not inline its definition.
    fn condition(&mut self, block: BlockId) -> Result<String, String>;
    /// Read an already-materialized integral switch selector.
    fn selector(&mut self, block: BlockId) -> Result<String, String>;
    /// Execute this edge's simultaneous Phi copies before transferring. All
    /// sources must be snapshotted before any destination is assigned.
    fn copies(&mut self, edge: EdgeId) -> Result<String, String>;
}

/// Emit an acyclic control skeleton with one placement per real block.
///
/// The snapshot is compared to canonical `structure_flow(cfg)` before invoking
/// any callbacks. Parent must validate SSA availability/types and terminal shapes,
/// and use a method-unique `prefix` Java identifier to avoid label collisions.
/// Exceptional instruction effects propagate normally; exception CFG edges
/// are not accepted here. No catch or finally is invented.
///
/// Labels enclose all earlier blocks. Breaking the target's label skips directly
/// to that block; all intervening instructions are bypassed. Exactly one edge
/// batch executes on each chosen transfer. No path reaches a block by implicit
/// fallthrough. Output size and bookkeeping are O(B+E+instruction output).
pub fn render<T>(
    cfg: &decomp_core::cfg::Cfg<T>,
    regions: &ForwardRegions,
    prefix: &str,
    emitter: &mut impl ForwardEmitter,
) -> Result<String, String> {
    match decomp_structure::extended::structure_flow(cfg).map_err(|e| e.to_string())? {
        decomp_structure::extended::FlowStructuredCfg::Shared(canonical)
            if canonical == *regions => {}
        _ => return Err("noncanonical forward region snapshot".into()),
    }
    if prefix.is_empty()
        || !prefix
            .bytes()
            .enumerate()
            .all(|(i, c)| c.is_ascii_alphabetic() || c == b'_' || (i > 0 && c.is_ascii_digit()))
    {
        return Err("invalid forward label prefix".into());
    }
    let index: HashMap<_, _> = regions
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.block, i))
        .collect();
    if index.len() != regions.blocks.len()
        || regions.blocks.first().map(|b| b.block) != Some(regions.entry)
    {
        return Err("invalid single-placement coverage".into());
    }
    let edges: HashMap<_, _> = regions.edges.iter().map(|e| (e.id, e)).collect();
    if edges.len() != regions.edges.len() {
        return Err("duplicate edge identity".into());
    }
    let mut out = String::new();
    for i in (1..regions.blocks.len()).rev() {
        writeln!(out, "{prefix}_{i}: {{").map_err(|e| e.to_string())?;
    }
    for (i, block) in regions.blocks.iter().enumerate() {
        if i > 0 {
            out.push_str("}\n");
        }
        out.push_str(&emitter.instructions(block.block)?);
        out.push('\n');
        let outgoing = block
            .outgoing
            .iter()
            .map(|id| {
                let edge = edges.get(id).ok_or("missing original edge")?;
                if edge.source != block.block
                    || index.get(&edge.target).is_none_or(|&t| t <= i)
                    || matches!(edge.kind, EdgeKind::Exception(_))
                {
                    return Err("not an acyclic normal transfer");
                }
                Ok(*edge)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut transfer = |edge: &decomp_structure::EdgeProvenance| -> Result<String, String> {
            Ok(format!(
                "{}\nbreak {prefix}_{};\n",
                emitter.copies(edge.id)?,
                index[&edge.target]
            ))
        };
        match outgoing.as_slice() {
            [] => {
                out.push_str(&emitter.terminal(block.block)?);
                out.push('\n');
            }
            [edge] if edge.kind == EdgeKind::Unconditional => out.push_str(&transfer(edge)?),
            [a, b]
                if matches!(
                    (&a.kind, &b.kind),
                    (EdgeKind::BranchTrue, EdgeKind::BranchFalse)
                        | (EdgeKind::BranchFalse, EdgeKind::BranchTrue)
                ) =>
            {
                let (yes, no) = if a.kind == EdgeKind::BranchTrue {
                    (*a, *b)
                } else {
                    (*b, *a)
                };
                let yes = transfer(yes)?;
                let no = transfer(no)?;
                writeln!(
                    out,
                    "if ({}) {{\n{yes}}} else {{\n{no}}}",
                    emitter.condition(block.block)?
                )
                .map_err(|e| e.to_string())?;
            }
            arms if arms.iter().any(|e| matches!(e.kind, EdgeKind::Switch(_))) => {
                // Missing-default dispatch needs a family-level unmatched-value
                // contract. Do not turn it into a made-up default destination.
                if arms
                    .iter()
                    .filter(|e| e.kind == EdgeKind::Unconditional)
                    .count()
                    != 1
                    || arms
                        .iter()
                        .any(|e| !matches!(e.kind, EdgeKind::Switch(_) | EdgeKind::Unconditional))
                {
                    return Err("switch requires an explicit original default".into());
                }
                let mut rendered = Vec::with_capacity(arms.len());
                for edge in arms {
                    rendered.push(transfer(edge)?);
                }
                writeln!(out, "switch ({}) {{", emitter.selector(block.block)?)
                    .map_err(|e| e.to_string())?;
                for (edge, body) in arms.iter().zip(rendered) {
                    match edge.kind {
                        EdgeKind::Switch(label) => {
                            let signed = label as i64;
                            let label = i32::try_from(signed)
                                .map_err(|_| "switch label outside Java int range")?;
                            writeln!(out, "case {label}: {{")
                        }
                        _ => writeln!(out, "default: {{"),
                    }
                    .map_err(|e| e.to_string())?;
                    out.push_str(&body);
                    out.push_str("}\n");
                }
                out.push_str("}\n");
            }
            _ => return Err("unsupported forward branch".into()),
        }
    }
    Ok(out)
}
