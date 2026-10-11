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

enum Exit {
    Terminal(String),
    Jump(EdgeId, String),
    Branch(String, EdgeId, String, EdgeId, String),
    Switch(String, Vec<(String, EdgeId, String)>),
}
struct PreparedState {
    instructions: String,
    exit: Exit,
}

#[derive(Clone)]
enum ArmEnd {
    Join(BlockId),
    Terminal,
}

#[derive(Clone)]
struct LinearArm {
    group: Option<usize>,
    end: ArmEnd,
}

enum StructuredExit {
    If {
        yes: LinearArm,
        no: LinearArm,
        join: Option<BlockId>,
    },
    Loop {
        body: LoopBody,
        body_edge: EdgeId,
        exit_edge: EdgeId,
    },
}

#[derive(Clone, Copy)]
enum LoopBody {
    Empty,
    Linear(usize),
}

fn append(out: &mut String, text: &str) {
    if !text.is_empty() {
        out.push_str(text);
        if !text.ends_with('\n') {
            out.push('\n');
        }
    }
}

fn append_scoped(out: &mut String, text: &str) {
    if !text.is_empty() {
        out.push_str("{\n");
        append(out, text);
        out.push_str("}\n");
    }
}

struct ProofView<'a, T> {
    cfg: &'a Cfg<T>,
    snapshot: &'a DispatchPlan,
    prepared: &'a [PreparedState],
    groups: &'a [Vec<usize>],
    group_states: &'a HashMap<BlockId, usize>,
}

impl<T> ProofView<'_, T> {
    fn linear_group_has_entry(
        &self,
        group: usize,
        first_block: BlockId,
        first_edge: EdgeId,
    ) -> Result<bool, String> {
        let members = &self.groups[group];
        if members
            .first()
            .is_none_or(|&index| self.snapshot.states[index].block != first_block)
        {
            return Ok(false);
        }
        let mut incoming = first_edge;
        for (position, &index) in members.iter().enumerate() {
            let block = self.snapshot.states[index].block;
            if self.cfg.block(block).map_err(|e| e.to_string())?.incoming() != [incoming] {
                return Ok(false);
            }
            if position + 1 < members.len() {
                let Exit::Jump(edge, _) = &self.prepared[index].exit else {
                    return Ok(false);
                };
                let edge_data = self.cfg.edge(*edge).map_err(|e| e.to_string())?;
                if *edge_data.kind() != EdgeKind::Unconditional
                    || edge_data.target() != self.snapshot.states[members[position + 1]].block
                {
                    return Ok(false);
                }
                incoming = *edge;
            }
        }
        Ok(true)
    }

    fn linear_arm(
        &self,
        source_group: usize,
        edge_id: EdgeId,
    ) -> Result<Option<LinearArm>, String> {
        let target = self.cfg.edge(edge_id).map_err(|e| e.to_string())?.target();
        let incoming = self
            .cfg
            .block(target)
            .map_err(|e| e.to_string())?
            .incoming();

        // A shared target or the dispatch entry stays a real dispatcher boundary.
        if target == self.snapshot.entry || incoming != [edge_id] {
            return Ok(Some(LinearArm {
                group: None,
                end: ArmEnd::Join(target),
            }));
        }
        let Some(&group) = self.group_states.get(&target) else {
            return Ok(None);
        };
        // A backedge into the source group is a cycle, not an arm to inline.
        if group == source_group || !self.linear_group_has_entry(group, target, edge_id)? {
            return Ok(None);
        }

        let last = *self.groups[group].last().ok_or("empty dispatch group")?;
        let end = match &self.prepared[last].exit {
            Exit::Terminal(_) => ArmEnd::Terminal,
            Exit::Jump(edge, _) => {
                let next = self.cfg.edge(*edge).map_err(|e| e.to_string())?.target();
                let next_incoming = self.cfg.block(next).map_err(|e| e.to_string())?.incoming();
                if next != self.snapshot.entry && next_incoming == [*edge] {
                    // Maximal chains should already have absorbed this successor.
                    return Ok(None);
                }
                ArmEnd::Join(next)
            }
            Exit::Branch(_, _, _, _, _) | Exit::Switch(_, _) => return Ok(None),
        };
        Ok(Some(LinearArm {
            group: Some(group),
            end,
        }))
    }

    fn branch_region(
        &self,
        source_group: usize,
        yes: EdgeId,
        no: EdgeId,
    ) -> Result<Option<StructuredExit>, String> {
        let Some(yes_arm) = self.linear_arm(source_group, yes)? else {
            return Ok(None);
        };
        let Some(no_arm) = self.linear_arm(source_group, no)? else {
            return Ok(None);
        };
        let join = match (&yes_arm.end, &no_arm.end) {
            (ArmEnd::Join(a), ArmEnd::Join(b)) if a == b => Some(*a),
            (ArmEnd::Terminal, ArmEnd::Terminal) => None,
            (ArmEnd::Join(join), ArmEnd::Terminal) | (ArmEnd::Terminal, ArmEnd::Join(join)) => {
                Some(*join)
            }
            _ => return Ok(None),
        };
        Ok(Some(StructuredExit::If {
            yes: yes_arm,
            no: no_arm,
            join,
        }))
    }

    fn loop_region(
        &self,
        source_group: usize,
        header: BlockId,
        yes: EdgeId,
        no: EdgeId,
    ) -> Result<Option<StructuredExit>, String> {
        let backedge_body = |edge_id: EdgeId| -> Result<Option<LoopBody>, String> {
            let target = self.cfg.edge(edge_id).map_err(|e| e.to_string())?.target();
            if target == header {
                return Ok(Some(LoopBody::Empty));
            }
            if target == self.snapshot.entry
                || self
                    .cfg
                    .block(target)
                    .map_err(|e| e.to_string())?
                    .incoming()
                    != [edge_id]
            {
                return Ok(None);
            }
            let Some(&group) = self.group_states.get(&target) else {
                return Ok(None);
            };
            if group == source_group || !self.linear_group_has_entry(group, target, edge_id)? {
                return Ok(None);
            }
            let last = *self.groups[group].last().ok_or("empty dispatch group")?;
            let Exit::Jump(backedge, _) = &self.prepared[last].exit else {
                return Ok(None);
            };
            let backedge_data = self.cfg.edge(*backedge).map_err(|e| e.to_string())?;
            if *backedge_data.kind() != EdgeKind::Unconditional || backedge_data.target() != header
            {
                return Ok(None);
            }
            Ok(Some(LoopBody::Linear(group)))
        };

        let yes_body = backedge_body(yes)?;
        let no_body = backedge_body(no)?;
        let (body_edge, exit_edge, body) = match (yes_body, no_body) {
            (Some(body), None) => (yes, no, body),
            (None, Some(body)) => (no, yes, body),
            _ => return Ok(None),
        };
        let exit_target = self
            .cfg
            .edge(exit_edge)
            .map_err(|e| e.to_string())?
            .target();
        let exit_reenters_body = match body {
            LoopBody::Empty => false,
            LoopBody::Linear(group) => self.group_states.get(&exit_target) == Some(&group),
        };
        if exit_target == header || exit_reenters_body {
            return Ok(None);
        }
        Ok(Some(StructuredExit::Loop {
            body,
            body_edge,
            exit_edge,
        }))
    }
}

struct RenderView<'a, T> {
    cfg: &'a Cfg<T>,
    group_states: &'a HashMap<BlockId, usize>,
    active_groups: &'a [bool],
    prefix: &'a str,
    prepared: &'a [PreparedState],
    groups: &'a [Vec<usize>],
}

impl<T> RenderView<'_, T> {
    fn dispatch_to(&self, out: &mut String, target: BlockId) -> Result<(), String> {
        let group = self.group_states[&target];
        if !self.active_groups[group] {
            return Err("dispatcher transfer enters a structured region body".into());
        }
        writeln!(
            out,
            "{}_pc = {group};\ncontinue {}_loop;",
            self.prefix, self.prefix
        )
        .map_err(|e| e.to_string())
    }

    fn transfer(&self, out: &mut String, edge_id: EdgeId, copies: &str) -> Result<(), String> {
        let target = self.cfg.edge(edge_id).map_err(|e| e.to_string())?.target();
        append(out, copies);
        self.dispatch_to(out, target)
    }

    fn linear_group(&self, out: &mut String, group: usize) -> Result<(), String> {
        for &index in &self.groups[group] {
            out.push_str("{\n");
            append(out, &self.prepared[index].instructions);
            match &self.prepared[index].exit {
                Exit::Terminal(text) => append(out, text),
                Exit::Jump(_, copies) => append(out, copies),
                Exit::Branch(_, _, _, _, _) | Exit::Switch(_, _) => {
                    return Err("structured linear path contains a branch".into());
                }
            }
            out.push_str("}\n");
        }
        Ok(())
    }

    fn arm(&self, out: &mut String, arm: &LinearArm) -> Result<(), String> {
        if let Some(group) = arm.group {
            let last = *self.groups[group].last().ok_or("empty dispatch group")?;
            match (&arm.end, &self.prepared[last].exit) {
                (ArmEnd::Terminal, Exit::Terminal(_)) => {}
                (ArmEnd::Join(_), Exit::Jump(_, _)) => {}
                _ => return Err("structured branch arm no longer matches its proof".into()),
            }
            self.linear_group(out, group)?;
        }
        Ok(())
    }

    fn structured_if(
        &self,
        out: &mut String,
        exit: &StructuredExit,
        condition: &str,
    ) -> Result<(), String> {
        let StructuredExit::If {
            yes: yes_arm,
            no: no_arm,
            join,
        } = exit
        else {
            return Err("expected a structured conditional".into());
        };
        writeln!(out, "if ({condition}) {{").map_err(|e| e.to_string())?;
        self.arm(out, yes_arm)?;
        out.push_str("} else {\n");
        self.arm(out, no_arm)?;
        out.push_str("}\n");
        if let Some(join) = join {
            self.dispatch_to(out, *join)?;
        }
        Ok(())
    }

    fn structured_loop(
        &self,
        out: &mut String,
        header: &PreparedState,
        exit: &StructuredExit,
    ) -> Result<(), String> {
        let StructuredExit::Loop {
            body,
            body_edge,
            exit_edge,
        } = exit
        else {
            return Err("expected a structured loop".into());
        };
        let Exit::Branch(condition, yes, yes_copies, no, no_copies) = &header.exit else {
            return Err("structured loop header is not conditional".into());
        };
        let (body_copies, exit_copies, body_on_true) = if body_edge == yes {
            (yes_copies, no_copies, true)
        } else if body_edge == no {
            (no_copies, yes_copies, false)
        } else {
            return Err("structured loop lost its original branch edge".into());
        };
        let exit_target = self
            .cfg
            .edge(*exit_edge)
            .map_err(|e| e.to_string())?
            .target();

        out.push_str("while (true) {\n{\n");
        append(out, &header.instructions);
        writeln!(out, "if ({condition}) {{").map_err(|e| e.to_string())?;
        if body_on_true {
            append_scoped(out, body_copies);
        } else {
            append_scoped(out, exit_copies);
            out.push_str("break;\n");
        }
        out.push_str("} else {\n");
        if body_on_true {
            append_scoped(out, exit_copies);
            out.push_str("break;\n");
        } else {
            append_scoped(out, body_copies);
        }
        out.push_str("}\n}\n");
        if let LoopBody::Linear(group) = body {
            // The loop body is a sibling of the header scope. Header-local
            // declarations therefore cannot leak into the original body block.
            self.linear_group(out, *group)?;
        }
        out.push_str("}\n");
        self.dispatch_to(out, exit_target)
    }
}
/// Render a canonical snapshot without duplicating real instruction blocks.
/// The parent reserves `{prefix}_pc`, `{prefix}_loop`, and the
/// `{prefix}_test_` identifier family, distinct from all source locals.
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
    // Preserve the original BFS callback order, including edge-copy/condition
    // ordering. Fusion below changes only runtime scaffolding, not parent state.
    let mut prepared = Vec::with_capacity(snapshot.states.len());
    for state in &snapshot.states {
        let instructions = emitter.instructions(state.block)?;
        let edges = state
            .outgoing
            .iter()
            .map(|&id| cfg.edge(id))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let exit = match edges.as_slice() {
            [] => Exit::Terminal(emitter.terminal(state.block)?),
            [_] => Exit::Jump(state.outgoing[0], emitter.copies(state.outgoing[0])?),
            [a, b] if matches!(a.kind(), EdgeKind::BranchTrue | EdgeKind::BranchFalse) => {
                let (yes, no) = if a.kind() == &EdgeKind::BranchTrue {
                    (state.outgoing[0], state.outgoing[1])
                } else {
                    (state.outgoing[1], state.outgoing[0])
                };
                let yes_copies = emitter.copies(yes)?;
                let no_copies = emitter.copies(no)?;
                let condition = emitter.condition(state.block)?;
                let _ = b;
                Exit::Branch(condition, yes, yes_copies, no, no_copies)
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
                    arms.push((label, id, emitter.copies(id)?));
                }
                Exit::Switch(emitter.selector(state.block)?, arms)
            }
        };
        prepared.push(PreparedState { instructions, exit });
    }
    // Fuse only a unique unconditional successor. Entry, joins, parallel edges
    // and branch/switch transfers remain boundaries. A reachable cycle always
    // has a boundary (entry or a multi-predecessor header), so groups terminate.
    let mut next = vec![None; snapshot.states.len()];
    let mut previous = vec![None; snapshot.states.len()];
    for (index, state) in snapshot.states.iter().enumerate() {
        let [edge_id] = state.outgoing.as_slice() else {
            continue;
        };
        let edge = cfg.edge(*edge_id).map_err(|e| e.to_string())?;
        let target = edge.target();
        if *edge.kind() == EdgeKind::Unconditional
            && target != snapshot.entry
            && cfg.block(target).map_err(|e| e.to_string())?.incoming() == [*edge_id]
        {
            next[index] = Some(states[&target]);
            previous[states[&target]] = Some(index);
        }
    }
    let mut groups = Vec::new();
    let mut group_states = HashMap::with_capacity(states.len());
    for (head, predecessor) in previous.iter().enumerate() {
        if predecessor.is_some() {
            continue;
        }
        let mut members = Vec::new();
        let mut cursor = Some(head);
        while let Some(index) = cursor {
            if group_states
                .insert(snapshot.states[index].block, groups.len())
                .is_some()
            {
                return Err("overlapping dispatch chain".into());
            }
            members.push(index);
            cursor = next[index];
        }
        groups.push(members);
    }
    if group_states.len() != states.len() {
        return Err("incomplete dispatch chains".into());
    }
    let mut cyclic_blocks = HashSet::new();
    for component in decomp_core::analysis::strongly_connected_components(cfg) {
        let mut is_cycle = component.len() > 1;
        if let [block] = component.as_slice() {
            for &edge in cfg.block(*block).map_err(|e| e.to_string())?.outgoing() {
                if cfg.edge(edge).map_err(|e| e.to_string())?.target() == *block {
                    is_cycle = true;
                    break;
                }
            }
        }
        if is_cycle {
            cyclic_blocks.extend(component);
        }
    }
    // Structuring is deliberately local: each removed group has one proven
    // incoming edge and one linear exit. All other groups retain exact PC cases.
    let proof = ProofView {
        cfg,
        snapshot,
        prepared: &prepared,
        groups: &groups,
        group_states: &group_states,
    };
    let mut structured: Vec<Option<StructuredExit>> =
        std::iter::repeat_with(|| None).take(groups.len()).collect();
    let mut consumed = vec![false; groups.len()];
    for (group, members) in groups.iter().enumerate() {
        if consumed[group] {
            continue;
        }
        let last = *members.last().ok_or("empty dispatch group")?;
        let Exit::Branch(_, yes, _, no, _) = &prepared[last].exit else {
            continue;
        };
        let header = snapshot.states[last].block;
        if let Some(region) = proof.loop_region(group, header, *yes, *no)? {
            let body_group = match &region {
                StructuredExit::Loop { body, .. } => match body {
                    LoopBody::Empty => None,
                    LoopBody::Linear(group) => Some(*group),
                },
                StructuredExit::If { .. } => return Err("loop analysis returned an if".into()),
            };
            if body_group.is_none_or(|body| !consumed[body]) {
                if let Some(body) = body_group {
                    consumed[body] = true;
                }
                structured[group] = Some(region);
                continue;
            }
        }
        // An unrecognized cyclic region (including every irreducible SCC) keeps
        // its original state transfers; local structuring never flattens it.
        if cyclic_blocks.contains(&header) {
            continue;
        }
        if let Some(region) = proof.branch_region(group, *yes, *no)? {
            let (yes_group, no_group) = match &region {
                StructuredExit::If { yes, no, .. } => (yes.group, no.group),
                StructuredExit::Loop { .. } => {
                    return Err("branch analysis returned a loop".into());
                }
            };
            if yes_group.is_none_or(|arm| !consumed[arm])
                && no_group.is_none_or(|arm| !consumed[arm])
            {
                if let Some(arm) = yes_group {
                    consumed[arm] = true;
                }
                if let Some(arm) = no_group {
                    consumed[arm] = true;
                }
                structured[group] = Some(region);
            }
        }
    }
    let active_groups: Vec<_> = consumed.iter().map(|is_consumed| !is_consumed).collect();
    let renderer = RenderView {
        cfg,
        group_states: &group_states,
        active_groups: &active_groups,
        prefix,
        prepared: &prepared,
        groups: &groups,
    };
    let mut out =
        format!("int {prefix}_pc = 0;\n{prefix}_loop: while (true) {{\nswitch ({prefix}_pc) {{\n");
    for (group, members) in groups.iter().enumerate() {
        if consumed[group] {
            continue;
        }
        writeln!(out, "case {group}: {{").map_err(|e| e.to_string())?;
        for (ordinal, &index) in members.iter().enumerate() {
            let state = &prepared[index];
            if ordinal + 1 < members.len()
                && state.instructions.is_empty()
                && matches!(&state.exit, Exit::Jump(_, copies) if copies.is_empty())
            {
                continue;
            }
            let last_member = ordinal + 1 == members.len();
            if last_member && matches!(&structured[group], Some(StructuredExit::Loop { .. })) {
                renderer.structured_loop(
                    &mut out,
                    state,
                    structured[group].as_ref().ok_or("missing loop region")?,
                )?;
                continue;
            }
            if last_member && matches!(&structured[group], Some(StructuredExit::If { .. })) {
                let Exit::Branch(condition, _, yes_copies, _, no_copies) = &state.exit else {
                    return Err("structured conditional lost its original branch".into());
                };
                // Evaluate the condition and selected edge copies in the source
                // block's scope, then close it before emitting either arm.
                let branch_test = format!("{prefix}_test_{group}");
                writeln!(out, "boolean {branch_test};").map_err(|e| e.to_string())?;
                out.push_str("{\n");
                append(&mut out, &state.instructions);
                writeln!(out, "{branch_test} = {condition};\nif ({branch_test}) {{")
                    .map_err(|e| e.to_string())?;
                append(&mut out, yes_copies);
                out.push_str("} else {\n");
                append(&mut out, no_copies);
                out.push_str("}\n");
                out.push_str("}\n");
                renderer.structured_if(
                    &mut out,
                    structured[group].as_ref().ok_or("missing if region")?,
                    &branch_test,
                )?;
                continue;
            }
            // Original case-local auxiliary declarations keep separate scopes.
            if members.len() > 1 {
                out.push_str("{\n");
            }
            append(&mut out, &state.instructions);
            if ordinal + 1 < members.len() {
                let Exit::Jump(_, copies) = &state.exit else {
                    return Err("nonlinear dispatch chain".into());
                };
                append(&mut out, copies);
            } else {
                match &state.exit {
                    Exit::Terminal(text) => append(&mut out, text),
                    Exit::Jump(edge, copies) => renderer.transfer(&mut out, *edge, copies)?,
                    Exit::Branch(condition, yes, yes_copies, no, no_copies) => {
                        writeln!(out, "if ({condition}) {{").map_err(|e| e.to_string())?;
                        renderer.transfer(&mut out, *yes, yes_copies)?;
                        out.push_str("} else {\n");
                        renderer.transfer(&mut out, *no, no_copies)?;
                        out.push_str("}\n");
                    }
                    Exit::Switch(selector, arms) => {
                        writeln!(out, "switch ({selector}) {{").map_err(|e| e.to_string())?;
                        for (label, edge, copies) in arms {
                            writeln!(out, "{label}: {{").map_err(|e| e.to_string())?;
                            renderer.transfer(&mut out, *edge, copies)?;
                            out.push_str("}\n");
                        }
                        out.push_str("}\n");
                    }
                }
            }
            if members.len() > 1 {
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
