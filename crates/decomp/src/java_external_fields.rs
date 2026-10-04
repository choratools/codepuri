//! Descriptor-exact field resolution over APK declarations and trusted classpath.
use super::{ExternalMetadata, FieldMetadata};
use crate::java_aliases::FieldKey;
use dex_format::DexFile;
use std::collections::{BTreeMap, BTreeSet};

#[path = "java_field_publication.rs"]
pub mod publication;

/// Once-per-export original owner locations. Ambiguous owners remain ambiguous.
pub struct InputIndex<'a> {
    owners: BTreeMap<String, Vec<(&'a DexFile, &'a dex_format::class::ClassDef)>>,
    invalid: Option<FieldRequirement>,
}
impl<'a> InputIndex<'a> {
    /// Exact unique original class location, independent of member publication
    /// or initialization. Invalid or duplicate input owners remain unavailable.
    pub fn declared_class(
        &self,
        owner: &str,
    ) -> Option<(&'a DexFile, &'a dex_format::class::ClassDef)> {
        if self.invalid.is_some() {
            return None;
        }
        let [location] = self.owners.get(owner)?.as_slice() else {
            return None;
        };
        Some(*location)
    }
    /// Cache class definitions across every input pool before source selection.
    pub fn new(files: &'a [DexFile]) -> Self {
        let mut owners: BTreeMap<String, Vec<_>> = BTreeMap::new();
        let mut invalid = None;
        for dex in files {
            for class in &dex.classes {
                match ty(dex, class.class_idx) {
                    Ok(owner) => owners.entry(owner).or_default().push((dex, class)),
                    Err(error) => {
                        invalid.get_or_insert(error);
                    }
                }
            }
        }
        Self { owners, invalid }
    }
}

/// Typed fail-closed requirements for a field use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldRequirement {
    /// Missing exact field declaration.
    Declaration(FieldKey),
    /// Missing hierarchy declaration.
    Hierarchy(String),
    /// Multiple input classes or inherited declarations.
    Ambiguous(String),
    /// Invalid or cyclic declaration metadata.
    Invalid(String),
    /// Original-package, visibility, or protected receiver check failed.
    Access(FieldKey),
    /// Opcode staticness differs from declaration.
    Staticness(FieldKey),
}
/// Access must use original descriptors, before source alias allocation.
pub struct FieldAccessContext<'a> {
    /// Original caller class descriptor.
    pub caller: &'a str,
    /// Static type of an instance receiver; required for protected access.
    pub receiver: Option<&'a str>,
    /// Expected opcode staticness.
    pub is_static: bool,
}
/// Parent resolver uses `key.owner` for qualification and alias lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedField {
    /// Exact identity with actual declaring owner.
    pub key: FieldKey,
    /// Genuine declaration and ConstantValue, if present.
    pub metadata: FieldMetadata,
    /// External dependencies need no emitted APK field declaration.
    pub origin: FieldOrigin,
}
/// Proven declaration source, after whole-class APK precedence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldOrigin {
    /// Declaration belongs to one of the supplied DEX files.
    Input,
    /// Declaration attested by the caller's compile classpath.
    Classpath,
}
struct Node {
    flags: u32,
    superclass: Option<String>,
    interfaces: Vec<String>,
}
fn descriptor(name: &str) -> String {
    format!("L{};", name.replace('.', "/"))
}
fn package(name: &str) -> &str {
    name.rsplit_once('/').map_or("", |(p, _)| p)
}
fn ty(dex: &DexFile, index: u32) -> Result<String, FieldRequirement> {
    dex.type_id(index)
        .and_then(|t| dex.string(t.descriptor_idx))
        .map(str::to_owned)
        .map_err(|_| FieldRequirement::Invalid(format!("type {index}")))
}
struct Resolver<'a> {
    files: &'a [DexFile],
    external: &'a dyn ExternalMetadata,
    index: Option<&'a InputIndex<'a>>,
}
impl Resolver<'_> {
    fn source_named(
        &self,
        owner: &str,
        name: &str,
        aliases: &crate::java_aliases::JavaAliases,
        path: &mut BTreeSet<String>,
    ) -> Result<BTreeSet<FieldKey>, FieldRequirement> {
        if !path.insert(owner.into()) {
            return Err(FieldRequirement::Invalid(owner.into()));
        }
        let node = self.node(owner)?;
        let mut found = BTreeSet::new();
        if let Some((dex, class)) = self.input(owner)? {
            if class.class_data_off != 0 && class.class_data.is_none() {
                return Err(FieldRequirement::Invalid(owner.into()));
            }
            if let Some(data) = &class.class_data {
                for f in data.static_fields.iter().chain(&data.instance_fields) {
                    let key = publication::operand_key(dex, f.field_idx)?;
                    if key.owner != owner {
                        return Err(FieldRequirement::Invalid(owner.into()));
                    }
                    let alias = aliases
                        .field_name(&key)
                        .ok_or_else(|| FieldRequirement::Invalid(owner.into()))?;
                    if alias == name {
                        found.insert(key);
                    }
                }
            }
        } else {
            for field in self
                .external
                .declared_fields(owner)
                .ok_or_else(|| FieldRequirement::Hierarchy(owner.into()))?
            {
                let key = FieldKey {
                    owner: field.owner,
                    name: field.name,
                    descriptor: field.descriptor,
                };
                if key.owner != owner {
                    return Err(FieldRequirement::Invalid(owner.into()));
                }
                // External members retain their genuine compile-classpath names.
                if key.name == name {
                    found.insert(key);
                }
            }
        }
        if found.is_empty() {
            for parent in node.superclass.iter().chain(&node.interfaces) {
                found.extend(self.source_named(parent, name, aliases, path)?);
            }
        }
        path.remove(owner);
        Ok(found)
    }
    fn named(
        &self,
        owner: &str,
        name: &str,
        path: &mut BTreeSet<String>,
    ) -> Result<BTreeSet<FieldKey>, FieldRequirement> {
        if !path.insert(owner.into()) {
            return Err(FieldRequirement::Invalid(owner.into()));
        }
        let node = self.node(owner)?;
        let mut found = BTreeSet::new();
        if let Some((dex, c)) = self.input(owner)? {
            if let Some(data) = &c.class_data {
                for f in data.static_fields.iter().chain(&data.instance_fields) {
                    let id = dex
                        .field_id(f.field_idx)
                        .map_err(|_| FieldRequirement::Invalid(owner.into()))?;
                    if dex
                        .string(id.name_idx)
                        .map_err(|_| FieldRequirement::Invalid(owner.into()))?
                        == name
                    {
                        found.insert(FieldKey {
                            owner: owner.into(),
                            name: name.into(),
                            descriptor: ty(dex, u32::from(id.type_idx))?,
                        });
                    }
                }
            }
        } else {
            for f in self
                .external
                .declared_fields(owner)
                .ok_or_else(|| FieldRequirement::Hierarchy(owner.into()))?
            {
                if f.name == name {
                    found.insert(FieldKey {
                        owner: owner.into(),
                        name: name.into(),
                        descriptor: f.descriptor,
                    });
                }
            }
        }
        if found.is_empty() {
            for parent in node.superclass.iter().chain(&node.interfaces) {
                found.extend(self.named(parent, name, path)?);
            }
        }
        path.remove(owner);
        Ok(found)
    }
    fn input(
        &self,
        owner: &str,
    ) -> Result<Option<(&DexFile, &dex_format::class::ClassDef)>, FieldRequirement> {
        if let Some(index) = self.index {
            if let Some(error) = &index.invalid {
                return Err(error.clone());
            }
            return match index.owners.get(owner).map(Vec::as_slice) {
                None => Ok(None),
                Some([location]) => Ok(Some(*location)),
                Some(_) => Err(FieldRequirement::Ambiguous(owner.into())),
            };
        }
        let mut found = None;
        for dex in self.files {
            for c in &dex.classes {
                if ty(dex, c.class_idx)? == owner {
                    if found.is_some() {
                        return Err(FieldRequirement::Ambiguous(owner.into()));
                    }
                    found = Some((dex, c));
                }
            }
        }
        Ok(found)
    }
    fn node(&self, owner: &str) -> Result<Node, FieldRequirement> {
        if let Some((dex, c)) = self.input(owner)? {
            return Ok(Node {
                flags: c.access_flags,
                superclass: if c.superclass_idx == u32::MAX {
                    None
                } else {
                    Some(ty(dex, c.superclass_idx)?)
                },
                interfaces: c
                    .interfaces
                    .as_ref()
                    .map(|i| i.types.iter().map(|t| ty(dex, u32::from(*t))).collect())
                    .unwrap_or(Ok(Vec::new()))?,
            });
        }
        let nodes = self.external.class_hierarchy(owner);
        if nodes.len() > 1 {
            return Err(FieldRequirement::Ambiguous(owner.into()));
        }
        let n = nodes
            .into_iter()
            .next()
            .ok_or_else(|| FieldRequirement::Hierarchy(owner.into()))?;
        if descriptor(&n.name) != owner {
            return Err(FieldRequirement::Invalid(owner.into()));
        }
        Ok(Node {
            flags: self
                .external
                .class_access(owner)
                .ok_or_else(|| FieldRequirement::Hierarchy(owner.into()))?,
            superclass: n.superclass.map(|s| descriptor(&s)),
            interfaces: n.interfaces.iter().map(|s| descriptor(s)).collect(),
        })
    }
    fn declared(&self, key: &FieldKey) -> Result<Option<FieldMetadata>, FieldRequirement> {
        let Some((dex, c)) = self.input(&key.owner)? else {
            return Ok(self.external.field(key));
        };
        let mut found = None;
        if c.class_data_off != 0 && c.class_data.is_none() {
            return Err(FieldRequirement::Invalid(key.owner.clone()));
        }
        if let Some(data) = &c.class_data {
            for (is_static, fields) in [(true, &data.static_fields), (false, &data.instance_fields)]
            {
                for f in fields {
                    let id = dex
                        .field_id(f.field_idx)
                        .map_err(|_| FieldRequirement::Invalid(key.owner.clone()))?;
                    let name = dex
                        .string(id.name_idx)
                        .map_err(|_| FieldRequirement::Invalid(key.owner.clone()))?;
                    let field_type = ty(dex, u32::from(id.type_idx))?;
                    if name == key.name && field_type == key.descriptor {
                        if found.is_some() {
                            return Err(FieldRequirement::Ambiguous(key.owner.clone()));
                        }
                        if ty(dex, u32::from(id.class_idx))? != key.owner
                            || (f.access_flags & 8 != 0) != is_static
                            || f.access_flags
                                & !(1 | 2 | 4 | 8 | 0x10 | 0x40 | 0x80 | 0x1000 | 0x4000)
                                != 0
                            || (f.access_flags & 7).count_ones() > 1
                            || f.access_flags & 0x50 == 0x50
                            || crate::java_descriptors::parse_type_descriptor(&field_type).is_err()
                            || field_type == "V"
                            || (f.access_flags & 0x4000 != 0
                                && (c.access_flags & 0x4000 == 0
                                    || f.access_flags & 0x18 != 0x18
                                    || field_type != key.owner))
                        {
                            return Err(FieldRequirement::Invalid(key.owner.clone()));
                        }
                        // DEX encoded values are not JVM ConstantValue attestations.
                        found = Some(FieldMetadata {
                            owner: key.owner.clone(),
                            name: name.into(),
                            descriptor: field_type,
                            access_flags: f.access_flags,
                            signature: None,
                            constant_value: None,
                            attributes: Vec::new(),
                        });
                    }
                }
            }
        }
        Ok(found)
    }
    fn lookup(
        &self,
        key: &FieldKey,
        path: &mut BTreeSet<String>,
    ) -> Result<Option<ResolvedField>, FieldRequirement> {
        if !path.insert(key.owner.clone()) {
            return Err(FieldRequirement::Invalid(key.owner.clone()));
        }
        let result = self.lookup_inner(key, path);
        path.remove(&key.owner);
        result
    }
    fn lookup_inner(
        &self,
        key: &FieldKey,
        path: &mut BTreeSet<String>,
    ) -> Result<Option<ResolvedField>, FieldRequirement> {
        let node = self.node(&key.owner)?;
        if let Some(metadata) = self.declared(key)? {
            if metadata.owner != key.owner
                || metadata.name != key.name
                || metadata.descriptor != key.descriptor
            {
                return Err(FieldRequirement::Invalid(key.owner.clone()));
            }
            return Ok(Some(ResolvedField {
                key: key.clone(),
                metadata,
                origin: if self.input(&key.owner)?.is_some() {
                    FieldOrigin::Input
                } else {
                    FieldOrigin::Classpath
                },
            }));
        }
        let mut interfaces = node.interfaces;
        interfaces.sort();
        interfaces.dedup();
        let mut found: Option<ResolvedField> = None;
        for owner in interfaces {
            if let Some(field) = self.lookup(
                &FieldKey {
                    owner,
                    ..key.clone()
                },
                path,
            )? {
                if found.as_ref().is_some_and(|f| f.key != field.key) {
                    return Err(FieldRequirement::Ambiguous(key.owner.clone()));
                }
                found = Some(field);
            }
        }
        if found.is_some() {
            return Ok(found);
        }
        match node.superclass {
            Some(owner) => self.lookup(
                &FieldKey {
                    owner,
                    ..key.clone()
                },
                path,
            ),
            None => Ok(None),
        }
    }
    fn subtype(
        &self,
        child: &str,
        parent: &str,
        path: &mut BTreeSet<String>,
    ) -> Result<bool, FieldRequirement> {
        if child == parent {
            return Ok(true);
        }
        if !path.insert(child.into()) {
            return Err(FieldRequirement::Invalid(child.into()));
        }
        let n = self.node(child)?;
        for next in n.superclass.iter().chain(&n.interfaces) {
            if self.subtype(next, parent, path)? {
                path.remove(child);
                return Ok(true);
            }
        }
        path.remove(child);
        Ok(false)
    }
}

/// Adapter for backend `retain_field_receiver`. Differently typed name shadows
/// also reject retention; use the actual declaring-owner cast instead.
pub fn retain_field_receiver(
    files: &[DexFile],
    field: &ResolvedField,
    external: &dyn ExternalMetadata,
    context: &FieldAccessContext<'_>,
) -> Result<bool, FieldRequirement> {
    let Some(receiver) = context.receiver else {
        return Ok(false);
    };
    if field.metadata.access_flags & 5 == 0 && receiver != field.key.owner {
        return Ok(false);
    }
    let r = Resolver {
        files,
        external,
        index: None,
    };
    if !r.subtype(receiver, &field.key.owner, &mut BTreeSet::new())? {
        return Ok(false);
    }
    let names = r.named(receiver, &field.key.name, &mut BTreeSet::new())?;
    if names.len() != 1 || !names.contains(&field.key) {
        return Ok(false);
    }
    let resolved = resolve_field(
        files,
        &FieldKey {
            owner: receiver.into(),
            ..field.key.clone()
        },
        external,
        context,
    )?;
    Ok(resolved.key == field.key)
}

/// Receiver proof in the frozen alias domain, after original-scope access proof.
/// An alias collision in a subclass can hide a differently named DEX member.
pub fn retain_field_receiver_with_aliases(
    files: &[DexFile],
    field: &ResolvedField,
    external: &dyn ExternalMetadata,
    context: &FieldAccessContext<'_>,
    aliases: &crate::java_aliases::JavaAliases,
) -> Result<bool, FieldRequirement> {
    retain_field_receiver_with_aliases_indexed(files, field, external, context, aliases, None)
}
pub(crate) fn retain_field_receiver_with_aliases_indexed(
    files: &[DexFile],
    field: &ResolvedField,
    external: &dyn ExternalMetadata,
    context: &FieldAccessContext<'_>,
    aliases: &crate::java_aliases::JavaAliases,
    index: Option<&InputIndex<'_>>,
) -> Result<bool, FieldRequirement> {
    let proven = resolve_field_indexed(files, &field.key, external, context, index)?;
    if proven != *field {
        return Err(FieldRequirement::Invalid(field.key.owner.clone()));
    }
    let Some(receiver) = context.receiver else {
        return Ok(false);
    };
    // Accessibility from the caller does not imply inheritance into the static
    // receiver type. Such fields require the actual declaring-owner cast.
    if field.metadata.access_flags & 5 == 0 && receiver != field.key.owner {
        return Ok(false);
    }
    let r = Resolver {
        files,
        external,
        index,
    };
    if !r.subtype(receiver, &field.key.owner, &mut BTreeSet::new())? {
        return Ok(false);
    }
    let name = if field.origin == FieldOrigin::Classpath {
        field.key.name.as_str()
    } else {
        aliases
            .field_name(&field.key)
            .ok_or_else(|| FieldRequirement::Invalid(field.key.owner.clone()))?
    };
    let found = r.source_named(receiver, name, aliases, &mut BTreeSet::new())?;
    Ok(found.len() == 1 && found.contains(&field.key))
}
/// Look up a genuine declaration independently of a particular use-site.
/// This is metadata, never an access/publication attestation: callers must also
/// run [`resolve_field`] with the original caller and actual SSA receiver type.
/// APK precedence, hierarchy/cycle/ambiguity checks and opcode staticness remain
/// enforced; protected or private metadata can be inspected without permission.
pub fn resolve_field_declaration(
    files: &[DexFile],
    key: &FieldKey,
    external: &dyn ExternalMetadata,
    is_static: bool,
) -> Result<ResolvedField, FieldRequirement> {
    resolve_field_declaration_indexed(files, key, external, is_static, None)
}
pub(crate) fn resolve_field_declaration_indexed(
    files: &[DexFile],
    key: &FieldKey,
    external: &dyn ExternalMetadata,
    is_static: bool,
    index: Option<&InputIndex<'_>>,
) -> Result<ResolvedField, FieldRequirement> {
    let r = Resolver {
        files,
        external,
        index,
    };
    let field = r
        .lookup(key, &mut BTreeSet::new())?
        .ok_or_else(|| FieldRequirement::Declaration(key.clone()))?;
    if (field.metadata.access_flags & 8 != 0) != is_static {
        return Err(FieldRequirement::Staticness(field.key));
    }
    Ok(field)
}

/// Resolve inherited fields with APK/cross-DEX class priority. Missing input
/// members never fall back to a shadowed external class. No aliases are guessed.
pub fn resolve_field(
    files: &[DexFile],
    key: &FieldKey,
    external: &dyn ExternalMetadata,
    context: &FieldAccessContext<'_>,
) -> Result<ResolvedField, FieldRequirement> {
    resolve_field_indexed(files, key, external, context, None)
}
pub(crate) fn resolve_field_indexed(
    files: &[DexFile],
    key: &FieldKey,
    external: &dyn ExternalMetadata,
    context: &FieldAccessContext<'_>,
    index: Option<&InputIndex<'_>>,
) -> Result<ResolvedField, FieldRequirement> {
    let r = Resolver {
        files,
        external,
        index,
    };
    let field = resolve_field_declaration_indexed(files, key, external, context.is_static, index)?;
    let flags = field.metadata.access_flags;
    let same_package = package(context.caller) == package(&field.key.owner);
    let accessible = if flags & 1 != 0 {
        true
    } else if flags & 2 != 0 {
        context.caller == field.key.owner
    } else if same_package {
        true
    } else if flags & 4 != 0 && r.subtype(context.caller, &field.key.owner, &mut BTreeSet::new())? {
        context.is_static
            || match context.receiver {
                Some(receiver) => r.subtype(receiver, context.caller, &mut BTreeSet::new())?,
                None => false,
            }
    } else {
        false
    };
    for owner in [&key.owner, &field.key.owner] {
        if r.node(owner)?.flags & 1 == 0 && package(context.caller) != package(owner) {
            return Err(FieldRequirement::Access(field.key.clone()));
        }
    }
    if !accessible {
        return Err(FieldRequirement::Access(field.key));
    }
    Ok(field)
}
