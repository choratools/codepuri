//! Original-range allocation/initializer proofs local to exception rendering.
use super::{RegionMetadata, reads};
use crate::{JavaField, JavaMethod, JavaResolver, JavaType, SuperInvocation};
use decomp_core::{
    cfg::EdgeKind,
    ids::{BlockId, EdgeId},
};
use managed_ir::{
    insn::{InvokeKind, ManagedInsn},
    method::{MethodIr, SsaForm},
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
    value::{InsnId, ValueId},
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

/// Original DEX range identity, distinct from augmented lexical protected blocks.
#[derive(Clone, Debug)]
pub struct OriginalScope {
    /// Original try table ordinal.
    pub ordinal: usize,
    /// Inclusive code-unit start.
    pub start: u32,
    /// Exclusive code-unit end.
    pub end: u32,
    /// Original encoded handler offset.
    pub handler_offset: u16,
    /// Original encoded catch priority and handler addresses.
    pub catches: Vec<(String, u32)>,
    /// Original instruction code-unit provenance; synthetic commits have no entry.
    pub offsets: HashMap<InsnId, u32>,
}
impl OriginalScope {
    fn contains(&self, id: InsnId) -> bool {
        self.offsets
            .get(&id)
            .is_some_and(|o| self.start <= *o && *o < self.end)
    }
}

/// Read-only local original-range allocation-chain evidence. Each returned
/// chain passes either the unchanged single-range proof or the narrow pure-gap
/// cross-scope transfer-equivalence proof, plus alias and handler-state checks used
/// by exception rendering. This does not certify SSA availability, argument
/// source expressions, a complete method body, or dependency closure.
pub fn original_allocation_chains(
    ir: &MethodIr<SsaForm>,
    metadata: &[&dyn RegionMetadata],
    resolver: &impl JavaResolver,
) -> HashSet<Vec<InsnId>> {
    let proof = prove(ir, metadata, resolver);
    proof
        .chains
        .into_iter()
        .chain(proof.pure.into_keys())
        .collect()
}

#[path = "exception_cross_scope_allocation.rs"]
mod cross_scope;

#[path = "exception_ordered_string_allocation.rs"]
mod ordered_string;

#[derive(Default)]
pub(super) struct Proof {
    chains: HashSet<Vec<InsnId>>,
    pure: HashMap<Vec<InsnId>, cross_scope::Witness>,
    sequences: HashMap<Vec<InsnId>, ordered_string::Witness>,
}
impl std::ops::Deref for Proof {
    type Target = HashSet<Vec<InsnId>>;
    fn deref(&self) -> &Self::Target {
        &self.chains
    }
}
pub(super) fn prove(
    ir: &MethodIr<SsaForm>,
    metadata: &[&dyn RegionMetadata],
    resolver: &impl JavaResolver,
) -> Proof {
    let chains = strict(ir, metadata, resolver);
    let pure = cross_scope::prove(ir, metadata, resolver);
    let sequences = ordered_string::prove(ir, metadata, resolver);
    Proof {
        chains,
        pure,
        sequences,
    }
}

fn strict(
    ir: &MethodIr<SsaForm>,
    metadata: &[&dyn RegionMetadata],
    resolver: &impl JavaResolver,
) -> HashSet<Vec<InsnId>> {
    let mut proved = HashSet::new();
    for block in ir.cfg().block_ids() {
        let Ok(node) = ir.cfg().block(block) else {
            continue;
        };
        let start = *node.payload();
        let Ok(ManagedInsn::NewInstance { result, class }) = ir.instruction(start) else {
            continue;
        };
        for md in metadata {
            let Some(original) = md.original_scope() else {
                continue;
            };
            if !original.contains(start)
                || original.catches.len() != md.region_scope().catches.len()
                || original
                    .catches
                    .iter()
                    .zip(&md.region_scope().catches)
                    .any(|((key, _), catch)| key != &catch.key)
            {
                continue;
            }
            if !md.region_scope().protected.contains(&block)
                || original.catches.iter().zip(&md.region_scope().catches).any(
                    |((_, address), catch)| !handler_address(ir, catch.entry, original, *address),
                )
            {
                continue;
            }
            let mut chain = vec![start];
            let mut aliases = HashSet::from([*result]);
            let mut delayed = aliases.clone();
            let mut throwing = vec![block];
            let mut current = block;
            let mut seen = HashSet::new();
            let mut finished = false;
            while seen.insert(current) && seen.len() <= 64 {
                let Ok(n) = ir.cfg().block(current) else {
                    break;
                };
                let normals: Vec<_> = n
                    .outgoing()
                    .iter()
                    .filter_map(|e| ir.cfg().edge(*e).ok())
                    .filter(|e| !matches!(e.kind(), EdgeKind::Exception(_)))
                    .collect();
                let [edge] = normals.as_slice() else { break };
                if edge.kind() != &EdgeKind::Unconditional {
                    break;
                }
                current = edge.target();
                let Ok(next) = ir.cfg().block(current) else {
                    break;
                };
                if !md.region_scope().protected.contains(&current) {
                    break;
                }
                let id = *next.payload();
                match ir.instruction(id) {
                    Ok(ManagedInsn::Move { result, value }) if aliases.contains(value) => {
                        if original.offsets.contains_key(&id) && !original.contains(id) {
                            break;
                        }
                        if next.outgoing().iter().any(|e| {
                            ir.cfg()
                                .edge(*e)
                                .is_ok_and(|e| matches!(e.kind(), EdgeKind::Exception(_)))
                        }) {
                            break;
                        }
                        aliases.insert(*result);
                        delayed.insert(*result);
                        chain.push(id);
                    }
                    Ok(ManagedInsn::Invoke {
                        kind: InvokeKind::Special,
                        result: None,
                        method,
                        args,
                    }) if args.first().is_some_and(|v| aliases.contains(v)) => {
                        let Some(target) = resolver.method(*method) else {
                            break;
                        };
                        if target.name != "<init>"
                            || resolver.class(*class).as_deref() != Some(target.owner.as_str())
                            || !original.contains(id)
                            || args.iter().skip(1).any(|v| aliases.contains(v))
                        {
                            break;
                        }
                        chain.push(id);
                        finished = true;
                        break;
                    }
                    Ok(instruction) => {
                        // Scope proof is separate from source-expression proof.
                        // Original producers and synthetic copies of already
                        // proved results may stay in this exact range. The object
                        // planner later validates their complete def/use trace.
                        let output = match instruction {
                            // An original in-range move may copy an already
                            // available input; it need not copy a producer from
                            // this allocation chain. Offsetless synthetic commits
                            // still require the delayed source's provenance.
                            ManagedInsn::Move { result, value }
                                if original.contains(id) || delayed.contains(value) =>
                            {
                                Some(*result)
                            }
                            ManagedInsn::Const { result, .. }
                            | ManagedInsn::FieldGet { result, .. }
                            | ManagedInsn::ArrayGet { result, .. }
                            | ManagedInsn::ArrayLength { result, .. }
                            | ManagedInsn::Arithmetic { result, .. }
                            | ManagedInsn::Cast { result, .. }
                            | ManagedInsn::NewArray { result, .. }
                            | ManagedInsn::NewInstance { result, .. } => Some(*result),
                            ManagedInsn::Invoke {
                                kind:
                                    InvokeKind::Static | InvokeKind::Virtual | InvokeKind::Interface,
                                result: Some(result),
                                ..
                            } => Some(*result),
                            ManagedInsn::Invoke {
                                kind:
                                    InvokeKind::Static | InvokeKind::Virtual | InvokeKind::Interface,
                                result: None,
                                ..
                            }
                            | ManagedInsn::FieldPut { .. }
                            | ManagedInsn::ArrayPut { .. }
                            | ManagedInsn::FillArray { .. } => None,
                            ManagedInsn::NewFilledArray {
                                result: Some(result),
                                ..
                            } => Some(*result),
                            ManagedInsn::Invoke {
                                kind: InvokeKind::Special,
                                result: None,
                                args,
                                ..
                            } if args.first().is_some_and(|value| delayed.contains(value)) => None,
                            _ => break,
                        };
                        let synthetic_copy = matches!(instruction, ManagedInsn::Move { value, .. } if delayed.contains(value));
                        if !(original.contains(id)
                            || synthetic_copy && !original.offsets.contains_key(&id))
                        {
                            break;
                        }
                        if reads(instruction)
                            .iter()
                            .any(|value| aliases.contains(value))
                        {
                            break;
                        }
                        if next.outgoing().iter().any(|edge| {
                            ir.cfg()
                                .edge(*edge)
                                .is_ok_and(|edge| matches!(edge.kind(), EdgeKind::Exception(_)))
                        }) {
                            if synthetic_copy {
                                break;
                            }
                            throwing.push(current);
                        }
                        if let Some(output) = output {
                            delayed.insert(output);
                        }
                        chain.push(id);
                    }
                    Err(_) => break,
                }
            }
            if !finished {
                continue;
            }
            let endpoint = *chain.last().unwrap_or(&start);
            let Some(endblock) = ir
                .cfg()
                .block_ids()
                .find(|b| ir.cfg().block(*b).is_ok_and(|n| *n.payload() == endpoint))
            else {
                continue;
            };
            throwing.push(endblock);
            if throwing
                .iter()
                .any(|operation| !same_dispatch(ir, block, *operation, *md, &delayed))
            {
                continue;
            }
            let handler_reads = throwing.iter().any(|operation| {
                ir.cfg().block(*operation).map_or(true, |node| {
                    node.outgoing().iter().any(|edge| {
                        ir.cfg().edge(*edge).map_or(true, |e| {
                            matches!(e.kind(), EdgeKind::Exception(_))
                                && handler_observes(ir, e.target(), *edge, &delayed, resolver)
                        })
                    })
                })
            });
            if handler_reads {
                continue;
            }
            proved.insert(chain);
        }
    }
    proved
}
fn handler_address(
    ir: &MethodIr<SsaForm>,
    entry: BlockId,
    original: &OriginalScope,
    address: u32,
) -> bool {
    let mut current = entry;
    let mut seen = HashSet::new();
    while seen.insert(current) {
        let Ok(block) = ir.cfg().block(current) else {
            return false;
        };
        let id = *block.payload();
        if let Some(offset) = original.offsets.get(&id) {
            return *offset == address;
        }
        if !matches!(
            ir.instruction(id),
            Ok(ManagedInsn::Phi { .. }
                | ManagedInsn::Opaque {
                    family_tag: 0x8002,
                    ..
                })
        ) {
            return false;
        }
        let [edge] = block.outgoing() else {
            return false;
        };
        let Ok(edge) = ir.cfg().edge(*edge) else {
            return false;
        };
        if edge.kind() != &EdgeKind::Unconditional {
            return false;
        }
        current = edge.target();
    }
    false
}
fn same_dispatch(
    ir: &MethodIr<SsaForm>,
    a: BlockId,
    b: BlockId,
    md: &dyn RegionMetadata,
    aliases: &HashSet<ValueId>,
) -> bool {
    let edges = |block| -> Option<Vec<_>> {
        Some(
            ir.cfg()
                .block(block)
                .ok()?
                .outgoing()
                .iter()
                .filter_map(|id| {
                    let e = ir.cfg().edge(*id).ok()?;
                    if let EdgeKind::Exception(key) = e.kind() {
                        Some((*id, key.clone(), e.target()))
                    } else {
                        None
                    }
                })
                .collect(),
        )
    };
    let (Some(a), Some(b)) = (edges(a), edges(b)) else {
        return false;
    };
    let expected = &md.region_scope().catches;
    if a.len() != expected.len() || b.len() != expected.len() || a.is_empty() {
        return false;
    }
    for catch in expected {
        let left: Vec<_> = a
            .iter()
            .filter(|(_, k, t)| k == &catch.key && t == &catch.entry)
            .collect();
        let right: Vec<_> = b
            .iter()
            .filter(|(_, k, t)| k == &catch.key && t == &catch.entry)
            .collect();
        let ([left], [right]) = (left.as_slice(), right.as_slice()) else {
            return false;
        };
        // Full-arena phi lookup handles consecutive handler clusters without
        // relying on block adjacency. Identical original selected sources are
        // required before either throwing operation.
        for instruction in ir.instructions().values() {
            if let ManagedInsn::Phi { inputs, .. } = instruction {
                let l = inputs.iter().find(|(e, _)| *e == left.0).map(|(_, v)| v);
                let r = inputs.iter().find(|(e, _)| *e == right.0).map(|(_, v)| v);
                if l != r || l.is_some_and(|v| aliases.contains(v)) {
                    return false;
                }
            }
        }
    }
    true
}
// Only an exact instruction result replaces that dynamic occurrence on success.
// NEW does not also replace its aliases; their own original Moves must execute.
fn successful_result(instruction: &ManagedInsn) -> Option<ValueId> {
    use ManagedInsn::*;
    match instruction {
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
        | InstanceOf { result, .. } => Some(*result),
        Invoke { result, .. } | NewFilledArray { result, .. } => *result,
        _ => None,
    }
}
fn handler_observes(
    ir: &MethodIr<SsaForm>,
    entry: BlockId,
    incoming: EdgeId,
    aliases: &HashSet<ValueId>,
    resolver: &impl JavaResolver,
) -> bool {
    handler_observes_from_invalid(ir, entry, incoming, aliases, aliases, resolver)
}
// Separating the initial path state permits direct proof of later loop occurrences.
fn handler_observes_from_invalid(
    ir: &MethodIr<SsaForm>,
    entry: BlockId,
    incoming: EdgeId,
    aliases: &HashSet<ValueId>,
    initial_invalid: &HashSet<ValueId>,
    resolver: &impl JavaResolver,
) -> bool {
    if !ir
        .cfg()
        .edge(incoming)
        .is_ok_and(|edge| edge.target() == entry)
    {
        return true;
    }
    let mut invalid: Vec<_> = initial_invalid.iter().copied().collect();
    invalid.sort();
    // Pending aliases remain source-unavailable until their exact fresh constructor
    // succeeds. Each Phi cluster additionally retains its predecessor snapshot.
    let mut todo = vec![(
        entry,
        incoming,
        Some((incoming, invalid.clone())),
        invalid,
        Vec::<(ValueId, InsnId, ClassRef)>::new(),
    )];
    let mut seen = HashSet::new();
    while let Some((b, incoming, cluster, invalid, pending)) = todo.pop() {
        if invalid.is_empty()
            && pending.is_empty()
            && cluster
                .as_ref()
                .is_none_or(|(_, snapshot)| snapshot.is_empty())
        {
            continue;
        }
        if seen.len() >= 65_536 {
            return true;
        }
        if !seen.insert((
            b,
            incoming,
            cluster.clone(),
            invalid.clone(),
            pending.clone(),
        )) {
            continue;
        }
        let Ok(block) = ir.cfg().block(b) else {
            return true;
        };
        let id = *block.payload();
        let Ok(instruction) = ir.instruction(id) else {
            return true;
        };
        let mut unavailable = invalid.clone();
        for (value, _, _) in &pending {
            if !unavailable.contains(value) {
                unavailable.push(*value);
            }
        }
        unavailable.sort();
        let mut after = invalid.clone();
        let mut pending_after = pending.clone();
        let pending_copy = if let ManagedInsn::Move { value, .. } = instruction {
            pending.iter().find(|(v, _, _)| v == value).copied()
        } else {
            None
        };
        let initializing = if let ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method,
            args,
        } = instruction
        {
            if let Some((_, root, class)) = args
                .first()
                .and_then(|receiver| pending.iter().find(|(v, _, _)| v == receiver))
            {
                let Some(target) = resolver.method(*method) else {
                    return true;
                };
                let Ok(args) = crate::invoke_arguments::operands(&target, args) else {
                    return true;
                };
                if target.name != "<init>"
                    || target.is_static
                    || target.return_type != JavaType::Void
                    || resolver.class(*class).as_deref() != Some(target.owner.as_str())
                    || args.len() != target.parameters.len() + 1
                    || args[1..]
                        .iter()
                        .any(|v| invalid.contains(v) || pending.iter().any(|(p, _, _)| p == v))
                {
                    return true;
                }
                Some(*root)
            } else {
                None
            }
        } else {
            None
        };
        let next_cluster = match instruction {
            ManagedInsn::Phi { inputs, .. } => {
                let selected: Vec<_> = inputs
                    .iter()
                    .filter(|(edge, _)| *edge == incoming)
                    .collect();
                let (selected, selected_edge) = if selected.is_empty() {
                    let Some((origin, _)) = &cluster else {
                        return true;
                    };
                    (
                        inputs
                            .iter()
                            .filter(|(edge, _)| edge == origin)
                            .collect::<Vec<_>>(),
                        *origin,
                    )
                } else {
                    (selected, incoming)
                };
                let [(_, source)] = selected.as_slice() else {
                    return true;
                };
                let snapshot = cluster
                    .as_ref()
                    .filter(|(edge, _)| *edge == selected_edge)
                    .map_or(&unavailable, |(_, snapshot)| snapshot);
                if snapshot.contains(source) || pending.iter().any(|(v, _, _)| v == source) {
                    return true;
                }
                Some((selected_edge, snapshot.clone()))
            }
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads: inputs,
                ..
            } if inputs.is_empty() => cluster,
            _ => {
                if pending_copy.is_none()
                    && initializing.is_none()
                    && reads(instruction)
                        .iter()
                        .any(|v| invalid.contains(v) || pending.iter().any(|(p, _, _)| p == v))
                {
                    return true;
                }
                None
            }
        };
        if let Some(result) = successful_result(instruction) {
            if !ir.value(result).is_ok_and(|value| value.def() == id) {
                return true;
            }
            if let ManagedInsn::NewInstance { class, .. } = instruction {
                if aliases.contains(&result)
                    || invalid.contains(&result)
                    || pending.iter().any(|(v, _, _)| *v == result)
                {
                    // Reexecution creates a distinct occurrence. Unreassigned old
                    // aliases remain invalid, never released by this new ctor.
                    for (alias, root, _) in &pending {
                        if *root == id && *alias != result && !after.contains(alias) {
                            after.push(*alias);
                        }
                    }
                    pending_after.retain(|(alias, root, _)| *alias != result && *root != id);
                    pending_after.push((result, id, *class));
                }
            } else {
                pending_after.retain(|(alias, _, _)| *alias != result);
                if let Some((_, root, class)) = pending_copy {
                    pending_after.push((result, root, class));
                }
            }
            after.retain(|value| *value != result);
        }
        if let Some(root) = initializing {
            pending_after.retain(|(_, allocation, _)| *allocation != root);
        }
        after.sort();
        pending_after.sort_by_key(|(value, _, _)| *value);
        for edge in block.outgoing() {
            let Ok(e) = ir.cfg().edge(*edge) else {
                return true;
            };
            if matches!(e.kind(), EdgeKind::Exception(_)) {
                todo.push((
                    e.target(),
                    *edge,
                    Some((*edge, unavailable.clone())),
                    invalid.clone(),
                    pending.clone(),
                ));
            } else {
                todo.push((
                    e.target(),
                    *edge,
                    next_cluster.clone(),
                    after.clone(),
                    pending_after.clone(),
                ));
            }
        }
    }
    false
}

#[cfg(test)]
#[path = "exception_allocation_reentry_tests.rs"]
mod reentry_tests;

pub(super) struct ScopedResolver<'a, R> {
    inner: &'a R,
    chains: RefCell<Proof>,
}
impl<'a, R> ScopedResolver<'a, R> {
    pub(super) fn new(inner: &'a R) -> Self {
        Self {
            inner,
            chains: RefCell::new(Proof::default()),
        }
    }
    // Narrow parent-owned resolver capability delegates to this exact witness.
    pub(super) fn pure_chain(&self, chain: &[InsnId]) -> bool {
        self.chains
            .borrow()
            .pure
            .get(chain)
            .is_some_and(cross_scope::Witness::attested)
    }
    pub(super) fn install(&self, chains: Proof) {
        *self.chains.borrow_mut() = chains;
    }
}
impl<R: JavaResolver> JavaResolver for ScopedResolver<'_, R> {
    fn discarded_invoke_source_target(&self, reference: MethodRef) -> bool {
        self.inner.discarded_invoke_source_target(reference)
    }
    fn array_clone_method(&self, reference: MethodRef, receiver: &JavaType) -> Option<JavaMethod> {
        self.inner.array_clone_method(reference, receiver)
    }
    fn exception_allocation_sequence_chain(&self, chain: &[InsnId]) -> bool {
        self.chains
            .borrow()
            .sequences
            .get(chain)
            .is_some_and(ordered_string::Witness::attested)
    }
    fn exception_allocation_pure_chain(&self, chain: &[InsnId]) -> bool {
        self.pure_chain(chain)
    }
    fn exception_allocation_chain(&self, chain: &[InsnId]) -> bool {
        self.chains.borrow().contains(chain)
    }
    fn trusted_string_builder_class(&self, r: ClassRef) -> bool {
        self.inner.trusted_string_builder_class(r)
    }
    fn java_super_selects_dex_target(&self, c: &str, r: MethodRef) -> bool {
        self.inner.java_super_selects_dex_target(c, r)
    }
    fn super_invocation(&self, c: &str, r: MethodRef) -> Option<SuperInvocation> {
        self.inner.super_invocation(c, r)
    }
    fn retain_method_receiver(&self, r: MethodRef, c: &str, t: &JavaType) -> bool {
        self.inner.retain_method_receiver(r, c, t)
    }
    fn field_access(&self, r: FieldRef) -> Option<u32> {
        self.inner.field_access(r)
    }
    fn static_method_type_context(&self, r: MethodRef, c: &str) -> bool {
        self.inner.static_method_type_context(r, c)
    }
    fn static_method_owner_access(&self, r: MethodRef) -> Option<u32> {
        self.inner.static_method_owner_access(r)
    }
    fn static_field_type_context(&self, r: FieldRef, c: &str) -> bool {
        self.inner.static_field_type_context(r, c)
    }
    fn field_source_nonconstant(&self, r: FieldRef) -> bool {
        self.inner.field_source_nonconstant(r)
    }
    fn retain_field_receiver(&self, r: FieldRef, c: &str, t: &JavaType) -> bool {
        self.inner.retain_field_receiver(r, c, t)
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        self.inner.method_access(r)
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        self.inner.assignable(a, b)
    }
    fn instruction_type(&self, r: TypeRef) -> Option<JavaType> {
        self.inner.instruction_type(r)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        self.inner.method(r)
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        self.inner.field(r)
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        self.inner.class(r)
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        self.inner.string(r)
    }
}

#[cfg(test)]
mod super_invocation_binding_tests {
    use super::*;

    #[test]
    fn scoped_resolver_forwards_selected_binding_without_legacy_attestation() {
        let inner = crate::super_invocation_binding_tests::Resolver::fixture();
        let scoped = ScopedResolver::new(&inner);
        let Some(binding) = scoped.super_invocation("SuperBindingHarness.Child", MethodRef::new(0))
        else {
            panic!("selected target binding survives exception resolver");
        };
        assert_eq!(binding.method.owner, "SuperBindingHarness.Mid");
        assert_eq!(
            binding.method.parameters,
            vec![JavaType::Class("java.lang.Object".into())]
        );
        assert_eq!(binding.access_flags, 1);
        assert!(
            !scoped.java_super_selects_dex_target("SuperBindingHarness.Child", MethodRef::new(0))
        );
    }
}

#[cfg(test)]
#[path = "exception_argument_scope_tests.rs"]
mod argument_scope_tests;

#[cfg(test)]
#[path = "exception_input_copy_scope_tests.rs"]
mod input_copy_scope_tests;

#[cfg(test)]
#[path = "exception_handler_edge_observation_tests.rs"]
mod handler_edge_observation_tests;

#[cfg(test)]
mod discarded_target_forwarding_tests {
    use super::*;
    struct Resolver;
    impl JavaResolver for Resolver {
        fn discarded_invoke_source_target(&self, r: MethodRef) -> bool {
            r == MethodRef::new(7)
        }
        fn method(&self, _: MethodRef) -> Option<JavaMethod> {
            None
        }
        fn field(&self, _: FieldRef) -> Option<JavaField> {
            None
        }
        fn class(&self, _: ClassRef) -> Option<String> {
            None
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
    }
    #[test]
    fn scoped_resolver_preserves_exact_discarded_target_attestation() {
        let scoped = ScopedResolver::new(&Resolver);
        assert!(scoped.discarded_invoke_source_target(MethodRef::new(7)));
        assert!(!scoped.discarded_invoke_source_target(MethodRef::new(8)));
    }
}
