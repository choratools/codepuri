//! Whole-input write evidence, collected before any class/method filtering.
//! Field pool indices are DEX-local; keys below are descriptor identities.
use dex_format::DexFile;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

// Inventory absence proofs need complete declaration lists at every external
// lookup node. A provider's missing exact field alone is not negative evidence.
struct CompleteInventoryMetadata<'a> {
    external: &'a dyn crate::java_external_metadata::ExternalMetadata,
    name: &'a str,
    descriptor: &'a str,
}
impl CompleteInventoryMetadata<'_> {
    fn fields(&self, owner: &str) -> Option<Vec<crate::java_external_metadata::FieldMetadata>> {
        let fields = self.external.declared_fields(owner)?;
        let requested = crate::java_aliases::FieldKey {
            owner: owner.into(),
            name: self.name.into(),
            descriptor: self.descriptor.into(),
        };
        if let Some(exact) = self.external.field(&requested)
            && fields
                .iter()
                .find(|field| field.name == self.name && field.descriptor == self.descriptor)
                != Some(&exact)
        {
            return None;
        }
        let mut identities = BTreeSet::new();
        for field in &fields {
            let key = crate::java_aliases::FieldKey {
                owner: field.owner.clone(),
                name: field.name.clone(),
                descriptor: field.descriptor.clone(),
            };
            if field.owner != owner
                || field.descriptor == "V"
                || crate::java_descriptors::parse_type_descriptor(&field.descriptor).is_err()
                || field.access_flags & !(1 | 2 | 4 | 8 | 0x10 | 0x40 | 0x80 | 0x1000 | 0x4000) != 0
                || (field.access_flags & 7).count_ones() > 1
                || field.access_flags & 0x50 == 0x50
                || !identities.insert((field.name.clone(), field.descriptor.clone()))
                || self
                    .external
                    .field(&key)
                    .is_some_and(|exact| exact != *field)
            {
                return None;
            }
        }
        Some(fields)
    }
}
impl crate::java_external_metadata::ExternalMetadata for CompleteInventoryMetadata<'_> {
    fn field(
        &self,
        key: &crate::java_aliases::FieldKey,
    ) -> Option<crate::java_external_metadata::FieldMetadata> {
        self.fields(&key.owner)?
            .into_iter()
            .find(|field| field.name == key.name && field.descriptor == key.descriptor)
    }
    fn class_hierarchy(&self, owner: &str) -> Vec<crate::type_compatibility::Declaration> {
        if self.fields(owner).is_none() {
            return vec![];
        }
        self.external.class_hierarchy(owner)
    }
    fn class_access(&self, owner: &str) -> Option<u32> {
        self.external.class_access(owner)
    }
    fn method(
        &self,
        key: &crate::java_aliases::MethodKey,
    ) -> Option<crate::java_external_metadata::MethodMetadata> {
        self.external.method(key)
    }
}

#[cfg(test)]
#[path = "java_capability_cache_tests.rs"]
mod capability_cache_tests;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// Exact DEX descriptor identity, shared across input pools.
pub struct FieldKey {
    /// Declaring class descriptor.
    pub owner: String,
    /// Original field name.
    pub name: String,
    /// Field type descriptor.
    pub descriptor: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// One raw bytecode write, including unreachable or dropped bodies.
pub struct WriteSite {
    /// Input DEX ordinal.
    pub dex: usize,
    /// DEX-local method pool index.
    pub method: u32,
    /// Original code-unit offset.
    pub offset: u32,
    /// Static rather than instance assignment.
    pub is_static: bool,
}

/// A genuine declaration location, never inferred from a pool reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDeclaration {
    /// Input DEX ordinal.
    pub dex: usize,
    /// DEX-local field index in the declaration owner.
    pub field: u32,
    /// Original access flags.
    pub access_flags: u32,
}

/// Typed inventory uncertainty; classification comes from decoded metadata,
/// never from diagnostic text. Capability observations remain runtime evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnknownKind {
    /// Incomplete/invalid decoding, declarations, or symbolic resolution.
    Structural,
    /// Genuine code-less native declaration with consistent flags and pools.
    NativeCapability {
        /// Original input DEX ordinal.
        dex: usize,
        /// Genuine DEX-local native declaration.
        method: u32,
    },
    /// Ordinary resolved invocation with a reflective/unsafe capability owner.
    ResolvedReflectiveCapability {
        /// Original input DEX ordinal.
        dex: usize,
        /// DEX-local caller identity.
        method: u32,
        /// Original invoke code-unit offset.
        offset: u32,
        /// Successfully resolved DEX-local target pool identity.
        target: u32,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// Immutable typed uncertainty and its original diagnostic.
pub struct UnknownObservation {
    /// Metadata-derived classification.
    pub kind: UnknownKind,
    /// Diagnostic only; never interpreted to authorize a proof.
    pub reason: String,
}
/// Explicit-store completeness failure, not runtime immutability evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExplicitStoreError {
    /// No unique genuine declaration was established.
    UndeclaredOrAmbiguousField,
    /// Global structural input uncertainty remains relevant to every field.
    StructuralUnknown(Vec<UnknownObservation>),
    /// Exact field identity has unresolved/invalid symbolic store evidence.
    SymbolicFieldUnknown(Vec<String>),
}

#[derive(Clone, Debug, Default)]
/// Complete-input evidence; unknown decoding forbids absence proofs.
pub struct WriteInventory {
    writes: BTreeMap<FieldKey, BTreeSet<WriteSite>>,
    declared_fields: BTreeSet<FieldKey>,
    declaration_locations: BTreeMap<FieldKey, FieldDeclaration>,
    ambiguous_declarations: BTreeSet<FieldKey>,
    reads: BTreeMap<FieldKey, BTreeSet<WriteSite>>,
    unknown: Vec<String>,
    observations: Vec<UnknownObservation>,
    runtime_capabilities: OnceLock<Arc<[UnknownObservation]>>,
    structural_unknown: bool,
    field_unknown: BTreeMap<FieldKey, Vec<String>>,
    methods: usize,
}

/// Resolve a local pool identity without conflating indices across DEX files.
pub fn field_key(dex: &DexFile, index: u32) -> Option<FieldKey> {
    let f = dex.field_id(index).ok()?;
    let descriptor = |i| dex.string(dex.type_id(i).ok()?.descriptor_idx).ok();
    Some(FieldKey {
        owner: descriptor(u32::from(f.class_idx))?.into(),
        name: dex.string(f.name_idx).ok()?.into(),
        descriptor: descriptor(u32::from(f.type_idx))?.into(),
    })
}

impl WriteInventory {
    fn record_unknown(&mut self, kind: UnknownKind, reason: String) {
        // Previously returned Arc snapshots stay immutable when the inventory
        // is extended, including when a cloned inventory diverges.
        let _ = self.runtime_capabilities.take();
        self.structural_unknown |= matches!(kind, UnknownKind::Structural);
        self.observations.push(UnknownObservation {
            kind,
            reason: reason.clone(),
        });
        self.unknown.push(reason);
    }
    /// Every capability/structural observation remains available independently
    /// of source-store legality; callers must retain genuine flags, ABI/names,
    /// native declarations and reflective operations, not infer immutability.
    pub fn unknown_observations(&self) -> &[UnknownObservation] {
        &self.observations
    }

    /// Shared capability evidence in original observation order. Classification
    /// uses metadata kinds, never reason text; structural observations remain
    /// available through `unknown_observations` and still block store proofs.
    pub(crate) fn runtime_capabilities_shared(&self) -> Arc<[UnknownObservation]> {
        Arc::clone(self.runtime_capabilities.get_or_init(|| {
            self.observations
                .iter()
                .filter(|observation| !matches!(observation.kind, UnknownKind::Structural))
                .cloned()
                .collect::<Vec<_>>()
                .into()
        }))
    }

    /// Whether the immutable scan recorded any structural uncertainty.
    pub fn has_structural_unknown(&self) -> bool {
        self.structural_unknown
    }

    /// Complete original explicit DEX stores only. This is neither a runtime
    /// write-absence proof nor permission to inline finals or move their stores.
    /// Native/reflection capabilities do not encode DEX field-store operations;
    /// all structural/symbolic uncertainties still prevent this contract.
    pub fn explicit_store_writes(
        &self,
        field: &FieldKey,
    ) -> Result<BTreeSet<WriteSite>, ExplicitStoreError> {
        if !self.declared_fields.contains(field) || self.ambiguous_declarations.contains(field) {
            return Err(ExplicitStoreError::UndeclaredOrAmbiguousField);
        }
        if self.structural_unknown {
            let structural: Vec<_> = self
                .observations
                .iter()
                .filter(|o| matches!(o.kind, UnknownKind::Structural))
                .cloned()
                .collect();
            return Err(ExplicitStoreError::StructuralUnknown(structural));
        }
        if let Some(reasons) = self.field_unknown.get(field) {
            return Err(ExplicitStoreError::SymbolicFieldUnknown(reasons.clone()));
        }
        Ok(self.writes.get(field).cloned().unwrap_or_default())
    }

    /// Supply every input DEX, including classes that will subsequently be dropped.
    /// Strict decoding scans unreachable instructions too. Unsupported/opaque code,
    /// missing bodies and native capabilities poison absence proofs globally.
    /// Symbolic writes are scoped by identity/signature; pool references alone
    /// never establish declarations. Positive evidence remains diagnostic.
    pub fn scan(inputs: &[DexFile]) -> Self {
        Self::scan_impl(inputs, None)
    }

    /// Scan all original stores with trusted complete input/classpath field
    /// resolution. Exact external targets cannot taint unrelated same-signature
    /// input fields. Missing/ambiguous lookup evidence keeps the conservative
    /// candidate scope; native/reflection and structural observations remain.
    pub fn scan_with_metadata(
        inputs: &[DexFile],
        external: &dyn crate::java_external_metadata::ExternalMetadata,
    ) -> Self {
        Self::scan_impl(inputs, Some(external))
    }

    fn scan_impl(
        inputs: &[DexFile],
        external: Option<&dyn crate::java_external_metadata::ExternalMetadata>,
    ) -> Self {
        let mut result = Self::default();
        let input_index =
            external.map(|_| crate::java_external_metadata::fields::InputIndex::new(inputs));
        let mut resolutions = BTreeMap::new();
        let mut declarations = BTreeMap::new();
        let mut class_names = BTreeSet::new();
        let mut superclasses = BTreeMap::<String, Option<String>>::new();
        for (dex_index, dex) in inputs.iter().enumerate() {
            for class in &dex.classes {
                let name = dex
                    .type_id(class.class_idx)
                    .ok()
                    .and_then(|t| dex.string(t.descriptor_idx).ok());
                if name.is_none_or(|n| {
                    !n.starts_with('L') || !n.ends_with(';') || !class_names.insert(n.to_string())
                }) {
                    result.record_unknown(
                        UnknownKind::Structural,
                        "invalid/duplicate class declaration".into(),
                    );
                }
                // Only unambiguous class-only lookup chains can narrow inherited
                // operands. Interfaces may introduce competing field identities.
                if let Some(name) = name
                    && class.interfaces.as_ref().is_none_or(|i| i.types.is_empty())
                {
                    let parent = if class.superclass_idx == u32::MAX {
                        Some(None)
                    } else {
                        dex.type_id(class.superclass_idx)
                            .ok()
                            .and_then(|t| dex.string(t.descriptor_idx).ok())
                            .map(|p| Some(p.to_string()))
                    };
                    if let Some(parent) = parent {
                        superclasses.insert(name.to_string(), parent);
                    }
                }
                if class.class_data_off != 0 && class.class_data.is_none() {
                    result.record_unknown(UnknownKind::Structural, "missing class data".into());
                }
                if let Some(data) = &class.class_data {
                    for (fields, is_static) in
                        [(&data.static_fields, true), (&data.instance_fields, false)]
                    {
                        for f in fields {
                            let invalid_flags = (f.access_flags & 8 != 0) != is_static
                                || f.access_flags & 0x50 == 0x50;
                            match field_key(dex, f.field_idx) {
                                Some(key)
                                    if u32::from(dex.field_ids[f.field_idx as usize].class_idx)
                                        == class.class_idx =>
                                {
                                    // Bad declaration flags cannot themselves write
                                    // unrelated fields. Body/capability uncertainty
                                    // is still scanned independently below.
                                    if invalid_flags {
                                        result
                                            .field_unknown
                                            .entry(key.clone())
                                            .or_default()
                                            .push("invalid field flags".into());
                                    }
                                    result.declaration_locations.insert(
                                        key.clone(),
                                        FieldDeclaration {
                                            dex: dex_index,
                                            field: f.field_idx,
                                            access_flags: f.access_flags,
                                        },
                                    );
                                    if declarations.insert(key.clone(), is_static).is_some() {
                                        result.ambiguous_declarations.insert(key);
                                        result.record_unknown(
                                            UnknownKind::Structural,
                                            "duplicate field declaration".into(),
                                        );
                                    }
                                }
                                _ => result.record_unknown(
                                    UnknownKind::Structural,
                                    "invalid field declaration".into(),
                                ),
                            }
                        }
                    }
                }
            }
        }
        result.declared_fields.extend(declarations.keys().cloned());
        // A symbolic owner can resolve an inherited field. Without a proven
        // hierarchy edge, conservatively include every declared same-signature
        // field, rather than assuming the symbolic owner is the declaring owner.
        let mut signatures: BTreeMap<(String, String), Vec<FieldKey>> = BTreeMap::new();
        for key in declarations.keys() {
            signatures
                .entry((key.name.clone(), key.descriptor.clone()))
                .or_default()
                .push(key.clone());
        }
        for (dex_index, dex) in inputs.iter().enumerate() {
            for class in &dex.classes {
                let Some(data) = &class.class_data else {
                    continue;
                };
                let mut seen = BTreeSet::new();
                for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                    result.methods += 1;
                    let label = format!("DEX {dex_index} method {}", method.method_idx);
                    if !seen.insert(method.method_idx)
                        || !dex
                            .method_id(method.method_idx)
                            .is_ok_and(|m| u32::from(m.class_idx) == class.class_idx)
                    {
                        result.record_unknown(
                            UnknownKind::Structural,
                            format!("{label}: invalid declaration"),
                        );
                    }
                    let Some(code) = &method.code else {
                        if method.access_flags & 0x100 != 0
                            && method.access_flags & 0x400 == 0
                            && method.code_off == 0
                            && dex.method_id(method.method_idx).is_ok_and(|m| {
                                dex.string(m.name_idx)
                                    .is_ok_and(|n| n != "<init>" && n != "<clinit>")
                                    && dex.proto_id(u32::from(m.proto_idx)).is_ok()
                            })
                        {
                            result.record_unknown(
                                UnknownKind::NativeCapability {
                                    dex: dex_index,
                                    method: method.method_idx,
                                },
                                format!("{label}: unavailable/native body"),
                            );
                        } else if method.access_flags & 0x400 == 0
                            || method.access_flags & 0x100 != 0
                            || method.code_off != 0
                        {
                            result.record_unknown(
                                UnknownKind::Structural,
                                format!("{label}: unavailable/native body"),
                            );
                        }
                        continue;
                    };
                    if method.code_off == 0 || method.access_flags & (0x100 | 0x400) != 0 {
                        result.record_unknown(
                            UnknownKind::Structural,
                            format!("{label}: inconsistent body metadata"),
                        );
                    }
                    match dex_decompiler::decode_method(code) {
                        Err(error) => result
                            .record_unknown(UnknownKind::Structural, format!("{label}: {error}")),
                        Ok(insns) => {
                            for insn in insns {
                                let dex_decompiler::Opcode::Supported(info) = insn.opcode else {
                                    continue;
                                };
                                if info.class == dex_decompiler::SemanticClass::Invoke {
                                    let resolved = insn
                                        .pool_index
                                        .and_then(|i| dex.method_id(i).ok())
                                        .and_then(|m| {
                                            let owner = dex
                                                .string(
                                                    dex.type_id(u32::from(m.class_idx))
                                                        .ok()?
                                                        .descriptor_idx,
                                                )
                                                .ok()?;
                                            dex.string(m.name_idx).ok()?;
                                            dex.proto_id(u32::from(m.proto_idx)).ok()?;
                                            Some(owner)
                                        });
                                    if info.opcode >= 0xfa || resolved.is_none() {
                                        result.record_unknown(
                                            UnknownKind::Structural,
                                            format!(
                                                "{label}: opaque/unresolved invocation at {}",
                                                insn.offset
                                            ),
                                        );
                                    } else if resolved.is_some_and(|n| {
                                        n.starts_with("Ljava/lang/reflect/")
                                            || n.contains("Unsafe;")
                                            || n.starts_with("Ljava/lang/invoke/")
                                    }) {
                                        // Successful resolution above attests this pool identity.
                                        if let Some(target) = insn.pool_index {
                                            result.record_unknown(
                                                UnknownKind::ResolvedReflectiveCapability {
                                                    dex: dex_index,
                                                    method: method.method_idx,
                                                    offset: insn.offset,
                                                    target,
                                                },
                                                format!(
                                                    "{label}: opaque/unresolved invocation at {}",
                                                    insn.offset
                                                ),
                                            );
                                        }
                                    }
                                }
                                if ((0x52..=0x58).contains(&info.opcode)
                                    || (0x60..=0x66).contains(&info.opcode))
                                    && let Some(key) =
                                        insn.pool_index.and_then(|i| field_key(dex, i))
                                {
                                    result.reads.entry(key).or_default().insert(WriteSite {
                                        dex: dex_index,
                                        method: method.method_idx,
                                        offset: insn.offset,
                                        is_static: info.opcode >= 0x60,
                                    });
                                }
                                if !(0x59..=0x5f).contains(&info.opcode)
                                    && !(0x67..=0x6d).contains(&info.opcode)
                                {
                                    continue;
                                }
                                let is_static = info.opcode >= 0x67;
                                match insn.pool_index.and_then(|i| field_key(dex, i)) {
                                    Some(key) if declarations.get(&key) == Some(&is_static) => {
                                        result.writes.entry(key).or_default().insert(WriteSite {
                                            dex: dex_index,
                                            method: method.method_idx,
                                            offset: insn.offset,
                                            is_static,
                                        });
                                    }
                                    Some(key) => {
                                        if !declarations.contains_key(&key)
                                            && let Some(external) = external
                                        {
                                            let resolution = resolutions
                                                .entry((key.clone(), is_static))
                                                .or_insert_with(|| {
                                                    let metadata = CompleteInventoryMetadata {
                                                        external,
                                                        name: &key.name,
                                                        descriptor: &key.descriptor,
                                                    };
                                                    crate::java_external_metadata::fields::resolve_field_declaration_indexed(
                                                        inputs,
                                                        &crate::java_aliases::FieldKey {
                                                            owner: key.owner.clone(),
                                                            name: key.name.clone(),
                                                            descriptor: key.descriptor.clone(),
                                                        },
                                                        &metadata,
                                                        is_static,
                                                        input_index.as_ref(),
                                                    )
                                                });
                                            if let Ok(resolved) = resolution {
                                                let canonical = FieldKey {
                                                    owner: resolved.key.owner.clone(),
                                                    name: resolved.key.name.clone(),
                                                    descriptor: resolved.key.descriptor.clone(),
                                                };
                                                if resolved.origin == crate::java_external_metadata::FieldOrigin::Classpath
                                                    || (declarations.get(&canonical) == Some(&is_static)
                                                        && !result.ambiguous_declarations.contains(&canonical))
                                                {
                                                    // Positive evidence remains inspectable for
                                                    // external fields, without turning them into
                                                    // emitted input declarations or runtime proofs.
                                                    result.writes.entry(canonical).or_default().insert(WriteSite {
                                                        dex: dex_index,
                                                        method: method.method_idx,
                                                        offset: insn.offset,
                                                        is_static,
                                                    });
                                                    continue;
                                                }
                                            }
                                        }
                                        let mut owner = key.owner.as_str();
                                        let mut visited = BTreeSet::new();
                                        let mut inherited = None;
                                        while !declarations.contains_key(&key)
                                            && visited.insert(owner)
                                        {
                                            // An unknown/interface-bearing edge is not a
                                            // negative lookup proof; retain signature scope.
                                            let Some(Some(parent)) = superclasses.get(owner) else {
                                                break;
                                            };
                                            let candidate = FieldKey {
                                                owner: parent.clone(),
                                                name: key.name.clone(),
                                                descriptor: key.descriptor.clone(),
                                            };
                                            if let Some(staticness) = declarations.get(&candidate) {
                                                inherited = Some((candidate, *staticness));
                                                break;
                                            }
                                            owner = parent;
                                        }
                                        if let Some((candidate, staticness)) = inherited
                                            && staticness == is_static
                                            && !result.ambiguous_declarations.contains(&candidate)
                                        {
                                            result.writes.entry(candidate).or_default().insert(
                                                WriteSite {
                                                    dex: dex_index,
                                                    method: method.method_idx,
                                                    offset: insn.offset,
                                                    is_static,
                                                },
                                            );
                                            continue;
                                        }
                                        // A symbolic operand names exactly one field. Missing
                                        // declaration/staticness evidence cannot turn it into
                                        // an arbitrary writer to every other input field.
                                        let reason = format!(
                                            "{label}: unresolved symbolic field write at {}",
                                            insn.offset
                                        );
                                        // A same-owner staticness contradiction must not
                                        // create a positive inherited write. Its malformed
                                        // metadata also cannot certify an absence proof for
                                        // same-signature ancestors, without tainting known
                                        // unrelated owners.
                                        if declarations.contains_key(&key) {
                                            let mut next = Some(key.owner.as_str());
                                            let mut visited = BTreeSet::new();
                                            while let Some(owner) = next {
                                                if !visited.insert(owner) {
                                                    break;
                                                }
                                                let candidate = FieldKey {
                                                    owner: owner.into(),
                                                    name: key.name.clone(),
                                                    descriptor: key.descriptor.clone(),
                                                };
                                                if declarations.contains_key(&candidate) {
                                                    result
                                                        .field_unknown
                                                        .entry(candidate)
                                                        .or_default()
                                                        .push(reason.clone());
                                                }
                                                next = superclasses
                                                    .get(owner)
                                                    .and_then(|parent| parent.as_deref());
                                            }
                                            continue;
                                        }
                                        if let Some(candidates) = signatures
                                            .get(&(key.name.clone(), key.descriptor.clone()))
                                        {
                                            for candidate in candidates {
                                                result
                                                    .field_unknown
                                                    .entry(candidate.clone())
                                                    .or_default()
                                                    .push(reason.clone());
                                            }
                                        }
                                        result.field_unknown.entry(key).or_default().push(reason);
                                    }
                                    None => result.record_unknown(
                                        UnknownKind::Structural,
                                        format!(
                                            "{label}: invalid field operand at {}",
                                            insn.offset
                                        ),
                                    ),
                                }
                            }
                        }
                    }
                }
            }
        }
        result
    }

    /// Arbitrary-capability/invalid-input reasons affecting every field. Scoped
    /// symbolic observations are queried with `field_unknown_reasons`.
    pub fn unknown_reasons(&self) -> &[String] {
        &self.unknown
    }
    /// All declared methods, independent of later source eligibility.
    pub fn scanned_methods(&self) -> usize {
        self.methods
    }
    /// Unknown evidence relevant to this exact descriptor identity. Arbitrary
    /// native/reflection/opaque capabilities remain relevant to every field.
    pub fn field_unknown_reasons(&self, field: &FieldKey) -> Vec<&str> {
        self.unknown
            .iter()
            .chain(self.field_unknown.get(field).into_iter().flatten())
            .map(String::as_str)
            .collect()
    }

    /// Genuine input declaration evidence, independent of initialization proof.
    pub fn declares_field(&self, field: &FieldKey) -> bool {
        self.declared_fields.contains(field)
    }

    /// Cross-DEX declaration availability. This does not certify initialization;
    /// duplicate/invalid input still prevents complete write proofs.
    pub fn field_declaration(&self, field: &FieldKey) -> Option<&FieldDeclaration> {
        if self.ambiguous_declarations.contains(field) {
            return None;
        }
        self.declaration_locations.get(field)
    }

    /// Known incoming read operands, including dropped/unreachable bodies. This
    /// is positive evidence only: opaque bodies may contain additional reads.
    pub fn known_reads(&self, field: &FieldKey) -> Option<&BTreeSet<WriteSite>> {
        self.reads.get(field)
    }

    /// Positive write operands remain available when exclusivity is unknown.
    /// An absent entry is not proof of absence; use `proven_writes` for that.
    pub fn known_writes(&self, field: &FieldKey) -> Option<&BTreeSet<WriteSite>> {
        self.writes.get(field)
    }

    /// Symbolic uncertainty grouped by affected descriptor identities, including
    /// conservative inherited-field candidates. Arbitrary capabilities are in
    /// `unknown_reasons`; diagnostics should report both evidence sources.
    pub fn scoped_unknowns(&self) -> &BTreeMap<FieldKey, Vec<String>> {
        &self.field_unknown
    }

    /// Class-level absence proof checks each actual declaration, not unrelated
    /// symbolic operands. This does not prove constructor/clinit completeness.
    pub fn class_writes_known(&self, owner: &str) -> bool {
        self.unknown.is_empty()
            && self
                .declared_fields
                .range(
                    FieldKey {
                        owner: owner.into(),
                        name: String::new(),
                        descriptor: String::new(),
                    }..,
                )
                .take_while(|f| f.owner == owner)
                .all(|f| !self.field_unknown.contains_key(f))
    }
    /// `None` is unknown, never an empty write set.
    pub fn proven_writes(&self, field: &FieldKey) -> Option<BTreeSet<WriteSite>> {
        (self.unknown.is_empty()
            && !self.field_unknown.contains_key(field)
            && self.declared_fields.contains(field))
        .then(|| self.writes.get(field).cloned().unwrap_or_default())
    }
}
