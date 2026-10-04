//! Global method declaration closure. Field and hierarchy validation are supplied
//! by the parent; semantic publication never implies that a class compiles.
//!
//! Parent wiring:
//! 1. Build `GlobalDeclarations` once over every input DEX.
//! 2. Pass original selected method keys to `close`; emit at the supplied defining
//!    location, borrowing `metadata` for cross-DEX resolver references. Return
//!    actual source and `body_references` only after semantic emission succeeds.
//! 3. Preserve `bodies` and `direct_missing` as independent analyst evidence.
//! 4. Supply the full proposed class inventory and explicit `ClassValidation`
//!    from field, hierarchy, original-modifier and compiler checks. Interface
//!    obligations must identify concrete implementations in the original class;
//!    an abstract declaration on the interface does not discharge that obligation.
//! 5. Choose an explicit `SourceSelection` and publish `failed` alongside output.
use super::{MemberResolver, MethodBodyStatus, external_metadata::ExternalMetadata};
use crate::java_aliases::MethodKey;
use dex_format::DexFile;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Pool-local position of an attested original definition.
pub struct MethodLocation {
    /// Defining DEX in the export input.
    pub dex: usize,
    /// Defining method index in that pool.
    pub index: u32,
}

/// Identity of a reference, not evidence of a declaration.
pub fn method_key(dex: &DexFile, index: u32) -> Option<MethodKey> {
    let descriptor = |index| dex.string(dex.type_id(index).ok()?.descriptor_idx).ok();
    let method = dex.method_id(index).ok()?;
    let proto = dex.proto_id(u32::from(method.proto_idx)).ok()?;
    Some(MethodKey {
        owner: descriptor(u32::from(method.class_idx))?.into(),
        name: dex.string(method.name_idx).ok()?.into(),
        parameters: proto
            .parameters
            .iter()
            .flat_map(|p| &p.types)
            .map(|t| descriptor(u32::from(*t)).map(str::to_owned))
            .collect::<Option<_>>()?,
        return_type: descriptor(proto.return_type_idx)?.into(),
    })
}

/// Only class-data declarations enter this index. Duplicate definitions remain
/// ambiguous even when their code happens to agree.
pub struct GlobalDeclarations<'a> {
    _files: &'a [DexFile],
    // Once-per-input owner shadowing guard for inherited external instance methods.
    input_owners: BTreeSet<String>,
    metadata: BTreeMap<MethodKey, Option<backend_java::JavaMethod>>,
    statuses: BTreeMap<MethodKey, Result<MethodBodyStatus, String>>,
    bodyless: BTreeMap<MethodKey, Result<String, String>>,
    /// Every defining location, retaining ambiguity rather than choosing a winner.
    pub methods: BTreeMap<MethodKey, Vec<MethodLocation>>,
    /// Cached original flags and exact exceptions, independent of source spelling.
    pub declaration_metadata: BTreeMap<MethodKey, Option<super::external_metadata::MethodMetadata>>,
}
impl<'a> GlobalDeclarations<'a> {
    /// Index all actual definitions across the supplied DEX pools.
    pub fn new(files: &'a [DexFile]) -> Result<Self, String> {
        let mut methods: BTreeMap<MethodKey, Vec<MethodLocation>> = BTreeMap::new();
        let mut metadata = BTreeMap::new();
        let mut statuses = BTreeMap::new();
        let mut bodyless = BTreeMap::new();
        let mut declaration_metadata = BTreeMap::new();
        let mut owners = BTreeMap::<String, usize>::new();
        for (pool, dex) in files.iter().enumerate() {
            for class in &dex.classes {
                let owner = dex
                    .string(
                        dex.type_id(class.class_idx)
                            .map_err(|e| e.to_string())?
                            .descriptor_idx,
                    )
                    .map_err(|e| e.to_string())?;
                *owners.entry(owner.to_owned()).or_default() += 1;
                let Some(data) = &class.class_data else {
                    continue;
                };
                for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                    let key = method_key(dex, method.method_idx).ok_or_else(|| {
                        format!("invalid declaration in DEX {pool}: {}", method.method_idx)
                    })?;
                    if key.owner != owner {
                        return Err(format!("declaration owner mismatch: {key:?}"));
                    }
                    let signature = (|| {
                        super::visibility(method.access_flags).ok()?;
                        if backend_java::sanitize_identifier(&key.name) != key.name {
                            return None;
                        }
                        let parameters: Vec<_> = key
                            .parameters
                            .iter()
                            .map(|d| crate::java_descriptors::descriptor_type(d))
                            .collect::<Option<_>>()?;
                        if parameters.contains(&backend_java::JavaType::Void) {
                            return None;
                        }
                        Some(backend_java::JavaMethod {
                            owner: match crate::java_descriptors::descriptor_type(owner)? {
                                backend_java::JavaType::Class(n) => n,
                                _ => return None,
                            },
                            name: key.name.clone(),
                            is_static: method.access_flags & 8 != 0,
                            parameters,
                            return_type: crate::java_descriptors::descriptor_type(
                                &key.return_type,
                            )?,
                            throws: MemberResolver { dex }
                                .declared_throws_for_class(class, method.method_idx)?,
                        })
                    })();
                    let flags = method.access_flags;
                    declaration_metadata.insert(
                        key.clone(),
                        Some(super::external_metadata::MethodMetadata {
                            access_flags: flags,
                            exceptions: MemberResolver { dex }
                                .declared_throws_for_class(class, method.method_idx)
                                .map(|names| {
                                    names
                                        .into_iter()
                                        .map(|n| format!("L{};", n.replace('.', "/")))
                                        .collect()
                                }),
                        }),
                    );
                    let absent = flags & (0x100 | 0x400);
                    let status = if absent == 0x500 || (absent != 0 && method.code.is_some()) {
                        Err("inconsistent native/abstract method body metadata".into())
                    } else {
                        Ok(match absent {
                            0x100 => MethodBodyStatus::Native,
                            0x400 => MethodBodyStatus::Abstract,
                            _ if method.code.is_some() => MethodBodyStatus::SourceBody,
                            _ => MethodBodyStatus::MissingBody,
                        })
                    };
                    if matches!(
                        status,
                        Ok(MethodBodyStatus::Native | MethodBodyStatus::Abstract)
                    ) {
                        let source = (|| {
                            if flags & !(7 | 8 | 0x10 | 0x20 | 0x100 | 0x400 | 0x1000 | 0x20000)
                                != 0
                                || (absent == 0x400
                                    && (class.access_flags & 0x400 == 0
                                        || class.access_flags & 0x10 != 0
                                        || flags & (2 | 8 | 0x10 | 0x20 | 0x20000) != 0))
                                || (absent == 0x100 && class.access_flags & 0x200 != 0)
                            {
                                return Err("unsupported bodyless declaration flags".into());
                            }
                            let m = signature.as_ref().ok_or("unresolved invocation metadata")?;
                            if m.name.starts_with('<') {
                                return Err("constructor requires semantic body".into());
                            }
                            let parameters = m
                                .parameters
                                .iter()
                                .enumerate()
                                .map(|(i, t)| format!("{} p{i}", super::type_source(t)))
                                .collect::<Vec<_>>()
                                .join(", ");
                            let throws = if m.throws.is_empty() {
                                String::new()
                            } else {
                                format!(" throws {}", m.throws.join(", "))
                            };
                            Ok(format!(
                                "{}{}{}{}{}{} {}({parameters}){throws};",
                                super::visibility(flags)?,
                                if flags & 8 != 0 { "static " } else { "" },
                                if flags & 0x10 != 0 { "final " } else { "" },
                                if flags & (0x20 | 0x20000) != 0 {
                                    "synchronized "
                                } else {
                                    ""
                                },
                                if absent == 0x100 {
                                    "native "
                                } else {
                                    "abstract "
                                },
                                super::type_source(&m.return_type),
                                m.name
                            ))
                        })();
                        bodyless.insert(key.clone(), source);
                    }
                    metadata.insert(key.clone(), signature);
                    statuses.insert(key.clone(), status);
                    methods.entry(key).or_default().push(MethodLocation {
                        dex: pool,
                        index: method.method_idx,
                    });
                }
            }
        }
        for (key, value) in &mut declaration_metadata {
            if owners.get(&key.owner) != Some(&1) || methods[key].len() != 1 {
                *value = None;
            }
        }
        Ok(Self {
            _files: files,
            input_owners: owners.into_keys().collect(),
            methods,
            metadata,
            statuses,
            bodyless,
            declaration_metadata,
        })
    }

    /// Borrow genuine metadata from the defining DEX, never from a method-id
    /// reference. Parent resolvers can use this for cross-pool emission.
    pub fn metadata(&self, key: &MethodKey) -> Option<backend_java::JavaMethod> {
        let locations = self.methods.get(key)?;
        if locations.len() != 1 {
            return None;
        }
        self.metadata.get(key)?.clone()
    }

    /// The callback must return the actual successfully emitted source and all
    /// method references in its body (including constructors). Each original
    /// definition is visited at most once, including cross-DEX recursion.
    pub fn close(
        &self,
        roots: impl IntoIterator<Item = MethodKey>,
        external: &dyn ExternalMetadata,
        mut emit: impl FnMut(&MethodKey, MethodLocation) -> Result<EmittedMethod, String>,
    ) -> SourceClosure {
        let mut report = SourceClosure::default();
        for key in self.methods.keys() {
            report
                .original_methods
                .entry(key.owner.clone())
                .or_default()
                .insert(key.clone());
        }
        let mut pending: BTreeSet<_> = roots.into_iter().collect();
        let mut visited = BTreeSet::new();
        while let Some(key) = pending.pop_first() {
            if !visited.insert(key.clone()) {
                continue;
            }
            let Some(locations) = self.methods.get(&key) else {
                match external.method(&key).or_else(|| {
                    if self.input_owners.contains(&key.owner) {
                        None
                    } else {
                        let proof = external.inherited_public_instance(&key)?;
                        (!proof
                            .owners()
                            .iter()
                            .any(|owner| self.input_owners.contains(owner)))
                        .then(|| proof.declaration().metadata.clone())
                    }
                }) {
                    Some(metadata) if metadata.exceptions.is_some() => {
                        report.external.insert(key);
                    }
                    _ => {
                        report.unresolved.insert(
                            key,
                            "exact declaration/classpath metadata unavailable".into(),
                        );
                    }
                }
                continue;
            };
            if locations.len() != 1 {
                report
                    .unresolved
                    .insert(key, "ambiguous global declaration".into());
                continue;
            }
            let location = locations[0];
            match self
                .statuses
                .get(&key)
                .cloned()
                .unwrap_or_else(|| Err("missing target definition".into()))
            {
                Ok(MethodBodyStatus::SourceBody) => match emit(&key, location) {
                    Ok(body) if !body.source.trim().is_empty() => {
                        pending.extend(body.dependencies.iter().cloned());
                        report.bodies.insert(key, body);
                    }
                    Ok(_) => {
                        report
                            .unresolved
                            .insert(key, "empty semantic publication".into());
                    }
                    Err(reason) => {
                        report.unresolved.insert(key, reason);
                    }
                },
                Ok(MethodBodyStatus::Native | MethodBodyStatus::Abstract) => {
                    match self
                        .bodyless
                        .get(&key)
                        .cloned()
                        .unwrap_or_else(|| Err("bodyless declaration unavailable".into()))
                    {
                        Ok(source) => {
                            report.bodyless.insert(key, source);
                        }
                        Err(reason) => {
                            report.unresolved.insert(key, reason);
                        }
                    }
                }
                Ok(MethodBodyStatus::MissingBody) => {
                    report.unresolved.insert(
                        key,
                        "original concrete method body unavailable; dependency required".into(),
                    );
                }
                Err(reason) => {
                    report.unresolved.insert(key, reason);
                }
            }
        }
        report
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Successful semantic emission supplied by the parent adapter.
pub struct EmittedMethod {
    /// Actual Java declaration and body; never a generated fallback.
    pub source: String,
    /// All exact method references in that body.
    pub dependencies: BTreeSet<MethodKey>,
}

#[derive(Default, Debug)]
/// Independent semantic output and declaration-availability evidence.
pub struct SourceClosure {
    /// Full original declaration inventory, preventing silent source omissions.
    pub original_methods: BTreeMap<String, BTreeSet<MethodKey>>,
    /// Actual emitted original bodies.
    pub bodies: BTreeMap<MethodKey, EmittedMethod>,
    /// Actual original native/abstract declarations.
    pub bodyless: BTreeMap<MethodKey, String>,
    /// Exact declarations attested by the supplied compiled classpath.
    pub external: BTreeSet<MethodKey>,
    /// Exact dependencies that cannot be published, with explanatory reasons.
    pub unresolved: BTreeMap<MethodKey, String>,
}
impl SourceClosure {
    /// (semantic bodies, callers with direct missing declarations, unmet edges).
    /// Class/compiler eligibility is deliberately reported by `select_classes`.
    pub fn status_counts(&self) -> (usize, usize, usize) {
        let missing = self.direct_missing();
        (
            self.bodies.len(),
            missing.len(),
            missing.values().map(BTreeSet::len).sum(),
        )
    }
    /// Direct unresolved edges preserve caller body identity. Transitive class
    /// failures are computed separately, after field/hierarchy/compiler checks.
    pub fn direct_missing(&self) -> BTreeMap<MethodKey, BTreeSet<MethodKey>> {
        self.bodies
            .iter()
            .filter_map(|(caller, body)| {
                let missing: BTreeSet<_> = body
                    .dependencies
                    .iter()
                    .filter(|key| self.unresolved.contains_key(*key))
                    .cloned()
                    .collect();
                (!missing.is_empty()).then(|| (caller.clone(), missing))
            })
            .collect()
    }
}

/// Explicit output policy. Neither mode claims that all analyst files compile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceSelection {
    /// Retain every proposed file with its explicit failure report.
    AnalystAll,
    /// Select only validated classes whose source dependencies also survive.
    StrictValidClasses,
}

/// Parent-owned class checks, including fields, superclass/interface obligations,
/// original class modifiers, type closure and actual compiler diagnostics.
/// Missing validation fails closed. Keys and diagnostics retain exact identities.
#[derive(Default, Debug)]
pub struct ClassValidation {
    /// Full original method inventory required in this proposed source class.
    pub methods: BTreeSet<MethodKey>,
    /// Exact concrete implementations required by superclass/interface contracts.
    pub required_methods: BTreeSet<MethodKey>,
    /// Source superclass/interface/type dependencies, established by the parent.
    /// Explicit compiled-classpath classes should not be included here.
    pub source_classes: BTreeSet<String>,
    /// Field/hierarchy/modifier/compiler diagnostics collected by the parent.
    pub failures: Vec<String>,
}
#[derive(Default, Debug)]
/// Selected inventory plus all excluded/failed classes, regardless of policy.
pub struct ClassSelectionReport {
    /// Files selected by the explicit output policy.
    pub selected: BTreeSet<String>,
    /// Validated source classes with a closed surviving class dependency set.
    pub strict_valid: BTreeSet<String>,
    /// Failed proposed classes and exact explanatory diagnostics.
    pub failed: BTreeMap<String, Vec<String>>,
}
impl SourceClosure {
    /// `classes` is the full proposed source inventory, not just classes with
    /// successful bodies. Parent validation must cover every proposed file.
    pub fn select_classes(
        &self,
        classes: &BTreeSet<String>,
        validations: &BTreeMap<String, ClassValidation>,
        mode: SourceSelection,
    ) -> ClassSelectionReport {
        let mut result = ClassSelectionReport::default();
        for owner in classes {
            let Some(validation) = validations.get(owner) else {
                result.failed.insert(
                    owner.clone(),
                    vec!["parent class validation unavailable".into()],
                );
                continue;
            };
            let mut failures = validation.failures.clone();
            let original = self.original_methods.get(owner).into_iter().flatten();
            for key in validation
                .methods
                .iter()
                .chain(&validation.required_methods)
                .chain(original)
            {
                if !self.bodies.contains_key(key) && !self.bodyless.contains_key(key) {
                    failures.push(format!(
                        "required original declaration {key:?}: {}",
                        self.unresolved
                            .get(key)
                            .map(String::as_str)
                            .unwrap_or("not published")
                    ));
                }
            }
            for (key, body) in &self.bodies {
                if &key.owner == owner {
                    for dependency in &body.dependencies {
                        if let Some(reason) = self.unresolved.get(dependency) {
                            failures.push(format!("{key:?} requires {dependency:?}: {reason}"));
                        }
                    }
                }
            }
            if failures.is_empty() {
                result.strict_valid.insert(owner.clone());
            } else {
                result.failed.insert(owner.clone(), failures);
            }
        }
        // A source dependency must survive class-level selection as well. Cycles
        // survive together; invalid classes propagate removal to their callers.
        loop {
            let mut rejected = BTreeMap::new();
            for owner in &result.strict_valid {
                for dependency in &validations[owner].source_classes {
                    if !result.strict_valid.contains(dependency) {
                        rejected
                            .entry(owner.clone())
                            .or_insert_with(Vec::new)
                            .push(format!("required source class unavailable: {dependency}"));
                    }
                }
            }
            for (key, body) in &self.bodies {
                if !result.strict_valid.contains(&key.owner) {
                    continue;
                }
                for dependency in &body.dependencies {
                    if !self.external.contains(dependency)
                        && !result.strict_valid.contains(&dependency.owner)
                    {
                        rejected
                            .entry(key.owner.clone())
                            .or_insert_with(Vec::new)
                            .push(format!(
                                "source class dependency unavailable: {dependency:?}"
                            ));
                    }
                }
            }
            if rejected.is_empty() {
                break;
            }
            for (owner, reasons) in rejected {
                result.strict_valid.remove(&owner);
                result.failed.entry(owner).or_default().extend(reasons);
            }
        }
        result.selected = match mode {
            SourceSelection::AnalystAll => classes.clone(),
            SourceSelection::StrictValidClasses => result.strict_valid.clone(),
        };
        result
    }
}

/// Collect exact method edges from successfully emitted IR in its owning pool.
/// Field declarations and dispatch/exception/access checks belong to their
/// respective parent adapters and must already have succeeded at emission.
pub fn body_references(
    ir: &managed_ir::method::MethodIr<managed_ir::method::SsaForm>,
    dex: &DexFile,
) -> Result<BTreeSet<MethodKey>, String> {
    let mut result = BTreeSet::new();
    for instruction in ir.instructions().values() {
        if let managed_ir::insn::ManagedInsn::Invoke { method, .. } = instruction {
            result.insert(
                method_key(dex, method.index())
                    .ok_or_else(|| format!("invalid method reference {}", method.index()))?,
            );
        }
    }
    Ok(result)
}
