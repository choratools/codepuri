//! Bounded, descriptor-preserving source names for genuine input bridge families.
use super::{JavaAliases, MethodAlias, MethodKey};
use crate::{
    java_external_metadata::ExternalMetadata,
    type_compatibility::{Compatibility, Hierarchy},
};
use backend_java::JavaType;
use dex_format::{
    DexFile,
    class::{ClassDef, EncodedMethod},
};
use std::collections::{BTreeMap, BTreeSet};

/// Exact original identities whose source names changed; no body/throws claim.
#[derive(Clone, Debug, Default)]
pub struct BridgeAliasReport {
    /// Successfully committed families, including input interface declarations.
    pub families: Vec<Vec<MethodKey>>,
    /// Original identities and their before/after source aliases.
    pub changes: Vec<(MethodKey, String, String)>,
}
/// Opaque, privately constructed evidence binding genuine input declarations
/// and bridge bytes to a complete narrow-descriptor source alias plan.
#[derive(Clone, Debug)]
pub struct FinalBridgeAliasProof {
    pub(crate) target: MethodKey,
    pub(crate) bridge: MethodKey,
    pub(crate) bridge_code: dex_format::code::CodeItem,
    pub(crate) typed_name: String,
    narrow: BTreeSet<MethodKey>,
    wide: BTreeMap<MethodKey, String>,
}
impl FinalBridgeAliasProof {
    /// Original exact final typed declaration.
    pub fn target(&self) -> &MethodKey {
        &self.target
    }
    /// Original exact wide bridge declaration, retained with its body.
    pub fn bridge(&self) -> &MethodKey {
        &self.bridge
    }
    /// Genuine original bridge code used to bind the metadata provider.
    pub fn bridge_code(&self) -> &dex_format::code::CodeItem {
        &self.bridge_code
    }
    /// Exact selected source spelling for the final typed declaration.
    pub fn typed_name(&self) -> &str {
        &self.typed_name
    }
    pub(super) fn still_matches(&self, aliases: &JavaAliases) -> bool {
        self.narrow.iter().all(|k| {
            aliases.methods.get(k) == Some(&MethodAlias::Ordinary(self.typed_name.clone()))
        }) && self
            .wide
            .iter()
            .all(|(k, name)| aliases.methods.get(k) == Some(&MethodAlias::Ordinary(name.clone())))
    }
}
struct Input<'a> {
    owners: BTreeMap<String, (&'a DexFile, &'a ClassDef)>,
    methods: BTreeMap<MethodKey, (&'a DexFile, &'a EncodedMethod)>,
    overload_counts: BTreeMap<(String, String, Vec<String>), usize>,
}

/// Select only a unique ordinary instance declaration satisfying every accessible
/// ancestor declaration. Complete hierarchy and declaration lists are mandatory.
pub(super) fn align_obligations(
    aliases: &mut JavaAliases,
    files: &[DexFile],
    external: &dyn ExternalMetadata,
    hierarchy: &Hierarchy,
) -> BridgeAliasReport {
    let Some(input) = Input::collect(files) else {
        return BridgeAliasReport::default();
    };
    let mut families = BTreeMap::<(String, String, Vec<String>), Vec<MethodKey>>::new();
    let mut declarations_by_owner = BTreeMap::<String, Vec<(MethodKey, u32)>>::new();
    for (k, (_, m)) in &input.methods {
        families
            .entry((k.owner.clone(), k.name.clone(), k.parameters.clone()))
            .or_default()
            .push(k.clone());
        declarations_by_owner
            .entry(k.owner.clone())
            .or_default()
            .push((k.clone(), m.access_flags));
    }
    let mut report = BridgeAliasReport::default();
    for ((owner, name, parameters), count) in &input.overload_counts {
        if *count < 2 || name.starts_with('<') {
            continue;
        }
        let family = families[&(owner.clone(), name.clone(), parameters.clone())].clone();
        let Some(ancestors) = input.ancestors(owner, external) else {
            continue;
        };
        let mut obligations = Vec::new();
        let mut complete = true;
        for ancestor in ancestors.iter().filter(|a| *a != owner) {
            let declarations: Option<Vec<_>> = if input.owners.contains_key(ancestor) {
                Some(
                    declarations_by_owner
                        .get(ancestor)
                        .cloned()
                        .unwrap_or_default(),
                )
            } else {
                external.declared_methods(ancestor).map(|ms| {
                    ms.into_iter()
                        .map(|m| (m.key, m.metadata.access_flags))
                        .collect()
                })
            };
            let Some(declarations) = declarations else {
                complete = false;
                break;
            };
            for (k, flags) in declarations {
                if k.owner != *ancestor {
                    complete = false;
                    break;
                }
                if k.name != *name || k.parameters != *parameters || flags & 0xa != 0 {
                    continue;
                }
                // Package-private declarations do not cross package boundaries.
                if flags & 0x5 == 0
                    && k.owner.rsplit_once('/').map(|p| p.0) != owner.rsplit_once('/').map(|p| p.0)
                {
                    continue;
                }
                obligations.push((k, flags));
            }
        }
        if !complete || obligations.is_empty() {
            continue;
        }
        let candidates: Vec<_> = family
            .iter()
            .filter(|k| {
                let flags = input.methods[*k].1.access_flags;
                flags & (0x2 | 0x8 | 0x40 | 0x1000) == 0
                    && obligations.iter().all(|(root, access)| {
                        access & 0x10 == 0
                            && (access & 1 == 0 || flags & 1 != 0)
                            && (access & 4 == 0 || flags & 5 != 0)
                            && (root.return_type == k.return_type
                                || java_class(&k.return_type)
                                    .zip(java_class(&root.return_type))
                                    .is_some_and(|(a, b)| {
                                        hierarchy.assignable(&a, &b) == Compatibility::Assignable
                                    }))
                    })
            })
            .collect();
        if candidates.len() != 1 {
            continue;
        }
        let selected = candidates[0];
        // A nonfinal owner permits descendants to override the broad JVM
        // descriptor alone. Renaming that bridge and emitting a compiler bridge
        // routes broad calls through the narrow slot, bypassing such overrides.
        // Input-only absence of descendants cannot exclude external subclasses.
        // Private delegates have no inherited virtual slot and remain eligible.
        if input.owners[owner].1.access_flags & 0x10 == 0
            && family
                .iter()
                .any(|k| k != selected && input.methods[k].1.access_flags & 2 == 0)
        {
            continue;
        }
        // Every nonprivate alternative can affect JVM descriptor dispatch,
        // even if it did not receive the base alias. Require the same pure
        // forwarding witness before a compiler may synthesize broad bridges.
        if family.iter().any(|k| {
            k != selected
                && input.methods[k].1.access_flags & 2 == 0
                && !pure_forward_bridge(&input, k, selected, hierarchy)
        }) {
            continue;
        }
        // Input roots must already agree on the source spelling. External roots
        // require an inventoried exact alias too; unknown names remain closed.
        let root_names: Option<BTreeSet<_>> = obligations
            .iter()
            .map(|(k, _)| match aliases.methods.get(k) {
                Some(MethodAlias::Ordinary(n)) => Some(n.clone()),
                _ => None,
            })
            .collect();
        let Some(root_names) = root_names else {
            continue;
        };
        if root_names.len() != 1 {
            continue;
        }
        let Some(base) = root_names.iter().next() else {
            continue;
        };
        let Some(MethodAlias::Ordinary(old)) = aliases.methods.get(selected).cloned() else {
            continue;
        };
        if &old == base {
            continue;
        }
        let holders: Vec<_> = family
            .iter()
            .filter(|k| aliases.methods.get(*k) == Some(&MethodAlias::Ordinary(base.clone())))
            .collect();
        if holders.len() != 1 {
            continue;
        }
        let displaced = holders[0];
        let displaced_flags = input.methods[displaced].1.access_flags;
        if displaced_flags & 2 == 0 && !pure_forward_bridge(&input, displaced, selected, hierarchy)
        {
            continue;
        }
        // Swapping existing spellings cannot allocate a conflicting identifier.
        aliases
            .methods
            .insert(selected.clone(), MethodAlias::Ordinary(base.clone()));
        aliases
            .methods
            .insert(displaced.clone(), MethodAlias::Ordinary(old.clone()));
        report
            .changes
            .push((selected.clone(), old.clone(), base.clone()));
        report.changes.push((displaced.clone(), base.clone(), old));
        report.families.push(family);
    }
    report
}
// Exact three-instruction bridge witness: invoke-virtual with unchanged incoming
// receiver/argument registers, move-result-object, return-object. No handlers,
// casts, synchronization, stores, or extra calls can pass this grammar.
fn pure_forward_bridge(
    input: &Input<'_>,
    bridge: &MethodKey,
    target: &MethodKey,
    hierarchy: &Hierarchy,
) -> bool {
    if !java_class(&target.return_type)
        .zip(java_class(&bridge.return_type))
        .is_some_and(|(a, b)| hierarchy.assignable(&a, &b) == Compatibility::Assignable)
    {
        return false;
    }
    let Some((dex, method)) = input.methods.get(bridge) else {
        return false;
    };
    if method.access_flags != 0x1041 {
        return false;
    }
    let Some(code) = &method.code else {
        return false;
    };
    let argc = 1 + bridge
        .parameters
        .iter()
        .map(|p| if p == "J" || p == "D" { 2 } else { 1 })
        .sum::<usize>();
    if argc > 5
        || code.ins_size as usize != argc
        || code.registers_size < code.ins_size
        || code.insns_size != 5
        || code.insns.len() != 10
        || !code.tries.is_empty()
        || !code.handlers.is_empty()
    {
        return false;
    }
    let w: Vec<_> = code
        .insns
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let args = [
        w[2] & 15,
        (w[2] >> 4) & 15,
        (w[2] >> 8) & 15,
        (w[2] >> 12) & 15,
        (w[0] >> 8) & 15,
    ];
    let start = code.registers_size - code.ins_size;
    w[0] & 255 == 0x6e
        && (w[0] >> 12) as usize == argc
        && args[..argc]
            .iter()
            .enumerate()
            .all(|(i, r)| *r == start + i as u16)
        && w[3] & 255 == 0x0c
        && w[4] & 255 == 0x11
        && w[3] >> 8 == w[4] >> 8
        && w[3] >> 8 < code.registers_size
        && key(dex, w[1] as u32).as_ref() == Some(target)
}

fn ty(dex: &DexFile, index: u32) -> Option<String> {
    Some(
        dex.string(dex.type_ids.get(index as usize)?.descriptor_idx)
            .ok()?
            .into(),
    )
}
fn key(dex: &DexFile, index: u32) -> Option<MethodKey> {
    let raw = dex.method_ids.get(index as usize)?;
    let proto = dex.proto_ids.get(raw.proto_idx as usize)?;
    Some(MethodKey {
        owner: ty(dex, u32::from(raw.class_idx))?,
        name: dex.string(raw.name_idx).ok()?.into(),
        parameters: proto
            .parameters
            .as_ref()
            .map(|p| p.types.iter().map(|i| ty(dex, u32::from(*i))).collect())
            .unwrap_or(Some(vec![]))?,
        return_type: ty(dex, proto.return_type_idx)?,
    })
}
fn java_class(descriptor: &str) -> Option<JavaType> {
    Some(JavaType::Class(
        descriptor
            .strip_prefix('L')?
            .strip_suffix(';')?
            .replace('/', "."),
    ))
}
fn same_signature(a: &MethodKey, b: &MethodKey) -> bool {
    a.name == b.name && a.parameters == b.parameters && a.return_type == b.return_type
}
type Signature = (String, Vec<String>, String);
fn signature(k: &MethodKey) -> Signature {
    (k.name.clone(), k.parameters.clone(), k.return_type.clone())
}
struct FamilyIndex {
    signatures: BTreeMap<Signature, Vec<MethodKey>>,
    named: BTreeMap<(String, Vec<String>), Vec<MethodKey>>,
    ancestors: BTreeMap<String, BTreeSet<String>>,
}
impl FamilyIndex {
    fn new(aliases: &JavaAliases) -> Self {
        let mut signatures = BTreeMap::<Signature, Vec<MethodKey>>::new();
        let mut named = BTreeMap::<(String, Vec<String>), Vec<MethodKey>>::new();
        for k in aliases.methods.keys() {
            signatures.entry(signature(k)).or_default().push(k.clone());
            named
                .entry((k.name.clone(), k.parameters.clone()))
                .or_default()
                .push(k.clone());
        }
        Self {
            signatures,
            named,
            ancestors: BTreeMap::new(),
        }
    }
}
impl<'a> Input<'a> {
    fn collect(files: &'a [DexFile]) -> Option<Self> {
        let mut owners = BTreeMap::new();
        let mut methods = BTreeMap::new();
        for dex in files {
            for class in &dex.classes {
                let owner = ty(dex, class.class_idx)?;
                if owners.insert(owner.clone(), (dex, class)).is_some()
                    || (class.class_data_off != 0 && class.class_data.is_none())
                    || (class.interfaces_off != 0 && class.interfaces.is_none())
                {
                    return None;
                }
                if let Some(data) = &class.class_data {
                    for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                        let k = key(dex, method.method_idx)?;
                        if k.owner != owner || methods.insert(k, (dex, method)).is_some() {
                            return None;
                        }
                    }
                }
            }
        }
        let mut overload_counts = BTreeMap::new();
        for k in methods.keys() {
            *overload_counts
                .entry((k.owner.clone(), k.name.clone(), k.parameters.clone()))
                .or_default() += 1;
        }
        Some(Self {
            owners,
            methods,
            overload_counts,
        })
    }
    fn ancestors(&self, owner: &str, external: &dyn ExternalMetadata) -> Option<BTreeSet<String>> {
        fn visit(
            input: &Input<'_>,
            external: &dyn ExternalMetadata,
            owner: &str,
            active: &mut BTreeSet<String>,
            done: &mut BTreeSet<String>,
        ) -> Option<()> {
            if done.contains(owner) {
                return Some(());
            }
            if active.len() >= 256 || !active.insert(owner.into()) {
                return None;
            }
            let parents = if let Some((dex, class)) = input.owners.get(owner) {
                let mut parents = vec![];
                if class.superclass_idx != u32::MAX {
                    parents.push(ty(dex, class.superclass_idx)?);
                }
                if let Some(list) = &class.interfaces {
                    for i in &list.types {
                        parents.push(ty(dex, u32::from(*i))?);
                    }
                }
                parents
            } else {
                let declarations = external.class_hierarchy(owner);
                if declarations.len() != 1 {
                    return None;
                }
                let d = &declarations[0];
                d.superclass
                    .iter()
                    .chain(&d.interfaces)
                    .map(|n| format!("L{};", n.replace('.', "/")))
                    .collect()
            };
            for parent in parents {
                visit(input, external, &parent, active, done)?;
            }
            active.remove(owner);
            done.insert(owner.into());
            Some(())
        }
        let mut done = BTreeSet::new();
        visit(self, external, owner, &mut BTreeSet::new(), &mut done)?;
        Some(done)
    }
    fn seed(&self, bridge: &MethodKey, hierarchy: &Hierarchy) -> Option<MethodKey> {
        let (dex, method) = self.methods.get(bridge)?;
        if !bridge.parameters.is_empty() || method.access_flags != 0x1041 {
            return None;
        }
        let code = method.code.as_ref()?;
        if code.ins_size != 1
            || code.registers_size == 0
            || code.insns_size != 5
            || code.insns.len() != 10
            || !code.tries.is_empty()
            || !code.handlers.is_empty()
        {
            return None;
        }
        let words = code
            .insns
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect::<Vec<_>>();
        let receiver = code.registers_size - 1;
        if receiver > 15
            || words[0] != 0x106e
            || words[2] != receiver
            || words[3] & 255 != 0x0c
            || words[4] & 255 != 0x11
            || words[3] >> 8 != words[4] >> 8
        {
            return None;
        }
        if words[3] >> 8 >= code.registers_size {
            return None;
        }
        let target = key(dex, u32::from(words[1]))?;
        if target.owner != bridge.owner
            || target.name != bridge.name
            || target.parameters != bridge.parameters
            || target.return_type == bridge.return_type
        {
            return None;
        }
        let (_, target_method) = self.methods.get(&target)?;
        if target_method.access_flags != 0x11 || target_method.code.is_none() {
            return None;
        }
        if hierarchy.assignable(
            &java_class(&target.return_type)?,
            &java_class(&bridge.return_type)?,
        ) != Compatibility::Assignable
        {
            return None;
        }
        if self.overload_counts.get(&(
            bridge.owner.clone(),
            bridge.name.clone(),
            bridge.parameters.clone(),
        )) != Some(&2)
        {
            return None;
        }
        Some(target)
    }
}
/// Atomically name eligible final typed targets while retaining broad families. The first
/// grammar is a zero-argument public bridge to a same-owner public-final target.
/// Unknown hierarchy, external source obligations, duplicates, private/static/
/// native family members and unavailable inventory retain existing names.
pub(super) fn align(
    aliases: &mut JavaAliases,
    files: &[DexFile],
    external: &dyn ExternalMetadata,
    hierarchy: &Hierarchy,
) -> BridgeAliasReport {
    let Some(input) = Input::collect(files) else {
        return BridgeAliasReport::default();
    };
    if files.iter().any(|dex| {
        (0..dex.method_ids.len())
            .any(|i| key(dex, i as u32).is_none_or(|k| !aliases.methods.contains_key(&k)))
    }) {
        return BridgeAliasReport::default();
    }
    let seeds = input
        .methods
        .keys()
        .filter_map(|bridge| {
            input
                .seed(bridge, hierarchy)
                .map(|target| (bridge.clone(), target))
        })
        .collect::<Vec<_>>();
    let mut report = BridgeAliasReport::default();
    let mut committed = BTreeSet::new();
    let mut index = FamilyIndex::new(aliases);
    for (bridge, target) in seeds {
        if committed.contains(&bridge) {
            continue;
        }
        let Some(plan) = family(
            aliases, &input, &bridge, &target, external, hierarchy, &mut index,
        ) else {
            continue;
        };
        if plan.0.iter().any(|key| committed.contains(key)) {
            continue;
        }
        let Some(MethodAlias::Ordinary(wide_name)) = aliases.methods.get(&target).cloned() else {
            continue;
        };
        let Some(bridge_code) = input.methods.get(&bridge).and_then(|(_, m)| m.code.clone()) else {
            continue;
        };
        for key in &plan.0 {
            let name = if same_signature(key, &target) {
                plan.1.clone()
            } else {
                wide_name.clone()
            };
            if let Some(MethodAlias::Ordinary(old)) = aliases.methods.get(key)
                && old != &name
            {
                report
                    .changes
                    .push((key.clone(), old.clone(), name.clone()));
            }
            aliases
                .methods
                .insert(key.clone(), MethodAlias::Ordinary(name));
            committed.insert(key.clone());
        }
        let proof = FinalBridgeAliasProof {
            target: target.clone(),
            bridge: bridge.clone(),
            bridge_code,
            typed_name: plan.1.clone(),
            narrow: plan
                .0
                .iter()
                .filter(|k| same_signature(k, &target))
                .cloned()
                .collect(),
            wide: aliases
                .methods
                .iter()
                .filter(|(k, _)| same_signature(k, &bridge))
                .filter_map(|(k, a)| {
                    if let MethodAlias::Ordinary(name) = a {
                        Some((k.clone(), name.clone()))
                    } else {
                        None
                    }
                })
                .collect(),
        };
        for key in &proof.narrow {
            aliases
                .final_bridge_targets
                .insert(key.clone(), proof.clone());
        }
        report.families.push(plan.0);
    }
    report
}
fn selected_class_declaration(
    input: &Input<'_>,
    owner: &str,
    prototype: &MethodKey,
) -> Option<MethodKey> {
    let mut owner = owner.to_owned();
    let mut seen = BTreeSet::new();
    while seen.len() < 256 && seen.insert(owner.clone()) {
        let (dex, class) = input.owners.get(&owner)?;
        let candidate = MethodKey {
            owner: owner.clone(),
            ..prototype.clone()
        };
        if input.methods.contains_key(&candidate) {
            return Some(candidate);
        }
        if class.superclass_idx == u32::MAX {
            return None;
        }
        owner = ty(dex, class.superclass_idx)?;
    }
    None
}
fn family(
    aliases: &JavaAliases,
    input: &Input<'_>,
    bridge: &MethodKey,
    target: &MethodKey,
    external: &dyn ExternalMetadata,
    hierarchy: &Hierarchy,
    index: &mut FamilyIndex,
) -> Option<(Vec<MethodKey>, String)> {
    if input.seed(bridge, hierarchy).as_ref() != Some(target) {
        return None;
    }
    let seed_ancestors = input.ancestors(&target.owner, external)?;
    let MethodAlias::Ordinary(wide_name) = aliases.methods.get(target)? else {
        return None;
    };
    if wide_name != &target.name {
        return None;
    }
    // Preserve the original broad source obligation. It is never renamed,
    // deleted, or replaced by a compiler-generated body in this grammar.
    let wide_roots = index
        .signatures
        .get(&signature(bridge))?
        .iter()
        .filter(|k| {
            input.methods.contains_key(*k) && *k != bridge && seed_ancestors.contains(&k.owner)
        })
        .collect::<Vec<_>>();
    if !wide_roots.iter().any(|k| {
        input
            .owners
            .get(&k.owner)
            .is_some_and(|(_, c)| c.access_flags & 0x200 != 0)
    }) {
        return None;
    }
    for root in wide_roots {
        if aliases.methods.get(root) != Some(&MethodAlias::Ordinary(wide_name.clone())) {
            return None;
        }
    }
    for owner in &seed_ancestors {
        if owner == &target.owner {
            continue;
        }
        if input.owners.contains_key(owner) {
            let broad = MethodKey {
                owner: owner.clone(),
                ..bridge.clone()
            };
            let count = input
                .overload_counts
                .get(&(
                    owner.clone(),
                    target.name.clone(),
                    target.parameters.clone(),
                ))
                .copied()
                .unwrap_or_default();
            if count > usize::from(input.methods.contains_key(&broad)) {
                return None;
            }
        } else if external.declared_methods(owner)?.iter().any(|m| {
            m.key.name == target.name
                && m.key.parameters == target.parameters
                && !same_signature(&m.key, bridge)
        }) {
            return None;
        }
    }
    let mut narrow = BTreeSet::new();
    for k in index.signatures.get(&signature(target))? {
        let ancestors = input.ancestors(&k.owner, external)?;
        if !ancestors.contains(&target.owner) {
            continue;
        }
        if selected_class_declaration(input, &k.owner, target).as_ref() != Some(target) {
            return None;
        }
        if aliases.methods.get(k) != Some(&MethodAlias::Ordinary(wide_name.clone())) {
            return None;
        }
        narrow.insert(k.clone());
    }
    if !narrow.contains(target) {
        return None;
    }
    let mut restored_wide = BTreeSet::new();
    let mut descendant_wide_override = false;
    for k in index
        .named
        .get(&(target.name.clone(), target.parameters.clone()))?
    {
        let parents = if let Some(a) = index.ancestors.get(&k.owner) {
            a.clone()
        } else {
            let a = input.ancestors(&k.owner, external)?;
            index.ancestors.insert(k.owner.clone(), a.clone());
            a
        };
        if !parents.contains(&target.owner) {
            continue;
        }
        if same_signature(k, target) {
            if !narrow.contains(k) {
                return None;
            }
            continue;
        }
        if !same_signature(k, bridge) {
            return None;
        }
        let selected = selected_class_declaration(input, &k.owner, bridge)?;
        if selected == *bridge {
            restored_wide.insert(k.clone());
        } else {
            let (_, m) = input.methods.get(&selected)?;
            if m.access_flags & 1 == 0
                || m.access_flags & (2 | 4 | 8 | 0x100) != 0
                || aliases.methods.get(k) != Some(&MethodAlias::Ordinary(wide_name.clone()))
                || aliases.methods.get(&selected) != Some(&MethodAlias::Ordinary(wide_name.clone()))
            {
                return None;
            }
            descendant_wide_override = true;
        }
    }
    if !restored_wide.contains(bridge) || !descendant_wide_override {
        return None;
    }
    let MethodAlias::Ordinary(preferred) = aliases.methods.get(bridge)? else {
        return None;
    };
    let available = |candidate: &str| {
        aliases.methods.iter().all(|(k, n)| {
            narrow.contains(k)
                || restored_wide.contains(k)
                || !matches!(n,MethodAlias::Ordinary(n) if n==candidate)
        })
    };
    let name = if preferred != wide_name && available(preferred) {
        preferred.clone()
    } else {
        (1..=1024)
            .map(|i| format!("{wide_name}$typed{i}"))
            .find(|n| available(n))?
    };
    // Plan contains narrow keys first and wide keys last, with a separate
    // application step retaining the original wide name.
    let mut keys = narrow.into_iter().collect::<Vec<_>>();
    keys.extend(restored_wide);
    Some((keys, name))
}

#[cfg(test)]
#[path = "java_aliases_bridge_tests.rs"]
mod tests;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod obligation_tests {
    use super::*;
    use crate::java_aliases::{AliasInventory, AliasMode};
    use crate::java_external_metadata::Classpath;
    use std::io::Read;

    #[test]
    #[ignore = "requires local Samsung Pass APK, Android35 and JDK17 metadata"]
    fn actual_samsung_return_collisions_and_failclosed_negatives() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let apk = std::fs::File::open(
            root.join("tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
        )
        .unwrap();
        let mut archive = zip::ZipArchive::new(apk).unwrap();
        let mut files = Vec::new();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).unwrap();
            if entry.name().ends_with(".dex") {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                files.push(DexFile::from_bytes(&bytes).unwrap());
            }
        }
        let cp = Classpath::from_paths([
            "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod",
            "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
        ])
        .unwrap();
        let mut inventory = AliasInventory::default();
        for dex in &files {
            for i in 0..dex.method_ids.len() {
                let k = key(dex, i as u32).unwrap();
                if k.name == "getAuthService" || (k.name == "c" && k.parameters == ["Lkt/f2;"]) {
                    inventory.add_method(k).unwrap();
                }
            }
        }
        let original = inventory.build(AliasMode::Unicode);
        let hierarchy =
            crate::java_hierarchy::build_with_metadata(&files.iter().collect::<Vec<_>>(), &cp)
                .unwrap();
        let input = Input::collect(&files).unwrap();
        let public = input.methods.keys().find(|k| k.owner == "Lcom/samsung/android/samsungpass/passservice/service/SamsungPassServiceImpl;" && k.name == "getAuthService" && k.return_type == "Lcom/samsung/android/samsungpass/sdk/IAuthService;").unwrap().clone();
        let private = input
            .methods
            .keys()
            .find(|k| {
                k.owner == public.owner
                    && k.name == public.name
                    && k.return_type != public.return_type
            })
            .unwrap()
            .clone();
        let real = input
            .methods
            .keys()
            .find(|k| {
                k.owner == "Lqr/k0;"
                    && k.name == "c"
                    && k.parameters == ["Lkt/f2;"]
                    && k.return_type == "Lnr/y0;"
            })
            .unwrap()
            .clone();
        let bridge = input
            .methods
            .keys()
            .find(|k| {
                k.owner == real.owner
                    && k.name == real.name
                    && k.parameters == real.parameters
                    && k.return_type == "Lnr/n;"
            })
            .unwrap()
            .clone();
        assert_eq!(input.methods[&private].1.access_flags, 18);
        assert_eq!(input.methods[&public].1.access_flags, 1);
        assert_eq!(input.methods[&bridge].1.access_flags, 4161);
        let mut aliases = original.clone();
        let report = aliases.align_override_obligations(&files, &cp, &hierarchy);
        assert_eq!(
            aliases.method_name(&public).unwrap(),
            &MethodAlias::Ordinary("getAuthService".into())
        );
        assert_ne!(
            aliases.method_name(&private).unwrap(),
            &MethodAlias::Ordinary("getAuthService".into())
        );
        // Actual qr/k0 is open: no input descendant inventory can rule out
        // legal external broad-only overrides. Its family stays unchanged.
        assert_eq!(input.owners[&real.owner].1.access_flags & 0x10, 0);
        assert_eq!(aliases.method_name(&real), original.method_name(&real));
        assert_eq!(aliases.method_name(&bridge), original.method_name(&bridge));
        assert!(
            !report
                .changes
                .iter()
                .any(|(k, _, _)| k == &real || k == &bridge)
        );
        // Flag mutation fixture proves the final-owner boundary separately;
        // the genuine APK bridge bytes and all ancestor metadata remain intact.
        let mut final_files = files.clone();
        for dex in &mut final_files {
            let indices: BTreeSet<_> = dex
                .classes
                .iter()
                .filter(|c| ty(dex, c.class_idx).as_deref() == Some(&real.owner))
                .map(|c| c.class_idx)
                .collect();
            for class in &mut dex.classes {
                if indices.contains(&class.class_idx) {
                    class.access_flags |= 0x10;
                }
            }
        }
        let mut final_aliases = original.clone();
        final_aliases.align_override_obligations(&final_files, &cp, &hierarchy);
        assert_eq!(
            final_aliases.method_name(&real),
            Some(&MethodAlias::Ordinary("c".into()))
        );
        assert_ne!(
            final_aliases.method_name(&bridge),
            Some(&MethodAlias::Ordinary("c".into()))
        );
        assert!(report.changes.iter().any(|(k, _, _)| k == &public));
        assert!(
            aliases
                .align_override_obligations(&files, &cp, &hierarchy)
                .changes
                .is_empty()
        );
        // Missing genuine external ancestors must never guess a selection.
        let mut unknown = original.clone();
        unknown.align_override_obligations(&files, &Classpath::default(), &hierarchy);
        assert_eq!(unknown.method_name(&public), original.method_name(&public));
        // An altered invocation opcode cannot attest the unchanged virtual
        // dispatch; bridge flags alone never authorize source reassignment.
        let mut effectful = final_files.clone();
        for dex in &mut effectful {
            let indices: BTreeSet<_> = (0..dex.method_ids.len())
                .filter(|i| key(dex, *i as u32).as_ref() == Some(&bridge))
                .map(|i| i as u32)
                .collect();
            for class in &mut dex.classes {
                if let Some(data) = &mut class.class_data {
                    for method in data
                        .direct_methods
                        .iter_mut()
                        .chain(&mut data.virtual_methods)
                    {
                        if indices.contains(&method.method_idx) {
                            method.code.as_mut().unwrap().insns[0] = 0x70;
                        }
                    }
                }
            }
        }
        let mut altered = original.clone();
        altered.align_override_obligations(&effectful, &cp, &hierarchy);
        assert_eq!(altered.method_name(&real), original.method_name(&real));
        // A second genuine invocation can repeat target side effects and rejects.
        let mut extra = final_files.clone();
        for dex in &mut extra {
            let indices: BTreeSet<_> = (0..dex.method_ids.len())
                .filter(|i| key(dex, *i as u32).as_ref() == Some(&bridge))
                .map(|i| i as u32)
                .collect();
            for class in &mut dex.classes {
                if let Some(data) = &mut class.class_data {
                    for method in data
                        .direct_methods
                        .iter_mut()
                        .chain(&mut data.virtual_methods)
                    {
                        if indices.contains(&method.method_idx) {
                            let code = method.code.as_mut().unwrap();
                            let extra_call = code.insns[..6].to_vec();
                            code.insns.splice(0..0, extra_call);
                            code.insns_size += 3;
                        }
                    }
                }
            }
        }
        let mut extra_aliases = original.clone();
        extra_aliases.align_override_obligations(&extra, &cp, &hierarchy);
        assert_eq!(
            extra_aliases.method_name(&real),
            original.method_name(&real)
        );
        // Focused Java fixture exercises the compiler-generated broad bridge
        // and exact selected alias under virtual dispatch. These are fixture
        // bodies, not reconstructed APK method bodies or full-app proof.
        let dir = std::env::temp_dir().join(format!("alias-obligation-jvm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = format!(
            r#"public class AliasOracle {{
 interface Wide {{ Object c(String s); }}
 interface Narrow extends Wide {{ String c(String s); }}
 static class Base implements Narrow {{
  public String c(String s) {{ return "base:"+s; }}
  public Object {displaced}(String s) {{ return c(s); }}
 }}
 static class Child extends Base {{ public String c(String s) {{ return "child:"+s; }} }}
 public static void main(String[] args) {{
  Base b = new Child(); Narrow n=b; Wide w=b;
  if(!"child:x".equals(n.c("x")) || !n.c("x").equals(w.c("x")) || !n.c("x").equals(b.{displaced}("x"))) throw new AssertionError();
 }}
}}"#,
            displaced = match final_aliases.method_name(&bridge).unwrap() {
                MethodAlias::Ordinary(n) => n,
                _ => panic!(),
            }
        );
        let source_path = dir.join("AliasOracle.java");
        std::fs::write(&source_path, source).unwrap();
        let javac = std::process::Command::new("/home/jtjisgod/tools/jdk-17.0.13+11/bin/javac")
            .arg(&source_path)
            .output()
            .unwrap();
        assert!(
            javac.status.success(),
            "{}",
            String::from_utf8_lossy(&javac.stderr)
        );
        let runtime = std::process::Command::new("/home/jtjisgod/tools/jdk-17.0.13+11/bin/java")
            .arg("-cp")
            .arg(&dir)
            .arg("AliasOracle")
            .output()
            .unwrap();
        assert!(
            runtime.status.success(),
            "{}",
            String::from_utf8_lossy(&runtime.stderr)
        );
        // Two ordinary public covariant candidates are ambiguous, even when
        // one is a narrower return. Mutate only declaration flags as a negative.
        let mut ambiguous = files.clone();
        for dex in &mut ambiguous {
            let indices: BTreeSet<_> = (0..dex.method_ids.len())
                .filter(|i| key(dex, *i as u32).as_ref() == Some(&private))
                .map(|i| i as u32)
                .collect();
            for class in &mut dex.classes {
                if let Some(data) = &mut class.class_data {
                    for method in data
                        .direct_methods
                        .iter_mut()
                        .chain(&mut data.virtual_methods)
                    {
                        if indices.contains(&method.method_idx) {
                            method.access_flags = 1;
                        }
                    }
                }
            }
        }
        let mut rejected = original.clone();
        rejected.align_override_obligations(&ambiguous, &cp, &hierarchy);
        assert_eq!(rejected.method_name(&public), original.method_name(&public));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod descendant_inventory_review {
    use super::*;
    use std::io::Read;
    #[test]
    #[ignore = "requires genuine local JDK17 and Python3"]
    fn broad_only_descendant_dispatch_requires_final_owner() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let result = std::process::Command::new("python3")
            .arg(root.join("bench/alias-descendant-dispatch-witness.py"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8(result.stdout).unwrap();
        assert!(
            stdout.contains("\"dispatch_preserved\": false"),
            "{}",
            stdout
        );
        println!("{}", stdout.trim());
    }
    #[test]
    #[ignore = "read-only genuine Samsung APK descendant inventory"]
    fn actual_samsung_k0_descendant_slots() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let apk = std::fs::File::open(
            root.join("tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
        )
        .unwrap();
        let mut archive = zip::ZipArchive::new(apk).unwrap();
        let mut files = Vec::new();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).unwrap();
            if entry.name().ends_with(".dex") {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                files.push(DexFile::from_bytes(&bytes).unwrap());
            }
        }
        let input = Input::collect(&files).unwrap();
        let owner = "Lqr/k0;";
        println!(
            "OWNER {} flags={:#x}",
            owner, input.owners[owner].1.access_flags
        );
        let mut children = BTreeMap::<String, Vec<String>>::new();
        for (name, (dex, class)) in &input.owners {
            if class.superclass_idx != u32::MAX {
                children
                    .entry(ty(dex, class.superclass_idx).unwrap())
                    .or_default()
                    .push(name.clone());
            }
        }
        let mut pending = children.get(owner).cloned().unwrap_or_default();
        let mut descendants = BTreeSet::new();
        while let Some(child) = pending.pop() {
            if !descendants.insert(child.clone()) {
                continue;
            }
            pending.extend(children.get(&child).cloned().unwrap_or_default());
        }
        assert_eq!(
            descendants,
            BTreeSet::from(["Lds/d;".into(), "Lds/f;".into(), "Lht/n0;".into()])
        );
        assert_eq!(input.owners[owner].1.access_flags, 1);
        assert!(!input.methods.keys().any(|k| descendants.contains(&k.owner)
            && k.name == "c"
            && k.parameters == ["Lkt/f2;"]));
        println!("DESCENDANTS count={}", descendants.len());
        for child in &descendants {
            let (dex, class) = input.owners[child];
            println!(
                "DESCENDANT {} parent={} flags={:#x}",
                child,
                ty(dex, class.superclass_idx).unwrap(),
                class.access_flags
            );
        }
        for (k, (_, m)) in &input.methods {
            if (k.owner == owner || descendants.contains(&k.owner))
                && k.name == "c"
                && k.parameters == ["Lkt/f2;"]
            {
                println!(
                    "SLOT {}->{}({:?}){} flags={:#x}",
                    k.owner, k.name, k.parameters, k.return_type, m.access_flags
                );
            }
        }
    }
}
