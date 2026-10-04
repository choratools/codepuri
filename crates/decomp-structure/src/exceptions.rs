//! Explicit, frontend-neutral exception scopes and acyclic normal-flow regions.
//! No source scope or finally inference is performed here.
use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::{BlockId, EdgeId},
};
use std::collections::HashSet;

/// Catch order is semantic metadata, never CFG adjacency order. `key` identifies
/// the frontend catch type; a backend must resolve it to a proven Throwable type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catch {
    /// Frontend-owned caught type identity.
    pub key: String,
    /// Original handler entry, including its Phi cluster.
    pub entry: BlockId,
}
/// A facade supplies block identities AFTER SSA/CFG rewriting. `protected` is
/// the exact ordered scope, including nonthrowing commit and Phi nodes. The
/// exclusive continuation is shared by normal and all handler paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    /// Exact ordered protected nodes.
    pub protected: Vec<BlockId>,
    /// Typed dispatch in source order.
    pub catches: Vec<Catch>,
    /// Exclusive common normal continuation.
    pub continuation: BlockId,
    /// Catch-all cleanup is not evidence of Java finally. Reject it explicitly.
    pub has_unproven_finally: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// Validated, complete linear exception region.
pub struct ExceptionStructure {
    /// Instructions before entering the scope.
    pub prefix: Vec<BlockId>,
    /// Validated source scope.
    pub scope: Scope,
    /// Disjoint handler paths in catch order.
    pub handlers: Vec<Vec<BlockId>>,
    /// Shared continuation through method termination.
    pub suffix: Vec<BlockId>,
    /// Original exceptional edges.
    pub exceptional: Vec<EdgeId>,
    /// Input CFG revision.
    pub revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// Specific failure without a partial region.
pub enum Error {
    /// Invalid CFG or identity.
    Invalid(String),
    /// Scope outside the conservative grammar.
    Unsupported(&'static str),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exception structure: {self:?}")
    }
}
impl std::error::Error for Error {}

/// An exact single-entry source range, with an optional exclusive normal exit.
/// Catch-all is represented by its original CFG key, last in dispatch order;
/// this metadata makes no assertion that its body is a finally clause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionScope {
    /// All and only source-covered nodes and proven synthetic nodes.
    pub protected: Vec<BlockId>,
    /// Original dispatch order.
    pub catches: Vec<Catch>,
    /// None when every protected path terminates.
    pub continuation: Option<BlockId>,
}

/// Acyclic normal-flow tree retaining original branch edge identities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Region {
    /// One original instruction node.
    Block(BlockId),
    /// Conditional head followed by disjoint arms and an optional later join.
    If {
        /// Original conditional instruction.
        head: BlockId,
        /// Original true edge.
        then_edge: EdgeId,
        /// Original false edge.
        else_edge: EdgeId,
        /// True arm.
        then_body: Vec<Region>,
        /// False arm.
        else_body: Vec<Region>,
    },
}

/// Complete validated partition, including abrupt handler exits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionStructure {
    /// Outside the source range.
    pub prefix: Vec<Region>,
    /// Exact protected normal flow.
    pub protected: Vec<Region>,
    /// Handler normal flow, in dispatch order.
    pub handlers: Vec<Vec<Region>>,
    /// Shared continuing flow, if any.
    pub suffix: Vec<Region>,
    /// Original exceptional edges.
    pub exceptional: Vec<EdgeId>,
}

/// Structure an acyclic contiguous source scope. No instruction is duplicated,
/// no terminal arm is required to reach a join, and no exceptional edge is lost.
pub fn structure_region<T>(cfg: &Cfg<T>, scope: &RegionScope) -> Result<RegionStructure, Error> {
    cfg.validate().map_err(|e| Error::Invalid(e.to_string()))?;
    let protected: HashSet<_> = scope.protected.iter().copied().collect();
    if protected.is_empty() || protected.len() != scope.protected.len() || scope.catches.is_empty()
    {
        return Err(Error::Unsupported("empty or duplicate region metadata"));
    }
    let normal = |b| normal_edges(cfg, b);
    let entry_candidates: Vec<_> = scope
        .protected
        .iter()
        .copied()
        .filter(|b| {
            *b == cfg.entry()
                || cfg.block(*b).is_ok_and(|n| {
                    n.incoming().iter().any(|id| {
                        cfg.edge(*id).is_ok_and(|e| {
                            !matches!(e.kind(), EdgeKind::Exception(_))
                                && !protected.contains(&e.source())
                        })
                    })
                })
        })
        .collect();
    let [entry] = entry_candidates.as_slice() else {
        return Err(Error::Unsupported("protected region is not single-entry"));
    };
    for &b in &scope.protected {
        for (_, target) in normal(b)? {
            if !protected.contains(&target) && Some(target) != scope.continuation {
                return Err(Error::Unsupported("multiple protected normal exits"));
            }
        }
    }
    let mut seen = HashSet::new();
    let prefix = region_walk(cfg, cfg.entry(), Some(*entry), &mut seen)?;
    let before = seen.clone();
    let body = region_walk(cfg, *entry, scope.continuation, &mut seen)?;
    if seen.difference(&before).copied().collect::<HashSet<_>>() != protected {
        return Err(Error::Unsupported("noncontiguous protected region"));
    }
    let mut keys = HashSet::new();
    let mut handlers = Vec::new();
    for c in &scope.catches {
        if !keys.insert(&c.key) {
            return Err(Error::Unsupported("ambiguous catch identity"));
        }
        handlers.push(region_walk(cfg, c.entry, scope.continuation, &mut seen)?);
    }
    let suffix = match scope.continuation {
        Some(b) => region_walk(cfg, b, None, &mut seen)?,
        None => Vec::new(),
    };
    if seen.len() != cfg.block_ids().count() {
        return Err(Error::Unsupported("uncovered region nodes"));
    }
    let mut exceptional = Vec::new();
    for id in cfg.edge_ids() {
        let e = cfg.edge(id).map_err(|e| Error::Invalid(e.to_string()))?;
        if let EdgeKind::Exception(key) = e.kind() {
            if !protected.contains(&e.source())
                || !scope
                    .catches
                    .iter()
                    .any(|c| c.key == *key && c.entry == e.target())
            {
                return Err(Error::Unsupported("exception edge outside explicit region"));
            }
            exceptional.push(id);
        }
    }
    for &b in &scope.protected {
        let edges: Vec<_> = exceptional
            .iter()
            .filter_map(|id| cfg.edge(*id).ok())
            .filter(|e| e.source() == b)
            .collect();
        if !edges.is_empty()
            && (edges.len() != scope.catches.len()
                || scope.catches.iter().any(|c| {
                    edges
                        .iter()
                        .filter(|e| {
                            e.target() == c.entry && e.kind() == &EdgeKind::Exception(c.key.clone())
                        })
                        .count()
                        != 1
                }))
        {
            return Err(Error::Unsupported(
                "incomplete or duplicate region dispatch",
            ));
        }
    }
    if exceptional.is_empty() {
        return Err(Error::Unsupported("no throwing sites"));
    }
    Ok(RegionStructure {
        prefix,
        protected: body,
        handlers,
        suffix,
        exceptional,
    })
}

fn normal_edges<T>(cfg: &Cfg<T>, b: BlockId) -> Result<Vec<(EdgeId, BlockId)>, Error> {
    cfg.block(b)
        .map_err(|e| Error::Invalid(e.to_string()))?
        .outgoing()
        .iter()
        .filter_map(|id| match cfg.edge(*id) {
            Ok(e) if matches!(e.kind(), EdgeKind::Exception(_)) => None,
            Ok(e) => Some(Ok((*id, e.target()))),
            Err(e) => Some(Err(Error::Invalid(e.to_string()))),
        })
        .collect()
}

fn reachable<T>(
    cfg: &Cfg<T>,
    b: BlockId,
    stop: Option<BlockId>,
    active: &mut HashSet<BlockId>,
    out: &mut Vec<BlockId>,
) -> Result<(), Error> {
    if Some(b) == stop {
        return Ok(());
    }
    if !active.insert(b) {
        return Err(Error::Unsupported("cyclic exception region"));
    }
    if !out.contains(&b) {
        out.push(b);
        for (_, t) in normal_edges(cfg, b)? {
            reachable(cfg, t, stop, active, out)?;
        }
    }
    active.remove(&b);
    Ok(())
}

fn region_walk<T>(
    cfg: &Cfg<T>,
    start: BlockId,
    stop: Option<BlockId>,
    seen: &mut HashSet<BlockId>,
) -> Result<Vec<Region>, Error> {
    let mut out = Vec::new();
    let mut next = Some(start);
    while let Some(b) = next {
        if Some(b) == stop {
            break;
        }
        if !seen.insert(b) {
            return Err(Error::Unsupported("overlapping or cyclic exception region"));
        }
        let edges = normal_edges(cfg, b)?;
        match edges.as_slice() {
            [] => {
                out.push(Region::Block(b));
                next = None;
            }
            [(id, t)]
                if cfg
                    .edge(*id)
                    .is_ok_and(|e| e.kind() == &EdgeKind::Unconditional) =>
            {
                out.push(Region::Block(b));
                next = Some(*t);
            }
            [a, z] => {
                let (yes, no) = if cfg
                    .edge(a.0)
                    .is_ok_and(|e| e.kind() == &EdgeKind::BranchTrue)
                    && cfg
                        .edge(z.0)
                        .is_ok_and(|e| e.kind() == &EdgeKind::BranchFalse)
                {
                    (*a, *z)
                } else if cfg
                    .edge(z.0)
                    .is_ok_and(|e| e.kind() == &EdgeKind::BranchTrue)
                    && cfg
                        .edge(a.0)
                        .is_ok_and(|e| e.kind() == &EdgeKind::BranchFalse)
                {
                    (*z, *a)
                } else {
                    return Err(Error::Unsupported("unsupported normal dispatch"));
                };
                let mut left = Vec::new();
                let mut right = Vec::new();
                reachable(cfg, yes.1, stop, &mut HashSet::new(), &mut left)?;
                reachable(cfg, no.1, stop, &mut HashSet::new(), &mut right)?;
                let join = left.iter().find(|n| right.contains(n)).copied().or(stop);
                let then_body = region_walk(cfg, yes.1, join, seen)?;
                let else_body = region_walk(cfg, no.1, join, seen)?;
                out.push(Region::If {
                    head: b,
                    then_edge: yes.0,
                    else_edge: no.0,
                    then_body,
                    else_body,
                });
                next = join;
            }
            _ => return Err(Error::Unsupported("unsupported normal dispatch")),
        }
    }
    Ok(out)
}

/// Exception-aware acyclic tree with each source scope placed exactly once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisjointRegion {
    /// One ordinary original node.
    Block(BlockId),
    /// Original conditional with independently reachable arms.
    If {
        /// Original branch instruction.
        head: BlockId,
        /// Original true edge.
        then_edge: EdgeId,
        /// Original false edge.
        else_edge: EdgeId,
        /// True branch, including scoped tries.
        then_body: Vec<DisjointRegion>,
        /// False branch, including scoped tries.
        else_body: Vec<DisjointRegion>,
    },
    /// Index into the validated source-scope table.
    Try(usize),
}
/// Entire single-placement body and each exact protected/handler tree.
#[derive(Clone, Debug)]
pub struct DisjointStructure {
    /// Normal body containing scope nodes.
    pub body: Vec<DisjointRegion>,
    /// Scope-local trees; prefix/suffix are empty.
    pub scopes: Vec<RegionStructure>,
}
/// Structure disjoint, nonnested scopes within ordinary acyclic branches.
/// Shared handlers and cyclic/overlapping ordinary regions reject.
pub fn structure_disjoint<T>(
    cfg: &Cfg<T>,
    scopes: &[RegionScope],
) -> Result<DisjointStructure, Error> {
    structure_disjoint_impl(cfg, scopes, false)
}

/// One identical typed handler shared by acyclic disjoint protected ranges.
/// Handler region trees are identical references in this plan: consumers MUST
/// lower the common handler once, using forward catch transfers. Normal flow
/// may not reach it. Original exception edges and scope membership are retained.
pub fn structure_disjoint_shared_handlers<T>(
    cfg: &Cfg<T>,
    scopes: &[RegionScope],
) -> Result<DisjointStructure, Error> {
    let Some(first) = scopes.first().and_then(|s| s.catches.first()) else {
        return Err(Error::Unsupported("missing shared handler"));
    };
    if scopes.len() < 2
        || first.key.is_empty()
        || scopes.iter().any(|scope| {
            scope.catches.len() != 1
                || scope.catches[0] != *first
                || scope.protected.contains(&first.entry)
                || scope.continuation == Some(first.entry)
        })
    {
        return Err(Error::Unsupported("mismatched shared handler group"));
    }
    structure_disjoint_impl(cfg, scopes, true)
}

fn structure_disjoint_impl<T>(
    cfg: &Cfg<T>,
    scopes: &[RegionScope],
    shared: bool,
) -> Result<DisjointStructure, Error> {
    cfg.validate().map_err(|e| Error::Invalid(e.to_string()))?;
    if scopes.is_empty() || scopes.len() > 64 || cfg.block_ids().count() > 16384 {
        return Err(Error::Unsupported("disjoint scope budget"));
    }
    let mut all_protected = HashSet::new();
    let mut entries = Vec::new();
    for scope in scopes {
        let protected: HashSet<_> = scope.protected.iter().copied().collect();
        if protected.is_empty()
            || protected.len() != scope.protected.len()
            || scope.catches.is_empty()
            || protected.iter().any(|b| !all_protected.insert(*b))
        {
            return Err(Error::Unsupported("overlapping protected scopes"));
        }
        let candidates: Vec<_> = scope
            .protected
            .iter()
            .copied()
            .filter(|b| {
                *b == cfg.entry()
                    || cfg.block(*b).is_ok_and(|n| {
                        n.incoming().iter().any(|id| {
                            cfg.edge(*id).is_ok_and(|e| {
                                !matches!(e.kind(), EdgeKind::Exception(_))
                                    && !protected.contains(&e.source())
                            })
                        })
                    })
            })
            .collect();
        let [entry] = candidates.as_slice() else {
            return Err(Error::Unsupported("protected region is not single-entry"));
        };
        for &b in &protected {
            for (_, t) in normal_edges(cfg, b)? {
                if !protected.contains(&t) && Some(t) != scope.continuation {
                    return Err(Error::Unsupported("multiple protected normal exits"));
                }
            }
        }
        entries.push(*entry);
    }
    let mut seen = HashSet::new();
    let mut trees = Vec::new();
    for scope in scopes {
        let before = seen.clone();
        let protected = region_walk(cfg, entries[trees.len()], scope.continuation, &mut seen)?;
        if seen.difference(&before).copied().collect::<HashSet<_>>()
            != scope.protected.iter().copied().collect()
        {
            return Err(Error::Unsupported("noncontiguous protected region"));
        }
        trees.push(RegionStructure {
            prefix: Vec::new(),
            protected,
            handlers: Vec::new(),
            suffix: Vec::new(),
            exceptional: Vec::new(),
        });
    }
    for (index, scope) in scopes.iter().enumerate() {
        let mut keys = HashSet::new();
        for catch in &scope.catches {
            if !keys.insert(&catch.key) {
                return Err(Error::Unsupported("ambiguous catch identity"));
            }
            if shared && index > 0 {
                trees[index].handlers = trees[0].handlers.clone();
                continue;
            }
            let before = seen.clone();
            let body = region_walk(
                cfg,
                catch.entry,
                if shared { None } else { scope.continuation },
                &mut seen,
            )?;
            if seen.difference(&before).any(|b| all_protected.contains(b)) {
                return Err(Error::Unsupported("nested handler scope"));
            }
            trees[index].handlers.push(body);
        }
    }
    for id in cfg.edge_ids() {
        let e = cfg.edge(id).map_err(|e| Error::Invalid(e.to_string()))?;
        if let EdgeKind::Exception(key) = e.kind() {
            let Some(index) = scopes
                .iter()
                .position(|s| s.protected.contains(&e.source()))
            else {
                return Err(Error::Unsupported("exception edge outside explicit region"));
            };
            if !scopes[index]
                .catches
                .iter()
                .any(|c| c.key == *key && c.entry == e.target())
            {
                return Err(Error::Unsupported("exception dispatch mismatch"));
            }
            trees[index].exceptional.push(id);
        }
    }
    for (s, tree) in scopes.iter().zip(&trees) {
        if tree.exceptional.is_empty() {
            return Err(Error::Unsupported("no throwing sites"));
        }
        for &b in &s.protected {
            let edges: Vec<_> = tree
                .exceptional
                .iter()
                .filter_map(|id| cfg.edge(*id).ok())
                .filter(|e| e.source() == b)
                .collect();
            if !edges.is_empty()
                && (edges.len() != s.catches.len()
                    || s.catches.iter().any(|c| {
                        edges
                            .iter()
                            .filter(|e| {
                                e.target() == c.entry
                                    && e.kind() == &EdgeKind::Exception(c.key.clone())
                            })
                            .count()
                            != 1
                    }))
            {
                return Err(Error::Unsupported(
                    "incomplete or duplicate region dispatch",
                ));
            }
        }
    }
    let mut placed = HashSet::new();
    let body = disjoint_walk(
        cfg,
        cfg.entry(),
        None,
        &entries,
        scopes,
        &mut seen,
        &mut placed,
        0,
    )?;
    if seen.len() != cfg.block_ids().count() || placed.len() != scopes.len() {
        return Err(Error::Unsupported("uncovered disjoint region nodes"));
    }
    Ok(DisjointStructure {
        body,
        scopes: trees,
    })
}
#[allow(clippy::too_many_arguments)]
fn disjoint_walk<T>(
    cfg: &Cfg<T>,
    start: BlockId,
    stop: Option<BlockId>,
    entries: &[BlockId],
    scopes: &[RegionScope],
    seen: &mut HashSet<BlockId>,
    placed: &mut HashSet<usize>,
    depth: usize,
) -> Result<Vec<DisjointRegion>, Error> {
    if depth > 128 {
        return Err(Error::Unsupported("disjoint branch depth budget"));
    }
    let mut out = Vec::new();
    let mut next = Some(start);
    while let Some(b) = next {
        if Some(b) == stop {
            break;
        }
        if let Some(i) = entries.iter().position(|e| *e == b) {
            if !placed.insert(i) {
                return Err(Error::Unsupported("overlapping scope placement"));
            }
            out.push(DisjointRegion::Try(i));
            next = scopes[i].continuation;
            continue;
        }
        if !seen.insert(b) {
            return Err(Error::Unsupported("overlapping or cyclic exception region"));
        }
        let edges = normal_edges(cfg, b)?;
        match edges.as_slice() {
            [] => {
                out.push(DisjointRegion::Block(b));
                next = None;
            }
            [(id, t)]
                if cfg
                    .edge(*id)
                    .is_ok_and(|e| e.kind() == &EdgeKind::Unconditional) =>
            {
                out.push(DisjointRegion::Block(b));
                next = Some(*t);
            }
            [a, z] => {
                let (yes, no) = if cfg
                    .edge(a.0)
                    .is_ok_and(|e| e.kind() == &EdgeKind::BranchTrue)
                    && cfg
                        .edge(z.0)
                        .is_ok_and(|e| e.kind() == &EdgeKind::BranchFalse)
                {
                    (*a, *z)
                } else if cfg
                    .edge(z.0)
                    .is_ok_and(|e| e.kind() == &EdgeKind::BranchTrue)
                    && cfg
                        .edge(a.0)
                        .is_ok_and(|e| e.kind() == &EdgeKind::BranchFalse)
                {
                    (*z, *a)
                } else {
                    return Err(Error::Unsupported("unsupported normal dispatch"));
                };
                let mut left = Vec::new();
                let mut right = Vec::new();
                reachable(cfg, yes.1, stop, &mut HashSet::new(), &mut left)?;
                reachable(cfg, no.1, stop, &mut HashSet::new(), &mut right)?;
                let join = left.iter().find(|n| right.contains(n)).copied().or(stop);
                let then_body =
                    disjoint_walk(cfg, yes.1, join, entries, scopes, seen, placed, depth + 1)?;
                let else_body =
                    disjoint_walk(cfg, no.1, join, entries, scopes, seen, placed, depth + 1)?;
                out.push(DisjointRegion::If {
                    head: b,
                    then_edge: yes.0,
                    else_edge: no.0,
                    then_body,
                    else_body,
                });
                next = join;
            }
            _ => return Err(Error::Unsupported("unsupported normal dispatch")),
        }
    }
    Ok(out)
}

/// Conservative initial grammar: one linear protected scope, disjoint linear
/// catches and a common linear continuation. All blocks/edges must be covered.
pub fn structure<T>(cfg: &Cfg<T>, scopes: &[Scope]) -> Result<ExceptionStructure, Error> {
    let invalid = |e: decomp_core::cfg::CfgError| Error::Invalid(e.to_string());
    cfg.validate().map_err(invalid)?;
    if scopes.len() != 1 {
        return Err(Error::Unsupported("exactly one scope required"));
    }
    let scope = &scopes[0];
    if scope.has_unproven_finally || scope.protected.is_empty() || scope.catches.is_empty() {
        return Err(Error::Unsupported(
            "empty scope, catch-all or unproven finally",
        ));
    }
    let mut seen = HashSet::new();
    let mut walk = |start: BlockId, stop: Option<BlockId>| -> Result<Vec<BlockId>, Error> {
        let mut out = Vec::new();
        let mut next = Some(start);
        while let Some(b) = next {
            if Some(b) == stop {
                break;
            }
            if !seen.insert(b) {
                return Err(Error::Unsupported("overlap or cycle"));
            }
            out.push(b);
            let normal: Vec<_> = cfg
                .block(b)
                .map_err(invalid)?
                .outgoing()
                .iter()
                .filter_map(|e| {
                    let edge = cfg.edge(*e).ok()?;
                    (!matches!(edge.kind(), EdgeKind::Exception(_))).then_some(edge)
                })
                .collect();
            next = match normal.as_slice() {
                [] if stop.is_none() => None,
                [e] if e.kind() == &EdgeKind::Unconditional => Some(e.target()),
                _ => return Err(Error::Unsupported("nonlinear or prematurely terminal path")),
            };
        }
        Ok(out)
    };
    let prefix = walk(cfg.entry(), Some(scope.protected[0]))?;
    let protected = walk(scope.protected[0], Some(scope.continuation))?;
    if protected != scope.protected {
        return Err(Error::Unsupported("noncontiguous protected metadata"));
    }
    let mut keys = HashSet::new();
    let mut handlers = Vec::new();
    for catch in &scope.catches {
        if catch.key.is_empty() || !keys.insert(&catch.key) {
            return Err(Error::Unsupported("ambiguous catch identity"));
        }
        handlers.push(walk(catch.entry, Some(scope.continuation))?);
    }
    let suffix = walk(scope.continuation, None)?;
    if seen.len() != cfg.block_ids().count() {
        return Err(Error::Unsupported("uncovered blocks"));
    }
    let mut exceptional = Vec::new();
    for id in cfg.edge_ids() {
        let e = cfg.edge(id).map_err(invalid)?;
        if let EdgeKind::Exception(key) = e.kind() {
            if !scope.protected.contains(&e.source())
                || !scope
                    .catches
                    .iter()
                    .any(|c| c.key == *key && c.entry == e.target())
            {
                return Err(Error::Unsupported("exception edge outside explicit scope"));
            }
            exceptional.push(id);
        }
    }
    // Every throw point must carry the complete ordered dispatch table.
    for b in &scope.protected {
        let edges: Vec<_> = exceptional
            .iter()
            .filter(|id| cfg.edge(**id).is_ok_and(|e| e.source() == *b))
            .collect();
        if !edges.is_empty()
            && (edges.len() != scope.catches.len()
                || scope.catches.iter().any(|c| {
                    edges
                        .iter()
                        .filter(|id| {
                            cfg.edge(***id).is_ok_and(|e| {
                                e.target() == c.entry
                                    && e.kind() == &EdgeKind::Exception(c.key.clone())
                            })
                        })
                        .count()
                        != 1
                }))
        {
            return Err(Error::Unsupported("incomplete or duplicate dispatch table"));
        }
    }
    if exceptional.is_empty() {
        return Err(Error::Unsupported("no throwing sites"));
    }
    Ok(ExceptionStructure {
        prefix,
        scope: scope.clone(),
        handlers,
        suffix,
        exceptional,
        revision: cfg.revision(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod region_tests {
    use super::*;
    #[test]
    fn diamond_catchall_terminal_handler_and_exact_coverage() {
        let mut cfg = Cfg::new(());
        let head = cfg.entry();
        let yes = cfg.add_block(());
        let no = cfg.add_block(());
        let join = cfg.add_block(());
        let handler = cfg.add_block(());
        cfg.add_edge(head, no, EdgeKind::BranchFalse).unwrap();
        cfg.add_edge(head, yes, EdgeKind::BranchTrue).unwrap();
        cfg.add_edge(yes, join, EdgeKind::Unconditional).unwrap();
        cfg.add_edge(no, join, EdgeKind::Unconditional).unwrap();
        cfg.add_edge(yes, handler, EdgeKind::Exception(String::new()))
            .unwrap();
        cfg.add_edge(no, handler, EdgeKind::Exception(String::new()))
            .unwrap();
        let mut scope = RegionScope {
            protected: vec![head, yes, no],
            catches: vec![Catch {
                key: String::new(),
                entry: handler,
            }],
            continuation: Some(join),
        };
        let tree = structure_region(&cfg, &scope).unwrap();
        assert!(
            matches!(&tree.protected[0],Region::If { head: h, then_body, else_body, .. } if *h==head && *then_body==[Region::Block(yes)] && *else_body==[Region::Block(no)])
        );
        assert_eq!(tree.handlers, vec![vec![Region::Block(handler)]]);
        assert_eq!(tree.suffix, vec![Region::Block(join)]);
        scope.protected.pop();
        assert!(structure_region(&cfg, &scope).is_err());
    }
}
#[cfg(test)]
#[path = "disjoint_exception_tests.rs"]
mod disjoint_tests;
