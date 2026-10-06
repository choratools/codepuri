//! Source-only checked-throws evidence. Raw DEX declaration metadata is immutable.
use super::*;
use managed_ir::types::ManagedType;
#[path = "java_required_throws.rs"]
mod required;
#[cfg(test)]
#[path = "java_source_throws_wave7_probe.rs"]
mod wave7_probe;
pub(super) use required::SourceThrowsPlan;
use required::{
    DerivedThrows, ExceptionKind, InvocationContract, OriginalMethodIdentity, OverrideContract,
    OverrideDeclaration, RequiredThrowsContext, ThrowsHierarchy,
};

/// Opt-in stage wall times and summed body work (parallel sums are not wall time).
#[derive(Debug, Default)]
pub(super) struct SourceThrowsProfile {
    pub walls: BTreeMap<&'static str, std::time::Duration>,
    pub ssa_time: std::time::Duration,
    pub inspect_method_time: std::time::Duration,
    pub ssa_count: usize,
    pub inspect_method_count: usize,
    pub task_count: usize,
    pub scheduler: Option<scheduler::SchedulerProfile>,
}

#[derive(Default)]
pub(super) struct ThrowsTaskProfile {
    ssa_time: std::time::Duration,
    inspect_method_time: std::time::Duration,
    ssa_count: usize,
    inspect_method_count: usize,
}

/// Delivery-ordered aggregation state for the parallel collection scheduler.
/// Each ordered result is consumed the moment it completes, keyed directly by
/// its ordinal in the initial batch (delivery is strictly in ordinal order and
/// initial entries are unique). Duplicate traversal entries never consume a
/// result; discovered bodies outside the batch are inspected after the run.
pub(super) struct StreamedAggregation<E> {
    discovered: VecDeque<MethodKey>,
    visited: BTreeSet<MethodKey>,
    requirements: Vec<required::MethodRequirements>,
    missing: BTreeMap<MethodKey, E>,
    task_count: usize,
    timing: ThrowsTaskProfile,
}

impl<E> StreamedAggregation<E> {
    fn deliver(
        &mut self,
        key: &MethodKey,
        value: (Result<required::MethodRequirements, E>, ThrowsTaskProfile),
    ) {
        // An initial-batch key already consumed as a discovered target keeps
        // its delivered result unconsumed, exactly like the visited front pops.
        if !self.visited.insert(key.clone()) {
            return;
        }
        self.task_count += 1;
        let timing = value.1;
        self.timing.ssa_time += timing.ssa_time;
        self.timing.inspect_method_time += timing.inspect_method_time;
        self.timing.ssa_count += timing.ssa_count;
        self.timing.inspect_method_count += timing.inspect_method_count;
        match value.0 {
            Ok(requirements) => {
                self.discovered
                    .extend(requirements.source_targets().cloned());
                self.requirements.push(requirements);
            }
            Err(error) => {
                self.missing.insert(key.clone(), error);
            }
        }
        while self
            .discovered
            .front()
            .is_some_and(|key| self.visited.contains(key))
        {
            self.discovered.pop_front();
        }
    }
}

#[derive(Clone)]
struct RawClass {
    parent: Option<String>,
    interfaces: Vec<String>,
    methods: Vec<(MethodKey, MethodMetadata)>,
}
/// Original class-super dispatch evidence, independent of emitted Java names
/// and overload representability. Full descriptors select DEX methods.
#[derive(Clone)]
struct OriginalSuperClass {
    flags: u32,
    parent: Option<String>,
    methods: Vec<(MethodKey, MethodMetadata)>,
}
fn original_super_target(
    caller: &str,
    reference: &MethodKey,
    class: impl Fn(&str) -> Option<OriginalSuperClass>,
) -> Option<(MethodKey, MethodMetadata)> {
    if reference.name.starts_with('<') || reference.owner == caller {
        return None;
    }
    let current = class(caller)?;
    if current.flags & 0x200 != 0 {
        return None;
    }
    let mut next = current.parent;
    let mut seen = BTreeSet::from([caller.to_owned()]);
    let mut ancestry = Vec::new();
    let mut owner_seen = false;
    let mut symbolic_resolved = false;
    let mut selected = None;
    while let Some(owner) = next {
        if !seen.insert(owner.clone()) {
            return None;
        }
        let declaration = class(&owner)?;
        if declaration.flags & 0x200 != 0 {
            return None;
        }
        ancestry.push(owner.clone());
        owner_seen |= owner == reference.owner;
        let matching = declaration
            .methods
            .iter()
            .filter(|(key, _)| {
                key.name == reference.name
                    && key.parameters == reference.parameters
                    && key.return_type == reference.return_type
            })
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return None;
        }
        if let Some((key, metadata)) = matching.first().copied() {
            if key.owner != owner
                || metadata.access_flags & (2 | 8) != 0
                || (metadata.access_flags & 7).count_ones() > 1
                || metadata.exceptions.is_none()
            {
                return None;
            }
            if owner_seen {
                if !original_super_access(caller, key, metadata.access_flags, &ancestry) {
                    return None;
                }
                symbolic_resolved = true;
            }
            if selected.is_none() {
                if metadata.access_flags & 0x400 != 0 {
                    return None;
                }
                selected = Some((key.clone(), metadata.clone()));
            }
        }
        if owner_seen && symbolic_resolved && selected.is_some() {
            break;
        }
        next = declaration.parent;
    }
    if !owner_seen || !symbolic_resolved {
        return None;
    }
    let (key, metadata) = selected?;
    if !original_super_access(caller, &key, metadata.access_flags, &ancestry) {
        return None;
    }
    Some((key, metadata))
}
fn original_super_access(caller: &str, key: &MethodKey, flags: u32, ancestry: &[String]) -> bool {
    // Protected access is justified by the proved strict class ancestry.
    // Package members must survive every intervening superclass and be
    // accessible to the original caller. Java receiver/emission checks remain
    // independent prerequisites for publication of an executable body.
    flags & 7 != 0
        || (package(caller) == package(&key.owner)
            && ancestry
                .iter()
                .take_while(|owner| *owner != &key.owner)
                .all(|owner| package(owner) == package(&key.owner)))
}
/// Immutable original-input links, scoped to one source-contract collection.
/// Complete override/alias/bounds proofs still run when each link is consumed.
struct ImplementationIndex {
    links: BTreeMap<MethodKey, Vec<MethodKey>>,
    #[cfg(test)]
    construction: IndexConstruction,
}
#[cfg(test)]
#[derive(Default)]
struct IndexConstruction {
    cached_owners: usize,
    cached_ancestor_names: usize,
    ancestor_payload_bytes: usize,
}
impl ImplementationIndex {
    fn build(
        abstracts: &BTreeSet<MethodKey>,
        candidates: &BTreeMap<(String, Vec<String>), Vec<MethodKey>>,
        parents: impl Fn(&str) -> Vec<String>,
    ) -> Self {
        let mut by_owner = BTreeMap::<&str, BTreeMap<(&str, &[String]), Vec<&MethodKey>>>::new();
        for key in abstracts {
            by_owner
                .entry(&key.owner)
                .or_default()
                .entry((&key.name, &key.parameters))
                .or_default()
                .push(key);
        }
        let mut ancestors = BTreeMap::<&str, BTreeSet<String>>::new();
        let mut links = BTreeMap::<MethodKey, Vec<MethodKey>>::new();
        for ((name, parameters), methods) in candidates {
            for candidate in methods {
                // Cache only the completed owner traversal. Recursive partial
                // closures depend on the active path in cyclic input graphs.
                let closure = ancestors.entry(&candidate.owner).or_insert_with(|| {
                    let mut queue = vec![candidate.owner.clone()];
                    let mut seen = BTreeSet::new();
                    while let Some(owner) = queue.pop() {
                        if seen.insert(owner.clone()) {
                            queue.extend(parents(&owner));
                        }
                    }
                    seen
                });
                for owner in closure.iter() {
                    if let Some(targets) = by_owner.get(owner.as_str()).and_then(|signatures| {
                        signatures.get(&(name.as_str(), parameters.as_slice()))
                    }) {
                        for target in targets {
                            // Each abstract belongs to one signature bucket;
                            // appending preserves the original candidate order,
                            // including distinct return types and duplicates.
                            links
                                .entry((*target).clone())
                                .or_default()
                                .push(candidate.clone());
                        }
                    }
                }
            }
        }
        #[cfg(test)]
        let construction = IndexConstruction {
            cached_owners: ancestors.len(),
            cached_ancestor_names: ancestors.values().map(BTreeSet::len).sum(),
            ancestor_payload_bytes: ancestors
                .values()
                .flat_map(|s| s.iter())
                .map(|name| std::mem::size_of::<String>() + name.capacity())
                .sum(),
        };
        Self {
            links,
            #[cfg(test)]
            construction,
        }
    }
    fn get(&self, abstract_key: &MethodKey) -> &[MethodKey] {
        self.links
            .get(abstract_key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}
// Stop at external, duplicate or missing input owners, exactly as the original
// target-directed traversal did. The encountered owner itself remains visible.
fn input_parents(
    files: &[DexFile],
    owners: &BTreeMap<String, Option<(usize, usize)>>,
    owner: &str,
) -> Vec<String> {
    let Some(Some((pool, index))) = owners.get(owner) else {
        return vec![];
    };
    let dex = &files[*pool];
    let class = &dex.classes[*index];
    let mut parents = Vec::new();
    if class.superclass_idx != u32::MAX
        && let Some(parent) = descriptor(dex, class.superclass_idx)
    {
        parents.push(parent.into());
    }
    parents.extend(
        class
            .interfaces
            .as_ref()
            .into_iter()
            .flat_map(|v| &v.types)
            .filter_map(|i| descriptor(dex, u32::from(*i)).map(str::to_owned)),
    );
    parents
}
pub(super) struct Facts<'a> {
    files: &'a [DexFile],
    aliases: &'a JavaAliases,
    catalog: &'a BTreeMap<MethodKey, Option<MethodMetadata>>,
    external: Option<&'a dyn ExternalMetadata>,
    owners: BTreeMap<String, Option<(usize, usize)>>,
    bodies: BTreeSet<MethodKey>,
    abstracts: BTreeSet<MethodKey>,
    concrete_sources: BTreeSet<MethodKey>,
    implementations: ImplementationIndex,
    concrete_proofs: Mutex<BTreeMap<MethodKey, BTreeSet<MethodKey>>>,
    definitions: BTreeMap<MethodKey, (usize, &'a EncodedMethod)>,
    originals: BTreeMap<String, String>,
    classes: Mutex<BTreeMap<String, Option<Arc<RawClass>>>>,
    // Export-local immutable evidence; never cache mutable concrete proof state.
    kinds: Mutex<BTreeMap<String, Option<ExceptionKind>>>,
    override_contracts: Mutex<BTreeMap<(usize, u32, MethodKey), OverrideContract>>,
}
/// Per-input evidence scan results; merged in pool order by [`Facts::new`].
struct PartialFacts<'a> {
    owners: BTreeMap<String, Option<(usize, usize)>>,
    bodies: BTreeSet<MethodKey>,
    abstracts: BTreeSet<MethodKey>,
    concrete_sources: BTreeSet<MethodKey>,
    implementations: BTreeMap<(String, Vec<String>), Vec<MethodKey>>,
    definitions: BTreeMap<MethodKey, (usize, &'a EncodedMethod)>,
    originals: BTreeMap<String, String>,
}
fn scan_pool_facts<'a>(
    pool: usize,
    dex: &'a DexFile,
    aliases: &JavaAliases,
    catalog: &BTreeMap<MethodKey, Option<MethodMetadata>>,
) -> PartialFacts<'a> {
    let mut owners = BTreeMap::new();
    let mut bodies = BTreeSet::new();
    let mut abstracts = BTreeSet::new();
    let mut concrete_sources = BTreeSet::new();
    let mut implementations = BTreeMap::<(String, Vec<String>), Vec<MethodKey>>::new();
    let mut definitions = BTreeMap::new();
    let mut originals = BTreeMap::new();
    for (index, class) in dex.classes.iter().enumerate() {
        if let Some(owner) = descriptor(dex, class.class_idx) {
            owners
                .entry(owner.into())
                .and_modify(|v| *v = None)
                .or_insert(Some((pool, index)));
        }
        if let Some(data) = &class.class_data {
            for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                let abstract_declaration = method.code.is_none()
                    && method.access_flags & 0x400 != 0
                    && method.access_flags & (0x100 | 8 | 2) == 0;
                if (method.code.is_some() || abstract_declaration)
                    && let Some(key) = method_key(dex, method.method_idx)
                {
                    definitions.insert(key.clone(), (pool, method));
                    if abstract_declaration {
                        abstracts.insert(key);
                    } else {
                        if method.access_flags & (0x400 | 0x100 | 8 | 2) == 0
                            && !key.name.starts_with('<')
                        {
                            implementations
                                .entry((key.name.clone(), key.parameters.clone()))
                                .or_default()
                                .push(key.clone());
                        }
                        if concrete_source_reconstructible(dex, class, method)
                            && catalog.get(&key).is_some_and(|metadata| {
                                metadata.as_ref().is_some_and(|m| {
                                    m.access_flags == method.access_flags
                                        && m.exceptions.as_ref().is_some_and(Vec::is_empty)
                                })
                            })
                        {
                            concrete_sources.insert(key.clone());
                        }
                        bodies.insert(key);
                    }
                }
            }
        }
    }
    for ty in &dex.type_ids {
        if let Ok(raw) = dex.string(ty.descriptor_idx) {
            let raw = raw.to_string();
            if let Some(name) = aliases.class_name(&raw) {
                originals.insert(name.into(), raw);
            }
        }
    }
    PartialFacts {
        owners,
        bodies,
        abstracts,
        concrete_sources,
        implementations,
        definitions,
        originals,
    }
}

impl<'a> Facts<'a> {
    fn new(
        files: &'a [DexFile],
        aliases: &'a JavaAliases,
        catalog: &'a BTreeMap<MethodKey, Option<MethodMetadata>>,
        external: Option<&'a dyn ExternalMetadata>,
    ) -> Self {
        // Per-input scans are independent; run them concurrently and merge in
        // pool order so duplicate-owner evidence keeps its original precedence.
        let partials: Vec<PartialFacts<'a>> = std::thread::scope(|scope| {
            let workers: Vec<_> = files
                .iter()
                .enumerate()
                .map(|(pool, dex)| {
                    scope.spawn(move || scan_pool_facts(pool, dex, aliases, catalog))
                })
                .collect();
            let mut partials = Vec::with_capacity(workers.len());
            for worker in workers {
                match worker.join() {
                    Ok(partial) => partials.push(partial),
                    Err(payload) => std::panic::resume_unwind(payload),
                }
            }
            partials
        });
        let mut owners = BTreeMap::new();
        let mut bodies = BTreeSet::new();
        let mut abstracts = BTreeSet::new();
        let mut concrete_sources = BTreeSet::new();
        let mut implementations = BTreeMap::<(String, Vec<String>), Vec<MethodKey>>::new();
        let mut definitions = BTreeMap::new();
        let mut originals = BTreeMap::new();
        for partial in partials {
            for (owner, location) in partial.owners {
                match owners.entry(owner) {
                    std::collections::btree_map::Entry::Occupied(mut occupied) => {
                        *occupied.get_mut() = None;
                    }
                    std::collections::btree_map::Entry::Vacant(vacant) => {
                        vacant.insert(location);
                    }
                }
            }
            definitions.extend(partial.definitions);
            bodies.extend(partial.bodies);
            abstracts.extend(partial.abstracts);
            concrete_sources.extend(partial.concrete_sources);
            originals.extend(partial.originals);
            for (signature, mut keys) in partial.implementations {
                implementations
                    .entry(signature)
                    .or_default()
                    .append(&mut keys);
            }
        }
        if let Some(external) = external {
            for declaration in external.hierarchy() {
                let raw = format!("L{};", declaration.name.replace('.', "/"));
                let name = aliases
                    .class_name(&raw)
                    .unwrap_or(&declaration.name)
                    .to_owned();
                originals.entry(name).or_insert(raw);
            }
        }
        let domains = abstracts.union(&concrete_sources).cloned().collect();
        let implementations = ImplementationIndex::build(&domains, &implementations, |owner| {
            input_parents(files, &owners, owner)
        });
        Self {
            files,
            aliases,
            catalog,
            external,
            owners,
            bodies,
            abstracts,
            concrete_sources,
            implementations,
            concrete_proofs: Mutex::new(BTreeMap::new()),
            definitions,
            originals,
            classes: Mutex::new(BTreeMap::new()),
            kinds: Mutex::new(BTreeMap::new()),
            override_contracts: Mutex::new(BTreeMap::new()),
        }
    }
    fn emitted_method_name(&self, key: &MethodKey) -> Option<String> {
        match self.aliases.method_name(key) {
            Some(crate::java_aliases::MethodAlias::Ordinary(name)) => Some(name.clone()),
            Some(_) => None,
            None if !self.owners.contains_key(&key.owner) => Some(key.name.clone()),
            None => None,
        }
    }
    fn original_super_class(&self, owner: &str) -> Option<OriginalSuperClass> {
        let class = self.class(owner)?;
        let flags = if let Some(location) = self.owners.get(owner) {
            let (pool, index) = (*location)?;
            self.files[pool].classes[index].access_flags
        } else {
            self.external?.class_access(owner)?
        };
        Some(OriginalSuperClass {
            flags,
            parent: class.parent.clone(),
            methods: class.methods.clone(),
        })
    }
    fn names(&self, descriptors: &[String]) -> Option<Vec<String>> {
        descriptors
            .iter()
            .map(|raw| {
                self.aliases.class_name(raw).map(str::to_owned).or_else(|| {
                    match crate::java_descriptors::descriptor_type(raw)? {
                        JavaType::Class(name) => Some(name),
                        _ => None,
                    }
                })
            })
            .collect()
    }
    fn class(&self, owner: &str) -> Option<Arc<RawClass>> {
        if let Some(cached) = self.classes.lock().ok()?.get(owner) {
            return cached.clone();
        }
        let result = if let Some(location) = self.owners.get(owner) {
            let (pool, index) = (*location)?;
            let dex = &self.files[pool];
            let class = &dex.classes[index];
            if (class.class_data_off != 0 && class.class_data.is_none())
                || (class.interfaces_off != 0 && class.interfaces.is_none())
            {
                return None;
            }
            let parent = if class.superclass_idx == u32::MAX {
                if owner != "Ljava/lang/Object;" && class.access_flags & 0x200 == 0 {
                    return None;
                }
                None
            } else {
                Some(descriptor(dex, class.superclass_idx)?.to_owned())
            };
            let interfaces: Vec<String> = class
                .interfaces
                .as_ref()
                .into_iter()
                .flat_map(|v| &v.types)
                .map(|i| descriptor(dex, u32::from(*i)).map(str::to_owned))
                .collect::<Option<_>>()?;
            if parent.as_deref() == Some(owner) || interfaces.iter().any(|i| i == owner) {
                return None;
            }
            let methods = class
                .class_data
                .as_ref()
                .into_iter()
                .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
                .map(|method| {
                    let key = method_key(dex, method.method_idx)?;
                    Some((key.clone(), self.catalog.get(&key)?.clone()?))
                })
                .collect::<Option<_>>()?;
            Some(RawClass {
                parent,
                interfaces,
                methods,
            })
        } else {
            let provider = self.external?;
            provider.class_access(owner)?;
            let declarations = provider.class_hierarchy(owner);
            let [declaration] = declarations.as_slice() else {
                return None;
            };
            let methods = provider
                .declared_methods(owner)?
                .into_iter()
                .map(|m| (m.key, m.metadata))
                .collect();
            Some(RawClass {
                parent: declaration
                    .superclass
                    .as_ref()
                    .map(|n| format!("L{};", n.replace('.', "/"))),
                interfaces: declaration
                    .interfaces
                    .iter()
                    .map(|n| format!("L{};", n.replace('.', "/")))
                    .collect(),
                methods,
            })
        };
        // Completed immutable metadata is shared across consumers. A cache hit
        // must not clone every unrelated method and checked contract.
        let result = result.map(Arc::new);
        self.classes
            .lock()
            .ok()?
            .insert(owner.into(), result.clone());
        result
    }
    fn kind(&self, name: &str) -> Option<ExceptionKind> {
        if let Ok(cache) = self.kinds.lock()
            && let Some(result) = cache.get(name)
        {
            return *result;
        }
        let result = self.kind_uncached(name);
        if let Ok(mut cache) = self.kinds.lock() {
            cache.insert(name.to_owned(), result);
        }
        result
    }
    fn kind_uncached(&self, name: &str) -> Option<ExceptionKind> {
        let mut owner = self
            .originals
            .get(name)
            .cloned()
            .unwrap_or_else(|| format!("L{};", name.replace('.', "/")));
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(owner.clone()) {
                return None;
            }
            if matches!(
                owner.as_str(),
                "Ljava/lang/RuntimeException;" | "Ljava/lang/Error;"
            ) {
                // This name must itself have a genuine class declaration.
                self.class(&owner)?;
                return Some(ExceptionKind::Unchecked);
            }
            if owner == "Ljava/lang/Throwable;" {
                self.class(&owner)?;
                return Some(ExceptionKind::Checked);
            }
            owner = self.class(&owner)?.parent.clone()?;
        }
    }
}
// Only the complete absence of an original Throws annotation permits this
// joint source reconstruction. Explicit empty/nonempty annotations stay fixed.
fn concrete_source_reconstructible(
    dex: &DexFile,
    class: &ClassDef,
    method: &EncodedMethod,
) -> bool {
    if class.access_flags & (0x10 | 0x200) != 0
        || method.code.is_none()
        || method.access_flags & (2 | 8 | 0x10 | 0x100 | 0x400) != 0
        || method_key(dex, method.method_idx).is_none_or(|key| key.name.starts_with('<'))
        || class.class_data.as_ref().is_none_or(|data| {
            data.virtual_methods
                .iter()
                .filter(|m| m.method_idx == method.method_idx)
                .count()
                != 1
                || data
                    .direct_methods
                    .iter()
                    .any(|m| m.method_idx == method.method_idx)
        })
        || (class.annotations_off != 0 && class.annotations.is_none())
    {
        return false;
    }
    let Some(directory) = &class.annotations else {
        return true;
    };
    let members = directory
        .method_annotations
        .iter()
        .filter(|m| m.member_idx == method.method_idx)
        .collect::<Vec<_>>();
    if members.len() > 1 {
        return false;
    }
    for member in members {
        for item in &member.annotations.entries {
            let Some(name) = descriptor(dex, item.annotation.type_idx) else {
                return false;
            };
            if name == "Ldalvik/annotation/Throws;" {
                return false;
            }
        }
    }
    true
}
struct Context<'a, 'b> {
    resolver: &'a Resolver<'b>,
    facts: &'a Facts<'b>,
}
// Input ancestry and a raw name/parameter match are only a candidate link.
// Return-only DEX overloads can be separate emitted Java methods, including
// a renamed typed implementation alongside the actual Object-return bridge.
// Exclude only a positive contradiction; unavailable evidence still reaches
// the complete inherited-contract validator below.
fn proven_nonimplementation(
    candidate_name: Option<&str>,
    declaration_name: Option<&str>,
    candidate_return: Option<&JavaType>,
    declaration_return: Option<&JavaType>,
    hierarchy: &crate::type_compatibility::Hierarchy,
) -> bool {
    if let (Some(candidate), Some(declaration)) = (candidate_name, declaration_name)
        && candidate != declaration
    {
        return true;
    }
    let (Some(actual), Some(expected)) = (candidate_return, declaration_return) else {
        return false;
    };
    if actual == expected {
        return false;
    }
    match (actual, expected) {
        (JavaType::Class(_) | JavaType::Array(_), JavaType::Class(_) | JavaType::Array(_)) => {
            hierarchy.assignable(actual, expected)
                == crate::type_compatibility::Compatibility::Incompatible
        }
        // Override return substitutability permits neither primitive widening
        // nor a primitive/reference substitution (including void).
        _ => true,
    }
}
impl Context<'_, '_> {
    fn nonimplementation(&self, candidate: &MethodKey, declaration: &MethodKey) -> bool {
        let candidate_name = self.facts.emitted_method_name(candidate);
        let declaration_name = self.facts.emitted_method_name(declaration);
        if let (Some(candidate), Some(declaration)) = (&candidate_name, &declaration_name)
            && candidate != declaration
        {
            return true;
        }
        let mut actual = self.resolver.ty(&candidate.return_type);
        let mut expected = self.resolver.ty(&declaration.return_type);
        if matches!(&actual, Some(JavaType::Class(_) | JavaType::Array(_)))
            && matches!(&expected, Some(JavaType::Class(_) | JavaType::Array(_)))
        {
            // A known alias is not a declaration/hierarchy witness. Preserve
            // unknown and duplicate original owners, including array leaves.
            let known = |raw: &str| {
                let component = raw.trim_start_matches('[');
                !component.starts_with('L') || self.facts.class(component).is_some()
            };
            if !known(&candidate.return_type) || !known(&declaration.return_type) {
                actual = None;
                expected = None;
            }
        }
        proven_nonimplementation(
            candidate_name.as_deref(),
            declaration_name.as_deref(),
            actual.as_ref(),
            expected.as_ref(),
            &self.resolver.hierarchy,
        )
    }
}
impl JavaResolver for Context<'_, '_> {
    fn super_invocation(
        &self,
        caller: &str,
        reference: MethodRef,
    ) -> Option<backend_java::SuperInvocation> {
        self.resolver.super_invocation(caller, reference)
    }
    fn virtual_receiver_method(
        &self,
        reference: MethodRef,
        caller: &str,
        receiver: &JavaType,
    ) -> Option<JavaMethod> {
        self.resolver
            .virtual_receiver_method(reference, caller, receiver)
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        self.resolver.class(r)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        self.resolver.method(r)
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        self.resolver.method_access(r)
    }
    fn instruction_type(&self, r: managed_ir::refs::TypeRef) -> Option<JavaType> {
        self.resolver.instruction_type(r)
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        self.resolver.field(r)
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        self.resolver.string(r)
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        self.resolver.assignable(a, b)
    }
}
impl ThrowsHierarchy for Context<'_, '_> {
    fn exception_kind(&self, name: &str) -> Option<ExceptionKind> {
        self.facts.kind(name)
    }
}
impl RequiredThrowsContext for Context<'_, '_> {
    fn attests_concrete_override_domain(
        &self,
        identity: &OriginalMethodIdentity,
        implementations: &[MethodKey],
    ) -> bool {
        if !self.facts.concrete_sources.contains(&identity.key)
            || self
                .facts
                .definitions
                .get(&identity.key)
                .is_none_or(|(pool, method)| {
                    *pool != identity.dex || method.method_idx != identity.index
                })
        {
            return false;
        }
        self.facts
            .concrete_proofs
            .lock()
            .ok()
            .and_then(|proofs| {
                proofs.get(&identity.key).map(|proof| {
                    proof.len() == implementations.len()
                        && implementations.iter().all(|key| proof.contains(key))
                })
            })
            .unwrap_or(false)
    }
    fn receiver_invocation_contract(
        &self,
        caller: &OriginalMethodIdentity,
        kind: InvokeKind,
        reference: MethodRef,
        receiver: &ManagedType,
    ) -> Option<InvocationContract> {
        if kind != InvokeKind::Virtual {
            return None;
        }
        let receiver = super::managed_type_to_java(receiver, self.resolver)?;
        let caller = self.resolver.aliases.class_name(&caller.key.owner)?;
        let selected = self
            .resolver
            .virtual_receiver_method(reference, caller, &receiver)?;
        Some(InvocationContract {
            source_target: None,
            declared: selected.throws,
        })
    }
    fn array_clone_contract(
        &self,
        reference: MethodRef,
        receiver: &ManagedType,
    ) -> Option<InvocationContract> {
        let key = method_key(self.resolver.dex, reference.index())?;
        let receiver = super::managed_type_to_java(receiver, self.resolver)?;
        if crate::java_array_clone::is_array_clone_key(&key) {
            let expected = self.resolver.ty(&key.owner)?;
            if receiver != expected {
                return None;
            }
            return Some(InvocationContract {
                source_target: None,
                declared: vec![],
            });
        }
        if key.owner != "Ljava/lang/Object;"
            || key.name != "clone"
            || !key.parameters.is_empty()
            || key.return_type != "Ljava/lang/Object;"
            || self.facts.owners.contains_key(&key.owner)
        {
            return None;
        }
        // JLS 10.7: arrays implement a public clone with no checked throws.
        // Retain genuine Object metadata as the symbolic declaration witness;
        // the receiver proof selects the built-in array operation only here.
        let declaration = self.facts.external?.method(&key)?;
        if declaration.access_flags & (8 | 0x400) != 0
            || declaration.access_flags & (4 | 0x100) != (4 | 0x100)
            || self.resolver.declaration(reference.index()).ok()? != declaration
        {
            return None;
        }
        Some(InvocationContract {
            source_target: None,
            declared: vec![],
        })
    }
    fn invoked_contract(
        &self,
        caller: &OriginalMethodIdentity,
        kind: InvokeKind,
        reference: MethodRef,
    ) -> Option<InvocationContract> {
        if kind != InvokeKind::Super {
            return self.invocation_contract(reference);
        }
        let (pool, method) = self.facts.definitions.get(&caller.key)?;
        if *pool != caller.dex || method.method_idx != caller.index || method.access_flags & 8 != 0
        {
            return None;
        }
        let reference = method_key(self.resolver.dex, reference.index())?;
        let (selected, metadata) = original_super_target(&caller.key.owner, &reference, |owner| {
            self.facts.original_super_class(owner)
        })?;
        let declared = self.facts.names(metadata.exceptions.as_ref()?)?;
        let source_target = if self.facts.bodies.contains(&selected) {
            Some(selected)
        } else {
            if self.facts.owners.contains_key(&selected.owner)
                && metadata.access_flags & (0x100 | 0x400) == 0
            {
                return None;
            }
            None
        };
        Some(InvocationContract {
            source_target,
            declared,
        })
    }
    fn invocation_contract(&self, reference: MethodRef) -> Option<InvocationContract> {
        let key = method_key(self.resolver.dex, reference.index())?;
        let metadata = self.resolver.declaration(reference.index()).ok()?;
        let declared = self.facts.names(metadata.exceptions.as_ref()?)?;
        let source_target =
            if self.facts.bodies.contains(&key) || self.facts.abstracts.contains(&key) {
                Some(key)
            } else {
                if self.facts.owners.contains_key(&key.owner)
                    && metadata.access_flags & (0x100 | 0x400) == 0
                {
                    return None;
                }
                None
            };
        Some(InvocationContract {
            source_target,
            declared,
        })
    }
    fn override_contract(&self, identity: &OriginalMethodIdentity) -> OverrideContract {
        let key = (identity.dex, identity.index, identity.key.clone());
        if let Ok(cache) = self.facts.override_contracts.lock()
            && let Some(result) = cache.get(&key)
        {
            return result.clone();
        }
        // Do not hold the cache lock while walking classes or resolving aliases.
        let result = self.override_contract_uncached(identity);
        if let Ok(mut cache) = self.facts.override_contracts.lock() {
            cache.insert(key, result.clone());
        }
        result
    }
}
impl Context<'_, '_> {
    fn override_contract_uncached(&self, identity: &OriginalMethodIdentity) -> OverrideContract {
        // Constructors and private declarations do not override inherited
        // members. This fact comes from the exact original declaration flags,
        // not from source aliases or the calling instruction.
        if identity.key.name.starts_with('<')
            || self
                .resolver
                .declaration(identity.index)
                .is_ok_and(|m| m.access_flags & 2 != 0)
        {
            return OverrideContract::NoInheritedDeclaration;
        }
        let Some(current) = self.facts.class(&identity.key.owner) else {
            return OverrideContract::Unknown;
        };
        let mut todo = current
            .parent
            .clone()
            .into_iter()
            .chain(current.interfaces.iter().cloned())
            .map(|owner| (owner, BTreeSet::from([identity.key.owner.clone()])))
            .collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        let mut contracts = Vec::new();
        while let Some((owner, mut path)) = todo.pop() {
            if !path.insert(owner.clone()) {
                return OverrideContract::Unknown;
            }
            if !visited.insert(owner.clone()) {
                continue;
            }
            let Some(class) = self.facts.class(&owner) else {
                return OverrideContract::Unknown;
            };
            for (key, metadata) in &class.methods {
                if key.name != identity.key.name
                    || key.parameters != identity.key.parameters
                    || metadata.access_flags & 2 != 0
                {
                    continue;
                }
                if metadata.access_flags & (1 | 4) == 0
                    && package(&key.owner) != package(&identity.key.owner)
                {
                    continue;
                }
                let (Some(inherited_name), Some(current_name)) = (
                    self.facts.emitted_method_name(key),
                    self.facts.emitted_method_name(&identity.key),
                ) else {
                    return OverrideContract::Unknown;
                };
                // A frozen distinct source name proves this declaration is
                // not overridden by the current helper. Its actual bridge,
                // if any, must independently satisfy every inherited bound.
                if inherited_name != current_name {
                    continue;
                }
                if key.return_type != identity.key.return_type {
                    let Some(actual) = self.resolver.ty(&identity.key.return_type) else {
                        return OverrideContract::Unknown;
                    };
                    let Some(expected) = self.resolver.ty(&key.return_type) else {
                        return OverrideContract::Unknown;
                    };
                    if !self.resolver.assignable(&actual, &expected) {
                        return OverrideContract::Unknown;
                    }
                }
                let Some(raw) = &metadata.exceptions else {
                    return OverrideContract::Unknown;
                };
                let Some(throws) = self.facts.names(raw) else {
                    return OverrideContract::Unknown;
                };
                let source_target = (self.facts.owners.contains_key(&key.owner)
                    && (metadata.access_flags & (0x100 | 0x400) == 0
                        || self.facts.abstracts.contains(key)))
                .then(|| key.clone());
                contracts.push(OverrideDeclaration {
                    key: key.clone(),
                    throws,
                    source_target,
                });
            }
            todo.extend(
                class
                    .parent
                    .clone()
                    .into_iter()
                    .chain(class.interfaces.iter().cloned())
                    .map(|ancestor| (ancestor, path.clone())),
            );
        }
        if contracts.is_empty() {
            OverrideContract::NoInheritedDeclaration
        } else {
            OverrideContract::Declarations(contracts)
        }
    }
}
fn package(descriptor: &str) -> &str {
    descriptor.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
}

/// Scan one seeded SSA at a time; retain compact requirements, never every IR.
#[cfg(test)]
pub(super) fn collect(
    files: &[DexFile],
    aliases: &JavaAliases,
    catalog: &BTreeMap<MethodKey, Option<MethodMetadata>>,
    external: Option<&dyn ExternalMetadata>,
    hierarchy: &crate::type_compatibility::Hierarchy,
) -> SourceThrowsPlan {
    collect_roots(files, aliases, catalog, external, hierarchy, None)
}

/// Inspect independent initial bodies with bounded workers; source dependency
/// traversal and program derivation retain their original deterministic order.
#[cfg(test)]
pub(super) fn collect_with_workers(
    files: &[DexFile],
    aliases: &JavaAliases,
    catalog: &BTreeMap<MethodKey, Option<MethodMetadata>>,
    external: Option<&dyn ExternalMetadata>,
    hierarchy: &crate::type_compatibility::Hierarchy,
    workers: usize,
) -> SourceThrowsPlan {
    if workers <= 1 {
        return collect(files, aliases, catalog, external, hierarchy);
    }
    collect_roots_with_workers(
        files, aliases, catalog, external, hierarchy, None, workers, None,
    )
}
/// Retain seeded frontend results for later consumers in the same export.
#[cfg(test)]
pub(super) fn collect_cached(
    files: &[DexFile],
    aliases: &JavaAliases,
    catalog: &BTreeMap<MethodKey, Option<MethodMetadata>>,
    external: Option<&dyn ExternalMetadata>,
    hierarchy: &crate::type_compatibility::Hierarchy,
    workers: usize,
    cache: &ssa_cache::SsaCache,
) -> SourceThrowsPlan {
    collect_cached_profiled(
        files, aliases, catalog, external, hierarchy, workers, cache, None,
    )
    .0
}
/// Collection result plus per-run scratch state. The scratch carries no
/// observable output; it is separated so its deallocation can overlap later
/// export phases instead of stalling the calling thread.
pub(super) type SourceThrowsScraps<'a> = (
    Facts<'a>,
    VecDeque<MethodKey>,
    BTreeSet<MethodKey>,
    Vec<required::MethodRequirements>,
    Option<StreamedAggregation<required::ThrowsError>>,
    VecDeque<(
        Result<required::MethodRequirements, required::ThrowsError>,
        ThrowsTaskProfile,
    )>,
);
#[allow(clippy::too_many_arguments)]
pub(super) fn collect_cached_profiled<'a>(
    files: &'a [DexFile],
    aliases: &'a JavaAliases,
    catalog: &'a BTreeMap<MethodKey, Option<MethodMetadata>>,
    external: Option<&'a dyn ExternalMetadata>,
    hierarchy: &'a crate::type_compatibility::Hierarchy,
    workers: usize,
    cache: &'a ssa_cache::SsaCache,
    profile: Option<&mut SourceThrowsProfile>,
) -> (SourceThrowsPlan, SourceThrowsScraps<'a>) {
    collect_roots_profiled(
        files,
        aliases,
        catalog,
        external,
        hierarchy,
        None,
        workers,
        Some(cache),
        profile,
    )
}

#[cfg(test)]
pub(super) fn collect_roots(
    files: &[DexFile],
    aliases: &JavaAliases,
    catalog: &BTreeMap<MethodKey, Option<MethodMetadata>>,
    external: Option<&dyn ExternalMetadata>,
    hierarchy: &crate::type_compatibility::Hierarchy,
    roots: Option<&BTreeSet<MethodKey>>,
) -> SourceThrowsPlan {
    collect_roots_with_workers(files, aliases, catalog, external, hierarchy, roots, 1, None)
}
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn collect_roots_with_workers(
    files: &[DexFile],
    aliases: &JavaAliases,
    catalog: &BTreeMap<MethodKey, Option<MethodMetadata>>,
    external: Option<&dyn ExternalMetadata>,
    hierarchy: &crate::type_compatibility::Hierarchy,
    roots: Option<&BTreeSet<MethodKey>>,
    workers: usize,
    cache: Option<&ssa_cache::SsaCache>,
) -> SourceThrowsPlan {
    collect_roots_profiled(
        files, aliases, catalog, external, hierarchy, roots, workers, cache, None,
    )
    .0
}
#[allow(clippy::too_many_arguments)]
fn collect_roots_profiled<'a>(
    files: &'a [DexFile],
    aliases: &'a JavaAliases,
    catalog: &'a BTreeMap<MethodKey, Option<MethodMetadata>>,
    external: Option<&'a dyn ExternalMetadata>,
    hierarchy: &'a crate::type_compatibility::Hierarchy,
    roots: Option<&BTreeSet<MethodKey>>,
    workers: usize,
    cache: Option<&'a ssa_cache::SsaCache>,
    mut profile: Option<&mut SourceThrowsProfile>,
) -> (SourceThrowsPlan, SourceThrowsScraps<'a>) {
    let enabled = profile.is_some();
    let preparation_start = enabled.then(std::time::Instant::now);
    // Facts, the field index, the super index and per-input metadata are
    // independent scans; run them concurrently on bounded workers.
    let super_inputs: Vec<_> = files.iter().collect();
    let (facts, field_index, super_index, metadata) = std::thread::scope(|scope| {
        let facts_worker = scope.spawn(|| Facts::new(files, aliases, catalog, external));
        let field_worker =
            scope.spawn(|| crate::java_members::external_metadata::fields::InputIndex::new(files));
        let super_worker = scope.spawn(|| {
            crate::java_super_resolution::SuperIndex::collect(
                &super_inputs,
                external.unwrap_or(&MissingMetadata),
            )
            .ok()
        });
        let metadata_worker = scope.spawn(|| {
            files
                .iter()
                .map(dex_decompiler::infer::DexMetadata::new)
                .collect::<Vec<_>>()
        });
        let facts = match facts_worker.join() {
            Ok(facts) => facts,
            Err(payload) => std::panic::resume_unwind(payload),
        };
        let field_index = match field_worker.join() {
            Ok(field_index) => field_index,
            Err(payload) => std::panic::resume_unwind(payload),
        };
        let super_index = match super_worker.join() {
            Ok(super_index) => super_index,
            Err(payload) => std::panic::resume_unwind(payload),
        };
        let metadata = match metadata_worker.join() {
            Ok(metadata) => metadata,
            Err(payload) => std::panic::resume_unwind(payload),
        };
        (facts, field_index, super_index, metadata)
    });
    // Frozen per-input resolvers are built concurrently from the completed
    // preparation state; pool order retains the original construction sequence.
    let facts_ref = &facts;
    let metadata_ref = &metadata;
    let field_index_ref = &field_index;
    let super_index_ref = &super_index;
    let resolvers: Vec<_> = std::thread::scope(|scope| {
        let workers: Vec<_> = files
            .iter()
            .enumerate()
            .map(|(pool, dex)| {
                scope.spawn(move || {
                    let mut resolver =
                        Resolver::with_catalog(dex, aliases, &metadata_ref[pool], Some(catalog));
                    resolver.ssa_cache = cache;
                    resolver.field_index = Some(field_index_ref);
                    resolver.inputs = files;
                    resolver.dex_index = pool;
                    resolver.external = external;
                    resolver.input_owners = facts_ref.owners.keys().cloned().collect();
                    resolver.hierarchy = hierarchy.clone();
                    resolver.super_index = super_index_ref.as_ref();
                    resolver
                })
            })
            .collect();
        let mut resolvers = Vec::with_capacity(workers.len());
        for worker in workers {
            match worker.join() {
                Ok(resolver) => resolvers.push(resolver),
                Err(payload) => std::panic::resume_unwind(payload),
            }
        }
        resolvers
    });
    let mut queue: VecDeque<_> = roots
        .map(|roots| roots.iter().cloned().collect())
        .unwrap_or_else(|| facts.definitions.keys().cloned().collect());
    if let (Some(profile), Some(start)) = (profile.as_deref_mut(), preparation_start) {
        profile.walls.insert("preparation", start.elapsed());
    }
    let inspect = |key: &MethodKey| {
        let mut timing = ThrowsTaskProfile::default();
        let result =
            (|| {
                let (pool, method) = facts.definitions.get(key).ok_or_else(|| {
                    required::ThrowsError::Unknown("source body unavailable".into())
                })?;
                let resolver = &resolvers[*pool];
                let context = Context {
                    resolver,
                    facts: &facts,
                };
                let contract = context
                    .invocation_contract(MethodRef::new(method.method_idx))
                    .ok_or_else(|| required::ThrowsError::Unknown("original declaration".into()))?;
                let abstract_declaration = facts.abstracts.contains(key);
                let concrete_domain = facts.concrete_sources.contains(key);
                let implementations = if abstract_declaration || concrete_domain {
                    let mut implementations = Vec::new();
                    for candidate in facts.implementations.get(key) {
                        if candidate == key {
                            continue;
                        }
                        let (candidate_pool, candidate_method) = facts.definitions[candidate];
                        let candidate_context = Context {
                            resolver: &resolvers[candidate_pool],
                            facts: &facts,
                        };
                        if candidate_context.nonimplementation(candidate, key) {
                            continue;
                        }
                        let identity = OriginalMethodIdentity {
                            dex: candidate_pool,
                            index: candidate_method.method_idx,
                            key: candidate.clone(),
                        };
                        match candidate_context.override_contract(&identity) {
                            OverrideContract::Declarations(declarations)
                                if declarations.iter().any(|d| d.key == *key) =>
                            {
                                implementations.push(candidate.clone());
                            }
                            _ => {
                                return Err(required::ThrowsError::Unknown(
                                    "implementation inherited contract".into(),
                                ));
                            }
                        }
                    }
                    implementations
                } else {
                    vec![]
                };
                if abstract_declaration {
                    return required::inspect_abstract_declaration(
                        OriginalMethodIdentity {
                            dex: *pool,
                            index: method.method_idx,
                            key: key.clone(),
                        },
                        method.access_flags,
                        contract.declared,
                        implementations,
                        &context,
                    );
                }
                let code = method.code.as_ref().ok_or_else(|| {
                    required::ThrowsError::Unknown("source code unavailable".into())
                })?;
                let ssa_start = enabled.then(std::time::Instant::now);
                let ssa = (|| {
                    let (ctx, seeds) = initial_context_with_seeds(resolver, method)
                        .map_err(required::ThrowsError::Unknown)?;
                    cached_frontend(resolver, method, code, &ctx, seeds, None).map_err(|e| {
                        required::ThrowsError::Unknown(format!("source throws frontend: {e}"))
                    })
                })();
                if let Some(start) = ssa_start {
                    timing.ssa_time = start.elapsed();
                    timing.ssa_count = 1;
                }
                let ssa = ssa?;
                let inspect_start = enabled.then(std::time::Instant::now);
                let inspected = required::inspect_method(
                    OriginalMethodIdentity {
                        dex: *pool,
                        index: method.method_idx,
                        key: key.clone(),
                    },
                    &ssa,
                    code,
                    method.access_flags,
                    contract.declared,
                    &context,
                );
                if let Some(start) = inspect_start {
                    timing.inspect_method_time = start.elapsed();
                    timing.inspect_method_count = 1;
                }
                let inspected = inspected?;
                if concrete_domain && !implementations.is_empty() {
                    facts
                        .concrete_proofs
                        .lock()
                        .map_err(|_| {
                            required::ThrowsError::Unknown("concrete domain proof lock".into())
                        })?
                        .insert(key.clone(), implementations.iter().cloned().collect());
                    required::attest_concrete_override_domain(inspected, implementations, &context)
                } else {
                    Ok(inspected)
                }
            })();
        (result, timing)
    };
    // The initial queue is unique and indexable (BTreeSet/BTreeMap order).
    // Production scans seed every available body, so dependent bodies are
    // already in this batch. Selected-root scans may discover more bodies;
    // those retain the original breadth-first traversal below.
    let mut initial_results = VecDeque::new();
    let mut streamed: Option<StreamedAggregation<_>> = None;
    let mut visited = BTreeSet::new();
    let mut requirements = Vec::new();
    let mut missing = BTreeMap::new();
    if workers > 1 && queue.len() > 1 {
        // Ordered delivery lets aggregation consume each result on the
        // scheduler owner thread the moment its ordinal completes. The
        // traversal therefore keeps the exact delivery-ordered sequence of
        // the post-run loop below while hiding its cost under worker compute.
        let scheduler_start = enabled.then(std::time::Instant::now);
        let mut state = StreamedAggregation {
            discovered: VecDeque::new(),
            visited: BTreeSet::new(),
            requirements: Vec::new(),
            missing: BTreeMap::new(),
            task_count: 0,
            timing: ThrowsTaskProfile::default(),
        };
        let (result, scheduler_profile) = scheduler::run(
            queue.len(),
            workers,
            |index| inspect(&queue[index]),
            |index, value| {
                state.deliver(&queue[index], value);
                Ok(())
            },
            || (),
        );
        if let (Some(profile), Some(start)) = (profile.as_deref_mut(), scheduler_start) {
            profile.walls.insert("parallel_scheduler", start.elapsed());
            profile.scheduler = Some(scheduler_profile);
        }
        if result.is_err() {
            panic!("source throws collection worker panicked");
        }
        streamed = Some(state);
    }
    let aggregation_start = enabled.then(std::time::Instant::now);
    match streamed.as_mut() {
        // Finalize the streamed traversal: delivered results are already
        // consumed, so only discovered non-batch bodies remain, inspected in
        // the same breadth-first order as the direct loop.
        Some(state) => {
            visited = std::mem::take(&mut state.visited);
            requirements = std::mem::take(&mut state.requirements);
            missing = std::mem::take(&mut state.missing);
            if let Some(profile) = profile.as_deref_mut() {
                profile.task_count += state.task_count;
                profile.ssa_time += state.timing.ssa_time;
                profile.inspect_method_time += state.timing.inspect_method_time;
                profile.ssa_count += state.timing.ssa_count;
                profile.inspect_method_count += state.timing.inspect_method_count;
            }
            let StreamedAggregation { discovered, .. } = &mut *state;
            while let Some(key) = discovered.pop_front() {
                if !visited.insert(key.clone()) {
                    continue;
                }
                let (result, task_timing) = inspect(&key);
                if let Some(profile) = profile.as_deref_mut() {
                    profile.task_count += 1;
                    profile.ssa_time += task_timing.ssa_time;
                    profile.inspect_method_time += task_timing.inspect_method_time;
                    profile.ssa_count += task_timing.ssa_count;
                    profile.inspect_method_count += task_timing.inspect_method_count;
                }
                match result {
                    Ok(value) => {
                        discovered.extend(value.source_targets().cloned());
                        requirements.push(value);
                    }
                    Err(error) => {
                        missing.insert(key, error);
                    }
                }
            }
        }
        None => {
            while let Some(key) = queue.pop_front() {
                if !visited.insert(key.clone()) {
                    continue;
                }
                let (result, timing) = initial_results.pop_front().unwrap_or_else(|| inspect(&key));
                if let Some(profile) = profile.as_deref_mut() {
                    profile.task_count += 1;
                    profile.ssa_time += timing.ssa_time;
                    profile.inspect_method_time += timing.inspect_method_time;
                    profile.ssa_count += timing.ssa_count;
                    profile.inspect_method_count += timing.inspect_method_count;
                }
                match result {
                    Ok(value) => {
                        queue.extend(value.source_targets().cloned());
                        requirements.push(value);
                    }
                    Err(error) => {
                        missing.insert(key, error);
                    }
                }
            }
        }
    }
    if let (Some(profile), Some(start)) = (profile.as_deref_mut(), aggregation_start) {
        profile.walls.insert("aggregation", start.elapsed());
    }
    let Some(resolver) = resolvers.first() else {
        return (
            SourceThrowsPlan::default(),
            (
                facts,
                queue,
                visited,
                requirements,
                streamed,
                initial_results,
            ),
        );
    };
    let context = Context {
        resolver,
        facts: &facts,
    };
    let derive_start = enabled.then(std::time::Instant::now);
    let mut plan = required::derive_program(&requirements, &context).unwrap_or_default();
    if let (Some(profile), Some(start)) = (profile, derive_start) {
        profile.walls.insert("derive_program", start.elapsed());
    }
    plan.unavailable.extend(missing);
    (
        plan,
        (
            facts,
            queue,
            visited,
            requirements,
            streamed,
            initial_results,
        ),
    )
}

pub(super) fn source(plan: &SourceThrowsPlan, key: &MethodKey) -> Option<Vec<String>> {
    plan.methods.get(key).map(|m| m.source.clone())
}
/// Runtime evidence survives a source-only override conflict; unknown frontend,
/// hierarchy, or callee domains never acquire a bridge attestation.
pub(super) fn runtime<'a>(
    plan: &'a SourceThrowsPlan,
    key: &MethodKey,
) -> Option<&'a required::RuntimeThrows> {
    plan.runtime.get(key)
}
pub(super) fn evidence<'a>(
    plan: &'a SourceThrowsPlan,
    key: &MethodKey,
) -> Option<&'a DerivedThrows> {
    plan.methods.get(key)
}

#[cfg(test)]
#[path = "java_source_throws_original_tests.rs"]
mod original_tests;

#[cfg(test)]
#[path = "java_source_throws_index_tests.rs"]
mod index_tests;

#[cfg(test)]
#[path = "java_source_throws_implementation_tests.rs"]
mod implementation_tests;

#[cfg(test)]
#[path = "java_required_throws_shared_source_tests.rs"]
mod shared_source_tests;

#[cfg(test)]
#[path = "java_source_throws_wave10_tests.rs"]
mod wave10_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    fn throws_fixture() -> DexFile {
        use dex_format::{RawSpan, class::ClassData, ids::*, strings::StringData};
        let span = RawSpan { offset: 0, size: 0 };
        let mut dex = DexFile::from_bytes(&super::super::external_fixture::minimal_dex()).unwrap();
        dex.strings[0].value = "Ljava/lang/Object;".into();
        for text in [
            "V",
            "caller",
            "sibling",
            "leaf",
            "broken",
            "Ljava/lang/Throwable;",
            "Ljava/lang/Exception;",
        ] {
            dex.strings.push(StringData {
                value: text.into(),
                utf16_size: text.len() as u32,
                span,
            });
        }
        for descriptor_idx in [1, 6, 7] {
            dex.type_ids.push(TypeId {
                descriptor_idx,
                span,
            });
        }
        dex.proto_ids.push(ProtoId {
            shorty_idx: 1,
            return_type_idx: 1,
            parameters_off: 0,
            parameters: None,
            span,
        });
        let mut methods = Vec::new();
        for name_idx in [2, 3, 4, 5] {
            let method_idx = dex.method_ids.len() as u32;
            dex.method_ids.push(MethodId {
                class_idx: 0,
                proto_idx: 0,
                name_idx,
                span,
            });
            let insns = match method_idx {
                // invoke-static {}, leaf; return-void
                0 => vec![0x0071_u16, 2, 0, 0x000e],
                // new-instance v0, Exception; throw v0
                2 => vec![0x0022, 3, 0x0027],
                _ => vec![0x000e],
            };
            let code = dex_format::code::CodeItem {
                registers_size: 1,
                ins_size: 0,
                outs_size: 0,
                debug_info_off: 0,
                insns_size: insns.len() as u32,
                insns: insns.into_iter().flat_map(u16::to_le_bytes).collect(),
                tries: vec![],
                handlers: vec![],
                span,
            };
            methods.push(EncodedMethod {
                method_idx,
                access_flags: 10, // private static: no inherited declaration
                code_off: 1,
                code: Some(code),
            });
        }
        for (class_idx, superclass_idx) in [(2, 0), (3, 2)] {
            let mut class = dex.classes[0].clone();
            class.class_idx = class_idx;
            class.superclass_idx = superclass_idx;
            dex.classes.push(class);
        }
        dex.classes[0].class_data = Some(ClassData {
            static_fields: vec![],
            instance_fields: vec![],
            direct_methods: methods,
            virtual_methods: vec![],
            span,
        });
        dex
    }

    #[test]
    #[ignore = "bounded original Samsung contract memo comparison; no export"]
    fn samsung_contract_memo_receipt() {
        let files = dex_format::apk::parse_apk_path(
            "/storage_0/decomphify/tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
        )
        .unwrap();
        let external = crate::java_external_metadata::Classpath::from_paths(&[
            std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod"),
            std::path::PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
        ])
        .unwrap();
        let mut inventory = AliasInventory::default();
        for dex in &files {
            for ty in &dex.type_ids {
                inventory
                    .add_type(dex.string(ty.descriptor_idx).unwrap())
                    .unwrap();
            }
            for index in 0..dex.method_ids.len() {
                inventory
                    .add_method(method_key(dex, index as u32).unwrap())
                    .unwrap();
            }
        }
        let aliases = inventory.build(AliasMode::Ascii);
        let catalog = method_catalog(&files);
        let facts = Facts::new(&files, &aliases, &catalog, Some(&external));
        let mut calls = 0;
        let mut misses = 0;
        let mut uncached = std::time::Duration::ZERO;
        let mut cached = std::time::Duration::ZERO;
        for (pool, dex) in files.iter().enumerate() {
            let metadata = dex_decompiler::infer::DexMetadata::new(dex);
            let mut resolver = Resolver::with_catalog(dex, &aliases, &metadata, Some(&catalog));
            resolver.external = Some(&external);
            resolver.dex_index = pool;
            let context = Context {
                resolver: &resolver,
                facts: &facts,
            };
            for index in (0..dex.method_ids.len()).step_by(97).take(256) {
                let identity = OriginalMethodIdentity {
                    dex: pool,
                    index: index as u32,
                    key: method_key(dex, index as u32).unwrap(),
                };
                for _ in 0..8 {
                    calls += 1;
                    misses +=
                        usize::from(!facts.override_contracts.lock().unwrap().contains_key(&(
                            pool,
                            index as u32,
                            identity.key.clone(),
                        )));
                    let start = std::time::Instant::now();
                    let expected = context.override_contract_uncached(&identity);
                    uncached += start.elapsed();
                    let start = std::time::Instant::now();
                    let actual = context.override_contract(&identity);
                    cached += start.elapsed();
                    assert_eq!(format!("{:?}", actual), format!("{:?}", expected));
                }
            }
        }
        eprintln!("override calls={calls} misses={misses} uncached={uncached:?} cached={cached:?}");
        let mut calls = 0;
        let mut misses = 0;
        let mut uncached = std::time::Duration::ZERO;
        let mut cached = std::time::Duration::ZERO;
        for name in facts.originals.keys().step_by(53).take(512) {
            for _ in 0..8 {
                calls += 1;
                misses += usize::from(!facts.kinds.lock().unwrap().contains_key(name));
                let start = std::time::Instant::now();
                let expected = facts.kind_uncached(name);
                uncached += start.elapsed();
                let start = std::time::Instant::now();
                let actual = facts.kind(name);
                cached += start.elapsed();
                assert_eq!(actual, expected);
            }
        }
        eprintln!("kind calls={calls} misses={misses} uncached={uncached:?} cached={cached:?}");
    }

    #[test]
    fn immutable_evidence_memo_preserves_positive_cycle_and_unknown_results() {
        let files = vec![throws_fixture()];
        let aliases = JavaAliases::default();
        let catalog = method_catalog(&files);
        let facts = Facts::new(&files, &aliases, &catalog, None);
        for name in ["java.lang.Exception", "java.lang.Throwable", "missing.Type"] {
            assert_eq!(facts.kind(name), facts.kind_uncached(name));
            assert_eq!(facts.kind(name), facts.kind_uncached(name));
        }
        assert_eq!(
            facts.kind("java.lang.Exception"),
            Some(ExceptionKind::Checked)
        );
        assert!(facts.kinds.lock().unwrap().contains_key("missing.Type"));
        facts.classes.lock().unwrap().insert(
            "Lcycle/A;".into(),
            Some(Arc::new(RawClass {
                parent: Some("Lcycle/A;".into()),
                interfaces: vec![],
                methods: vec![],
            })),
        );
        assert_eq!(facts.kind("cycle.A"), None);
        assert_eq!(facts.kind("cycle.A"), facts.kind_uncached("cycle.A"));
        let metadata = dex_decompiler::infer::DexMetadata::new(&files[0]);
        let resolver = Resolver::with_catalog(&files[0], &aliases, &metadata, Some(&catalog));
        let context = Context {
            resolver: &resolver,
            facts: &facts,
        };
        let key = method_key(&files[0], 0).unwrap();
        let identities = [
            OriginalMethodIdentity {
                dex: 0,
                index: 0,
                key: key.clone(),
            },
            OriginalMethodIdentity {
                dex: 1,
                index: 0,
                key: key.clone(),
            },
            OriginalMethodIdentity {
                dex: 0,
                index: 999,
                key: key.clone(),
            },
            OriginalMethodIdentity {
                dex: 0,
                index: 999,
                key: MethodKey {
                    owner: "Lcycle/A;".into(),
                    ..key.clone()
                },
            },
            OriginalMethodIdentity {
                dex: 0,
                index: 999,
                key: MethodKey {
                    owner: "Lmissing/A;".into(),
                    ..key
                },
            },
        ];
        for identity in &identities {
            let expected = format!("{:?}", context.override_contract_uncached(identity));
            assert_eq!(
                format!("{:?}", context.override_contract(identity)),
                expected
            );
            assert_eq!(
                format!("{:?}", context.override_contract(identity)),
                expected
            );
        }
        assert_eq!(
            facts.override_contracts.lock().unwrap().len(),
            identities.len()
        );
        assert!(matches!(
            context.override_contract(&identities[3]),
            OverrideContract::Unknown
        ));
        assert!(matches!(
            context.override_contract(&identities[4]),
            OverrideContract::Unknown
        ));
    }

    #[test]
    fn frozen_return_only_alias_mismatch_proves_no_source_override() {
        use dex_format::{
            class::ClassData,
            ids::{MethodId, ProtoId, TypeId},
            strings::StringData,
        };
        let mut dex = throws_fixture();
        let span = dex_format::RawSpan { offset: 0, size: 0 };
        dex.classes[0].class_data.as_mut().unwrap().direct_methods[2].access_flags = 1;
        dex.strings.push(StringData {
            value: "I".into(),
            utf16_size: 1,
            span,
        });
        dex.type_ids.push(TypeId {
            descriptor_idx: 8,
            span,
        });
        dex.proto_ids.push(ProtoId {
            shorty_idx: 8,
            return_type_idx: 4,
            parameters_off: 0,
            parameters: None,
            span,
        });
        dex.method_ids.extend([
            MethodId {
                class_idx: 3,
                proto_idx: 0,
                name_idx: 4,
                span,
            },
            MethodId {
                class_idx: 3,
                proto_idx: 1,
                name_idx: 4,
                span,
            },
        ]);
        dex.classes[2].class_data = Some(ClassData {
            static_fields: vec![],
            instance_fields: vec![],
            direct_methods: vec![],
            virtual_methods: [4, 5]
                .into_iter()
                .map(|method_idx| EncodedMethod {
                    method_idx,
                    access_flags: 0x401,
                    code_off: 0,
                    code: None,
                })
                .collect(),
            span,
        });
        let files = vec![dex];
        let mut inventory = AliasInventory::default();
        for index in 0..files[0].method_ids.len() {
            inventory
                .add_method(method_key(&files[0], index as u32).unwrap())
                .unwrap();
        }
        let aliases = inventory.build(AliasMode::Ascii);
        let catalog = method_catalog(&files);
        let facts = Facts::new(&files, &aliases, &catalog, None);
        let metadata = dex_decompiler::infer::DexMetadata::new(&files[0]);
        let resolver = Resolver::with_catalog(&files[0], &aliases, &metadata, Some(&catalog));
        let context = Context {
            resolver: &resolver,
            facts: &facts,
        };
        let inherited = method_key(&files[0], 2).unwrap();
        let child = method_key(&files[0], 4).unwrap();
        assert_ne!(
            facts.emitted_method_name(&inherited),
            facts.emitted_method_name(&child)
        );
        assert!(matches!(
            context.override_contract(&OriginalMethodIdentity {
                dex: 0,
                index: 4,
                key: child
            }),
            OverrideContract::NoInheritedDeclaration
        ));
    }

    fn assert_same_plan(actual: &SourceThrowsPlan, expected: &SourceThrowsPlan) {
        assert_eq!(actual.unavailable, expected.unavailable);
        assert_eq!(
            actual.methods.keys().collect::<Vec<_>>(),
            expected.methods.keys().collect::<Vec<_>>()
        );
        for (key, value) in &actual.methods {
            let expected = &expected.methods[key];
            assert_eq!(value.identity, expected.identity);
            assert_eq!(value.original, expected.original);
            assert_eq!(value.derived, expected.derived);
            assert_eq!(value.source, expected.source);
        }
    }

    #[test]
    fn bounded_workers_preserve_full_plan_and_root_dependency_evidence() {
        let mut second = throws_fixture();
        second.strings[0].value = "Lsample/Other;".into();
        second.classes.truncate(1);
        second.strings.push(dex_format::strings::StringData {
            value: "Ljava/lang/Object;".into(),
            utf16_size: 18,
            span: dex_format::RawSpan { offset: 0, size: 0 },
        });
        second.type_ids.push(dex_format::ids::TypeId {
            descriptor_idx: 8,
            span: dex_format::RawSpan { offset: 0, size: 0 },
        });
        second.classes[0].superclass_idx = 4;
        let files = vec![throws_fixture(), second];
        let mut inventory = AliasInventory::default();
        for dex in &files {
            for ty in &dex.type_ids {
                inventory
                    .add_type(dex.string(ty.descriptor_idx).unwrap())
                    .unwrap();
            }
            for index in 0..dex.method_ids.len() {
                inventory
                    .add_method(method_key(dex, index as u32).unwrap())
                    .unwrap();
            }
        }
        let aliases = inventory.build(AliasMode::Ascii);
        let hierarchy =
            crate::java_hierarchy::build_hierarchy(&files.iter().collect::<Vec<_>>(), vec![])
                .unwrap()
                .map_names(|n| {
                    aliases
                        .class_name(&format!("L{};", n.replace('.', "/")))
                        .unwrap_or(n)
                        .into()
                })
                .unwrap();
        let mut catalog = method_catalog(&files);
        let broken = method_key(&files[0], 3).unwrap();
        catalog
            .get_mut(&broken)
            .unwrap()
            .as_mut()
            .unwrap()
            .exceptions = None;
        let serial = collect(&files, &aliases, &catalog, None, &hierarchy);
        let caller = method_key(&files[0], 0).unwrap();
        let leaf = method_key(&files[0], 2).unwrap();
        assert_eq!(serial.methods[&caller].derived, vec!["java.lang.Exception"]);
        assert_eq!(serial.methods[&leaf].derived, vec!["java.lang.Exception"]);
        let second_caller = method_key(&files[1], 0).unwrap();
        assert_eq!(serial.methods[&second_caller].identity.dex, 1);
        assert_eq!(
            serial.methods[&second_caller].derived,
            vec!["java.lang.Exception"]
        );
        assert!(matches!(
            serial.unavailable[&broken],
            required::ThrowsError::Unknown(_)
        ));
        for workers in [0, 1, 2, 4, 32] {
            let parallel =
                collect_with_workers(&files, &aliases, &catalog, None, &hierarchy, workers);
            assert_same_plan(&parallel, &serial);
        }
        let cache = ssa_cache::SsaCache::new(true);
        let cached = collect_cached(&files, &aliases, &catalog, None, &hierarchy, 4, &cache);
        assert_same_plan(&cached, &serial);
        let misses = cache.stats().misses;
        assert!(misses > 0);
        let repeated = collect_cached(&files, &aliases, &catalog, None, &hierarchy, 1, &cache);
        assert_same_plan(&repeated, &serial);
        assert_eq!(cache.stats().misses, misses);
        assert!(cache.stats().hits > 0);
        for workers in [1, 4] {
            let mut profile = SourceThrowsProfile::default();
            let profiled = collect_cached_profiled(
                &files,
                &aliases,
                &catalog,
                None,
                &hierarchy,
                workers,
                &cache,
                Some(&mut profile),
            )
            .0;
            assert_same_plan(&profiled, &serial);
            assert!(profile.task_count > 0);
            assert!(profile.ssa_count > 0);
            assert!(profile.inspect_method_count > 0);
            assert!(profile.inspect_method_count <= profile.ssa_count);
            for stage in ["preparation", "aggregation", "derive_program"] {
                assert!(profile.walls.contains_key(stage));
            }
            assert_eq!(profile.scheduler.is_some(), workers > 1);
        }

        let missing = MethodKey {
            name: "unavailable".into(),
            ..caller.clone()
        };
        let roots = BTreeSet::from([
            caller.clone(),
            method_key(&files[0], 1).unwrap(),
            broken,
            missing.clone(),
        ]);
        let selected = collect_roots(&files, &aliases, &catalog, None, &hierarchy, Some(&roots));
        assert_eq!(
            selected.methods[&caller].derived,
            vec!["java.lang.Exception"]
        );
        assert!(selected.methods.contains_key(&leaf));
        assert_eq!(
            selected.unavailable[&missing],
            required::ThrowsError::Unknown("source body unavailable".into())
        );
        for workers in [2, 4, 32] {
            assert_same_plan(
                &collect_roots_with_workers(
                    &files,
                    &aliases,
                    &catalog,
                    None,
                    &hierarchy,
                    Some(&roots),
                    workers,
                    None,
                ),
                &selected,
            );
        }
        let empty: Vec<DexFile> = vec![];
        let plan = collect_with_workers(&empty, &aliases, &BTreeMap::new(), None, &hierarchy, 4);
        assert!(plan.methods.is_empty() && plan.unavailable.is_empty());
    }

    #[test]
    #[ignore = "local Samsung Pass and explicit JDK17/Android35 declarations"]
    fn samsung_derived_source_throws_actual_leaf_and_public_contract() {
        let files = dex_format::apk::parse_apk_path(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
        )
        .unwrap();
        let external = crate::java_external_metadata::Classpath::from_paths(&[
            std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod"),
            std::path::PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
        ])
        .unwrap();
        let mut inventory = AliasInventory::default();
        for dex in &files {
            for ty in &dex.type_ids {
                inventory
                    .add_type(dex.string(ty.descriptor_idx).unwrap())
                    .unwrap();
            }
            for index in 0..dex.method_ids.len() {
                inventory
                    .add_method(method_key(dex, index as u32).unwrap())
                    .unwrap();
            }
            for index in 0..dex.field_ids.len() {
                inventory
                    .add_field(field_key(dex, index as u32).unwrap())
                    .unwrap();
            }
        }
        let aliases = inventory.build(AliasMode::Ascii);
        let hierarchy = crate::java_hierarchy::build_hierarchy(
            &files.iter().collect::<Vec<_>>(),
            external.hierarchy(),
        )
        .unwrap()
        .map_names(|n| {
            aliases
                .class_name(&format!("L{};", n.replace('.', "/")))
                .unwrap_or(n)
                .into()
        })
        .unwrap();
        let catalog = method_catalog(&files);
        let roots = [12288, 60314]
            .map(|index| method_key(&files[0], index).unwrap())
            .into_iter()
            .collect();
        let plan = collect_roots(
            &files,
            &aliases,
            &catalog,
            Some(&external),
            &hierarchy,
            Some(&roots),
        );
        for index in [12288, 60314] {
            let key = method_key(&files[0], index).unwrap();
            let evidence = plan
                .methods
                .get(&key)
                .unwrap_or_else(|| panic!("{key:?}: {:?}", plan.unavailable.get(&key)));
            assert!(evidence.original.is_empty());
            assert!(!evidence.derived.is_empty());
            assert_eq!(catalog[&key].as_ref().unwrap().exceptions, Some(vec![]));
            println!("EXACT {index} {:?} derived={:?}", key, evidence.derived);
        }
        let metadata = dex_decompiler::infer::DexMetadata::new(&files[0]);
        let mut resolver = Resolver::with_catalog(&files[0], &aliases, &metadata, Some(&catalog));
        resolver.source_throws = Some(&plan);
        for index in [12288, 60314] {
            let key = method_key(&files[0], index).unwrap();
            assert_eq!(
                resolver.method(MethodRef::new(index)).unwrap().throws,
                plan.methods[&key].source
            );
            assert_eq!(
                resolver.declaration(index).unwrap().exceptions,
                Some(vec![])
            );
        }
    }
    #[test]
    #[ignore = "local Samsung Pass and explicit JDK17/Android35 declarations"]
    fn samsung_wave13_lost_throws_contract_inventory() {
        let files = dex_format::apk::parse_apk_path(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
        )
        .unwrap();
        let external = crate::java_external_metadata::Classpath::from_paths(&[
            std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod"),
            std::path::PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
        ])
        .unwrap();
        let mut inventory = AliasInventory::default();
        for dex in &files {
            for ty in &dex.type_ids {
                inventory
                    .add_type(dex.string(ty.descriptor_idx).unwrap())
                    .unwrap();
            }
            for index in 0..dex.method_ids.len() {
                inventory
                    .add_method(method_key(dex, index as u32).unwrap())
                    .unwrap();
            }
            for index in 0..dex.field_ids.len() {
                inventory
                    .add_field(field_key(dex, index as u32).unwrap())
                    .unwrap();
            }
        }
        let aliases = inventory.build(AliasMode::Ascii);
        let hierarchy = crate::java_hierarchy::build_hierarchy(
            &files.iter().collect::<Vec<_>>(),
            external.hierarchy(),
        )
        .unwrap()
        .map_names(|n| {
            aliases
                .class_name(&format!("L{};", n.replace('.', "/")))
                .unwrap_or(n)
                .into()
        })
        .unwrap();
        let catalog = method_catalog(&files);
        let roots = [8865, 8866, 9649, 9990, 9991, 9992, 10013]
            .map(|index| method_key(&files[0], index).unwrap())
            .into_iter()
            .collect();
        let plan = collect_roots(
            &files,
            &aliases,
            &catalog,
            Some(&external),
            &hierarchy,
            Some(&roots),
        );
        for index in [8865, 8866, 9649, 9990, 9991, 9992, 10013] {
            let key = method_key(&files[0], index).unwrap();
            println!(
                "LOSS {index} {key:?} => {:?} / {:?}",
                plan.methods.get(&key),
                plan.unavailable.get(&key)
            );
        }
        for index in [8865, 8866, 8470, 8471] {
            let key = method_key(&files[0], index).unwrap();
            let approved = plan.methods.get(&key).unwrap();
            assert_eq!(approved.identity.key, key);
            assert!(approved.original.is_empty());
            assert!(
                approved
                    .source
                    .contains(&"org.xmlpull.v1.XmlPullParserException".to_owned())
            );
            assert!(
                catalog
                    .get(&key)
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .exceptions
                    .as_ref()
                    .unwrap()
                    .is_empty()
            );
        }
        for index in [9649, 9990, 9991, 9992, 10013] {
            let key = method_key(&files[0], index).unwrap();
            assert!(matches!(
                plan.unavailable.get(&key),
                Some(required::ThrowsError::OverrideConflict(_))
            ));
        }
        for (key, evidence) in &plan.methods {
            if key.owner.contains("appcompat/widget/o0") && key.name.starts_with("parseBundle") {
                println!("SOURCE ANCESTOR {key:?}: {evidence:?}");
            }
        }
    }
}

#[cfg(test)]
#[path = "java_source_throws_final_tests.rs"]
mod final_tests;

#[cfg(test)]
#[path = "java_checked_recipe_rollback_tests.rs"]
mod checked_recipe_rollback_tests;
