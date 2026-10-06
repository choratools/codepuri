//! Declaration initialization only: never move an arbitrary body assignment here.
use super::{
    ClassKind, DeclarationUnsupported as Error, annotation_type, class_kind, class_name,
    descriptor, ty, type_source,
};
use backend_java::JavaType;
use dex_format::{
    DexFile,
    class::ClassDef,
    encoded_value::{EncodedValue, EncodedValueKind as V},
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

#[path = "java_annotation_defaults_publication.rs"]
pub mod annotation_publication;

#[path = "java_marker_annotation_publication.rs"]
pub mod marker_annotation_publication;

#[path = "write_inventory.rs"]
pub mod write_inventory;

// Parsed DEX class definitions are ordered by type index. Positive equality
// still compares the complete original declaration. Unordered/duplicate or
// externally supplied snapshots retain the exhaustive legacy fallback.
fn contains_original_class(classes: &[ClassDef], class: &ClassDef) -> bool {
    classes
        .binary_search_by_key(&class.class_idx, |c| c.class_idx)
        .ok()
        .is_some_and(|index| classes[index] == *class)
        || classes.iter().any(|candidate| candidate == class)
}

#[cfg(test)]
#[path = "java_class_membership_speed_tests.rs"]
mod membership_speed_tests;

/// Source explicit-store legality and Java definite assignment, with retained
/// capabilities that may change fields at runtime. Never an immutability token.
pub struct ExplicitStorePlan<T> {
    /// Existing atomic source plan; original final modifiers remain unchanged.
    pub plan: T,
    /// Genuine native/reflection capabilities excluded only from DEX-store enumeration.
    pub runtime_capabilities: Vec<write_inventory::UnknownObservation>,
}

/// Internal source-store plan retaining one inventory's shared evidence.
pub(crate) struct SharedExplicitStorePlan<T> {
    pub(crate) plan: T,
    pub(crate) runtime_capabilities: Arc<[write_inventory::UnknownObservation]>,
}

fn shared_explicit_plan<T>(
    plan: T,
    inventory: &write_inventory::WriteInventory,
) -> SharedExplicitStorePlan<T> {
    SharedExplicitStorePlan {
        plan,
        runtime_capabilities: inventory.runtime_capabilities_shared(),
    }
}
#[derive(Clone, Copy)]
enum WriteContract {
    RuntimeExclusive,
    ExplicitStores,
}
impl WriteContract {
    fn writes(
        self,
        inventory: &write_inventory::WriteInventory,
        key: &write_inventory::FieldKey,
    ) -> Result<BTreeSet<write_inventory::WriteSite>, Error> {
        match self {
            Self::RuntimeExclusive => inventory
                .proven_writes(key)
                .ok_or(Error::FieldInitializationUnavailable),
            Self::ExplicitStores => inventory
                .explicit_store_writes(key)
                .map_err(|_| Error::FieldInitializationUnavailable),
        }
    }
}
fn require_explicit_completeness(inventory: &write_inventory::WriteInventory) -> Result<(), Error> {
    if inventory.has_structural_unknown() {
        return Err(Error::FieldInitializationUnavailable);
    }
    Ok(())
}
/// Constructor-only original explicit-store proof. Complete original constructor
/// coverage and every store's owner/domain remain mandatory; runtime capabilities
/// are retained separately and this never proves runtime write absence.
pub fn prove_instance_final_explicit_stores(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructors: &BTreeSet<u32>,
    inventory: &write_inventory::WriteInventory,
) -> Result<ExplicitStorePlan<Vec<backend_java::constructors::BlankFinal>>, Error> {
    require_explicit_completeness(inventory)?;
    Ok(explicit_plan(
        prove_instance_final_writes_contract(
            inputs,
            dex_index,
            class,
            constructors,
            inventory,
            WriteContract::ExplicitStores,
        )?,
        inventory,
    ))
}
fn explicit_plan<T>(plan: T, inventory: &write_inventory::WriteInventory) -> ExplicitStorePlan<T> {
    ExplicitStorePlan {
        plan,
        runtime_capabilities: inventory
            .unknown_observations()
            .iter()
            .filter(|o| !matches!(o.kind, write_inventory::UnknownKind::Structural))
            .cloned()
            .collect(),
    }
}
/// Plan original encoded stores under explicit DEX-store completeness, retaining
/// native/reflection capability evidence and refusing all structural uncertainty.
pub fn plan_encoded_initialization_explicit_stores(
    dex: &DexFile,
    class: &ClassDef,
    inventory: &write_inventory::WriteInventory,
) -> Result<ExplicitStorePlan<EncodedInitializationPlan>, Error> {
    Ok(explicit_plan(
        plan_encoded_explicit_stores_only(dex, class, inventory)?,
        inventory,
    ))
}
// Field publication retains the whole inventory itself. Do not allocate another
// copy of all capability observations merely to discard it after extracting plan.
pub(crate) fn plan_encoded_explicit_stores_only(
    dex: &DexFile,
    class: &ClassDef,
    inventory: &write_inventory::WriteInventory,
) -> Result<EncodedInitializationPlan, Error> {
    require_explicit_completeness(inventory)?;
    plan_encoded_initialization_contract(dex, class, inventory, WriteContract::ExplicitStores)
}
/// Atomic complete-family definite-assignment plan. This proves source-store
/// legality, not runtime immutability or absence of native/reflection writes.
pub fn plan_class_initialization_with_explicit_stores<
    R: backend_java::constructors::TrustedConstructorResolver,
>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    clinit: Option<(
        managed_ir::refs::MethodRef,
        &managed_ir::method::MethodIr<managed_ir::method::SsaForm>,
    )>,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
) -> Result<ExplicitStorePlan<ClassInitializationPlan>, Error> {
    Ok(explicit_plan(
        plan_class_initialization_explicit_stores_only(
            inputs,
            dex_index,
            class,
            constructors,
            clinit,
            resolver,
            inventory,
        )?,
        inventory,
    ))
}

/// Same complete-family proof as the public explicit-store API, retaining its
/// capabilities without copying the whole input's evidence for every class.
// Conservative compatibility route for callers without an original SSA witness.
#[allow(dead_code)]
pub(crate) fn plan_class_initialization_shared_explicit_stores<
    R: backend_java::constructors::TrustedConstructorResolver,
>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    clinit: Option<(
        managed_ir::refs::MethodRef,
        &managed_ir::method::MethodIr<managed_ir::method::SsaForm>,
    )>,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
) -> Result<SharedExplicitStorePlan<ClassInitializationPlan>, Error> {
    Ok(shared_explicit_plan(
        plan_class_initialization_explicit_stores_only(
            inputs,
            dex_index,
            class,
            constructors,
            clinit,
            resolver,
            inventory,
        )?,
        inventory,
    ))
}

fn plan_class_initialization_explicit_stores_only<
    R: backend_java::constructors::TrustedConstructorResolver,
>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    clinit: Option<(
        managed_ir::refs::MethodRef,
        &managed_ir::method::MethodIr<managed_ir::method::SsaForm>,
    )>,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
) -> Result<ClassInitializationPlan, Error> {
    require_explicit_completeness(inventory)?;
    plan_class_initialization_with_inventory_contract(
        inputs,
        dex_index,
        class,
        constructors,
        clinit,
        resolver,
        (inventory, WriteContract::ExplicitStores, None, None),
    )
}

/// Complete constructor-family proof with original static SSA provenance.
pub fn plan_class_initialization_with_explicit_stores_attested<
    R: backend_java::constructors::TrustedConstructorResolver,
>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    clinit: Option<(managed_ir::refs::MethodRef, &dex_decompiler::ssa::SsaMethod)>,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
) -> Result<ExplicitStorePlan<ClassInitializationPlan>, Error> {
    Ok(explicit_plan(
        plan_class_initialization_attested_only(
            inputs,
            dex_index,
            class,
            constructors,
            clinit,
            resolver,
            inventory,
            None,
        )?,
        inventory,
    ))
}

pub(crate) fn plan_class_initialization_shared_explicit_stores_attested<
    R: backend_java::constructors::TrustedConstructorResolver,
>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    clinit: Option<(managed_ir::refs::MethodRef, &dex_decompiler::ssa::SsaMethod)>,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
) -> Result<SharedExplicitStorePlan<ClassInitializationPlan>, Error> {
    Ok(shared_explicit_plan(
        plan_class_initialization_attested_only(
            inputs,
            dex_index,
            class,
            constructors,
            clinit,
            resolver,
            inventory,
            None,
        )?,
        inventory,
    ))
}

/// Shared-capability family proof reusing an already-computed attested static
/// plan (or its failure) instead of re-proving identical static evidence.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan_class_initialization_shared_explicit_stores_attested_reusing_static<
    R: backend_java::constructors::TrustedConstructorResolver,
>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    clinit: Option<(managed_ir::refs::MethodRef, &dex_decompiler::ssa::SsaMethod)>,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
    precomputed_static: Result<&StaticInitializerBodySubset, &Error>,
) -> Result<SharedExplicitStorePlan<ClassInitializationPlan>, Error> {
    Ok(shared_explicit_plan(
        plan_class_initialization_attested_only(
            inputs,
            dex_index,
            class,
            constructors,
            clinit,
            resolver,
            inventory,
            Some(precomputed_static),
        )?,
        inventory,
    ))
}

/// Copied-capability family proof reusing an already-computed attested static
/// plan (or its failure) instead of re-proving identical static evidence.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan_class_initialization_with_explicit_stores_attested_reusing_static<
    R: backend_java::constructors::TrustedConstructorResolver,
>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    clinit: Option<(managed_ir::refs::MethodRef, &dex_decompiler::ssa::SsaMethod)>,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
    precomputed_static: Result<&StaticInitializerBodySubset, &Error>,
) -> Result<ExplicitStorePlan<ClassInitializationPlan>, Error> {
    Ok(explicit_plan(
        plan_class_initialization_attested_only(
            inputs,
            dex_index,
            class,
            constructors,
            clinit,
            resolver,
            inventory,
            Some(precomputed_static),
        )?,
        inventory,
    ))
}

#[allow(clippy::too_many_arguments)]
fn plan_class_initialization_attested_only<
    R: backend_java::constructors::TrustedConstructorResolver,
>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    clinit: Option<(managed_ir::refs::MethodRef, &dex_decompiler::ssa::SsaMethod)>,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
    precomputed_static: Option<Result<&StaticInitializerBodySubset, &Error>>,
) -> Result<ClassInitializationPlan, Error> {
    require_explicit_completeness(inventory)?;
    plan_class_initialization_with_inventory_contract(
        inputs,
        dex_index,
        class,
        constructors,
        clinit.map(|(reference, ssa)| (reference, ssa.ir())),
        resolver,
        (
            inventory,
            WriteContract::ExplicitStores,
            clinit.map(|(_, ssa)| ssa),
            precomputed_static,
        ),
    )
}

/// Atomic parent-facing result. Declare fields without initializers, append
/// `encoded.prefix_block`, then `static_initializer`; attach all constructors.
/// Publish only after the entire plan succeeds.
pub struct ClassInitializationPlan {
    /// Encoded prefix and field definite-assignment categories.
    pub encoded: EncodedInitializationPlan,
    /// Complete retained constructor family.
    pub constructors: Vec<backend_java::constructors::ConstructorPlan>,
    /// Original clinit lowered in instruction order, if present.
    pub static_initializer: Option<String>,
    /// Local final-field effects for every retained constructor, in input order. The atomic
    /// plan additionally proves family coverage. Runtime write exclusivity holds
    /// only for the strict entrypoint, never the explicit-store wrapper.
    pub constructor_evidence: Vec<backend_java::constructors::ConstructorInitializationEvidence>,
}

/// Diagnostic evidence, deliberately not a initialized-class publication token.
/// Required declarations can be available even when a family or exclusivity
/// proof is unavailable (for example because the input contains native code).
pub struct ClassConstructorEvidence {
    /// Original field declarations referenced by constructors or needed for instance finals.
    pub required_declarations: BTreeSet<u32>,
    /// One local path analysis per exact retained constructor identity.
    pub constructors: BTreeMap<
        u32,
        Result<
            backend_java::constructors::ConstructorInitializationEvidence,
            backend_java::Unsupported,
        >,
    >,
    /// Every genuine initializer definition is represented, including clinit.
    /// Input coverage only: this does not attest source emission or implicit
    /// constructor recovery.
    pub complete_initializer_coverage: bool,
    /// Exact original constructor identities whose source is missing or cannot
    /// be lowered. A successful local effects analysis is not source recovery.
    /// These reasons may be emitted as optional unavailable comments, never as
    /// invented constructor bodies or a complete-class claim.
    pub unavailable_constructor_sources: BTreeMap<u32, backend_java::Unsupported>,
    /// Complete whole-input write-inventory availability for each instance final.
    /// True alone does not prove constructor-only exclusivity, receiver identity,
    /// or definite assignment.
    pub final_writes_known: BTreeMap<u32, bool>,
}

impl ClassConstructorEvidence {
    /// Optional diagnostic source comments. These are not initializer bodies,
    /// compile-subset membership, or a complete initialization publication token.
    pub fn unavailable_initialization_comments(&self) -> String {
        let mut comments = String::new();
        for (method, reason) in &self.unavailable_constructor_sources {
            comments.push_str(&format!(
                "// unavailable original constructor #{method}: {reason:?}\n"
            ));
        }
        for (field, known) in &self.final_writes_known {
            if !known {
                comments.push_str(&format!(
                    "// unavailable final initialization proof for field #{field}: whole-input writes unknown\n"
                ));
            }
        }
        comments
    }
}

/// Diagnose local initialization independently of global arbitrary-write blockers.
/// Preserve before-chain and catch-path observations even when source lowering
/// fails. Supply the same complete inventory and frozen resolver as the planner.
pub fn class_constructor_evidence<R: backend_java::constructors::TrustedConstructorResolver>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    clinit: Option<u32>,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
) -> Result<ClassConstructorEvidence, Error> {
    use managed_ir::{
        insn::ManagedInsn,
        refs::{ClassRef, FieldRef},
    };
    let dex = inputs.get(dex_index).ok_or(Error::InvalidMetadata)?;
    if !contains_original_class(&dex.classes, class) {
        return Err(Error::InvalidMetadata);
    }
    let parent = resolver
        .class(ClassRef::new(class.superclass_idx))
        .ok_or(Error::InvalidSuperclass)?;
    let data = class.class_data.as_ref().ok_or(Error::InvalidMetadata)?;
    let fields = data
        .instance_fields
        .iter()
        .map(|f| FieldRef::new(f.field_idx))
        .collect::<Vec<_>>();
    let mut report = ClassConstructorEvidence {
        required_declarations: data
            .instance_fields
            .iter()
            .filter(|f| f.access_flags & 0x10 != 0)
            .map(|f| f.field_idx)
            .collect(),
        constructors: BTreeMap::new(),
        complete_initializer_coverage: false,
        unavailable_constructor_sources: BTreeMap::new(),
        final_writes_known: BTreeMap::new(),
    };
    let declared_fields = data
        .instance_fields
        .iter()
        .chain(&data.static_fields)
        .map(|f| f.field_idx)
        .collect::<BTreeSet<_>>();
    for input in constructors {
        if !data.direct_methods.iter().any(|m| {
            m.method_idx == input.reference.index()
                && dex
                    .method_id(m.method_idx)
                    .ok()
                    .is_some_and(|m| dex.string(m.name_idx).ok() == Some("<init>"))
        }) {
            return Err(Error::InvalidMetadata);
        }
        for insn in input.ir.instructions().values() {
            if let ManagedInsn::FieldGet { field, .. } | ManagedInsn::FieldPut { field, .. } = insn
                && declared_fields.contains(&field.index())
            {
                report.required_declarations.insert(field.index());
            }
        }
        if report
            .constructors
            .insert(
                input.reference.index(),
                backend_java::constructors::constructor_initialization_evidence(
                    input, &parent, &fields, resolver,
                ),
            )
            .is_some()
        {
            return Err(Error::InvalidMetadata);
        }
        if let Err(reason) = backend_java::constructors::plan_constructor_declaration(
            input.ir,
            input.reference,
            &parent,
            &input.bindings.declaration_bindings(),
            resolver,
        ) {
            report
                .unavailable_constructor_sources
                .insert(input.reference.index(), reason);
        }
    }
    for method in &data.direct_methods {
        let raw = dex
            .method_id(method.method_idx)
            .map_err(|_| Error::InvalidMetadata)?;
        if dex
            .string(raw.name_idx)
            .map_err(|_| Error::InvalidMetadata)?
            == "<init>"
            && !report.constructors.contains_key(&method.method_idx)
        {
            report.unavailable_constructor_sources.insert(method.method_idx,
                backend_java::Unsupported::Metadata("original constructor body not retained; implicit constructor recovery unavailable".into()));
        }
    }
    report.complete_initializer_coverage = check_class_initializer_coverage(
        dex,
        class,
        &report.constructors.keys().copied().collect(),
        clinit,
    )
    .is_ok();
    for field in data
        .instance_fields
        .iter()
        .filter(|f| f.access_flags & 0x10 != 0)
    {
        let key = write_inventory::field_key(dex, field.field_idx).ok_or(Error::InvalidMetadata)?;
        report
            .final_writes_known
            .insert(field.field_idx, inventory.proven_writes(&key).is_some());
    }
    Ok(report)
}

/// A genuine constructor-family body subset, not a class-initialization proof.
/// Static fields and their original initialization remain independently unavailable.
pub struct ConstructorBodySubset {
    /// Exact input constructor references in caller-provided order.
    pub references: Vec<managed_ir::refs::MethodRef>,
    /// Validated original constructor sources, with original visibility and arity.
    pub constructors: Vec<backend_java::constructors::ConstructorPlan>,
    /// Reachable per-path final assignments, not whole-input initialization proof.
    pub constructor_evidence: Vec<backend_java::constructors::ConstructorInitializationEvidence>,
}

/// Recover a complete original constructor family independently of unrelated
/// static-final exclusivity. All instance finals require exact path-sensitive
/// assignment and complete this/super-family proof. All original static/final
/// declarations must be retained by the caller,
/// and the containing source unit must remain initialization-unavailable unless
/// the separate atomic class plan succeeds. Native/reflection inventory evidence
/// is neither inspected nor erased by this body-only operation.
pub fn plan_constructor_body_subset<R: backend_java::constructors::TrustedConstructorResolver>(
    dex: &DexFile,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    resolver: &R,
) -> Result<ConstructorBodySubset, Error> {
    plan_constructor_subset_impl(dex, class, constructors, resolver, true, None)
}

/// Recover independently proved constructors and their actual emitted this chains.
/// Unavailable unrelated overloads remain in the original declaration catalog.
/// This is local body evidence, never a complete family or initialization token.
pub fn plan_constructor_body_reachable_subset<
    R: backend_java::constructors::TrustedConstructorResolver,
>(
    dex: &DexFile,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    resolver: &R,
) -> Result<ConstructorBodySubset, Error> {
    plan_constructor_subset_impl(dex, class, constructors, resolver, false, None)
}

/// Preserve constructor-local rejection evidence even if no body can be retained.
/// Diagnostics do not grant body, family, or initialization proof.
pub fn plan_constructor_body_reachable_subset_diagnostics<
    R: backend_java::constructors::TrustedConstructorResolver,
>(
    dex: &DexFile,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    resolver: &R,
    diagnostics: &mut BTreeMap<u32, String>,
) -> Result<ConstructorBodySubset, Error> {
    plan_constructor_subset_impl(dex, class, constructors, resolver, false, Some(diagnostics))
}

fn plan_constructor_subset_impl<R: backend_java::constructors::TrustedConstructorResolver>(
    dex: &DexFile,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    resolver: &R,
    require_complete: bool,
    mut diagnostics: Option<&mut BTreeMap<u32, String>>,
) -> Result<ConstructorBodySubset, Error> {
    use managed_ir::refs::ClassRef;
    if !contains_original_class(&dex.classes, class)
        || class.access_flags & (0x200 | 0x2000 | 0x4000) != 0
    {
        return Err(Error::InvalidMetadata);
    }
    let data = class.class_data.as_ref().ok_or(Error::InvalidMetadata)?;
    let mut declared = BTreeMap::new();
    for (methods, direct) in [(&data.direct_methods, true), (&data.virtual_methods, false)] {
        for method in methods {
            let raw = dex
                .method_id(method.method_idx)
                .map_err(|_| Error::InvalidMetadata)?;
            let name = dex
                .string(raw.name_idx)
                .map_err(|_| Error::InvalidMetadata)?;
            if name != "<init>" {
                continue;
            }
            if !direct
                || u32::from(raw.class_idx) != class.class_idx
                || method.access_flags & (8 | 0x100 | 0x400) != 0
                || method.code.is_none()
                || method.code_off == 0
                || declared.insert(method.method_idx, method).is_some()
            {
                return Err(Error::InvalidMetadata);
            }
        }
    }
    let retained = constructors
        .iter()
        .map(|c| c.reference.index())
        .collect::<BTreeSet<_>>();
    if constructors.is_empty()
        || retained.len() != constructors.len()
        || (require_complete && retained != declared.keys().copied().collect())
        || retained.iter().any(|r| !declared.contains_key(r))
    {
        return Err(Error::FieldInitializationUnavailable);
    }
    let owner = resolver
        .class(ClassRef::new(class.class_idx))
        .ok_or(Error::InvalidMetadata)?;
    let parent = resolver
        .class(ClassRef::new(class.superclass_idx))
        .ok_or(Error::InvalidSuperclass)?;
    if resolver.constructor_class(&owner).is_none_or(|c| {
        c.name != owner
            || c.superclass.as_deref() != Some(parent.as_str())
            || c.access_flags != class.access_flags
    }) {
        return Err(Error::InvalidSuperclass);
    }
    for input in constructors {
        let original = declared
            .get(&input.reference.index())
            .ok_or(Error::InvalidMetadata)?;
        let raw = dex
            .method_id(input.reference.index())
            .map_err(|_| Error::InvalidMetadata)?;
        let proto = dex
            .proto_id(u32::from(raw.proto_idx))
            .map_err(|_| Error::InvalidMetadata)?;
        let initializer = resolver
            .initializer(input.reference)
            .ok_or(Error::InvalidMetadata)?;
        let expected = proto
            .parameters
            .as_ref()
            .into_iter()
            .flat_map(|p| &p.types)
            .map(|i| {
                source_type(dex, u32::from(*i), &|index| {
                    resolver
                        .class(ClassRef::new(index))
                        .ok_or(Error::InvalidMetadata)
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if descriptor(dex, proto.return_type_idx)? != "V"
            || initializer.method.owner != owner
            || initializer.method.name != "<init>"
            || initializer.method.is_static
            || initializer.method.return_type != JavaType::Void
            || initializer
                .method
                .parameters
                .iter()
                .map(type_source)
                .collect::<Vec<_>>()
                != expected
            || resolver.constructor_access(input.reference) != Some(original.access_flags)
        {
            return Err(Error::InvalidMetadata);
        }
    }
    let mut final_refs = Vec::new();
    let mut unique_fields = BTreeSet::new();
    for field in data
        .instance_fields
        .iter()
        .filter(|f| f.access_flags & 0x10 != 0)
    {
        let raw = dex
            .field_id(field.field_idx)
            .map_err(|_| Error::InvalidMetadata)?;
        if !unique_fields.insert(field.field_idx)
            || u32::from(raw.class_idx) != class.class_idx
            || field.access_flags & (8 | 0x40) != 0
            || field.access_flags & !(1 | 2 | 4 | 0x10 | 0x80 | 0x1000) != 0
            || (field.access_flags & 7).count_ones() > 1
        {
            return Err(Error::InvalidMetadata);
        }
        let reference = managed_ir::refs::FieldRef::new(field.field_idx);
        let mapped = resolver.field(reference).ok_or(Error::InvalidMetadata)?;
        let expected = source_type(dex, u32::from(raw.type_idx), &|index| {
            resolver
                .class(ClassRef::new(index))
                .ok_or(Error::InvalidMetadata)
        })?;
        if mapped.owner != owner || mapped.is_static || type_source(&mapped.ty) != expected {
            return Err(Error::InvalidMetadata);
        }
        final_refs.push(reference);
    }
    let mut available = Vec::new();
    let mut evidence = Vec::new();
    for input in constructors {
        match backend_java::constructors::constructor_initialization_evidence(
            input,
            &parent,
            &final_refs,
            resolver,
        ) {
            Ok(proof) => {
                available.push(backend_java::constructors::ConstructorInput {
                    reference: input.reference,
                    ir: input.ir,
                    bindings: input.bindings,
                });
                evidence.push(proof);
            }
            Err(_) if require_complete => return Err(Error::FieldInitializationUnavailable),
            Err(error) => {
                if let Some(output) = diagnostics.as_deref_mut() {
                    output
                        .entry(input.reference.index())
                        .or_insert_with(|| format!("constructor local evidence: {error:?}"));
                }
            }
        }
    }
    if available.is_empty() {
        return Err(Error::FieldInitializationUnavailable);
    }
    let constructors = available.as_slice();
    let finals = final_refs
        .iter()
        .enumerate()
        .map(|(field_index, &reference)| {
            let writes = constructors
                .iter()
                .zip(&evidence)
                .map(|(input, effects)| {
                    let effect = &effects.fields[field_index];
                    // The family verifier independently checks normal-return counts,
                    // receiver identity, pre-chain writes and actual this-chain totals.
                    (
                        input.reference,
                        effect.before_chain.union(&effect.after_chain).count(),
                    )
                })
                .collect();
            backend_java::constructors::BlankFinal { reference, writes }
        })
        .collect::<Vec<_>>();
    let (references, plans) = if require_complete {
        let plans = backend_java::constructors::plan_constructor_family(
            &owner,
            constructors,
            &finals,
            resolver,
        )
        .map_err(|_| Error::FieldInitializationUnavailable)?;
        (
            constructors.iter().map(|c| c.reference).collect::<Vec<_>>(),
            plans,
        )
    } else {
        let report = backend_java::constructors::plan_constructor_reachable_subset_detailed(
            &owner,
            constructors,
            &finals,
            resolver,
        )
        .map_err(|_| Error::FieldInitializationUnavailable)?;
        if let Some(output) = diagnostics {
            for rejected in &report.rejected {
                output.entry(rejected.reference.index()).or_insert_with(|| {
                    format!("constructor reachable-chain rejection: {rejected:?}")
                });
            }
        }
        let reachable = report.retained;
        (
            reachable.iter().map(|c| c.reference).collect::<Vec<_>>(),
            reachable.into_iter().map(|c| c.plan).collect(),
        )
    };
    let retained = references.iter().copied().collect::<HashSet<_>>();
    let constructor_evidence = constructors
        .iter()
        .zip(evidence)
        .filter_map(|(input, proof)| retained.contains(&input.reference).then_some(proof))
        .collect();
    Ok(ConstructorBodySubset {
        references,
        constructors: plans,
        constructor_evidence,
    })
}

/// Actual static-initializer body plus preceding original encoded assignments.
/// This is local body evidence, never whole-class initialization certification.
#[derive(Clone)]
pub struct StaticInitializerBodySubset {
    /// Encoded assignments and definite-assignment categories; fields retain finals.
    pub encoded: EncodedInitializationPlan,
    /// Original clinit source, not a substitute encoded-only initializer.
    pub source: String,
}

/// Preserve an exact original clinit independently of global arbitrary effects.
/// Known conflicting static-final writers reject; unknown effects are not erased.
/// Encoded writes stay in a prefix block, never Java constant-variable initializers.
pub fn plan_static_initializer_body_subset<R: backend_java::JavaResolver>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    reference: managed_ir::refs::MethodRef,
    ir: &managed_ir::method::MethodIr<managed_ir::method::SsaForm>,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
) -> Result<StaticInitializerBodySubset, Error> {
    plan_static_initializer_body_subset_contract(
        inputs,
        dex_index,
        class,
        reference,
        ir,
        resolver,
        (inventory, false, None, None),
    )
}
/// Independently prove original static initialization and blank-static-final
/// definite assignment. Instance constructor completeness remains unavailable;
/// runtime capabilities are retained and never interpreted as immutability.
pub fn plan_static_initializer_explicit_stores<R: backend_java::JavaResolver>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    reference: managed_ir::refs::MethodRef,
    ir: &managed_ir::method::MethodIr<managed_ir::method::SsaForm>,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
) -> Result<ExplicitStorePlan<StaticInitializerBodySubset>, Error> {
    Ok(explicit_plan(
        plan_static_initializer_explicit_stores_only(
            inputs, dex_index, class, reference, ir, resolver, inventory,
        )?,
        inventory,
    ))
}

/// Same local static-initialization proof as the public explicit-store API.
// Conservative compatibility route for callers without an original SSA witness.
#[allow(dead_code)]
pub(crate) fn plan_static_initializer_shared_explicit_stores<R: backend_java::JavaResolver>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    reference: managed_ir::refs::MethodRef,
    ir: &managed_ir::method::MethodIr<managed_ir::method::SsaForm>,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
) -> Result<SharedExplicitStorePlan<StaticInitializerBodySubset>, Error> {
    Ok(shared_explicit_plan(
        plan_static_initializer_explicit_stores_only(
            inputs, dex_index, class, reference, ir, resolver, inventory,
        )?,
        inventory,
    ))
}

/// Original SSA provenance is mandatory for encoded VM defaults and multiple
/// mutually exclusive stores. The SSA object owns both the IR and its offsets;
/// callers cannot attach offsets from a different IR to this proof.
pub fn plan_static_initializer_explicit_stores_attested<R: backend_java::JavaResolver>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    reference: managed_ir::refs::MethodRef,
    ssa: &dex_decompiler::ssa::SsaMethod,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
) -> Result<ExplicitStorePlan<StaticInitializerBodySubset>, Error> {
    Ok(explicit_plan(
        plan_static_initializer_attested_only(
            inputs, dex_index, class, reference, ssa, resolver, inventory,
        )?,
        inventory,
    ))
}

/// Original structured exception scopes are derived from the defining code and
/// the supplied frozen declaration hierarchy. Constructor coverage is separate.
#[allow(clippy::too_many_arguments)]
pub fn plan_static_initializer_explicit_stores_with_exceptions_attested<
    R: backend_java::JavaResolver,
>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    reference: managed_ir::refs::MethodRef,
    ssa: &dex_decompiler::ssa::SsaMethod,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
    hierarchy: &backend_java::type_compatibility::Hierarchy,
) -> Result<ExplicitStorePlan<StaticInitializerBodySubset>, Error> {
    Ok(explicit_plan(
        plan_static_initializer_exception_attested_only(
            inputs, dex_index, class, reference, ssa, resolver, inventory, hierarchy,
        )?,
        inventory,
    ))
}

/// Shared-capability equivalent of the public static exception proof.
// Compatibility API is consumed by parent-owned facade wiring.
#[allow(dead_code, clippy::too_many_arguments)]
pub(crate) fn plan_static_initializer_shared_explicit_stores_with_exceptions_attested<
    R: backend_java::JavaResolver,
>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    reference: managed_ir::refs::MethodRef,
    ssa: &dex_decompiler::ssa::SsaMethod,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
    hierarchy: &backend_java::type_compatibility::Hierarchy,
) -> Result<SharedExplicitStorePlan<StaticInitializerBodySubset>, Error> {
    Ok(shared_explicit_plan(
        plan_static_initializer_exception_attested_only(
            inputs, dex_index, class, reference, ssa, resolver, inventory, hierarchy,
        )?,
        inventory,
    ))
}

#[allow(clippy::too_many_arguments)]
fn plan_static_initializer_exception_attested_only<R: backend_java::JavaResolver>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    reference: managed_ir::refs::MethodRef,
    ssa: &dex_decompiler::ssa::SsaMethod,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
    hierarchy: &backend_java::type_compatibility::Hierarchy,
) -> Result<StaticInitializerBodySubset, Error> {
    require_explicit_completeness(inventory)?;
    let dex = inputs.get(dex_index).ok_or(Error::InvalidMetadata)?;
    if !contains_original_class(&dex.classes, class) {
        return Err(Error::InvalidMetadata);
    }
    let original = class
        .class_data
        .as_ref()
        .ok_or(Error::InvalidMetadata)?
        .direct_methods
        .iter()
        .find(|m| m.method_idx == reference.index())
        .ok_or(Error::InvalidMetadata)?;
    let metadata = crate::java_exception_regions::from_dex_disjoint(
        ssa,
        original.code.as_ref().ok_or(Error::InvalidMetadata)?,
        resolver,
        hierarchy,
    )
    .map_err(|_| Error::FieldInitializationUnavailable)?;
    let scopes = metadata
        .scopes
        .iter()
        .map(|m| m as &dyn backend_java::exceptions::RegionMetadata)
        .collect::<Vec<_>>();
    plan_static_initializer_body_subset_contract(
        inputs,
        dex_index,
        class,
        reference,
        ssa.ir(),
        resolver,
        (inventory, true, Some(ssa), Some(&scopes)),
    )
}

/// Production equivalent of the public attested proof, retaining shared
/// capability evidence rather than copying it for every emitted method.
pub(crate) fn plan_static_initializer_shared_explicit_stores_attested<
    R: backend_java::JavaResolver,
>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    reference: managed_ir::refs::MethodRef,
    ssa: &dex_decompiler::ssa::SsaMethod,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
) -> Result<SharedExplicitStorePlan<StaticInitializerBodySubset>, Error> {
    Ok(shared_explicit_plan(
        plan_static_initializer_attested_only(
            inputs, dex_index, class, reference, ssa, resolver, inventory,
        )?,
        inventory,
    ))
}

fn plan_static_initializer_attested_only<R: backend_java::JavaResolver>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    reference: managed_ir::refs::MethodRef,
    ssa: &dex_decompiler::ssa::SsaMethod,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
) -> Result<StaticInitializerBodySubset, Error> {
    require_explicit_completeness(inventory)?;
    plan_static_initializer_body_subset_contract(
        inputs,
        dex_index,
        class,
        reference,
        ssa.ir(),
        resolver,
        (inventory, true, Some(ssa), None),
    )
}

fn plan_static_initializer_explicit_stores_only<R: backend_java::JavaResolver>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    reference: managed_ir::refs::MethodRef,
    ir: &managed_ir::method::MethodIr<managed_ir::method::SsaForm>,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
) -> Result<StaticInitializerBodySubset, Error> {
    require_explicit_completeness(inventory)?;
    plan_static_initializer_body_subset_contract(
        inputs,
        dex_index,
        class,
        reference,
        ir,
        resolver,
        (inventory, true, None, None),
    )
}
fn plan_static_initializer_body_subset_contract<R: backend_java::JavaResolver>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    reference: managed_ir::refs::MethodRef,
    ir: &managed_ir::method::MethodIr<managed_ir::method::SsaForm>,
    resolver: &R,
    write_evidence: (
        &write_inventory::WriteInventory,
        bool,
        Option<&dex_decompiler::ssa::SsaMethod>,
        Option<&[&dyn backend_java::exceptions::RegionMetadata]>,
    ),
) -> Result<StaticInitializerBodySubset, Error> {
    let (inventory, explicit_stores, attested, exception_metadata) = write_evidence;
    use managed_ir::{
        insn::ManagedInsn,
        refs::{ClassRef, FieldRef},
    };
    let dex = inputs.get(dex_index).ok_or(Error::InvalidMetadata)?;
    if !contains_original_class(&dex.classes, class)
        || (class.static_values_off != 0 && class.static_values.is_none())
        || class_kind(class.access_flags)? != ClassKind::Class
    {
        return Err(Error::InvalidMetadata);
    }
    let data = class.class_data.as_ref().ok_or(Error::InvalidMetadata)?;
    let declared = data
        .direct_methods
        .iter()
        .chain(&data.virtual_methods)
        .filter(|m| {
            dex.method_id(m.method_idx)
                .ok()
                .is_some_and(|r| dex.string(r.name_idx).ok() == Some("<clinit>"))
        })
        .collect::<Vec<_>>();
    if declared.len() != 1
        || data
            .virtual_methods
            .iter()
            .any(|m| m.method_idx == reference.index())
        || declared[0].method_idx != reference.index()
        || declared[0].code.is_none()
        || declared[0].code_off == 0
        || declared[0].access_flags & !(8 | 0x1000 | 0x10000) != 0
        || declared[0].access_flags & 8 == 0
    {
        return Err(Error::InvalidMetadata);
    }
    let method = resolver.method(reference).ok_or(Error::InvalidMetadata)?;
    let owner = resolver
        .class(ClassRef::new(class.class_idx))
        .ok_or(Error::InvalidMetadata)?;
    let raw = dex
        .method_id(reference.index())
        .map_err(|_| Error::InvalidMetadata)?;
    let proto = dex
        .proto_id(u32::from(raw.proto_idx))
        .map_err(|_| Error::InvalidMetadata)?;
    if u32::from(raw.class_idx) != class.class_idx
        || method.owner != owner
        || !method.is_static
        || method.name != "<clinit>"
        || method.return_type != JavaType::Void
        || !method.parameters.is_empty()
        || descriptor(dex, proto.return_type_idx)? != "V"
        || proto
            .parameters
            .as_ref()
            .is_some_and(|p| !p.types.is_empty())
    {
        return Err(Error::InvalidMetadata);
    }
    let mut written = HashSet::new();
    for insn in ir.instructions().values() {
        if let ManagedInsn::FieldPut {
            object: None,
            field,
            ..
        } = insn
        {
            written.insert(field.index());
        }
    }
    let mut copy = class.clone();
    let copied = copy.class_data.as_mut().ok_or(Error::InvalidMetadata)?;
    for f in &mut copied.instance_fields {
        f.access_flags &= !0x10;
    }
    let mut plan = EncodedInitializationPlan {
        assignments: vec![],
        prefix_block: String::new(),
        blank_static_finals: BTreeSet::new(),
        blank_instance_finals: BTreeSet::new(),
    };
    let values = class.static_values.as_deref().unwrap_or_default();
    let mut writable = vec![];
    for (position, f) in copied.static_fields.iter_mut().enumerate() {
        let original = dex
            .field_id(f.field_idx)
            .map_err(|_| Error::InvalidMetadata)?;
        if u32::from(original.class_idx) != class.class_idx
            || f.access_flags & 8 == 0
            || f.access_flags & !(1 | 2 | 4 | 8 | 0x10 | 0x40 | 0x80 | 0x1000) != 0
            || (f.access_flags & 7).count_ones() > 1
            || f.access_flags & 0x50 == 0x50
        {
            return Err(Error::InvalidMetadata);
        }
        let mapped = resolver
            .field(FieldRef::new(f.field_idx))
            .ok_or(Error::InvalidMetadata)?;
        let expected = source_type(dex, u32::from(original.type_idx), &|index| {
            resolver
                .class(ClassRef::new(index))
                .ok_or(Error::InvalidMetadata)
        })?;
        if mapped.owner != owner
            || !mapped.is_static
            || mapped.ty == JavaType::Void
            || type_source(&mapped.ty) != expected
        {
            return Err(Error::InvalidMetadata);
        }
        if f.access_flags & 0x10 != 0 {
            let key = write_inventory::field_key(dex, f.field_idx).ok_or(Error::InvalidMetadata)?;
            if explicit_stores {
                let sites = inventory
                    .explicit_store_writes(&key)
                    .map_err(|_| Error::FieldInitializationUnavailable)?;
                let retained=ir.instructions().values().filter(|i|matches!(i,ManagedInsn::FieldPut{object:None,field,..} if field.index()==f.field_idx)).count();
                if sites.len() != retained
                    || (attested.is_none() && sites.len() > 1)
                    || sites.iter().any(|s| {
                        s.dex != dex_index || s.method != reference.index() || !s.is_static
                    })
                {
                    return Err(Error::FieldInitializationUnavailable);
                }
                if let Some(ssa) = attested {
                    prove_attested_static_store_sites(
                        dex_index,
                        reference.index(),
                        f.field_idx,
                        ssa,
                        &sites,
                    )?;
                }
            }
            if inventory.known_writes(&key).is_some_and(|sites| {
                sites
                    .iter()
                    .any(|s| s.dex != dex_index || s.method != reference.index() || !s.is_static)
            }) {
                return Err(Error::FieldInitializationUnavailable);
            }
            if written.contains(&f.field_idx) {
                if let Some(value) = values.get(position)
                    && (attested.is_none() || !encoded_vm_default(&value.value, &mapped.ty))
                {
                    return Err(Error::FieldInitializationUnavailable);
                }
                plan.blank_static_finals.insert(f.field_idx);
            }
            f.access_flags &= !0x10;
        } else {
            writable.push(mapped);
        }
    }
    // Genuine public foreign non-final stores are legal class-qualified writes.
    // The declaration catalogue, not the operand pool alone, attests permission.
    for index in &written {
        let key = write_inventory::field_key(dex, *index).ok_or(Error::InvalidMetadata)?;
        let declaration = inventory
            .field_declaration(&key)
            .ok_or(Error::InvalidMetadata)?;
        let owner_dex = inputs.get(declaration.dex).ok_or(Error::InvalidMetadata)?;
        let raw = owner_dex
            .field_id(declaration.field)
            .map_err(|_| Error::InvalidMetadata)?;
        if u32::from(raw.class_idx) == class.class_idx && declaration.dex == dex_index {
            continue;
        }
        let owner_class = owner_dex
            .classes
            .iter()
            .find(|candidate| candidate.class_idx == u32::from(raw.class_idx))
            .ok_or(Error::InvalidMetadata)?;
        let flags = declaration.access_flags;
        if flags & (1 | 8) != (1 | 8)
            || flags & (2 | 4 | 0x10) != 0
            || owner_class.access_flags & 1 == 0
            || resolver.field_access(FieldRef::new(*index)) != Some(flags)
        {
            return Err(Error::FieldInitializationUnavailable);
        }
        let mapped = resolver
            .field(FieldRef::new(*index))
            .ok_or(Error::InvalidMetadata)?;
        let operand = dex.field_id(*index).map_err(|_| Error::InvalidMetadata)?;
        let expected_owner = resolver
            .class(ClassRef::new(u32::from(operand.class_idx)))
            .ok_or(Error::InvalidMetadata)?;
        if !mapped.is_static || mapped.owner != expected_owner || mapped.ty == JavaType::Void {
            return Err(Error::InvalidMetadata);
        }
        writable.push(mapped);
    }
    if let Some(ssa) = attested
        && !plan.blank_static_finals.is_empty()
    {
        prove_original_static_final_paths(
            dex,
            declared[0],
            &plan.blank_static_finals,
            ssa,
            exception_metadata.is_some(),
        )?;
    }
    plan.assignments = encoded_field_initializations(dex, &copy, &written)?;
    plan.assignments
        .retain(|f| !plan.blank_static_finals.contains(&f.field_idx));
    if !plan.assignments.is_empty() {
        plan.prefix_block.push_str("static {\n");
        for f in &plan.assignments {
            let mapped = resolver
                .field(FieldRef::new(f.field_idx))
                .ok_or(Error::InvalidMetadata)?;
            identifier(&mapped.name)?;
            plan.prefix_block
                .push_str(&format!("    {} = {};\n", mapped.name, f.expression));
        }
        plan.prefix_block.push_str("}\n");
    }
    let finals = plan
        .blank_static_finals
        .iter()
        .map(|i| FieldRef::new(*i))
        .collect::<Vec<_>>();
    let context = backend_java::static_initializer::StaticInitializerContext {
        owner,
        writable_static_fields: writable,
    };
    let source = match exception_metadata {
        Some(metadata) => {
            backend_java::static_initializer::emit_static_initializer_body_subset_with_exceptions(
                ir,
                &method,
                &backend_java::ParameterBindings::default(),
                resolver,
                &context,
                &finals,
                metadata,
            )
        }
        None => backend_java::static_initializer::emit_static_initializer_body_subset(
            ir,
            &method,
            &backend_java::ParameterBindings::default(),
            resolver,
            &context,
            &finals,
        ),
    }
    .map_err(|_| Error::FieldInitializationUnavailable)?;
    Ok(StaticInitializerBodySubset {
        encoded: plan,
        source,
    })
}

// Equality with zero is insufficient for floating values: -0.0 is an
// observable nondefault bit pattern. Encoded kinds must also match field types.
fn encoded_vm_default(value: &V, ty: &JavaType) -> bool {
    match (value, ty) {
        (V::Boolean(false), JavaType::Boolean)
        | (V::Byte(0), JavaType::Byte)
        | (V::Short(0), JavaType::Short)
        | (V::Char(0), JavaType::Char)
        | (V::Int(0), JavaType::Int)
        | (V::Long(0), JavaType::Long)
        | (V::Null, JavaType::Class(_) | JavaType::Array(_)) => true,
        (V::Float(value), JavaType::Float) => value.to_bits() == 0,
        (V::Double(value), JavaType::Double) => value.to_bits() == 0,
        _ => false,
    }
}

fn prove_attested_static_store_sites(
    dex_index: usize,
    method: u32,
    field_index: u32,
    ssa: &dex_decompiler::ssa::SsaMethod,
    sites: &BTreeSet<write_inventory::WriteSite>,
) -> Result<(), Error> {
    use managed_ir::insn::ManagedInsn;
    let mut retained = BTreeSet::new();
    for (id, insn) in ssa.ir().instructions() {
        if let ManagedInsn::FieldPut { object, field, .. } = insn
            && field.index() == field_index
            && (object.is_some()
                || !retained.insert(write_inventory::WriteSite {
                    dex: dex_index,
                    method,
                    offset: ssa
                        .offset(id)
                        .ok_or(Error::FieldInitializationUnavailable)?,
                    is_static: true,
                }))
        {
            return Err(Error::FieldInitializationUnavailable);
        }
    }
    if &retained != sites {
        return Err(Error::FieldInitializationUnavailable);
    }
    Ok(())
}

// Prove the original bytecode, independently of SSA branch restructuring.
// Every normal path assigns all blank finals once; throwing paths retain
// their original effects and need not establish definite assignment.
fn prove_original_static_final_paths(
    dex: &DexFile,
    original: &dex_format::class::EncodedMethod,
    finals: &BTreeSet<u32>,
    ssa: &dex_decompiler::ssa::SsaMethod,
    exceptional: bool,
) -> Result<(), Error> {
    if let Some(summary) = ssa.original_static_final_proof().filter(|summary| {
        original
            .code
            .as_ref()
            .is_some_and(|code| summary.matches_original(dex, original.method_idx, code))
    }) {
        return prove_original_static_final_paths_from_graph(summary, finals, ssa, exceptional);
    }
    let lifted = dex_decompiler::DexPasses::run_with(
        original.code.as_ref().ok_or(Error::InvalidMetadata)?,
        &dex_decompiler::DexTypes::new(dex, Some(original.method_idx)),
    )
    .map_err(|_| Error::FieldInitializationUnavailable)?;
    prove_original_static_final_paths_from_graph(&lifted, finals, ssa, exceptional)
}

trait OriginalStaticFinalProofGraph {
    type Block: Copy + Ord;

    fn entry(&self) -> Self::Block;
    fn field_puts(&self) -> impl Iterator<Item = (u32, bool, Option<u32>)> + '_;
    fn instruction(
        &self,
        block: Self::Block,
    ) -> Result<dex_decompiler::static_final_proof::ProofInstruction, Error>;
    fn outgoing_len(&self, block: Self::Block) -> Result<usize, Error>;
    fn edge(&self, block: Self::Block, index: usize) -> Result<(Self::Block, bool), Error>;
}

impl OriginalStaticFinalProofGraph for dex_decompiler::lift::LiftedMethod {
    type Block = decomp_core::ids::BlockId;

    fn entry(&self) -> Self::Block {
        self.ir().cfg().entry()
    }

    fn field_puts(&self) -> impl Iterator<Item = (u32, bool, Option<u32>)> + '_ {
        use managed_ir::insn::ManagedInsn;
        self.ir()
            .instructions()
            .iter()
            .filter_map(move |(id, instruction)| {
                if let ManagedInsn::FieldPut { field, object, .. } = instruction {
                    Some((field.index(), object.is_none(), self.offset(id)))
                } else {
                    None
                }
            })
    }

    fn instruction(
        &self,
        block: Self::Block,
    ) -> Result<dex_decompiler::static_final_proof::ProofInstruction, Error> {
        use managed_ir::insn::ManagedInsn;
        let node = self
            .ir()
            .cfg()
            .block(block)
            .map_err(|_| Error::InvalidMetadata)?;
        Ok(
            match self
                .ir()
                .instruction(*node.payload())
                .map_err(|_| Error::InvalidMetadata)?
            {
                ManagedInsn::FieldPut { field, object, .. } => {
                    dex_decompiler::static_final_proof::ProofInstruction::FieldPut {
                        field: field.index(),
                        is_static: object.is_none(),
                    }
                }
                ManagedInsn::FieldGet { field, object, .. } => {
                    dex_decompiler::static_final_proof::ProofInstruction::FieldGet {
                        field: field.index(),
                        is_static: object.is_none(),
                    }
                }
                ManagedInsn::Return { value: None } => {
                    dex_decompiler::static_final_proof::ProofInstruction::ReturnVoid
                }
                ManagedInsn::Throw { .. } => {
                    dex_decompiler::static_final_proof::ProofInstruction::Throw
                }
                _ => dex_decompiler::static_final_proof::ProofInstruction::Other,
            },
        )
    }

    fn outgoing_len(&self, block: Self::Block) -> Result<usize, Error> {
        self.ir()
            .cfg()
            .block(block)
            .map(|node| node.outgoing().len())
            .map_err(|_| Error::InvalidMetadata)
    }

    fn edge(&self, block: Self::Block, index: usize) -> Result<(Self::Block, bool), Error> {
        let cfg = self.ir().cfg();
        let node = cfg.block(block).map_err(|_| Error::InvalidMetadata)?;
        let edge_id = *node.outgoing().get(index).ok_or(Error::InvalidMetadata)?;
        let edge = cfg.edge(edge_id).map_err(|_| Error::InvalidMetadata)?;
        Ok((
            edge.target(),
            matches!(edge.kind(), decomp_core::cfg::EdgeKind::Exception(_)),
        ))
    }
}

impl OriginalStaticFinalProofGraph for dex_decompiler::static_final_proof::StaticFinalProofSummary {
    type Block = usize;

    fn entry(&self) -> Self::Block {
        dex_decompiler::static_final_proof::StaticFinalProofSummary::entry(self)
    }

    fn field_puts(&self) -> impl Iterator<Item = (u32, bool, Option<u32>)> + '_ {
        self.field_puts()
            .iter()
            .map(|site| (site.field(), site.is_static(), Some(site.offset())))
    }

    fn instruction(
        &self,
        block: Self::Block,
    ) -> Result<dex_decompiler::static_final_proof::ProofInstruction, Error> {
        self.block(block)
            .map(|(instruction, _)| instruction)
            .ok_or(Error::InvalidMetadata)
    }

    fn outgoing_len(&self, block: Self::Block) -> Result<usize, Error> {
        self.block(block)
            .map(|(_, edges)| edges.len())
            .ok_or(Error::InvalidMetadata)
    }

    fn edge(&self, block: Self::Block, index: usize) -> Result<(Self::Block, bool), Error> {
        let (_, edges) = self.block(block).ok_or(Error::InvalidMetadata)?;
        let edge = edges.get(index).ok_or(Error::InvalidMetadata)?;
        Ok((edge.target(), edge.is_exceptional()))
    }
}

fn prove_original_static_final_paths_from_graph<G: OriginalStaticFinalProofGraph>(
    graph: &G,
    finals: &BTreeSet<u32>,
    ssa: &dex_decompiler::ssa::SsaMethod,
    exceptional: bool,
) -> Result<(), Error> {
    use dex_decompiler::static_final_proof::ProofInstruction;
    use managed_ir::insn::ManagedInsn;
    let mut raw_sites = BTreeSet::new();
    for (field, is_static, offset) in graph.field_puts() {
        if finals.contains(&field) {
            if !is_static {
                return Err(Error::FieldInitializationUnavailable);
            }
            raw_sites.insert((field, offset.ok_or(Error::InvalidMetadata)?));
        }
    }
    let mut ssa_sites = BTreeSet::new();
    for (id, insn) in ssa.ir().instructions() {
        if let ManagedInsn::FieldPut { field, object, .. } = insn
            && finals.contains(&field.index())
        {
            if object.is_some() {
                return Err(Error::FieldInitializationUnavailable);
            }
            ssa_sites.insert((field.index(), ssa.offset(id).ok_or(Error::InvalidMetadata)?));
        }
    }
    if raw_sites != ssa_sites {
        return Err(Error::FieldInitializationUnavailable);
    }
    let mut pending = vec![(graph.entry(), BTreeSet::<u32>::new())];
    let mut seen = BTreeSet::new();
    let mut normal = false;
    while let Some((block, mut assigned)) = pending.pop() {
        // Reuse an identical join/cycle state. A repeated final store changes
        // the assignment state and is rejected before that state can recur.
        if !seen.insert((block, assigned.clone())) {
            continue;
        }
        if seen.len() > 65_536 {
            return Err(Error::FieldInitializationUnavailable);
        }
        let before = assigned.clone();
        match graph.instruction(block)? {
            ProofInstruction::FieldPut { field, is_static }
                if finals.contains(&field) && (!is_static || !assigned.insert(field)) =>
            {
                return Err(Error::FieldInitializationUnavailable);
            }
            ProofInstruction::FieldGet { field, .. }
                if finals.contains(&field) && !assigned.contains(&field) =>
            {
                return Err(Error::FieldInitializationUnavailable);
            }
            ProofInstruction::ReturnVoid => {
                if graph.outgoing_len(block)? != 0 || &assigned != finals {
                    return Err(Error::FieldInitializationUnavailable);
                }
                normal = true;
                continue;
            }
            ProofInstruction::Throw if graph.outgoing_len(block)? == 0 => continue,
            _ => {}
        }
        let outgoing_len = graph.outgoing_len(block)?;
        if outgoing_len == 0 {
            return Err(Error::FieldInitializationUnavailable);
        }
        for index in 0..outgoing_len {
            let (target, exceptional_edge) = graph.edge(block, index)?;
            if exceptional_edge {
                if !exceptional {
                    return Err(Error::FieldInitializationUnavailable);
                }
                pending.push((target, before.clone()));
            } else {
                pending.push((target, assigned.clone()));
            }
        }
    }
    if !normal {
        return Err(Error::FieldInitializationUnavailable);
    }
    Ok(())
}

/// Verify that a complete-output claim retains every actual initializer body.
/// Declaration availability and an encoded-prefix plan alone are insufficient.
/// Call before publishing a class; propagate failure as unavailable status.
pub fn check_class_initializer_coverage(
    dex: &DexFile,
    class: &ClassDef,
    constructors: &BTreeSet<u32>,
    clinit: Option<u32>,
) -> Result<(), Error> {
    if class.class_data_off != 0 && class.class_data.is_none() {
        return Err(Error::InvalidMetadata);
    }
    let mut declared = BTreeSet::new();
    let mut static_initializer = None;
    if let Some(data) = &class.class_data {
        for (methods, direct) in [(&data.direct_methods, true), (&data.virtual_methods, false)] {
            for method in methods {
                let raw = dex
                    .method_id(method.method_idx)
                    .map_err(|_| Error::InvalidMetadata)?;
                let name = dex
                    .string(raw.name_idx)
                    .map_err(|_| Error::InvalidMetadata)?;
                if name != "<init>" && name != "<clinit>" {
                    continue;
                }
                if !direct || u32::from(raw.class_idx) != class.class_idx {
                    return Err(Error::InvalidMetadata);
                }
                if method.code.is_none()
                    || method.code_off == 0
                    || method.access_flags & (0x100 | 0x400) != 0
                {
                    return Err(Error::FieldInitializationUnavailable);
                }
                if name == "<init>" {
                    if method.access_flags & 8 != 0 || !declared.insert(method.method_idx) {
                        return Err(Error::InvalidMetadata);
                    }
                } else if method.access_flags & 8 == 0
                    || static_initializer.replace(method.method_idx).is_some()
                {
                    return Err(Error::InvalidMetadata);
                }
            }
        }
    }
    if &declared != constructors || static_initializer != clinit {
        return Err(Error::FieldInitializationUnavailable);
    }
    Ok(())
}

/// Complete initialization wiring without editing the shared facade. `inputs`
/// must be the complete unfiltered DEX input. Resolver names must be frozen and
/// its declaration/access/exception evidence must come from input or trusted
/// classpath declarations, not pool references. IR references are DEX-local.
pub fn plan_class_initialization<R: backend_java::constructors::TrustedConstructorResolver>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    clinit: Option<(
        managed_ir::refs::MethodRef,
        &managed_ir::method::MethodIr<managed_ir::method::SsaForm>,
    )>,
    resolver: &R,
) -> Result<ClassInitializationPlan, Error> {
    let inventory = write_inventory::WriteInventory::scan(inputs);
    plan_class_initialization_with_inventory(
        inputs,
        dex_index,
        class,
        constructors,
        clinit,
        resolver,
        &inventory,
    )
}

/// Same atomic wiring with a reusable inventory scanned once before filtering.
/// The inventory must have been collected from this complete `inputs` slice.
pub fn plan_class_initialization_with_inventory<
    R: backend_java::constructors::TrustedConstructorResolver,
>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    clinit: Option<(
        managed_ir::refs::MethodRef,
        &managed_ir::method::MethodIr<managed_ir::method::SsaForm>,
    )>,
    resolver: &R,
    inventory: &write_inventory::WriteInventory,
) -> Result<ClassInitializationPlan, Error> {
    plan_class_initialization_with_inventory_contract(
        inputs,
        dex_index,
        class,
        constructors,
        clinit,
        resolver,
        (inventory, WriteContract::RuntimeExclusive, None, None),
    )
}

fn plan_class_initialization_with_inventory_contract<
    R: backend_java::constructors::TrustedConstructorResolver,
>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    clinit: Option<(
        managed_ir::refs::MethodRef,
        &managed_ir::method::MethodIr<managed_ir::method::SsaForm>,
    )>,
    resolver: &R,
    write_evidence: (
        &write_inventory::WriteInventory,
        WriteContract,
        Option<&dex_decompiler::ssa::SsaMethod>,
        Option<Result<&StaticInitializerBodySubset, &Error>>,
    ),
) -> Result<ClassInitializationPlan, Error> {
    let (inventory, contract, attested, precomputed_static) = write_evidence;
    use managed_ir::refs::{ClassRef, FieldRef};
    let dex = inputs.get(dex_index).ok_or(Error::InvalidMetadata)?;
    if !contains_original_class(&dex.classes, class) {
        return Err(Error::InvalidMetadata);
    }
    let retained = constructors
        .iter()
        .map(|c| c.reference.index())
        .collect::<BTreeSet<_>>();
    if retained.len() != constructors.len() {
        return Err(Error::FieldInitializationUnavailable);
    }
    check_class_initializer_coverage(dex, class, &retained, clinit.map(|(r, _)| r.index()))?;
    let attested_static = if let Some(ssa) = attested {
        let (reference, ir) = clinit.ok_or(Error::InvalidMetadata)?;
        if !std::ptr::eq(ssa.ir(), ir) {
            return Err(Error::InvalidMetadata);
        }
        match precomputed_static {
            // Driver-computed proof of the identical evidence (same inputs);
            // reuses it, or its failure, instead of re-proving after a retry.
            Some(Ok(plan)) => Some(plan.clone()),
            Some(Err(error)) => return Err(error.clone()),
            None => Some(plan_static_initializer_attested_only(
                inputs, dex_index, class, reference, ssa, resolver, inventory,
            )?),
        }
    } else {
        None
    };
    let encoded = if let Some(static_plan) = &attested_static {
        // Keep the complete field-metadata/instance-final validation. Only
        // static finals already proven by the attested source plan bypass the
        // legacy encoded-value/body-store preflight in this temporary copy.
        let mut validation = class.clone();
        for field in &mut validation
            .class_data
            .as_mut()
            .ok_or(Error::InvalidMetadata)?
            .static_fields
        {
            if static_plan
                .encoded
                .blank_static_finals
                .contains(&field.field_idx)
            {
                field.access_flags &= !0x10;
            }
        }
        let instance_plan =
            plan_encoded_initialization_contract(dex, &validation, inventory, contract)?;
        let mut encoded = static_plan.encoded.clone();
        encoded.blank_instance_finals = instance_plan.blank_instance_finals;
        encoded
    } else {
        plan_encoded_initialization_contract(dex, class, inventory, contract)?
    };
    let owner = resolver
        .class(ClassRef::new(class.class_idx))
        .ok_or(Error::InvalidMetadata)?;
    let direct = resolver
        .class(ClassRef::new(class.superclass_idx))
        .ok_or(Error::InvalidSuperclass)?;
    if resolver.constructor_class(&owner).is_none_or(|c| {
        c.superclass.as_deref() != Some(direct.as_str()) || c.access_flags != class.access_flags
    }) {
        return Err(Error::InvalidSuperclass);
    }
    let data = class.class_data.as_ref();
    if let Some(data) = data {
        for (fields, is_static) in [(&data.static_fields, true), (&data.instance_fields, false)] {
            for f in fields {
                let raw = dex
                    .field_id(f.field_idx)
                    .map_err(|_| Error::InvalidMetadata)?;
                let mapped = resolver
                    .field(FieldRef::new(f.field_idx))
                    .ok_or(Error::InvalidMetadata)?;
                let expected = source_type(dex, u32::from(raw.type_idx), &|index| {
                    resolver
                        .class(ClassRef::new(index))
                        .ok_or(Error::InvalidMetadata)
                })?;
                if mapped.owner != owner
                    || mapped.is_static != is_static
                    || mapped.name
                        != dex
                            .string(raw.name_idx)
                            .map_err(|_| Error::InvalidMetadata)?
                    || type_source(&mapped.ty) != expected
                {
                    return Err(Error::InvalidMetadata);
                }
            }
        }
    }
    if data.is_some_and(|d| {
        d.virtual_methods.iter().any(|m| {
            dex.method_id(m.method_idx)
                .ok()
                .is_some_and(|r| dex.string(r.name_idx).ok() == Some("<init>"))
        })
    }) {
        return Err(Error::InvalidMetadata);
    }
    for input in constructors {
        let original = data
            .and_then(|d| {
                d.direct_methods
                    .iter()
                    .find(|m| m.method_idx == input.reference.index())
            })
            .ok_or(Error::InvalidMetadata)?;
        let raw = dex
            .method_id(original.method_idx)
            .map_err(|_| Error::InvalidMetadata)?;
        if original.code.is_none()
            || original.code_off == 0
            || original.access_flags & (8 | 0x100 | 0x400) != 0
        {
            return Err(Error::FieldInitializationUnavailable);
        }
        let proto = dex
            .proto_id(u32::from(raw.proto_idx))
            .map_err(|_| Error::InvalidMetadata)?;
        let mapped = resolver
            .initializer(input.reference)
            .ok_or(Error::InvalidMetadata)?;
        let expected = proto
            .parameters
            .as_ref()
            .into_iter()
            .flat_map(|p| &p.types)
            .map(|i| {
                source_type(dex, u32::from(*i), &|index| {
                    resolver
                        .class(ClassRef::new(index))
                        .ok_or(Error::InvalidMetadata)
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let actual = mapped
            .method
            .parameters
            .iter()
            .map(type_source)
            .collect::<Vec<_>>();
        if u32::from(raw.class_idx) != class.class_idx
            || dex
                .string(raw.name_idx)
                .map_err(|_| Error::InvalidMetadata)?
                != "<init>"
            || descriptor(dex, proto.return_type_idx)? != "V"
            || mapped.method.owner != owner
            || expected != actual
            || resolver.constructor_access(input.reference) != Some(original.access_flags)
        {
            return Err(Error::InvalidMetadata);
        }
    }
    let references = constructors
        .iter()
        .map(|c| c.reference.index())
        .collect::<BTreeSet<_>>();
    let finals = prove_instance_final_writes_contract(
        inputs,
        dex_index,
        class,
        &references,
        inventory,
        contract,
    )?;
    let original_owner = descriptor(dex, class.class_idx)?;
    let constructor_plans = backend_java::constructors::plan_constructor_family(
        &owner,
        constructors,
        &finals,
        resolver,
    )
    .map_err(|error| constructor_planning_error(original_owner, constructors, error))?;
    let constructor_evidence = constructors
        .iter()
        .map(|input| {
            backend_java::constructors::constructor_initialization_evidence(
                input,
                &direct,
                &finals.iter().map(|f| f.reference).collect::<Vec<_>>(),
                resolver,
            )
            .map_err(|_| Error::FieldInitializationUnavailable)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let declared_clinit = class
        .class_data
        .as_ref()
        .into_iter()
        .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
        .filter(|m| {
            dex.method_id(m.method_idx)
                .ok()
                .is_some_and(|raw| dex.string(raw.name_idx).ok() == Some("<clinit>"))
        })
        .collect::<Vec<_>>();
    if declared_clinit.len() > 1
        || declared_clinit.first().map(|m| m.method_idx) != clinit.map(|(r, _)| r.index())
    {
        return Err(Error::FieldInitializationUnavailable);
    }
    let static_initializer = if let Some(static_plan) = attested_static {
        Some(static_plan.source)
    } else if let Some((reference, ir)) = clinit {
        if declared_clinit[0].code.is_none() || declared_clinit[0].code_off == 0 {
            return Err(Error::FieldInitializationUnavailable);
        }
        let method = resolver.method(reference).ok_or(Error::InvalidMetadata)?;
        let raw = dex
            .method_id(reference.index())
            .map_err(|_| Error::InvalidMetadata)?;
        let proto = dex
            .proto_id(u32::from(raw.proto_idx))
            .map_err(|_| Error::InvalidMetadata)?;
        if method.owner != owner
            || u32::from(raw.class_idx) != class.class_idx
            || descriptor(dex, proto.return_type_idx)? != "V"
            || proto
                .parameters
                .as_ref()
                .is_some_and(|p| !p.types.is_empty())
            || declared_clinit[0].access_flags & !(8 | 0x1000 | 0x10000) != 0
            || declared_clinit[0].access_flags & 8 == 0
        {
            return Err(Error::InvalidMetadata);
        }
        let data = class.class_data.as_ref().ok_or(Error::InvalidMetadata)?;
        let blank_finals = prove_static_final_writes_contract(
            dex,
            dex_index,
            reference.index(),
            &encoded,
            inventory,
            contract,
        )?;
        let mut writable = Vec::new();
        for f in &data.static_fields {
            if f.access_flags & 0x10 == 0 {
                let resolved = resolver
                    .field(FieldRef::new(f.field_idx))
                    .ok_or(Error::InvalidMetadata)?;
                let original = dex
                    .field_id(f.field_idx)
                    .map_err(|_| Error::InvalidMetadata)?;
                if resolved.owner != owner
                    || !resolved.is_static
                    || dex
                        .string(original.name_idx)
                        .map_err(|_| Error::InvalidMetadata)?
                        != resolved.name
                {
                    return Err(Error::InvalidMetadata);
                }
                writable.push(resolved);
            }
        }
        Some(
            backend_java::static_initializer::emit_static_initializer_with_finals(
                ir,
                &method,
                &backend_java::ParameterBindings::default(),
                resolver,
                &backend_java::static_initializer::StaticInitializerContext {
                    owner,
                    writable_static_fields: writable,
                },
                &blank_finals,
            )
            .map_err(|_| Error::FieldInitializationUnavailable)?,
        )
    } else {
        if !encoded.blank_static_finals.is_empty() {
            return Err(Error::FieldInitializationUnavailable);
        }
        None
    };
    Ok(ClassInitializationPlan {
        encoded,
        constructors: constructor_plans,
        static_initializer,
        constructor_evidence,
    })
}

/// Encoded assignments run before the original clinit, in class-data order.
/// Using a block rather than constant-variable declarations preserves active-use
/// class initialization (including effects of the original clinit).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedInitializationPlan {
    /// Encoded/default assignments in class-data order.
    pub assignments: Vec<FieldInitialization>,
    /// Insert after field declarations and before original clinit.
    pub prefix_block: String,
    /// Finals requiring once-only assignment by original clinit.
    pub blank_static_finals: BTreeSet<u32>,
    /// Finals requiring complete constructor-family proof.
    pub blank_instance_finals: BTreeSet<u32>,
}

/// Plan encoded/default initialization against complete input write evidence.
pub fn plan_encoded_initialization(
    dex: &DexFile,
    class: &ClassDef,
    inventory: &write_inventory::WriteInventory,
) -> Result<EncodedInitializationPlan, Error> {
    plan_encoded_initialization_contract(dex, class, inventory, WriteContract::RuntimeExclusive)
}

fn plan_encoded_initialization_contract(
    dex: &DexFile,
    class: &ClassDef,
    inventory: &write_inventory::WriteInventory,
    contract: WriteContract,
) -> Result<EncodedInitializationPlan, Error> {
    if (class.class_data_off != 0 && class.class_data.is_none())
        || (class.static_values_off != 0 && class.static_values.is_none())
    {
        return Err(Error::InvalidMetadata);
    }
    let mut copy = class.clone();
    let mut plan = EncodedInitializationPlan {
        assignments: vec![],
        prefix_block: String::new(),
        blank_static_finals: BTreeSet::new(),
        blank_instance_finals: BTreeSet::new(),
    };
    let values = class.static_values.as_deref().unwrap_or_default();
    let mut written = HashSet::new();
    if let Some(data) = &mut copy.class_data {
        let mut seen = HashSet::new();
        for (fields, is_static) in [(&data.static_fields, true), (&data.instance_fields, false)] {
            for f in fields {
                let raw = dex
                    .field_id(f.field_idx)
                    .map_err(|_| Error::InvalidMetadata)?;
                if !seen.insert(f.field_idx)
                    || u32::from(raw.class_idx) != class.class_idx
                    || (f.access_flags & 8 != 0) != is_static
                    || f.access_flags & !(1 | 2 | 4 | 8 | 0x10 | 0x40 | 0x80 | 0x1000) != 0
                    || (f.access_flags & 7).count_ones() > 1
                    || f.access_flags & 0x50 == 0x50
                    || ty(dex, u32::from(raw.type_idx))? == JavaType::Void
                {
                    return Err(Error::InvalidMetadata);
                }
            }
        }
        for f in &mut data.instance_fields {
            if f.access_flags & 0x10 != 0 {
                plan.blank_instance_finals.insert(f.field_idx);
            }
            f.access_flags &= !0x10;
        }
        for (position, f) in data.static_fields.iter_mut().enumerate() {
            let key = write_inventory::field_key(dex, f.field_idx).ok_or(Error::InvalidMetadata)?;
            let sites = if f.access_flags & 0x10 != 0 {
                contract.writes(inventory, &key)?
            } else {
                BTreeSet::new()
            };
            if !sites.is_empty() {
                written.insert(f.field_idx);
            }
            if f.access_flags & 0x10 != 0 {
                if sites.is_empty() {
                    // An omitted value is the actual DEX default, not an invented value.
                    f.access_flags &= !0x10;
                } else {
                    // Explicit encoded initialization plus a body write is not Java
                    // definite assignment, even if the encoded value happens to be zero.
                    if position < values.len() {
                        return Err(Error::FieldInitializationUnavailable);
                    }
                    plan.blank_static_finals.insert(f.field_idx);
                    f.access_flags &= !0x10;
                }
            }
        }
    }
    plan.assignments = encoded_field_initializations(dex, &copy, &written)?;
    plan.assignments
        .retain(|f| !plan.blank_static_finals.contains(&f.field_idx));
    if !plan.assignments.is_empty() {
        plan.prefix_block.push_str("static {\n");
        for f in &plan.assignments {
            let raw = dex
                .field_id(f.field_idx)
                .map_err(|_| Error::InvalidMetadata)?;
            let name = dex
                .string(raw.name_idx)
                .map_err(|_| Error::InvalidMetadata)?;
            identifier(name)?;
            plan.prefix_block
                .push_str(&format!("    {name} = {};\n", f.expression));
        }
        plan.prefix_block.push_str("}\n");
    }
    Ok(plan)
}

/// Parent wiring for `plan_constructor_family`: the allowed methods must be the
/// complete retained constructor set, in this DEX's method-reference domain.
/// Cross-DEX/dropped/non-constructor writers are rejected, not filtered out.
pub fn prove_instance_final_writes(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructors: &BTreeSet<u32>,
    inventory: &write_inventory::WriteInventory,
) -> Result<Vec<backend_java::constructors::BlankFinal>, Error> {
    prove_instance_final_writes_contract(
        inputs,
        dex_index,
        class,
        constructors,
        inventory,
        WriteContract::RuntimeExclusive,
    )
}

fn prove_instance_final_writes_contract(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructors: &BTreeSet<u32>,
    inventory: &write_inventory::WriteInventory,
    contract: WriteContract,
) -> Result<Vec<backend_java::constructors::BlankFinal>, Error> {
    let dex = inputs.get(dex_index).ok_or(Error::InvalidMetadata)?;
    let mut out = Vec::new();
    if let Some(data) = &class.class_data {
        let declared = data
            .direct_methods
            .iter()
            .filter_map(|m| {
                let raw = dex.method_id(m.method_idx).ok()?;
                (dex.string(raw.name_idx).ok()? == "<init>" && m.access_flags & 8 == 0)
                    .then_some(m.method_idx)
            })
            .collect::<BTreeSet<_>>();
        if &declared != constructors {
            return Err(Error::FieldInitializationUnavailable);
        }
        for f in &data.instance_fields {
            if f.access_flags & 0x10 == 0 {
                continue;
            }
            let key = write_inventory::field_key(dex, f.field_idx).ok_or(Error::InvalidMetadata)?;
            if u32::from(
                dex.field_id(f.field_idx)
                    .map_err(|_| Error::InvalidMetadata)?
                    .class_idx,
            ) != class.class_idx
            {
                return Err(Error::InvalidMetadata);
            }
            let sites = contract.writes(inventory, &key)?;
            let mut writes = std::collections::HashMap::new();
            for site in sites {
                if site.dex != dex_index || site.is_static || !constructors.contains(&site.method) {
                    return Err(Error::FieldInitializationUnavailable);
                }
                *writes
                    .entry(managed_ir::refs::MethodRef::new(site.method))
                    .or_insert(0) += 1;
            }
            out.push(backend_java::constructors::BlankFinal {
                reference: managed_ir::refs::FieldRef::new(f.field_idx),
                writes,
            });
        }
    }
    Ok(out)
}

/// Parent wiring for `emit_static_initializer_with_finals`; encoded prefixes
/// precede that block, and declarations carry no duplicate initializer.
pub fn prove_static_final_writes(
    dex: &DexFile,
    dex_index: usize,
    clinit: u32,
    plan: &EncodedInitializationPlan,
    inventory: &write_inventory::WriteInventory,
) -> Result<Vec<managed_ir::refs::FieldRef>, Error> {
    prove_static_final_writes_contract(
        dex,
        dex_index,
        clinit,
        plan,
        inventory,
        WriteContract::RuntimeExclusive,
    )
}

fn prove_static_final_writes_contract(
    dex: &DexFile,
    dex_index: usize,
    clinit: u32,
    plan: &EncodedInitializationPlan,
    inventory: &write_inventory::WriteInventory,
    contract: WriteContract,
) -> Result<Vec<managed_ir::refs::FieldRef>, Error> {
    let raw = dex.method_id(clinit).map_err(|_| Error::InvalidMetadata)?;
    if dex
        .string(raw.name_idx)
        .map_err(|_| Error::InvalidMetadata)?
        != "<clinit>"
    {
        return Err(Error::InvalidMetadata);
    }
    plan.blank_static_finals
        .iter()
        .map(|index| {
            let key = write_inventory::field_key(dex, *index).ok_or(Error::InvalidMetadata)?;
            let sites = contract.writes(inventory, &key)?;
            if sites.len() != 1
                || sites
                    .iter()
                    .any(|s| s.dex != dex_index || s.method != clinit || !s.is_static)
                || dex
                    .field_id(*index)
                    .map_err(|_| Error::InvalidMetadata)?
                    .class_idx
                    != raw.class_idx
            {
                return Err(Error::FieldInitializationUnavailable);
            }
            Ok(managed_ir::refs::FieldRef::new(*index))
        })
        .collect()
}

fn identifier(name: &str) -> Result<(), Error> {
    if name.is_empty() || backend_java::sanitize_identifier(name) != name {
        return Err(Error::InvalidMetadata);
    }
    Ok(())
}

fn source_type(
    dex: &DexFile,
    index: u32,
    name: &impl Fn(u32) -> Result<String, Error>,
) -> Result<String, Error> {
    fn render(
        t: &JavaType,
        dex: &DexFile,
        name: &impl Fn(u32) -> Result<String, Error>,
    ) -> Result<String, Error> {
        Ok(match t {
            JavaType::Class(n) => {
                // Resolve the same first descriptor identity without parsing and
                // allocating a Java class name for every preceding pool entry.
                let wanted = format!("L{};", n.replace('.', "/"));
                let index = dex
                    .type_ids
                    .iter()
                    .enumerate()
                    .find_map(|(i, raw)| {
                        (dex.string(raw.descriptor_idx).ok() == Some(wanted.as_str()))
                            .then_some(i as u32)
                    })
                    .ok_or(Error::InvalidMetadata)?;
                name(index)?
            }
            JavaType::Array(t) => format!("{}[]", render(t, dex, name)?),
            _ => type_source(t),
        })
    }
    render(&ty(dex, index)?, dex, name)
}

// Escape literal backslashes (including a literal `\u`), quotes and ASCII controls.
// Octal controls avoid Java's pre-lexical Unicode escape processing.
fn string_literal(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if (c as u32) < 32 || c == '\u{7f}' => out.push_str(&format!("\\{:03o}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn primitive(value: &V, expected: &JavaType) -> Option<String> {
    Some(match (value, expected) {
        (V::Boolean(v), JavaType::Boolean) => v.to_string(),
        (V::Byte(v), JavaType::Byte) => v.to_string(),
        (V::Short(v), JavaType::Short) => v.to_string(),
        (V::Char(v), JavaType::Char) => format!("(char) 0x{v:04x}"),
        (V::Int(v), JavaType::Int) => v.to_string(),
        (V::Long(v), JavaType::Long) => format!("{v}L"),
        (V::Float(v), JavaType::Float) => {
            if v.is_nan() {
                return None;
            } // Payload is not preserved by Java constants.
            if v.is_infinite() {
                format!(
                    "({}1.0f / 0.0f)",
                    if v.is_sign_negative() { "-" } else { "" }
                )
            } else {
                format!("{v:?}f")
            }
        }
        (V::Double(v), JavaType::Double) => {
            if v.is_nan() {
                return None;
            }
            if v.is_infinite() {
                format!(
                    "({}1.0d / 0.0d)",
                    if v.is_sign_negative() { "-" } else { "" }
                )
            } else {
                format!("{v:?}d")
            }
        }
        _ => return None,
    })
}

fn annotation_value(
    dex: &DexFile,
    value: &EncodedValue,
    expected: &JavaType,
    name: &impl Fn(u32) -> Result<String, Error>,
    depth: usize,
) -> Result<String, Error> {
    if depth > 64 {
        return Err(Error::InvalidEncodedValue);
    }
    if let Some(s) = primitive(&value.value, expected) {
        return Ok(s);
    }
    match (&value.value, expected) {
        (V::String(i), JavaType::Class(n)) if n == "java.lang.String" => Ok(string_literal(
            dex.string(*i).map_err(|_| Error::InvalidMetadata)?,
        )),
        (V::Type(i), JavaType::Class(n)) if n == "java.lang.Class" => {
            Ok(format!("{}.class", source_type(dex, *i, name)?))
        }
        (V::Array(values), JavaType::Array(component))
            if !matches!(**component, JavaType::Array(_)) =>
        {
            let values = values
                .iter()
                .map(|v| annotation_value(dex, v, component, name, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(format!("{{{}}}", values.join(", ")))
        }
        (V::Enum(i), JavaType::Class(n)) => {
            let field = dex.field_id(*i).map_err(|_| Error::InvalidMetadata)?;
            if class_name(dex, u32::from(field.class_idx))? != *n
                || ty(dex, u32::from(field.type_idx))? != *expected
            {
                return Err(Error::InvalidEncodedValue);
            }
            // A pool reference is not evidence of an enum constant declaration.
            let owner = dex
                .classes
                .iter()
                .find(|c| c.class_idx == u32::from(field.class_idx))
                .ok_or(Error::InvalidEncodedValue)?;
            if class_kind(owner.access_flags)? != ClassKind::Enum
                || !owner.class_data.as_ref().is_some_and(|d| {
                    d.static_fields
                        .iter()
                        .any(|f| f.field_idx == *i && f.access_flags & 0x4019 == 0x4019)
                })
            {
                return Err(Error::InvalidEncodedValue);
            }
            let member = dex
                .string(field.name_idx)
                .map_err(|_| Error::InvalidMetadata)?;
            identifier(member)?;
            Ok(format!("{}.{member}", name(u32::from(field.class_idx))?))
        }
        // Nested annotations need element-level validation; do not print opaque metadata.
        _ => Err(Error::InvalidEncodedValue),
    }
}

/// Validated defaults keyed by method ID, using raw source class names.
pub fn annotation_defaults(
    dex: &DexFile,
    class: &ClassDef,
) -> Result<BTreeMap<u32, String>, Error> {
    annotation_defaults_with(dex, class, &|i| class_name(dex, i))
}

/// Defaults plus references the facade must include in its member/class closure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnotationDefaultPlan {
    /// Validated source expressions keyed by annotation element method ID.
    pub by_method: BTreeMap<u32, String>,
    /// Type-pool references from class literals and enum constants.
    pub referenced_types: BTreeSet<u32>,
    /// Exact enum constant field references; never infer these from source text.
    pub enum_fields: BTreeSet<u32>,
}

/// Facade-ready atomic default plan in the frozen naming domain. Validate the
/// whole plan before attaching defaults; merge its references into closure
/// requirements so referenced enums/classes cannot be dropped independently.
pub fn plan_annotation_defaults_with(
    dex: &DexFile,
    class: &ClassDef,
    name: &impl Fn(u32) -> Result<String, Error>,
) -> Result<AnnotationDefaultPlan, Error> {
    let by_method = annotation_defaults_with(dex, class, name)?;
    let mut plan = AnnotationDefaultPlan {
        by_method,
        referenced_types: BTreeSet::new(),
        enum_fields: BTreeSet::new(),
    };
    fn references(value: &V, dex: &DexFile, plan: &mut AnnotationDefaultPlan) -> Result<(), Error> {
        match value {
            V::Type(i) => {
                plan.referenced_types.insert(*i);
            }
            V::Enum(i) => {
                let field = dex.field_id(*i).map_err(|_| Error::InvalidMetadata)?;
                plan.enum_fields.insert(*i);
                plan.referenced_types.insert(u32::from(field.class_idx));
                plan.referenced_types.insert(u32::from(field.type_idx));
            }
            V::Array(values) => {
                for value in values {
                    references(&value.value, dex, plan)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    if let Some(set) = class
        .annotations
        .as_ref()
        .and_then(|a| a.class_annotations.as_ref())
    {
        for item in &set.entries {
            if descriptor(dex, item.annotation.type_idx)? != "Ldalvik/annotation/AnnotationDefault;"
            {
                continue;
            }
            for wrapper in &item.annotation.elements {
                if let V::Annotation(defaults) = &wrapper.value.value {
                    for element in &defaults.elements {
                        references(&element.value.value, dex, &mut plan)?;
                    }
                }
            }
        }
    }
    Ok(plan)
}

/// Decode the system AnnotationDefault wrapper and type-check every element.
/// `name` must be the caller's frozen, validated class-name registry. No partial
/// map is returned when any default is malformed, unknown, duplicated or unsupported.
pub fn annotation_defaults_with(
    dex: &DexFile,
    class: &ClassDef,
    name: &impl Fn(u32) -> Result<String, Error>,
) -> Result<BTreeMap<u32, String>, Error> {
    annotation_defaults_inner(dex, class, name, None)
}

pub(crate) fn annotation_defaults_inner(
    dex: &DexFile,
    class: &ClassDef,
    name: &impl Fn(u32) -> Result<String, Error>,
    mut orphaned: Option<&mut Vec<dex_format::encoded_value::AnnotationElement>>,
) -> Result<BTreeMap<u32, String>, Error> {
    let mut out = BTreeMap::new();
    let mut seen_names = BTreeSet::new();
    let Some(set) = class
        .annotations
        .as_ref()
        .and_then(|a| a.class_annotations.as_ref())
    else {
        return Ok(out);
    };
    let mut found = false;
    for item in &set.entries {
        if descriptor(dex, item.annotation.type_idx)? != "Ldalvik/annotation/AnnotationDefault;" {
            continue;
        }
        if found
            || class_kind(class.access_flags)? != ClassKind::Annotation
            || item.visibility != dex_format::annotations::Visibility::System
            || item.annotation.elements.len() != 1
        {
            return Err(Error::AnnotationDefaultsUnavailable);
        }
        found = true;
        let wrapper = &item.annotation.elements[0];
        if dex
            .string(wrapper.name_idx)
            .map_err(|_| Error::InvalidMetadata)?
            != "value"
        {
            return Err(Error::AnnotationDefaultsUnavailable);
        }
        let V::Annotation(defaults) = &wrapper.value.value else {
            return Err(Error::AnnotationDefaultsUnavailable);
        };
        if defaults.type_idx != class.class_idx {
            return Err(Error::AnnotationDefaultsUnavailable);
        }
        for element in &defaults.elements {
            let element_name = dex
                .string(element.name_idx)
                .map_err(|_| Error::InvalidMetadata)?;
            identifier(element_name)?;
            if !seen_names.insert(element_name) {
                return Err(Error::AnnotationDefaultsUnavailable);
            }
            let methods = class
                .class_data
                .as_ref()
                .ok_or(Error::AnnotationDefaultsUnavailable)?;
            let candidates = methods
                .virtual_methods
                .iter()
                .chain(&methods.direct_methods)
                .filter(|m| {
                    dex.method_id(m.method_idx)
                        .ok()
                        .is_some_and(|raw| dex.string(raw.name_idx).ok() == Some(element_name))
                })
                .collect::<Vec<_>>();
            if candidates.is_empty()
                && let Some(retained) = orphaned.as_deref_mut()
            {
                annotation_publication::validate_orphan_value(dex, &element.value, 0)?;
                retained.push(element.clone());
                continue;
            }
            if candidates.len() != 1 {
                return Err(Error::AnnotationDefaultsUnavailable);
            }
            let method = candidates[0];
            let raw = dex
                .method_id(method.method_idx)
                .map_err(|_| Error::InvalidMetadata)?;
            let proto = dex
                .proto_id(u32::from(raw.proto_idx))
                .map_err(|_| Error::InvalidMetadata)?;
            let ret = ty(dex, proto.return_type_idx)?;
            // Reuse all declaration validation, without recursively reading this wrapper.
            let mut declaration = class.clone();
            declaration.annotations = None;
            super::abstract_method_declaration(dex, &declaration, method)?;
            if !annotation_type(dex, &ret) {
                return Err(Error::InvalidAnnotationElement);
            }
            let source = annotation_value(dex, &element.value, &ret, name, 0)?;
            if out.insert(method.method_idx, source).is_some() {
                return Err(Error::AnnotationDefaultsUnavailable);
            }
        }
    }
    Ok(out)
}

/// Encoded initialization attached to a static field, never a body assignment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldInitialization {
    /// Absolute field pool index.
    pub field_idx: u32,
    /// Expression to append to the declaration after ` = `.
    pub expression: String,
    /// Whether the DEX value was explicit (otherwise an omitted default zero/null).
    pub explicit: bool,
}

/// Plan static encoded values by class-data order, not field pool order.
/// `written_fields` must contain every field written by any input body, including
/// methods otherwise diagnosed/dropped, from a complete write inventory.
/// Final fields with writes or without explicit
/// values require a separate definite-assignment plan and are rejected here.
/// Instance finals are likewise rejected, never silently default-initialized.
/// Ordinary non-final body writes must remain in their original order.
pub fn encoded_field_initializations(
    dex: &DexFile,
    class: &ClassDef,
    written_fields: &HashSet<u32>,
) -> Result<Vec<FieldInitialization>, Error> {
    if class_kind(class.access_flags)? != ClassKind::Class {
        return Err(Error::FieldInitializationUnavailable);
    }
    let values = class.static_values.as_deref().unwrap_or_default();
    let Some(data) = &class.class_data else {
        return if values.is_empty() {
            Ok(vec![])
        } else {
            Err(Error::InvalidMetadata)
        };
    };
    if values.len() > data.static_fields.len()
        || data
            .instance_fields
            .iter()
            .any(|f| f.access_flags & 0x10 != 0)
    {
        return Err(Error::FieldInitializationUnavailable);
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (position, field) in data.static_fields.iter().enumerate() {
        let raw = dex
            .field_id(field.field_idx)
            .map_err(|_| Error::InvalidMetadata)?;
        if !seen.insert(field.field_idx)
            || u32::from(raw.class_idx) != class.class_idx
            || field.access_flags & 8 == 0
            || field.access_flags & !(1 | 2 | 4 | 8 | 0x10 | 0x40 | 0x80 | 0x1000) != 0
            || (field.access_flags & 7).count_ones() > 1
            || field.access_flags & 0x50 == 0x50
        {
            return Err(Error::InvalidMetadata);
        }
        if field.access_flags & 0x10 != 0
            && (written_fields.contains(&field.field_idx) || position >= values.len())
        {
            return Err(Error::FieldInitializationUnavailable);
        }
        let expected = ty(dex, u32::from(raw.type_idx))?;
        let expression = if let Some(value) = values.get(position) {
            if let Some(s) = primitive(&value.value, &expected) {
                s
            } else {
                match (&value.value, &expected) {
                    (V::String(i), JavaType::Class(n)) if n == "java.lang.String" => {
                        string_literal(dex.string(*i).map_err(|_| Error::InvalidMetadata)?)
                    }
                    (V::Null, JavaType::Class(_) | JavaType::Array(_)) => "null".into(),
                    _ => return Err(Error::InvalidEncodedValue),
                }
            }
        } else {
            match expected {
                JavaType::Void => return Err(Error::InvalidMetadata),
                JavaType::Boolean => "false".into(),
                JavaType::Char => "(char) 0".into(),
                JavaType::Long => "0L".into(),
                JavaType::Float => "0.0f".into(),
                JavaType::Double => "0.0d".into(),
                JavaType::Class(_) | JavaType::Array(_) => "null".into(),
                _ => "0".into(),
            }
        };
        out.push(FieldInitialization {
            field_idx: field.field_idx,
            expression,
            explicit: position < values.len(),
        });
    }
    Ok(out)
}

/// Facade gate for the existing constructor planner. Unlike an assignment
/// query, this verifies an exact direct-super edge and a complete acyclic
/// superclass chain rooted at Object. Supply input/classpath *declarations*,
/// never type-pool references, in the same frozen naming domain as `name`.
/// This is hierarchy evidence only: superclass legality (including final/access
/// flags), constructor accessibility, exact overload, exceptions and the complete
/// `this` chain still require the facade's metadata checks/ConstructorResolver.
pub fn trusted_constructor_superclass(
    dex: &DexFile,
    class: &ClassDef,
    declarations: &[backend_java::type_compatibility::Declaration],
    name: &impl Fn(u32) -> Result<String, Error>,
) -> Result<String, Error> {
    if class_kind(class.access_flags)? != ClassKind::Class || class.superclass_idx == u32::MAX {
        return Err(Error::InvalidSuperclass);
    }
    let owner = name(class.class_idx)?;
    let direct = name(class.superclass_idx)?;
    if owner == direct {
        return Err(Error::InvalidSuperclass);
    }
    let mut registry = BTreeMap::new();
    for declaration in declarations {
        if let Some(old) = registry.insert(declaration.name.as_str(), declaration)
            && old != declaration
        {
            return Err(Error::InvalidSuperclass);
        }
    }
    let declared = registry
        .get(owner.as_str())
        .ok_or(Error::InvalidSuperclass)?;
    if declared.is_interface || declared.superclass.as_deref() != Some(direct.as_str()) {
        return Err(Error::InvalidSuperclass);
    }
    let mut visited = HashSet::from([owner.as_str()]);
    let mut next = direct.as_str();
    loop {
        if !visited.insert(next) {
            return Err(Error::InvalidSuperclass);
        }
        let parent = registry.get(next).ok_or(Error::InvalidSuperclass)?;
        if parent.is_interface {
            return Err(Error::InvalidSuperclass);
        }
        if next == "java.lang.Object" {
            if parent.superclass.is_some() || !parent.interfaces.is_empty() {
                return Err(Error::InvalidSuperclass);
            }
            break;
        }
        next = parent
            .superclass
            .as_deref()
            .ok_or(Error::InvalidSuperclass)?;
    }
    // Validate descriptor metadata even if the registry has a mapped name.
    class_name(dex, class.class_idx)?;
    class_name(dex, class.superclass_idx)?;
    Ok(direct)
}

fn constructor_planning_error(
    owner: &str,
    constructors: &[backend_java::constructors::ConstructorInput<'_>],
    error: backend_java::Unsupported,
) -> Error {
    Error::ConstructorPlanningUnavailable {
        owner: owner.to_owned(),
        methods: constructors
            .iter()
            .map(|input| input.reference.index())
            .collect(),
        reason: format!("{error:?}"),
    }
}

#[cfg(test)]
mod constructor_planning_diagnostic_tests {
    use super::*;

    #[test]
    fn underlying_source_failure_is_not_final_assignment_failure() {
        let error = constructor_planning_error(
            "Loriginal/Owner;",
            &[],
            backend_java::Unsupported::ControlFlow("overlapping original block85".into()),
        );
        assert_eq!(
            error,
            Error::ConstructorPlanningUnavailable {
                owner: "Loriginal/Owner;".into(),
                methods: vec![],
                reason: "ControlFlow(\"overlapping original block85\")".into(),
            }
        );
        assert_ne!(error, Error::FieldInitializationUnavailable);
    }
}

#[cfg(test)]
mod source_type_lookup_tests {
    use super::*;

    fn legacy(dex: &DexFile, index: u32) -> Result<String, Error> {
        fn render(t: &JavaType, dex: &DexFile) -> Result<String, Error> {
            Ok(match t {
                JavaType::Class(n) => {
                    let index = dex
                        .type_ids
                        .iter()
                        .enumerate()
                        .find_map(|(i, _)| {
                            class_name(dex, i as u32)
                                .ok()
                                .filter(|s| s == n)
                                .map(|_| i as u32)
                        })
                        .ok_or(Error::InvalidMetadata)?;
                    format!("alias_{index}")
                }
                JavaType::Array(t) => format!("{}[]", render(t, dex)?),
                _ => type_source(t),
            })
        }
        render(&ty(dex, index)?, dex)
    }

    #[test]
    fn raw_identity_lookup_preserves_aliases_arrays_and_duplicate_first_match()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut dex = DexFile::from_bytes(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/corpus/dex/clean/classes.dex"
        )))?;
        dex.type_ids.reverse();
        if let Some(first) = dex.type_ids.first().cloned() {
            dex.type_ids.push(first);
        }
        for index in 0..dex.type_ids.len() as u32 {
            assert_eq!(
                source_type(&dex, index, &|i| Ok(format!("alias_{i}"))),
                legacy(&dex, index)
            );
        }
        assert_eq!(
            source_type(&dex, u32::MAX, &|i| Ok(format!("alias_{i}"))),
            legacy(&dex, u32::MAX)
        );
        Ok(())
    }
}
