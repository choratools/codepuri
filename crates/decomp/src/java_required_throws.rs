//! Derived Java-source checked exceptions; original DEX annotations stay untouched.
use crate::java_aliases::MethodKey;
use backend_java::{JavaResolver, JavaType};
use dex_decompiler::ssa::SsaMethod;
use dex_format::code::CodeItem;
use managed_ir::{
    insn::ManagedInsn,
    ops::InvokeKind,
    refs::{ClassRef, MethodRef},
    types::ManagedType,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[path = "java_required_throws_handlers.rs"]
mod handler_provenance;

/// Exact original method identity, including its DEX-local pool domain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OriginalMethodIdentity {
    /// Input DEX ordinal.
    pub dex: usize,
    /// Original method-pool index.
    pub index: u32,
    /// Full descriptor identity, including return type.
    pub key: MethodKey,
}
/// Attested classification through genuine complete exception hierarchy edges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExceptionKind {
    /// Throwable subtype excluding RuntimeException and Error.
    Checked,
    /// RuntimeException or Error subtype.
    Unchecked,
}
/// Genuine inherited declaration, before source throws derivation.
#[derive(Clone, Debug)]
pub struct OverrideDeclaration {
    /// Concrete input ancestor requiring an approved source contract; external/native/abstract declarations use None.
    pub source_target: Option<MethodKey>,
    /// Exact original inherited method identity.
    pub key: MethodKey,
    /// Genuine source-declaration checked exceptions in the frozen naming domain.
    pub throws: Vec<String>,
}
/// Evidence for all Java overriding/hiding/interface obligations.
#[derive(Clone, Debug)]
pub enum OverrideContract {
    /// Complete hierarchy lookup proved no affected inherited declaration.
    NoInheritedDeclaration,
    /// Every relevant inherited declaration, including empty throws contracts.
    Declarations(Vec<OverrideDeclaration>),
    /// Hierarchy or declaration lookup is incomplete.
    Unknown,
}
/// A genuine invocation declaration and its source-analysis status.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct InvocationContract {
    /// Input source target requiring analysis; None means an attested fixed declaration.
    pub source_target: Option<MethodKey>,
    /// Original declaration throws, never inferred empty from unavailable metadata.
    pub declared: Vec<String>,
}
/// Positive hierarchy evidence; missing or ambiguous declarations reject.
pub trait ThrowsHierarchy: JavaResolver {
    /// Classify a genuine thrown type; None is insufficient hierarchy evidence.
    fn exception_kind(&self, name: &str) -> Option<ExceptionKind>;
}
/// Method-local pool resolver and original inherited declaration evidence.
pub trait RequiredThrowsContext: ThrowsHierarchy {
    /// Exact target metadata; concrete input targets retain their full source identity.
    fn invocation_contract(&self, reference: MethodRef) -> Option<InvocationContract>;
    /// Attest source reconstruction for an unannotated executable input virtual
    /// declaration and these exact executable descendant overrides. Explicit
    /// original Throws annotations, native/external declarations, incomplete
    /// ancestry and unknown emitted override identities stay closed.
    fn attests_concrete_override_domain(
        &self,
        _identity: &OriginalMethodIdentity,
        _implementations: &[MethodKey],
    ) -> bool {
        false
    }
    /// Attest the platform array clone operation for a symbolic method. Called
    /// only for virtual invocation on a published SSA array receiver. Ordinary
    /// Object.clone and user declarations retain their genuine checked contract.
    fn array_clone_contract(
        &self,
        _reference: MethodRef,
        _receiver: &ManagedType,
    ) -> Option<InvocationContract> {
        None
    }
    /// Receiver-static declaration selected by the same proof used by Java
    /// emission. This never changes the runtime virtual target.
    fn receiver_invocation_contract(
        &self,
        _caller: &OriginalMethodIdentity,
        _kind: InvokeKind,
        _reference: MethodRef,
        _receiver: &ManagedType,
    ) -> Option<InvocationContract> {
        None
    }
    /// Resolve an invocation using its original caller and dispatch kind.
    /// Inherited super targets may differ from the symbolic pool owner.
    fn invoked_contract(
        &self,
        _caller: &OriginalMethodIdentity,
        _kind: InvokeKind,
        reference: MethodRef,
    ) -> Option<InvocationContract> {
        self.invocation_contract(reference)
    }
    /// Complete original-name overriding and interface lookup.
    fn override_contract(&self, identity: &OriginalMethodIdentity) -> OverrideContract;
}
/// Typed source-throws derivation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ThrowsError {
    /// Missing/ambiguous original metadata, hierarchy or value type.
    Unknown(String),
    /// Protected scopes cannot be proved exact.
    InvalidScope,
    /// A concrete input callee has no source-analysis contract.
    MissingSourceTarget(MethodKey),
    /// An inherited declaration forbids a required checked exception.
    OverrideConflict(String),
    /// Java cannot add a throws clause to this declaration (for example clinit).
    IllegalDeclaration,
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Coverage {
    catches: Vec<String>,
    all: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Event {
    Throw(String, Coverage),
    /// Finite nominal types of a mixed value; never captured-object identity.
    NominalThrow(BTreeSet<String>, Coverage),
    Invoke(InvocationContract, Coverage),
    /// Original catch-all object, with exact protected throwing predecessors.
    Rethrow(Vec<RethrowOrigin>, Coverage),
    /// Source declaration constraint from an exact attested implementation body.
    Implementation(MethodKey, Coverage),
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RethrowOrigin {
    source: usize,
    caught: Option<String>,
    excluded: Vec<String>,
}
/// Exact body requirements, independent of its original throws annotations.
#[derive(Clone, Debug)]
pub struct MethodRequirements {
    /// Original identity and provenance.
    pub identity: OriginalMethodIdentity,
    /// Original annotations, unchanged and separately retained.
    pub original: Vec<String>,
    flags: u32,
    overrides: OverrideContract,
    events: Vec<Event>,
}
impl MethodRequirements {
    // A swallowed catch-all invocation has no outward exception domain. Keep
    // the callee dependency when a captured-object rethrow refers to this
    // event: that handler needs the exact prethrow domain, even if the call
    // itself is fully covered. Member/body publication closure is independent.
    fn needs_source_domain(&self, index: usize) -> bool {
        !matches!(&self.events[index], Event::Invoke(_, Coverage { all: true, .. }))
            || self.events.iter().any(|event| {
                matches!(event, Event::Rethrow(origins, _) if origins.iter().any(|origin| origin.source == index))
            })
    }

    /// Exact input-source callees requiring source-contract analysis.
    /// External declarations are not source-analysis dependencies.
    pub fn source_targets(&self) -> impl Iterator<Item = &MethodKey> {
        self.events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| match event {
                Event::Invoke(contract, _) if self.needs_source_domain(index) => {
                    contract.source_target.as_ref()
                }
                Event::Implementation(target, _) => Some(target),
                Event::Invoke(_, _)
                | Event::Throw(_, _)
                | Event::NominalThrow(_, _)
                | Event::Rethrow(_, _) => None,
            })
            .chain(
                match &self.overrides {
                    OverrideContract::Declarations(declarations) => declarations.as_slice(),
                    _ => &[],
                }
                .iter()
                .filter_map(|d| d.source_target.as_ref()),
            )
    }
}
/// Source-only declaration plan; presence does not certify a complete source class.
#[derive(Clone, Debug)]
pub struct DerivedThrows {
    /// Exact original identity.
    pub identity: OriginalMethodIdentity,
    /// Original annotation names, preserved unchanged.
    pub original: Vec<String>,
    /// Additional required uncaught checked exceptions.
    pub derived: Vec<String>,
    /// Sorted original-plus-derived source declaration.
    pub source: Vec<String>,
}
/// Per-method successes and typed failures; no original metadata is modified.
#[derive(Clone, Debug, Default)]
pub struct SourceThrowsPlan {
    /// Exact full-descriptor source declarations.
    pub methods: BTreeMap<MethodKey, DerivedThrows>,
    /// Exact unavailable source declarations.
    pub unavailable: BTreeMap<MethodKey, ThrowsError>,
    /// Proven conservative runtime escaping domains, independent of whether a
    /// widened Java declaration would satisfy the original override contract.
    pub runtime: BTreeMap<MethodKey, RuntimeThrows>,
}

/// Runtime evidence is not a replacement for an emitted Java throws clause.
#[derive(Clone, Debug)]
pub struct RuntimeThrows {
    /// Exact input identity, including its defining DEX domain.
    pub identity: OriginalMethodIdentity,
    /// Genuine original annotation contract, retained unchanged.
    pub original: Vec<String>,
    /// Conservative checked escaping domain after original handler coverage.
    pub escaping: Vec<String>,
}

/// Extract throw/invoke requirements at their original code-unit offsets.
/// Typed catches cover only the exact raw protected range; handler bodies are
/// independently inspected and can contribute new uncaught exceptions.
pub fn inspect_method(
    identity: OriginalMethodIdentity,
    ssa: &SsaMethod,
    code: &CodeItem,
    flags: u32,
    original: Vec<String>,
    context: &impl RequiredThrowsContext,
) -> Result<MethodRequirements, ThrowsError> {
    let caller = context
        .invocation_contract(MethodRef::new(identity.index))
        .ok_or_else(|| ThrowsError::Unknown("original caller declaration".into()))?;
    if caller.source_target.as_ref() != Some(&identity.key)
        || caller.declared != original
        || context.method_access(MethodRef::new(identity.index)) != Some(flags)
    {
        return Err(ThrowsError::Unknown(
            "original caller identity/flags/throws".into(),
        ));
    }
    let boundaries = if code.tries.is_empty() {
        BTreeSet::new()
    } else {
        dex_decompiler::decode_method(code)
            .map_err(|_| ThrowsError::InvalidScope)?
            .iter()
            .map(|i| i.offset)
            .collect::<BTreeSet<_>>()
    };
    let mut ranges = Vec::new();
    let mut previous_end = 0;
    for range in &code.tries {
        let end = range
            .start_addr
            .checked_add(u32::from(range.insn_count))
            .ok_or(ThrowsError::InvalidScope)?;
        if range.insn_count == 0
            || range.start_addr < previous_end
            || end > code.insns_size
            || !boundaries.contains(&range.start_addr)
            || (end != code.insns_size && !boundaries.contains(&end))
        {
            return Err(ThrowsError::InvalidScope);
        }
        previous_end = end;
        let handler = code
            .handlers
            .iter()
            .find(|h| h.relative_offset == u32::from(range.handler_off))
            .ok_or(ThrowsError::InvalidScope)?;
        let mut catches = Vec::new();
        for typed in &handler.handlers {
            if typed.address >= code.insns_size
                || !ssa
                    .ir()
                    .instructions()
                    .keys()
                    .any(|id| ssa.offset(id) == Some(typed.address))
            {
                return Err(ThrowsError::InvalidScope);
            }
            let name = context
                .class(ClassRef::new(typed.type_idx))
                .ok_or_else(|| ThrowsError::Unknown("catch declaration".into()))?;
            context
                .exception_kind(&name)
                .ok_or_else(|| ThrowsError::Unknown(format!("catch hierarchy {name}")))?;
            catches.push(name);
        }
        if let Some(address) = handler.catch_all_addr
            && (address >= code.insns_size
                || !ssa
                    .ir()
                    .instructions()
                    .keys()
                    .any(|id| ssa.offset(id) == Some(address)))
        {
            return Err(ThrowsError::InvalidScope);
        }
        ranges.push((
            range.start_addr,
            end,
            Coverage {
                catches,
                all: handler.catch_all_addr.is_some(),
            },
        ));
    }
    let handler_types = handler_provenance::HandlerTypes::collect(ssa, code, context);
    let mut events = Vec::new();
    let mut event_ids = BTreeMap::new();
    let mut rethrows = Vec::new();
    for (id, instruction) in ssa.ir().instructions() {
        if let ManagedInsn::Opaque { effects, .. } = instruction
            && effects.contains(managed_ir::effects::EffectFlags::MAY_THROW)
            && !handler_types.attests(id)
        {
            return Err(ThrowsError::Unknown("opaque throwing effects".into()));
        }
        if !matches!(
            instruction,
            ManagedInsn::Throw { .. } | ManagedInsn::Invoke { .. }
        ) {
            continue;
        }
        let offset = ssa
            .offset(id)
            .ok_or_else(|| ThrowsError::Unknown("throwing instruction provenance".into()))?;
        if offset >= code.insns_size {
            return Err(ThrowsError::InvalidScope);
        }
        let coverage = ranges
            .iter()
            .find(|(start, end, _)| *start <= offset && offset < *end)
            .map(|(_, _, c)| c.clone())
            .unwrap_or(Coverage {
                catches: vec![],
                all: false,
            });
        event_ids.insert(id, events.len());
        match instruction {
            ManagedInsn::Throw { value } => {
                let record = ssa
                    .ir()
                    .value(*value)
                    .map_err(|_| ThrowsError::Unknown("throw value".into()))?;
                let name = if backend_java::proven_null_throw(ssa.ir(), *value) {
                    "java.lang.NullPointerException".into()
                } else if let Some(origins) = handler_types.origins(ssa.ir(), *value) {
                    rethrows.push((events.len(), origins));
                    events.push(Event::Rethrow(vec![], coverage));
                    continue;
                } else if let Some(name) = handler_types.thrown_type(ssa.ir(), *value) {
                    name.to_owned()
                } else if matches!(record.ty(), ManagedType::Unknown)
                    && let Some(names) =
                        handler_types.nominal_thrown_types(ssa.ir(), *value, context)
                {
                    events.push(Event::NominalThrow(names, coverage));
                    continue;
                } else {
                    match record.ty() {
                        ManagedType::Reference(reference) => context
                            .class(*reference)
                            .ok_or_else(|| ThrowsError::Unknown("throw class".into()))?,
                        _ => return Err(ThrowsError::Unknown("throw value type".into())),
                    }
                };
                events.push(Event::Throw(name, coverage));
            }
            ManagedInsn::Invoke {
                kind, method, args, ..
            } => events.push(Event::Invoke(
                (if *kind == InvokeKind::Virtual && args.len() == 1 {
                    ssa.ir().value(args[0]).ok().and_then(|value| {
                        matches!(value.ty(), ManagedType::Array { .. })
                            .then(|| context.array_clone_contract(*method, value.ty()))
                            .flatten()
                    })
                } else {
                    None
                })
                .or_else(|| {
                    args.first()
                        .and_then(|v| ssa.ir().value(*v).ok())
                        .and_then(|v| {
                            context.receiver_invocation_contract(&identity, *kind, *method, v.ty())
                        })
                })
                .or_else(|| context.invoked_contract(&identity, *kind, *method))
                .ok_or_else(|| ThrowsError::Unknown(format!("callee {method:?}")))?,
                coverage,
            )),
            _ => {}
        }
    }
    let has_rethrows = !rethrows.is_empty();
    for (index, origins) in rethrows {
        let Event::Rethrow(bound, _) = &mut events[index] else {
            unreachable!();
        };
        for origin in origins {
            // Non-invoke/throw Dalvik operations have only unchecked runtime
            // exceptions. Unsupported opaque effects were rejected above.
            if let Some(source) = event_ids.get(&origin.source) {
                bound.push(RethrowOrigin {
                    source: *source,
                    caught: origin.caught,
                    excluded: origin.excluded,
                });
            }
        }
    }
    // Rethrow predecessors refer to original event positions. Sorting or
    // deduplicating those records would silently bind a different instruction.
    // Legacy bodies retain their compact canonical event representation.
    if !has_rethrows {
        events.sort();
        events.dedup();
    }
    let overrides = context.override_contract(&identity);
    Ok(MethodRequirements {
        identity,
        original,
        flags,
        overrides,
        events,
    })
}

#[cfg(test)]
#[path = "java_required_throws_array_tests.rs"]
mod array_clone_tests;
#[cfg(test)]
#[path = "java_required_throws_null_tests.rs"]
mod null_throw_tests;

fn covered(name: &str, coverage: &Coverage, context: &impl ThrowsHierarchy) -> bool {
    coverage.all
        || coverage.catches.iter().any(|catch| {
            name == catch
                || context.assignable(
                    &JavaType::Class(name.into()),
                    &JavaType::Class(catch.clone()),
                )
        })
}

/// Preserve precise rethrow domains instead of claiming every catch-all object
/// imposes Throwable on the caller. The finite union also handles nested
/// handler copies without recursion depending on a traversal's active path.
fn event_types(
    method: &MethodRequirements,
    required: &BTreeMap<MethodKey, BTreeSet<String>>,
    failures: &BTreeMap<MethodKey, ThrowsError>,
    context: &impl ThrowsHierarchy,
) -> Result<Vec<BTreeSet<String>>, ThrowsError> {
    let mut types = Vec::with_capacity(method.events.len());
    for (index, event) in method.events.iter().enumerate() {
        types.push(match event {
            Event::Throw(name, _) => BTreeSet::from([name.clone()]),
            Event::NominalThrow(names, _) => names.clone(),
            Event::Invoke(call, _) => {
                let mut names = call.declared.iter().cloned().collect::<BTreeSet<_>>();
                if let Some(target) = &call.source_target
                    && method.needs_source_domain(index)
                {
                    if failures.contains_key(target) {
                        return Err(ThrowsError::MissingSourceTarget(target.clone()));
                    }
                    names.extend(
                        required
                            .get(target)
                            .ok_or_else(|| ThrowsError::MissingSourceTarget(target.clone()))?
                            .iter()
                            .cloned(),
                    );
                }
                names
            }
            Event::Rethrow(_, _) => BTreeSet::new(),
            Event::Implementation(target, _) => {
                if failures.contains_key(target) {
                    return Err(ThrowsError::MissingSourceTarget(target.clone()));
                }
                required
                    .get(target)
                    .ok_or_else(|| ThrowsError::MissingSourceTarget(target.clone()))?
                    .clone()
            }
        });
    }
    let matches = |name: &str, base: &str| {
        name == base
            || context.assignable(&JavaType::Class(name.into()), &JavaType::Class(base.into()))
    };
    loop {
        let mut changed = false;
        for (index, event) in method.events.iter().enumerate() {
            let Event::Rethrow(origins, _) = event else {
                continue;
            };
            let mut found = BTreeSet::new();
            for origin in origins {
                let source = types
                    .get(origin.source)
                    .ok_or_else(|| ThrowsError::Unknown("handler throwing predecessor".into()))?;
                for name in source {
                    let bound = match &origin.caught {
                        Some(base) if matches(name, base) => name,
                        Some(base) if matches(base, name) => base,
                        Some(_) => continue,
                        None => name,
                    };
                    if !origin.excluded.iter().any(|base| matches(bound, base)) {
                        found.insert(bound.clone());
                    }
                }
            }
            let old = types[index].len();
            types[index].extend(found);
            changed |= old != types[index].len();
        }
        if !changed {
            return Ok(types);
        }
    }
}
/// Build a source-only abstract declaration constraint. Implementations must be
/// exact bodies attested by the collector's complete original override lookup.
/// No native declaration or missing implementation can supply a widened domain.
pub fn inspect_abstract_declaration(
    identity: OriginalMethodIdentity,
    flags: u32,
    original: Vec<String>,
    implementations: Vec<MethodKey>,
    context: &impl RequiredThrowsContext,
) -> Result<MethodRequirements, ThrowsError> {
    if flags & 0x400 == 0
        || flags & (0x100 | 8 | 2) != 0
        || context.method_access(MethodRef::new(identity.index)) != Some(flags)
        || identity.key.name.starts_with('<')
    {
        return Err(ThrowsError::Unknown(
            "abstract implementation evidence".into(),
        ));
    }
    let caller = context
        .invocation_contract(MethodRef::new(identity.index))
        .ok_or_else(|| ThrowsError::Unknown("abstract declaration metadata".into()))?;
    if caller.source_target.as_ref() != Some(&identity.key)
        || caller.declared != original
        || implementations.iter().any(|key| {
            key == &identity.key
                || key.name != identity.key.name
                || key.parameters != identity.key.parameters
        })
    {
        return Err(ThrowsError::Unknown(
            "abstract implementation identity".into(),
        ));
    }
    let overrides = context.override_contract(&identity);
    Ok(MethodRequirements {
        identity,
        original,
        flags,
        overrides,
        events: implementations
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|key| {
                Event::Implementation(
                    key,
                    Coverage {
                        catches: vec![],
                        all: false,
                    },
                )
            })
            .collect(),
    })
}
/// Extend an already inspected concrete body with its attested input override
/// domain. The body events, original flags/annotations and inherited bounds are
/// retained; the existing fixed point checks every member of the joint domain.
pub fn attest_concrete_override_domain(
    mut inspected: MethodRequirements,
    implementations: Vec<MethodKey>,
    context: &impl RequiredThrowsContext,
) -> Result<MethodRequirements, ThrowsError> {
    let identity = &inspected.identity;
    let caller = context
        .invocation_contract(MethodRef::new(identity.index))
        .ok_or_else(|| ThrowsError::Unknown("concrete declaration metadata".into()))?;
    let mut unique = BTreeSet::new();
    if inspected.flags & (2 | 8 | 0x10 | 0x100 | 0x400) != 0
        || identity.key.name.starts_with('<')
        || !inspected.original.is_empty()
        || context.method_access(MethodRef::new(identity.index)) != Some(inspected.flags)
        || caller.source_target.as_ref() != Some(&identity.key)
        || caller.declared != inspected.original
        || implementations.iter().any(|key| {
            key.owner == identity.key.owner
                || key.name != identity.key.name
                || key.parameters != identity.key.parameters
                || !unique.insert(key.clone())
        })
        || !context.attests_concrete_override_domain(identity, &implementations)
    {
        return Err(ThrowsError::Unknown(
            "concrete override domain evidence".into(),
        ));
    }
    inspected
        .events
        .extend(implementations.into_iter().map(|key| {
            Event::Implementation(
                key,
                Coverage {
                    catches: vec![],
                    all: false,
                },
            )
        }));
    Ok(inspected)
}
/// Compute uncaught checked exceptions to a fixed point over exact input calls.
/// Public/protected/package declarations require complete inherited-contract
/// evidence before adding throws. Private declarations do not override; actual
/// callers still propagate or catch the resulting source contract.
pub fn derive_program(
    methods: &[MethodRequirements],
    context: &impl ThrowsHierarchy,
) -> Result<SourceThrowsPlan, ThrowsError> {
    // Borrow immutable identities and event targets for graph scheduling. BTree
    // ordering still compares full MethodKey values, including missing targets.
    let mut catalog = BTreeMap::new();
    for method in methods {
        if catalog.insert(&method.identity.key, method).is_some() {
            return Err(ThrowsError::Unknown(
                "duplicate original method identity".into(),
            ));
        }
    }
    let mut required = catalog
        .iter()
        .map(|(key, m)| {
            (
                (*key).clone(),
                m.original.iter().cloned().collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut reverse = BTreeMap::<&MethodKey, BTreeSet<&MethodKey>>::new();
    for (key, method) in &catalog {
        if let OverrideContract::Declarations(declarations) = &method.overrides {
            for target in declarations.iter().filter_map(|d| d.source_target.as_ref()) {
                reverse.entry(target).or_default().insert(*key);
            }
        }
        for (index, event) in method.events.iter().enumerate() {
            if !method.needs_source_domain(index) {
                continue;
            }
            if let Some(target) = match event {
                Event::Invoke(
                    InvocationContract {
                        source_target: Some(target),
                        ..
                    },
                    _,
                )
                | Event::Implementation(target, _) => Some(target),
                _ => None,
            } {
                reverse.entry(target).or_default().insert(*key);
            }
        }
    }
    let mut dirty = catalog.keys().copied().collect::<VecDeque<_>>();
    let mut queued = catalog.keys().copied().collect::<BTreeSet<_>>();
    let mut failures = BTreeMap::new();
    let mut inherited_degree = catalog
        .keys()
        .map(|key| (*key, 0usize))
        .collect::<BTreeMap<_, _>>();
    let mut inherited_reverse = BTreeMap::<&MethodKey, BTreeSet<&MethodKey>>::new();
    for (key, method) in &catalog {
        if let OverrideContract::Declarations(declarations) = &method.overrides {
            let targets = declarations
                .iter()
                .filter_map(|d| d.source_target.as_ref())
                .collect::<BTreeSet<_>>();
            for target in targets {
                if !catalog.contains_key(target)
                    || !declarations
                        .iter()
                        .any(|d| d.source_target.as_ref() == Some(target) && &d.key == target)
                {
                    failures.insert(
                        (*key).clone(),
                        ThrowsError::MissingSourceTarget(target.clone()),
                    );
                } else {
                    if let Some(degree) = inherited_degree.get_mut(key) {
                        *degree += 1;
                    }
                    inherited_reverse.entry(target).or_default().insert(*key);
                }
            }
        }
    }
    let mut roots = inherited_degree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(key, _)| *key)
        .collect::<VecDeque<_>>();
    let mut ordered = BTreeSet::new();
    while let Some(key) = roots.pop_front() {
        ordered.insert(key);
        for child in inherited_reverse.get(&key).into_iter().flatten() {
            if let Some(degree) = inherited_degree.get_mut(child) {
                *degree -= 1;
                if *degree == 0 {
                    roots.push_back(*child);
                }
            }
        }
    }
    for key in catalog.keys().filter(|key| !ordered.contains(*key)) {
        failures.insert(
            (*key).clone(),
            ThrowsError::Unknown("cyclic inherited source contracts".into()),
        );
    }
    while let Some(key) = dirty.pop_front() {
        queued.remove(&key);
        if failures.contains_key(key) {
            continue;
        }
        let method = catalog[&key];
        let mut collected = required[key].clone();
        if collected
            .iter()
            .any(|name| context.exception_kind(name).is_none())
        {
            failures.insert(
                key.clone(),
                ThrowsError::Unknown("original throws hierarchy".into()),
            );
        }
        let types = match event_types(method, &required, &failures, context) {
            Ok(types) => types,
            Err(error) => {
                failures.insert(key.clone(), error);
                vec![]
            }
        };
        for (event, names) in method.events.iter().zip(types) {
            if failures.contains_key(key) {
                break;
            }
            let coverage = match event {
                Event::Throw(_, coverage)
                | Event::NominalThrow(_, coverage)
                | Event::Invoke(_, coverage)
                | Event::Rethrow(_, coverage)
                | Event::Implementation(_, coverage) => coverage,
            };
            // Full original coverage is already sufficient, including an
            // unavailable callee domain. Handler throws/rethrows are inspected
            // as their own original events; no body or catch is manufactured.
            if coverage.all {
                continue;
            }
            for name in names {
                match context.exception_kind(&name) {
                    Some(ExceptionKind::Unchecked) => {}
                    Some(ExceptionKind::Checked) if covered(&name, coverage, context) => {}
                    Some(ExceptionKind::Checked) => {
                        collected.insert(name);
                    }
                    None => {
                        failures.insert(
                            key.clone(),
                            ThrowsError::Unknown(format!("exception hierarchy {name}")),
                        );
                        break;
                    }
                }
            }
        }
        let changed = collected != required[key];
        if changed {
            required.insert(key.clone(), collected);
        }
        if changed || failures.contains_key(key) {
            for caller in reverse.get(&key).into_iter().flatten() {
                if queued.insert(*caller) {
                    dirty.push_back(*caller);
                }
            }
        }
    }
    let mut plan = SourceThrowsPlan {
        methods: BTreeMap::new(),
        unavailable: failures,
        runtime: BTreeMap::new(),
    };
    for (key, method) in catalog {
        if plan.unavailable.contains_key(key) {
            continue;
        }
        let source = required[key].iter().cloned().collect::<Vec<_>>();
        plan.runtime.insert(
            (*key).clone(),
            RuntimeThrows {
                identity: method.identity.clone(),
                original: method.original.clone(),
                escaping: source.clone(),
            },
        );
        let derived = source
            .iter()
            .filter(|s| !method.original.contains(s))
            .cloned()
            .collect::<Vec<_>>();
        let checked = source
            .iter()
            .filter(|name| context.exception_kind(name) == Some(ExceptionKind::Checked))
            .collect::<Vec<_>>();
        let error = if !checked.is_empty() && method.identity.key.name == "<clinit>" {
            Some(ThrowsError::IllegalDeclaration)
        } else if !checked.is_empty()
            && method.flags & 2 == 0
            && method.identity.key.name != "<init>"
        {
            match &method.overrides {
                OverrideContract::Unknown => {
                    Some(ThrowsError::Unknown("inherited throws contract".into()))
                }
                OverrideContract::NoInheritedDeclaration => None,
                OverrideContract::Declarations(declarations) if declarations.is_empty() => Some(
                    ThrowsError::Unknown("empty inherited contract evidence".into()),
                ),
                OverrideContract::Declarations(declarations)
                    if declarations.iter().any(|d| {
                        d.key.owner.is_empty()
                            || d.key.name != method.identity.key.name
                            || d.key.parameters != method.identity.key.parameters
                    }) =>
                {
                    Some(ThrowsError::Unknown(
                        "inherited declaration identity".into(),
                    ))
                }
                OverrideContract::Declarations(declarations)
                    if declarations.iter().any(|d| {
                        d.source_target.as_ref().is_some_and(|target| {
                            target != &d.key || !required.contains_key(target)
                        })
                    }) =>
                {
                    Some(ThrowsError::Unknown(
                        "unavailable input inherited source contract".into(),
                    ))
                }
                OverrideContract::Declarations(declarations) => checked
                    .into_iter()
                    .find(|name| {
                        !declarations.iter().all(|d| {
                            let approved = d
                                .source_target
                                .as_ref()
                                .and_then(|target| required.get(target));
                            let allowed = approved
                                .map(|set| set.iter().collect::<Vec<_>>())
                                .unwrap_or_else(|| d.throws.iter().collect());
                            allowed.into_iter().any(|base| {
                                *name == base
                                    || context.assignable(
                                        &JavaType::Class((*name).clone()),
                                        &JavaType::Class(base.clone()),
                                    )
                            })
                        })
                    })
                    .map(|name| ThrowsError::OverrideConflict(name.clone())),
            }
        } else {
            None
        };
        if let Some(error) = error {
            plan.unavailable.insert(key.clone(), error);
            continue;
        }
        plan.methods.insert(
            key.clone(),
            DerivedThrows {
                identity: method.identity.clone(),
                original: method.original.clone(),
                derived,
                source,
            },
        );
    }
    // Post-fixed-point contract failures invalidate reverse source dependencies,
    // including recursive components, without rescanning the whole program.
    let mut blocked = plan.unavailable.keys().cloned().collect::<VecDeque<_>>();
    while let Some(target) = blocked.pop_front() {
        for caller in reverse.get(&target).into_iter().flatten() {
            if plan.methods.remove(*caller).is_some() {
                plan.unavailable.insert(
                    (*caller).clone(),
                    ThrowsError::MissingSourceTarget(target.clone()),
                );
                blocked.push_back((*caller).clone());
            }
        }
    }
    Ok(plan)
}

// Frozen owned-key implementation for differential validation of scheduling
// and error selection. Production uses only the borrowed-key graph above.
#[cfg(test)]
fn derive_program_owned_reference(
    methods: &[MethodRequirements],
    context: &impl ThrowsHierarchy,
) -> Result<SourceThrowsPlan, ThrowsError> {
    let mut catalog = BTreeMap::new();
    for method in methods {
        if catalog
            .insert(method.identity.key.clone(), method)
            .is_some()
        {
            return Err(ThrowsError::Unknown(
                "duplicate original method identity".into(),
            ));
        }
    }
    let mut required = catalog
        .iter()
        .map(|(key, m)| {
            (
                key.clone(),
                m.original.iter().cloned().collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut reverse = BTreeMap::<MethodKey, BTreeSet<MethodKey>>::new();
    for (key, method) in &catalog {
        if let OverrideContract::Declarations(declarations) = &method.overrides {
            for target in declarations.iter().filter_map(|d| d.source_target.as_ref()) {
                reverse
                    .entry(target.clone())
                    .or_default()
                    .insert(key.clone());
            }
        }
        for (index, event) in method.events.iter().enumerate() {
            if !method.needs_source_domain(index) {
                continue;
            }
            if let Some(target) = match event {
                Event::Invoke(
                    InvocationContract {
                        source_target: Some(target),
                        ..
                    },
                    _,
                )
                | Event::Implementation(target, _) => Some(target),
                _ => None,
            } {
                reverse
                    .entry(target.clone())
                    .or_default()
                    .insert(key.clone());
            }
        }
    }
    let mut dirty = catalog.keys().cloned().collect::<VecDeque<_>>();
    let mut queued = catalog.keys().cloned().collect::<BTreeSet<_>>();
    let mut failures = BTreeMap::new();
    let mut inherited_degree = catalog
        .keys()
        .map(|key| (key.clone(), 0usize))
        .collect::<BTreeMap<_, _>>();
    let mut inherited_reverse = BTreeMap::<MethodKey, BTreeSet<MethodKey>>::new();
    for (key, method) in &catalog {
        if let OverrideContract::Declarations(declarations) = &method.overrides {
            let targets = declarations
                .iter()
                .filter_map(|d| d.source_target.as_ref())
                .collect::<BTreeSet<_>>();
            for target in targets {
                if !catalog.contains_key(target)
                    || !declarations
                        .iter()
                        .any(|d| d.source_target.as_ref() == Some(target) && &d.key == target)
                {
                    failures.insert(
                        key.clone(),
                        ThrowsError::MissingSourceTarget(target.clone()),
                    );
                } else {
                    if let Some(degree) = inherited_degree.get_mut(key) {
                        *degree += 1;
                    }
                    inherited_reverse
                        .entry(target.clone())
                        .or_default()
                        .insert(key.clone());
                }
            }
        }
    }
    let mut roots = inherited_degree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(key, _)| key.clone())
        .collect::<VecDeque<_>>();
    let mut ordered = BTreeSet::new();
    while let Some(key) = roots.pop_front() {
        ordered.insert(key.clone());
        for child in inherited_reverse.get(&key).into_iter().flatten() {
            if let Some(degree) = inherited_degree.get_mut(child) {
                *degree -= 1;
                if *degree == 0 {
                    roots.push_back(child.clone());
                }
            }
        }
    }
    for key in catalog.keys().filter(|key| !ordered.contains(*key)) {
        failures.insert(
            key.clone(),
            ThrowsError::Unknown("cyclic inherited source contracts".into()),
        );
    }
    while let Some(key) = dirty.pop_front() {
        queued.remove(&key);
        if failures.contains_key(&key) {
            continue;
        }
        let method = catalog[&key];
        let mut collected = required[&key].clone();
        if collected
            .iter()
            .any(|name| context.exception_kind(name).is_none())
        {
            failures.insert(
                key.clone(),
                ThrowsError::Unknown("original throws hierarchy".into()),
            );
        }
        let types = match event_types(method, &required, &failures, context) {
            Ok(types) => types,
            Err(error) => {
                failures.insert(key.clone(), error);
                vec![]
            }
        };
        for (event, names) in method.events.iter().zip(types) {
            if failures.contains_key(&key) {
                break;
            }
            let coverage = match event {
                Event::Throw(_, coverage)
                | Event::NominalThrow(_, coverage)
                | Event::Invoke(_, coverage)
                | Event::Rethrow(_, coverage)
                | Event::Implementation(_, coverage) => coverage,
            };
            // Full original coverage is already sufficient, including an
            // unavailable callee domain. Handler throws/rethrows are inspected
            // as their own original events; no body or catch is manufactured.
            if coverage.all {
                continue;
            }
            for name in names {
                match context.exception_kind(&name) {
                    Some(ExceptionKind::Unchecked) => {}
                    Some(ExceptionKind::Checked) if covered(&name, coverage, context) => {}
                    Some(ExceptionKind::Checked) => {
                        collected.insert(name);
                    }
                    None => {
                        failures.insert(
                            key.clone(),
                            ThrowsError::Unknown(format!("exception hierarchy {name}")),
                        );
                        break;
                    }
                }
            }
        }
        let changed = collected != required[&key];
        if changed {
            required.insert(key.clone(), collected);
        }
        if changed || failures.contains_key(&key) {
            for caller in reverse.get(&key).into_iter().flatten() {
                if queued.insert(caller.clone()) {
                    dirty.push_back(caller.clone());
                }
            }
        }
    }
    let mut plan = SourceThrowsPlan {
        methods: BTreeMap::new(),
        unavailable: failures,
        runtime: BTreeMap::new(),
    };
    for (key, method) in catalog {
        if plan.unavailable.contains_key(&key) {
            continue;
        }
        let source = required[&key].iter().cloned().collect::<Vec<_>>();
        plan.runtime.insert(
            key.clone(),
            RuntimeThrows {
                identity: method.identity.clone(),
                original: method.original.clone(),
                escaping: source.clone(),
            },
        );
        let derived = source
            .iter()
            .filter(|s| !method.original.contains(s))
            .cloned()
            .collect::<Vec<_>>();
        let checked = source
            .iter()
            .filter(|name| context.exception_kind(name) == Some(ExceptionKind::Checked))
            .collect::<Vec<_>>();
        let error = if !checked.is_empty() && method.identity.key.name == "<clinit>" {
            Some(ThrowsError::IllegalDeclaration)
        } else if !checked.is_empty()
            && method.flags & 2 == 0
            && method.identity.key.name != "<init>"
        {
            match &method.overrides {
                OverrideContract::Unknown => {
                    Some(ThrowsError::Unknown("inherited throws contract".into()))
                }
                OverrideContract::NoInheritedDeclaration => None,
                OverrideContract::Declarations(declarations) if declarations.is_empty() => Some(
                    ThrowsError::Unknown("empty inherited contract evidence".into()),
                ),
                OverrideContract::Declarations(declarations)
                    if declarations.iter().any(|d| {
                        d.key.owner.is_empty()
                            || d.key.name != method.identity.key.name
                            || d.key.parameters != method.identity.key.parameters
                    }) =>
                {
                    Some(ThrowsError::Unknown(
                        "inherited declaration identity".into(),
                    ))
                }
                OverrideContract::Declarations(declarations)
                    if declarations.iter().any(|d| {
                        d.source_target.as_ref().is_some_and(|target| {
                            target != &d.key || !required.contains_key(target)
                        })
                    }) =>
                {
                    Some(ThrowsError::Unknown(
                        "unavailable input inherited source contract".into(),
                    ))
                }
                OverrideContract::Declarations(declarations) => checked
                    .into_iter()
                    .find(|name| {
                        !declarations.iter().all(|d| {
                            let approved = d
                                .source_target
                                .as_ref()
                                .and_then(|target| required.get(target));
                            let allowed = approved
                                .map(|set| set.iter().collect::<Vec<_>>())
                                .unwrap_or_else(|| d.throws.iter().collect());
                            allowed.into_iter().any(|base| {
                                *name == base
                                    || context.assignable(
                                        &JavaType::Class((*name).clone()),
                                        &JavaType::Class(base.clone()),
                                    )
                            })
                        })
                    })
                    .map(|name| ThrowsError::OverrideConflict(name.clone())),
            }
        } else {
            None
        };
        if let Some(error) = error {
            plan.unavailable.insert(key, error);
            continue;
        }
        plan.methods.insert(
            key,
            DerivedThrows {
                identity: method.identity.clone(),
                original: method.original.clone(),
                derived,
                source,
            },
        );
    }
    // Post-fixed-point contract failures invalidate reverse source dependencies,
    // including recursive components, without rescanning the whole program.
    let mut blocked = plan.unavailable.keys().cloned().collect::<VecDeque<_>>();
    while let Some(target) = blocked.pop_front() {
        for caller in reverse.get(&target).into_iter().flatten() {
            if plan.methods.remove(caller).is_some() {
                plan.unavailable.insert(
                    caller.clone(),
                    ThrowsError::MissingSourceTarget(target.clone()),
                );
                blocked.push_back(caller.clone());
            }
        }
    }
    Ok(plan)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    struct Hierarchy;
    impl JavaResolver for Hierarchy {
        fn method(&self, _: MethodRef) -> Option<backend_java::JavaMethod> {
            None
        }
        fn field(&self, _: managed_ir::refs::FieldRef) -> Option<backend_java::JavaField> {
            None
        }
        fn class(&self, _: ClassRef) -> Option<String> {
            None
        }
        fn string(&self, _: managed_ir::refs::StringRef) -> Option<Vec<u16>> {
            None
        }
        fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
            a == b
                || (a == &JavaType::Class("java.io.IOException".into())
                    && b == &JavaType::Class("java.lang.Exception".into()))
        }
    }
    impl ThrowsHierarchy for Hierarchy {
        fn exception_kind(&self, n: &str) -> Option<ExceptionKind> {
            match n {
                "java.io.IOException" | "java.lang.Exception" => Some(ExceptionKind::Checked),
                "java.lang.RuntimeException" => Some(ExceptionKind::Unchecked),
                _ => None,
            }
        }
    }
    fn key(i: usize) -> MethodKey {
        MethodKey {
            owner: "LTest;".into(),
            name: format!("m{i:04}"),
            parameters: vec![],
            return_type: "V".into(),
        }
    }
    fn body(i: usize) -> MethodRequirements {
        MethodRequirements {
            identity: OriginalMethodIdentity {
                dex: 0,
                index: i as u32,
                key: key(i),
            },
            original: vec![],
            flags: 2,
            overrides: OverrideContract::Unknown,
            events: vec![],
        }
    }
    fn call(target: MethodKey) -> Event {
        Event::Invoke(
            InvocationContract {
                source_target: Some(target),
                declared: vec![],
            },
            Coverage {
                catches: vec![],
                all: false,
            },
        )
    }
    fn throwing() -> Event {
        Event::Throw(
            "java.io.IOException".into(),
            Coverage {
                catches: vec![],
                all: false,
            },
        )
    }
    #[test]
    fn borrowed_graph_matches_owned_reference() {
        fn parity(methods: &[MethodRequirements]) {
            // Debug includes all owned identities, original/derived/source domains,
            // runtime domains, and the exact selected errors in BTree order.
            assert_eq!(
                format!("{:?}", derive_program(methods, &Hierarchy)),
                format!("{:?}", derive_program_owned_reference(methods, &Hierarchy)),
            );
        }
        parity(&[]);
        parity(&[body(0), body(0)]);
        for seed in 0..64 {
            let mut methods = (0..12).map(body).collect::<Vec<_>>();
            for (index, method) in methods.iter_mut().enumerate() {
                method.events.push(call(key((index * 7 + seed) % 14)));
                if (index + seed) % 3 == 0 {
                    method.events.push(throwing());
                }
                if (index + seed) % 5 == 0 {
                    method.original.push("java.lang.Exception".into());
                }
                if (index + seed) % 7 == 0 {
                    method.flags = 1;
                    let target = key((index + seed) % 14);
                    method.overrides = OverrideContract::Declarations(vec![OverrideDeclaration {
                        source_target: Some(target.clone()),
                        key: target,
                        throws: vec![],
                    }]);
                }
            }
            parity(&methods);
            methods.reverse();
            parity(&methods);
        }
        let mut a = body(0);
        let mut b = body(1);
        a.events = vec![throwing(), call(key(1))];
        b.events = vec![call(key(0))];
        parity(&[a.clone(), b.clone()]);
        a.overrides = OverrideContract::Declarations(vec![OverrideDeclaration {
            source_target: Some(key(1)),
            key: key(1),
            throws: vec![],
        }]);
        b.overrides = OverrideContract::Declarations(vec![OverrideDeclaration {
            source_target: Some(key(0)),
            key: key(0),
            throws: vec![],
        }]);
        parity(&[a.clone(), b.clone()]);
        a.overrides = OverrideContract::Declarations(vec![OverrideDeclaration {
            source_target: Some(key(1)),
            key: key(8),
            throws: vec![],
        }]);
        parity(&[a, b]);
    }
    #[test]
    fn long_chain_and_recursive_component_reach_fixed_point() {
        let mut methods = (0..2100).map(body).collect::<Vec<_>>();
        for (i, m) in methods.iter_mut().enumerate().take(2099) {
            m.events.push(call(key(i + 1)));
        }
        methods[2099].events = vec![throwing(), call(key(2098))];
        let p = derive_program(&methods, &Hierarchy).unwrap();
        assert!(p.unavailable.is_empty());
        assert_eq!(p.methods.len(), 2100);
        for m in p.methods.values() {
            assert_eq!(m.derived, vec!["java.io.IOException"]);
            assert!(m.original.is_empty());
        }
    }
    #[test]
    fn override_failure_invalidates_callers_and_recursive_peers() {
        let mut bad = body(2);
        bad.flags = 1;
        bad.events = vec![throwing(), call(key(1))];
        bad.overrides = OverrideContract::Declarations(vec![OverrideDeclaration {
            source_target: None,
            key: key(2),
            throws: vec![],
        }]);
        let mut a = body(0);
        a.events = vec![call(key(1))];
        let mut b = body(1);
        b.events = vec![call(key(2))];
        let p = derive_program(&[a, b, bad], &Hierarchy).unwrap();
        assert!(p.methods.is_empty());
        assert_eq!(p.unavailable.len(), 3);
        assert_eq!(p.runtime.len(), 3);
        for facts in p.runtime.values() {
            assert!(facts.original.is_empty());
            assert_eq!(facts.escaping, vec!["java.io.IOException"]);
        }
        assert!(matches!(
            p.unavailable[&key(2)],
            ThrowsError::OverrideConflict(_)
        ));
    }
    #[test]
    fn invalid_clinit_and_unknown_hierarchy_poison_cycles() {
        for unknown in [false, true] {
            let mut a = body(0);
            let mut b = body(1);
            if !unknown {
                a.identity.key.name = "<clinit>".into();
            }
            a.events = vec![
                Event::Throw(
                    if unknown {
                        "missing.Exception"
                    } else {
                        "java.io.IOException"
                    }
                    .into(),
                    Coverage {
                        catches: vec![],
                        all: false,
                    },
                ),
                call(b.identity.key.clone()),
            ];
            b.events = vec![call(a.identity.key.clone())];
            let p = derive_program(&[a, b], &Hierarchy).unwrap();
            assert!(p.methods.is_empty());
            assert_eq!(p.unavailable.len(), 2);
        }
    }
    #[test]
    fn inherited_contract_and_exact_catch_preserve_original_metadata() {
        let mut m = body(0);
        m.flags = 1;
        m.overrides = OverrideContract::Declarations(vec![OverrideDeclaration {
            source_target: None,
            key: key(0),
            throws: vec!["java.lang.Exception".into()],
        }]);
        m.events = vec![throwing()];
        let p = derive_program(&[m.clone()], &Hierarchy).unwrap();
        assert_eq!(p.methods[&key(0)].derived, vec!["java.io.IOException"]);
        assert!(m.original.is_empty());
        m.events = vec![Event::Throw(
            "java.io.IOException".into(),
            Coverage {
                catches: vec!["java.lang.Exception".into()],
                all: false,
            },
        )];
        let p = derive_program(&[m], &Hierarchy).unwrap();
        assert!(p.methods[&key(0)].derived.is_empty());
    }
    #[test]
    fn derived_contract_compiles_and_preserves_checked_exception_runtime() {
        let mut leaf = body(0);
        leaf.events = vec![throwing()];
        let mut caller = body(1);
        caller.events = vec![call(key(0))];
        let plan = derive_program(&[leaf, caller], &Hierarchy).unwrap();
        let clause = |i| plan.methods[&key(i)].source.join(", ");
        let dir =
            std::env::temp_dir().join(format!("decomphify-source-throws-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Explicit semantic fixture; this is not a reconstructed Samsung body.
        let source = format!(
            "public class RequiredThrowsFixture {{ private static void leaf() throws {} {{ throw new java.io.IOException(\"expected\"); }} private static void caller() throws {} {{ leaf(); }} public static void main(String[] a) {{ try {{ caller(); throw new AssertionError(); }} catch(java.io.IOException e) {{ if(!\"expected\".equals(e.getMessage())) throw new AssertionError(); }} }} }}",
            clause(0),
            clause(1)
        );
        let path = dir.join("RequiredThrowsFixture.java");
        std::fs::write(&path, source).unwrap();
        let java_home = std::env::var_os("JAVA_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11"));
        let compile = std::process::Command::new(java_home.join("bin/javac"))
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let run = std::process::Command::new(java_home.join("bin/java"))
            .arg("-cp")
            .arg(&dir)
            .arg("RequiredThrowsFixture")
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
    #[test]
    fn approved_input_ancestor_source_contract_allows_override_without_mutating_raw() {
        let mut ancestor = body(0);
        ancestor.identity.key.name = "parse".into();
        ancestor.flags = 1;
        ancestor.overrides = OverrideContract::NoInheritedDeclaration;
        ancestor.events = vec![throwing()];
        let mut child = body(1);
        child.identity.key.owner = "LChild;".into();
        child.identity.key.name = "parse".into();
        child.flags = 1;
        child.events = vec![throwing()];
        child.overrides = OverrideContract::Declarations(vec![OverrideDeclaration {
            key: ancestor.identity.key.clone(),
            source_target: Some(ancestor.identity.key.clone()),
            throws: vec![],
        }]);
        let p = derive_program(&[ancestor.clone(), child.clone()], &Hierarchy).unwrap();
        assert_eq!(p.methods.len(), 2);
        assert!(p.unavailable.is_empty());
        assert!(p.methods[&child.identity.key].original.is_empty());
        assert_eq!(
            p.methods[&child.identity.key].derived,
            vec!["java.io.IOException"]
        );
        ancestor.overrides = OverrideContract::Declarations(vec![OverrideDeclaration {
            key: ancestor.identity.key.clone(),
            source_target: None,
            throws: vec![],
        }]);
        let p = derive_program(&[ancestor, child.clone()], &Hierarchy).unwrap();
        assert!(p.methods.is_empty());
        assert!(matches!(
            p.unavailable[&child.identity.key],
            ThrowsError::MissingSourceTarget(_)
        ));
    }
    #[test]
    fn inherited_source_cycles_and_missing_ancestors_never_approve() {
        let mut a = body(0);
        let mut b = body(1);
        a.identity.key.name = "same".into();
        b.identity.key.name = "same".into();
        b.identity.key.owner = "LChild;".into();
        a.overrides = OverrideContract::Declarations(vec![OverrideDeclaration {
            key: b.identity.key.clone(),
            source_target: Some(b.identity.key.clone()),
            throws: vec![],
        }]);
        b.overrides = OverrideContract::Declarations(vec![OverrideDeclaration {
            key: a.identity.key.clone(),
            source_target: Some(a.identity.key.clone()),
            throws: vec![],
        }]);
        let p = derive_program(&[a.clone(), b], &Hierarchy).unwrap();
        assert!(p.methods.is_empty());
        assert_eq!(p.unavailable.len(), 2);
        let p = derive_program(&[a], &Hierarchy).unwrap();
        assert!(p.methods.is_empty());
        assert_eq!(p.unavailable.len(), 1);
    }
    #[test]
    fn swallowed_catch_all_call_does_not_need_callee_source_domain() {
        let mut caller = body(0);
        caller.events = vec![Event::Invoke(
            InvocationContract {
                source_target: Some(key(1)),
                declared: vec!["unknown.checked.Declaration".into()],
            },
            Coverage {
                catches: vec![],
                all: true,
            },
        )];
        assert_eq!(caller.source_targets().count(), 0);
        let plan = derive_program(&[caller.clone()], &Hierarchy).unwrap();
        assert!(plan.methods[&key(0)].source.is_empty());
        let mut unavailable = body(1);
        unavailable.events = vec![Event::Throw(
            "unknown.Checked".into(),
            Coverage {
                catches: vec![],
                all: false,
            },
        )];
        let plan = derive_program(&[caller.clone(), unavailable], &Hierarchy).unwrap();
        assert!(plan.unavailable.contains_key(&key(1)));
        assert!(plan.methods.contains_key(&key(0)));
        caller.events.push(throwing());
        let plan = derive_program(&[caller], &Hierarchy).unwrap();
        assert_eq!(plan.methods[&key(0)].derived, ["java.io.IOException"]);
    }
    #[test]
    fn partial_catch_and_captured_rethrow_keep_exact_callee_dependency() {
        let mut caller = body(0);
        caller.events = vec![call(key(1))];
        let Event::Invoke(_, coverage) = &mut caller.events[0] else {
            unreachable!()
        };
        coverage.catches = vec!["java.io.IOException".into()];
        assert_eq!(caller.source_targets().count(), 1);
        assert!(
            derive_program(&[caller.clone()], &Hierarchy)
                .unwrap()
                .unavailable
                .contains_key(&key(0))
        );
        let Event::Invoke(_, coverage) = &mut caller.events[0] else {
            unreachable!()
        };
        coverage.all = true;
        caller.events.push(Event::Rethrow(
            vec![RethrowOrigin {
                source: 0,
                caught: None,
                excluded: vec![],
            }],
            Coverage {
                catches: vec![],
                all: false,
            },
        ));
        assert_eq!(caller.source_targets().count(), 1);
        assert!(
            derive_program(&[caller], &Hierarchy)
                .unwrap()
                .unavailable
                .contains_key(&key(0))
        );
    }
}

#[cfg(test)]
#[path = "java_required_throws_abstract_tests.rs"]
mod abstract_tests;

#[cfg(test)]
#[path = "java_required_throws_concrete_tests.rs"]
mod concrete_tests;
