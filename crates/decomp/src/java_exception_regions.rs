//! Exact DEX exception metadata for structured and instruction-dispatch backends.
//! Parent wiring: register this module; call `from_dex` instead of the linear
//! bridge with the frozen-name JavaHierarchy; call `plan_region`/`emit_region`.
//! References are never promoted to class declarations by this adapter.
//!
//! The production declaration entry point is
//! `backend_java::exceptions::emit_method_region(ir, method, bindings, resolver,
//! &metadata)`. It uses the existing typed instruction renderer transactionally;
//! the parent does not need to implement RegionEmitter. Keep `catch_names()` for
//! dependency closure, and pass the same `Resolver::hierarchy` used by directed
//! assignment checks. Registering this file and selecting that entry point in
//! `java_integration` are parent-owned changes.
//!
//! `from_dex_dispatch` supplies the same original range/type/handler evidence
//! for `backend_java::exceptions::dispatch::emit_method`. Its per-instruction
//! schedule accepts cyclic flow and multiple source entries/exits; it does not
//! widen ranges to absorb outside control bridges for a structured source tree.
//!
//! Bounded real evidence (pool indices are zero-based): OKcashbag pool 2
//! commons-lang3 math/c `toDouble` #56276, `toInt` #56280, `toLong` #56282 all
//! have range [3,7), NumberFormatException type #5146, handler offset 8, and a
//! success-return offset 7. Their unchanged emitted declarations compile/run on
//! JDK17 with attested JDK parsing signatures and hierarchy. Pool 0 customtabs
//! IPostMessageService$Stub$Proxy #2170 has [8,29), catch-all offset 36, matching
//! recycle calls at 29/32 versus 37/40, and an identity-move rethrow at 43; its
//! finally region is proved, but Android dependency closure is not validated by
//! the JDK-only runtime tests. Pool 2 Kotlin z0 #44647 has [2,6), handler 14,
//! calls bracketing monitor-exit at 9/18, and an intervening constant after
//! monitor-enter. It rejects rather than relocating unproved calls across lock
//! release. SLF4J t #61324 passes exact scope construction but retains a
//! frontend Zero/reference Phi mismatch. In the legacy structured APIs,
//! multiple/nested ranges, cyclic flow,
//! overlapping handlers and distinct effectful normal exits remain explicit
//! unsupported shapes; missing classpath declaration facts remain indeterminate.
use backend_java::{
    JavaResolver, JavaType,
    exceptions::{
        RegionMetadata,
        scopes::{Catch, RegionScope},
    },
    type_compatibility::Hierarchy,
};
use decomp_core::{cfg::EdgeKind, ids::BlockId};
use dex_decompiler::ssa::SsaMethod;
use dex_format::code::CodeItem;
use managed_ir::{insn::ManagedInsn, refs::ClassRef};
use std::collections::{BTreeMap, HashSet};

/// Metadata in the same frozen alias domain as the parent resolver.
#[derive(Debug)]
pub struct Metadata {
    scope: RegionScope,
    types: BTreeMap<String, JavaType>,
    hierarchy: Hierarchy,
    catch_all: bool,
    original: backend_java::exceptions::OriginalScope,
}
impl RegionMetadata for Metadata {
    fn original_scope(&self) -> Option<&backend_java::exceptions::OriginalScope> {
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
impl Metadata {
    /// Names for dependency closure. Instruction coverage remains scope-local.
    pub fn catch_names(&self) -> impl Iterator<Item = &str> {
        self.types.values().filter_map(|t| match t {
            JavaType::Class(n) => Some(n.as_str()),
            _ => None,
        })
    }
}

/// Recover only exact source-covered nodes, adjacent success commits, and Phi
/// clusters anchored to a protected instruction. Branches are followed by CFG
/// identity, not arena/source ordering. No arbitrary synthetic span inheritance.
pub fn from_dex(
    ssa: &SsaMethod,
    code: &CodeItem,
    resolver: &impl JavaResolver,
    hierarchy: &Hierarchy,
) -> Result<Metadata, String> {
    let reject = |s: &str| format!("exception region: {s}");
    if code.tries.len() != 1 {
        return Err(reject(
            "multiple/nested source ranges require a separate proof",
        ));
    }
    let metadata = from_dex_range(ssa, code, 0, resolver, hierarchy, RangeMode::Structured)?;
    if let Err(error) =
        backend_java::exceptions::scopes::structure_region(ssa.ir().cfg(), &metadata.scope)
    {
        if !backend_java::exceptions::shared_void_terminal(ssa.ir(), &metadata.scope) {
            return Err(reject(&error.to_string()));
        }
        backend_java::exceptions::scopes::structure_disjoint(
            ssa.ir().cfg(),
            std::slice::from_ref(&metadata.scope),
        )
        .map_err(|error| reject(&error.to_string()))?;
    }
    Ok(metadata)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RangeMode {
    Structured,
    Dispatch,
}

fn from_dex_range(
    ssa: &SsaMethod,
    code: &CodeItem,
    index: usize,
    resolver: &impl JavaResolver,
    hierarchy: &Hierarchy,
    mode: RangeMode,
) -> Result<Metadata, String> {
    let reject = |s: &str| format!("exception region: {s}");
    let range = &code.tries[index];
    let end = range
        .start_addr
        .checked_add(u32::from(range.insn_count))
        .ok_or_else(|| reject("range overflow"))?;
    let table = code
        .handlers
        .iter()
        .find(|h| h.relative_offset == u32::from(range.handler_off))
        .ok_or_else(|| reject("missing encoded table"))?;
    let ir = ssa.ir();
    let cfg = ir.cfg();
    let insn = |b| -> Result<_, String> {
        let id = *cfg.block(b).map_err(|e| e.to_string())?.payload();
        Ok((id, ir.instruction(id).map_err(|e| e.to_string())?))
    };
    let next = |b| -> Result<BlockId, String> {
        let edges: Vec<_> = cfg
            .block(b)
            .map_err(|e| e.to_string())?
            .outgoing()
            .iter()
            .filter_map(|id| {
                let e = cfg.edge(*id).ok()?;
                (!matches!(e.kind(), EdgeKind::Exception(_))).then_some(e)
            })
            .collect();
        match edges.as_slice() {
            [e] if e.kind() == &EdgeKind::Unconditional => Ok(e.target()),
            _ => Err(reject(
                "synthetic anchor is not an exclusive linear successor",
            )),
        }
    };
    let covered = |id| {
        ssa.offset(id)
            .is_some_and(|o| range.start_addr <= o && o < end)
    };
    let mut protected = HashSet::new();
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
                let incoming = cfg.block(b).map_err(|e| e.to_string())?.incoming();
                if let [edge] = incoming {
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
                let mut seen = HashSet::new();
                loop {
                    if !seen.insert(anchor) {
                        return Err(reject("cyclic Phi provenance"));
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
    // The frontend materializes a branch comparison before its offset-bearing
    // Branch. Associate only the exact nonthrowing condition definition with a
    // sole normal successor, rather than covering arbitrary offsetless nodes.
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
            .filter(|n| cfg.block(*n).is_ok_and(|n| *n.payload() == def))
            .collect();
        if let [comparison] = candidates.as_slice()
            && ssa.offset(def).is_none()
            && matches!(insn(*comparison)?.1, ManagedInsn::Compare { result, .. } if result == condition)
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
                && matches!(insn(anchor)?.1, ManagedInsn::Compare { lhs, rhs, .. } if lhs == result || rhs == result)
            {
                protected.insert(b);
            }
        }
    }
    // DEX try ranges can physically cover their own nonthrowing handler
    // capture/goto scaffold. Those nodes belong to handler dispatch, not to a
    // second normal entry into the protected body. Exclude only the exact
    // capture identity and its exclusive control-only chain; every effectful
    // handler instruction and its encoded exception edges remain unchanged.
    for initial in cfg.block_ids() {
        let ManagedInsn::Opaque {
            family_tag: 0x8002,
            writes,
            ..
        } = insn(initial)?.1
        else {
            continue;
        };
        let [caught] = writes.as_slice() else {
            continue;
        };
        let Ok(capture) = next(initial) else {
            continue;
        };
        if !matches!(insn(capture)?.1, ManagedInsn::Move { value, .. } if value == caught) {
            continue;
        }
        let mut previous = initial;
        let mut current = capture;
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(current) {
                break;
            }
            let block = cfg.block(current).map_err(|e| e.to_string())?;
            let [incoming] = block.incoming() else {
                break;
            };
            if !cfg.edge(*incoming).is_ok_and(|edge| {
                edge.kind() == &EdgeKind::Unconditional && edge.source() == previous
            }) || block.outgoing().iter().any(|id| {
                cfg.edge(*id)
                    .is_ok_and(|edge| matches!(edge.kind(), EdgeKind::Exception(_)))
            }) {
                break;
            }
            if current != capture
                && !matches!(
                    insn(current)?.1,
                    ManagedInsn::Branch {
                        condition: None,
                        else_edge: None,
                        ..
                    }
                )
            {
                break;
            }
            protected.remove(&current);
            let Ok(successor) = next(current) else {
                break;
            };
            previous = current;
            current = successor;
        }
    }
    // Structured source only: control-only bridges may be placed inside the try
    // without adding catchable effects. The dispatcher has no region-boundary
    // need to absorb outside source offsets, so it retains their outside state.
    // Every predecessor must already belong to this scope:
    // handler/shared outside entry paths must never be swallowed. A Return's
    // operand is still checked for actual materialization by the typed backend.
    if mode == RangeMode::Structured {
        loop {
            let mut added = false;
            for b in cfg.block_ids() {
                if protected.contains(&b) {
                    continue;
                }
                let (_, instruction) = insn(b)?;
                // An outside condition selecting two already-protected arms has
                // no catchable effect. Keep its exact synthetic Compare adjacent
                // to the original Branch; operand availability remains a backend
                // obligation. Never absorb an effectful condition definition.
                if let ManagedInsn::Branch {
                    condition: Some(condition),
                    ..
                } = instruction
                {
                    let block = cfg.block(b).map_err(|e| e.to_string())?;
                    if block.outgoing().len() == 2
                        && block.outgoing().iter().all(|id| {
                            cfg.edge(*id).is_ok_and(|edge| {
                                matches!(edge.kind(), EdgeKind::BranchTrue | EdgeKind::BranchFalse)
                                    && protected.contains(&edge.target())
                            })
                        })
                    {
                        let definition = ir.value(*condition).map_err(|e| e.to_string())?.def();
                        let comparisons: Vec<_> = cfg
                            .block_ids()
                            .filter(|id| {
                                cfg.block(*id)
                                    .is_ok_and(|block| *block.payload() == definition)
                            })
                            .collect();
                        if let [comparison] = comparisons.as_slice()
                            && ssa.offset(definition).is_none()
                            && matches!(insn(*comparison)?.1, ManagedInsn::Compare { result, .. } if result == condition)
                            && next(*comparison).is_ok_and(|target| target == b)
                            && cfg
                                .block(*comparison)
                                .map_err(|e| e.to_string())?
                                .incoming()
                                .iter()
                                .all(|id| {
                                    cfg.edge(*id).is_ok_and(|edge| {
                                        !matches!(edge.kind(), EdgeKind::Exception(_))
                                    })
                                })
                        {
                            protected.insert(*comparison);
                            protected.insert(b);
                            added = true;
                            continue;
                        }
                    }
                }
                if !matches!(
                    instruction,
                    ManagedInsn::Return { .. }
                        | ManagedInsn::Branch {
                            condition: None,
                            else_edge: None,
                            ..
                        }
                ) {
                    continue;
                }
                let block = cfg.block(b).map_err(|e| e.to_string())?;
                if block.incoming().is_empty()
                    || !block.incoming().iter().all(|id| {
                        cfg.edge(*id).is_ok_and(|edge| {
                            !matches!(edge.kind(), EdgeKind::Exception(_))
                                && protected.contains(&edge.source())
                        })
                    })
                    || block.outgoing().iter().any(|id| {
                        cfg.edge(*id)
                            .is_ok_and(|edge| matches!(edge.kind(), EdgeKind::Exception(_)))
                    })
                {
                    continue;
                }
                if matches!(instruction, ManagedInsn::Return { .. }) && !block.outgoing().is_empty()
                {
                    continue;
                }
                protected.insert(b);
                added = true;
            }
            if !added {
                break;
            }
        }
    }
    let mut exits = HashSet::new();
    for &b in &protected {
        for id in cfg.block(b).map_err(|e| e.to_string())?.outgoing() {
            let e = cfg.edge(*id).map_err(|e| e.to_string())?;
            if !matches!(e.kind(), EdgeKind::Exception(_)) && !protected.contains(&e.target()) {
                exits.insert(e.target());
            }
        }
    }
    if exits.len() > 1 && mode == RangeMode::Structured {
        return Err(reject("multiple normal exits from exact source range"));
    }
    let continuation = if exits.len() == 1 {
        exits.iter().next().copied()
    } else {
        None
    };
    let mut catches = Vec::new();
    let mut types = BTreeMap::new();
    let dispatch = table
        .handlers
        .iter()
        .map(|h| (h.type_idx.to_string(), h.address, Some(h.type_idx)))
        .chain(table.catch_all_addr.map(|a| (String::new(), a, None)));
    for (key, address, ty) in dispatch {
        let targets: HashSet<_> = cfg
            .edge_ids()
            .filter_map(|id| {
                let e = cfg.edge(id).ok()?;
                (protected.contains(&e.source()) && e.kind() == &EdgeKind::Exception(key.clone()))
                    .then_some(e.target())
            })
            .collect();
        if targets.len() != 1 {
            return Err(reject("ambiguous or absent handler target"));
        }
        let entry = *targets
            .iter()
            .next()
            .ok_or_else(|| reject("missing handler entry"))?;
        let mut anchor = entry;
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(anchor) {
                return Err(reject("cyclic handler provenance"));
            }
            let (id, i) = insn(anchor)?;
            if let Some(offset) = ssa.offset(id) {
                if offset != address {
                    return Err(reject("encoded handler address disagrees with SSA"));
                }
                break;
            }
            if !matches!(
                i,
                ManagedInsn::Phi { .. }
                    | ManagedInsn::Opaque {
                        family_tag: 0x8002,
                        ..
                    }
            ) {
                return Err(reject("unproven synthetic handler provenance"));
            }
            anchor = next(anchor)?;
        }
        let name = match ty {
            Some(t) => resolver
                .class(ClassRef::new(t))
                .ok_or_else(|| reject("caught type metadata missing"))?,
            None => "java.lang.Throwable".into(),
        };
        if types.insert(key.clone(), JavaType::Class(name)).is_some() {
            return Err(reject("duplicate dispatch key"));
        }
        catches.push(Catch { key, entry });
    }
    let scope = RegionScope {
        protected: cfg.block_ids().filter(|b| protected.contains(b)).collect(),
        catches,
        continuation,
    };
    let original = backend_java::exceptions::OriginalScope {
        ordinal: index,
        start: range.start_addr,
        end,
        handler_offset: range.handler_off,
        catches: table
            .handlers
            .iter()
            .map(|h| (h.type_idx.to_string(), h.address))
            .chain(table.catch_all_addr.map(|a| (String::new(), a)))
            .collect(),
        offsets: ir
            .instructions()
            .keys()
            .filter_map(|id| ssa.offset(id).map(|offset| (id, offset)))
            .collect(),
    };
    Ok(Metadata {
        original,
        scope,
        types,
        hierarchy: hierarchy.clone(),
        catch_all: table.catch_all_addr.is_some(),
    })
}

/// Exact source scopes for an acyclic disjoint exception-aware body.
#[derive(Debug)]
pub struct DisjointMetadata {
    /// Ordered original source ranges with independent catch metadata.
    pub scopes: Vec<Metadata>,
}
impl DisjointMetadata {
    /// Genuine caught types required by publication closure.
    pub fn catch_names(&self) -> impl Iterator<Item = &str> {
        self.scopes.iter().flat_map(Metadata::catch_names)
    }
}
/// Preserve every encoded range independently; nested/shared/cyclic bodies reject.
pub fn from_dex_disjoint(
    ssa: &SsaMethod,
    code: &CodeItem,
    resolver: &impl JavaResolver,
    hierarchy: &Hierarchy,
) -> Result<DisjointMetadata, String> {
    if code.tries.is_empty() || code.tries.len() > 64 {
        return Err("exception region: disjoint scope budget".into());
    }
    let mut previous_end = None;
    for range in &code.tries {
        let end = range
            .start_addr
            .checked_add(u32::from(range.insn_count))
            .ok_or("exception region: range overflow")?;
        if previous_end.is_some_and(|p| p > range.start_addr) {
            return Err("exception region: overlapping source ranges".into());
        }
        previous_end = Some(end);
    }
    let scopes = (0..code.tries.len())
        .map(|i| from_dex_range(ssa, code, i, resolver, hierarchy, RangeMode::Structured))
        .collect::<Result<Vec<_>, _>>()?;
    let original_scopes = scopes.iter().map(|m| m.scope.clone()).collect::<Vec<_>>();
    backend_java::exceptions::scopes::structure_disjoint(ssa.ir().cfg(), &original_scopes)
        .or_else(|original| {
            backend_java::exceptions::scopes::structure_disjoint_shared_handlers(
                ssa.ir().cfg(),
                &original_scopes,
            )
            .map_err(|_| original)
        })
        .map_err(|e| format!("exception region: {e}"))?;
    Ok(DisjointMetadata { scopes })
}

/// Exact original source tables for the per-instruction dispatch backend.
/// Original ranges must remain ordered, nonoverlapping and within the encoded
/// instruction stream, with at most 64 tables. Handler addresses, offsetless
/// Phi/capture provenance, and encoded catch priority retain the same proofs
/// as the structured adapter. Cycles, multiple entries/exits and shared handler
/// bodies impose no source-grammar restrictions.
pub fn from_dex_dispatch(
    ssa: &SsaMethod,
    code: &CodeItem,
    resolver: &impl JavaResolver,
    hierarchy: &Hierarchy,
) -> Result<DisjointMetadata, String> {
    if code.tries.is_empty() || code.tries.len() > 64 {
        return Err("exception region: dispatch scope budget".into());
    }
    let mut previous_end = None;
    for range in &code.tries {
        let end = range
            .start_addr
            .checked_add(u32::from(range.insn_count))
            .ok_or("exception region: range overflow")?;
        if range.insn_count == 0 || end > code.insns_size {
            return Err("exception region: source range outside encoded instructions".into());
        }
        if previous_end.is_some_and(|p| p > range.start_addr) {
            return Err("exception region: overlapping source ranges".into());
        }
        previous_end = Some(end);
    }
    let scopes = (0..code.tries.len())
        .map(|index| from_dex_range(ssa, code, index, resolver, hierarchy, RangeMode::Dispatch))
        .collect::<Result<Vec<_>, _>>()?;
    let tables = scopes.iter().map(|md| md.scope.clone()).collect::<Vec<_>>();
    decomp_structure::exception_dispatch::plan_scopes(ssa.ir().cfg(), &tables)
        .map_err(|e| format!("exception region: {e}"))?;
    Ok(DisjointMetadata { scopes })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use backend_java::{JavaField, JavaMethod, type_compatibility::Declaration};
    use managed_ir::{
        refs::{FieldRef, MethodRef, StringRef},
        value::ValueId,
    };

    struct Names<'a>(&'a dex_format::DexFile);
    impl JavaResolver for Names<'_> {
        fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
            jdk_hierarchy().assignable(a, b)
                == backend_java::type_compatibility::Compatibility::Assignable
        }
        fn class(&self, r: ClassRef) -> Option<String> {
            let t = self.0.type_id(r.index()).ok()?;
            let d = self.0.string(t.descriptor_idx).ok()?;
            Some(
                d.trim_start_matches('L')
                    .trim_end_matches(';')
                    .replace('/', "."),
            )
        }
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            if let Some(m) = (crate::dex_java::DexJavaResolver { dex: self.0 }).method(r) {
                return Some(m);
            }
            let m = crate::java_descriptors::method_from_pool(self.0, r.index(), true).ok()?;
            // Independently established JDK17 declarations for these exact
            // signatures; static dispatch is not inferred from the IR callsite.
            if matches!(
                (m.owner.as_str(), m.name.as_str()),
                ("java.lang.Integer", "parseInt")
                    | ("java.lang.Long", "parseLong")
                    | ("java.lang.Double", "parseDouble")
            ) && m.parameters == [JavaType::Class("java.lang.String".into())]
            {
                Some(m)
            } else {
                None
            }
        }
        fn field(&self, _: FieldRef) -> Option<JavaField> {
            None
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
    }
    fn jdk_hierarchy() -> Hierarchy {
        let mut h = Hierarchy::default();
        // These are explicit JDK17 declaration facts for this bounded test,
        // never inferred from APK type references or installed as global defaults.
        for (name, parent) in [
            ("java.lang.Throwable", "java.lang.Object"),
            ("java.lang.Exception", "java.lang.Throwable"),
            ("java.lang.RuntimeException", "java.lang.Exception"),
            (
                "java.lang.ArithmeticException",
                "java.lang.RuntimeException",
            ),
            (
                "java.lang.NumberFormatException",
                "java.lang.IllegalArgumentException",
            ),
            (
                "java.lang.IllegalArgumentException",
                "java.lang.RuntimeException",
            ),
            ("java.lang.SecurityException", "java.lang.RuntimeException"),
        ] {
            h.insert(Declaration {
                name: name.into(),
                superclass: Some(parent.into()),
                interfaces: vec![],
                is_interface: false,
            })
            .unwrap();
        }
        h
    }

    #[test]
    fn exact_raw_branching_scope_compiles_runs_and_rejects_bad_handler_address() {
        use dex_format::{
            RawSpan,
            code::{EncodedCatchHandler, TryItem, TypeAddrPair},
        };
        struct Types;
        impl dex_decompiler::TypeContext for Types {
            fn enclosing_return(&self) -> Option<managed_ir::types::ManagedType> {
                Some(managed_ir::types::ManagedType::Int)
            }
        }
        struct Resolver;
        impl JavaResolver for Resolver {
            fn class(&self, _: ClassRef) -> Option<String> {
                Some("java.lang.ArithmeticException".into())
            }
            fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
                jdk_hierarchy().assignable(a, b)
                    == backend_java::type_compatibility::Compatibility::Assignable
            }
            fn method(&self, _: MethodRef) -> Option<JavaMethod> {
                None
            }
            fn field(&self, _: FieldRef) -> Option<JavaField> {
                None
            }
            fn string(&self, _: StringRef) -> Option<Vec<u16>> {
                None
            }
        }
        let units = [
            0x0013u16, 10, 0x1112, 0x1232, 5, 0x0093, 0x0200, 0x0328, 0xf012, 0x0128, 0x000f,
            0x010d, 0xe012, 0xfd28,
        ];
        let span = RawSpan { offset: 0, size: 0 };
        let code = CodeItem {
            registers_size: 3,
            ins_size: 1,
            outs_size: 0,
            debug_info_off: 0,
            insns_size: units.len() as u32,
            insns: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
            tries: vec![TryItem {
                start_addr: 3,
                insn_count: 7,
                handler_off: 1,
                span,
            }],
            handlers: vec![EncodedCatchHandler {
                relative_offset: 1,
                handlers: vec![TypeAddrPair {
                    type_idx: 1,
                    address: 11,
                }],
                catch_all_addr: None,
                span,
            }],
            span,
        };
        let ssa = dex_decompiler::DexPasses::run_ssa_with(&code, &Types).unwrap();
        let metadata = from_dex(&ssa, &code, &Resolver, &jdk_hierarchy()).unwrap();
        assert_eq!(
            metadata.catch_names().collect::<Vec<_>>(),
            ["java.lang.ArithmeticException"]
        );
        let method = JavaMethod {
            owner: "RawBranch".into(),
            name: "divide".into(),
            is_static: true,
            return_type: JavaType::Int,
            parameters: vec![JavaType::Int],
            throws: vec![],
        };
        let bindings = crate::dex_java::parameter_bindings(ssa.ir(), &method, 3, 1).unwrap();
        let result = backend_java::exceptions::emit_method_region(
            ssa.ir(),
            &method,
            &backend_java::DeclarationBindings {
                receiver: None,
                parameters: bindings.parameters.into_iter().map(Some).collect(),
                names: vec![],
            },
            &Resolver,
            &metadata,
        );
        let backend_java::SourceResult::Complete { source, .. } = result else {
            panic!("{result:?}");
        };
        let source = format!(
            "public class RawBranch {{ {source} public static void main(String[] a) {{if(divide(1)!=-1 || divide(0)!=-2 || divide(2)!=5 || divide(-2)!=-5) throw new AssertionError();System.out.println(\"PASS raw branching DEX scope\");}} }}"
        );
        let root =
            std::env::temp_dir().join(format!("decomphify-raw-exception-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("RawBranch.java"), &source).unwrap();
        let compiled = std::process::Command::new("javac")
            .arg("RawBranch.java")
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}\n{source}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let run = std::process::Command::new("java")
            .args(["-cp", ".", "RawBranch"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        let mut bad = code.clone();
        bad.handlers[0].handlers[0].address = 12;
        assert!(
            from_dex(&ssa, &bad, &Resolver, &jdk_hierarchy())
                .unwrap_err()
                .contains("encoded handler address")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Narrow method inventory; opt-in because the APK is a local corpus asset.
    #[test]
    #[ignore = "requires local OKcashbag APK; no corpus export"]
    fn selected_real_method_inventory() {
        let apk = std::env::var("EXCEPTION_APK").unwrap();
        let files = dex_format::apk::parse_apk_path(apk).unwrap();
        let mut found = 0;
        let mut real_sources = Vec::new();
        for (pool, dex) in files.iter().enumerate() {
            let r = Names(dex);
            for class in &dex.classes {
                let owner = r.class(ClassRef::new(class.class_idx)).unwrap();
                if !matches!(
                    owner.as_str(),
                    "org.apache.commons.lang3.math.c"
                        | "org.slf4j.helpers.t"
                        | "kotlinx.coroutines.internal.z0"
                        | "android.support.customtabs.IPostMessageService$Stub$Proxy"
                ) {
                    continue;
                }
                let Some(data) = &class.class_data else {
                    continue;
                };
                for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                    let m = dex.method_id(method.method_idx).unwrap();
                    let name = dex.string(m.name_idx).unwrap();
                    if !matches!(
                        name,
                        "toInt"
                            | "toLong"
                            | "toDouble"
                            | "safeGetSystemProperty"
                            | "synchronizedImpl"
                            | "onMessageChannelReady"
                    ) {
                        continue;
                    }
                    let Some(code) = &method.code else {
                        continue;
                    };
                    if code.tries.is_empty() {
                        continue;
                    }
                    found += 1;
                    let ssa = dex_decompiler::DexPasses::run_ssa_with(
                        code,
                        &dex_decompiler::DexTypes::new(dex, Some(method.method_idx)),
                    )
                    .unwrap();
                    println!(
                        "REAL pool={pool} owner={owner} method={name} index={} tries={:?} handlers={:?}",
                        method.method_idx, code.tries, code.handlers
                    );
                    for b in ssa.ir().cfg().block_ids() {
                        let id = *ssa.ir().cfg().block(b).unwrap().payload();
                        println!(
                            "  {b:?} offset={:?} {:?} out={:?}",
                            ssa.offset(id),
                            ssa.ir().instruction(id).unwrap(),
                            ssa.ir()
                                .cfg()
                                .block(b)
                                .unwrap()
                                .outgoing()
                                .iter()
                                .map(|e| {
                                    let e = ssa.ir().cfg().edge(*e).unwrap();
                                    (e.kind(), e.target())
                                })
                                .collect::<Vec<_>>()
                        );
                    }
                    let metadata = from_dex(&ssa, code, &r, &jdk_hierarchy());
                    println!("  adapter={metadata:?}");
                    if let Ok(metadata) = metadata {
                        let bindings: Vec<ValueId> = ssa
                            .ir()
                            .instructions()
                            .values()
                            .filter_map(|i| match i {
                                ManagedInsn::Opaque {
                                    family_tag: 0x8001 | 0x8003,
                                    writes,
                                    ..
                                } => Some(writes.clone()),
                                _ => None,
                            })
                            .flatten()
                            .collect();
                        println!(
                            "  plan={:?}",
                            backend_java::exceptions::plan_region(
                                ssa.ir(),
                                &metadata,
                                &bindings,
                                &r
                            )
                        );
                        if matches!(name, "toInt" | "toLong" | "toDouble") {
                            let method = r.method(MethodRef::new(method.method_idx)).unwrap();
                            let bound = crate::dex_java::parameter_bindings(
                                ssa.ir(),
                                &method,
                                code.registers_size,
                                code.ins_size,
                            )
                            .unwrap();
                            let result = backend_java::exceptions::emit_method_region(
                                ssa.ir(),
                                &method,
                                &backend_java::DeclarationBindings {
                                    receiver: bound.receiver,
                                    parameters: bound.parameters.into_iter().map(Some).collect(),
                                    names: vec![],
                                },
                                &r,
                                &metadata,
                            );
                            println!("  emitted={result:?}");
                            if let backend_java::SourceResult::Complete { source, .. } = result {
                                real_sources.push(source);
                            }
                        }
                    }
                }
            }
        }
        assert!(found >= 3, "selected real methods absent");
        assert_eq!(
            real_sources.len(),
            3,
            "three actual APK declarations must emit completely"
        );
        let source = format!(
            r#"public class RealExceptionSubset {{ {}
          public static void main(String[] args) {{
            if(toInt("123",7)!=123 || toInt(null,7)!=7 || toInt("bad",7)!=7 || toInt("2147483648",7)!=7) throw new AssertionError("int");
            if(toLong("9223372036854775807",7)!=Long.MAX_VALUE || toLong(null,7)!=7 || toLong("bad",7)!=7 || toLong("9223372036854775808",7)!=7) throw new AssertionError("long");
            if(toDouble("1.25",7)!=1.25 || toDouble(null,7)!=7 || toDouble("bad",7)!=7 || !Double.isNaN(toDouble("NaN",7))) throw new AssertionError("double");
            System.out.println("PASS three actual APK exception methods");
          }} }}"#,
            real_sources.join("\n")
        );
        let root = std::env::temp_dir().join(format!(
            "decomphify-real-exception-subset-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("RealExceptionSubset.java"), &source).unwrap();
        let compiled = std::process::Command::new("javac")
            .arg("RealExceptionSubset.java")
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}\n{source}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let run = std::process::Command::new("java")
            .args(["-cp", ".", "RealExceptionSubset"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        println!("{}", String::from_utf8_lossy(&run.stdout));
        println!("real source subset preserved at {}", root.display());
    }
    include!("java_disjoint_exception_tests.rs");
    include!("java_exception_exit_tests.rs");
    include!("java_exception_dispatch_tests.rs");
}
