//! Exact declaration defaults and source-published enum-constant dependencies.
//! Retained orphan metadata never becomes an invented annotation element.
use super::{Error, annotation_defaults_inner, annotation_value, descriptor, ty};
use crate::java_class_kinds::{
    abstract_method_declaration, enum_initialization::EnumInitializationPlan,
};
use crate::output::{GeneratedSource, OutputError, SourceSink, source_path_from_descriptor};
use dex_format::{
    DexFile,
    class::ClassDef,
    encoded_value::{AnnotationElement, EncodedValue, EncodedValueKind as Value},
};
use std::collections::{BTreeMap, BTreeSet};

/// Quarantined expression and exact enum requirements, not a publication receipt.
#[derive(Clone, Debug)]
pub struct DeferredDefault {
    owner: String,
    expression: String,
    enum_fields: BTreeSet<(String, String)>,
}
impl DeferredDefault {
    /// Source planning only. Keep this token with its pending annotation unit;
    /// delivery must use `PublishedEnumConstants::emit_annotation`.
    pub fn expression(&self) -> &str {
        &self.expression
    }
}

/// Whether this annotation requires ordered, receipt-guarded default planning.
/// Malformed metadata is still rejected by the full declaration/default validators.
pub fn requires_deferred_defaults(dex: &DexFile, class: &ClassDef) -> bool {
    fn has_enum(value: &EncodedValue) -> bool {
        match &value.value {
            Value::Enum(_) => true,
            Value::Array(v) => v.iter().any(has_enum),
            _ => false,
        }
    }
    let names: BTreeSet<_> = class
        .class_data
        .iter()
        .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
        .filter_map(|m| {
            dex.method_id(m.method_idx)
                .ok()
                .and_then(|m| dex.string(m.name_idx).ok())
        })
        .collect();
    class
        .annotations
        .iter()
        .flat_map(|a| a.class_annotations.iter())
        .flat_map(|a| &a.entries)
        .filter(|a| {
            descriptor(dex, a.annotation.type_idx).ok()
                == Some("Ldalvik/annotation/AnnotationDefault;")
        })
        .flat_map(|a| &a.annotation.elements)
        .any(|wrapper| {
            if let Value::Annotation(defaults) = &wrapper.value.value {
                defaults.elements.iter().any(|e| {
                    has_enum(&e.value)
                        || dex
                            .string(e.name_idx)
                            .ok()
                            .is_some_and(|n| !names.contains(n))
                })
            } else {
                false
            }
        })
}

/// Declared-element recovery, explicitly distinct from complete wrapper equivalence.
#[derive(Clone, Debug, PartialEq)]
pub struct DeclaredDefaultsPlan {
    /// Defaults for exact genuine original method definitions only.
    pub by_method: BTreeMap<u32, String>,
    /// Original entries whose names have no declared method; values/spans unchanged.
    pub retained_orphans: Vec<AnnotationElement>,
    /// Complete original annotation metadata; this remains provenance, not Java source.
    pub original_annotations: Option<dex_format::annotations::AnnotationsDirectory>,
}

/// Validate all genuine annotation declarations and every original wrapper entry.
/// Orphan entries are retained, not published; missing defaults remain genuinely absent.
/// This plan attests declaration/default recovery, never full metadata equivalence.
pub fn declared_defaults_with(
    dex: &DexFile,
    class: &ClassDef,
    name: &impl Fn(u32) -> Result<String, Error>,
) -> Result<DeclaredDefaultsPlan, Error> {
    if crate::java_class_kinds::class_kind(class.access_flags)?
        != crate::java_class_kinds::ClassKind::Annotation
    {
        return Err(Error::InvalidAnnotationElement);
    }
    let mut declaration = class.clone();
    declaration.annotations = None;
    let data = class
        .class_data
        .as_ref()
        .ok_or(Error::AnnotationDefaultsUnavailable)?;
    let mut names = BTreeSet::new();
    for method in data.direct_methods.iter().chain(&data.virtual_methods) {
        abstract_method_declaration(dex, &declaration, method)?;
        let raw = dex
            .method_id(method.method_idx)
            .map_err(|_| Error::InvalidMetadata)?;
        if !names.insert(
            dex.string(raw.name_idx)
                .map_err(|_| Error::InvalidMetadata)?,
        ) {
            return Err(Error::AnnotationDefaultsUnavailable);
        }
    }
    let mut retained_orphans = Vec::new();
    let by_method = annotation_defaults_inner(dex, class, name, Some(&mut retained_orphans))?;
    Ok(DeclaredDefaultsPlan {
        by_method,
        retained_orphans,
        original_annotations: class.annotations.clone(),
    })
}

// An orphan has no return descriptor to type-check. Accept only bounded,
// structurally representable primitive/String values and flat arrays thereof.
// Refuse type/enum/nested annotation values rather than inventing dependency semantics.
pub(crate) fn validate_orphan_value(
    dex: &DexFile,
    value: &EncodedValue,
    depth: usize,
) -> Result<(), Error> {
    if depth > 1 {
        return Err(Error::InvalidEncodedValue);
    }
    match &value.value {
        Value::Byte(_)
        | Value::Short(_)
        | Value::Char(_)
        | Value::Int(_)
        | Value::Long(_)
        | Value::Boolean(_) => Ok(()),
        Value::Float(bits) if bits.is_finite() => Ok(()),
        Value::Double(bits) if bits.is_finite() => Ok(()),
        Value::String(index) => {
            dex.string(*index).map_err(|_| Error::InvalidMetadata)?;
            Ok(())
        }
        Value::Array(values) if depth == 0 => {
            for value in values {
                validate_orphan_value(dex, value, depth + 1)?;
            }
            Ok(())
        }
        _ => Err(Error::InvalidEncodedValue),
    }
}

/// Prepared proofs remain unusable until the owning source unit is actually delivered.
#[derive(Default)]
pub struct PublishedEnumConstants {
    prepared: BTreeMap<(String, String), String>,
    committed: BTreeSet<String>,
    source_bodies: BTreeMap<String, (String, String)>,
}
impl PublishedEnumConstants {
    /// Register an existing body/declaration/dependency-validated enum plan.
    /// The parent must already validate every remaining member and initializer suffix;
    /// this method adds exact field, constructor-modifier and runtime-name checks.
    pub fn prepare(
        &mut self,
        dex: &DexFile,
        class: &ClassDef,
        plan: &EnumInitializationPlan,
        source_owner: &str,
        validated_source_body: &str,
    ) -> Result<(), Error> {
        let owner = descriptor(dex, class.class_idx)?.to_owned();
        let data = class.class_data.as_ref().ok_or(Error::InvalidMetadata)?;
        if class.access_flags & 0x4000 == 0
            || plan.constants.is_empty()
            || source_owner.is_empty()
            || validated_source_body.is_empty()
            || self.source_bodies.contains_key(&owner)
        {
            return Err(Error::EnumConstructorsUnavailable);
        }
        let actual_constructors = data
            .direct_methods
            .iter()
            .filter(|m| {
                dex.method_id(m.method_idx)
                    .ok()
                    .is_some_and(|raw| dex.string(raw.name_idx).ok() == Some("<init>"))
            })
            .map(|m| m.method_idx)
            .collect::<BTreeSet<_>>();
        let planned_constructors = plan
            .absorbed_constructors
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if actual_constructors.is_empty()
            || actual_constructors != planned_constructors
            || planned_constructors.len() != plan.absorbed_constructors.len()
        {
            return Err(Error::EnumConstructorsUnavailable);
        }
        let actual_constants = data
            .static_fields
            .iter()
            .filter(|f| f.access_flags & 0x4000 != 0)
            .map(|f| f.field_idx)
            .collect::<BTreeSet<_>>();
        let planned_constants = plan
            .constants
            .iter()
            .map(|f| f.field_idx)
            .collect::<BTreeSet<_>>();
        if actual_constants != planned_constants || planned_constants.len() != plan.constants.len()
        {
            return Err(Error::EnumConstructorsUnavailable);
        }
        for index in &plan.absorbed_constructors {
            let method = data
                .direct_methods
                .iter()
                .find(|m| m.method_idx == *index)
                .ok_or(Error::InvalidMetadata)?;
            if method.access_flags & 7 != 2 {
                return Err(Error::InvalidFlags);
            }
        }
        let mut pending = BTreeMap::new();
        for (ordinal, constant) in plan.constants.iter().enumerate() {
            let field = dex
                .field_id(constant.field_idx)
                .map_err(|_| Error::InvalidMetadata)?;
            let declared = data
                .static_fields
                .iter()
                .find(|f| f.field_idx == constant.field_idx)
                .ok_or(Error::InvalidMetadata)?;
            let field_name = dex
                .string(field.name_idx)
                .map_err(|_| Error::InvalidMetadata)?;
            if declared.access_flags != 0x4019
                || u32::from(field.class_idx) != class.class_idx
                || u32::from(field.type_idx) != class.class_idx
                || field_name != constant.name
                || constant.ordinal != ordinal
            {
                return Err(Error::EnumConstructorsUnavailable);
            }
            let key = (owner.clone(), field_name.to_owned());
            if pending
                .insert(key.clone(), source_owner.to_owned())
                .is_some()
                || self.prepared.contains_key(&key)
            {
                return Err(Error::InvalidMetadata);
            }
        }
        self.prepared.extend(pending);
        self.source_bodies.insert(
            owner,
            (source_owner.to_owned(), validated_source_body.to_owned()),
        );
        Ok(())
    }

    /// Whether a genuine validated owner has a pending plan (never a receipt).
    pub fn prepared_owner(&self, descriptor: &str) -> bool {
        self.source_bodies.contains_key(descriptor)
    }

    /// Deliver the exact validated enum unit to the caller's actual outer sink.
    /// Only successful delivery creates a receipt. Internal worker collectors
    /// must never call this function. No public manual-commit API exists.
    pub fn emit_enum(
        &mut self,
        source: &GeneratedSource,
        sink: &mut impl SourceSink,
    ) -> Result<(), OutputError> {
        let owner = &source.provenance.descriptor;
        let (source_owner, body) = self
            .source_bodies
            .get(owner)
            .ok_or_else(|| OutputError::InvalidDescriptor("enum source plan unavailable".into()))?;
        let expected_path =
            source_path_from_descriptor(&format!("L{};", source_owner.replace('.', "/")))?;
        let package = source_owner
            .rsplit_once('.')
            .map(|(p, _)| format!("package {p};\n\n"))
            .unwrap_or_default();
        let prefix = format!(
            "// Proven enum initialization and synthetic method equivalence.\n{package}{body}"
        );
        let text = source
            .text
            .strip_prefix("// Required Java source release: 25\n")
            .unwrap_or(&source.text);
        let tail = text.strip_prefix(&prefix).ok_or_else(|| {
            OutputError::InvalidDescriptor("enum source differs from validated plan".into())
        })?;
        if source.relative_path != expected_path
            || tail.contains("\\u")
            || tail
                .lines()
                .any(|l| !l.trim().is_empty() && !l.starts_with("//"))
        {
            return Err(OutputError::InvalidDescriptor(
                "enum source identity/body mismatch".into(),
            ));
        }
        sink.emit(source)?;
        self.committed.insert(owner.clone());
        Ok(())
    }

    /// Require every exact enum dependency token before actual annotation delivery.
    /// Prepared or filtered owners cannot satisfy these guards; failed sinks return Err.
    pub fn emit_annotation(
        &self,
        source: &GeneratedSource,
        requirements: &[DeferredDefault],
        sink: &mut impl SourceSink,
    ) -> Result<(), OutputError> {
        for token in requirements {
            if token.owner != source.provenance.descriptor
                || token.enum_fields.iter().any(|(owner, name)| {
                    !self.committed.contains(owner)
                        || !self.prepared.contains_key(&(owner.clone(), name.clone()))
                })
            {
                return Err(OutputError::InvalidDescriptor(
                    "annotation enum publication receipt unavailable".into(),
                ));
            }
        }
        sink.emit(source)
    }

    /// Resolve a DEX-local encoded_enum operand to its proven source constant.
    /// Class aliases may change; original constant name/Enum.name may never change.
    pub fn source(
        &self,
        dex: &DexFile,
        index: u32,
        name: &impl Fn(u32) -> Result<String, Error>,
    ) -> Result<String, Error> {
        let field = dex.field_id(index).map_err(|_| Error::InvalidMetadata)?;
        let owner = descriptor(dex, u32::from(field.class_idx))?;
        if !self.committed.contains(owner) {
            return Err(Error::AnnotationDefaultsUnavailable);
        }
        self.prepared_source(dex, index, name)
    }

    fn prepared_source(
        &self,
        dex: &DexFile,
        index: u32,
        name: &impl Fn(u32) -> Result<String, Error>,
    ) -> Result<String, Error> {
        let field = dex.field_id(index).map_err(|_| Error::InvalidMetadata)?;
        let owner = descriptor(dex, u32::from(field.class_idx))?;
        let member = dex
            .string(field.name_idx)
            .map_err(|_| Error::InvalidMetadata)?;
        let source_owner = self
            .prepared
            .get(&(owner.to_owned(), member.to_owned()))
            .ok_or(Error::AnnotationDefaultsUnavailable)?;
        if name(u32::from(field.class_idx))? != *source_owner {
            return Err(Error::InvalidMetadata);
        }
        Ok(format!("{source_owner}.{member}"))
    }

    /// Validate enum dependencies recursively for one exact declared default.
    /// Existing typed rendering checks descriptor identity and genuine enum fields.
    pub fn validate_value(
        &self,
        dex: &DexFile,
        value: &EncodedValue,
        expected: &backend_java::JavaType,
        name: &impl Fn(u32) -> Result<String, Error>,
    ) -> Result<String, Error> {
        let rendered = annotation_value(dex, value, expected, name, 0)?;
        fn check(
            registry: &PublishedEnumConstants,
            dex: &DexFile,
            value: &EncodedValue,
            name: &impl Fn(u32) -> Result<String, Error>,
        ) -> Result<(), Error> {
            match &value.value {
                Value::Enum(index) => {
                    registry.source(dex, *index, name)?;
                }
                Value::Array(values) => {
                    for value in values {
                        check(registry, dex, value, name)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        check(self, dex, value, name)?;
        Ok(rendered)
    }

    /// Validate an exact original method's encoded default; do not add dependency
    /// edges from unrelated sibling elements or retained orphan metadata.
    pub fn prepare_method_default(
        &self,
        dex: &DexFile,
        class: &ClassDef,
        method_index: u32,
        name: &impl Fn(u32) -> Result<String, Error>,
    ) -> Result<Option<DeferredDefault>, Error> {
        let plan = declared_defaults_with(dex, class, name)?;
        let method = dex
            .method_id(method_index)
            .map_err(|_| Error::InvalidMetadata)?;
        if u32::from(method.class_idx) != class.class_idx
            || !class.class_data.as_ref().is_some_and(|d| {
                d.direct_methods
                    .iter()
                    .chain(&d.virtual_methods)
                    .any(|m| m.method_idx == method_index)
            })
        {
            return Err(Error::InvalidMetadata);
        }
        if !plan.by_method.contains_key(&method_index) {
            return Ok(None);
        }
        let raw_name = dex
            .string(method.name_idx)
            .map_err(|_| Error::InvalidMetadata)?;
        let ret = ty(
            dex,
            dex.proto_id(u32::from(method.proto_idx))
                .map_err(|_| Error::InvalidMetadata)?
                .return_type_idx,
        )?;
        for annotation in class
            .annotations
            .as_ref()
            .and_then(|a| a.class_annotations.as_ref())
            .into_iter()
            .flat_map(|a| &a.entries)
        {
            if descriptor(dex, annotation.annotation.type_idx)?
                != "Ldalvik/annotation/AnnotationDefault;"
            {
                continue;
            }
            for wrapper in &annotation.annotation.elements {
                if let Value::Annotation(defaults) = &wrapper.value.value {
                    for element in &defaults.elements {
                        if dex
                            .string(element.name_idx)
                            .map_err(|_| Error::InvalidMetadata)?
                            == raw_name
                        {
                            let expression = annotation_value(dex, &element.value, &ret, name, 0)?;
                            let mut enum_fields = BTreeSet::new();
                            fn collect(
                                registry: &PublishedEnumConstants,
                                dex: &DexFile,
                                value: &EncodedValue,
                                name: &impl Fn(u32) -> Result<String, Error>,
                                out: &mut BTreeSet<(String, String)>,
                            ) -> Result<(), Error> {
                                match &value.value {
                                    Value::Enum(index) => {
                                        registry.prepared_source(dex, *index, name)?;
                                        let f = dex
                                            .field_id(*index)
                                            .map_err(|_| Error::InvalidMetadata)?;
                                        out.insert((
                                            descriptor(dex, u32::from(f.class_idx))?.to_owned(),
                                            dex.string(f.name_idx)
                                                .map_err(|_| Error::InvalidMetadata)?
                                                .to_owned(),
                                        ));
                                    }
                                    Value::Array(v) => {
                                        for value in v {
                                            collect(registry, dex, value, name, out)?;
                                        }
                                    }
                                    _ => {}
                                }
                                Ok(())
                            }
                            collect(self, dex, &element.value, name, &mut enum_fields)?;
                            return Ok(Some(DeferredDefault {
                                owner: descriptor(dex, class.class_idx)?.to_owned(),
                                expression,
                                enum_fields,
                            }));
                        }
                    }
                }
            }
        }
        Err(Error::AnnotationDefaultsUnavailable)
    }
    /// Resolve only defaults whose exact enum owners reached the actual outer sink.
    pub fn method_default(
        &self,
        dex: &DexFile,
        class: &ClassDef,
        method_index: u32,
        name: &impl Fn(u32) -> Result<String, Error>,
    ) -> Result<Option<String>, Error> {
        let Some(token) = self.prepare_method_default(dex, class, method_index, name)? else {
            return Ok(None);
        };
        if token
            .enum_fields
            .iter()
            .any(|(owner, _)| !self.committed.contains(owner))
        {
            return Err(Error::AnnotationDefaultsUnavailable);
        }
        Ok(Some(token.expression))
    }
}
