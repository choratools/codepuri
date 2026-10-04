//! Raw original DEX exception dispatch evidence; no structured-region proof.
use backend_java::{
    JavaResolver, JavaType,
    exceptions::{
        OriginalScope, RegionMetadata,
        scopes::{Catch, RegionScope},
    },
    type_compatibility::Hierarchy,
};
use decomp_core::{
    cfg::EdgeKind,
    ids::{BlockId, EdgeId},
};
use dex_decompiler::ssa::SsaMethod;
use dex_format::code::CodeItem;
use managed_ir::{insn::ManagedInsn, refs::ClassRef, value::ValueId};
use std::collections::{BTreeMap, BTreeSet};

/// One original ordered scope, without any single-entry/exit/loop grammar claim.
#[derive(Debug)]
pub struct RawScope {
    original: OriginalScope,
    scope: RegionScope,
    types: BTreeMap<String, JavaType>,
    hierarchy: Hierarchy,
    catch_all: bool,
    /// Actual original exception-edge identities grouped by encoded dispatch key.
    pub exception_edges: BTreeMap<String, Vec<EdgeId>>,
}
impl RegionMetadata for RawScope {
    fn original_scope(&self) -> Option<&OriginalScope> {
        Some(&self.original)
    }
    fn region_scope(&self) -> &RegionScope {
        &self.scope
    }
    fn catch_type(&self, key: &str) -> Option<JavaType> {
        self.types.get(key).cloned()
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.hierarchy
    }
    fn is_catch_all(&self, key: &str) -> bool {
        self.catch_all && key.is_empty()
    }
}
/// Raw source scopes and exact original catch-entry values. Never authorizes
/// Java exception restructuring, synthetic catches, or altered dispatch order.
#[derive(Debug)]
pub struct Metadata {
    /// Original try-item ordinal order and encoded catch priority.
    pub scopes: Vec<RawScope>,
    /// Actual handler scaffold entry to its verified captured SSA identity.
    pub captures: BTreeMap<BlockId, Option<ValueId>>,
}

/// Preserve raw DEX ranges and exception targets, including cyclic CFGs and
/// several normal exits. Reject unproved synthetic provenance/capture entries.
pub fn from_dex_raw(
    ssa: &SsaMethod,
    code: &CodeItem,
    resolver: &impl JavaResolver,
    hierarchy: &Hierarchy,
) -> Result<Metadata, String> {
    let reject = |s: &str| format!("raw exception metadata: {s}");
    if code.tries.is_empty() || code.tries.len() > 64 {
        return Err(reject("scope budget"));
    }
    let decoded = dex_decompiler::decode_method(code)
        .map_err(|_| reject("invalid original instruction stream"))?;
    let boundaries: BTreeSet<_> = decoded
        .iter()
        .map(|i| i.offset)
        .chain(std::iter::once(code.insns_size))
        .collect();
    let ir = ssa.ir();
    let cfg = ir.cfg();
    let insn = |b| -> Result<_, String> {
        let id = *cfg.block(b).map_err(|e| e.to_string())?.payload();
        Ok((id, ir.instruction(id).map_err(|e| e.to_string())?))
    };
    let next = |b| -> Result<BlockId, String> {
        let normal: Vec<_> = cfg
            .block(b)
            .map_err(|e| e.to_string())?
            .outgoing()
            .iter()
            .filter_map(|id| {
                let e = cfg.edge(*id).ok()?;
                (!matches!(e.kind(), EdgeKind::Exception(_))).then_some(e)
            })
            .collect();
        match normal.as_slice() {
            [e] if e.kind() == &EdgeKind::Unconditional => Ok(e.target()),
            _ => Err(reject("synthetic scaffold not exclusive linear successor")),
        }
    };
    let mut metadata = Metadata {
        scopes: vec![],
        captures: BTreeMap::new(),
    };
    for (ordinal, range) in code.tries.iter().enumerate() {
        let end = range
            .start_addr
            .checked_add(u32::from(range.insn_count))
            .ok_or_else(|| reject("range overflow"))?;
        if range.insn_count == 0
            || !boundaries.contains(&range.start_addr)
            || !boundaries.contains(&end)
            || end > code.insns_size
        {
            return Err(reject("range is not original instruction boundaries"));
        }
        let tables: Vec<_> = code
            .handlers
            .iter()
            .filter(|h| h.relative_offset == u32::from(range.handler_off))
            .collect();
        let [table] = tables.as_slice() else {
            return Err(reject("absent/ambiguous encoded table"));
        };
        let covered = |id| {
            ssa.offset(id)
                .is_some_and(|o| range.start_addr <= o && o < end)
        };
        let mut protected = BTreeSet::new();
        for b in cfg.block_ids() {
            if covered(insn(b)?.0) {
                protected.insert(b);
            }
        }
        for b in cfg.block_ids() {
            let (id, i) = insn(b)?;
            if ssa.offset(id).is_some() {
                continue;
            }
            match i {
                ManagedInsn::Move { value, .. } => {
                    if let [edge] = cfg.block(b).map_err(|e| e.to_string())?.incoming() {
                        let e = cfg.edge(*edge).map_err(|e| e.to_string())?;
                        let previous = insn(e.source())?.0;
                        if e.kind() == &EdgeKind::Unconditional
                            && covered(previous)
                            && ir.value(*value).map_err(|e| e.to_string())?.def() == previous
                        {
                            protected.insert(b);
                        }
                    }
                }
                ManagedInsn::Phi { .. } => {
                    let mut anchor = b;
                    let mut seen = BTreeSet::new();
                    loop {
                        if !seen.insert(anchor) {
                            return Err(reject("cyclic synthetic Phi provenance"));
                        }
                        let (aid, ai) = insn(anchor)?;
                        if ssa.offset(aid).is_some() {
                            if covered(aid) {
                                protected.insert(b);
                            }
                            break;
                        }
                        if !matches!(ai, ManagedInsn::Phi { .. }) {
                            break;
                        }
                        anchor = next(anchor)?;
                    }
                }
                _ => {}
            }
        }
        // Only exact frontend predicate/zero definitions adjacent to their
        // original branch are source-associated. No arbitrary span inheritance.
        for b in cfg.block_ids() {
            if !protected.contains(&b) {
                continue;
            }
            let ManagedInsn::Branch {
                condition: Some(condition),
                ..
            } = insn(b)?.1
            else {
                continue;
            };
            let def = ir.value(*condition).map_err(|e| e.to_string())?.def();
            let candidates: Vec<_> = cfg
                .block_ids()
                .filter(|n| cfg.block(*n).is_ok_and(|node| *node.payload() == def))
                .collect();
            if let [comparison] = candidates.as_slice()
                && ssa.offset(def).is_none()
                && matches!(insn(*comparison)?.1,ManagedInsn::Compare{result,..}if result==condition)
                && next(*comparison)? == b
            {
                protected.insert(*comparison);
            }
        }
        for b in cfg.block_ids() {
            let (id, i) = insn(b)?;
            if ssa.offset(id).is_none()
                && let ManagedInsn::Const {
                    result,
                    value: managed_ir::ops::Constant::Zero,
                } = i
            {
                let anchor = next(b)?;
                if protected.contains(&anchor)
                    && matches!(insn(anchor)?.1,ManagedInsn::Compare{lhs,rhs,..}if lhs==result||rhs==result)
                {
                    protected.insert(b);
                }
            }
        }
        let dispatch: Vec<_> = table
            .handlers
            .iter()
            .map(|h| (h.type_idx.to_string(), h.address, Some(h.type_idx)))
            .chain(table.catch_all_addr.map(|a| (String::new(), a, None)))
            .collect();
        if dispatch.is_empty() {
            return Err(reject("empty encoded dispatch"));
        }
        let mut catches = vec![];
        let mut types = BTreeMap::new();
        let mut exception_edges = BTreeMap::new();
        for (key, address, ty) in &dispatch {
            if !boundaries.contains(address) || *address == code.insns_size {
                return Err(reject("handler address boundary"));
            }
            let mut targets = BTreeSet::new();
            let mut edges = vec![];
            for edge in cfg.edge_ids() {
                let e = cfg.edge(edge).map_err(|e| e.to_string())?;
                if e.kind() == &EdgeKind::Exception(key.clone()) && protected.contains(&e.source())
                {
                    targets.insert(e.target());
                    edges.push(edge);
                }
            }
            if targets.len() != 1 {
                return Err(reject("absent/ambiguous actual handler target"));
            }
            let entry = *targets.first().ok_or_else(|| reject("missing entry"))?;
            let mut anchor = entry;
            let mut seen = BTreeSet::new();
            let mut capture = None;
            let mut outside_normal = false;
            loop {
                if !seen.insert(anchor) {
                    return Err(reject("cyclic handler scaffold"));
                }
                let (id, i) = insn(anchor)?;
                for incoming in cfg.block(anchor).map_err(|e| e.to_string())?.incoming() {
                    let e = cfg.edge(*incoming).map_err(|e| e.to_string())?;
                    if !matches!(e.kind(), EdgeKind::Exception(_)) && !seen.contains(&e.source()) {
                        outside_normal = true;
                    }
                }
                if let Some(offset) = ssa.offset(id) {
                    if offset != *address {
                        return Err(reject("handler original offset disagreement"));
                    }
                    let original = decoded
                        .iter()
                        .find(|i| i.offset == offset)
                        .ok_or_else(|| reject("handler instruction absent"))?;
                    let move_exception = matches!(original.opcode,dex_decompiler::Opcode::Supported(info)if info.opcode==0x0d);
                    if let Some(caught) = capture {
                        if outside_normal {
                            return Err(reject(
                                "outside normal edge into handler capture scaffold",
                            ));
                        }

                        if !move_exception
                            || !matches!(i,ManagedInsn::Move{value,..}if *value==caught)
                        {
                            return Err(reject("capture does not match original move-exception"));
                        }
                    } else if move_exception {
                        return Err(reject("move-exception missing capture definition"));
                    }
                    break;
                }
                match i {
                    ManagedInsn::Phi { .. } => {}
                    ManagedInsn::Opaque {
                        family_tag: 0x8002,
                        reads,
                        writes,
                        effects,
                    } => {
                        let [caught] = writes.as_slice() else {
                            return Err(reject("capture must have one write"));
                        };
                        if !reads.is_empty()
                            || *effects != managed_ir::effects::EffectFlags::MAY_THROW
                            || capture.is_some()
                            || ir.value(*caught).map_err(|e| e.to_string())?.def() != id
                        {
                            return Err(reject("invalid capture definition"));
                        }
                        if ir.instructions().values().filter(|instruction|matches!(instruction,ManagedInsn::Opaque{family_tag:0x8002,writes,..}if writes.contains(caught))).count()!=1 {
                            return Err(reject("capture identity has duplicate synthetic definitions"));
                        }
                        capture = Some(*caught);
                    }
                    _ => return Err(reject("unproved offsetless handler instruction")),
                }
                anchor = next(anchor)?;
            }
            if let Some(previous) = metadata.captures.insert(entry, capture)
                && previous != capture
            {
                return Err(reject("conflicting shared capture identity"));
            }
            let name = match ty {
                Some(t) => resolver
                    .class(ClassRef::new(*t))
                    .ok_or_else(|| reject("missing catch class"))?,
                None => "java.lang.Throwable".into(),
            };
            if types.insert(key.clone(), JavaType::Class(name)).is_some() {
                return Err(reject("duplicate dispatch key"));
            }
            catches.push(Catch {
                key: key.clone(),
                entry,
            });
            exception_edges.insert(key.clone(), edges);
        }
        for edge in cfg.edge_ids() {
            let e = cfg.edge(edge).map_err(|e| e.to_string())?;
            if matches!(e.kind(), EdgeKind::Exception(_))
                && ssa.offset(insn(e.source())?.0).is_none()
                && !protected.contains(&e.source())
            {
                return Err(reject(
                    "offsetless throwing source has no attested scope provenance",
                ));
            }
        }
        metadata.scopes.push(RawScope {
            original: OriginalScope {
                ordinal,
                start: range.start_addr,
                end,
                handler_offset: range.handler_off,
                catches: dispatch.into_iter().map(|(k, a, _)| (k, a)).collect(),
                offsets: ir
                    .instructions()
                    .keys()
                    .filter_map(|id| ssa.offset(id).map(|o| (id, o)))
                    .collect(),
            },
            scope: RegionScope {
                protected: cfg.block_ids().filter(|b| protected.contains(b)).collect(),
                catches,
                continuation: None,
            },
            types,
            hierarchy: hierarchy.clone(),
            catch_all: table.catch_all_addr.is_some(),
            exception_edges,
        });
    }
    Ok(metadata)
}
