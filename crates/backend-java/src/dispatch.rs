//! Exact Java17 dispatcher for a validated ordinary normal CFG.
//! Parents prove SSA read availability/types and hoist concrete local declarations.
//! Default local initializers, if needed for Java definite assignment, are permitted
//! only after that proof; this renderer invents neither values nor source methods.
use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::{BlockId, EdgeId},
};
use decomp_structure::dispatch::{DispatchPlan, plan};
use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
};

/// Typed parent lowering hooks. Errors abort the complete rendering transaction.
pub trait DispatchEmitter {
    /// Ordered original non-control operations; reject monitors and unknown reads.
    fn instructions(&mut self, block: BlockId) -> Result<String, String>;
    /// Original typed return/throw; must terminate and not fall through.
    fn terminal(&mut self, block: BlockId) -> Result<String, String>;
    /// Original Boolean condition, evaluated once at this state.
    fn condition(&mut self, block: BlockId) -> Result<String, String>;
    /// Original int switch selector, evaluated once at this state.
    fn selector(&mut self, block: BlockId) -> Result<String, String>;
    /// Original edge's parallel copies, with all sources snapshotted first.
    fn copies(&mut self, edge: EdgeId) -> Result<String, String>;
}
/// Render a canonical snapshot without duplicating real instruction blocks.
/// `prefix` must be a parent-reserved identifier, distinct from all source locals.
/// All callbacks preserve original operations and may fail closed. No exceptional
/// transfers are filtered out; the language-neutral planner rejects them.
pub fn render<T>(
    cfg: &Cfg<T>,
    snapshot: &DispatchPlan,
    prefix: &str,
    emitter: &mut impl DispatchEmitter,
) -> Result<String, String> {
    if plan(cfg).map_err(|e| e.to_string())? != *snapshot {
        return Err("noncanonical dispatch snapshot".into());
    }
    if prefix.is_empty()
        || !prefix
            .bytes()
            .enumerate()
            .all(|(i, c)| c.is_ascii_alphabetic() || c == b'_' || (i > 0 && c.is_ascii_digit()))
    {
        return Err("invalid dispatch identifier prefix".into());
    }
    let states: HashMap<_, _> = snapshot
        .states
        .iter()
        .enumerate()
        .map(|(i, s)| (s.block, i))
        .collect();
    let mut out =
        format!("int {prefix}_pc = 0;\n{prefix}_loop: while (true) {{\nswitch ({prefix}_pc) {{\n");
    for (i, state) in snapshot.states.iter().enumerate() {
        writeln!(out, "case {i}: {{").map_err(|e| e.to_string())?;
        out.push_str(&emitter.instructions(state.block)?);
        out.push('\n');
        let edges = state
            .outgoing
            .iter()
            .map(|&id| cfg.edge(id))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let mut transfer = |id: EdgeId| -> Result<String, String> {
            let edge = cfg.edge(id).map_err(|e| e.to_string())?;
            Ok(format!(
                "{}\n{prefix}_pc = {};\ncontinue {prefix}_loop;\n",
                emitter.copies(id)?,
                states[&edge.target()]
            ))
        };
        match edges.as_slice() {
            [] => {
                out.push_str(&emitter.terminal(state.block)?);
                out.push('\n');
            }
            [_] => out.push_str(&transfer(state.outgoing[0])?),
            [a, b] if matches!(a.kind(), EdgeKind::BranchTrue | EdgeKind::BranchFalse) => {
                let (yes, no) = if a.kind() == &EdgeKind::BranchTrue {
                    (state.outgoing[0], state.outgoing[1])
                } else {
                    (state.outgoing[1], state.outgoing[0])
                };
                let yes = transfer(yes)?;
                let no = transfer(no)?;
                let condition = emitter.condition(state.block)?;
                writeln!(out, "if ({condition}) {{\n{yes}}} else {{\n{no}}}")
                    .map_err(|e| e.to_string())?;
                let _ = b;
            }
            _ => {
                // Java selectors are int. Conversion preserves signed DEX labels;
                // wider noncanonical u64 labels must not silently truncate.
                let mut arms = Vec::new();
                let mut labels = HashSet::new();
                for (&id, edge) in state.outgoing.iter().zip(&edges) {
                    let label = match edge.kind() {
                        EdgeKind::Unconditional => "default".to_owned(),
                        EdgeKind::Switch(v) => {
                            let n = *v as i32;
                            if *v != u64::from(n as u32) && *v != n as i64 as u64 {
                                return Err("switch label outside int domain".into());
                            }
                            format!("case {n}")
                        }
                        _ => return Err("invalid dispatch edge".into()),
                    };
                    if !labels.insert(label.clone()) {
                        return Err("duplicate Java switch label".into());
                    }
                    arms.push((label, transfer(id)?));
                }
                writeln!(out, "switch ({}) {{", emitter.selector(state.block)?)
                    .map_err(|e| e.to_string())?;
                for (label, body) in arms {
                    writeln!(out, "{label}: {{\n{body}}}").map_err(|e| e.to_string())?;
                }
                out.push_str("}\n");
            }
        }
        out.push_str("}\n");
    }
    // Invariant guard is unreachable for a valid original path, not a fallback
    // implementation of any original body or a guessed successor.
    out.push_str("default: throw new IllegalStateException(\"invalid CFG state\");\n}\n}\n");
    Ok(out)
}
