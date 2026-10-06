//! Whole-input naming and conservative class-member closure for Java export.
#[path = "java_field_receiver_projection.rs"]
mod field_receiver_projection;

use crate::java_members::external_metadata::{
    ExternalMetadata, MetadataRequirement, MethodMetadata,
};
use crate::java_members::{
    external_metadata::fields::publication::FieldPublication,
    source_closure::{EmittedMethod, GlobalDeclarations},
};
use crate::{
    dex_java::{ClassProfile, DexJavaResolver, JavaOptions, JavaProfile, JavaReport, Timer, stage},
    java_aliases::*,
    java_class_kinds::*,
    java_initial_parameters::{self, InitialParameter},
    java_members::{MemberRequirements, MemberResolver, type_source},
    java_method_flags::*,
    output::*,
};
use backend_java::{
    DeclarationBindings, JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings,
    SourceResult,
};
use dex_format::{
    DexFile,
    class::{ClassDef, EncodedMethod},
};
use managed_ir::{
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
    ops::InvokeKind,
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};
#[cfg(test)]
#[path = "java_annotation_dependency_tests.rs"]
mod annotation_dependency_tests;
#[cfg(test)]
#[path = "java_constructor_chain_tests.rs"]
mod constructor_chain_tests;
#[path = "java_constructor_delegation.rs"]
mod constructor_delegation;
#[cfg(test)]
#[path = "java_constructor_exception_identity_tests.rs"]
mod constructor_exception_identity_tests;
#[cfg(test)]
#[path = "java_enum_production_wave17_tests.rs"]
mod enum_production_wave17_tests;
#[cfg(test)]
#[path = "java_exception_dispatch_facade_tests.rs"]
mod exception_dispatch_facade_tests;
#[cfg(test)]
#[path = "java_external_nonconstant_source_tests.rs"]
mod external_nonconstant_source_tests;
#[cfg(test)]
#[path = "java_integration_index_tests.rs"]
mod index_tests;
#[path = "java_receiver_methods.rs"]
mod receiver_methods;
#[path = "java_resolution_cache.rs"]
mod resolution_cache;
#[path = "java_scheduler.rs"]
pub(crate) mod scheduler;
#[path = "java_source_throws_integration.rs"]
mod source_throws;
#[path = "java_ssa_cache.rs"]
mod ssa_cache;
#[cfg(test)]
#[path = "java_static_publication_tests.rs"]
mod static_publication_tests;
#[cfg(test)]
#[path = "java_super_binding_tests.rs"]
mod super_binding_tests;

fn descriptor(dex: &DexFile, index: u32) -> Option<&str> {
    dex.string(dex.type_id(index).ok()?.descriptor_idx).ok()
}
fn managed_type_to_java(t: &managed_ir::types::ManagedType, r: &Resolver<'_>) -> Option<JavaType> {
    use managed_ir::types::ManagedType;
    Some(match t {
        ManagedType::Boolean => JavaType::Boolean,
        ManagedType::Byte => JavaType::Byte,
        ManagedType::Short => JavaType::Short,
        ManagedType::Char => JavaType::Char,
        ManagedType::Int | ManagedType::Zero => JavaType::Int,
        ManagedType::Long => JavaType::Long,
        ManagedType::Float => JavaType::Float,
        ManagedType::Double => JavaType::Double,
        ManagedType::Reference(c) => JavaType::Class(r.class(*c)?),
        ManagedType::Array { element, .. } => {
            JavaType::Array(Box::new(managed_type_to_java(element, r)?))
        }
        ManagedType::Null => JavaType::Class("java.lang.Object".into()),
        ManagedType::Unknown
        | ManagedType::UninitializedThis
        | ManagedType::UninitializedNew { .. }
        | ManagedType::Conflict => return None,
    })
}
fn method_key(dex: &DexFile, index: u32) -> Option<MethodKey> {
    let m = dex.method_id(index).ok()?;
    let p = dex.proto_id(u32::from(m.proto_idx)).ok()?;
    Some(MethodKey {
        owner: descriptor(dex, u32::from(m.class_idx))?.into(),
        name: dex.string(m.name_idx).ok()?.into(),
        parameters: p
            .parameters
            .iter()
            .flat_map(|p| &p.types)
            .map(|t| descriptor(dex, u32::from(*t)).map(str::to_owned))
            .collect::<Option<_>>()?,
        return_type: descriptor(dex, p.return_type_idx)?.into(),
    })
}
fn field_key(dex: &DexFile, index: u32) -> Option<FieldKey> {
    let f = dex.field_id(index).ok()?;
    Some(FieldKey {
        owner: descriptor(dex, u32::from(f.class_idx))?.into(),
        name: dex.string(f.name_idx).ok()?.into(),
        descriptor: descriptor(dex, u32::from(f.type_idx))?.into(),
    })
}
fn build_alias_inventory(files: &[DexFile]) -> Result<AliasInventory, OutputError> {
    let mut inventory = AliasInventory::default();
    let invalid = |s: String| OutputError::InvalidDescriptor(s);
    for dex in files {
        for ty in &dex.type_ids {
            inventory
                .add_type(
                    dex.string(ty.descriptor_idx)
                        .map_err(|e| invalid(e.to_string()))?,
                )
                .map_err(invalid)?;
        }
        for i in 0..dex.method_ids.len() {
            inventory
                .add_method(
                    method_key(dex, i as u32).ok_or_else(|| invalid("method inventory".into()))?,
                )
                .map_err(invalid)?;
        }
        for i in 0..dex.field_ids.len() {
            inventory
                .add_field(
                    field_key(dex, i as u32).ok_or_else(|| invalid("field inventory".into()))?,
                )
                .map_err(invalid)?;
        }
    }
    Ok(inventory)
}
fn extend_alias_inventory(
    files: &[DexFile],
    inventory: &mut AliasInventory,
    super_index: Option<&crate::java_super_resolution::SuperIndex<'_>>,
) -> Result<(), OutputError> {
    let invalid = |s: String| OutputError::InvalidDescriptor(s);
    if let Some(index) = super_index {
        crate::java_super_resolution::inventory_ancestors(files, index, inventory)
            .map_err(invalid)?;
    }
    Ok(())
}
fn collect_field_publication(
    files: &[DexFile],
    mut inventory: AliasInventory,
    writes: &initializers::write_inventory::WriteInventory,
) -> Result<(JavaAliases, FieldPublication), OutputError> {
    // The backend's conservative identifier validator currently accepts ASCII.
    let publication =
        FieldPublication::collect_analyst_with_explicit_stores(files, &mut inventory, writes)
            .map_err(|e| {
                OutputError::InvalidDescriptor(format!("global field inventory: {e:?}"))
            })?;
    Ok((inventory.build(AliasMode::Ascii), publication))
}
#[cfg(test)]
fn export_inventory(
    files: &[DexFile],
    profile: &mut Option<&mut JavaProfile>,
    writes: &initializers::write_inventory::WriteInventory,
    super_index: Option<&crate::java_super_resolution::SuperIndex<'_>>,
) -> Result<(JavaAliases, FieldPublication), OutputError> {
    let timer = Timer::new(
        profile
            .as_deref_mut()
            .map(|p| p.serial.entry("alias_inventory").or_default()),
    );
    let mut inventory = build_alias_inventory(files)?;
    extend_alias_inventory(files, &mut inventory, super_index)?;
    drop(timer);
    let _timer = Timer::new(
        profile
            .as_deref_mut()
            .map(|p| p.serial.entry("alias_build").or_default()),
    );
    collect_field_publication(files, inventory, writes)
}
#[cfg(test)]
fn inventory(
    files: &[DexFile],
    profile: &mut Option<&mut JavaProfile>,
) -> Result<JavaAliases, OutputError> {
    let writes = initializers::write_inventory::WriteInventory::scan(files);
    export_inventory(files, profile, &writes, None).map(|(aliases, _)| aliases)
}
struct MissingMetadata;
impl ExternalMetadata for MissingMetadata {
    fn method(&self, _: &MethodKey) -> Option<MethodMetadata> {
        None
    }
}
type FieldUses = Vec<(FieldKey, Option<String>, bool)>;
type PublishedFieldUses = Mutex<BTreeMap<MethodKey, FieldUses>>;
#[derive(Clone)]
struct BoundSuper {
    interface_qualifier: Option<String>,
    key: MethodKey,
    metadata: MethodMetadata,
    method: JavaMethod,
}
struct StoredConstructorException {
    ssa: Arc<dex_decompiler::ssa::SsaMethod>,
    metadata: crate::java_exception_regions::DisjointMetadata,
}
struct ConstructorExceptionScopes(Arc<StoredConstructorException>);
impl backend_java::constructors::ConstructorExceptionMetadata for ConstructorExceptionScopes {
    fn scopes(&self) -> Vec<&dyn backend_java::exceptions::RegionMetadata> {
        self.0
            .metadata
            .scopes
            .iter()
            .map(|scope| scope as &dyn backend_java::exceptions::RegionMetadata)
            .collect()
    }
}
struct Resolver<'a> {
    source_release: u16,
    ssa_cache: Option<&'a ssa_cache::SsaCache>,
    cache_inventory: bool,
    method_cache: Option<resolution_cache::LookupCache<JavaMethod>>,
    field_cache: Option<resolution_cache::LookupCache<JavaField>>,
    field_use_cache: Option<
        &'a crate::java_members::external_metadata::fields::publication::cache::FieldUseCache<'a>,
    >,
    member_index: Option<&'a crate::java_members::MemberIndex<'a>>,
    source_throws: Option<&'a source_throws::SourceThrowsPlan>,
    field_throw_catalog: Option<&'a crate::java_checked_bound_targets::FieldThrowCatalog>,
    checked_bridge_names: Mutex<BTreeMap<String, Option<BTreeSet<String>>>>,
    constructor_delegations: Mutex<BTreeMap<u32, Option<MethodRef>>>,
    constructor_exception_scopes: Mutex<BTreeMap<u32, Vec<Arc<StoredConstructorException>>>>,
    constructor_definitions: BTreeMap<u32, Option<&'a EncodedMethod>>,
    super_index: Option<&'a crate::java_super_resolution::SuperIndex<'a>>,
    cache_super_bindings: bool,
    super_bindings: Mutex<BTreeMap<(String, u32), Option<BoundSuper>>>,
    receiver_methods: Mutex<BTreeMap<(String, String, u32), Option<JavaMethod>>>,
    publications: Option<&'a Mutex<BTreeMap<MethodKey, EmittedMethod>>>,
    field_catalog: Option<&'a FieldPublication>,
    field_index: Option<&'a crate::java_members::external_metadata::fields::InputIndex<'a>>,
    field_uses: Option<&'a PublishedFieldUses>,
    published_owners: Option<&'a Mutex<BTreeMap<String, bool>>>,
    enum_fields: Option<&'a Mutex<BTreeSet<FieldKey>>>,
    annotation_defaults:
        Option<&'a Mutex<initializers::annotation_publication::PublishedEnumConstants>>,
    annotation_guards: Option<
        &'a Mutex<BTreeMap<String, Vec<initializers::annotation_publication::DeferredDefault>>>,
    >,
    defer_annotation_defaults: bool,
    original_names: BTreeMap<String, String>,
    class_catalog: BTreeMap<String, Option<backend_java::constructors::ConstructorClass>>,
    constructor_sets: BTreeMap<String, Vec<MethodRef>>,
    inputs: &'a [DexFile],
    dex_index: usize,
    writes: Option<&'a initializers::write_inventory::WriteInventory>,
    hierarchy: crate::type_compatibility::Hierarchy,
    original_hierarchy: Option<Arc<crate::type_compatibility::Hierarchy>>,
    dex: &'a DexFile,
    metadata: &'a dex_decompiler::infer::DexMetadata,
    aliases: &'a JavaAliases,
    methods: BTreeMap<u32, (u32, Option<Vec<String>>)>,
    fields: BTreeMap<u32, (u32, bool)>,
    static_positions: BTreeMap<(u32, u32), usize>,
    catalog: BTreeMap<MethodKey, Option<MethodMetadata>>,
    global_catalog: Option<&'a BTreeMap<MethodKey, Option<MethodMetadata>>>,
    field_names: BTreeMap<String, BTreeSet<String>>,
    external: Option<&'a dyn ExternalMetadata>,
    input_owners: BTreeSet<String>,
}
impl<'a> Resolver<'a> {
    #[cfg(test)]
    fn new(
        dex: &'a DexFile,
        aliases: &'a JavaAliases,
        metadata: &'a dex_decompiler::infer::DexMetadata,
    ) -> Self {
        Self::with_catalog(dex, aliases, metadata, None)
    }
    fn with_catalog(
        dex: &'a DexFile,
        aliases: &'a JavaAliases,
        metadata: &'a dex_decompiler::infer::DexMetadata,
        global_catalog: Option<&'a BTreeMap<MethodKey, Option<MethodMetadata>>>,
    ) -> Self {
        let mut r = Self {
            source_release: 17,
            ssa_cache: None,
            cache_inventory: false,
            method_cache: None,
            field_cache: None,
            field_use_cache: None,
            member_index: None,
            source_throws: None,
            field_throw_catalog: None,
            checked_bridge_names: Mutex::new(BTreeMap::new()),
            constructor_delegations: Mutex::new(BTreeMap::new()),
            constructor_exception_scopes: Mutex::new(BTreeMap::new()),
            constructor_definitions: BTreeMap::new(),
            super_index: None,
            cache_super_bindings: false,
            super_bindings: Mutex::new(BTreeMap::new()),
            receiver_methods: Mutex::new(BTreeMap::new()),
            publications: None,
            field_catalog: None,
            field_index: None,
            field_uses: None,
            published_owners: None,
            original_names: BTreeMap::new(),
            class_catalog: BTreeMap::new(),
            constructor_sets: BTreeMap::new(),
            inputs: std::slice::from_ref(dex),
            dex_index: 0,
            writes: None,
            hierarchy: crate::type_compatibility::Hierarchy::default(),
            original_hierarchy: None,
            enum_fields: None,
            annotation_defaults: None,
            annotation_guards: None,
            defer_annotation_defaults: false,
            dex,
            metadata,
            aliases,
            methods: BTreeMap::new(),
            fields: BTreeMap::new(),
            static_positions: BTreeMap::new(),
            catalog: BTreeMap::new(),
            global_catalog,
            field_names: BTreeMap::new(),
            external: None,
            input_owners: BTreeSet::new(),
        };
        let raw = MemberResolver { dex };
        for ty in &dex.type_ids {
            if let Ok(d) = dex.string(ty.descriptor_idx)
                && let Some(original) = d.strip_prefix('L').and_then(|d| d.strip_suffix(';'))
                && let Some(emitted) = aliases.class_name(d)
            {
                r.original_names
                    .insert(emitted.into(), original.replace('/', "."));
            }
        }
        for c in &dex.classes {
            if let Some(owner) = descriptor(dex, c.class_idx).and_then(|d| aliases.class_name(d)) {
                let constructors = c
                    .class_data
                    .as_ref()
                    .into_iter()
                    .flat_map(|d| &d.direct_methods)
                    .filter(|m| method_key(dex, m.method_idx).is_some_and(|k| k.name == "<init>"))
                    .map(|m| MethodRef::new(m.method_idx))
                    .collect();
                r.constructor_sets.insert(owner.into(), constructors);
            }
            if let Some(data) = &c.class_data {
                for method in &data.direct_methods {
                    if method_key(dex, method.method_idx).is_some_and(|k| k.name == "<init>") {
                        let valid = dex
                            .method_id(method.method_idx)
                            .is_ok_and(|m| u32::from(m.class_idx) == c.class_idx);
                        r.constructor_definitions
                            .entry(method.method_idx)
                            .and_modify(|entry| *entry = None)
                            .or_insert(valid.then_some(method));
                    }
                }
                // Preserve duplicate annotation entries and their original order.
                let mut annotations = BTreeMap::<_, Vec<_>>::new();
                if let Some(directory) = &c.annotations {
                    for member in &directory.method_annotations {
                        annotations
                            .entry(member.member_idx)
                            .or_default()
                            .push(member);
                    }
                }
                for m in data.direct_methods.iter().chain(&data.virtual_methods) {
                    // Production declarations/throws were scanned once in the
                    // defining pool and cached globally before worker scheduling.
                    if r.global_catalog.is_some() {
                        continue;
                    }
                    let throws = (|| {
                        let mut result = None;
                        for member in annotations.get(&m.method_idx).into_iter().flatten() {
                            for item in &member.annotations.entries {
                                let annotation = &item.annotation;
                                if descriptor(dex, annotation.type_idx)?
                                    != "Ldalvik/annotation/Throws;"
                                {
                                    continue;
                                }
                                if result.is_some()
                                    || annotation.elements.len() != 1
                                    || dex.string(annotation.elements[0].name_idx).ok()? != "value"
                                    || item.visibility
                                        != dex_format::annotations::Visibility::System
                                {
                                    return None;
                                }
                                let dex_format::encoded_value::EncodedValueKind::Array(values) =
                                    &annotation.elements[0].value.value
                                else {
                                    return None;
                                };
                                let mut names = Vec::new();
                                for value in values {
                                    let dex_format::encoded_value::EncodedValueKind::Type(index) =
                                        value.value
                                    else {
                                        return None;
                                    };
                                    names.push(raw.class(ClassRef::new(index))?);
                                }
                                result = Some(names);
                            }
                        }
                        Some(result.unwrap_or_default())
                    })()
                    .and_then(|names| {
                        names
                            .into_iter()
                            .map(|n| {
                                aliases
                                    .class_name(&format!("L{};", n.replace('.', "/")))
                                    .map(str::to_owned)
                            })
                            .collect()
                    });
                    r.methods.insert(m.method_idx, (m.access_flags, throws));
                }
                for (is_static, fields) in
                    [(true, &data.static_fields), (false, &data.instance_fields)]
                {
                    for (position, f) in fields.iter().enumerate() {
                        r.fields.insert(f.field_idx, (f.access_flags, is_static));
                        if is_static {
                            r.static_positions
                                .entry((c.class_idx, f.field_idx))
                                .or_insert(position);
                        }
                    }
                }
            }
        }
        r
    }
    fn declaration(&self, index: u32) -> Result<MethodMetadata, MetadataRequirement> {
        let key =
            method_key(self.dex, index).ok_or(MetadataRequirement::InvalidReference(index))?;
        let metadata =
            if let Some(metadata) = self.global_catalog.unwrap_or(&self.catalog).get(&key) {
                metadata.clone()
            } else if let Some((flags, throws)) = self.methods.get(&index) {
                Some(MethodMetadata {
                    access_flags: *flags,
                    exceptions: throws.as_ref().map(|names| {
                        names
                            .iter()
                            .map(|n| format!("L{};", n.replace('.', "/")))
                            .collect()
                    }),
                })
            } else if self.input_owners.contains(&key.owner) {
                None
            } else {
                self.external.and_then(|provider| {
                    provider.method(&key).or_else(|| {
                        let proof = provider.inherited_public_instance(&key)?;
                        (!proof
                            .owners()
                            .iter()
                            .any(|owner| self.input_owners.contains(owner)))
                        .then(|| proof.declaration().metadata.clone())
                    })
                })
            }
            .ok_or_else(|| MetadataRequirement::Declaration(key.clone()))?;
        if metadata.exceptions.is_none() {
            return Err(MetadataRequirement::Exceptions(key));
        }
        Ok(metadata)
    }
    fn ty(&self, d: &str) -> Option<JavaType> {
        if let Some(component) = d.strip_prefix('[') {
            return Some(JavaType::Array(Box::new(self.ty(component)?)));
        }
        if d.starts_with('L') {
            return Some(JavaType::Class(self.aliases.class_name(d)?.into()));
        }
        crate::java_descriptors::descriptor_type(d)
    }
    fn signature(&self, index: u32, is_static: bool, throws: Vec<String>) -> Option<JavaMethod> {
        let key = method_key(self.dex, index)?;
        self.signature_key(&key, is_static, throws)
    }
    fn signature_key(
        &self,
        key: &MethodKey,
        is_static: bool,
        throws: Vec<String>,
    ) -> Option<JavaMethod> {
        let name = match self.aliases.method_name(key)? {
            MethodAlias::Ordinary(n) => n.clone(),
            MethodAlias::Constructor => "<init>".into(),
            MethodAlias::ClassInitializer => "<clinit>".into(),
        };
        Some(JavaMethod {
            owner: self.aliases.class_name(&key.owner)?.into(),
            name,
            is_static,
            parameters: key
                .parameters
                .iter()
                .map(|d| self.ty(d))
                .collect::<Option<_>>()?,
            return_type: self.ty(&key.return_type)?,
            throws,
        })
    }
    fn field_declaration(&self, class: &ClassDef, index: u32) -> Result<String, String> {
        if class.access_flags & 0x200 != 0 {
            // Interface fields are implicitly public static final. A blank
            // declaration is illegal even when a DEX clinit exists: Java has
            // no interface static blocks. Until an initializer expression is
            // proven, retain neither the field nor bodies depending on it.
            return Err("interface field initializer unavailable".into());
        }
        let (flags, is_static) = *self
            .fields
            .get(&index)
            .ok_or("unresolved field definition")?;
        if flags & !(1 | 2 | 4 | 8 | 0x10 | 0x40 | 0x80 | 0x1000) != 0 || flags & 0x50 == 0x50 {
            return Err("unsupported field flags".into());
        }
        if is_static {
            class.class_data.as_ref().ok_or("field owner data")?;
            let position = *self
                .static_positions
                .get(&(class.class_idx, index))
                .ok_or("field owner mismatch")?;
            let _ = position;
        }
        let f = self
            .field(FieldRef::new(index))
            .ok_or("invalid field metadata")?;
        Ok(format!(
            "{}{}{}{}{}{} {};",
            visibility(flags)?,
            if is_static { "static " } else { "" },
            if flags & 0x10 != 0 { "final " } else { "" },
            if flags & 0x40 != 0 { "volatile " } else { "" },
            if flags & 0x80 != 0 { "transient " } else { "" },
            type_source(&f.ty),
            f.name
        ))
    }
}
impl Resolver<'_> {
    fn bound_super(&self, caller: &str, reference: MethodRef) -> Option<BoundSuper> {
        let cache_key = (caller.to_owned(), reference.index());
        if self.cache_super_bindings
            && let Some(cached) = self.super_bindings.lock().ok()?.get(&cache_key)
        {
            return cached.clone();
        }
        let compute = || {
            let original = self.original_names.get(caller)?;
            let key = method_key(self.dex, reference.index())?;
            let selected = crate::java_super_resolution::select_super(
                &format!("L{};", original.replace('.', "/")),
                &key,
                self.super_index?,
                self.aliases,
                self.original_hierarchy
                    .as_deref()
                    .unwrap_or(&self.hierarchy),
            )
            .or_else(|_| {
                crate::java_super_resolution::select_interface_super(
                    &format!("L{};", original.replace('.', "/")),
                    &key,
                    self.super_index
                        .ok_or(crate::java_super_resolution::SuperError::Metadata)?,
                    self.aliases,
                )
            })
            .ok()?;
            let throws = selected
                .metadata
                .exceptions
                .as_ref()?
                .iter()
                .map(|descriptor| {
                    self.aliases
                        .class_name(descriptor)
                        .map(str::to_owned)
                        .or_else(
                            || match crate::java_descriptors::descriptor_type(descriptor)? {
                                JavaType::Class(name) => Some(name),
                                _ => None,
                            },
                        )
                })
                .collect::<Option<Vec<_>>>()?;
            let throws = self
                .source_throws
                .and_then(|plan| source_throws::source(plan, &selected.target))
                .unwrap_or(throws);
            let method = self.signature_key(&selected.target, false, throws)?;
            Some(BoundSuper {
                interface_qualifier: selected.interface_qualifier,
                key: selected.target,
                metadata: selected.metadata,
                method,
            })
        };
        let binding = compute();
        if self.cache_super_bindings {
            self.super_bindings
                .lock()
                .ok()?
                .insert(cache_key, binding.clone());
        }
        binding
    }
    fn original_method(&self, reference: MethodRef) -> Option<JavaMethod> {
        method_key(self.dex, reference.index())?;
        let metadata = self.declaration(reference.index()).ok()?;
        let throws = metadata
            .exceptions?
            .iter()
            .map(|d| {
                self.aliases.class_name(d).map(str::to_owned).or_else(|| {
                    match crate::java_descriptors::descriptor_type(d)? {
                        JavaType::Class(n) => Some(n),
                        _ => None,
                    }
                })
            })
            .collect::<Option<Vec<_>>>()?;
        self.signature(reference.index(), metadata.access_flags & 8 != 0, throws)
    }
    fn resolve_method(&self, reference: MethodRef) -> Option<JavaMethod> {
        let mut method = self.original_method(reference)?;
        let key = method_key(self.dex, reference.index())?;
        if let Some(throws) = self
            .source_throws
            .and_then(|plan| source_throws::source(plan, &key))
        {
            method.throws = throws;
        }
        Some(method)
    }
    fn resolve_field(&self, reference: FieldRef) -> Option<JavaField> {
        let mut key = field_key(self.dex, reference.index())?;
        let (flags, is_static) = match self.fields.get(&reference.index()) {
            Some(flags) => *flags,
            None if self
                .field_catalog
                .and_then(|p| p.declarations.get(&key))
                .is_some() =>
            {
                let definition = self.field_catalog?.declarations.get(&key)?;
                (definition.access_flags, definition.access_flags & 8 != 0)
            }
            None if !self.input_owners.contains(&key.owner) => {
                let provider = self.external?;
                let resolved = [false, true].into_iter().find_map(|is_static| {
                    crate::java_members::external_metadata::fields::resolve_field_declaration_indexed(
                        self.inputs, &key, provider, is_static, self.field_index,
                    ).ok()
                })?;
                // Metadata is not a permission token. The requirements pass
                // checks the original caller and actual SSA receiver, while
                // receiver retention proves source-level member selection.
                if resolved.origin != crate::java_members::external_metadata::FieldOrigin::Classpath
                    || provider.class_access(&resolved.key.owner)? & 1 == 0
                    || self
                        .aliases
                        .field_name(&resolved.key)
                        .is_some_and(|name| name != resolved.key.name)
                {
                    return None;
                }
                key = resolved.key;
                let metadata = resolved.metadata;
                (metadata.access_flags, metadata.access_flags & 8 != 0)
            }
            None => {
                self.field_index?;
                let provider = self.external.unwrap_or(&MissingMetadata);
                let resolved = [false, true].into_iter().find_map(|is_static| {
                    crate::java_members::external_metadata::fields::resolve_field_indexed(
                        self.inputs,
                        &key,
                        provider,
                        &crate::java_members::external_metadata::FieldAccessContext {
                            caller: &key.owner,
                            receiver: Some(&key.owner),
                            is_static,
                        },
                        self.field_index,
                    )
                    .ok()
                })?;
                key = resolved.key;
                (
                    resolved.metadata.access_flags,
                    resolved.metadata.access_flags & 8 != 0,
                )
            }
        };
        if (flags & 8 != 0) != is_static {
            return None;
        }
        Some(JavaField {
            owner: self
                .aliases
                .class_name(&key.owner)
                .map(str::to_owned)
                .or_else(|| {
                    if self.input_owners.contains(&key.owner) {
                        return None;
                    }
                    match crate::java_descriptors::descriptor_type(&key.owner)? {
                        JavaType::Class(name) => Some(name),
                        _ => None,
                    }
                })?,
            name: self.aliases.field_name(&key).unwrap_or(&key.name).into(),
            ty: self.ty(&key.descriptor)?,
            is_static,
        })
    }
}
fn discarded_external_source_target(
    provider: &dyn ExternalMetadata,
    key: &MethodKey,
    source_name: &str,
) -> bool {
    let Some(metadata) = provider.method(key) else {
        return false;
    };
    if metadata.access_flags & (0x40 | 0x80 | 0x1000) != 0 || source_name != key.name {
        return false;
    }
    let Some(declarations) = provider.declared_methods(&key.owner) else {
        return false;
    };
    declarations.iter().any(|method| method.key == *key)
        && declarations.iter().all(|method| {
            method.key.name != key.name
                || method.key.parameters != key.parameters
                || method.key.return_type == key.return_type
                || provider.attests_covariant_bridge(&method.key, key)
        })
}

#[cfg(test)]
#[path = "java_discarded_target_tests.rs"]
mod discarded_target_tests;

impl JavaResolver for Resolver<'_> {
    fn super_interface_qualifier(&self, caller: &str, reference: MethodRef) -> Option<String> {
        self.bound_super(caller, reference)?.interface_qualifier
    }
    fn super_invocation(
        &self,
        caller: &str,
        reference: MethodRef,
    ) -> Option<backend_java::SuperInvocation> {
        if let Some(binding) = self.bound_super(caller, reference) {
            return Some(backend_java::SuperInvocation {
                method: binding.method,
                access_flags: binding.metadata.access_flags,
            });
        }
        if self.super_index.is_some() || !self.java_super_selects_dex_target(caller, reference) {
            return None;
        }
        Some(backend_java::SuperInvocation {
            method: self.method(reference)?,
            access_flags: self.method_access(reference)?,
        })
    }
    fn trusted_string_builder_class(&self, reference: ClassRef) -> bool {
        let Some(owner) = descriptor(self.dex, reference.index()) else {
            return false;
        };
        owner == "Ljava/lang/StringBuilder;"
            && !self.input_owners.contains(owner)
            && self
                .external
                .and_then(|provider| provider.class_access(owner))
                .is_some_and(|flags| flags & 0x11 == 0x11 && flags & 0x200 == 0)
    }
    fn java_super_selects_dex_target(&self, caller: &str, reference: MethodRef) -> bool {
        use backend_java::constructors::TrustedConstructorResolver;
        let Some(key) = method_key(self.dex, reference.index()) else {
            return false;
        };
        if key.name.starts_with('<') {
            return false;
        }
        if self.super_index.is_some() {
            return self.bound_super(caller, reference).is_some();
        }
        let Some(parent) = self.constructor_class(caller).and_then(|c| c.superclass) else {
            return false;
        };
        if self
            .class_catalog
            .get(&parent)
            .and_then(Option::as_ref)
            .is_none_or(|class| class.access_flags & 0x200 != 0)
            || self.aliases.class_name(&key.owner) != Some(parent.as_str())
            || !matches!(self.aliases.method_name(&key), Some(MethodAlias::Ordinary(name)) if name == &key.name)
        {
            return false;
        }
        let Ok(metadata) = self.declaration(reference.index()) else {
            return false;
        };
        // Bridge, varargs, abstract, private, and static declarations cannot
        // establish this bounded direct-parent source selection.
        if metadata.access_flags & (2 | 8 | 0x40 | 0x80 | 0x400) != 0 {
            return false;
        }
        if self.input_owners.contains(&key.owner) {
            let mut found = false;
            for dex in self.inputs {
                for class in &dex.classes {
                    if descriptor(dex, class.class_idx) != Some(key.owner.as_str()) {
                        continue;
                    }
                    if found || class.access_flags & 0x200 != 0 {
                        return false;
                    }
                    found = true;
                    let Some(data) = &class.class_data else {
                        return false;
                    };
                    let mut exact = false;
                    for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                        let Some(candidate) = method_key(dex, method.method_idx) else {
                            return false;
                        };
                        if candidate.name == key.name {
                            if method.access_flags & (0x40 | 0x80) != 0 {
                                return false;
                            }
                            if candidate.parameters == key.parameters {
                                if candidate != key {
                                    return false;
                                }
                                exact = true;
                            }
                        }
                    }
                    if !exact {
                        return false;
                    }
                }
            }
            found
        } else {
            let Some(methods) = self.external.and_then(|e| e.declared_methods(&key.owner)) else {
                return false;
            };
            methods
                .iter()
                .any(|m| m.key == key && m.signature.is_none())
                && !methods.iter().filter(|m| m.key.name == key.name).any(|m| {
                    m.signature.is_some()
                        || m.metadata.access_flags & (0x40 | 0x80) != 0
                        || (m.key.parameters == key.parameters && m.key != key)
                })
        }
    }

    fn virtual_receiver_method(
        &self,
        reference: MethodRef,
        caller: &str,
        receiver: &JavaType,
    ) -> Option<JavaMethod> {
        receiver_methods::resolve(self, reference, caller, receiver)
    }
    fn retain_method_receiver(
        &self,
        reference: MethodRef,
        caller: &str,
        receiver: &JavaType,
    ) -> bool {
        let JavaType::Class(receiver) = receiver else {
            return false;
        };
        let Some(target) = self.method(reference) else {
            return false;
        };
        let Some(key) = method_key(self.dex, reference.index()) else {
            return false;
        };
        let Some(flags) = self.method_access(reference) else {
            return false;
        };
        if flags & 4 == 0 || target.is_static {
            return false;
        }
        let Some(original_caller) = self.original_names.get(caller) else {
            return false;
        };
        let Some(original_receiver) = self.original_names.get(receiver) else {
            return false;
        };
        let original_owner = key
            .owner
            .trim_start_matches('L')
            .trim_end_matches(';')
            .replace('/', ".");
        let access = crate::java_access::AccessContext {
            caller: original_caller,
            owner: &original_owner,
            receiver: Some(original_receiver),
            hierarchy: self
                .original_hierarchy
                .as_deref()
                .unwrap_or(&self.hierarchy),
            nests: None,
            emitted_caller: caller,
            emitted_owner: &target.owner,
        };
        if access
            .method_access_with_source_hierarchy(flags, &self.hierarchy, Some(receiver))
            .is_err()
        {
            return false;
        }
        // Preserve the direct-parent proof, or require a complete unshadowed
        // ancestor path to a genuine external protected declaration.
        let Some(class) = self.class_catalog.get(receiver).and_then(Option::as_ref) else {
            return false;
        };
        if !self
            .input_owners
            .contains(&format!("L{};", original_receiver.replace('.', "/")))
        {
            return false;
        }
        if class.superclass.as_deref() != Some(&target.owner) {
            return receiver_methods::protected_ancestor(self, reference, receiver);
        }
        let descriptor = format!("L{};", original_receiver.replace('.', "/"));
        // Descriptor arguments are pinned by the emitter. Reject other same-name
        // declarations conservatively, including aliased overloads and bridges.
        !self
            .global_catalog
            .unwrap_or(&self.catalog)
            .iter()
            .any(|(candidate, metadata)| {
                candidate.owner == descriptor
                    && self.aliases.method_name(candidate).is_some_and(
                        |name| matches!(name, MethodAlias::Ordinary(n) if n == &target.name),
                    )
                    && (candidate.parameters != key.parameters
                        || candidate.return_type != key.return_type
                        || metadata
                            .as_ref()
                            .is_none_or(|m| m.access_flags & (2 | 8) != 0))
            })
    }
    fn field_access(&self, reference: FieldRef) -> Option<u32> {
        self.fields
            .get(&reference.index())
            .map(|(flags, _)| *flags)
            .or_else(|| {
                let key = field_key(self.dex, reference.index())?;
                if let Some(field) = self.field_catalog?.declarations.get(&key) {
                    return Some(field.access_flags);
                }
                let field = self.field(reference)?;
                crate::java_members::external_metadata::fields::resolve_field_declaration_indexed(
                    self.inputs,
                    &key,
                    self.external.unwrap_or(&MissingMetadata),
                    field.is_static,
                    self.field_index,
                )
                .ok()
                .map(|f| f.metadata.access_flags)
            })
    }
    fn field_source_nonconstant(&self, reference: FieldRef) -> bool {
        let Some(key) = field_key(self.dex, reference.index()) else {
            return false;
        };
        let Some(field) = self.field(reference) else {
            return false;
        };
        let Ok(resolved) =
            crate::java_members::external_metadata::fields::resolve_field_declaration_indexed(
                self.inputs,
                &key,
                self.external.unwrap_or(&MissingMetadata),
                field.is_static,
                self.field_index,
            )
        else {
            return false;
        };
        if resolved.origin == crate::java_members::external_metadata::fields::FieldOrigin::Classpath
        {
            // Genuine class-file absence of ConstantValue makes a source read
            // retain GETSTATIC and its owner-initialization effect. Input fields
            // still require their separately published nonconstant declaration.
            return resolved.metadata.constant_value.is_none();
        }
        let Some(declaration) = self
            .field_catalog
            .and_then(|catalog| catalog.declarations.get(&resolved.key))
        else {
            return false;
        };
        // Ordinary input owner_declarations emits blank declarations, including
        // encoded finals. Initialization is emitted in separate source blocks;
        // no literal initializer can make this exact field a constant variable.
        // This does not attest that the owner initialization/body is complete.
        declaration.kind == ClassKind::Class
            && declaration.access_flags == resolved.metadata.access_flags
            && self.aliases.field_name(&resolved.key).is_some()
            && self.aliases.type_name(&resolved.key.descriptor).is_some()
    }
    fn static_method_owner_access(&self, reference: MethodRef) -> Option<u32> {
        use backend_java::constructors::TrustedConstructorResolver;
        let method = self.method(reference)?;
        if !method.is_static {
            return None;
        }
        self.constructor_class(&method.owner)
            .map(|class| class.access_flags)
    }
    fn static_method_type_context(&self, reference: MethodRef, caller: &str) -> bool {
        let Some(method) = self.method(reference) else {
            return false;
        };
        let Some(root) = method.owner.split('.').next() else {
            return false;
        };
        let mut current = Some(caller);
        let mut seen = BTreeSet::new();
        while let Some(owner) = current {
            if !seen.insert(owner) {
                break;
            }
            if self
                .field_names
                .get(owner)
                .is_some_and(|names| names.contains(root))
            {
                return true;
            }
            current = self
                .class_catalog
                .get(owner)
                .and_then(Option::as_ref)
                .and_then(|class| class.superclass.as_deref());
        }
        false
    }
    fn static_field_type_context(&self, reference: FieldRef, caller: &str) -> bool {
        let Some(field) = self.field(reference) else {
            return false;
        };
        let Some(root) = field.owner.split('.').next() else {
            return false;
        };
        let mut current = Some(caller);
        let mut seen = BTreeSet::new();
        while let Some(owner) = current {
            if !seen.insert(owner) {
                break;
            }
            if self
                .field_names
                .get(owner)
                .is_some_and(|names| names.contains(root))
            {
                return true;
            }
            current = self
                .class_catalog
                .get(owner)
                .and_then(Option::as_ref)
                .and_then(|class| class.superclass.as_deref());
        }
        false
    }
    fn retain_field_receiver(
        &self,
        reference: FieldRef,
        caller: &str,
        receiver: &JavaType,
    ) -> bool {
        let JavaType::Class(receiver) = receiver else {
            return false;
        };
        if let Some(index) = self.field_index {
            let proof = (|| {
                let key = field_key(self.dex, reference.index())?;
                let caller = format!("L{};", self.original_names.get(caller)?.replace('.', "/"));
                let receiver =
                    format!("L{};", self.original_names.get(receiver)?.replace('.', "/"));
                let provider = self.external.unwrap_or(&MissingMetadata);
                let context = crate::java_members::external_metadata::FieldAccessContext {
                    caller: &caller,
                    receiver: Some(&receiver),
                    is_static: false,
                };
                if let Some(cache) = &self.field_use_cache {
                    cache.resolve(&key, &context).ok()?;
                    return Some(cache.retain_receiver(&key, &context).unwrap_or(false));
                }
                let resolved =
                    crate::java_members::external_metadata::fields::resolve_field_indexed(
                        self.inputs,
                        &key,
                        provider,
                        &context,
                        Some(index),
                    )
                    .ok()?;
                if resolved.origin == crate::java_members::external_metadata::FieldOrigin::Classpath
                {
                    return Some(crate::java_members::external_metadata::fields::retain_field_receiver_with_aliases_indexed(
                        self.inputs, &resolved, provider, &context, self.aliases, Some(index),
                    ).unwrap_or(false));
                }
                Some(crate::java_members::external_metadata::fields::publication::source_use_indexed(
                    self.inputs, &resolved, provider, &context, self.aliases, Some(index),
                ).is_ok_and(|source| source.receiver == crate::java_members::external_metadata::fields::publication::FieldReceiver::Retain))
            })();
            if let Some(retain) = proof {
                return retain;
            }
        }
        let Some(target) = self.field(reference) else {
            return false;
        };
        let Some(flags) = self.field_access(reference) else {
            return false;
        };
        if flags & 4 == 0 || target.is_static {
            return false;
        }
        let Some(original_caller) = self.original_names.get(caller) else {
            return false;
        };
        let Some(original_receiver) = self.original_names.get(receiver) else {
            return false;
        };
        let Some(key) = field_key(self.dex, reference.index()) else {
            return false;
        };
        let original_owner = key
            .owner
            .trim_start_matches('L')
            .trim_end_matches(';')
            .replace('/', ".");
        let access = crate::java_access::AccessContext {
            caller: original_caller,
            owner: &original_owner,
            receiver: Some(original_receiver),
            hierarchy: self
                .original_hierarchy
                .as_deref()
                .unwrap_or(&self.hierarchy),
            nests: None,
            emitted_caller: caller,
            emitted_owner: &target.owner,
        };
        if access
            .method_access_with_source_hierarchy(flags, &self.hierarchy, Some(receiver))
            .is_err()
        {
            return false;
        }
        self.class_catalog
            .get(receiver)
            .and_then(Option::as_ref)
            .is_some_and(|c| c.superclass.as_deref() == Some(&target.owner))
            && self
                .field_names
                .get(receiver)
                .is_some_and(|names| !names.contains(&target.name))
    }
    fn array_clone_method(&self, reference: MethodRef, receiver: &JavaType) -> Option<JavaMethod> {
        let key = method_key(self.dex, reference.index())?;
        let expected = self.ty(&key.owner)?;
        if &expected != receiver {
            return None;
        }
        crate::java_array_clone::method_for_receiver(&key, receiver)
    }

    fn discarded_invoke_source_target(&self, reference: MethodRef) -> bool {
        let Some(key) = method_key(self.dex, reference.index()) else {
            return false;
        };
        let Ok(metadata) = self.declaration(reference.index()) else {
            return false;
        };
        if metadata.access_flags & (0x40 | 0x80 | 0x1000) != 0 {
            return false;
        }
        let Some(target) = self.method(reference) else {
            return false;
        };
        if self.input_owners.contains(&key.owner) {
            let lower = MethodKey {
                owner: key.owner.clone(),
                name: String::new(),
                parameters: vec![],
                return_type: String::new(),
            };
            return !self
                .global_catalog
                .unwrap_or(&self.catalog)
                .range(lower..)
                .take_while(|(candidate, _)| candidate.owner == key.owner)
                .any(|(candidate, _)| {
                    candidate.parameters == key.parameters
                        && candidate.return_type != key.return_type
                        && self.aliases.method_name(candidate) == self.aliases.method_name(&key)
                });
        }
        self.external
            .is_some_and(|provider| discarded_external_source_target(provider, &key, &target.name))
    }

    fn method_access(&self, reference: MethodRef) -> Option<u32> {
        self.declaration(reference.index())
            .ok()
            .map(|m| m.access_flags)
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        self.hierarchy.assignable(actual, expected)
            == crate::type_compatibility::Compatibility::Assignable
    }
    fn instruction_type(&self, reference: TypeRef) -> Option<JavaType> {
        match reference.index() {
            0x8000_0081 | 0x8000_0088 | 0x8000_008b => Some(JavaType::Long),
            0x8000_0082 | 0x8000_0085 | 0x8000_008c => Some(JavaType::Float),
            0x8000_0083 | 0x8000_0086 | 0x8000_0089 => Some(JavaType::Double),
            0x8000_0080 | 0x8000_0084 | 0x8000_0087 | 0x8000_008a => Some(JavaType::Int),
            0x8000_008d => Some(JavaType::Byte),
            0x8000_008e => Some(JavaType::Char),
            0x8000_008f => Some(JavaType::Short),
            index if index < 0x8000_0000 => self.ty(descriptor(self.dex, index)?),
            _ => None,
        }
    }
    fn class(&self, reference: ClassRef) -> Option<String> {
        self.aliases
            .class_name(descriptor(self.dex, reference.index())?)
            .map(str::to_owned)
    }
    fn string(&self, reference: StringRef) -> Option<Vec<u16>> {
        DexJavaResolver { dex: self.dex }.string(reference)
    }
    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        match &self.method_cache {
            Some(cache) => {
                cache.get_or_compute(reference.index(), || self.resolve_method(reference))
            }
            None => self.resolve_method(reference),
        }
    }
    fn field(&self, reference: FieldRef) -> Option<JavaField> {
        match &self.field_cache {
            Some(cache) => {
                cache.get_or_compute(reference.index(), || self.resolve_field(reference))
            }
            None => self.resolve_field(reference),
        }
    }
}
impl backend_java::constructors::ConstructorResolver for Resolver<'_> {
    fn constructor_exception_metadata(
        &self,
        reference: MethodRef,
        ir: &MethodIr<SsaForm>,
    ) -> Option<Box<dyn backend_java::constructors::ConstructorExceptionMetadata + '_>> {
        let scopes = self.constructor_exception_scopes.lock().ok()?;
        // Arena IDs can coincide between distinct methods or frontend runs.
        // Only the identical, still-owned original SSA instance authorizes
        // these source offsets and handler/edge identities. Retaining several
        // instances also keeps cache-disabled exports independent of cache hits.
        let exact = scopes
            .get(&reference.index())?
            .iter()
            .find(|stored| std::ptr::eq(stored.ssa.ir(), ir))?
            .clone();
        Some(Box::new(ConstructorExceptionScopes(exact)))
    }
    fn constructor_exception_field_store(&self, reference: FieldRef) -> bool {
        self.field_access(reference)
            .is_some_and(|flags| flags & 0x10 == 0)
            && self.constructor_early_field_store(reference)
    }
    fn constructor_source_level(&self) -> u16 {
        self.source_release
    }
    fn constructor_early_field_store(&self, reference: FieldRef) -> bool {
        let Some(key) = field_key(self.dex, reference.index()) else {
            return false;
        };
        let Some(declaration) = self.field_catalog.and_then(|c| c.declarations.get(&key)) else {
            return false;
        };
        // The ordinary-owner publisher emits these exact instance fields as
        // blank declarations. DEX encoded initializers apply only to statics.
        // The constructor backend separately proves this is its own receiver,
        // not an inherited field, read, escape or instance invocation.
        declaration.kind == ClassKind::Class
            && declaration.access_flags & 8 == 0
            && self.aliases.field_name(&key).is_some()
            && self.aliases.type_name(&key.descriptor).is_some()
    }
    fn constructor_super_target(&self, caller: &str, reference: MethodRef) -> bool {
        use backend_java::constructors::TrustedConstructorResolver;
        let Some(key) = method_key(self.dex, reference.index()) else {
            return false;
        };
        if key.name.starts_with('<') || self.input_owners.contains(&key.owner) {
            return false;
        }
        if !matches!(self.aliases.method_name(&key), Some(MethodAlias::Ordinary(name)) if name == &key.name)
        {
            return false;
        }
        let Some(parent) = self.constructor_class(caller).and_then(|c| c.superclass) else {
            return false;
        };
        if self.aliases.class_name(&key.owner) != Some(parent.as_str()) {
            return false;
        }
        let Some(methods) = self.external.and_then(|e| e.declared_methods(&key.owner)) else {
            return false;
        };
        if !methods.iter().any(|m| {
            m.key == key
                && m.signature.is_none()
                && m.metadata.access_flags & (2 | 8 | 0x80 | 0x400) == 0
        }) || methods
            .iter()
            .filter(|m| m.key.name == key.name)
            .any(|m| m.signature.is_some() || m.metadata.access_flags & 0x80 != 0)
        {
            return false;
        }
        // Only original invoke-super opcodes witness this dispatch. A pool entry
        // or invoke-direct on a similarly named method is not a super proof.
        let mut witnessed = false;
        for class in &self.dex.classes {
            if descriptor(self.dex, class.class_idx).and_then(|d| self.aliases.class_name(d))
                != Some(caller)
            {
                continue;
            }
            let Some(data) = &class.class_data else {
                return false;
            };
            for method in &data.direct_methods {
                let Some(code) = &method.code else {
                    continue;
                };
                let Ok(instructions) = dex_decompiler::decode_method(code) else {
                    return false;
                };
                for insn in instructions {
                    let dex_decompiler::Opcode::Supported(info) = insn.opcode else {
                        continue;
                    };
                    if insn.pool_index == Some(reference.index())
                        && [0x6f, 0x70, 0x75, 0x76].contains(&info.opcode)
                    {
                        if ![0x6f, 0x75].contains(&info.opcode) {
                            return false;
                        }
                        witnessed = true;
                    }
                }
            }
        }
        witnessed
    }
    fn initializer(&self, reference: MethodRef) -> Option<backend_java::constructors::Initializer> {
        let key = method_key(self.dex, reference.index())?;
        if key.name != "<init>" {
            return None;
        }
        // External constructors require exception/hierarchy metadata. Object's
        // no-argument constructor is the one explicitly known safe root.
        let method = if key.owner == "Ljava/lang/Object;"
            && key.parameters.is_empty()
            && key.return_type == "V"
        {
            self.signature(reference.index(), false, vec![])?
        } else {
            self.method(reference)?
        };
        if method.is_static || method.return_type != JavaType::Void {
            return None;
        }
        let cached = {
            self.constructor_delegations
                .lock()
                .ok()?
                .get(&reference.index())
                .copied()
        };
        let delegates_to = if let Some(cached) = cached {
            cached
        } else {
            // Missing attestation is not a proved super root: the constructor
            // planner must still check the actual this/super source call.
            let target = attested_this_delegation(self, reference);
            self.constructor_delegations
                .lock()
                .ok()?
                .insert(reference.index(), target);
            target
        };
        Some(backend_java::constructors::Initializer {
            method,
            delegates_to,
        })
    }
}
impl backend_java::constructors::TrustedConstructorResolver for Resolver<'_> {
    fn constructor_class(
        &self,
        name: &str,
    ) -> Option<backend_java::constructors::ConstructorClass> {
        if !self.class_catalog.is_empty() {
            return self.class_catalog.get(name)?.clone();
        }
        let mut found = None;
        for dex in self.inputs {
            for class in &dex.classes {
                let original = descriptor(dex, class.class_idx)?;
                if self.aliases.class_name(original) == Some(name) {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(backend_java::constructors::ConstructorClass {
                        name: name.into(),
                        superclass: if class.superclass_idx == u32::MAX {
                            None
                        } else {
                            Some(
                                self.aliases
                                    .class_name(descriptor(dex, class.superclass_idx)?)?
                                    .into(),
                            )
                        },
                        access_flags: class.access_flags,
                    });
                }
            }
        }
        if found.is_some() {
            return found;
        }
        let external = self.external?;
        let original = external.hierarchy().into_iter().find(|d| {
            self.aliases
                .class_name(&format!("L{};", d.name.replace('.', "/")))
                .unwrap_or(&d.name)
                == name
        })?;
        let flags = external.class_access(&format!("L{};", original.name.replace('.', "/")))?;
        Some(backend_java::constructors::ConstructorClass {
            name: name.into(),
            superclass: original.superclass.map(|n| {
                self.aliases
                    .class_name(&format!("L{};", n.replace('.', "/")))
                    .unwrap_or(&n)
                    .to_owned()
            }),
            access_flags: flags,
        })
    }
    fn constructor_access(&self, reference: MethodRef) -> Option<u32> {
        let key = method_key(self.dex, reference.index())?;
        if key.name != "<init>" {
            return None;
        }
        self.declaration(reference.index())
            .ok()
            .map(|m| m.access_flags)
    }
    fn declared_constructors(&self, owner: &str) -> Option<Vec<MethodRef>> {
        if !self.class_catalog.is_empty() {
            return self.constructor_sets.get(owner).cloned();
        }
        let mut found = None;
        for dex in self.inputs {
            for class in &dex.classes {
                if self.aliases.class_name(descriptor(dex, class.class_idx)?) != Some(owner) {
                    continue;
                }
                if found.is_some() {
                    return None;
                }
                let mut references = Vec::new();
                for method in &class.class_data.as_ref()?.direct_methods {
                    let key = method_key(dex, method.method_idx)?;
                    if key.name != "<init>" {
                        continue;
                    }
                    let index = (0..self.dex.method_ids.len())
                        .find(|i| method_key(self.dex, *i as u32).as_ref() == Some(&key))?;
                    references.push(MethodRef::new(index as u32));
                }
                found = Some(references);
            }
        }
        found
    }
}

fn visibility(flags: u32) -> Result<&'static str, String> {
    Ok(match flags & 7 {
        0 => "",
        1 => "public ",
        2 => "private ",
        4 => "protected ",
        _ => return Err("conflicting visibility".into()),
    })
}
fn declaration_bindings(
    ssa: &dex_decompiler::ssa::SsaMethod,
    m: &JavaMethod,
    code: &dex_format::code::CodeItem,
) -> Result<DeclarationBindings, String> {
    let provenance: Vec<_> = ssa
        .initial_registers()
        .iter()
        .map(
            |(value, registers)| java_initial_parameters::InitialProvenance {
                value: *value,
                registers: registers.clone(),
            },
        )
        .collect();
    let plan = java_initial_parameters::parameter_bindings_with_provenance(
        ssa.ir(),
        m,
        code.registers_size,
        code.ins_size,
        &provenance,
    )
    .map_err(|e| e.to_string())?;
    let value = |p| match p {
        InitialParameter::Bound(v) => Some(v),
        InitialParameter::Unused { .. } => None,
    };
    Ok(DeclarationBindings {
        receiver: plan.receiver.and_then(value),
        parameters: plan.parameters.into_iter().map(value).collect(),
        names: vec![],
    })
}
#[cfg(test)]
fn initial_context<'a>(
    r: &Resolver<'a>,
    method: &EncodedMethod,
) -> Result<dex_decompiler::DexTypes<'a>, String> {
    initial_context_with_seeds(r, method).map(|(context, _)| context)
}
type RegisterSeeds = Vec<((u16, bool), managed_ir::types::ManagedType)>;
fn initial_context_with_seeds<'a>(
    r: &Resolver<'a>,
    method: &EncodedMethod,
) -> Result<(dex_decompiler::DexTypes<'a>, RegisterSeeds), String> {
    use dex_decompiler::infer::TypeContext;
    let code = method.code.as_ref().ok_or("incoming signature: no body")?;
    let raw = r
        .dex
        .method_id(method.method_idx)
        .map_err(|e| e.to_string())?;
    let layout = crate::java_incoming_registers::IncomingLayout::from_proto(
        r.dex,
        r.dex
            .proto_id(u32::from(raw.proto_idx))
            .map_err(|e| e.to_string())?,
        method.access_flags & 8 != 0,
        code.registers_size,
        code.ins_size,
    )
    .map_err(|e| e.to_string())?;
    let ctx = dex_decompiler::DexTypes::with_metadata(r.metadata, Some(method.method_idx));
    let mut seeds = std::collections::HashMap::new();
    if let Some(slot) = layout.receiver {
        seeds.insert(
            (slot.register, false),
            ctx.type_at(u32::from(raw.class_idx))
                .ok_or("incoming receiver type unavailable")?,
        );
    }
    let mut types = Vec::new();
    if !ctx.method_params(method.method_idx, &mut types) || types.len() != layout.parameters.len() {
        return Err("incoming parameter type unavailable".into());
    }
    for (slot, ty) in layout.parameters.iter().zip(types) {
        seeds.insert((slot.register, slot.high_register.is_some()), ty);
    }
    let mut key_seeds: RegisterSeeds = seeds.iter().map(|(k, v)| (*k, v.clone())).collect();
    key_seeds.sort_by_key(|(register, _)| *register);
    Ok((ctx.with_initial_registers(seeds), key_seeds))
}
fn cached_frontend(
    r: &Resolver<'_>,
    method: &EncodedMethod,
    code: &dex_format::code::CodeItem,
    context: &dex_decompiler::DexTypes<'_>,
    seeds: RegisterSeeds,
    timings: Option<&mut dex_decompiler::passes::FrontendTimings>,
) -> Result<Arc<dex_decompiler::ssa::SsaMethod>, dex_decompiler::passes::StageError> {
    let compute = || match timings {
        Some(timings) => dex_decompiler::DexPasses::run_ssa_with_timings(code, context, timings),
        None => dex_decompiler::DexPasses::run_ssa_with(code, context),
    };
    match r.ssa_cache {
        Some(cache) => cache.get_or_compute(
            ssa_cache::CacheKey::new(r.dex_index, method.method_idx, code, seeds),
            compute,
        ),
        None => compute().map(Arc::new),
    }
}
struct Candidate {
    dependencies: BTreeSet<MethodKey>,
    field_uses: FieldUses,
    source: String,
    requirements: MemberRequirements,
    body: bool,
    constructor: bool,
}
fn body_dependencies(
    ir: &MethodIr<SsaForm>,
    r: &Resolver<'_>,
    caller: &str,
) -> Result<BTreeSet<MethodKey>, String> {
    ir.instructions()
        .values()
        .filter_map(|insn| match insn {
            ManagedInsn::Invoke {
                kind: InvokeKind::Super,
                method,
                ..
            } if r.super_index.is_some() => Some(
                r.bound_super(caller, *method)
                    .map(|binding| binding.key)
                    .ok_or_else(|| "super dependency binding unavailable".to_owned()),
            ),
            ManagedInsn::Invoke { method, .. } => Some(
                method_key(r.dex, method.index())
                    .ok_or_else(|| "invocation dependency identity unavailable".to_owned()),
            ),
            _ => None,
        })
        .collect()
}
fn body_field_uses(ir: &MethodIr<SsaForm>, dex: &DexFile) -> FieldUses {
    ir.instructions()
        .values()
        .filter_map(|insn| match insn {
            ManagedInsn::FieldGet { field, object, .. }
            | ManagedInsn::FieldPut { field, object, .. } => {
                let receiver = object
                    .and_then(|v| ir.value(v).ok())
                    .and_then(|v| match v.ty() {
                        managed_ir::types::ManagedType::Reference(c) => {
                            descriptor(dex, c.index()).map(str::to_owned)
                        }
                        _ => None,
                    });
                Some((field_key(dex, field.index())?, receiver, object.is_none()))
            }
            _ => None,
        })
        .collect()
}
fn dependency_signature_with_catches(
    ir: &MethodIr<SsaForm>,
    caller: &JavaMethod,
    r: &Resolver<'_>,
    caught_types: &[&str],
) -> JavaMethod {
    let mut signature = caller.clone();
    if !caught_types.is_empty() {
        for instruction in ir.instructions().values() {
            if let ManagedInsn::Invoke { kind, method, .. } = instruction
                && let Some(target) = if *kind == InvokeKind::Super {
                    r.super_invocation(&caller.owner, *method)
                        .map(|binding| binding.method)
                } else {
                    r.method(*method)
                }
            {
                for thrown in target.throws {
                    if caught_types.iter().any(|caught| {
                        r.assignable(
                            &JavaType::Class(thrown.clone()),
                            &JavaType::Class((*caught).into()),
                        )
                    }) {
                        signature.throws.push(thrown);
                    }
                }
            }
        }
    }
    signature
}

fn requirements(
    ir: &MethodIr<SsaForm>,
    caller: &JavaMethod,
    r: &Resolver<'_>,
    constructor: bool,
) -> Result<MemberRequirements, String> {
    requirements_with_bridge(ir, caller, r, constructor, None)
}
fn requirements_with_bridge(
    ir: &MethodIr<SsaForm>,
    caller: &JavaMethod,
    r: &Resolver<'_>,
    constructor: bool,
    bridge: Option<&backend_java::checked_source_bridge::Plan>,
) -> Result<MemberRequirements, String> {
    let mut result = MemberRequirements::default();
    let mut field_receiver_projection = field_receiver_projection::Projection::default();
    for (id, instruction) in ir.instructions() {
        match instruction {
            ManagedInsn::Invoke {
                kind, method, args, ..
            } => {
                if constructor && *kind == InvokeKind::Special {
                    let k = method_key(r.dex, method.index()).ok_or("initializer metadata")?;
                    if k.name == "<init>"
                        && k.owner == "Ljava/lang/Object;"
                        && k.parameters.is_empty()
                    {
                        continue;
                    }
                }
                let symbolic_key =
                    method_key(r.dex, method.index()).ok_or("invalid method pool reference")?;
                if *kind == InvokeKind::Virtual
                    && args.len() == 1
                    && crate::java_array_clone::is_array_clone_key(&symbolic_key)
                {
                    let receiver = ir
                        .value(args[0])
                        .ok()
                        .and_then(|value| managed_type_to_java(value.ty(), r))
                        .ok_or("array clone receiver type unavailable")?;
                    if r.array_clone_method(*method, &receiver).is_some() {
                        continue;
                    }
                    return Err("array clone receiver proof unavailable".into());
                }
                let (key, metadata, target) =
                    if *kind == InvokeKind::Super && r.super_index.is_some() {
                        let binding = r
                            .bound_super(&caller.owner, *method)
                            .ok_or("super invocation: exact selected-target binding unavailable")?;
                        (binding.key, binding.metadata, binding.method)
                    } else {
                        let metadata = r.declaration(method.index()).map_err(|requirement| {
                            format!("input metadata requirement: {requirement:?}")
                        })?;
                        (
                            symbolic_key,
                            metadata,
                            r.method(*method).ok_or("invalid invocation metadata")?,
                        )
                    };
                let external = target.owner != caller.owner;
                if external {
                    let original_caller = r
                        .original_names
                        .get(&caller.owner)
                        .ok_or("caller access identity unavailable")?;
                    let original_owner = key
                        .owner
                        .trim_start_matches('L')
                        .trim_end_matches(';')
                        .replace('/', ".");
                    let receiver =
                        args.first()
                            .and_then(|v| ir.value(*v).ok())
                            .and_then(|v| match v.ty() {
                                managed_ir::types::ManagedType::Reference(c) => r.class(*c),
                                _ => None,
                            });
                    let original_receiver = receiver
                        .as_ref()
                        .and_then(|name| r.original_names.get(name))
                        .map(String::as_str);
                    let access = crate::java_access::AccessContext {
                        caller: original_caller,
                        owner: &original_owner,
                        receiver: original_receiver,
                        hierarchy: r.original_hierarchy.as_deref().unwrap_or(&r.hierarchy),
                        nests: None,
                        emitted_caller: &caller.owner,
                        emitted_owner: &target.owner,
                    };
                    let class_flags = r
                        .class_catalog
                        .get(&target.owner)
                        .and_then(Option::as_ref)
                        .map(|c| c.access_flags)
                        .ok_or("invocation declaring class accessibility unavailable")?;
                    access
                        .class_access(class_flags)
                        .map_err(|e| format!("invocation class access: {e:?}"))?;
                    access
                        .method_access(metadata.access_flags)
                        .map_err(|e| format!("invocation member access: {e:?}"))?;
                    if metadata.access_flags & 4 != 0 {
                        access
                            .method_access_with_source_hierarchy(
                                metadata.access_flags,
                                &r.hierarchy,
                                receiver.as_deref(),
                            )
                            .map_err(|e| format!("invocation emitted protected access: {e:?}"))?;
                    }
                }
                if *kind == InvokeKind::Special && target.name == "<init>" {
                    if target.is_static || target.return_type != JavaType::Void {
                        return Err("invalid allocation initializer".into());
                    }
                    if backend_java::checked_exceptions::check(r, &target.throws, &caller.throws)
                        .is_err()
                        && !bridge.is_some_and(|plan| {
                            plan.approves_invoke(id, instruction, &target, caller)
                        })
                    {
                        return Err("initializer exceptions not declared by caller".into());
                    }
                    if !external {
                        result.methods.insert(method.index());
                    }
                    continue;
                }
                let direct_private =
                    *kind == InvokeKind::Special && metadata.access_flags & 2 != 0 && !external;
                if *kind == InvokeKind::Super
                    && !r.java_super_selects_dex_target(&caller.owner, *method)
                {
                    return Err(
                        "super invocation: exact direct-parent target proof unavailable".into(),
                    );
                }
                if *kind == InvokeKind::Special && !direct_private
                    && !(constructor && backend_java::constructors::ConstructorResolver::constructor_super_target(r, &caller.owner, *method)) {
                    return Err(
                        "special invocation: exact Java super target proof unavailable".into(),
                    );
                }
                if target.name.starts_with('<')
                    || !(direct_private
                        || *kind == InvokeKind::Super
                        || (constructor && *kind == InvokeKind::Special)
                        || matches!(
                            kind,
                            InvokeKind::Static | InvokeKind::Virtual | InvokeKind::Interface
                        ))
                    || (*kind == InvokeKind::Static) != target.is_static
                {
                    return Err("unsupported invocation dispatch".into());
                }
                if metadata.access_flags & 2 != 0
                    && matches!(kind, InvokeKind::Virtual | InvokeKind::Interface)
                {
                    return Err("private virtual dispatch is not exact direct invocation".into());
                }
                // Match the exact receiver-static source declaration already
                // selected by exception inference and backend invocation
                // emission. Symbolic access/dispatch/dependency checks above
                // remain bound to the original DEX pool reference.
                let checked_target = if *kind == InvokeKind::Virtual {
                    args.first()
                        .and_then(|value| ir.value(*value).ok())
                        .and_then(|value| managed_type_to_java(value.ty(), r))
                        .and_then(|receiver| {
                            r.virtual_receiver_method(*method, &caller.owner, &receiver)
                        })
                } else {
                    None
                };
                let checked_target = checked_target.as_ref().unwrap_or(&target);
                if backend_java::checked_exceptions::check(
                    r,
                    &checked_target.throws,
                    &caller.throws,
                )
                .is_err()
                    && !bridge.is_some_and(|plan| {
                        plan.approves_invoke(id, instruction, checked_target, caller)
                    })
                {
                    return Err("callee exceptions not declared by caller".into());
                }
                if !external {
                    result.methods.insert(method.index());
                }
            }
            ManagedInsn::FieldGet { field, object, .. }
            | ManagedInsn::FieldPut { field, object, .. } => {
                let target = r.field(*field).ok_or("unresolved field metadata")?;
                if target.owner != caller.owner {
                    let key =
                        field_key(r.dex, field.index()).ok_or("invalid field pool reference")?;
                    let provider = r.external.unwrap_or(&MissingMetadata);
                    let original_caller = r
                        .original_names
                        .get(&caller.owner)
                        .ok_or("field caller access identity unavailable")?;
                    let original_caller = format!("L{};", original_caller.replace('.', "/"));
                    let receiver =
                        object
                            .and_then(|v| ir.value(v).ok())
                            .and_then(|v| match v.ty() {
                                managed_ir::types::ManagedType::Reference(class) => {
                                    descriptor(r.dex, class.index())
                                }
                                _ => None,
                            });
                    let projected_receiver = if constructor && receiver.is_none() {
                        object.and_then(|value| {
                            let selected = field_receiver_projection.selected_class(
                                ir,
                                value,
                                &caller.return_type,
                                r,
                            )?;
                            let original = r.original_names.get(selected)?;
                            Some(format!("L{};", original.replace('.', "/")))
                        })
                    } else {
                        None
                    };
                    let context = crate::java_members::external_metadata::FieldAccessContext {
                        caller: &original_caller,
                        receiver: receiver.or(projected_receiver.as_deref()),
                        is_static: object.is_none(),
                    };
                    let resolved = if let Some(cache) = &r.field_use_cache {
                        cache.resolve(&key, &context)
                    } else {
                        crate::java_members::external_metadata::fields::resolve_field_indexed(
                            r.inputs,
                            &key,
                            provider,
                            &context,
                            r.field_index,
                        )
                    }
                    .map_err(|e| format!("external field requirement: {e:?}"))?;
                    if resolved.origin == crate::java_members::external_metadata::FieldOrigin::Input
                    {
                        if let Some(cache) = &r.field_use_cache {
                            cache.source_use(&key, &context)
                        } else {
                            crate::java_members::external_metadata::fields::publication::source_use_indexed(
                            r.inputs,
                            &resolved,
                            provider,
                            &context,
                            r.aliases,
                            r.field_index,
                        )
                        }.map_err(|e| format!("source field requirement: {e:?}"))?;
                    }
                    let actual_owner = r
                        .aliases
                        .class_name(&resolved.key.owner)
                        .map(str::to_owned)
                        .or_else(|| {
                            match crate::java_descriptors::descriptor_type(&resolved.key.owner)? {
                                JavaType::Class(name) => Some(name),
                                _ => None,
                            }
                        });
                    if actual_owner.as_deref() != Some(target.owner.as_str())
                        || r.aliases
                            .field_name(&resolved.key)
                            .unwrap_or(&resolved.key.name)
                            != target.name
                    {
                        return Err("external field actual owner alias mismatch".into());
                    }
                    continue;
                }
                if target.is_static != object.is_none() {
                    return Err("field staticness mismatch".into());
                }
                result.fields.insert(field.index());
            }
            _ => {}
        }
    }
    Ok(result)
}
fn checked_bridge_helper(r: &Resolver<'_>, key: &MethodKey, index: u32) -> Option<String> {
    let mut cache = r.checked_bridge_names.lock().ok()?;
    let names = cache
        .entry(key.owner.clone())
        .or_insert_with(|| checked_bridge_name_inventory(r, &key.owner))
        .as_ref()?;
    for suffix in 0..=names.len() {
        let helper = format!("decomp$checked${index}${suffix}");
        if !names.contains(&helper) {
            return Some(helper);
        }
    }
    None
}
fn checked_bridge_name_inventory(r: &Resolver<'_>, root: &str) -> Option<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    let mut pending = vec![root.to_owned()];
    let mut seen = BTreeSet::new();
    while let Some(owner) = pending.pop() {
        if !seen.insert(owner.clone()) {
            continue;
        }
        if r.input_owners.contains(&owner) {
            let (dex, class) = r.field_index?.declared_class(&owner)?;
            if let Some(data) = &class.class_data {
                for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                    let identity = method_key(dex, method.method_idx)?;
                    if let Some(MethodAlias::Ordinary(name)) = r.aliases.method_name(&identity) {
                        names.insert(name.clone());
                    }
                    names.insert(identity.name);
                }
            }
            if class.superclass_idx != u32::MAX {
                pending.push(descriptor(dex, class.superclass_idx)?.into());
            }
            if let Some(interfaces) = &class.interfaces {
                for interface in &interfaces.types {
                    pending.push(descriptor(dex, u32::from(*interface))?.into());
                }
            }
        } else {
            let metadata = r.external?;
            for method in metadata.declared_methods(&owner)? {
                names.insert(method.key.name);
            }
            let declarations = metadata.class_hierarchy(&owner);
            if declarations.len() != 1 {
                return None;
            }
            let declaration = &declarations[0];
            pending.extend(
                declaration
                    .superclass
                    .iter()
                    .chain(&declaration.interfaces)
                    .map(|name| format!("L{};", name.replace('.', "/"))),
            );
        }
    }
    Some(names)
}
fn method_source(
    class: &ClassDef,
    method: &EncodedMethod,
    r: &Resolver<'_>,
    kind: ClassKind,
    profile: &mut Option<ClassProfile>,
) -> Result<Candidate, String> {
    if let Some(p) = profile.as_mut() {
        p.methods += 1;
    }
    let mut m = r
        .method(MethodRef::new(method.method_idx))
        .ok_or("unresolved signature/exception metadata")?;
    let constructor = m.name == "<init>";
    let clinit = m.name == "<clinit>";
    if kind == ClassKind::Annotation {
        // Validate source element legality against real metadata, then render
        // signature types and member names from the frozen alias registry.
        let mut declared = class.clone();
        if r.annotation_defaults.is_some() {
            declared.annotations = None;
        }
        abstract_method_declaration(r.dex, &declared, method)
            .map_err(|e| format!("annotation declaration: {e:?}"))?;
        let defaults =
            initializers::annotation_publication::declared_defaults_with(r.dex, class, &|index| {
                r.class(ClassRef::new(index))
                    .ok_or(DeclarationUnsupported::InvalidMetadata)
            })
            .map_err(|e| format!("annotation defaults: {e:?}"))?;
        // The complete wrapper above must remain valid, but only the emitted
        // element's default introduces dependencies into this declaration.
        use dex_format::encoded_value::EncodedValueKind as Value;
        fn references(value: &Value, types: &mut BTreeSet<u32>, uses_enum: &mut bool) {
            match value {
                Value::Type(index) => {
                    types.insert(*index);
                }
                Value::Enum(_) => *uses_enum = true,
                Value::Array(values) => {
                    for value in values {
                        references(&value.value, types, uses_enum);
                    }
                }
                _ => {}
            }
        }
        let raw_name = r
            .dex
            .method_id(method.method_idx)
            .ok()
            .and_then(|raw| r.dex.string(raw.name_idx).ok())
            .ok_or("annotation default dependency: invalid method name")?;
        let mut referenced_types = BTreeSet::new();
        let mut uses_enum = false;
        let mut found_default = false;
        if let Some(set) = class
            .annotations
            .as_ref()
            .and_then(|a| a.class_annotations.as_ref())
        {
            for item in &set.entries {
                if descriptor(r.dex, item.annotation.type_idx)
                    != Some("Ldalvik/annotation/AnnotationDefault;")
                {
                    continue;
                }
                for wrapper in &item.annotation.elements {
                    if let Value::Annotation(values) = &wrapper.value.value {
                        for element in &values.elements {
                            if r.dex.string(element.name_idx).ok() == Some(raw_name) {
                                found_default = true;
                                references(
                                    &element.value.value,
                                    &mut referenced_types,
                                    &mut uses_enum,
                                );
                            }
                        }
                    }
                }
            }
        }
        if found_default != defaults.by_method.contains_key(&method.method_idx) {
            return Err("annotation default dependency: element identity mismatch".into());
        }
        let deferred_default = if let Some(registry) = r.annotation_defaults {
            registry
                .lock()
                .map_err(|_| "annotation default publication lock")?
                .prepare_method_default(r.dex, class, method.method_idx, &|index| {
                    r.class(ClassRef::new(index))
                        .ok_or(DeclarationUnsupported::InvalidMetadata)
                })
                .map_err(|e| {
                    if uses_enum {
                        "annotation default dependency: enum constant source closure unproven"
                            .into()
                    } else {
                        format!("annotation defaults: {e:?}")
                    }
                })?
        } else {
            if uses_enum {
                return Err(
                    "annotation default dependency: enum constant source closure unproven".into(),
                );
            }
            None
        };
        for index in &referenced_types {
            let d =
                descriptor(r.dex, *index).ok_or("annotation default dependency: invalid type")?;
            if d.starts_with('L')
                && !r.input_owners.contains(d)
                && !r.external.is_some_and(|metadata| {
                    metadata
                        .hierarchy()
                        .iter()
                        .any(|decl| format!("L{};", decl.name.replace('.', "/")) == d)
                })
                && !matches!(
                    d,
                    "Ljava/lang/String;" | "Ljava/lang/Class;" | "Ljava/lang/Object;"
                )
            {
                return Err("annotation default dependency: class declaration unavailable".into());
            }
        }
        if let (Some(token), Some(guards)) = (deferred_default, r.annotation_guards) {
            guards
                .lock()
                .map_err(|_| "annotation default guard lock")?
                .entry(
                    descriptor(r.dex, class.class_idx)
                        .ok_or("annotation owner identity")?
                        .into(),
                )
                .or_default()
                .push(token);
        }
        let default = defaults
            .by_method
            .get(&method.method_idx)
            .map(|s| format!(" default {s}"))
            .unwrap_or_default();
        return Ok(Candidate {
            dependencies: BTreeSet::new(),
            field_uses: Vec::new(),
            source: format!("{} {}(){default};\n", type_source(&m.return_type), m.name),
            requirements: MemberRequirements::default(),
            body: false,
            constructor: false,
        });
    }
    let owner = match kind {
        ClassKind::Class => MethodOwner::Class {
            is_abstract: class.access_flags & 0x400 != 0,
        },
        ClassKind::Interface => MethodOwner::Interface,
        _ => return Err("annotation/enum body unavailable".into()),
    };
    let mut flags = if !constructor && !clinit {
        Some(
            plan_method_declaration(MethodFlagInput {
                access_flags: method.access_flags,
                owner,
                kind: MethodKind::Ordinary,
                has_code: method.code.is_some(),
                last_parameter_is_array: matches!(m.parameters.last(), Some(JavaType::Array(_))),
            })
            .map_err(|e| format!("method flags: {e}"))?,
        )
    } else {
        None
    };
    let parameters = |varargs: bool| {
        m.parameters
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let ty = if varargs && i + 1 == m.parameters.len() {
                    if let JavaType::Array(component) = t {
                        format!("{}...", type_source(component))
                    } else {
                        type_source(t)
                    }
                } else {
                    type_source(t)
                };
                format!("{ty} p{i}")
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    if let Some(plan) = &flags
        && plan.body == MethodBody::Forbidden
    {
        let throws = if m.throws.is_empty() {
            String::new()
        } else {
            format!(" throws {}", m.throws.join(", "))
        };
        return Ok(Candidate {
            dependencies: BTreeSet::new(),
            field_uses: Vec::new(),
            source: format!(
                "{} {} {}({}){throws};\n",
                plan.modifiers.join(" "),
                type_source(&m.return_type),
                m.name,
                parameters(plan.varargs)
            ),
            requirements: MemberRequirements::default(),
            body: false,
            constructor: false,
        });
    }
    if flags.as_ref().is_some_and(|plan| plan.synchronized) {
        let code = method
            .code
            .as_ref()
            .ok_or("synchronized method: no original code")?;
        let proven = crate::java_method_flags::empty_instance_monitor_scaffold(
            MethodFlagInput {
                access_flags: method.access_flags,
                owner,
                kind: MethodKind::Ordinary,
                has_code: true,
                last_parameter_is_array: matches!(m.parameters.last(), Some(JavaType::Array(_))),
            },
            code,
            &m.return_type,
            &JavaType::Class(m.owner.clone()),
            r,
        );
        if let Ok(proven) = proven {
            let plan = flags
                .as_ref()
                .ok_or("synchronized declaration unavailable")?;
            let result = match proven {
                crate::java_method_flags::EmptySynchronizedBody::Void => "return;",
                crate::java_method_flags::EmptySynchronizedBody::Receiver => "return this;",
            };
            let throws = if m.throws.is_empty() {
                String::new()
            } else {
                format!(" throws {}", m.throws.join(", "))
            };
            return Ok(Candidate {
                dependencies: BTreeSet::new(),
                field_uses: Vec::new(),
                source: format!(
                    "{} {} {}({}){throws} {{ {result} }}\n",
                    plan.modifiers.join(" "),
                    type_source(&m.return_type),
                    m.name,
                    parameters(plan.varargs)
                ),
                requirements: MemberRequirements::default(),
                body: true,
                constructor: false,
            });
        } else {
            // DECLARED_SYNCHRONIZED is reflective metadata, not an implicit
            // executable monitor. Normal exception lowering must prove and
            // preserve every explicit monitor; no monitor is removed here.
            let body = crate::java_method_flags::plan_body_modifiers(MethodFlagInput {
                access_flags: method.access_flags,
                owner,
                kind: MethodKind::Ordinary,
                has_code: true,
                last_parameter_is_array: matches!(m.parameters.last(), Some(JavaType::Array(_))),
            })
            .map_err(|e| format!("synchronized body flags: {e}"))?;
            if body.execution
                != crate::java_method_flags::SynchronizationExecution::ExplicitBodyMonitors
            {
                return Err("synchronized execution contract unavailable".into());
            }
            let plan = flags
                .as_mut()
                .ok_or("synchronized declaration unavailable")?;
            plan.modifiers = body.modifiers;
            plan.synchronized = false;
        }
    }
    let code = method.code.as_ref().ok_or("no code")?;
    let raw = r
        .dex
        .method_id(method.method_idx)
        .map_err(|e| e.to_string())?;
    let incoming_layout = crate::java_incoming_registers::IncomingLayout::from_proto(
        r.dex,
        r.dex
            .proto_id(u32::from(raw.proto_idx))
            .map_err(|e| e.to_string())?,
        m.is_static,
        code.registers_size,
        code.ins_size,
    )
    .map_err(|e| e.to_string())?;
    let ssa = {
        let enabled = profile.is_some();
        let timer = stage(profile, "frontend");
        let context_start = enabled.then(std::time::Instant::now);
        let mut initial_registers = std::collections::HashMap::new();
        let base_ctx = dex_decompiler::DexTypes::with_metadata(r.metadata, Some(method.method_idx));
        use dex_decompiler::infer::TypeContext;
        if let Some(slot) = incoming_layout.receiver
            && let Some(ty) = base_ctx.type_at(u32::from(raw.class_idx))
        {
            initial_registers.insert((slot.register, false), ty);
        }
        let mut parameter_types = Vec::new();
        if !base_ctx.method_params(method.method_idx, &mut parameter_types)
            || parameter_types.len() != incoming_layout.parameters.len()
        {
            return Err("incoming signature type resolution unavailable".into());
        }
        for (slot, ty) in incoming_layout.parameters.iter().zip(parameter_types) {
            initial_registers.insert((slot.register, slot.high_register.is_some()), ty);
        }
        let mut key_seeds: RegisterSeeds = initial_registers
            .iter()
            .map(|(register, ty)| (*register, ty.clone()))
            .collect();
        key_seeds.sort_by_key(|(register, _)| *register);
        let ctx = base_ctx.with_initial_registers(initial_registers);
        let context_elapsed = context_start.map(|start| start.elapsed());
        let mut timings = dex_decompiler::passes::FrontendTimings::default();
        let result = if enabled {
            cached_frontend(r, method, code, &ctx, key_seeds, Some(&mut timings))
        } else {
            cached_frontend(r, method, code, &ctx, key_seeds, None)
        };
        drop(timer);
        if let Some(p) = profile.as_mut() {
            if let Some(elapsed) = context_elapsed {
                *p.ssa.entry("type_context_prepare").or_default() += elapsed;
            }
            for (name, duration) in [
                ("decode", timings.decode),
                ("lift", timings.lift),
                ("merge", timings.merge),
            ] {
                *p.ssa.entry(name).or_default() += duration;
            }
            let timings = timings.ssa;
            for (name, duration) in [
                ("analysis", timings.analysis),
                ("placement", timings.placement),
                ("liveness", timings.liveness),
                ("phi_placement", timings.phi_placement),
                ("rename", timings.rename),
                ("phi_cleanup", timings.phi_cleanup),
                ("materialize", timings.materialize),
                ("inference", timings.inference),
                ("verification", timings.verification),
            ] {
                *p.ssa.entry(name).or_default() += duration;
            }
        }
        result.map_err(|e| format!("frontend: {e}"))?
    };
    let _timer = stage(profile, "structure_backend");
    let ir = ssa.ir();
    let prior_signature = m.clone();
    let mut checked_bridge = None;
    if kind == ClassKind::Class
        && !constructor
        && !clinit
        && code.tries.is_empty()
        && method.access_flags & (0x20 | 0x20000 | 0x100 | 0x400) == 0
        && let Some(key) = method_key(r.dex, method.method_idx)
        // Keep every currently valid callee source projection unchanged: an
        // encoded checked catch may depend on that exact declared contract.
        && r.source_throws.is_some_and(|plan| plan.unavailable.contains_key(&key))
        && let Some(original) = r.original_method(MethodRef::new(method.method_idx))
        && let Some(helper) = checked_bridge_helper(r, &key, method.method_idx)
    {
        let facts = r
            .source_throws
            .and_then(|plan| source_throws::runtime(plan, &key));
        // Absence is an unknown runtime domain, never an empty escaping set.
        // The recipe uses genuine original metadata and known current source
        // clauses without changing the separately retained runtime analysis.
        let identity_matches = facts.is_none_or(|facts| {
            facts.identity.dex == r.dex_index
                && facts.identity.index == method.method_idx
                && facts.identity.key == key
                && facts.original == original.throws
        });
        if identity_matches
            && let Ok(Some(plan)) = backend_java::checked_source_bridge::plan_source_contract(
                ir,
                &original,
                facts.map(|facts| facts.escaping.as_slice()),
                &helper,
                r,
            )
            .or_else(|_| {
                let targets: Vec<_> = r
                    .field_throw_catalog
                    .into_iter()
                    .flat_map(|catalog| {
                        ir.instructions().iter().filter_map(|(_, insn)| {
                            if let ManagedInsn::Invoke {
                                kind: managed_ir::ops::InvokeKind::Static,
                                method,
                                ..
                            } = insn
                            {
                                catalog.bind(r.dex, *method, r)
                            } else {
                                None
                            }
                        })
                    })
                    .collect();
                backend_java::checked_source_bridge::plan_whole_body_bound_targets(
                    ir,
                    &original,
                    facts.map(|facts| facts.escaping.as_slice()),
                    &helper,
                    r,
                    &targets,
                )
            })
        {
            m = original;
            checked_bridge = Some(plan);
        }
    }
    let mut disjoint_scopes = None;
    let mut exception_scopes = None;
    let mut dispatch_scopes = None;
    if !code.tries.is_empty() {
        if constructor || clinit {
            return Err("exception scope: initializer exception lowering deferred".into());
        }
        let legacy = if code.tries.len() > 1 {
            crate::java_exception_regions::from_dex_disjoint(&ssa, code, r, &r.hierarchy)
                .map(|scopes| disjoint_scopes = Some(scopes))
        } else {
            crate::java_exception_regions::from_dex(&ssa, code, r, &r.hierarchy)
                .map(|scopes| exception_scopes = Some(scopes))
        };
        if let Err(original) = legacy {
            dispatch_scopes = Some(
                crate::java_exception_regions::from_dex_dispatch(&ssa, code, r, &r.hierarchy)
                    .map_err(|fallback| {
                        format!("{original}; exception dispatch metadata: {fallback}")
                    })?,
            );
        }
    }
    drop(_timer);
    let precheck_timer = stage(profile, "structure_precheck");
    if exception_scopes.is_some() || disjoint_scopes.is_some() || dispatch_scopes.is_some() {
        // The exact scope grammar is validated by the metadata bridge and the
        // exception planner; the branch-only structurer rejects these edges.
    } else if ir.cfg().edge_ids().any(|id| {
        ir.cfg()
            .edge(id)
            .is_ok_and(|e| matches!(e.kind(), decomp_core::cfg::EdgeKind::Switch(_)))
    }) {
        decomp_structure::structure_extended(ir.cfg())
            .map(|_| ())
            .or_else(|original| {
                decomp_structure::extended::structure_flow(ir.cfg())
                    .map(|_| ())
                    .or_else(|_| decomp_structure::dispatch::plan(ir.cfg()).map(|_| ()))
                    .map_err(|_| original)
            })
            .map_err(|e| format!("structure: {e}"))?;
    } else {
        if decomp_structure::structure(ir.cfg()).is_err() {
            decomp_structure::loop_control::structure(ir.cfg())
                .map(|_| ())
                .or_else(|original| {
                    decomp_structure::extended::structure_flow(ir.cfg())
                        .map(|_| ())
                        .or_else(|_| decomp_structure::dispatch::plan(ir.cfg()).map(|_| ()))
                        .map_err(|_| original)
                })
                .map_err(|e| format!("structure: {e}"))?;
        }
    }
    drop(precheck_timer);
    let _timer = stage(profile, "structure_backend");
    let bindings = declaration_bindings(&ssa, &m, code)?;
    if checked_bridge.is_none()
        && kind == ClassKind::Class
        && !constructor
        && !clinit
        && !code.tries.is_empty()
        && method.access_flags & (0x20 | 0x20000 | 0x100 | 0x400) == 0
        && let Some(key) = method_key(r.dex, method.method_idx)
        && r.source_throws
            .is_some_and(|plan| plan.unavailable.contains_key(&key))
        && let Some(original) = r.original_method(MethodRef::new(method.method_idx))
        && let Some(helper) = checked_bridge_helper(r, &key, method.method_idx)
    {
        let facts = r
            .source_throws
            .and_then(|plan| source_throws::runtime(plan, &key));
        let identity_matches = facts.is_none_or(|facts| {
            facts.identity.dex == r.dex_index
                && facts.identity.index == method.method_idx
                && facts.identity.key == key
                && facts.original == original.throws
        });
        if identity_matches
            && let Ok(scopes) =
                crate::java_exception_regions::from_dex_dispatch(&ssa, code, r, &r.hierarchy)
        {
            let refs: Vec<&dyn backend_java::exceptions::RegionMetadata> = scopes
                .scopes
                .iter()
                .map(|scope| scope as &dyn backend_java::exceptions::RegionMetadata)
                .collect();
            if let Ok(Some(recipe)) = backend_java::checked_source_bridge::plan_dispatch_body(
                ir,
                &original,
                facts.map(|facts| facts.escaping.as_slice()),
                &helper,
                r,
                &bindings,
                &refs,
            ) {
                m = original;
                checked_bridge = Some(recipe);
                dispatch_scopes = Some(scopes);
            }
        }
    }
    let mut dependency_signature = m.clone();
    let caught_types: Vec<_> = exception_scopes
        .iter()
        .flat_map(|scopes| scopes.catch_names())
        .chain(
            disjoint_scopes
                .iter()
                .flat_map(|scopes| scopes.catch_names()),
        )
        .chain(
            dispatch_scopes
                .iter()
                .flat_map(|scopes| scopes.catch_names()),
        )
        .collect();
    if checked_bridge.is_none() {
        dependency_signature = dependency_signature_with_catches(ir, &m, r, &caught_types);
    }
    let mut requirements = requirements_with_bridge(
        ir,
        &dependency_signature,
        r,
        constructor,
        checked_bridge.as_ref(),
    )?;
    if ir.instructions().values().any(|i| {
        matches!(i, ManagedInsn::FieldPut { field, .. }
        if r.field_access(*field).is_some_and(|flags| flags & 0x10 != 0))
    }) {
        return Err("final field write requires atomic initialization proof".into());
    }
    let mut parameter_names = Vec::new();
    let mut source = if constructor {
        if kind != ClassKind::Class || method.access_flags & !(7 | 0x1000 | 0x10000) != 0 {
            return Err("unsupported constructor flags".into());
        }
        let super_name = r
            .class(ClassRef::new(class.superclass_idx))
            .ok_or("constructor superclass unavailable")?;
        if super_name != "java.lang.Object" {
            return Err("constructor hierarchy outside safe root subset".into());
        }
        backend_java::constructors::plan_constructor_declaration(
            ir,
            MethodRef::new(method.method_idx),
            &super_name,
            &bindings,
            r,
        )
        .map_err(|e| format!("constructor: {e:?}"))?
        .source
    } else if clinit {
        if kind != ClassKind::Class || method.access_flags & !(8 | 0x1000 | 0x10000) != 0 {
            return Err("unsupported class initializer flags/kind".into());
        }
        let writable = requirements
            .fields
            .iter()
            .filter_map(|index| r.field(FieldRef::new(*index)))
            .filter(|f| f.is_static)
            .collect();
        backend_java::static_initializer::emit_static_initializer(
            ir,
            &m,
            &ParameterBindings::default(),
            r,
            &backend_java::static_initializer::StaticInitializerContext {
                owner: m.owner.clone(),
                writable_static_fields: writable,
            },
        )
        .map_err(|e| format!("static initializer: {e:?}"))?
    } else {
        let emit_original = |caller: &JavaMethod| {
            if let Some(scopes) = &disjoint_scopes {
                let refs: Vec<&dyn backend_java::exceptions::RegionMetadata> = scopes
                    .scopes
                    .iter()
                    .map(|scope| scope as &dyn backend_java::exceptions::RegionMetadata)
                    .collect();
                backend_java::exceptions::emit_method_disjoint(ir, caller, &bindings, r, &refs)
            } else if let Some(scopes) = &exception_scopes {
                backend_java::exceptions::emit_method_region(ir, caller, &bindings, r, scopes)
            } else if let Some(scopes) = &dispatch_scopes {
                let refs: Vec<&dyn backend_java::exceptions::RegionMetadata> = scopes
                    .scopes
                    .iter()
                    .map(|scope| scope as &dyn backend_java::exceptions::RegionMetadata)
                    .collect();
                backend_java::exceptions::dispatch::emit_method(ir, caller, &bindings, r, &refs)
            } else {
                backend_java::emit_method_declaration_with_exceptions(
                    ir, caller, &bindings, r, None,
                )
            }
        };
        let result = if let Some(plan) = &checked_bridge {
            if let Some(scopes) = &dispatch_scopes {
                let refs: Vec<&dyn backend_java::exceptions::RegionMetadata> = scopes
                    .scopes
                    .iter()
                    .map(|scope| scope as &dyn backend_java::exceptions::RegionMetadata)
                    .collect();
                backend_java::checked_source_bridge::emit_dispatch_body(
                    ir, &bindings, r, plan, &refs,
                )
            } else {
                backend_java::checked_source_bridge::emit(ir, &bindings, r, plan)
            }
        } else {
            emit_original(&m)
        };
        // A source recipe is transactional: unsuccessful adoption leaves the
        // previously validated caller projection untouched.
        let result = if matches!(result, SourceResult::Unsupported(_)) && checked_bridge.is_some() {
            m = prior_signature.clone();
            checked_bridge = None;
            let rollback_signature = dependency_signature_with_catches(ir, &m, r, &caught_types);
            requirements = requirements_with_bridge(ir, &rollback_signature, r, constructor, None)?;
            emit_original(&m)
        } else {
            result
        };
        // Keep successful structured declarations verbatim. Only an ordinary
        // exception-bearing method rejected by that backend gets a second,
        // independently attested per-instruction schedule.
        let result = match result {
            SourceResult::Unsupported(original)
                if !code.tries.is_empty() && dispatch_scopes.is_none() =>
            {
                let scopes =
                    crate::java_exception_regions::from_dex_dispatch(&ssa, code, r, &r.hierarchy)
                        .map_err(|fallback| {
                        format!("backend: {original:?}; exception dispatch metadata: {fallback}")
                    })?;
                let refs: Vec<&dyn backend_java::exceptions::RegionMetadata> = scopes
                    .scopes
                    .iter()
                    .map(|scope| scope as &dyn backend_java::exceptions::RegionMetadata)
                    .collect();
                match backend_java::exceptions::dispatch::emit_method(ir, &m, &bindings, r, &refs) {
                    complete @ SourceResult::Complete { .. } => complete,
                    SourceResult::Unsupported(fallback) => {
                        return Err(format!(
                            "backend: {original:?}; exception dispatch: {fallback:?}"
                        ));
                    }
                }
            }
            result => result,
        };
        match result {
            SourceResult::Complete {
                source,
                parameter_names: names,
                ..
            } => {
                parameter_names = names;
                source
            }
            SourceResult::Unsupported(e) => return Err(format!("backend: {e:?}")),
        }
    };
    if constructor {
        source = source.replacen("public ", visibility(method.access_flags)?, 1);
    }
    if method.access_flags & 0x20000 != 0 && flags.as_ref().is_some_and(|p| !p.synchronized) {
        source.push_str("// Original DEX DECLARED_SYNCHRONIZED metadata retained; explicit body monitors preserved.\n");
    }
    if let Some(plan) = flags {
        let prefix = if m.is_static {
            "public static "
        } else {
            "public "
        };
        source = source
            .strip_prefix(prefix)
            .map(|tail| {
                format!(
                    "{}{}{tail}",
                    plan.modifiers.join(" "),
                    if plan.modifiers.is_empty() { "" } else { " " }
                )
            })
            .ok_or("backend declaration contract")?;
        if plan.varargs {
            // Only the declaration's last descriptor position is changed.
            let t = m.parameters.last().unwrap_or_else(|| {
                unreachable!("validated varargs declaration has a last parameter")
            });
            let position = m.parameters.len() - 1;
            let name = &parameter_names[position];
            if let JavaType::Array(component) = t {
                source = source.replacen(
                    &format!("{} {name}", type_source(t)),
                    &format!("{}... {name}", type_source(component)),
                    1,
                );
            }
        }
    }
    if let Some(plan) = checked_bridge {
        source.push_str(&format!("\n// Caller-local checked-source recipe: original declared {:?}; runtime escaping {:?}; {} exact sites.\n", plan.original().throws, plan.runtime_escaping(), plan.site_count()));
        source.push_str(&plan.helper_source());
    }
    Ok(Candidate {
        dependencies: body_dependencies(ir, r, &m.owner)?,
        field_uses: body_field_uses(ir, r.dex),
        source,
        requirements,
        body: true,
        constructor,
    })
}

pub(crate) fn emit(
    files: &[DexFile],
    options: &JavaOptions,
    sink: &mut impl SourceSink,
    mut profile: Option<&mut JavaProfile>,
) -> Result<JavaReport, OutputError> {
    let start = profile.as_ref().map(|_| std::time::Instant::now());
    let classpath = crate::java_external_metadata::Classpath::from_paths(&options.classpath)
        .map_err(|e| OutputError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
    // The singleton retains the same genuine JVM owner selection while sharing
    // inherited declaration proofs across all class workers in this export.
    let metadata = crate::java_external_metadata::MetadataChain::new(vec![
        crate::java_external_metadata::MetadataProvider::Jvm(&classpath),
    ]);
    let external = (!options.classpath.is_empty()).then_some(&metadata as &dyn ExternalMetadata);
    let cache = ssa_cache::SsaCache::new(options.cache_ssa);
    let result = emit_pipelined(files, options, sink, &mut profile, external, &cache);
    if let Some(p) = profile.as_deref_mut() {
        record_cache_profile(p, &cache, options.cache_ssa);
    }
    let timer = Timer::new(
        profile
            .as_deref_mut()
            .map(|p| p.serial.entry("cache_release").or_default()),
    );
    cache.release();
    drop(timer);
    if let (Some(p), Some(start)) = (profile, start) {
        p.wall = start.elapsed();
    }
    result
}

pub(crate) fn emit_with_metadata(
    files: &[DexFile],
    options: &JavaOptions,
    sink: &mut impl SourceSink,
    profile: Option<&mut JavaProfile>,
    external: &dyn ExternalMetadata,
) -> Result<JavaReport, OutputError> {
    emit_with_optional_metadata(files, options, sink, profile, Some(external))
}

pub(crate) fn emit_with_optional_metadata(
    files: &[DexFile],
    options: &JavaOptions,
    sink: &mut impl SourceSink,
    mut profile: Option<&mut JavaProfile>,
    external: Option<&dyn ExternalMetadata>,
) -> Result<JavaReport, OutputError> {
    let start = profile.as_ref().map(|_| std::time::Instant::now());
    let cache = ssa_cache::SsaCache::new(options.cache_ssa);
    let result = emit_pipelined(files, options, sink, &mut profile, external, &cache);
    if let Some(p) = profile.as_deref_mut() {
        record_cache_profile(p, &cache, options.cache_ssa);
    }
    let timer = Timer::new(
        profile
            .as_deref_mut()
            .map(|p| p.serial.entry("cache_release").or_default()),
    );
    cache.release();
    drop(timer);
    if let (Some(p), Some(start)) = (profile, start) {
        p.wall = start.elapsed();
    }
    result
}

fn record_cache_profile(profile: &mut JavaProfile, cache: &ssa_cache::SsaCache, enabled: bool) {
    let stats = cache.stats();
    profile.ssa_cache_enabled = enabled;
    profile.ssa_cache_hits = stats.hits;
    profile.ssa_cache_misses = stats.misses;
    profile.ssa_cache_entries = stats.entries;
    profile.ssa_cache_failures = stats.failures;
}

pub(crate) fn method_catalog(files: &[DexFile]) -> BTreeMap<MethodKey, Option<MethodMetadata>> {
    // Pool indices are local; only original descriptor keys cross DEX boundaries.
    // Duplicate class owners are ambiguous even when their methods differ.
    let mut owners = BTreeMap::<String, usize>::new();
    for dex in files {
        for class in &dex.classes {
            if let Some(owner) = descriptor(dex, class.class_idx) {
                *owners.entry(owner.into()).or_default() += 1;
            }
        }
    }
    let mut catalog = BTreeMap::new();
    for dex in files {
        let raw = MemberResolver { dex };
        for class in &dex.classes {
            if let Some(data) = &class.class_data {
                for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                    if let Some(key) = method_key(dex, method.method_idx) {
                        let declaration = (owners.get(&key.owner) == Some(&1)
                            && descriptor(dex, class.class_idx) == Some(key.owner.as_str()))
                        .then(|| MethodMetadata {
                            access_flags: method.access_flags,
                            exceptions: raw
                                .declared_throws_for_class(class, method.method_idx)
                                .map(|names| {
                                    names
                                        .into_iter()
                                        .map(|n| format!("L{};", n.replace('.', "/")))
                                        .collect()
                                }),
                        });
                        catalog
                            .entry(key)
                            .and_modify(|v| *v = None)
                            .or_insert(declaration);
                    }
                }
            }
        }
    }
    catalog
}

#[cfg(test)]
use index_tests::fixture as external_fixture;

#[cfg(test)]
#[allow(clippy::expect_used, clippy::items_after_test_module)]
mod external_metadata_tests {
    use super::external_fixture as fixture;
    use super::*;
    use backend_java::constructors::ConstructorResolver;

    #[test]
    fn crossdex_constructor_uses_declaration_flags_and_exceptions() {
        use dex_format::{
            RawSpan, annotations::*, class::ClassData, encoded_value::*, ids::*,
            strings::StringData,
        };
        let span = RawSpan { offset: 0, size: 0 };
        let mut declaration = DexFile::from_bytes(&fixture::minimal_dex()).expect("fixture");
        for text in [
            "V",
            "<init>",
            "Ldalvik/annotation/Throws;",
            "value",
            "Ljava/lang/Exception;",
        ] {
            declaration.strings.push(StringData {
                value: text.into(),
                utf16_size: text.len() as u32,
                span,
            });
        }
        for descriptor_idx in [1, 3, 5] {
            declaration.type_ids.push(TypeId {
                descriptor_idx,
                span,
            });
        }
        declaration.proto_ids.push(ProtoId {
            shorty_idx: 1,
            return_type_idx: 1,
            parameters_off: 0,
            parameters: None,
            span,
        });
        declaration.method_ids.push(MethodId {
            class_idx: 0,
            proto_idx: 0,
            name_idx: 2,
            span,
        });
        declaration.classes[0].class_data = Some(ClassData {
            static_fields: vec![],
            instance_fields: vec![],
            virtual_methods: vec![],
            direct_methods: vec![EncodedMethod {
                method_idx: 0,
                access_flags: 0x10001,
                code_off: 0,
                code: None,
            }],
            span,
        });
        declaration.classes[0].annotations = Some(AnnotationsDirectory {
            class_annotations: None,
            field_annotations: vec![],
            parameter_annotations: vec![],
            span,
            method_annotations: vec![MemberAnnotations {
                member_idx: 0,
                annotations: AnnotationSet {
                    span,
                    entries: vec![AnnotationItem {
                        visibility: Visibility::System,
                        span,
                        annotation: EncodedAnnotation {
                            type_idx: 2,
                            span,
                            elements: vec![AnnotationElement {
                                name_idx: 4,
                                value: EncodedValue {
                                    span,
                                    value: EncodedValueKind::Array(vec![EncodedValue {
                                        span,
                                        value: EncodedValueKind::Type(3),
                                    }]),
                                },
                            }],
                        },
                    }],
                },
            }],
        });
        let mut caller = declaration.clone();
        caller.classes.clear();
        let files = [caller, declaration];
        let aliases = inventory(&files, &mut None).expect("aliases");
        let metadata = dex_decompiler::infer::DexMetadata::new(&files[0]);
        let mut resolver = Resolver::new(&files[0], &aliases, &metadata);
        resolver.catalog = method_catalog(&files);
        assert_eq!(
            resolver.declaration(0).expect("crossdex").access_flags,
            0x10001
        );
        let initializer = resolver
            .initializer(MethodRef::new(0))
            .expect("constructor");
        assert!(!initializer.method.is_static);
        assert_eq!(initializer.method.throws, ["java.lang.Exception"]);
        let key = method_key(&files[0], 0).expect("key");
        let empty = BTreeMap::<MethodKey, MethodMetadata>::new();
        let resolved = crate::java_members::external_metadata::resolve_method(&files, &key, &empty)
            .expect("input declaration");
        assert_eq!(
            resolved.exceptions,
            Some(vec!["Ljava/lang/Exception;".into()])
        );
        let duplicate = [files[1].clone(), files[1].clone()];
        assert_eq!(
            crate::java_members::external_metadata::resolve_method(&duplicate, &key, &empty),
            Err(MetadataRequirement::Declaration(key.clone()))
        );
        resolver.catalog.clear();
        let key = method_key(&files[0], 0).expect("key");
        assert_eq!(
            resolver.declaration(0),
            Err(MetadataRequirement::Declaration(key.clone()))
        );
        let provider = BTreeMap::from([(
            key.clone(),
            MethodMetadata {
                access_flags: 1,
                exceptions: None,
            },
        )]);
        resolver.external = Some(&provider);
        assert_eq!(
            crate::java_members::external_metadata::resolve_method(&files[..1], &key, &provider),
            Err(MetadataRequirement::Exceptions(key.clone()))
        );
        assert_eq!(
            resolver.declaration(0),
            Err(MetadataRequirement::Exceptions(key.clone()))
        );
        assert!(resolver.initializer(MethodRef::new(0)).is_none());
        let provider = BTreeMap::from([(
            key,
            MethodMetadata {
                access_flags: 9,
                exceptions: Some(vec![]),
            },
        )]);
        resolver.external = Some(&provider);
        assert!(resolver.initializer(MethodRef::new(0)).is_none());
        let key = method_key(&files[0], 0).expect("key");
        let mut instructions = managed_ir::method::InsnArena::default();
        let invoke = instructions.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(0),
            args: vec![],
        });
        let ir = MethodIr::<SsaForm>::from_parts(
            decomp_core::cfg::Cfg::new(invoke),
            instructions,
            managed_ir::method::ValueArena::default(),
        );
        let caller = JavaMethod {
            owner: "Caller".into(),
            name: "call".into(),
            is_static: true,
            return_type: JavaType::Void,
            parameters: vec![],
            throws: vec![],
        };
        for access_flags in [1, 4, 2, 0] {
            let provider = BTreeMap::from([(
                key.clone(),
                MethodMetadata {
                    access_flags,
                    exceptions: Some(vec![]),
                },
            )]);
            let mut resolver = Resolver::new(&files[0], &aliases, &metadata);
            resolver.external = Some(&provider);
            resolver
                .original_names
                .insert("Caller".into(), "Caller".into());
            let owner = resolver
                .method(MethodRef::new(0))
                .expect("explicit target metadata")
                .owner;
            resolver.class_catalog.insert(
                owner.clone(),
                Some(backend_java::constructors::ConstructorClass {
                    name: owner,
                    superclass: Some("java.lang.Object".into()),
                    access_flags: 1,
                }),
            );
            assert_eq!(
                requirements(&ir, &caller, &resolver, false).is_ok(),
                access_flags != 2,
                "external accessibility flags {access_flags}"
            );
        }
    }
}

// Keep arbitrary (including non-Send) user sinks on their calling thread.
// Dependency receipts wait for real acknowledgement; only final ordinary units
// may run ahead. The channel bounds retained source units, not their byte size.
fn emit_pipelined(
    files: &[DexFile],
    options: &JavaOptions,
    sink: &mut impl SourceSink,
    profile: &mut Option<&mut JavaProfile>,
    external: Option<&dyn ExternalMetadata>,
    cache: &ssa_cache::SsaCache,
) -> Result<JavaReport, OutputError> {
    use std::sync::mpsc::{self, SyncSender};
    type Delivery = (GeneratedSource, Option<SyncSender<()>>);
    struct QueuedSink(SyncSender<Delivery>);
    fn disconnected() -> OutputError {
        OutputError::Io(std::io::Error::other("Java source delivery stopped"))
    }
    impl SourceSink for QueuedSink {
        fn emit(&mut self, source: &GeneratedSource) -> Result<(), OutputError> {
            let (tx, rx) = mpsc::sync_channel(0);
            self.0
                .send((source.clone(), Some(tx)))
                .map_err(|_| disconnected())?;
            rx.recv().map_err(|_| disconnected())
        }
        fn emit_owned(&mut self, source: GeneratedSource) -> Result<(), OutputError> {
            self.0.send((source, None)).map_err(|_| disconnected())
        }
    }
    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::sync_channel(64);
        let producer = scope.spawn(move || {
            emit_inner(
                files,
                options,
                &mut QueuedSink(tx),
                profile,
                external,
                cache,
            )
        });
        let mut failure = None;
        for (source, receipt) in &rx {
            if let Err(error) = sink.emit(&source) {
                failure = Some(error);
                break;
            }
            if let Some(receipt) = receipt {
                let _ = receipt.send(());
            }
        }
        // Disconnect before joining: a failed writer must unblock the producer.
        drop(rx);
        // Deferred sink work must complete and report its first failure before
        // export success can be claimed.
        if failure.is_none()
            && let Err(error) = sink.flush_pending()
        {
            failure = Some(error);
        }
        let result = producer.join().unwrap_or_else(|_| {
            Err(OutputError::Io(std::io::Error::other(
                "Java export producer panicked",
            )))
        });
        match failure {
            Some(error) => Err(error),
            None => result,
        }
    })
}

fn emit_inner(
    files: &[DexFile],
    options: &JavaOptions,
    sink: &mut impl SourceSink,
    profile: &mut Option<&mut JavaProfile>,
    external: Option<&dyn ExternalMetadata>,
    cache: &ssa_cache::SsaCache,
) -> Result<JavaReport, OutputError> {
    if !matches!(options.source_release, 17 | 25) {
        return Err(OutputError::InvalidOptions(format!(
            "source_release {} is unsupported; choose 17 or 25",
            options.source_release
        )));
    }
    // Prep phases only read `files`/`external`; the three scans run on workers
    // while the alias inventory fills on this thread.
    let (writes, super_index, (mut aliases, mut field_publication), global, field_index) =
        std::thread::scope(|scope| -> Result<_, OutputError> {
            let writes_worker = {
                let _timer = Timer::new(
                    profile
                        .as_deref_mut()
                        .map(|p| p.serial.entry("write_inventory").or_default()),
                );
                scope.spawn(|| match external {
                    Some(metadata) => {
                        initializers::write_inventory::WriteInventory::scan_with_metadata(
                            files, metadata,
                        )
                    }
                    None => initializers::write_inventory::WriteInventory::scan(files),
                })
            };
            let super_worker = {
                let _timer = Timer::new(
                    profile
                        .as_deref_mut()
                        .map(|p| p.serial.entry("super_index").or_default()),
                );
                let super_inputs: Vec<_> = files.iter().collect();
                scope.spawn(move || {
                    crate::java_super_resolution::SuperIndex::collect(
                        &super_inputs,
                        external.unwrap_or(&MissingMetadata),
                    )
                    .ok()
                })
            };
            let global_worker = {
                let _timer = Timer::new(
                    profile
                        .as_deref_mut()
                        .map(|p| p.serial.entry("global_indices").or_default()),
                );
                scope.spawn(|| {
                    (
                        GlobalDeclarations::new(files).map_err(OutputError::InvalidDescriptor),
                        crate::java_members::external_metadata::fields::InputIndex::new(files),
                    )
                })
            };
            let fill = {
                let _timer = Timer::new(
                    profile
                        .as_deref_mut()
                        .map(|p| p.serial.entry("alias_inventory").or_default()),
                );
                build_alias_inventory(files)
            };
            let writes = {
                let _timer = Timer::new(
                    profile
                        .as_deref_mut()
                        .map(|p| p.serial.entry("write_inventory").or_default()),
                );
                match writes_worker.join() {
                    Ok(writes) => writes,
                    Err(_) => {
                        return Err(OutputError::Io(std::io::Error::other(
                            "Java export write-inventory worker panicked",
                        )));
                    }
                }
            };
            let super_index = {
                let _timer = Timer::new(
                    profile
                        .as_deref_mut()
                        .map(|p| p.serial.entry("super_index").or_default()),
                );
                match super_worker.join() {
                    Ok(super_index) => super_index,
                    Err(_) => {
                        return Err(OutputError::Io(std::io::Error::other(
                            "Java export super-index worker panicked",
                        )));
                    }
                }
            };
            let mut inventory = fill?;
            {
                let _timer = Timer::new(
                    profile
                        .as_deref_mut()
                        .map(|p| p.serial.entry("alias_inventory").or_default()),
                );
                extend_alias_inventory(files, &mut inventory, super_index.as_ref())?;
            }
            // Export errors keep precedence over the deferred declaration scan.
            let export = {
                let _timer = Timer::new(
                    profile
                        .as_deref_mut()
                        .map(|p| p.serial.entry("alias_build").or_default()),
                );
                collect_field_publication(files, inventory, &writes)?
            };
            let (global, field_index) = {
                let _timer = Timer::new(
                    profile
                        .as_deref_mut()
                        .map(|p| p.serial.entry("global_indices").or_default()),
                );
                match global_worker.join() {
                    Ok(prep) => prep,
                    Err(_) => {
                        return Err(OutputError::Io(std::io::Error::other(
                            "Java export global-declarations worker panicked",
                        )));
                    }
                }
            };
            Ok((writes, super_index, export, global?, field_index))
        })?;
    let publications = Mutex::new(BTreeMap::new());
    let field_uses = Mutex::new(BTreeMap::new());
    let published_owners = Mutex::new(BTreeMap::new());
    let enum_fields = Mutex::new(BTreeSet::new());
    let annotation_defaults =
        Mutex::new(initializers::annotation_publication::PublishedEnumConstants::default());
    let annotation_guards = Mutex::new(BTreeMap::new());
    let mut deferred_sources = Vec::new();
    let deferred_publications = Mutex::new(BTreeMap::new());

    let mut pending_sources = Vec::new();
    if let Some(external) = external {
        let owners = files
            .iter()
            .flat_map(|dex| {
                dex.classes
                    .iter()
                    .filter_map(|c| descriptor(dex, c.class_idx).map(str::to_owned))
            })
            .collect();
        aliases.apply_external_member_names(&external.member_source_names(), &owners);
    }
    let original_hierarchy = Arc::new(
        crate::java_hierarchy::build_hierarchy(
            &files.iter().collect::<Vec<_>>(),
            external.map(|m| m.hierarchy()).unwrap_or_default(),
        )
        .unwrap_or_default(),
    );
    if let Some(external) = external {
        let phase_timer = Timer::new(
            profile
                .as_deref_mut()
                .map(|p| p.serial.entry("alias_override_align").or_default()),
        );
        aliases.align_override_obligations(files, external, &original_hierarchy);
        drop(phase_timer);
        let phase_timer = Timer::new(
            profile
                .as_deref_mut()
                .map(|p| p.serial.entry("alias_final_bridge_align").or_default()),
        );
        aliases.align_final_bridge_targets(files, external, &original_hierarchy);
        drop(phase_timer);
    }
    let hierarchy = original_hierarchy
        .map_names(|name| {
            aliases
                .class_name(&format!("L{};", name.replace('.', "/")))
                .unwrap_or(name)
                .to_owned()
        })
        .unwrap_or_default();
    let catalog = &global.declaration_metadata;
    let field_throw_catalog =
        crate::java_checked_bound_targets::FieldThrowCatalog::new(files, catalog);
    let mut throws_profile = profile
        .as_ref()
        .map(|_| source_throws::SourceThrowsProfile::default());
    let source_throws_timer = Timer::new(
        profile
            .as_deref_mut()
            .map(|p| p.serial.entry("source_throws").or_default()),
    );
    let source_throws = source_throws::collect_cached_profiled(
        files,
        &aliases,
        catalog,
        external,
        &hierarchy,
        options.workers,
        cache,
        throws_profile.as_mut(),
    );
    drop(source_throws_timer);
    if let (Some(profile), Some(throws)) = (profile.as_deref_mut(), throws_profile) {
        profile.throws_phases = throws.walls;
        profile.throws_tasks.insert("ssa", throws.ssa_time);
        profile.throws_counts.insert("ssa", throws.ssa_count);
        profile
            .throws_counts
            .insert("inspect_method", throws.inspect_method_count);
        profile.throws_counts.insert("tasks", throws.task_count);
        profile
            .throws_tasks
            .insert("inspect_method", throws.inspect_method_time);
        profile.throws_scheduler = throws.scheduler;
    }
    let mut class_catalog = BTreeMap::new();
    let mut field_names: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    if let Some(external) = external {
        for declaration in external.hierarchy() {
            let original = format!("L{};", declaration.name.replace('.', "/"));
            if let Some(access_flags) = external.class_access(&original) {
                let name = aliases
                    .class_name(&original)
                    .unwrap_or(&declaration.name)
                    .to_owned();
                let superclass = declaration.superclass.map(|s| {
                    aliases
                        .class_name(&format!("L{};", s.replace('.', "/")))
                        .unwrap_or(&s)
                        .to_owned()
                });
                class_catalog.insert(
                    name.clone(),
                    Some(backend_java::constructors::ConstructorClass {
                        name,
                        superclass,
                        access_flags,
                    }),
                );
            }
        }
    }
    let mut input_class_names = BTreeSet::new();
    for dex in files {
        for class in &dex.classes {
            if let Some(name) = descriptor(dex, class.class_idx).and_then(|d| aliases.class_name(d))
            {
                let declaration = backend_java::constructors::ConstructorClass {
                    name: name.into(),
                    superclass: descriptor(dex, class.superclass_idx)
                        .and_then(|d| aliases.class_name(d))
                        .map(str::to_owned),
                    access_flags: class.access_flags,
                };
                let value = input_class_names
                    .insert(name.to_owned())
                    .then_some(declaration);
                class_catalog.insert(name.into(), value);
                let names = field_names.entry(name.into()).or_default();
                if let Some(data) = &class.class_data {
                    for field in data.static_fields.iter().chain(&data.instance_fields) {
                        if let Some(key) = field_key(dex, field.field_idx)
                            && let Some(name) = aliases.field_name(&key)
                        {
                            names.insert(name.into());
                        }
                    }
                }
            }
        }
    }
    let input_owners: BTreeSet<String> = files
        .iter()
        .flat_map(|dex| {
            dex.classes
                .iter()
                .filter_map(|class| descriptor(dex, class.class_idx).map(str::to_owned))
        })
        .collect();
    let mut report = JavaReport::default();
    let field_use_cache = options.cache_resolutions.then(|| {
        crate::java_members::external_metadata::fields::publication::cache::FieldUseCache::new(
            true,
            files,
            external.unwrap_or(&MissingMetadata),
            &aliases,
            Some(&field_index),
        )
    });
    let timer = Timer::new(
        profile
            .as_deref_mut()
            .map(|p| p.serial.entry("resolver_prepare").or_default()),
    );
    let dex_metadata: Vec<_> = files
        .iter()
        .map(dex_decompiler::infer::DexMetadata::new)
        .collect();
    let member_indices: Vec<_> = files
        .iter()
        .map(crate::java_members::MemberIndex::new)
        .collect();
    drop(timer);
    let mut resolvers = Vec::with_capacity(files.len());
    for (dex_index, dex) in files.iter().enumerate() {
        let timer = Timer::new(
            profile
                .as_deref_mut()
                .map(|p| p.serial.entry("resolver_prepare").or_default()),
        );
        let metadata = &dex_metadata[dex_index];
        let member_index = &member_indices[dex_index];
        let mut r = Resolver::with_catalog(dex, &aliases, metadata, Some(catalog));
        r.source_release = options.source_release;
        r.cache_super_bindings = options.cache_resolutions;
        r.ssa_cache = Some(cache);
        r.cache_inventory = options.cache_inventory;
        r.member_index = Some(member_index);
        r.source_throws = Some(&source_throws);
        r.field_throw_catalog = Some(&field_throw_catalog);
        r.publications = Some(&publications);
        r.field_catalog = Some(&field_publication);
        r.field_index = Some(&field_index);
        r.field_uses = Some(&field_uses);
        r.published_owners = Some(&published_owners);
        r.enum_fields = Some(&enum_fields);
        r.annotation_defaults = options.limit.is_none().then_some(&annotation_defaults);
        r.defer_annotation_defaults = options.limit.is_none();
        r.inputs = files;
        r.super_index = super_index.as_ref();
        r.class_catalog = class_catalog.clone();
        r.dex_index = dex_index;
        r.writes = Some(&writes);
        r.hierarchy = hierarchy.clone();
        r.original_hierarchy = Some(Arc::clone(&original_hierarchy));
        r.field_names = field_names.clone();
        r.external = external;
        r.input_owners = input_owners.clone();
        // Enable only after the immutable metadata, source-throws plan and
        // aliases are final. Test resolvers retain uncached mutable setup.
        r.method_cache = Some(resolution_cache::LookupCache::new(
            options.cache_resolutions,
            dex.method_ids.len(),
        ));
        r.field_cache = Some(resolution_cache::LookupCache::new(
            options.cache_resolutions,
            dex.field_ids.len(),
        ));
        r.field_use_cache = field_use_cache.as_ref();
        drop(timer);
        resolvers.push(r);
    }
    // DEX-major/class-major admission and delivery retain the original order.
    // Each DEX keeps one frozen resolver and its shared resolution caches.
    let class_tasks: Vec<_> = files
        .iter()
        .enumerate()
        .flat_map(|(dex_index, dex)| {
            (0..dex.classes.len()).map(move |class_index| (dex_index, class_index))
        })
        .collect();
    // Duplicate owners share publication keys. Keep their original DEX-order
    // overwrite semantics rather than allowing cross-DEX last-writer races.
    let spans = if class_catalog.values().any(Option::is_none) {
        let mut offset = 0;
        files
            .iter()
            .map(|dex| {
                let start = offset;
                offset += dex.classes.len();
                start..offset
            })
            .collect::<Vec<_>>()
    } else {
        std::iter::once(0..class_tasks.len()).collect()
    };
    // Unlimited exports deliver per-ordinal results without ordered admission:
    // each class writes into its own slot and the logical order is reassembled
    // from ordinals afterwards. Heavy classes dispatch first so their long
    // planning tails never strand the remaining workers; a static code-size
    // proxy orders them without running any task early.
    let dispatch_order: Option<Vec<usize>> = options.limit.is_none().then(|| {
        // Static cost proxy: whole-class code volume dominates task time
        // (initialization-family planning scales with method code), so the
        // heaviest ordinals dispatch first and the tail stays lightweight.
        let weight = |ordinal: usize| -> u64 {
            let (dex_index, class_index) = class_tasks[ordinal];
            let Some(data) = files[dex_index].classes[class_index].class_data.as_ref() else {
                return 0;
            };
            let methods = data.direct_methods.len() + data.virtual_methods.len();
            let code_units: u64 = data
                .direct_methods
                .iter()
                .chain(&data.virtual_methods)
                .filter_map(|method| method.code.as_ref())
                .map(|code| code.insns_size as u64)
                .sum();
            (code_units << 8) | methods as u64
        };
        let mut order: Vec<usize> = (0..class_tasks.len()).collect();
        order.sort_by_key(|&ordinal| std::cmp::Reverse(weight(ordinal)));
        order
    });
    let mut scheduling_result = Ok(());
    for (span_index, span) in spans.into_iter().enumerate() {
        let scheduler_start = profile.as_ref().map(|_| std::time::Instant::now());
        let mut span_sources: Vec<Option<Vec<GeneratedSource>>> =
            (0..span.len()).map(|_| None).collect();
        let mut span_profiles: Vec<(usize, ClassProfile)> = Vec::new();
        let mut report_part = |ordinal: usize,
                               part: JavaReport,
                               sources: Vec<GeneratedSource>,
                               task_profile: Option<ClassProfile>| {
            if let Some(task) = task_profile {
                span_profiles.push((ordinal, task));
            }
            if options.limit.is_some_and(|l| report.classes_emitted >= l) {
                report.classes_seen += 1;
                report.skipped_classes += 1;
                return Ok(());
            }
            span_sources[ordinal] = Some(sources);
            report.classes_seen += part.classes_seen;
            report.classes_emitted += part.classes_emitted;
            report.skipped_classes += part.skipped_classes;
            report.complete_methods += part.complete_methods;
            report.complete_constructors += part.complete_constructors;
            report.complete_static_initializers += part.complete_static_initializers;
            report.semantic_bodies_with_unresolved_dependencies +=
                part.semantic_bodies_with_unresolved_dependencies;
            report.semantic_bodies_outside_strict_dependency_closure +=
                part.semantic_bodies_outside_strict_dependency_closure;
            report.declaration_only_methods += part.declaration_only_methods;
            report.fields_emitted += part.fields_emitted;
            report.unsupported_methods += part.unsupported_methods;
            report.unavailable_constructors += part.unavailable_constructors;
            report.unavailable_initialization_units += part.unavailable_initialization_units;
            report
                .unpublished_method_reasons
                .extend(part.unpublished_method_reasons);
            for (reason, count) in part.category_reasons {
                *report.category_reasons.entry(reason).or_default() += count;
            }
            for (reason, count) in part.reasons {
                *report.reasons.entry(reason).or_default() += count;
            }
            Ok(())
        };
        let task_class = |index: usize| {
            let (dex_index, class_index) = class_tasks[span.start + index];
            let dex = &files[dex_index];
            let r = &resolvers[dex_index];
            let class = &dex.classes[class_index];
            let task_start = scheduler_start.map(|_| std::time::Instant::now());
            let mut task_profile =
                task_start
                    .zip(scheduler_start)
                    .map(|(start, scheduler_start)| ClassProfile {
                        dex: dex_index,
                        class: class.class_idx,
                        descriptor: descriptor(dex, class.class_idx).unwrap_or("?").into(),
                        start: start.duration_since(scheduler_start),
                        ..ClassProfile::default()
                    });
            let mut sources = Vec::new();
            let mut collect = |s: &GeneratedSource| {
                sources.push(s.clone());
                Ok(())
            };
            let result = emit_classes(
                dex,
                r,
                std::slice::from_ref(class),
                &JavaOptions {
                    limit: None,
                    ..options.clone()
                },
                &mut collect,
                &mut task_profile,
            );
            if let (Some(p), Some(start)) = (task_profile.as_mut(), task_start) {
                p.elapsed = start.elapsed();
            }
            (result, sources, task_profile)
        };
        let (result, mut scheduling) = match dispatch_order.as_ref() {
            Some(order) => {
                let span_order: Vec<usize> = order[span.start..span.end]
                    .iter()
                    .map(|ordinal| ordinal - span.start)
                    .collect();
                scheduler::run_unordered(
                    &span_order,
                    options.workers,
                    task_class,
                    |ordinal, (result, sources, task_profile)| {
                        report_part(ordinal, result?, sources, task_profile)
                    },
                    || OutputError::Io(std::io::Error::other("Java class worker panicked")),
                )
            }
            None => scheduler::run(
                span.len(),
                options.workers,
                task_class,
                |ordinal, (result, sources, task_profile)| {
                    report_part(ordinal, result?, sources, task_profile)
                },
                || OutputError::Io(std::io::Error::other("Java class worker panicked")),
            ),
        };
        for slot in span_sources.into_iter().flatten() {
            pending_sources.extend(slot);
        }
        if let Some(p) = profile.as_deref_mut() {
            span_profiles.sort_by_key(|(ordinal, _)| *ordinal);
            p.classes
                .extend(span_profiles.into_iter().map(|(_, task)| task));
        }
        scheduling.dex = span_index;
        if let Some(p) = profile.as_deref_mut() {
            p.schedulers.push(scheduling);
        }
        if result.is_err() {
            scheduling_result = result;
            break;
        }
    }
    if let Some(p) = profile.as_deref_mut() {
        p.resolution_cache_enabled = options.cache_resolutions;
        p.field_use_cache_enabled = options.cache_resolutions;
        for r in &resolvers {
            if let (Some(methods), Some(fields)) = (&r.method_cache, &r.field_cache) {
                for stats in [methods.stats(), fields.stats()] {
                    p.resolution_cache_hits += stats.hits;
                    p.resolution_cache_misses += stats.misses;
                    p.resolution_cache_failures += stats.failures;
                }
            }
        }
    }
    scheduling_result?;
    drop(resolvers);
    let phase_timer = Timer::new(
        profile
            .as_deref_mut()
            .map(|p| p.serial.entry("deferred_annotation_prepare").or_default()),
    );
    // Only unlimited exports enable this bounded extension. Quarantined source
    // plans are private and do not enter public closure or successful counts.
    if options.limit.is_none() {
        // Deferred annotation units from every input share one scheduler; ordered
        // delivery retains the original DEX-order, class-order acknowledgement
        // sequence, and frozen resolvers are only built for contributing inputs.
        let mut deferred_tasks: Vec<(usize, usize)> = Vec::new();
        let mut deferred_pools = vec![false; files.len()];
        let mut duplicated_deferred_owners = BTreeSet::new();
        let mut deferred_owner_duplicated = false;
        for (dex_index, dex) in files.iter().enumerate() {
            for (class_index, class) in dex.classes.iter().enumerate() {
                if class_kind(class.access_flags) == Ok(ClassKind::Annotation)
                    && initializers::annotation_publication::requires_deferred_defaults(dex, class)
                {
                    deferred_pools[dex_index] = true;
                    if let Some(owner) = descriptor(dex, class.class_idx)
                        && !duplicated_deferred_owners.insert(owner)
                    {
                        deferred_owner_duplicated = true;
                    }
                    deferred_tasks.push((dex_index, class_index));
                }
            }
        }
        let deferred_metadata: Vec<_> = files
            .iter()
            .enumerate()
            .map(|(dex_index, dex)| {
                deferred_pools[dex_index].then(|| dex_decompiler::infer::DexMetadata::new(dex))
            })
            .collect();
        // Only contributing inputs get a frozen resolver; duplicate owners keep
        // the original per-DEX scheduling below so publication-key overwrite
        // semantics cannot become a cross-DEX race.
        let mut deferred_resolvers: Vec<Option<Resolver>> =
            (0..files.len()).map(|_| None).collect();
        for (dex_index, dex) in files.iter().enumerate() {
            if deferred_pools[dex_index]
                && let Some(metadata) = deferred_metadata[dex_index].as_ref()
            {
                let mut r = Resolver::with_catalog(dex, &aliases, metadata, Some(catalog));
                r.inputs = files;
                r.dex_index = dex_index;
                r.external = external;
                r.input_owners = input_owners.clone();
                r.original_hierarchy = Some(Arc::clone(&original_hierarchy));
                r.hierarchy = hierarchy.clone();
                r.source_release = options.source_release;
                r.field_catalog = Some(&field_publication);
                r.field_index = Some(&field_index);
                r.source_throws = Some(&source_throws);
                r.field_throw_catalog = Some(&field_throw_catalog);
                r.writes = Some(&writes);
                r.class_catalog = class_catalog.clone();
                r.field_names = field_names.clone();
                r.annotation_defaults = Some(&annotation_defaults);
                r.annotation_guards = Some(&annotation_guards);
                r.publications = Some(&deferred_publications);
                deferred_resolvers[dex_index] = Some(r);
            }
        }
        let deferred_options = JavaOptions {
            limit: None,
            ..options.clone()
        };
        // No global owner/field/publication token is promoted during preview.
        // The frozen resolvers are shared; dependency receipts remain quarantined
        // until the existing serial acknowledgement pass below. Duplicate owners
        // keep the original per-DEX scheduling so publication-key overwrite
        // semantics cannot become a cross-DEX race.
        let mut run_deferred_span = |tasks: &[(usize, usize)]| -> Result<(), OutputError> {
            let (result, _) = scheduler::run(
                tasks.len(),
                options.workers,
                |index| {
                    let (dex_index, class_index) = tasks[index];
                    let dex = &files[dex_index];
                    let mut sources = Vec::new();
                    let mut collect = |s: &GeneratedSource| {
                        sources.push(s.clone());
                        Ok(())
                    };
                    let part = match &deferred_resolvers[dex_index] {
                        Some(r) => emit_classes(
                            dex,
                            r,
                            std::slice::from_ref(&dex.classes[class_index]),
                            &deferred_options,
                            &mut collect,
                            &mut None,
                        ),
                        None => Err(OutputError::Io(std::io::Error::other(
                            "Java annotation resolver unavailable",
                        ))),
                    };
                    (part, sources)
                },
                |_, (part, sources)| {
                    deferred_sources.push((part?, sources));
                    Ok(())
                },
                || OutputError::Io(std::io::Error::other("Java annotation worker panicked")),
            );
            result
        };
        if deferred_owner_duplicated {
            let mut span_start = 0;
            while span_start < deferred_tasks.len() {
                let dex_index = deferred_tasks[span_start].0;
                let span_end = deferred_tasks[span_start..]
                    .iter()
                    .position(|(pool, _)| *pool != dex_index)
                    .map_or(deferred_tasks.len(), |offset| span_start + offset);
                run_deferred_span(&deferred_tasks[span_start..span_end])?;
                span_start = span_end;
            }
        } else {
            run_deferred_span(&deferred_tasks)?;
        }
    }
    drop(phase_timer);
    // Actual external sink acknowledgements are serial, deterministic, and
    // distinct from worker collection. Enum requirements cannot use preparation.
    let mut prepared_enums = Vec::new();
    let mut ordinary_sources = Vec::new();
    for source in pending_sources {
        if annotation_defaults
            .lock()
            .map_err(|_| OutputError::InvalidDescriptor("enum receipt lock".into()))?
            .prepared_owner(&source.provenance.descriptor)
        {
            prepared_enums.push(source);
        } else {
            ordinary_sources.push(source);
        }
    }
    let mut published = publications
        .into_inner()
        .map_err(|_| OutputError::InvalidDescriptor("method publication lock".into()))?;
    let mut acknowledged_dependency_owners = BTreeSet::new();
    for mut source in prepared_enums {
        acknowledged_dependency_owners.insert(source.provenance.descriptor.clone());
        if options.source_release == 25 {
            source
                .text
                .insert_str(0, "// Required Java source release: 25\n");
        }
        source.text.push_str("// This dependency unit was acknowledged before the global closure audit; no complete-class closure claim is made.\n");
        let _timer = Timer::new(
            profile
                .as_deref_mut()
                .map(|p| p.serial.entry("sink_io").or_default()),
        );
        if annotation_defaults
            .lock()
            .map_err(|_| OutputError::InvalidDescriptor("enum receipt lock".into()))?
            .prepared_owner(&source.provenance.descriptor)
        {
            annotation_defaults
                .lock()
                .map_err(|_| OutputError::InvalidDescriptor("enum receipt lock".into()))?
                .emit_enum(&source, sink)?;
        } else {
            sink.emit(&source)?;
        }
    }
    let guards = annotation_guards
        .into_inner()
        .map_err(|_| OutputError::InvalidDescriptor("annotation guard lock".into()))?;
    let mut acknowledged_annotations = BTreeSet::new();
    for (part, sources) in deferred_sources {
        for mut source in sources {
            if options.source_release == 25 {
                source
                    .text
                    .insert_str(0, "// Required Java source release: 25\n");
            }
            source.text.push_str("// This dependency unit was acknowledged before the global closure audit; no complete-class closure claim is made.\n");
            let requirements = guards
                .get(&source.provenance.descriptor)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            annotation_defaults
                .lock()
                .map_err(|_| OutputError::InvalidDescriptor("enum receipt lock".into()))?
                .emit_annotation(&source, requirements, sink)?;
            acknowledged_dependency_owners.insert(source.provenance.descriptor.clone());
            acknowledged_annotations.insert(source.provenance.descriptor);
        }
        merge_deferred_annotation_report(&mut report, part);
    }
    // Only actual acknowledged source units now make their private declarations
    // eligible for source closure; no prepared plan can enter earlier audits.
    for (key, body) in deferred_publications
        .into_inner()
        .map_err(|_| OutputError::InvalidDescriptor("annotation publication lock".into()))?
    {
        if acknowledged_annotations.contains(&key.owner) {
            published.insert(key, body);
        }
    }
    pending_sources = ordinary_sources;
    let emitted_owners: BTreeSet<_> = pending_sources
        .iter()
        .map(|s| s.provenance.descriptor.clone())
        .chain(acknowledged_dependency_owners)
        .collect();
    // Workers may finish units beyond the user's publication limit. Completed
    // analysis is not evidence that their declarations reached the source sink.
    published.retain(|key, _| emitted_owners.contains(&key.owner));
    let provider = external.unwrap_or(&MissingMetadata);
    let phase_timer = Timer::new(
        profile
            .as_deref_mut()
            .map(|p| p.serial.entry("method_closure").or_default()),
    );
    let mut closure = global.close(published.keys().cloned(), provider, |key, _location| {
        published
            .get(key)
            .cloned()
            .ok_or_else(|| "original semantic body unavailable in exported source".into())
    });
    drop(phase_timer);
    // A genuine native/abstract definition still needs actual source publication.
    let bodyless: BTreeSet<_> = closure
        .bodyless
        .keys()
        .cloned()
        .chain(
            published
                .keys()
                .filter(|key| {
                    global
                        .declaration_metadata
                        .get(*key)
                        .and_then(Option::as_ref)
                        .is_some_and(|metadata| {
                            matches!(metadata.access_flags & 0x500, 0x100 | 0x400)
                        })
                })
                .cloned(),
        )
        .collect();
    for key in bodyless {
        if let Some(body) = published.get(&key) {
            // Raw-name helper rejection is not an availability failure when the
            // production declaration validated original flags and frozen aliases.
            closure.unresolved.remove(&key);
            closure.bodyless.insert(key, body.source.clone());
        } else {
            closure.bodyless.remove(&key);
            closure
                .unresolved
                .insert(key, "original bodyless declaration not published".into());
        }
    }
    for (owner, initialized) in published_owners
        .into_inner()
        .map_err(|_| OutputError::InvalidDescriptor("owner publication lock".into()))?
    {
        if !emitted_owners.contains(&owner) {
            continue;
        }
        let initialized = initialized
            && field_publication
                .encoded
                .get(&owner)
                .is_some_and(Result::is_ok)
            && !field_publication.limitations.contains_key(&owner);
        let _ = field_publication.record_owner_publication(&owner, initialized);
    }
    // Ordinary-class initialization cannot attest enum fields. Only the enum
    // renderer's exact emitted declarations and proved constant initialization
    // enter this separate publication token; compiler backing arrays do not.
    for key in enum_fields
        .into_inner()
        .map_err(|_| OutputError::InvalidDescriptor("enum field attestation lock".into()))?
    {
        if emitted_owners.contains(&key.owner)
            && field_publication
                .declarations
                .get(&key)
                .is_some_and(|declaration| declaration.kind == ClassKind::Enum)
            && let Some(status) = field_publication.statuses.get_mut(&key)
        {
            *status = crate::java_members::external_metadata::fields::publication::FieldStatus::InitializedDeclaration;
        }
    }
    let phase_timer = Timer::new(
        profile
            .as_deref_mut()
            .map(|p| p.serial.entry("final_field_require").or_default()),
    );
    for (caller, uses) in field_uses
        .into_inner()
        .map_err(|_| OutputError::InvalidDescriptor("field publication lock".into()))?
    {
        if !emitted_owners.contains(&caller.owner) {
            continue;
        }
        for (key, receiver, is_static) in uses {
            let context = crate::java_members::external_metadata::FieldAccessContext {
                caller: &caller.owner,
                receiver: receiver.as_deref(),
                is_static,
            };
            if let Some(cache) = &field_use_cache {
                let _ = field_publication.require_field_cached(&caller, &key, &context, cache);
            } else {
                let _ = field_publication.require_field_indexed(
                    files,
                    &caller,
                    &key,
                    provider,
                    &context,
                    Some(&field_index),
                );
            }
        }
    }
    drop(phase_timer);
    if let (Some(p), Some(cache)) = (profile.as_deref_mut(), &field_use_cache) {
        let stats = cache.stats();
        p.field_use_cache_entries = stats.entries;
        for stage in [stats.resolution, stats.source_use, stats.retain] {
            p.field_use_cache_hits += stage.hits;
            p.field_use_cache_misses += stage.misses;
            p.field_use_cache_failures += stage.failures;
        }
    }
    let phase_timer = Timer::new(
        profile
            .as_deref_mut()
            .map(|p| p.serial.entry("field_audit").or_default()),
    );
    field_publication.audit();
    drop(phase_timer);
    let mut evidence = BTreeMap::<String, Vec<String>>::new();
    for (owner, limitations) in &field_publication.limitations {
        evidence.entry(owner.clone()).or_default().push(format!(
            "// Global field inventory limitations: {limitations:?}"
        ));
    }
    for (caller, missing) in closure.direct_missing() {
        evidence
            .entry(caller.owner.clone())
            .or_default()
            .push(format!(
                "// Global method source closure: caller={caller:?}; missing={missing:?}"
            ));
    }
    for (caller, blockers) in &field_publication.unresolved {
        if !blockers.is_empty() {
            evidence
                .entry(caller.owner.clone())
                .or_default()
                .push(format!(
                    "// Global field source closure: caller={caller:?}; blockers={blockers:?}"
                ));
        }
    }
    for mut source in pending_sources {
        if options.source_release == 25 {
            source
                .text
                .insert_str(0, "// Required Java source release: 25\n");
        }
        if let Some(lines) = evidence.get(&source.provenance.descriptor) {
            for line in lines {
                source.text.push_str(&line.replace("\\u", "\\ u"));
                source.text.push('\n');
            }
        }
        let _timer = Timer::new(
            profile
                .as_deref_mut()
                .map(|p| p.serial.entry("sink_io").or_default()),
        );
        // In pipelined exports sink_io measures bounded delivery admission;
        // actual sink writes overlap producer teardown on the calling thread.
        sink.emit_owned(source)?;
    }
    Ok(report)
}

fn merge_deferred_annotation_report(report: &mut JavaReport, part: JavaReport) {
    report
        .unpublished_method_reasons
        .extend(part.unpublished_method_reasons);
    report.classes_seen += part.classes_seen;
    report.classes_emitted += part.classes_emitted;
    report.skipped_classes += part.skipped_classes;
    report.complete_methods += part.complete_methods;
    report.complete_constructors += part.complete_constructors;
    report.complete_static_initializers += part.complete_static_initializers;
    report.semantic_bodies_with_unresolved_dependencies +=
        part.semantic_bodies_with_unresolved_dependencies;
    report.semantic_bodies_outside_strict_dependency_closure +=
        part.semantic_bodies_outside_strict_dependency_closure;
    report.declaration_only_methods += part.declaration_only_methods;
    report.fields_emitted += part.fields_emitted;
    report.unsupported_methods += part.unsupported_methods;
    report.unavailable_constructors += part.unavailable_constructors;
    report.unavailable_initialization_units += part.unavailable_initialization_units;
    for (reason, count) in part.category_reasons {
        *report.category_reasons.entry(reason).or_default() += count;
    }
    for (reason, count) in part.reasons {
        *report.reasons.entry(reason).or_default() += count;
    }
}

// Seed failures against the original body set, then propagate only removals.
// FIFO propagation preserves simultaneous-round reasons: initially invalid
// fields keep their field reason even when a callee fails in the same round.
fn closure_failures<'a>(
    candidates: impl Iterator<Item = (u32, bool, &'a MemberRequirements)>,
    declared_fields: &BTreeSet<u32>,
) -> BTreeMap<u32, String> {
    let candidates: Vec<_> = candidates.collect();
    let complete = candidates.iter().map(|(index, _, _)| *index).collect();
    let mut reverse = BTreeMap::<u32, Vec<u32>>::new();
    let mut failures = BTreeMap::new();
    let mut queue = VecDeque::new();
    for &(index, _, requirements) in &candidates {
        for &target in &requirements.methods {
            reverse.entry(target).or_default().push(index);
        }
        if let Err(reason) = requirements.check_closure(&complete, declared_fields) {
            failures.insert(index, reason);
            queue.push_back(index);
        }
    }
    while let Some(index) = queue.pop_front() {
        if let Some(callers) = reverse.get(&index) {
            for &caller in callers {
                if let std::collections::btree_map::Entry::Vacant(entry) = failures.entry(caller) {
                    entry.insert("required complete method body unavailable".into());
                    queue.push_back(caller);
                }
            }
        }
    }
    failures
}

type InitializationSource = (BTreeMap<u32, Candidate>, BTreeMap<u32, String>, String);
// Constructor-body recovery does not attest unrelated static initialization.
// The subset planner proves instance-final paths and every original chain.
// Only a receiver-bound original initializer can attest this delegation.
// New objects allocated in argument expressions have different SSA identities.
fn attested_this_delegation(r: &Resolver<'_>, reference: MethodRef) -> Option<MethodRef> {
    let original = method_key(r.dex, reference.index())?;
    let method = r
        .constructor_definitions
        .get(&reference.index())
        .copied()
        .flatten()?;
    let code = method.code.as_ref()?;
    if !code.tries.is_empty() {
        return None;
    }
    let signature = r.method(reference)?;
    let (context, seeds) = initial_context_with_seeds(r, method).ok()?;
    let ssa = cached_frontend(r, method, code, &context, seeds, None).ok()?;
    let bindings = declaration_bindings(&ssa, &signature, code).ok()?;
    if let Some(target) = attested_mask_this_delegation(r, reference, ssa.ir(), &bindings) {
        return Some(target);
    }
    let receiver = bindings.receiver?;
    attested_this_target(ssa.ir(), receiver, &original.owner, |target| {
        method_key(r.dex, target.index())
    })
}

fn attested_mask_this_delegation(
    r: &Resolver<'_>,
    reference: MethodRef,
    ir: &MethodIr<SsaForm>,
    bindings: &DeclarationBindings,
) -> Option<MethodRef> {
    let resolver = MaskDelegationResolver { inner: r };
    backend_java::constructors::mask_choices::plan(
        &backend_java::constructors::ConstructorInput {
            reference,
            ir,
            bindings,
        },
        &resolver,
    )
    .ok()
    .map(|plan| plan.target)
}

struct MaskDelegationResolver<'a, 'r> {
    inner: &'a Resolver<'r>,
}
impl JavaResolver for MaskDelegationResolver<'_, '_> {
    fn array_clone_method(&self, reference: MethodRef, receiver: &JavaType) -> Option<JavaMethod> {
        self.inner.array_clone_method(reference, receiver)
    }
    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        self.inner.method(reference)
    }
    fn field(&self, reference: FieldRef) -> Option<JavaField> {
        self.inner.field(reference)
    }
    fn class(&self, reference: ClassRef) -> Option<String> {
        self.inner.class(reference)
    }
    fn string(&self, reference: StringRef) -> Option<Vec<u16>> {
        self.inner.string(reference)
    }
    fn discarded_invoke_source_target(&self, reference: MethodRef) -> bool {
        self.inner.discarded_invoke_source_target(reference)
    }
    fn method_access(&self, reference: MethodRef) -> Option<u32> {
        self.inner.method_access(reference)
    }
    fn field_access(&self, reference: FieldRef) -> Option<u32> {
        self.inner.field_access(reference)
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        self.inner.assignable(actual, expected)
    }
    fn instruction_type(&self, reference: TypeRef) -> Option<JavaType> {
        self.inner.instruction_type(reference)
    }
    fn exception_allocation_chain(&self, chain: &[managed_ir::value::InsnId]) -> bool {
        self.inner.exception_allocation_chain(chain)
    }
    fn exception_allocation_sequence_chain(&self, chain: &[managed_ir::value::InsnId]) -> bool {
        self.inner.exception_allocation_sequence_chain(chain)
    }
    fn exception_allocation_pure_chain(&self, chain: &[managed_ir::value::InsnId]) -> bool {
        self.inner.exception_allocation_pure_chain(chain)
    }
    fn trusted_string_builder_class(&self, reference: ClassRef) -> bool {
        self.inner.trusted_string_builder_class(reference)
    }
    fn java_super_selects_dex_target(&self, caller: &str, reference: MethodRef) -> bool {
        self.inner.java_super_selects_dex_target(caller, reference)
    }
    fn super_invocation(
        &self,
        caller: &str,
        reference: MethodRef,
    ) -> Option<backend_java::SuperInvocation> {
        self.inner.super_invocation(caller, reference)
    }
    fn super_interface_qualifier(&self, caller: &str, reference: MethodRef) -> Option<String> {
        self.inner.super_interface_qualifier(caller, reference)
    }
    fn virtual_receiver_method(
        &self,
        reference: MethodRef,
        caller: &str,
        receiver: &JavaType,
    ) -> Option<JavaMethod> {
        self.inner
            .virtual_receiver_method(reference, caller, receiver)
    }
    fn retain_method_receiver(
        &self,
        reference: MethodRef,
        caller: &str,
        receiver: &JavaType,
    ) -> bool {
        self.inner
            .retain_method_receiver(reference, caller, receiver)
    }
    fn static_method_type_context(&self, r: MethodRef, c: &str) -> bool {
        self.inner.static_method_type_context(r, c)
    }
    fn static_method_owner_access(&self, r: MethodRef) -> Option<u32> {
        self.inner.static_method_owner_access(r)
    }
    fn static_field_type_context(&self, reference: FieldRef, caller: &str) -> bool {
        self.inner.static_field_type_context(reference, caller)
    }
    fn field_source_nonconstant(&self, reference: FieldRef) -> bool {
        self.inner.field_source_nonconstant(reference)
    }
    fn retain_field_receiver(
        &self,
        reference: FieldRef,
        caller: &str,
        receiver: &JavaType,
    ) -> bool {
        self.inner
            .retain_field_receiver(reference, caller, receiver)
    }
}
impl backend_java::constructors::ConstructorResolver for MaskDelegationResolver<'_, '_> {
    fn constructor_exception_metadata(
        &self,
        reference: MethodRef,
        ir: &MethodIr<SsaForm>,
    ) -> Option<Box<dyn backend_java::constructors::ConstructorExceptionMetadata + '_>> {
        self.inner.constructor_exception_metadata(reference, ir)
    }
    fn constructor_exception_field_store(&self, reference: FieldRef) -> bool {
        self.inner.constructor_exception_field_store(reference)
    }
    fn initializer(&self, reference: MethodRef) -> Option<backend_java::constructors::Initializer> {
        let method = self.inner.method(reference)?;
        (!method.is_static && method.name == "<init>" && method.return_type == JavaType::Void)
            .then_some(backend_java::constructors::Initializer {
                method,
                delegates_to: None,
            })
    }
    fn constructor_source_level(&self) -> u16 {
        self.inner.source_release
    }
    fn constructor_early_field_store(&self, reference: FieldRef) -> bool {
        self.inner.constructor_early_field_store(reference)
    }
    fn constructor_super_target(&self, caller: &str, reference: MethodRef) -> bool {
        self.inner.constructor_super_target(caller, reference)
    }
    fn exception_covered(&self, exception: &str, declared: &[String]) -> bool {
        self.inner.exception_covered(exception, declared)
    }
}
fn attested_this_target(
    ir: &MethodIr<SsaForm>,
    receiver: managed_ir::value::ValueId,
    owner: &str,
    resolve: impl Fn(MethodRef) -> Option<MethodKey>,
) -> Option<MethodRef> {
    constructor_delegation::attest(ir, receiver, owner, resolve)
}

fn constructor_subset_source(
    class: &ClassDef,
    r: &Resolver<'_>,
    diagnostics: &mut BTreeMap<u32, String>,
) -> Result<BTreeMap<u32, Candidate>, String> {
    let data = class
        .class_data
        .as_ref()
        .ok_or("constructor subset: missing data")?;
    let mut methods = Vec::new();
    let mut ssas = Vec::new();
    let mut bindings = Vec::new();
    for method in data
        .direct_methods
        .iter()
        .filter(|m| method_key(r.dex, m.method_idx).is_some_and(|k| k.name == "<init>"))
    {
        let available = (|| -> Result<_, String> {
            let code = method
                .code
                .as_ref()
                .ok_or("constructor subset: missing body")?;
            if !code.tries.is_empty() {
                return Err("constructor subset: exceptional chain unavailable".into());
            }
            let (context, seeds) = initial_context_with_seeds(r, method)?;
            let ssa = cached_frontend(r, method, code, &context, seeds, None)
                .map_err(|e| format!("constructor subset frontend: {e}"))?;
            let signature = r
                .method(MethodRef::new(method.method_idx))
                .ok_or("constructor subset: signature unavailable")?;
            let binding = declaration_bindings(&ssa, &signature, code)?;
            Ok((ssa, binding))
        })();
        match available {
            Ok((ssa, binding)) => {
                methods.push(method);
                bindings.push(binding);
                ssas.push(ssa);
            }
            Err(error) => {
                diagnostics.insert(method.method_idx, error);
            }
        }
    }
    let inputs: Vec<_> = methods
        .iter()
        .enumerate()
        .map(|(i, m)| backend_java::constructors::ConstructorInput {
            reference: MethodRef::new(m.method_idx),
            ir: ssas[i].ir(),
            bindings: &bindings[i],
        })
        .collect();
    let plan = initializers::plan_constructor_body_reachable_subset_diagnostics(
        r.dex,
        class,
        &inputs,
        r,
        diagnostics,
    )
    .map_err(|e| {
        let reason = format!("constructor subset proof: {e:?}");
        for method in &methods {
            diagnostics
                .entry(method.method_idx)
                .or_insert_with(|| reason.clone());
        }
        reason
    })?;
    let mut candidates = BTreeMap::new();
    for (reference, body) in plan.references.into_iter().zip(plan.constructors) {
        let index = methods
            .iter()
            .position(|m| m.method_idx == reference.index())
            .ok_or("constructor subset: returned identity outside inputs")?;
        let method = methods[index];
        let ssa = &ssas[index];
        let signature = r
            .method(MethodRef::new(method.method_idx))
            .ok_or("constructor subset: signature unavailable")?;
        let requirements = match requirements(ssa.ir(), &signature, r, true) {
            Ok(requirements) => requirements,
            Err(error) => {
                diagnostics.insert(method.method_idx, error);
                continue;
            }
        };
        candidates.insert(
            method.method_idx,
            Candidate {
                dependencies: body_dependencies(ssa.ir(), r, &signature.owner)?,
                field_uses: body_field_uses(ssa.ir(), r.dex),
                source: body.source,
                requirements,
                body: true,
                constructor: true,
            },
        );
    }
    for method in &data.direct_methods {
        if method_key(r.dex, method.method_idx).is_some_and(|key| key.name == "<init>")
            && !candidates.contains_key(&method.method_idx)
        {
            diagnostics.entry(method.method_idx).or_insert_with(||
                "constructor subset: original body not retained by reachable-family proof; finer backend failure unavailable".into());
        }
    }
    Ok(candidates)
}
fn static_initializer_subset_source(
    class: &ClassDef,
    r: &Resolver<'_>,
    precomputed: Option<(initializers::StaticInitializerBodySubset, usize)>,
) -> Result<(u32, Candidate, String), String> {
    let method = class
        .class_data
        .as_ref()
        .ok_or("static subset: no class data")?
        .direct_methods
        .iter()
        .find(|m| method_key(r.dex, m.method_idx).is_some_and(|k| k.name == "<clinit>"))
        .ok_or("static subset: no original initializer")?;
    let code = method.code.as_ref().ok_or("static subset: no code")?;
    let (context, seeds) = initial_context_with_seeds(r, method)?;
    let ssa = cached_frontend(r, method, code, &context, seeds, None)
        .map_err(|e| format!("static subset frontend: {e}"))?;
    let signature = r
        .method(MethodRef::new(method.method_idx))
        .ok_or("static subset: signature unavailable")?;
    // Only the plain attested proof (no exception-scope derivation) is shared
    // with the family plan's internal static proof, and only for clinit
    // without handler tables.
    let precomputed = precomputed.filter(|_| code.tries.is_empty());
    let (plan, runtime_capability_count) =
        if let Some((plan, count)) = precomputed {
            (plan, count)
        } else if r.cache_inventory {
            let proof = if code.tries.is_empty() {
            initializers::plan_static_initializer_shared_explicit_stores_attested(
            r.inputs,
            r.dex_index,
            class,
            MethodRef::new(method.method_idx),
            ssa.as_ref(),
            r,
            r.writes
                .ok_or("static subset: write inventory unavailable")?,
            )
        } else {
            initializers::plan_static_initializer_shared_explicit_stores_with_exceptions_attested(
                r.inputs, r.dex_index, class, MethodRef::new(method.method_idx),
                ssa.as_ref(), r, r.writes.ok_or("static subset: write inventory unavailable")?,
                &r.hierarchy,
            )
        }
        .map_err(|e| format!("static subset proof: {e:?}"))?;
            (proof.plan, proof.runtime_capabilities.len())
        } else {
            let proof = if code.tries.is_empty() {
                initializers::plan_static_initializer_explicit_stores_attested(
                    r.inputs,
                    r.dex_index,
                    class,
                    MethodRef::new(method.method_idx),
                    ssa.as_ref(),
                    r,
                    r.writes
                        .ok_or("static subset: write inventory unavailable")?,
                )
            } else {
                initializers::plan_static_initializer_explicit_stores_with_exceptions_attested(
                    r.inputs,
                    r.dex_index,
                    class,
                    MethodRef::new(method.method_idx),
                    ssa.as_ref(),
                    r,
                    r.writes
                        .ok_or("static subset: write inventory unavailable")?,
                    &r.hierarchy,
                )
            }
            .map_err(|e| format!("static subset proof: {e:?}"))?;
            (proof.plan, proof.runtime_capabilities.len())
        };
    // The completed backend plan has checked exact original handler scopes.
    // This signature is only a dependency preflight view; the emitted clinit
    // remains declaration-less and the backend checks uncovered operations.
    let mut dependency_signature = signature.clone();
    for table in &code.handlers {
        for catch in &table.handlers {
            dependency_signature.throws.push(
                r.class(ClassRef::new(catch.type_idx))
                    .ok_or("static subset: catch declaration unavailable")?,
            );
        }
        if table.catch_all_addr.is_some() {
            dependency_signature
                .throws
                .push("java.lang.Throwable".into());
        }
    }
    let encoded_prefix = render_initialization_prefix(&plan.encoded, r)?;
    let prefix = if runtime_capability_count == 0 {
        encoded_prefix
    } else {
        format!(
            "// Original static stores and Java definite assignment proved; {} runtime native/reflection capability observations retained. Instance initialization remains unavailable.\n{}",
            runtime_capability_count, encoded_prefix,
        )
    };
    Ok((
        method.method_idx,
        Candidate {
            dependencies: body_dependencies(ssa.ir(), r, &signature.owner)?,
            field_uses: body_field_uses(ssa.ir(), r.dex),
            source: plan.source,
            requirements: requirements(ssa.ir(), &dependency_signature, r, false)?,
            body: true,
            constructor: false,
        },
        prefix,
    ))
}
fn render_initialization_prefix(
    plan: &initializers::EncodedInitializationPlan,
    r: &Resolver<'_>,
) -> Result<String, String> {
    // Render the exact successful initialization plan through frozen field
    // aliases. The publication preflight may have rejected an encoded VM
    // default which the later original-SSA proof legitimately absorbed.
    let mut block = String::new();
    for assignment in &plan.assignments {
        let field = r
            .field(FieldRef::new(assignment.field_idx))
            .ok_or("initialization prefix: field metadata unavailable")?;
        if !field.is_static {
            return Err("initialization prefix: non-static assignment".into());
        }
        block.push_str(&format!(
            "    {} = {};\n",
            field.name, assignment.expression
        ));
    }
    Ok(if block.is_empty() {
        block
    } else {
        format!("static {{\n{block}}}\n")
    })
}
fn initialization_source(
    class: &ClassDef,
    r: &Resolver<'_>,
    reuse_static: &mut Option<(initializers::StaticInitializerBodySubset, usize)>,
) -> Result<InitializationSource, String> {
    let data = class
        .class_data
        .as_ref()
        .ok_or("initialization: missing declarations")?;
    let inventory = r.writes.ok_or("initialization: missing inventory")?;
    let has_finals = data
        .static_fields
        .iter()
        .chain(&data.instance_fields)
        .any(|f| f.access_flags & 0x10 != 0);
    if has_finals {
        for field in data
            .static_fields
            .iter()
            .chain(&data.instance_fields)
            .filter(|f| f.access_flags & 0x10 != 0)
        {
            let key = initializers::write_inventory::field_key(r.dex, field.field_idx)
                .ok_or("initialization: invalid final declaration identity")?;
            inventory.explicit_store_writes(&key).map_err(|error| {
                format!("initialization: explicit-store proof unavailable: {error:?}")
            })?;
        }
        // The complete attested plan below validates encoded initialization
        // against the original static SSA. An encoded VM default followed by
        // a proven final store must not be rejected by the legacy preflight.
        // Every final's whole-input store completeness remains checked above.
    } else {
        initializers::plan_encoded_initialization(r.dex, class, inventory)
            .map_err(|error| format!("initialization inventory: {error:?}"))?;
    }
    let methods: Vec<_> = data
        .direct_methods
        .iter()
        .filter(|m| {
            method_key(r.dex, m.method_idx)
                .is_some_and(|k| matches!(k.name.as_str(), "<init>" | "<clinit>"))
        })
        .collect();
    let mut ssas = Vec::new();
    let mut bindings = Vec::new();
    for method in &methods {
        let code = method.code.as_ref().ok_or("initialization: missing body")?;
        let constructor =
            method_key(r.dex, method.method_idx).is_some_and(|key| key.name == "<init>");
        if !code.tries.is_empty() && !constructor {
            return Err("initialization: exception family proof unavailable".into());
        }
        let (context, seeds) = initial_context_with_seeds(r, method)?;
        let ssa = cached_frontend(r, method, code, &context, seeds, None)
            .map_err(|e| format!("initialization frontend: {e}"))?;
        let signature = r
            .method(MethodRef::new(method.method_idx))
            .ok_or("initialization: declaration metadata unavailable")?;
        let bound = declaration_bindings(&ssa, &signature, code)?;
        if !code.tries.is_empty() {
            let metadata =
                crate::java_exception_regions::from_dex_dispatch(&ssa, code, r, &r.hierarchy)
                    .map_err(|error| {
                        format!("initialization: constructor exception metadata: {error}")
                    })?;
            let mut stored = r
                .constructor_exception_scopes
                .lock()
                .map_err(|_| "initialization: constructor exception evidence lock")?;
            let instances = stored.entry(method.method_idx).or_default();
            if !instances
                .iter()
                .any(|original| std::ptr::eq(original.ssa.ir(), ssa.ir()))
            {
                instances.push(Arc::new(StoredConstructorException {
                    ssa: ssa.clone(),
                    metadata,
                }));
            }
        }
        bindings.push(bound);
        ssas.push(ssa);
    }
    let constructors: Vec<_> = methods
        .iter()
        .enumerate()
        .filter(|(_, m)| method_key(r.dex, m.method_idx).is_some_and(|k| k.name == "<init>"))
        .map(|(i, m)| backend_java::constructors::ConstructorInput {
            reference: MethodRef::new(m.method_idx),
            ir: ssas[i].ir(),
            bindings: &bindings[i],
        })
        .collect();
    let clinit = methods
        .iter()
        .enumerate()
        .find(|(_, m)| method_key(r.dex, m.method_idx).is_some_and(|k| k.name == "<clinit>"))
        .map(|(i, m)| (MethodRef::new(m.method_idx), ssas[i].as_ref()));
    // The attested family proof computes this exact static-initializer proof
    // internally; compute it once so the static-subset retry below reuses the
    // same evidence instead of re-proving it after a family rejection.
    let precomputed_static = if has_finals {
        clinit.as_ref().map(|(reference, ssa)| {
            if r.cache_inventory {
                initializers::plan_static_initializer_shared_explicit_stores_attested(
                    r.inputs,
                    r.dex_index,
                    class,
                    *reference,
                    ssa,
                    r,
                    inventory,
                )
                .map(|proof| (proof.plan, proof.runtime_capabilities.len()))
            } else {
                initializers::plan_static_initializer_explicit_stores_attested(
                    r.inputs,
                    r.dex_index,
                    class,
                    *reference,
                    ssa,
                    r,
                    inventory,
                )
                .map(|proof| (proof.plan, proof.runtime_capabilities.len()))
            }
        })
    } else {
        None
    };
    let (plan, runtime_capability_count) = if has_finals {
        let reuse = precomputed_static
            .as_ref()
            .map(|proof| proof.as_ref().map(|(plan, _)| plan));
        let family = if r.cache_inventory {
            match reuse {
                Some(plan) => {
                    initializers::plan_class_initialization_shared_explicit_stores_attested_reusing_static(
                        r.inputs,
                        r.dex_index,
                        class,
                        &constructors,
                        clinit,
                        r,
                        inventory,
                        plan,
                    )
                }
                None => initializers::plan_class_initialization_shared_explicit_stores_attested(
                    r.inputs,
                    r.dex_index,
                    class,
                    &constructors,
                    clinit,
                    r,
                    inventory,
                ),
            }
            .map_err(|error| {
                format!("initialization explicit-store definite assignment: {error:?}")
            })
            .map(|proof| (proof.plan, proof.runtime_capabilities.len()))
        } else {
            match reuse {
                Some(plan) => {
                    initializers::plan_class_initialization_with_explicit_stores_attested_reusing_static(
                        r.inputs,
                        r.dex_index,
                        class,
                        &constructors,
                        clinit,
                        r,
                        inventory,
                        plan,
                    )
                }
                None => initializers::plan_class_initialization_with_explicit_stores_attested(
                    r.inputs,
                    r.dex_index,
                    class,
                    &constructors,
                    clinit,
                    r,
                    inventory,
                ),
            }
            .map_err(|error| {
                format!("initialization explicit-store definite assignment: {error:?}")
            })
            .map(|proof| (proof.plan, proof.runtime_capabilities.len()))
        };
        *reuse_static = precomputed_static.and_then(Result::ok);
        family?
    } else {
        let plan = initializers::plan_class_initialization_with_inventory(
            r.inputs,
            r.dex_index,
            class,
            &constructors,
            clinit.map(|(reference, ssa)| (reference, ssa.ir())),
            r,
            r.writes
                .ok_or("initialization: whole-input inventory unavailable")?,
        )
        .map_err(|e| format!("initialization proof: {e:?}"))?;
        (plan, 0)
    };
    let mut fields = BTreeMap::new();
    for field in data.static_fields.iter().chain(&data.instance_fields) {
        let resolved = r
            .field(FieldRef::new(field.field_idx))
            .ok_or("initialization: unresolved field")?;
        let flags = field.access_flags;
        fields.insert(
            field.field_idx,
            format!(
                "{}{}{}{}{}{} {};",
                visibility(flags)?,
                if resolved.is_static { "static " } else { "" },
                if flags & 0x10 != 0 { "final " } else { "" },
                if flags & 0x40 != 0 { "volatile " } else { "" },
                if flags & 0x80 != 0 { "transient " } else { "" },
                type_source(&resolved.ty),
                resolved.name
            ),
        );
    }
    let mut candidates = BTreeMap::new();
    let mut ctor_plans = plan.constructors.into_iter();
    for (i, method) in methods.iter().enumerate() {
        let signature = r
            .method(MethodRef::new(method.method_idx))
            .ok_or("initialization: missing signature")?;
        let constructor = signature.name == "<init>";
        let source = if constructor {
            ctor_plans
                .next()
                .ok_or("initialization: family coverage mismatch")?
                .source
                .replacen("public ", visibility(method.access_flags)?, 1)
        } else {
            plan.static_initializer
                .clone()
                .ok_or("initialization: missing static plan")?
        };
        candidates.insert(
            method.method_idx,
            Candidate {
                dependencies: body_dependencies(ssas[i].ir(), r, &signature.owner)?,
                field_uses: body_field_uses(ssas[i].ir(), r.dex),
                source,
                requirements: requirements(ssas[i].ir(), &signature, r, constructor)?,
                body: true,
                constructor,
            },
        );
    }
    let encoded_prefix = render_initialization_prefix(&plan.encoded, r)?;
    let prefix = if runtime_capability_count == 0 {
        encoded_prefix
    } else {
        format!(
            "// Original explicit stores and Java definite assignment proved; {} runtime native/reflection capability observations retained. No runtime immutability claim.\n{}",
            runtime_capability_count, encoded_prefix
        )
    };
    Ok((candidates, fields, prefix))
}

struct EnumSource {
    source: String,
    fields: usize,
    methods: usize,
    constructors: usize,
    declarations: usize,
    publications: BTreeMap<MethodKey, EmittedMethod>,
    field_uses: BTreeMap<MethodKey, FieldUses>,
    published_fields: BTreeSet<FieldKey>,
}

fn enum_source(
    class: &ClassDef,
    r: &Resolver<'_>,
    profile: &mut Option<ClassProfile>,
) -> Result<EnumSource, String> {
    use crate::java_class_kinds::enum_initialization::{self as enumeration, emission};
    match enumeration::classification::classify(r.inputs, r.dex, class)
        .map_err(|error| format!("enum ownership: {error:?}"))?
    {
        enumeration::classification::EnumOwnership::Direct => {}
        enumeration::classification::EnumOwnership::ConstantBody {
            child,
            parent,
            methods,
        } => {
            return Err(format!(
                "enum constant body requires original parent integration: child={child}; parent={parent}; methods={methods:?}"
            ));
        }
    }
    let data = class
        .class_data
        .as_ref()
        .ok_or("enum: missing class data")?;
    let original_methods: BTreeSet<_> = data
        .direct_methods
        .iter()
        .chain(&data.virtual_methods)
        .map(|method| method.method_idx)
        .collect();
    if original_methods.len() != data.direct_methods.len() + data.virtual_methods.len() {
        return Err("enum: duplicate original method identities".into());
    }
    let find = |name: &str| {
        data.direct_methods
            .iter()
            .find(|m| method_key(r.dex, m.method_idx).is_some_and(|k| k.name == name))
    };
    let constructors: Vec<_> = data
        .direct_methods
        .iter()
        .filter(|m| method_key(r.dex, m.method_idx).is_some_and(|k| k.name == "<init>"))
        .collect();
    if constructors.is_empty() {
        return Err("enum: missing constructor".into());
    }
    let clinit = find("<clinit>").ok_or("enum: missing initializer")?;
    let lift = |method: &EncodedMethod| {
        let (context, seeds) = initial_context_with_seeds(r, method)?;
        cached_frontend(
            r,
            method,
            method.code.as_ref().ok_or("enum: missing code")?,
            &context,
            seeds,
            None,
        )
        .map_err(|e| format!("enum frontend: {e}"))
    };
    let constructor_ssas = constructors
        .iter()
        .map(|constructor| lift(constructor))
        .collect::<Result<Vec<_>, _>>()?;
    let clinit_ssa = lift(clinit)?;
    let constructor_bindings = constructors
        .iter()
        .zip(&constructor_ssas)
        .map(|(constructor, ssa)| {
            let signature = r
                .method(MethodRef::new(constructor.method_idx))
                .ok_or("enum: unresolved constructor")?;
            declaration_bindings(
                ssa,
                &signature,
                constructor
                    .code
                    .as_ref()
                    .ok_or("enum: missing constructor code")?,
            )
        })
        .collect::<Result<Vec<_>, String>>()?;
    let constructor_inputs = constructors
        .iter()
        .zip(&constructor_ssas)
        .zip(&constructor_bindings)
        .map(
            |((constructor, ssa), bindings)| enumeration::EnumConstructorDeclarationInput {
                reference: MethodRef::new(constructor.method_idx),
                ir: ssa.ir(),
                bindings,
            },
        )
        .collect::<Vec<_>>();
    let init = match r.writes {
        Some(writes) => enumeration::plan_enum_initialization_family_with_prepared_values(
            r.inputs,
            r.dex_index,
            class,
            &constructor_inputs,
            MethodRef::new(clinit.method_idx),
            clinit_ssa.ir(),
            r,
            writes,
        ),
        None => enumeration::plan_enum_initialization_family(
            r.dex,
            class,
            &constructor_inputs,
            MethodRef::new(clinit.method_idx),
            clinit_ssa.ir(),
            r,
        ),
    }
    .map_err(|e| format!("enum initialization: {e:?}"))?;
    if let Some(suffix) = &init.static_initializer_ir {
        let caller = r
            .method(MethodRef::new(init.absorbed_clinit))
            .ok_or("enum suffix: original initializer declaration unavailable")?;
        // The backend proves stores and expression order. The facade separately
        // proves declaration access, checked contracts and original references.
        requirements(suffix, &caller, r, false)
            .map_err(|e| format!("enum suffix requirements: {e}"))?;
    }
    let methods = emission::plan_enum_methods(r.dex, class, &init)
        .map_err(|e| format!("enum methods: {e:?}"))?;
    let absorbed_fields: BTreeSet<_> = init
        .constants
        .iter()
        .map(|c| c.field_idx)
        .chain([init.absorbed_values_field])
        .collect();
    let absorbed_methods: BTreeSet<_> = [
        init.absorbed_constructor,
        init.absorbed_clinit,
        methods.values,
        methods.value_of,
    ]
    .into_iter()
    .chain(init.absorbed_values_helper)
    .chain(init.absorbed_constructors.iter().copied())
    .collect();
    let mut fields = BTreeMap::new();
    for f in data.static_fields.iter().chain(&data.instance_fields) {
        let key = field_key(r.dex, f.field_idx).ok_or("enum: invalid field identity")?;
        if r.aliases.field_name(&key) != Some(key.name.as_str()) {
            return Err("enum: aliased constant closure unproven".into());
        }
        if !absorbed_fields.contains(&f.field_idx) {
            fields.insert(
                f.field_idx,
                r.field_declaration(class, f.field_idx)
                    .map_err(|e| format!("enum field: {e:?}"))?,
            );
        }
    }
    let mut members = BTreeMap::new();
    let mut member_body_evidence = BTreeMap::new();
    let mut publications = BTreeMap::new();
    let mut field_uses = BTreeMap::new();
    for method in data.direct_methods.iter().chain(&data.virtual_methods) {
        if absorbed_methods.contains(&method.method_idx) {
            continue;
        }
        if method.code.is_some() {
            let ssa = lift(method)?;
            emission::validate_enum_member_ir(class, &init, ssa.ir())
                .map_err(|e| format!("enum member: {e:?}"))?;
        }
        let candidate = method_source(class, method, r, ClassKind::Class, profile)?;
        if !candidate.body
            && (method.code.is_some()
                || method.access_flags & 0x100 == 0
                || method.access_flags & 0x400 != 0)
        {
            return Err("enum: bodyless member requires genuine native declaration or proved constant implementations".into());
        }
        if candidate
            .requirements
            .fields
            .iter()
            .any(|f| !fields.contains_key(f) && !absorbed_fields.contains(f))
            || candidate.requirements.methods.iter().any(|m| {
                !absorbed_methods.contains(m)
                    && !data
                        .direct_methods
                        .iter()
                        .chain(&data.virtual_methods)
                        .any(|v| v.method_idx == *m)
            })
        {
            return Err("enum: remaining member closure unavailable".into());
        }
        member_body_evidence.insert(method.method_idx, candidate.body);
        let key = method_key(r.dex, method.method_idx)
            .ok_or("enum: remaining member identity unavailable")?;
        publications.insert(
            key.clone(),
            EmittedMethod {
                source: candidate.source.clone(),
                dependencies: candidate.dependencies,
            },
        );
        field_uses.insert(key, candidate.field_uses);
        members.insert(method.method_idx, candidate.source);
    }
    let source = emission::emit_enum_declaration(class, &init, &methods, &fields, &members)
        .map_err(|e| format!("enum emission: {e:?}"))?;
    let constructor_indices: BTreeSet<_> = init.absorbed_constructors.iter().copied().collect();
    let mut evidence = String::new();
    for index in original_methods {
        let (status, category) = if constructor_indices.contains(&index) {
            ("complete_body", "constructor")
        } else if index == init.absorbed_clinit {
            ("proven_enum_equivalence", "static_initializer")
        } else if absorbed_methods.contains(&index) {
            ("proven_enum_equivalence", "ordinary")
        } else {
            match member_body_evidence.get(&index) {
                Some(true) => ("complete_body", "ordinary"),
                Some(false) => ("declaration_only", "ordinary"),
                None => return Err("enum: original method evidence unavailable".into()),
            }
        };
        evidence.push_str(&format!(
            "// decomp-enum-method {index} {status} {category}\n"
        ));
    }
    let ordinary_bodies = member_body_evidence.values().filter(|body| **body).count();
    let declarations = member_body_evidence.len() - ordinary_bodies;
    let source = format!("{evidence}{source}");
    let compiler_methods: BTreeSet<_> = [methods.values, methods.value_of]
        .into_iter()
        .chain(init.absorbed_values_helper)
        .collect();
    let backing_key = field_key(r.dex, init.absorbed_values_field)
        .ok_or("enum: compiler backing-array identity unavailable")?;
    // These original methods are represented by the actual emitted enum unit,
    // not by invented helper bodies. Keep their original dependency evidence so
    // the global closure pass cannot overlook suffix or constructor references.
    for index in &absorbed_methods {
        let original = data
            .direct_methods
            .iter()
            .chain(&data.virtual_methods)
            .find(|method| method.method_idx == *index)
            .ok_or("enum: absorbed original definition unavailable")?;
        let ssa = lift(original)?;
        let key = method_key(r.dex, *index).ok_or("enum: absorbed identity unavailable")?;
        let caller = r
            .method(MethodRef::new(*index))
            .ok_or("enum: absorbed declaration unavailable")?;
        // Exact generated-method bodies have already been matched to javac's
        // enum operations. Array.clone/valueOf references in those absorbed
        // bodies are not arbitrary user calls or literal source-field uses.
        let mut dependencies = if compiler_methods.contains(index) {
            BTreeSet::new()
        } else {
            body_dependencies(ssa.ir(), r, &caller.owner)?
        };
        if constructor_indices.contains(index) {
            dependencies.retain(|key| {
                !(key.owner == "Ljava/lang/Enum;"
                    && key.name == "<init>"
                    && key.parameters == ["Ljava/lang/String;", "I"]
                    && key.return_type == "V")
            });
        }
        let uses = if compiler_methods.contains(index) {
            Vec::new()
        } else {
            body_field_uses(ssa.ir(), r.dex)
                .into_iter()
                .filter(|(key, _, _)| key != &backing_key)
                .collect()
        };
        publications.insert(
            key.clone(),
            EmittedMethod {
                source: source.clone(),
                dependencies,
            },
        );
        field_uses.insert(key, uses);
    }
    if let Some(registry) = r.annotation_defaults {
        let owner = r
            .class(ClassRef::new(class.class_idx))
            .ok_or("enum source owner unavailable")?;
        registry
            .lock()
            .map_err(|_| "enum default preparation lock")?
            .prepare(r.dex, class, &init, &owner, &source)
            .map_err(|e| format!("enum default preparation: {e:?}"))?;
    }
    Ok(EnumSource {
        source,
        fields: fields.len() + init.constants.len(),
        methods: ordinary_bodies + 2 + usize::from(init.absorbed_values_helper.is_some()),
        constructors: constructor_indices.len(),
        declarations,
        publications,
        field_uses,
        published_fields: fields
            .keys()
            .copied()
            .chain(init.constants.iter().map(|constant| constant.field_idx))
            .map(|index| {
                field_key(r.dex, index).ok_or("enum: published field identity unavailable")
            })
            .collect::<Result<_, _>>()?,
    })
}

fn emit_classes(
    dex: &DexFile,
    r: &Resolver<'_>,
    classes: &[ClassDef],
    options: &JavaOptions,
    sink: &mut impl SourceSink,
    profile: &mut Option<ClassProfile>,
) -> Result<JavaReport, OutputError> {
    let mut report = JavaReport::default();
    for class in classes {
        if r.defer_annotation_defaults
            && class_kind(class.access_flags) == Ok(ClassKind::Annotation)
            && initializers::annotation_publication::requires_deferred_defaults(dex, class)
        {
            continue;
        }
        report.classes_seen += 1;
        let descriptor = descriptor(dex, class.class_idx)
            .ok_or_else(|| OutputError::InvalidDescriptor("class pool".into()))?;
        if options
            .class_filter
            .as_ref()
            .is_some_and(|f| !descriptor.contains(f))
            || options.limit.is_some_and(|l| report.classes_emitted >= l)
        {
            report.skipped_classes += 1;
            continue;
        }
        let kind = class_kind(class.access_flags);
        let mut enum_failure = None;
        if kind == Ok(ClassKind::Enum)
            && let Ok(enum_unit) = enum_source(class, r, profile).inspect_err(|e| {
                enum_failure = Some(e.clone());
            })
        {
            let owner = r
                .class(ClassRef::new(class.class_idx))
                .ok_or_else(|| OutputError::InvalidDescriptor(descriptor.into()))?;
            let original_owner = descriptor
                .trim_start_matches('L')
                .trim_end_matches(';')
                .replace('/', ".");
            if owner == original_owner {
                let package = owner.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
                let text = format!(
                    "// Proven enum initialization and synthetic method equivalence.\n{}{}",
                    if package.is_empty() {
                        String::new()
                    } else {
                        format!("package {package};\n\n")
                    },
                    enum_unit.source
                );
                let source = GeneratedSource::with_source_name(
                    SourceProvenance {
                        descriptor: descriptor.into(),
                        origin: None,
                        status: SourceStatus::Fallback,
                    },
                    &owner,
                    text,
                    Vec::new(),
                )?;
                if let Some(publications) = r.publications {
                    publications
                        .lock()
                        .map_err(|_| {
                            OutputError::InvalidDescriptor("enum publication lock".into())
                        })?
                        .extend(enum_unit.publications);
                }
                if let Some(uses) = r.field_uses {
                    uses.lock()
                        .map_err(|_| {
                            OutputError::InvalidDescriptor("enum field publication lock".into())
                        })?
                        .extend(enum_unit.field_uses);
                }
                if let Some(owners) = r.published_owners {
                    owners
                        .lock()
                        .map_err(|_| {
                            OutputError::InvalidDescriptor("enum owner publication lock".into())
                        })?
                        .insert(descriptor.into(), true);
                }
                if let Some(fields) = r.enum_fields {
                    fields
                        .lock()
                        .map_err(|_| {
                            OutputError::InvalidDescriptor("enum field attestation lock".into())
                        })?
                        .extend(enum_unit.published_fields);
                }
                sink.emit(&source)?;
                report.classes_emitted += 1;
                report.complete_methods += enum_unit.methods;
                report.complete_constructors += enum_unit.constructors;
                report.complete_static_initializers += 1;
                report.declaration_only_methods += enum_unit.declarations;
                report.fields_emitted += enum_unit.fields;
                continue;
            }
        }
        let mut header_class = class.clone();
        if kind == Ok(ClassKind::Annotation)
            && r.annotation_defaults.is_some()
            && initializers::annotation_publication::declared_defaults_with(dex, class, &|index| {
                r.class(ClassRef::new(index))
                    .ok_or(DeclarationUnsupported::InvalidMetadata)
            })
            .is_ok()
        {
            // Defaults are validated separately against every genuine element;
            // orphan metadata remains preserved, never an invented declaration.
            header_class.annotations = None;
        }
        // Only an attested genuinely empty annotation may bypass the ordinary
        // nonempty source-unit gate. Actual original owners and selected
        // platform metadata remain required; no orphan elements are inferred.
        let marker_annotation = (kind == Ok(ClassKind::Annotation))
            .then(|| {
                r.external.and_then(|external| {
                    initializers::marker_annotation_publication::plan_marker_annotation_indexed(
                        dex,
                        class,
                        &r.input_owners,
                        external,
                        &|index| {
                            r.class(ClassRef::new(index))
                                .ok_or(DeclarationUnsupported::InvalidMetadata)
                        },
                    )
                    .ok()
                })
            })
            .flatten();
        let empty_interface = empty_type_publication::plan_empty_interface(dex, class, |index| {
            r.class(ClassRef::new(index))
                .ok_or(DeclarationUnsupported::InvalidMetadata)
        })
        .ok();
        let header = marker_annotation
            .as_ref()
            .map(|plan| Ok(plan.header.clone()))
            .unwrap_or_else(|| {
                class_declaration_with(dex, &header_class, |index| {
                    r.class(ClassRef::new(index))
                        .ok_or(DeclarationUnsupported::InvalidMetadata)
                })
            });
        let mut candidates = BTreeMap::new();
        let mut failures = BTreeMap::new();
        let mut first_body_failures = BTreeMap::new();
        let mut constructor_subset_failures = BTreeMap::new();
        let mut fields = BTreeMap::new();
        let mut unavailable_fields = BTreeMap::new();
        let mut initialization_prefix = String::new();
        let mut initialization_proven = false;
        let mut unresolved_bodies = BTreeMap::new();
        if let Some(data) = &class.class_data {
            for f in data.static_fields.iter().chain(&data.instance_fields) {
                match r.field_declaration(class, f.field_idx) {
                    Ok(source) => {
                        fields.insert(f.field_idx, source);
                    }
                    Err(reason) => {
                        unavailable_fields.insert(f.field_idx, reason);
                    }
                }
            }
            if kind == Ok(ClassKind::Class)
                && let Some(publication) = r.field_catalog
            {
                fields.clear();
                for (key, source) in publication
                    .owner_declarations(descriptor, r.aliases)
                    .map_err(|e| {
                        OutputError::InvalidDescriptor(format!("owner field publication: {e:?}"))
                    })?
                {
                    let location = &publication.declarations[&key];
                    fields.insert(location.field, source);
                }
            }
            for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                let result = match (&header, &kind) {
                    (Ok(_), Ok(kind)) => method_source(class, method, r, *kind, profile),
                    _ => Err(enum_failure.clone().unwrap_or_else(|| {
                        format!("class declaration: {:?}", header.as_ref().err())
                    })),
                };
                match result {
                    Ok(c) => {
                        candidates.insert(method.method_idx, c);
                    }
                    Err(reason) => {
                        first_body_failures.insert(method.method_idx, reason.clone());
                        failures.insert(method.method_idx, reason);
                    }
                }
            }
            let initialization_timer = stage(profile, "initialization_family");
            let mut reuse_static = None;
            let initialization = (kind == Ok(ClassKind::Class))
                .then(|| initialization_source(class, r, &mut reuse_static));
            drop(initialization_timer);
            if let Some(Ok((initialized, initialized_fields, prefix))) = initialization {
                let proposed_fields = initialized_fields.keys().copied().collect();
                let rejected = closure_failures(
                    candidates
                        .iter()
                        .filter(|(index, _)| !initialized.contains_key(index))
                        .map(|(&index, c)| (index, c.body, &c.requirements))
                        .chain(
                            initialized
                                .iter()
                                .map(|(&index, c)| (index, c.body, &c.requirements)),
                        ),
                    &proposed_fields,
                );
                if !initialized.keys().any(|index| rejected.contains_key(index)) {
                    for index in initialized.keys() {
                        failures.remove(index);
                    }
                    candidates.extend(initialized);
                    fields = initialized_fields;
                    initialization_prefix = prefix;
                    initialization_proven = true;
                }
            } else if let Some(Err(reason)) = initialization {
                for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                    if failures.contains_key(&method.method_idx)
                        && method_key(dex, method.method_idx)
                            .is_some_and(|k| matches!(k.name.as_str(), "<init>" | "<clinit>"))
                    {
                        failures.insert(method.method_idx, reason.clone());
                    }
                }
            }
            // A lowered full family can still fail publication closure. Retry
            // the independently attested static body in that case as well as
            // on a planning error; missing instance dependencies do not erase
            // a proved static store sequence. This grants no family proof.
            if !initialization_proven
                && kind == Ok(ClassKind::Class)
                && let Ok((index, candidate, prefix)) =
                    static_initializer_subset_source(class, r, reuse_static.take())
            {
                failures.remove(&index);
                candidates.insert(index, candidate);
                initialization_prefix = prefix;
            }
            // An atomic plan can lower successfully but fail publication
            // closure. Its success is not evidence that missing constructors
            // were published. Retain independently proved reachable bodies in
            // that case too, without granting class-initialization proof.
            if !initialization_proven
                && kind == Ok(ClassKind::Class)
                && data.direct_methods.iter().any(|method| {
                    !candidates.contains_key(&method.method_idx)
                        && method_key(dex, method.method_idx)
                            .is_some_and(|key| key.name == "<init>")
                })
                && let Ok(subset) =
                    constructor_subset_source(class, r, &mut constructor_subset_failures)
            {
                for (index, candidate) in subset {
                    if let std::collections::btree_map::Entry::Vacant(slot) =
                        candidates.entry(index)
                    {
                        failures.remove(&index);
                        slot.insert(candidate);
                    }
                }
                // initialization_proven deliberately remains false.
            }
            let declared_fields = fields.keys().copied().collect();
            let timer = stage(profile, "closure_pruning");
            for (index, reason) in closure_failures(
                candidates
                    .iter()
                    .map(|(&index, c)| (index, c.body, &c.requirements)),
                &declared_fields,
            ) {
                if reason == "required complete method body unavailable"
                    && candidates.get(&index).is_some_and(|c| {
                        c.body
                            && !c.constructor
                            && c.requirements.fields.is_subset(&declared_fields)
                    })
                    && method_key(dex, index).is_some_and(|k| !k.name.starts_with('<'))
                {
                    unresolved_bodies.insert(index, reason.clone());
                } else {
                    candidates.remove(&index);
                }
                failures.insert(index, reason);
            }
            drop(timer);
            for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                if let Some(reason) = failures.get(&method.method_idx) {
                    let name = method_key(dex, method.method_idx)
                        .map(|k| k.name)
                        .unwrap_or_default();
                    if name == "<init>" {
                        report.unavailable_constructors += 1;
                    } else {
                        report.unsupported_methods += 1;
                    }
                    *report.reasons.entry(reason.clone()).or_default() += 1;
                    let category = match name.as_str() {
                        "<init>" => "constructor",
                        "<clinit>" => "static_initializer",
                        _ => "ordinary",
                    };
                    *report
                        .category_reasons
                        .entry(format!("{category}\t{reason}"))
                        .or_default() += 1;
                }
            }
        }
        if candidates.is_empty()
            && (fields.is_empty() || header.is_err())
            && marker_annotation.is_none()
            && empty_interface.is_none()
        {
            if let Some(data) = &class.class_data {
                for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                    if let Some(key) = method_key(dex, method.method_idx) {
                        let identity = format!(
                            "{}->{}({}){}",
                            key.owner,
                            key.name,
                            key.parameters.join(""),
                            key.return_type
                        );
                        let reason = failures.get(&method.method_idx)
                            .or_else(|| first_body_failures.get(&method.method_idx))
                            .or_else(|| constructor_subset_failures.get(&method.method_idx))
                            .cloned().unwrap_or_else(|| enum_failure.clone().unwrap_or_else(|| {
                                header.as_ref().err().map(|error| format!("class declaration: {error:?}"))
                                    .unwrap_or_else(|| "unpublished class: no retained candidates and no publishable fields".into())
                            }));
                        report.unpublished_method_reasons.insert(identity, reason);
                    }
                }
            }
            continue;
        }
        let timer = stage(profile, "source_serialization");
        // Pure original marker interfaces restore declarations only: no body,
        // constructor, initializer or original-method coverage is fabricated.

        let retained_constructors = candidates
            .iter()
            .filter(|(_, c)| c.constructor)
            .map(|(i, _)| *i)
            .collect();
        let retained_clinit = candidates
            .keys()
            .copied()
            .find(|i| method_key(dex, *i).is_some_and(|k| k.name == "<clinit>"));
        let initializer_coverage = initializers::check_class_initializer_coverage(
            dex,
            class,
            &retained_constructors,
            retained_clinit,
        );
        let field_initialization_required = !unavailable_fields.is_empty()
            || class.static_values.as_ref().is_some_and(|v| !v.is_empty())
            || class.class_data.as_ref().is_some_and(|d| {
                d.static_fields
                    .iter()
                    .chain(&d.instance_fields)
                    .any(|f| f.access_flags & 0x10 != 0)
            });
        let initialization_available = initializer_coverage.is_ok()
            && (!field_initialization_required || initialization_proven)
            && !r
                .field_catalog
                .is_some_and(|p| p.limitations.contains_key(descriptor));
        if !initialization_available {
            report.unavailable_initialization_units += 1;
        }
        let owner = r
            .class(ClassRef::new(class.class_idx))
            .ok_or_else(|| OutputError::InvalidDescriptor(descriptor.into()))?;
        let package = owner.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
        let mut text = "// HONEST SUBSET: retained declarations and supported bodies only.\n// Unsupported original members and initialization are unavailable; this is not a complete app.\n// Verified input/classpath references may require compile dependencies absent from this subset.\n// An implicit Java constructor is not a recovered DEX constructor.\n".to_string();
        text.push_str(&format!("// Original initializer body coverage: {}; declaration-only fields do not prove initialization.\n", if initializer_coverage.is_ok() { "retained" } else { "unavailable" }));
        text.push_str(&format!(
            "// Original field initialization proof: {}.\n",
            if initialization_available {
                "retained"
            } else {
                "unavailable"
            }
        ));
        if !package.is_empty() {
            text.push_str(&format!("package {package};\n\n"));
        }
        text.push_str(&format!(
            "{} {{\n",
            header.unwrap_or_else(|_| unreachable!("candidates require a valid class header"))
        ));
        for (index, reason) in &unavailable_fields {
            text.push_str(&format!(
                "    // unavailable original field index {index}: {reason}\n"
            ));
        }
        let required_fields: BTreeSet<_> = fields.keys().copied().collect();
        report.fields_emitted += required_fields.len();
        for index in required_fields {
            text.push_str(&format!("    {}\n", fields[&index]));
        }
        for line in initialization_prefix.lines() {
            text.push_str(&format!("    {line}\n"));
        }
        let emitted_bodies = candidates
            .iter()
            .filter(|(_, c)| c.body)
            .map(|(i, _)| *i)
            .collect();
        let bodyless = candidates
            .iter()
            .filter(|(_, c)| !c.body)
            .map(|(i, _)| *i)
            .collect();
        for (index, candidate) in &candidates {
            if let Some(key) = method_key(dex, *index) {
                if let Some(publications) = r.publications {
                    publications
                        .lock()
                        .map_err(|_| {
                            OutputError::InvalidDescriptor("method publication lock".into())
                        })?
                        .insert(
                            key.clone(),
                            EmittedMethod {
                                source: candidate.source.clone(),
                                dependencies: candidate.dependencies.clone(),
                            },
                        );
                }
                if let Some(uses) = r.field_uses {
                    uses.lock()
                        .map_err(|_| {
                            OutputError::InvalidDescriptor("field publication lock".into())
                        })?
                        .insert(key, candidate.field_uses.clone());
                }
            }
            let dependency_report = if let Some(index) = r.member_index {
                candidate
                    .requirements
                    .method_closure_indexed(index, &emitted_bodies, &bodyless)
            } else {
                candidate.requirements.method_closure(
                    &crate::java_members::MemberResolver { dex },
                    &emitted_bodies,
                    &bodyless,
                )
            };
            if candidate.body
                && !candidate.constructor
                && method_key(dex, *index).is_some_and(|k| !k.name.starts_with('<'))
                && !dependency_report.unmet.is_empty()
            {
                report.semantic_bodies_with_unresolved_dependencies += 1;
            }
            if unresolved_bodies.contains_key(index) {
                report.semantic_bodies_outside_strict_dependency_closure += 1;
            } else if candidate.constructor {
                report.complete_constructors += 1;
            } else if !candidate.body {
                report.declaration_only_methods += 1;
            } else if method_key(dex, *index).is_some_and(|k| k.name == "<clinit>") {
                report.complete_static_initializers += 1;
            } else {
                report.complete_methods += 1;
            }
            text.push_str(
                &format!(
                    "    // {}original method pool index {index}; {:?}; complete_body={}; semantic_complete={}; dependency_complete={}\n",
                    if unresolved_bodies.contains_key(index) { "semantically complete " } else { "" },
                    method_key(dex, *index),
                    candidate.body && !unresolved_bodies.contains_key(index),
                    candidate.body,
                    dependency_report.unmet.is_empty()
                )
                .replace("\\u", "\\ u"),
            );
            for (dependency, reason) in dependency_report.unmet {
                text.push_str(&format!("    // unresolved original dependency index {dependency}; {:?}: {reason}\n", method_key(dex, dependency)).replace("\\u", "\\ u"));
            }
            if let Some(evidence) = method_key(dex, *index).and_then(|key| {
                r.source_throws
                    .and_then(|plan| source_throws::evidence(plan, &key))
            }) && !evidence.derived.is_empty()
                && !candidate
                    .source
                    .contains("// Caller-local checked-source recipe:")
            {
                text.push_str(&format!("    // source-only derived checked throws: {:?}; original annotations unchanged: {:?}; original source identity {:?}\n",
                    evidence.derived, evidence.original, evidence.identity).replace("\\u", "\\ u"));
            }
            for line in candidate.source.lines() {
                text.push_str(&format!("    {line}\n"));
            }
        }
        for (index, reason) in &failures {
            if let Some(subset) = constructor_subset_failures.get(index) {
                text.push_str(
                    &format!(
                        "    // constructor subset rejection index {index}: {:?}: {}\n",
                        method_key(dex, *index),
                        subset.replace(['\r', '\n'], " ")
                    )
                    .replace("\\u", "\\ u"),
                );
            }
            if let Some(first) = first_body_failures.get(index)
                && first != reason
            {
                text.push_str(
                    &format!(
                        "    // first original body rejection index {index}: {:?}: {}\n",
                        method_key(dex, *index),
                        first.replace(['\r', '\n'], " ")
                    )
                    .replace("\\u", "\\ u"),
                );
            }
            text.push_str(
                &format!(
                    "    // unavailable original method index {index}: {:?}: {}\n",
                    method_key(dex, *index),
                    reason.replace(['\r', '\n'], " ")
                )
                .replace("\\u", "\\ u"),
            );
        }
        text.push_str("}\n");
        if let Some(owners) = r.published_owners {
            owners
                .lock()
                .map_err(|_| OutputError::InvalidDescriptor("owner publication lock".into()))?
                .insert(descriptor.into(), initialization_available);
        }
        let mut source = GeneratedSource::with_source_name(SourceProvenance { descriptor: descriptor.into(), origin: None, status: SourceStatus::Fallback }, &owner, text, vec![SourceDiagnostic { severity: DiagnosticSeverity::Warning, message: "Subset class; complete bodies do not imply class/app completeness or dependency closure".into() }])?;
        if let Some(plan) = empty_interface {
            let message = format!(
                "Genuine empty interface declaration only; original descriptor {}; original flags {:#x}; original defined methods {}; no constructors, bodies or initializer identities fabricated.",
                plan.descriptor,
                plan.access_flags,
                plan.original_defined_methods()
            );
            source.diagnostics.push(SourceDiagnostic {
                severity: DiagnosticSeverity::Info,
                message: message.clone(),
            });
            source.text.push_str(&format!("// {message}\n"));
        }
        if let Some(plan) = marker_annotation {
            let message = format!(
                "Genuine zero-member annotation declaration; checked platform annotation metadata {}; {} original entries retained but not emitted. This grants no method/body coverage or enclosing-source/app completeness.",
                if plan.annotation_metadata_restored() {
                    "restored"
                } else {
                    "partial"
                },
                plan.retained_annotations.len(),
            );
            source.diagnostics.push(SourceDiagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "{message} Original raw metadata: {:?}",
                    plan.original_annotations
                ),
            });
            source.text.push_str(&format!("// {message}\n"));
        }
        drop(timer);
        let _timer = stage(profile, "source_collection");
        if kind == Ok(ClassKind::Annotation)
            && let Ok(defaults) =
                initializers::annotation_publication::declared_defaults_with(dex, class, &|index| {
                    r.class(ClassRef::new(index))
                        .ok_or(DeclarationUnsupported::InvalidMetadata)
                })
            && !defaults.retained_orphans.is_empty()
        {
            let message = format!(
                "Declared annotation elements recovered; orphan AnnotationDefault entries retained, never emitted as methods: {:?}; original metadata: {:?}",
                defaults.retained_orphans, defaults.original_annotations
            );
            source.diagnostics.push(SourceDiagnostic {
                severity: DiagnosticSeverity::Warning,
                message: message.clone(),
            });
            source
                .text
                .push_str(&format!("// {}\n", message.replace("\\u", "\\ u")));
        }
        sink.emit(&source)?;
        report.classes_emitted += 1;
    }
    Ok(report)
}

#[cfg(test)]
#[path = "java_checked_source_bridge_tests.rs"]
mod checked_source_bridge_tests;

#[cfg(test)]
#[path = "java_receiver_checked_target_tests.rs"]
mod receiver_checked_target_tests;

#[cfg(test)]
#[path = "java_enum_classification_route_tests.rs"]
mod enum_classification_route_tests;

#[cfg(test)]
#[path = "java_checked_bound_targets_actual_tests.rs"]
mod checked_bound_targets_actual_tests;

#[cfg(test)]
mod deferred_annotation_worker_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    #[ignore = "bounded original Samsung annotation/enum slice and local JDK17"]
    fn deferred_annotation_workers_preserve_sources_reports_and_receipts() {
        let mut files = dex_format::apk::parse_apk_path(
            "/storage_0/decomphify/tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
        )
        .unwrap();
        for dex in &mut files {
            dex.classes = dex
                .classes
                .iter()
                .filter(|class| {
                    matches!(
                        descriptor(dex, class.class_idx),
                        Some(
                            "Lorg/simpleframework/xml/DefaultType;"
                                | "Lorg/simpleframework/xml/Default;"
                                | "Llq/f;"
                        )
                    )
                })
                .cloned()
                .collect();
        }
        let deferred: usize = files
            .iter()
            .map(|dex| {
                dex.classes
                    .iter()
                    .filter(|class| {
                        class_kind(class.access_flags) == Ok(ClassKind::Annotation)
                            && initializers::annotation_publication::requires_deferred_defaults(
                                dex, class,
                            )
                    })
                    .count()
            })
            .sum();
        assert_eq!(
            deferred, 2,
            "the bounded slice must exercise multiple deferred tasks"
        );
        let options = JavaOptions {
            classpath: vec!["/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod".into()],
            ..JavaOptions::default()
        };
        let emit = |workers| {
            let mut sources = Vec::new();
            let report = crate::dex_java::emit_java(
                &files,
                &JavaOptions {
                    workers,
                    ..options.clone()
                },
                &mut |source: &GeneratedSource| {
                    sources.push(source.clone());
                    Ok(())
                },
            )
            .unwrap();
            (report, sources)
        };
        let (expected_report, expected_sources) = emit(1);
        assert_eq!(expected_sources.len(), 3);
        for workers in [2, 4, 32, 4] {
            let (report, sources) = emit(workers);
            assert_eq!(report, expected_report, "workers={workers}");
            assert_eq!(sources, expected_sources, "workers={workers}");
        }
        let enum_position = expected_sources
            .iter()
            .position(|source| {
                source.provenance.descriptor == "Lorg/simpleframework/xml/DefaultType;"
            })
            .unwrap();
        let annotation_position = expected_sources
            .iter()
            .position(|source| source.provenance.descriptor == "Lorg/simpleframework/xml/Default;")
            .unwrap();
        assert!(enum_position < annotation_position);
        for workers in [1, 4] {
            let mut callbacks = Vec::new();
            let result = crate::dex_java::emit_java(
                &files,
                &JavaOptions {
                    workers,
                    ..options.clone()
                },
                &mut |source: &GeneratedSource| {
                    callbacks.push(source.provenance.descriptor.clone());
                    if source.provenance.descriptor == "Lorg/simpleframework/xml/DefaultType;" {
                        return Err(OutputError::Io(std::io::Error::other(
                            "intentional enum receipt denial",
                        )));
                    }
                    Ok(())
                },
            );
            assert!(result.is_err());
            assert!(
                !callbacks
                    .iter()
                    .any(|owner| owner == "Lorg/simpleframework/xml/Default;")
            );
        }
        eprintln!(
            "deferred_annotation parity deferred={deferred} sources={} classes={} methods={} workers=1,2,4,32,4 receipt_denial=1,4",
            expected_sources.len(),
            expected_report.classes_emitted,
            expected_report.complete_methods
        );
    }
}
