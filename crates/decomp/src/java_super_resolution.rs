//! Exact class-only invoke-super selection through trusted ancestor declarations.
use crate::{
    java_access::AccessContext,
    java_aliases::{AliasInventory, JavaAliases, MethodAlias, MethodKey},
    java_external_metadata::{ExternalMetadata, MethodMetadata},
};
use backend_java::type_compatibility::Hierarchy;
use dex_format::DexFile;
use std::collections::{BTreeMap, BTreeSet};
/// Complete declared method evidence.
#[derive(Clone, Debug)]
pub struct SuperMethod {
    /// Original exact signature.
    pub key: MethodKey,
    /// Genuine access/exception metadata.
    pub metadata: MethodMetadata,
    /// Generic Signature presence; selection needs an erased-source attestation.
    pub generic: bool,
}
/// Genuine superclass and complete declaration inventory.
#[derive(Clone, Debug)]
pub struct SuperClass {
    /// Original direct parent descriptor.
    pub superclass: Option<String>,
    /// Original class flags.
    pub access_flags: u32,
    /// All declared methods, including private/static/bridge declarations.
    pub methods: Vec<SuperMethod>,
}
/// Provider must return complete genuine declarations, not reference inventories.
pub trait SuperMetadata {
    /// Bind a privately generated final-target alias proof to this exact
    /// original input bridge body and genuine final target declaration.
    fn attests_final_bridge_alias(
        &self,
        _bridge: &MethodKey,
        _code: &dex_format::code::CodeItem,
        _target: &MethodKey,
    ) -> bool {
        false
    }
    /// Complete original direct interfaces; absent evidence rejects interface selection.
    fn interfaces(&self, _descriptor: &str) -> Option<Vec<String>> {
        None
    }
    /// Original descriptor lookup, unambiguous or unavailable.
    fn class(&self, descriptor: &str) -> Option<SuperClass>;
    /// Attest that this exact synthetic companion is a genuine source-erased
    /// covariance bridge to the unique nonbridge target. Flags or a pool
    /// reference alone are insufficient; published input aliases/body evidence
    /// and classpath bridges require an explicit byte-derived origin proof.
    fn attests_covariant_bridge(&self, _bridge: &MethodKey, _target: &MethodKey) -> bool {
        false
    }
    /// Attest that this genuine input declaration is emitted with the exact
    /// erased descriptor, rather than its original generic Signature. This
    /// describes source spelling only; selected body publication remains a
    /// separate dependency requirement. External declarations default closed.
    fn source_signature_is_erased(&self, _method: &MethodKey) -> bool {
        false
    }
    /// Contextual erased source signature through this exact superclass path.
    /// Providers must attest source spelling; a descriptor alone is insufficient.
    fn source_super_signature_is_erased(
        &self,
        _caller: &str,
        _ancestry: &[String],
        method: &MethodKey,
    ) -> bool {
        self.source_signature_is_erased(method)
    }
}
/// Indexed input declarations with explicit classpath fallback.
pub struct SuperIndex<'a> {
    inputs: BTreeMap<String, SuperClass>,
    external: &'a dyn ExternalMetadata,
    interfaces: BTreeMap<String, Vec<String>>,
    alias_bridge_bodies: BTreeMap<MethodKey, (dex_format::code::CodeItem, MethodKey)>,
    final_source_targets: BTreeSet<MethodKey>,
}
impl<'a> SuperIndex<'a> {
    /// Collect all genuine input definitions once; duplicate owners fail closed.
    pub fn collect(
        inputs: &[&DexFile],
        external: &'a dyn ExternalMetadata,
    ) -> Result<Self, SuperError> {
        let mut out = BTreeMap::new();
        let mut interfaces = BTreeMap::new();
        let mut alias_bridge_bodies = BTreeMap::new();
        let mut final_source_targets = BTreeSet::new();
        for d in inputs {
            for c in &d.classes {
                let owner = descriptor(d, c.class_idx).ok_or(SuperError::Metadata)?;
                interfaces.insert(
                    owner.clone(),
                    c.interfaces
                        .as_ref()
                        .map(|xs| xs.types.as_slice())
                        .unwrap_or(&[])
                        .iter()
                        .map(|i| descriptor(d, u32::from(*i)))
                        .collect::<Option<Vec<_>>>()
                        .ok_or(SuperError::Metadata)?,
                );
                let superclass = if c.superclass_idx == u32::MAX {
                    None
                } else {
                    Some(descriptor(d, c.superclass_idx).ok_or(SuperError::Metadata)?)
                };
                let mut methods = Vec::new();
                if let Some(data) = &c.class_data {
                    for m in data.direct_methods.iter().chain(&data.virtual_methods) {
                        let key = crate::java_members::source_closure::method_key(d, m.method_idx)
                            .ok_or(SuperError::Metadata)?;
                        if let Some(code) = &m.code {
                            if m.access_flags == 0x1041
                                && code.insns_size == 5
                                && code.ins_size == 1
                                && code.insns.len() == 10
                            {
                                let target_index =
                                    u16::from_le_bytes([code.insns[2], code.insns[3]]);
                                if let Some(target) =
                                    crate::java_members::source_closure::method_key(
                                        d,
                                        u32::from(target_index),
                                    )
                                {
                                    alias_bridge_bodies.insert(key.clone(), (code.clone(), target));
                                }
                            }
                            if m.access_flags == 0x11 {
                                final_source_targets.insert(key.clone());
                            }
                        }
                        let exceptions = crate::java_members::MemberResolver { dex: d }
                            .declared_throws_for_class(c, m.method_idx)
                            .map(|xs| {
                                xs.into_iter()
                                    .map(|s| format!("L{};", s.replace('.', "/")))
                                    .collect()
                            });
                        methods.push(SuperMethod {
                            key,
                            metadata: MethodMetadata {
                                access_flags: m.access_flags,
                                exceptions,
                            },
                            generic: c.annotations.as_ref().is_some_and(|directory| {
                                directory
                                    .method_annotations
                                    .iter()
                                    .filter(|a| a.member_idx == m.method_idx)
                                    .flat_map(|a| &a.annotations.entries)
                                    .any(|a| {
                                        descriptor(d, a.annotation.type_idx).as_deref()
                                            == Some("Ldalvik/annotation/Signature;")
                                    })
                            }),
                        });
                    }
                }
                if out
                    .insert(
                        owner,
                        SuperClass {
                            superclass,
                            access_flags: c.access_flags,
                            methods,
                        },
                    )
                    .is_some()
                {
                    return Err(SuperError::Metadata);
                }
            }
        }
        Ok(Self {
            inputs: out,
            external,
            interfaces,
            alias_bridge_bodies,
            final_source_targets,
        })
    }
}
impl SuperMetadata for SuperIndex<'_> {
    fn attests_final_bridge_alias(
        &self,
        bridge: &MethodKey,
        code: &dex_format::code::CodeItem,
        target: &MethodKey,
    ) -> bool {
        self.alias_bridge_bodies
            .get(bridge)
            .is_some_and(|(original, resolved)| original == code && resolved == target)
            && self.final_source_targets.contains(target)
    }
    fn interfaces(&self, owner: &str) -> Option<Vec<String>> {
        if let Some(xs) = self.interfaces.get(owner) {
            return Some(xs.clone());
        }
        let mut xs = self.external.class_hierarchy(owner).into_iter();
        let declaration = xs.next()?;
        if xs.next().is_some() {
            return None;
        }
        Some(
            declaration
                .interfaces
                .iter()
                .map(|s| format!("L{};", s.replace('.', "/")))
                .collect(),
        )
    }
    fn attests_covariant_bridge(&self, bridge: &MethodKey, target: &MethodKey) -> bool {
        // An input bridge may be published under a frozen alias or a different
        // source body. External classfile evidence cannot attest that spelling.
        !self.inputs.contains_key(&bridge.owner)
            && !self.inputs.contains_key(&target.owner)
            && self.external.attests_covariant_bridge(bridge, target)
    }
    fn source_signature_is_erased(&self, method: &MethodKey) -> bool {
        self.inputs
            .get(&method.owner)
            .is_some_and(|class| class.methods.iter().any(|m| m.key == *method))
    }
    fn source_super_signature_is_erased(
        &self,
        caller: &str,
        ancestry: &[String],
        method: &MethodKey,
    ) -> bool {
        if self.source_signature_is_erased(method) {
            return true;
        }
        let Some(methods) = self.external.declared_methods(&method.owner) else {
            return false;
        };
        let mut matches = methods.iter().filter(|m| m.key == *method);
        let Some(declaration) = matches.next() else {
            return false;
        };
        if matches.next().is_some() {
            return false;
        }
        let class_signature = self.external.class_generic_signature(&method.owner);
        let Some(class_signature) = class_signature else {
            return false;
        };
        let Some(needs_raw) = declaration
            .signature
            .as_deref()
            .and_then(|s| signature::checked_method(s, method, class_signature.as_deref()))
        else {
            return false;
        };
        // A descriptor-cast raw Collection/List remains applicable to its
        // parameterized counterpart; erasure and return descriptor are exact.
        // Top-level type variables additionally need contextual raw ancestry.
        if !needs_raw {
            return true;
        }
        if !self.inputs.contains_key(caller) {
            return false;
        }
        let external: Vec<_> = ancestry
            .iter()
            .skip_while(|owner| self.inputs.contains_key(*owner))
            .collect();
        if external.is_empty() {
            return false;
        }
        let mut raw = false;
        for (index, owner) in external.iter().enumerate() {
            let Some(class) = self.class(owner) else {
                return false;
            };
            let Some(text) = self.external.class_generic_signature(owner) else {
                return false;
            };
            let parsed = match text.as_deref() {
                Some(s) => match signature::class(s) {
                    Some(p) => Some(p),
                    None => return false,
                },
                None => None,
            };
            if parsed
                .as_ref()
                .is_some_and(|p| Some(&p.parent) != class.superclass.as_ref())
            {
                return false;
            }
            let generic = parsed.as_ref().is_some_and(|p| p.generic);
            if index == 0 {
                raw = generic;
            }
            if **owner == method.owner {
                return raw;
            }
            let Some(next) = external.get(index + 1) else {
                return false;
            };
            if class.superclass.as_ref() != Some(*next) {
                return false;
            }
            // A raw generic superclass erases its direct ancestor type; a
            // nongeneric intermediate retains concrete parent bindings.
            raw = (raw && generic) || parsed.as_ref().is_none_or(|p| !p.parent_parameterized);
        }
        false
    }

    fn class(&self, owner: &str) -> Option<SuperClass> {
        if let Some(c) = self.inputs.get(owner) {
            return Some(c.clone());
        }
        let mut h = self.external.class_hierarchy(owner).into_iter();
        let c = h.next()?;
        if h.next().is_some() {
            return None;
        }
        let flags = self.external.class_access(owner)?;
        if c.is_interface != (flags & 0x200 != 0) {
            return None;
        }
        let methods = self
            .external
            .declared_methods(owner)?
            .into_iter()
            .map(|m| SuperMethod {
                key: m.key,
                metadata: m.metadata,
                generic: m.signature.is_some(),
            })
            .collect();
        Some(SuperClass {
            superclass: c.superclass.map(|s| format!("L{};", s.replace('.', "/"))),
            access_flags: flags,
            methods,
        })
    }
}

#[path = "java_super_signature.rs"]
mod signature;

// Legacy focused helper: class-variable signatures require contextual raw proof.
#[cfg(test)]
fn class_variable_signature(signature: &str, key: &MethodKey) -> bool {
    signature::method(signature, key) == Some(true)
}
/// Inventory genuine ancestor declarations before freezing aliases. A missing
/// hierarchy ends discovery; it is never treated as an empty method inventory
/// by the later exact target-selection proof.
pub(crate) fn inventory_ancestors(
    files: &[DexFile],
    provider: &impl SuperMetadata,
    inventory: &mut AliasInventory,
) -> Result<(), String> {
    let mut visited = BTreeSet::new();
    for dex in files {
        for class in &dex.classes {
            let Some(mut owner) = descriptor(dex, class.class_idx) else {
                return Err("invalid original super inventory owner".into());
            };
            loop {
                if !visited.insert(owner.clone()) {
                    break;
                }
                let Some(class) = provider.class(&owner) else {
                    break;
                };
                if class.access_flags & 0x200 != 0 {
                    break;
                }
                inventory.add_type(&owner)?;
                for method in class.methods {
                    inventory.add_method(method.key)?;
                }
                let Some(parent) = class.superclass else {
                    break;
                };
                owner = parent;
            }
        }
    }
    Ok(())
}
fn descriptor(d: &DexFile, i: u32) -> Option<String> {
    Some(d.string(d.type_id(i).ok()?.descriptor_idx).ok()?.into())
}
fn dotted(d: &str) -> Result<String, SuperError> {
    if !d.starts_with('L') || !d.ends_with(';') {
        return Err(SuperError::Metadata);
    }
    Ok(d[1..d.len() - 1].replace('/', "."))
}
/// Explicit reason an exact Java super target cannot be established.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SuperError {
    /// Missing, duplicate, cyclic or invalid declaration evidence.
    Metadata,
    /// Interface ancestry or interface-super is outside the class-only proof.
    Interface,
    /// Original reference owner is not on the caller's superclass chain.
    NotAncestor,
    /// Java selection differs in return identity or original dispatch shape.
    Selection,
    /// Generic selection, selected bridge or malformed varargs lacks proof.
    Overload,
    /// Source naming changes the original overload/override identity.
    Aliases,
    /// Original method access is not legal for the caller.
    Access,
}
/// Exact executable target selected from the caller's immediate superclass.
#[derive(Clone, Debug)]
pub struct SuperSelection {
    /// Actual declaring owner, not merely the pool-reference owner.
    pub target: MethodKey,
    /// Metadata of that exact selected declaration.
    pub metadata: MethodMetadata,
    /// Genuine classes inspected, from immediate parent towards root.
    pub ancestry: Vec<String>,
    /// Direct source interface qualifier, absent for class-super.
    pub interface_qualifier: Option<String>,
}
// Retain original hierarchy names, independently of frozen source aliases.
// This local decoder also keeps the included super-proof harness independent
// of the facade descriptor module; malformed/void/unknown returns stay closed.
fn bridge_return_type(descriptor: &str) -> Option<backend_java::JavaType> {
    use backend_java::JavaType;
    let rank = descriptor.bytes().take_while(|b| *b == b'[').count();
    if rank > 255 {
        return None;
    }
    let mut ty = match &descriptor[rank..] {
        "Z" => JavaType::Boolean,
        "B" => JavaType::Byte,
        "S" => JavaType::Short,
        "C" => JavaType::Char,
        "I" => JavaType::Int,
        "J" => JavaType::Long,
        "F" => JavaType::Float,
        "D" => JavaType::Double,
        base => {
            let name = dotted(base).ok()?;
            if name
                .split('.')
                .any(|part| part.is_empty() || backend_java::sanitize_identifier(part) != part)
            {
                return None;
            }
            JavaType::Class(name)
        }
    };
    for _ in 0..rank {
        ty = JavaType::Array(Box::new(ty));
    }
    Some(ty)
}
fn attested_return_bridge(
    bridge: &SuperMethod,
    target: &SuperMethod,
    provider: &impl SuperMetadata,
    hierarchy: &Hierarchy,
) -> bool {
    if bridge.key.owner != target.key.owner
        || bridge.key.name != target.key.name
        || bridge.key.parameters != target.key.parameters
        || bridge.key.return_type == target.key.return_type
        || bridge.metadata.access_flags & (0x1040 | 2 | 8 | 0x80 | 0x100 | 0x400) != 0x1040
        || target.metadata.access_flags & (2 | 8 | 0x40 | 0x80 | 0x400) != 0
        || bridge.generic
        || bridge.metadata.exceptions.is_none()
        || !provider.attests_covariant_bridge(&bridge.key, &target.key)
    {
        return false;
    }
    let Some(actual) = bridge_return_type(&target.key.return_type) else {
        return false;
    };
    let Some(expected) = bridge_return_type(&bridge.key.return_type) else {
        return false;
    };
    matches!(
        actual,
        backend_java::JavaType::Class(_) | backend_java::JavaType::Array(_)
    ) && matches!(
        expected,
        backend_java::JavaType::Class(_) | backend_java::JavaType::Array(_)
    ) && hierarchy.assignable(&actual, &expected)
        == backend_java::type_compatibility::Compatibility::Assignable
}
/// Prove Java `super.name(descriptor-cast arguments)` selects the exact DEX
/// class-super target. Reference owner must be an ancestor; Java/Dalvik both
/// start lookup at immediate parent. Complete intermediate method inventories
/// prevent missed overrides. Only byte-attested covariance companions may be
/// omitted; ordinary return-only overloads, selected bridges, unproved generic selection, static/private/abstract targets and interfaces stay closed.
/// Parent must emit the proved target with exact parameter casts and use its
/// metadata, rather than the original reference's potentially inherited owner.
pub fn select_super(
    caller: &str,
    reference: &MethodKey,
    provider: &impl SuperMetadata,
    aliases: &JavaAliases,
    hierarchy: &Hierarchy,
) -> Result<SuperSelection, SuperError> {
    if let Some(proof) = aliases.final_bridge_alias(reference) {
        return final_bridge_alias::select(caller, reference, provider, aliases, hierarchy, &proof);
    }
    if reference.name.starts_with('<') {
        return Err(SuperError::Selection);
    }
    let class = provider.class(caller).ok_or(SuperError::Metadata)?;
    if class.access_flags & 0x200 != 0 {
        return Err(SuperError::Interface);
    }
    let mut next = class.superclass;
    let mut seen = BTreeSet::new();
    let mut ancestry = Vec::new();
    let mut selected = None;
    let mut owner_seen = false;
    let mut symbolic_resolved = false;
    while let Some(owner) = next {
        if !seen.insert(owner.clone()) {
            return Err(SuperError::Metadata);
        }
        let class = provider.class(&owner).ok_or(SuperError::Metadata)?;
        if class.access_flags & 0x200 != 0 {
            return Err(SuperError::Interface);
        }
        let companions = class
            .methods
            .iter()
            .filter(|m| m.key.name == reference.name && m.key.parameters == reference.parameters)
            .collect::<Vec<_>>();
        let mut ignored_bridges = BTreeSet::new();
        if companions.len() > 1 {
            let mut distinct = BTreeSet::new();
            if companions.iter().any(|m| !distinct.insert(&m.key)) {
                return Err(SuperError::Selection);
            }
            // A bridge-selected DEX descriptor stays closed: Java source must
            // select the original exact nonbridge descriptor, not its erased
            // companion whose invokevirtual can dispatch to a later override.
            let candidates = companions
                .iter()
                .copied()
                .filter(|m| {
                    m.key.return_type == reference.return_type
                        && m.metadata.access_flags & 0x40 == 0
                })
                .collect::<Vec<_>>();
            let [target] = candidates.as_slice() else {
                return Err(SuperError::Selection);
            };
            for companion in &companions {
                if std::ptr::eq(*companion, *target) {
                    continue;
                }
                if !attested_return_bridge(companion, target, provider, hierarchy) {
                    return Err(SuperError::Selection);
                }
                ignored_bridges.insert(companion.key.clone());
            }
        }
        ancestry.push(owner.clone());
        owner_seen |= owner == reference.owner;
        for method in class
            .methods
            .iter()
            .filter(|m| m.key.name == reference.name && !ignored_bridges.contains(&m.key))
        {
            if method.key.parameters == reference.parameters
                && method.key.return_type != reference.return_type
            {
                return Err(SuperError::Selection);
            }
            let applicable_arity = method.key.parameters.len() == reference.parameters.len();
            // Descriptor casts supply an exact fixed-arity candidate. A
            // differently parameterized bridge cannot beat that candidate;
            // variable-arity expansion is considered only after this phase.
            // A selected bridge or malformed varargs declaration stays closed.
            if method.metadata.access_flags & 0x40 != 0
                && method.key.parameters == reference.parameters
                || method.metadata.access_flags & 0x80 != 0
                    && !method
                        .key
                        .parameters
                        .last()
                        .is_some_and(|p| p.starts_with('['))
                || method.generic
                    && applicable_arity
                    && !provider.source_super_signature_is_erased(caller, &ancestry, &method.key)
            {
                return Err(SuperError::Overload);
            }
            if !matches!(aliases.method_name(&method.key),Some(MethodAlias::Ordinary(n)) if n==&reference.name)
            {
                return Err(SuperError::Aliases);
            }
            if owner_seen
                && method.key.parameters == reference.parameters
                && method.key.return_type == reference.return_type
                && method.metadata.access_flags & (2 | 8) == 0
            {
                symbolic_resolved = true;
            }
            if selected.is_none() && method.key.parameters == reference.parameters {
                if method.key.return_type != reference.return_type {
                    return Err(SuperError::Selection);
                }
                if method.metadata.access_flags & (2 | 8 | 0x400) != 0 {
                    return Err(SuperError::Selection);
                }
                selected = Some(method.clone());
            }
        }
        if symbolic_resolved && selected.is_some() {
            break;
        }
        next = class.superclass;
    }
    if !owner_seen {
        return Err(SuperError::NotAncestor);
    }
    if !symbolic_resolved {
        return Err(SuperError::Selection);
    }
    let method = selected.ok_or(SuperError::Selection)?;
    if method.metadata.access_flags & 7 == 0 {
        let owner_package = crate::java_access::package(&dotted(&method.key.owner)?)
            .map_err(|_| SuperError::Metadata)?
            .to_owned();
        for ancestor in ancestry.iter().take_while(|a| *a != &method.key.owner) {
            if crate::java_access::package(&dotted(ancestor)?).map_err(|_| SuperError::Metadata)?
                != owner_package
            {
                return Err(SuperError::Access);
            }
        }
    }
    let original_caller = dotted(caller)?;
    let original_owner = dotted(&method.key.owner)?;
    let emitted_caller = aliases.class_name(caller).ok_or(SuperError::Aliases)?;
    let emitted_owner = aliases
        .class_name(&method.key.owner)
        .ok_or(SuperError::Aliases)?;
    AccessContext {
        caller: &original_caller,
        owner: &original_owner,
        receiver: Some(&original_caller),
        hierarchy,
        nests: None,
        emitted_caller,
        emitted_owner,
    }
    .method_access(method.metadata.access_flags)
    .map_err(|_| SuperError::Access)?;
    Ok(SuperSelection {
        target: method.key,
        metadata: method.metadata,
        ancestry,
        interface_qualifier: None,
    })
}

#[path = "java_super_final_bridge_alias.rs"]
mod final_bridge_alias;

#[cfg(test)]
#[path = "java_super_generic_signature_tests.rs"]
mod generic_signature_tests;

#[cfg(test)]
#[path = "java_super_return_bridge_tests.rs"]
mod return_bridge_tests;

/// Select a bounded exact direct-interface default invocation. Static/interface
/// caller helpers and superclass-inherited defaults require different proofs.
pub fn select_interface_super(
    caller: &str,
    reference: &MethodKey,
    provider: &impl SuperMetadata,
    aliases: &JavaAliases,
) -> Result<SuperSelection, SuperError> {
    fn graph(
        owner: &str,
        provider: &impl SuperMetadata,
        active: &mut BTreeSet<String>,
        seen: &mut BTreeSet<String>,
    ) -> Result<(), SuperError> {
        if active.contains(owner) {
            return Err(SuperError::Metadata);
        }
        if !seen.insert(owner.to_owned()) {
            return Ok(());
        }
        let class = provider.class(owner).ok_or(SuperError::Metadata)?;
        if class.access_flags & 0x200 == 0 {
            return Err(SuperError::Interface);
        }
        active.insert(owner.to_owned());
        for parent in provider.interfaces(owner).ok_or(SuperError::Metadata)? {
            graph(&parent, provider, active, seen)?;
        }
        active.remove(owner);
        Ok(())
    }
    let class = provider.class(caller).ok_or(SuperError::Metadata)?;
    if class.access_flags & 0x200 != 0 || reference.name.starts_with('<') {
        return Err(SuperError::Interface);
    }
    let direct = provider.interfaces(caller).ok_or(SuperError::Metadata)?;
    if direct.iter().filter(|i| *i == &reference.owner).count() != 1 {
        return Err(SuperError::NotAncestor);
    }
    let mut selected_graph = BTreeSet::new();
    graph(
        &reference.owner,
        provider,
        &mut BTreeSet::new(),
        &mut selected_graph,
    )?;
    for other in direct.iter().filter(|i| *i != &reference.owner) {
        let mut reachable = BTreeSet::new();
        graph(other, provider, &mut BTreeSet::new(), &mut reachable)?;
        if reachable.contains(&reference.owner) {
            return Err(SuperError::Selection);
        }
        for owner in reachable {
            if provider
                .class(&owner)
                .ok_or(SuperError::Metadata)?
                .methods
                .iter()
                .any(|m| {
                    m.key.name == reference.name
                        && m.key.parameters == reference.parameters
                        && m.metadata.access_flags & (2 | 8) == 0
                })
            {
                return Err(SuperError::Selection);
            }
        }
    }
    // A superclass implementing this interface, or declaring an override,
    // invalidates direct qualified-super selection. Inspect complete ancestry.
    let mut parent = class.superclass;
    let mut parents = BTreeSet::new();
    while let Some(owner) = parent {
        if !parents.insert(owner.clone()) {
            return Err(SuperError::Metadata);
        }
        let inherited = provider.class(&owner).ok_or(SuperError::Metadata)?;
        for interface in provider.interfaces(&owner).ok_or(SuperError::Metadata)? {
            let mut reachable = BTreeSet::new();
            graph(&interface, provider, &mut BTreeSet::new(), &mut reachable)?;
            if reachable.contains(&reference.owner) {
                return Err(SuperError::Selection);
            }
        }
        if inherited.methods.iter().any(|m| {
            m.key.name == reference.name
                && m.key.parameters == reference.parameters
                && m.metadata.access_flags & (2 | 8) == 0
        }) {
            return Err(SuperError::Selection);
        }
        parent = inherited.superclass;
    }
    let target_class = provider
        .class(&reference.owner)
        .ok_or(SuperError::Metadata)?;
    if target_class.access_flags & 1 == 0 {
        return Err(SuperError::Access);
    }
    let candidates: Vec<_> = target_class
        .methods
        .iter()
        .filter(|m| m.key.name == reference.name && m.key.parameters == reference.parameters)
        .collect();
    let [target] = candidates.as_slice() else {
        return Err(SuperError::Selection);
    };
    if target.key != *reference
        || target.metadata.access_flags & (1 | 2 | 8 | 0x40 | 0x80 | 0x100 | 0x400) != 1
        || target.metadata.exceptions.is_none()
    {
        return Err(SuperError::Selection);
    }
    if target.generic
        && !provider.source_super_signature_is_erased(
            caller,
            std::slice::from_ref(&reference.owner),
            reference,
        )
    {
        return Err(SuperError::Overload);
    }
    if !matches!(aliases.method_name(reference), Some(MethodAlias::Ordinary(n)) if n == &reference.name)
        || aliases.class_name(caller).is_none()
    {
        return Err(SuperError::Aliases);
    }
    let qualifier = aliases
        .class_name(&reference.owner)
        .ok_or(SuperError::Aliases)?
        .to_owned();
    Ok(SuperSelection {
        target: target.key.clone(),
        metadata: target.metadata.clone(),
        ancestry: vec![reference.owner.clone()],
        interface_qualifier: Some(qualifier),
    })
}

#[cfg(test)]
#[path = "java_interface_super_tests.rs"]
mod interface_super_tests;
