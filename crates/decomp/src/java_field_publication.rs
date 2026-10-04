//! Whole-input field declarations and source dependency publication.
//!
//! Parent contract: collect all DEX declarations before freezing aliases; resolve
//! operands in their own DEX domain; publish declarations in the *actual* owner's
//! compilation unit. Declaration closure is not initialization/body completeness.
//! No pool reference creates a declaration and no DEX value is an inline constant.
//!
//! Parent sequence:
//! 1. `collect(all_inputs, alias_inventory)` before source filtering; add genuine
//!    external keys/types and freeze the complete alias inventory once.
//! 2. `operand_key(caller_dex, local_index)`, `require_field(...)`, then
//!    `source_use(...)`; the result retains actual owner and receiver strategy.
//! 3. Attach `owner_declarations(...)` to that owner's compilation unit. Attach
//!    `encoded_prefix(...)` before retained original clinit. Complete constructor
//!    and clinit proof comes from the existing atomic initialization planners.
//! 4. `record_owner_publication(owner, initialized)` only after attachment.
//!    `initialized = true` is an explicit parent attestation, not inferred from
//!    declarations, encoded values, a local body, or write-inventory availability.
//! 5. `audit()` after owner publication; inspect both closure predicates and the
//!    typed per-caller blockers. Declaration-only output remains partial.
use super::{FieldAccessContext, FieldOrigin, FieldRequirement, ResolvedField};
use crate::{
    java_aliases::{AliasInventory, FieldKey, JavaAliases, MethodKey},
    java_class_kinds::{
        ClassKind, class_kind,
        initializers::{self, EncodedInitializationPlan},
    },
    java_external_metadata::ExternalMetadata,
};
use dex_format::DexFile;
use std::collections::{BTreeMap, BTreeSet};

#[path = "java_field_use_cache.rs"]
pub(crate) mod cache;

/// Local indices are evidence locations, never global identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDeclaration {
    /// Actual declaring-owner identity.
    pub key: FieldKey,
    /// Input DEX ordinal.
    pub dex: usize,
    /// Owner-local field pool index.
    pub field: u32,
    /// Genuine original flags.
    pub access_flags: u32,
    /// Genuine source class category.
    pub kind: ClassKind,
}

/// Typed unmet source publication evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublicationBlocker {
    /// Original descriptor/access resolution failed.
    Resolution(FieldRequirement),
    /// Frozen registry lacks this exact identity.
    AliasUnavailable(FieldKey),
    /// Owner unit has not published this declaration.
    SourceSubsetAbsent(FieldKey),
    /// Declaration exists but initialization is not proven/published.
    InitializationUnavailable(String),
}

/// Metadata-only availability; initialization must be attested separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldStatus {
    /// Genuine input declaration outside the published subset.
    SourceSubsetAbsent,
    /// Attached declaration with explicitly partial initialization status.
    DeclarationOnly,
    /// Parent attests complete atomic initialization publication.
    InitializedDeclaration,
    /// Genuine compile-classpath dependency.
    ExternalClasspath,
}

/// Backend receiver strategy with access evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldReceiver {
    /// Qualify a static reference by actual owner.
    StaticOwner,
    /// Existing receiver's source lookup reaches this exact member.
    Retain,
    /// Only when casting to the actual owner also passes protected access.
    CastToOwner,
}

/// Source spelling is separate from exact original identity. No literal inlining.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldSourceUse {
    /// Exact original identity.
    pub key: FieldKey,
    /// Frozen declaring-owner name.
    pub owner: String,
    /// Frozen input alias or genuine classpath member name.
    pub name: String,
    /// Frozen source type.
    pub ty: String,
    /// Proven receiver strategy.
    pub receiver: FieldReceiver,
}

/// Use after `require_field`. Access is rechecked before any alias lookup. For
/// protected hidden fields an illegal declaring-owner cast rejects publication.
pub fn source_use(
    inputs: &[DexFile],
    field: &ResolvedField,
    external: &dyn ExternalMetadata,
    context: &FieldAccessContext<'_>,
    aliases: &JavaAliases,
) -> Result<FieldSourceUse, PublicationBlocker> {
    source_use_indexed(inputs, field, external, context, aliases, None)
}
pub(crate) fn source_use_indexed(
    inputs: &[DexFile],
    field: &ResolvedField,
    external: &dyn ExternalMetadata,
    context: &FieldAccessContext<'_>,
    aliases: &JavaAliases,
    index: Option<&super::InputIndex<'_>>,
) -> Result<FieldSourceUse, PublicationBlocker> {
    let resolved = super::resolve_field_indexed(inputs, &field.key, external, context, index)
        .map_err(PublicationBlocker::Resolution)?;
    if resolved != *field {
        return Err(PublicationBlocker::Resolution(FieldRequirement::Invalid(
            field.key.owner.clone(),
        )));
    }
    if !context.is_static {
        let receiver = context.receiver.ok_or_else(|| {
            PublicationBlocker::Resolution(FieldRequirement::Access(field.key.clone()))
        })?;
        let resolver = super::Resolver {
            files: inputs,
            external,
            index,
        };
        if !resolver
            .subtype(receiver, &field.key.owner, &mut BTreeSet::new())
            .map_err(PublicationBlocker::Resolution)?
        {
            return Err(PublicationBlocker::Resolution(FieldRequirement::Access(
                field.key.clone(),
            )));
        }
    }
    let receiver = if context.is_static {
        FieldReceiver::StaticOwner
    } else if super::retain_field_receiver_with_aliases_indexed(
        inputs, field, external, context, aliases, index,
    )
    .map_err(PublicationBlocker::Resolution)?
    {
        FieldReceiver::Retain
    } else {
        super::resolve_field_indexed(
            inputs,
            &field.key,
            external,
            &FieldAccessContext {
                caller: context.caller,
                receiver: Some(&field.key.owner),
                is_static: false,
            },
            index,
        )
        .map_err(PublicationBlocker::Resolution)?;
        FieldReceiver::CastToOwner
    };
    let missing = || PublicationBlocker::AliasUnavailable(field.key.clone());
    let owner = aliases
        .class_name(&field.key.owner)
        .ok_or_else(missing)?
        .into();
    let name = if field.origin == FieldOrigin::Classpath {
        field.key.name.clone()
    } else {
        aliases.field_name(&field.key).ok_or_else(missing)?.into()
    };
    let ty = aliases
        .type_name(&field.key.descriptor)
        .ok_or_else(missing)?;
    Ok(FieldSourceUse {
        key: field.key.clone(),
        owner,
        name,
        ty,
        receiver,
    })
}

/// Descriptor catalog and per-caller declaration/initialization closure.
#[derive(Default, Debug)]
pub struct FieldPublication {
    /// Invalid original metadata retained as evidence, never invented declarations.
    pub limitations: BTreeMap<String, Vec<FieldRequirement>>,
    /// Genuine input field locations across all DEX.
    pub declarations: BTreeMap<FieldKey, FieldDeclaration>,
    /// Actual publication availability, independent of method bodies.
    pub statuses: BTreeMap<FieldKey, FieldStatus>,
    /// Resolved actual-owner field dependencies of each emitted caller.
    pub required_fields: BTreeMap<MethodKey, BTreeSet<FieldKey>>,
    /// Genuine referenced compile-classpath fields.
    pub required_external_fields: BTreeSet<FieldKey>,
    /// Latest order-independent audit, including resolution failures.
    pub unresolved: BTreeMap<MethodKey, Vec<PublicationBlocker>>,
    resolution_failures: BTreeMap<MethodKey, Vec<PublicationBlocker>>,
    /// Each owner retains its encoded-prefix proof or an explicit limitation.
    pub encoded: BTreeMap<String, Result<EncodedInitializationPlan, PublicationBlocker>>,
}

fn descriptor(dex: &DexFile, index: u32) -> Result<String, FieldRequirement> {
    dex.type_id(index)
        .and_then(|t| dex.string(t.descriptor_idx))
        .map(str::to_owned)
        .map_err(|_| FieldRequirement::Invalid(format!("type {index}")))
}

/// Exact symbolic operand identity, before inherited declaring-owner resolution.
pub fn operand_key(dex: &DexFile, index: u32) -> Result<FieldKey, FieldRequirement> {
    let f = dex
        .field_id(index)
        .map_err(|_| FieldRequirement::Invalid(format!("field {index}")))?;
    Ok(FieldKey {
        owner: descriptor(dex, u32::from(f.class_idx))?,
        name: dex
            .string(f.name_idx)
            .map_err(|_| FieldRequirement::Invalid(format!("field {index}")))?
            .into(),
        descriptor: descriptor(dex, u32::from(f.type_idx))?,
    })
}

// The existing planner's raw-name prefix renderer can reject an otherwise valid
// declaration that needs an alias. Keep its type/value decoder and whole-input
// write proof, but defer prefix spelling to the frozen alias registry.
fn encoded_plan(
    dex: &DexFile,
    class: &dex_format::class::ClassDef,
    writes: &initializers::write_inventory::WriteInventory,
    explicit_stores: bool,
) -> Result<EncodedInitializationPlan, crate::java_class_kinds::DeclarationUnsupported> {
    use crate::java_class_kinds::DeclarationUnsupported as Error;
    if explicit_stores && writes.has_structural_unknown() {
        return Err(Error::FieldInitializationUnavailable);
    }
    if explicit_stores && let Some(data) = &class.class_data {
        for field in data
            .static_fields
            .iter()
            .chain(&data.instance_fields)
            .filter(|f| f.access_flags & 0x10 != 0)
        {
            let key = initializers::write_inventory::field_key(dex, field.field_idx)
                .ok_or(Error::InvalidMetadata)?;
            writes
                .explicit_store_writes(&key)
                .map_err(|_| Error::FieldInitializationUnavailable)?;
        }
    }
    let planned = if explicit_stores {
        initializers::plan_encoded_explicit_stores_only(dex, class, writes)
    } else {
        initializers::plan_encoded_initialization(dex, class, writes)
    };
    if let Ok(plan) = planned {
        return Ok(plan);
    }
    if class_kind(class.access_flags)? != ClassKind::Class
        || (class.class_data_off != 0 && class.class_data.is_none())
        || (class.static_values_off != 0 && class.static_values.is_none())
    {
        return Err(Error::FieldInitializationUnavailable);
    }
    let mut copy = class.clone();
    let mut plan = EncodedInitializationPlan {
        assignments: vec![],
        prefix_block: String::new(),
        blank_static_finals: BTreeSet::new(),
        blank_instance_finals: BTreeSet::new(),
    };
    let mut written = std::collections::HashSet::new();
    if let Some(data) = &mut copy.class_data {
        for f in &mut data.instance_fields {
            if f.access_flags & 0x10 != 0 {
                plan.blank_instance_finals.insert(f.field_idx);
            }
            f.access_flags &= !0x10;
        }
        for (position, f) in data.static_fields.iter_mut().enumerate() {
            if f.access_flags & 0x10 == 0 {
                continue;
            }
            let key = initializers::write_inventory::field_key(dex, f.field_idx)
                .ok_or(Error::InvalidMetadata)?;
            let sites = if explicit_stores {
                writes
                    .explicit_store_writes(&key)
                    .map_err(|_| Error::FieldInitializationUnavailable)?
            } else {
                writes
                    .proven_writes(&key)
                    .ok_or(Error::FieldInitializationUnavailable)?
            };
            if !sites.is_empty() {
                if position < class.static_values.as_deref().unwrap_or_default().len() {
                    return Err(Error::FieldInitializationUnavailable);
                }
                written.insert(f.field_idx);
                plan.blank_static_finals.insert(f.field_idx);
            }
            f.access_flags &= !0x10;
        }
    }
    plan.assignments = initializers::encoded_field_initializations(dex, &copy, &written)?;
    plan.assignments
        .retain(|f| !plan.blank_static_finals.contains(&f.field_idx));
    // Catalog validation already checked genuine flags/owner/staticness/types.
    // This is encoded/default initialization only, never constructor/clinit proof.
    Ok(plan)
}

impl FieldPublication {
    fn owner_fields(&self, owner: &str) -> impl Iterator<Item = &FieldDeclaration> {
        self.declarations
            .range(
                FieldKey {
                    owner: owner.into(),
                    name: String::new(),
                    descriptor: String::new(),
                }..,
            )
            .take_while(move |(key, _)| key.owner == owner)
            .map(|(_, field)| field)
    }
    /// Atomic catalog of genuine class-data declarations across all input DEX.
    /// Unsupported bodies poison initialization evidence, not declaration identity.
    pub fn collect(
        inputs: &[DexFile],
        aliases: &mut AliasInventory,
    ) -> Result<Self, FieldRequirement> {
        let writes = initializers::write_inventory::WriteInventory::scan(inputs);
        Self::collect_with_inventory(inputs, aliases, &writes)
    }

    /// Reuse the export's single whole-input write inventory.
    pub fn collect_with_inventory(
        inputs: &[DexFile],
        aliases: &mut AliasInventory,
        writes: &initializers::write_inventory::WriteInventory,
    ) -> Result<Self, FieldRequirement> {
        Self::collect_inventory(inputs, aliases, writes, true, false)
    }
    /// Analyst export inventory: retain valid original declarations while reporting
    /// invalid metadata separately. One malformed field must not hide other units.
    pub fn collect_analyst_with_inventory(
        inputs: &[DexFile],
        aliases: &mut AliasInventory,
        writes: &initializers::write_inventory::WriteInventory,
    ) -> Result<Self, FieldRequirement> {
        Self::collect_inventory(inputs, aliases, writes, false, false)
    }
    /// Analyst declaration catalog with explicit-store definite-assignment evidence
    /// for ordinary final-bearing classes. Native/reflection capability metadata
    /// remains in the supplied inventory; this does not attest initialization or
    /// runtime immutability. Structural and symbolic store uncertainty still deny.
    pub fn collect_analyst_with_explicit_stores(
        inputs: &[DexFile],
        aliases: &mut AliasInventory,
        writes: &initializers::write_inventory::WriteInventory,
    ) -> Result<Self, FieldRequirement> {
        Self::collect_inventory(inputs, aliases, writes, false, true)
    }
    fn collect_inventory(
        inputs: &[DexFile],
        aliases: &mut AliasInventory,
        writes: &initializers::write_inventory::WriteInventory,
        strict: bool,
        explicit_stores: bool,
    ) -> Result<Self, FieldRequirement> {
        let mut result = Self::default();
        let mut inventory = aliases.clone();
        let mut owners = BTreeSet::new();
        let mut ambiguous_fields = BTreeSet::new();
        for (dex_index, dex) in inputs.iter().enumerate() {
            for class in &dex.classes {
                let owner = descriptor(dex, class.class_idx)?;
                if !owners.insert(owner.clone()) {
                    let error = FieldRequirement::Ambiguous(owner.clone());
                    if strict {
                        return Err(error);
                    }
                    result
                        .limitations
                        .entry(owner.clone())
                        .or_default()
                        .push(error);
                    result.declarations.retain(|key, _| key.owner != owner);
                    result.statuses.retain(|key, _| key.owner != owner);
                    continue;
                }
                inventory
                    .add_type(&owner)
                    .map_err(FieldRequirement::Invalid)?;
                let kind = match class_kind(class.access_flags) {
                    Ok(kind) => kind,
                    Err(_) => {
                        let error = FieldRequirement::Invalid(owner.clone());
                        if strict {
                            return Err(error);
                        }
                        result.limitations.entry(owner).or_default().push(error);
                        continue;
                    }
                };
                if class.class_data_off != 0 && class.class_data.is_none() {
                    let error = FieldRequirement::Invalid(owner.clone());
                    if strict {
                        return Err(error);
                    }
                    result.limitations.entry(owner).or_default().push(error);
                    continue;
                }
                if let Some(data) = &class.class_data {
                    for (fields, is_static) in
                        [(&data.static_fields, true), (&data.instance_fields, false)]
                    {
                        for f in fields {
                            let key = operand_key(dex, f.field_idx)?;
                            if ambiguous_fields.contains(&key) {
                                continue;
                            }
                            let flags = f.access_flags;
                            if key.owner != owner
                                || (flags & 8 != 0) != is_static
                                || flags & !(1 | 2 | 4 | 8 | 0x10 | 0x40 | 0x80 | 0x1000 | 0x4000)
                                    != 0
                                || (flags & 7).count_ones() > 1
                                || flags & 0x50 == 0x50
                                || crate::java_descriptors::parse_type_descriptor(&key.descriptor)
                                    .is_err()
                                || key.descriptor == "V"
                                || (flags & 0x4000 != 0
                                    && (kind != ClassKind::Enum
                                        || flags & 0x18 != 0x18
                                        || key.descriptor != owner))
                                || (matches!(kind, ClassKind::Interface | ClassKind::Annotation)
                                    && flags != 0x19
                                    && flags != 0x1019)
                            {
                                let error = FieldRequirement::Invalid(format!(
                                    "{owner}: {key:?}; flags={flags:#x}; static={is_static}"
                                ));
                                if strict {
                                    return Err(error);
                                }
                                result
                                    .limitations
                                    .entry(owner.clone())
                                    .or_default()
                                    .push(error);
                                continue;
                            }
                            inventory
                                .add_field(key.clone())
                                .map_err(FieldRequirement::Invalid)?;
                            if result
                                .declarations
                                .insert(
                                    key.clone(),
                                    FieldDeclaration {
                                        key: key.clone(),
                                        dex: dex_index,
                                        field: f.field_idx,
                                        access_flags: flags,
                                        kind,
                                    },
                                )
                                .is_some()
                            {
                                let error = FieldRequirement::Ambiguous(owner.clone());
                                if strict {
                                    return Err(error);
                                }
                                result
                                    .limitations
                                    .entry(owner.clone())
                                    .or_default()
                                    .push(error);
                                ambiguous_fields.insert(key.clone());
                                result.declarations.remove(&key);
                                result.statuses.remove(&key);
                                continue;
                            }
                            result.statuses.insert(key, FieldStatus::SourceSubsetAbsent);
                        }
                    }
                }
                result.encoded.insert(
                    owner.clone(),
                    encoded_plan(
                        dex,
                        class,
                        writes,
                        explicit_stores
                            && kind == ClassKind::Class
                            && class.class_data.as_ref().is_some_and(|d| {
                                d.static_fields
                                    .iter()
                                    .chain(&d.instance_fields)
                                    .any(|f| f.access_flags & 0x10 != 0)
                            }),
                    )
                    .map_err(|_| PublicationBlocker::InitializationUnavailable(owner)),
                );
            }
        }
        *aliases = inventory;
        Ok(result)
    }

    /// Render declarations with frozen whole-input names. Ordinary blank finals
    /// are honest partial declarations. Interface finals are never emitted blank:
    /// DEX encoded values alone do not prove safe Java constant-variable semantics.
    pub fn owner_declarations(
        &self,
        owner: &str,
        aliases: &JavaAliases,
    ) -> Result<Vec<(FieldKey, String)>, PublicationBlocker> {
        self.owner_fields(owner)
            .map(|f| {
                if matches!(
                    f.kind,
                    ClassKind::Interface | ClassKind::Annotation | ClassKind::Enum
                ) {
                    return Err(PublicationBlocker::InitializationUnavailable(owner.into()));
                }
                let name = aliases
                    .field_name(&f.key)
                    .ok_or_else(|| PublicationBlocker::AliasUnavailable(f.key.clone()))?;
                let ty = aliases
                    .type_name(&f.key.descriptor)
                    .ok_or_else(|| PublicationBlocker::AliasUnavailable(f.key.clone()))?;
                let flags = f.access_flags;
                let visibility = if flags & 1 != 0 {
                    "public "
                } else if flags & 2 != 0 {
                    "private "
                } else if flags & 4 != 0 {
                    "protected "
                } else {
                    ""
                };
                Ok((
                    f.key.clone(),
                    format!(
                        "{visibility}{}{}{}{}{ty} {name};",
                        if flags & 8 != 0 { "static " } else { "" },
                        if flags & 0x10 != 0 { "final " } else { "" },
                        if flags & 0x40 != 0 { "volatile " } else { "" },
                        if flags & 0x80 != 0 { "transient " } else { "" }
                    ),
                ))
            })
            .collect()
    }

    /// Re-render validated encoded assignments, never textual alias replacement.
    /// This prefix alone is not complete initialization proof: retain original
    /// clinit and the complete constructor family with existing atomic planners.
    pub fn encoded_prefix(
        &self,
        owner: &str,
        aliases: &JavaAliases,
    ) -> Result<String, PublicationBlocker> {
        let plan = self
            .encoded
            .get(owner)
            .ok_or_else(|| PublicationBlocker::InitializationUnavailable(owner.into()))?
            .as_ref()
            .map_err(Clone::clone)?;
        let mut block = String::new();
        for assignment in &plan.assignments {
            let field = self
                .owner_fields(owner)
                .find(|f| f.field == assignment.field_idx)
                .ok_or_else(|| PublicationBlocker::InitializationUnavailable(owner.into()))?;
            let name = aliases
                .field_name(&field.key)
                .ok_or_else(|| PublicationBlocker::AliasUnavailable(field.key.clone()))?;
            block.push_str(&format!("    {name} = {};\n", assignment.expression));
        }
        Ok(if block.is_empty() {
            block
        } else {
            format!("static {{\n{block}}}\n")
        })
    }

    /// Parent calls only AFTER attaching every returned declaration to the actual
    /// owner's source unit. `initialized` attests successful existing atomic
    /// initialization planning AND publication of its complete blocks/family.
    /// Unsupported ordinary bodies do not prevent declaration-only publication.
    pub fn record_owner_publication(
        &mut self,
        owner: &str,
        initialized: bool,
    ) -> Result<(), PublicationBlocker> {
        if self.owner_fields(owner).any(|f| f.kind != ClassKind::Class)
            || (initialized && self.encoded.get(owner).is_none_or(Result::is_err))
        {
            return Err(PublicationBlocker::InitializationUnavailable(owner.into()));
        }
        for (key, status) in self
            .statuses
            .range_mut(
                FieldKey {
                    owner: owner.into(),
                    name: String::new(),
                    descriptor: String::new(),
                }..,
            )
            .take_while(|(key, _)| key.owner == owner)
        {
            if key.owner == owner && self.declarations.contains_key(key) {
                *status = if initialized {
                    FieldStatus::InitializedDeclaration
                } else {
                    FieldStatus::DeclarationOnly
                };
            }
        }
        Ok(())
    }

    /// Resolve first in original access/package scope, then record the actual
    /// declaration dependency. No fallback to invented input fields or aliases.
    /// Frozen aliases must include genuine external declarations in advance.
    pub fn require_field(
        &mut self,
        inputs: &[DexFile],
        caller: &MethodKey,
        key: &FieldKey,
        external: &dyn ExternalMetadata,
        context: &FieldAccessContext<'_>,
    ) -> Result<ResolvedField, PublicationBlocker> {
        self.require_field_indexed(inputs, caller, key, external, context, None)
    }
    pub(crate) fn require_field_indexed(
        &mut self,
        inputs: &[DexFile],
        caller: &MethodKey,
        key: &FieldKey,
        external: &dyn ExternalMetadata,
        context: &FieldAccessContext<'_>,
        index: Option<&super::InputIndex<'_>>,
    ) -> Result<ResolvedField, PublicationBlocker> {
        if caller.owner != context.caller {
            let blocker =
                PublicationBlocker::Resolution(FieldRequirement::Invalid(caller.owner.clone()));
            let failures = self.resolution_failures.entry(caller.clone()).or_default();
            if !failures.contains(&blocker) {
                failures.push(blocker.clone());
            }
            return Err(blocker);
        }
        let resolved = match super::resolve_field_indexed(inputs, key, external, context, index) {
            Ok(field) => field,
            Err(error) => {
                let blocker = PublicationBlocker::Resolution(error);
                let failures = self.resolution_failures.entry(caller.clone()).or_default();
                if !failures.contains(&blocker) {
                    failures.push(blocker.clone());
                }
                return Err(blocker);
            }
        };
        self.required_fields
            .entry(caller.clone())
            .or_default()
            .insert(resolved.key.clone());
        if resolved.origin == FieldOrigin::Classpath {
            self.required_external_fields.insert(resolved.key.clone());
            self.statuses
                .insert(resolved.key.clone(), FieldStatus::ExternalClasspath);
        }
        Ok(resolved)
    }

    /// Recompute after all owners are published; discovery/emission order cannot
    /// leave stale blockers. Declaration-only edges explicitly keep partial status.
    pub fn audit(&mut self) {
        self.unresolved = self.resolution_failures.clone();
        for (caller, fields) in &self.required_fields {
            let mut blockers = self.unresolved.remove(caller).unwrap_or_default();
            for key in fields {
                match self.statuses.get(key) {
                    Some(FieldStatus::InitializedDeclaration | FieldStatus::ExternalClasspath) => {}
                    Some(FieldStatus::DeclarationOnly) => {
                        let blocker =
                            PublicationBlocker::InitializationUnavailable(key.owner.clone());
                        if !blockers.contains(&blocker) {
                            blockers.push(blocker);
                        }
                    }
                    _ => blockers.push(PublicationBlocker::SourceSubsetAbsent(key.clone())),
                }
            }
            self.unresolved.insert(caller.clone(), blockers);
        }
    }

    /// Every resolved edge has a declaration and no resolution failed.
    pub fn declaration_closed(&self) -> bool {
        self.resolution_failures.is_empty()
            && self.required_fields.values().flatten().all(|key| {
                matches!(
                    self.statuses.get(key),
                    Some(
                        FieldStatus::DeclarationOnly
                            | FieldStatus::InitializedDeclaration
                            | FieldStatus::ExternalClasspath
                    )
                )
            })
    }

    /// Every resolved edge additionally has initialization publication evidence.
    pub fn initialization_closed(&self) -> bool {
        self.resolution_failures.is_empty()
            && self.required_fields.values().flatten().all(|key| {
                matches!(
                    self.statuses.get(key),
                    Some(FieldStatus::InitializedDeclaration | FieldStatus::ExternalClasspath)
                )
            })
    }
}
