//! Metadata-backed member dependencies for honest Java subsets.
//!
//! This module never invents external dispatch flags or missing declarations.
use crate::dex_java::DexJavaResolver;
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType};
use dex_format::{
    DexFile,
    class::{ClassDef, EncodedMethod},
    encoded_value::EncodedValueKind,
};
use managed_ir::{
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
    ops::InvokeKind,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
};
use std::collections::{BTreeSet, HashMap, HashSet};

#[path = "java_external_metadata.rs"]
pub mod external_metadata;

#[path = "java_dependency_publication.rs"]
pub mod dependency_publication;
#[path = "java_source_closure.rs"]
pub mod source_closure;

/// Immutable genuine method declarations for one DEX pool.
///
/// Build once and share with class workers. A method-id reference alone never
/// constitutes a definition. Like `MemberResolver`, only the first class for
/// an owner is considered, and duplicate methods in that class are unresolved.
pub struct MemberIndex<'a> {
    dex: &'a DexFile,
    methods: HashMap<u32, Option<(&'a ClassDef, &'a EncodedMethod)>>,
}

impl<'a> MemberIndex<'a> {
    /// Index each selected declaring class once, retaining original metadata.
    pub fn new(dex: &'a DexFile) -> Self {
        let mut owners = HashSet::new();
        let mut methods = HashMap::new();
        for class in &dex.classes {
            if !owners.insert(class.class_idx) {
                continue;
            }
            let Some(data) = &class.class_data else {
                continue;
            };
            for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                let Ok(id) = dex.method_id(method.method_idx) else {
                    continue;
                };
                if u32::from(id.class_idx) != class.class_idx {
                    continue;
                }
                methods
                    .entry(method.method_idx)
                    .and_modify(|definition| *definition = None)
                    .or_insert(Some((class, method)));
            }
        }
        Self { dex, methods }
    }

    fn method_definition(&self, index: u32) -> Option<(&ClassDef, &EncodedMethod)> {
        self.methods.get(&index).copied().flatten()
    }

    /// Original body status, with the same metadata checks as `MemberResolver`.
    pub fn method_body_status(&self, index: u32) -> Result<MethodBodyStatus, String> {
        let (_, method) = self
            .method_definition(index)
            .ok_or("missing target definition")?;
        method_body_status(method)
    }

    /// A validated original native/abstract declaration; this grants no body
    /// publication permission and does not fabricate missing source bodies.
    pub fn bodyless_method_declaration(&self, index: u32) -> Result<String, String> {
        let status = self.method_body_status(index)?;
        let (class, method) = self
            .method_definition(index)
            .ok_or("missing target definition")?;
        bodyless_method_declaration(class, method.access_flags, status, || {
            method_metadata(
                self.dex,
                class,
                method.access_flags,
                MethodRef::new(index),
                || indexed_signature(self.dex, method),
            )
        })
    }
}

/// Resolver for genuine declarations in one DEX pool, including Throws metadata.
pub struct MemberResolver<'a> {
    /// The pool owning all reference indices.
    pub dex: &'a DexFile,
}
impl MemberResolver<'_> {
    /// Real declaration metadata, not inferred from a method-id reference.
    pub fn method_body_status(&self, index: u32) -> Result<MethodBodyStatus, String> {
        let (_, method) = self
            .method_definition_ref(index)
            .ok_or("missing target definition")?;
        method_body_status(method)
    }

    /// Only native/abstract methods can truthfully be emitted without a body.
    /// Abstract declarations require the original declaring class to be abstract.
    pub fn bodyless_method_declaration(&self, index: u32) -> Result<String, String> {
        let status = self.method_body_status(index)?;
        let (class, flags) = self
            .method_definition(index)
            .ok_or("missing target definition")?;
        bodyless_method_declaration(class, flags, status, || self.method(MethodRef::new(index)))
    }
    fn method_definition(&self, index: u32) -> Option<(&ClassDef, u32)> {
        self.method_definition_ref(index)
            .map(|(class, method)| (class, method.access_flags))
    }

    fn method_definition_ref(&self, index: u32) -> Option<(&ClassDef, &EncodedMethod)> {
        let owner = u32::from(self.dex.method_id(index).ok()?.class_idx);
        let class = self.dex.classes.iter().find(|c| c.class_idx == owner)?;
        let data = class.class_data.as_ref()?;
        let mut matches = data
            .direct_methods
            .iter()
            .chain(&data.virtual_methods)
            .filter(|m| m.method_idx == index);
        let method = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some((class, method))
    }

    /// Exact declared exception names. Missing external definitions are unknown.
    pub fn declared_throws(&self, index: u32) -> Option<Vec<String>> {
        let (class, _) = self.method_definition(index)?;
        self.declared_throws_for_class(class, index)
    }

    /// Read annotations from an already indexed declaration owner.
    pub fn declared_throws_for_class(&self, class: &ClassDef, index: u32) -> Option<Vec<String>> {
        let mut result = None;
        if let Some(directory) = &class.annotations {
            for member in directory
                .method_annotations
                .iter()
                .filter(|a| a.member_idx == index)
            {
                for item in &member.annotations.entries {
                    let annotation = &item.annotation;
                    let descriptor = self
                        .dex
                        .string(self.dex.type_id(annotation.type_idx).ok()?.descriptor_idx)
                        .ok()?;
                    if descriptor != "Ldalvik/annotation/Throws;" {
                        continue;
                    }
                    if result.is_some()
                        || annotation.elements.len() != 1
                        || self.dex.string(annotation.elements[0].name_idx).ok()? != "value"
                        || item.visibility != dex_format::annotations::Visibility::System
                    {
                        return None;
                    }
                    let EncodedValueKind::Array(values) = &annotation.elements[0].value.value
                    else {
                        return None;
                    };
                    let mut names = Vec::new();
                    for value in values {
                        let EncodedValueKind::Type(index) = value.value else {
                            return None;
                        };
                        names.push(self.class(ClassRef::new(index))?);
                    }
                    result = Some(names);
                }
            }
        }
        Some(result.unwrap_or_default())
    }

    fn field_definition(&self, index: u32) -> Option<(&ClassDef, u32, bool, usize)> {
        let owner = u32::from(self.dex.field_id(index).ok()?.class_idx);
        let class = self.dex.classes.iter().find(|c| c.class_idx == owner)?;
        let data = class.class_data.as_ref()?;
        let mut found = None;
        for (is_static, fields) in [(true, &data.static_fields), (false, &data.instance_fields)] {
            for (position, field) in fields
                .iter()
                .enumerate()
                .filter(|(_, f)| f.field_idx == index)
            {
                if found.is_some() || (field.access_flags & 8 != 0) != is_static {
                    return None;
                }
                found = Some((class, field.access_flags, is_static, position));
            }
        }
        found
    }

    /// A real field declaration without fabricated initialization. Final fields
    /// and explicit encoded initializers require a separate initialization layer.
    pub fn field_declaration(&self, index: u32) -> Result<String, String> {
        let (class, flags, is_static, position) = self
            .field_definition(index)
            .ok_or("unresolved field definition")?;
        if class.access_flags & 0x200 != 0 {
            return Err("interface field initializer unavailable".into());
        }
        if flags & !(1 | 2 | 4 | 8 | 0x40 | 0x80 | 0x1000) != 0 {
            return Err("unsupported field flags (including final)".into());
        }
        if is_static
            && class
                .static_values
                .as_ref()
                .is_some_and(|v| position < v.len())
        {
            return Err("encoded field initializer requires preservation".into());
        }
        let field = self
            .field(FieldRef::new(index))
            .ok_or("invalid field metadata")?;
        let visibility = visibility(flags)?;
        Ok(format!(
            "{visibility}{}{}{}{} {};",
            if is_static { "static " } else { "" },
            if flags & 0x40 != 0 { "volatile " } else { "" },
            if flags & 0x80 != 0 { "transient " } else { "" },
            type_source(&field.ty),
            field.name
        ))
    }

    /// Genuine declaration for a partial dependency or an atomic initialization
    /// plan. No initializer is fabricated; callers must separately prove and
    /// retain encoded-prefix, clinit and constructor ordering before claiming
    /// complete output. In particular a blank final is not executable proof.
    pub fn field_declaration_without_initialization(&self, index: u32) -> Result<String, String> {
        let (class, flags, is_static, _) = self
            .field_definition(index)
            .ok_or("unresolved field definition")?;
        if class.access_flags & 0x200 != 0 {
            return Err("interface field initializer unavailable".into());
        }
        if flags & !(1 | 2 | 4 | 8 | 0x10 | 0x40 | 0x80 | 0x1000) != 0 || flags & 0x50 == 0x50 {
            return Err("unsupported field flags".into());
        }
        let field = self
            .field(FieldRef::new(index))
            .ok_or("invalid field metadata")?;
        Ok(format!(
            "{}{}{}{}{}{} {};",
            visibility(flags)?,
            if is_static { "static " } else { "" },
            if flags & 0x10 != 0 { "final " } else { "" },
            if flags & 0x40 != 0 { "volatile " } else { "" },
            if flags & 0x80 != 0 { "transient " } else { "" },
            type_source(&field.ty),
            field.name
        ))
    }
}

impl JavaResolver for MemberResolver<'_> {
    fn class(&self, r: ClassRef) -> Option<String> {
        DexJavaResolver { dex: self.dex }.class(r)
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        DexJavaResolver { dex: self.dex }.string(r)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let (class, flags) = self.method_definition(r.index())?;
        method_metadata(self.dex, class, flags, r, || {
            DexJavaResolver { dex: self.dex }.method(r)
        })
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        self.field_definition(r.index())?;
        let field = DexJavaResolver { dex: self.dex }.field(r)?;
        if field.ty == JavaType::Void
            || backend_java::sanitize_identifier(&field.name) != field.name
        {
            return None;
        }
        Some(field)
    }
}

/// Genuine publication status of an original method definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MethodBodyStatus {
    /// Original code is available; this is not proof of successful publication.
    SourceBody,
    /// Original body has not been published.
    MissingBody,
    /// Genuine native declaration.
    Native,
    /// Genuine abstract declaration.
    Abstract,
}

/// Required declaration closure, independent of semantic caller emission.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MethodClosureReport {
    /// Required methods with emitted bodies.
    pub emitted_bodies: BTreeSet<u32>,
    /// Required genuine native or abstract declarations.
    pub bodyless_declarations: BTreeSet<u32>,
    /// Exact unresolved dependencies and reasons.
    pub unmet: std::collections::BTreeMap<u32, String>,
}

/// Dependencies that must exist in the emitted class. Indices are DEX-local.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MemberRequirements {
    /// Genuine field indices whose declarations must be emitted.
    pub fields: BTreeSet<u32>,
    /// Genuine target method indices requiring emitted declarations. The legacy
    /// check_closure requires bodies; method_closure also accepts real native/abstract declarations.
    pub methods: BTreeSet<u32>,
}
impl MemberRequirements {
    /// Declaration closure is independent of caller semantic-body completeness.
    /// Bodyless declarations must have actually been emitted by the caller.
    pub fn method_closure(
        &self,
        resolver: &MemberResolver<'_>,
        emitted: &BTreeSet<u32>,
        bodyless: &BTreeSet<u32>,
    ) -> MethodClosureReport {
        self.method_closure_with(
            emitted,
            bodyless,
            |index| resolver.method_body_status(index),
            |index| resolver.bodyless_method_declaration(index).is_ok(),
        )
    }

    /// Declaration closure using a shared DEX index. Actual emitted-body and
    /// emitted-bodyless sets remain mandatory, exactly as for `method_closure`.
    pub fn method_closure_indexed(
        &self,
        index: &MemberIndex<'_>,
        emitted: &BTreeSet<u32>,
        bodyless: &BTreeSet<u32>,
    ) -> MethodClosureReport {
        self.method_closure_with(
            emitted,
            bodyless,
            |method| index.method_body_status(method),
            |method| index.bodyless_method_declaration(method).is_ok(),
        )
    }

    fn method_closure_with(
        &self,
        emitted: &BTreeSet<u32>,
        bodyless: &BTreeSet<u32>,
        status: impl Fn(u32) -> Result<MethodBodyStatus, String>,
        declaration_available: impl Fn(u32) -> bool,
    ) -> MethodClosureReport {
        let mut report = MethodClosureReport::default();
        for &index in &self.methods {
            match status(index) {
                Ok(MethodBodyStatus::SourceBody) if emitted.contains(&index) => {
                    report.emitted_bodies.insert(index);
                }
                Ok(MethodBodyStatus::Native | MethodBodyStatus::Abstract)
                    if bodyless.contains(&index) && declaration_available(index) =>
                {
                    report.bodyless_declarations.insert(index);
                }
                Ok(MethodBodyStatus::Native | MethodBodyStatus::Abstract) => {
                    report.unmet.insert(
                        index,
                        "required genuine bodyless declaration unavailable".into(),
                    );
                }
                Ok(MethodBodyStatus::MissingBody) => {
                    report
                        .unmet
                        .insert(index, "source declared signature has missing body".into());
                }
                Ok(MethodBodyStatus::SourceBody) => {
                    report.unmet.insert(
                        index,
                        "source declared signature absent from emitted subset".into(),
                    );
                }
                Err(reason) => {
                    report.unmet.insert(index, reason);
                }
            }
        }
        report
    }
    /// Check after body selection; use repeatedly while pruning callers until
    /// the retained body set reaches a fixed point (mutual recursion is allowed).
    pub fn check_closure(
        &self,
        emitted_methods: &BTreeSet<u32>,
        declared_fields: &BTreeSet<u32>,
    ) -> Result<(), String> {
        if !self.methods.is_subset(emitted_methods) {
            return Err("required complete method body unavailable".into());
        }
        if !self.fields.is_subset(declared_fields) {
            return Err("required genuine field declaration unavailable".into());
        }
        Ok(())
    }
}

/// Gather a conservative same-class member closure. Cross-class dependencies
/// need an independently verified source/classpath closure and are rejected here.
pub fn requirements(
    ir: &MethodIr<SsaForm>,
    caller: u32,
    resolver: &MemberResolver<'_>,
) -> Result<MemberRequirements, String> {
    let signature = resolver
        .method(MethodRef::new(caller))
        .ok_or("unresolved caller")?;
    let (class, _) = resolver
        .method_definition(caller)
        .ok_or("unresolved caller definition")?;
    if class.access_flags & (0x200 | 0x2000 | 0x4000) != 0 {
        return Err("unsupported declaring class kind".into());
    }
    let mut result = MemberRequirements::default();
    for instruction in ir.instructions().values() {
        match instruction {
            ManagedInsn::Invoke { kind, method, .. } => {
                let target = resolver
                    .method(*method)
                    .ok_or("unresolved invocation metadata")?;
                let (_, flags) = resolver
                    .method_definition(method.index())
                    .ok_or("missing target definition")?;
                if target.owner != signature.owner {
                    return Err("invocation requires external class closure".into());
                }
                if !matches!(kind, InvokeKind::Static | InvokeKind::Virtual)
                    || (*kind == InvokeKind::Static) != target.is_static
                    || (*kind == InvokeKind::Virtual && flags & 2 != 0)
                {
                    return Err("unsupported invocation dispatch".into());
                }
                if target.throws.iter().any(|t| !signature.throws.contains(t)) {
                    return Err("callee exceptions not declared by caller".into());
                }
                result.methods.insert(method.index());
            }
            ManagedInsn::FieldGet { field, object, .. }
            | ManagedInsn::FieldPut { field, object, .. } => {
                let target = resolver.field(*field).ok_or("unresolved field metadata")?;
                if target.owner != signature.owner {
                    return Err("field requires external class closure".into());
                }
                if target.is_static != object.is_none() {
                    return Err("field staticness mismatch".into());
                }
                resolver.field_declaration(field.index())?;
                result.fields.insert(field.index());
            }
            _ => {}
        }
    }
    Ok(result)
}

fn method_body_status(method: &EncodedMethod) -> Result<MethodBodyStatus, String> {
    let bodyless = method.access_flags & (0x100 | 0x400);
    if bodyless == (0x100 | 0x400) || (bodyless != 0 && method.code.is_some()) {
        return Err("inconsistent native/abstract method body metadata".into());
    }
    Ok(match bodyless {
        0x100 => MethodBodyStatus::Native,
        0x400 => MethodBodyStatus::Abstract,
        _ if method.code.is_some() => MethodBodyStatus::SourceBody,
        _ => MethodBodyStatus::MissingBody,
    })
}

fn indexed_signature(dex: &DexFile, method: &EncodedMethod) -> Option<JavaMethod> {
    let id = dex.method_id(method.method_idx).ok()?;
    let prototype = dex.proto_id(u32::from(id.proto_idx)).ok()?;
    let ty = |index| {
        crate::dex_java::descriptor_type(dex.string(dex.type_id(index).ok()?.descriptor_idx).ok()?)
    };
    Some(JavaMethod {
        owner: DexJavaResolver { dex }.class(ClassRef::new(u32::from(id.class_idx)))?,
        name: dex.string(id.name_idx).ok()?.into(),
        is_static: method.access_flags & 8 != 0,
        return_type: ty(prototype.return_type_idx)?,
        parameters: prototype
            .parameters
            .as_ref()
            .map(|parameters| {
                parameters
                    .types
                    .iter()
                    .map(|index| ty(u32::from(*index)))
                    .collect::<Option<Vec<_>>>()
            })
            .unwrap_or(Some(vec![]))?,
        throws: vec![],
    })
}

fn method_metadata(
    dex: &DexFile,
    class: &ClassDef,
    flags: u32,
    r: MethodRef,
    signature: impl FnOnce() -> Option<JavaMethod>,
) -> Option<JavaMethod> {
    visibility(flags).ok()?;
    let mut method = signature()?;
    if backend_java::sanitize_identifier(&method.name) != method.name {
        return None;
    }
    if method.parameters.contains(&JavaType::Void) {
        return None;
    }
    method.throws = MemberResolver { dex }.declared_throws_for_class(class, r.index())?;
    Some(method)
}

fn bodyless_method_declaration(
    class: &ClassDef,
    flags: u32,
    status: MethodBodyStatus,
    signature: impl FnOnce() -> Option<JavaMethod>,
) -> Result<String, String> {
    if !matches!(
        status,
        MethodBodyStatus::Native | MethodBodyStatus::Abstract
    ) {
        return Err("source declared signature requires semantic body".into());
    }
    if flags & !(7 | 8 | 0x10 | 0x20 | 0x100 | 0x400 | 0x1000 | 0x20000) != 0
        || (status == MethodBodyStatus::Abstract
            && (class.access_flags & 0x400 == 0
                || class.access_flags & 0x10 != 0
                || flags & (2 | 8 | 0x10 | 0x20 | 0x20000) != 0))
        || (status == MethodBodyStatus::Native && class.access_flags & 0x200 != 0)
    {
        return Err("unsupported bodyless declaration flags".into());
    }
    let signature = signature().ok_or("unresolved invocation metadata")?;
    if signature.name.starts_with('<') {
        return Err("constructor requires semantic body".into());
    }
    let parameters = signature
        .parameters
        .iter()
        .enumerate()
        .map(|(i, ty)| format!("{} p{i}", type_source(ty)))
        .collect::<Vec<_>>()
        .join(", ");
    let throws = if signature.throws.is_empty() {
        String::new()
    } else {
        format!(" throws {}", signature.throws.join(", "))
    };
    Ok(format!(
        "{}{}{}{}{}{} {}({parameters}){throws};",
        visibility(flags)?,
        if flags & 8 != 0 { "static " } else { "" },
        if flags & 0x10 != 0 { "final " } else { "" },
        if flags & (0x20 | 0x20000) != 0 {
            "synchronized "
        } else {
            ""
        },
        if status == MethodBodyStatus::Native {
            "native "
        } else {
            "abstract "
        },
        type_source(&signature.return_type),
        signature.name
    ))
}

fn visibility(flags: u32) -> Result<&'static str, String> {
    Ok(match flags & 7 {
        0 => "",
        1 => "public ",
        2 => "private ",
        4 => "protected ",
        _ => return Err("conflicting visibility flags".into()),
    })
}

/// Render an already validated descriptor type, retaining binary `$` names.
pub fn type_source(ty: &JavaType) -> String {
    match ty {
        JavaType::Void => "void".into(),
        JavaType::Boolean => "boolean".into(),
        JavaType::Byte => "byte".into(),
        JavaType::Short => "short".into(),
        JavaType::Char => "char".into(),
        JavaType::Int => "int".into(),
        JavaType::Long => "long".into(),
        JavaType::Float => "float".into(),
        JavaType::Double => "double".into(),
        JavaType::Class(n) => n.clone(),
        JavaType::Array(t) => format!("{}[]", type_source(t)),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod method_publication_tests {
    use super::*;

    #[test]
    fn samsung_required_complete_body_reporter() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
        let files = dex_format::apk::parse_apk_path(&path).unwrap();
        let mut count = 0;
        for (pool, dex) in files.iter().enumerate() {
            let resolver = MemberResolver { dex };
            for class in &dex.classes {
                let Some(data) = &class.class_data else {
                    continue;
                };
                for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                    let Some(code) = &method.code else { continue };
                    let Some(signature) = resolver.method(MethodRef::new(method.method_idx)) else {
                        continue;
                    };
                    let Ok(ssa) = dex_decompiler::DexPasses::run_ssa_with(
                        code,
                        &dex_decompiler::DexTypes::new(dex, Some(method.method_idx)),
                    ) else {
                        continue;
                    };
                    let Ok(deps) = requirements(ssa.ir(), method.method_idx, &resolver) else {
                        continue;
                    };
                    if deps.methods.is_empty() {
                        continue;
                    }
                    let Ok(bindings) = crate::dex_java::parameter_bindings(
                        ssa.ir(),
                        &signature,
                        code.registers_size,
                        code.ins_size,
                    ) else {
                        continue;
                    };
                    let backend_java::SourceResult::Complete { source, .. } =
                        backend_java::emit_method(ssa.ir(), &signature, &bindings, &resolver)
                    else {
                        continue;
                    };
                    let emitted = BTreeSet::from([method.method_idx]);
                    if deps.check_closure(&emitted, &deps.fields).err().as_deref()
                        != Some("required complete method body unavailable")
                    {
                        continue;
                    }
                    let report = deps.method_closure(&resolver, &emitted, &BTreeSet::new());
                    println!(
                        "pool={pool} caller={} {}.{}({:?})->{:?} semantic_body=complete source={source:?} closure={report:?}",
                        method.method_idx,
                        signature.owner,
                        signature.name,
                        signature.parameters,
                        signature.return_type
                    );
                    for index in &deps.methods {
                        let (_, flags) = resolver.method_definition(*index).unwrap();
                        println!(
                            "callee={index} flags={flags:#x} body={:?} metadata={:?}",
                            resolver.method_body_status(*index),
                            resolver.method(MethodRef::new(*index))
                        );
                    }
                    count += 1;
                    if count == 3 {
                        println!("semantic_complete=3 legacy_dependency_rejected=3");
                        return;
                    }
                }
            }
        }
        panic!("expected three real Samsung dependency-only rejections; found {count}");
    }

    #[test]
    fn missing_native_recursive_and_constructor_reasons() {
        let mut dex = DexFile::from_bytes(include_bytes!(
            "../../../tests/corpus/dex/clean/classes.dex"
        ))
        .unwrap();
        let (ci, mi, index) = dex
            .classes
            .iter()
            .enumerate()
            .find_map(|(ci, c)| {
                c.class_data
                    .as_ref()?
                    .direct_methods
                    .iter()
                    .enumerate()
                    .find_map(|(mi, m)| {
                        let name = dex
                            .string(dex.method_id(m.method_idx).ok()?.name_idx)
                            .ok()?;
                        (!name.starts_with('<')).then_some((ci, mi, m.method_idx))
                    })
            })
            .unwrap();
        let deps = MemberRequirements {
            methods: BTreeSet::from([index]),
            ..Default::default()
        };
        let resolver = MemberResolver { dex: &dex };
        assert!(
            deps.method_closure(&resolver, &BTreeSet::from([index]), &BTreeSet::new())
                .unmet
                .is_empty()
        );
        assert_eq!(
            deps.method_closure(&resolver, &BTreeSet::new(), &BTreeSet::new())
                .unmet[&index],
            "source declared signature absent from emitted subset"
        );
        let method = &mut dex.classes[ci].class_data.as_mut().unwrap().direct_methods[mi];
        method.code = None;
        method.access_flags = 9;
        assert_eq!(
            deps.method_closure(
                &MemberResolver { dex: &dex },
                &BTreeSet::from([index]),
                &BTreeSet::new()
            )
            .unmet[&index],
            "source declared signature has missing body"
        );
        dex.classes[ci].class_data.as_mut().unwrap().direct_methods[mi].access_flags = 0x109;
        let resolver = MemberResolver { dex: &dex };
        assert!(
            deps.method_closure(&resolver, &BTreeSet::new(), &BTreeSet::from([index]))
                .unmet
                .is_empty()
        );
        let declaration = resolver.bodyless_method_declaration(index).unwrap();
        assert!(
            declaration.contains("native ")
                && declaration.ends_with(';')
                && !declaration.contains('{')
        );
        let constructor = dex
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|d| &d.direct_methods)
            .find(|m| {
                dex.string(dex.method_id(m.method_idx).unwrap().name_idx)
                    .unwrap()
                    == "<init>"
            })
            .unwrap();
        assert_eq!(
            resolver
                .bodyless_method_declaration(constructor.method_idx)
                .unwrap_err(),
            "source declared signature requires semantic body"
        );
    }
}
