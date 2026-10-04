//! Genuine zero-member annotation declarations and checked platform metadata.
//! This recovers declarations, never bodies or unrepresented annotation semantics.
use crate::{
    java_aliases::{FieldKey, MethodKey},
    java_class_kinds::{ClassKind, DeclarationUnsupported, class_declaration_with, class_kind},
    java_external_metadata::ExternalMetadata,
};
use dex_format::{
    DexFile,
    annotations::{AnnotationItem, AnnotationsDirectory, Visibility},
    class::ClassDef,
    encoded_value::{EncodedValue, EncodedValueKind as Value},
};
use std::collections::BTreeSet;

/// A bounded reason preventing marker-declaration publication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkerRequirement {
    /// The input is not a genuine, zero-member annotation definition.
    NotMarker,
    /// The original Java header cannot be represented without changing flags.
    Header(DeclarationUnsupported),
    /// Referenced raw metadata is malformed, duplicated or contradictory.
    InvalidMetadata,
    /// Required standard-platform declaration evidence is unavailable/shadowed.
    Platform(String),
}

/// Checked declaration source plus explicitly separate raw metadata provenance.
#[derive(Clone, Debug)]
pub struct MarkerAnnotationPlan {
    /// Checked restored platform annotations followed by the genuine class header.
    pub header: String,
    /// Exact original annotation directory, including source/system annotations.
    pub original_annotations: Option<AnnotationsDirectory>,
    /// Exact original entries not emitted; these prevent metadata-completeness claims.
    pub retained_annotations: Vec<AnnotationItem>,
    /// Original descriptor/flags are evidence; this is not a body-publication token.
    pub descriptor: String,
    /// Genuine DEX class flags, never rewritten to permit publication.
    pub access_flags: u32,
}
impl MarkerAnnotationPlan {
    /// All raw annotation entries were restored. This does not attest enclosing
    /// source identity, whole-class closure, app completeness or runtime execution.
    pub fn annotation_metadata_restored(&self) -> bool {
        self.retained_annotations.is_empty()
    }
}

fn descriptor(dex: &DexFile, index: u32) -> Result<&str, MarkerRequirement> {
    dex.type_id(index)
        .ok()
        .and_then(|t| dex.string(t.descriptor_idx).ok())
        .filter(|s| s.starts_with('L') && s.ends_with(';'))
        .ok_or(MarkerRequirement::InvalidMetadata)
}

fn standard_class(
    input_owners: &BTreeSet<String>,
    platform: &dyn ExternalMetadata,
    owner: &str,
    annotation: bool,
) -> Result<(), MarkerRequirement> {
    let evidence = platform.class_hierarchy(owner);
    let expected = owner
        .trim_start_matches('L')
        .trim_end_matches(';')
        .replace('/', ".");
    if input_owners.contains(owner)
        || evidence.len() != 1
        || evidence[0].name != expected
        || platform
            .class_access(owner)
            .is_none_or(|flags| flags & 1 == 0 || (annotation && flags & 0x2200 != 0x2200))
        || (annotation
            && (!evidence[0].is_interface
                || evidence[0].superclass.as_deref() != Some("java.lang.Object")
                || evidence[0].interfaces != ["java.lang.annotation.Annotation"]))
    {
        return Err(MarkerRequirement::Platform(owner.into()));
    }
    Ok(())
}
fn enum_literal(
    dex: &DexFile,
    input_owners: &BTreeSet<String>,
    platform: &dyn ExternalMetadata,
    value: &EncodedValue,
    expected: &str,
    name: &impl Fn(u32) -> Result<String, DeclarationUnsupported>,
) -> Result<String, MarkerRequirement> {
    let Value::Enum(index) = value.value else {
        return Err(MarkerRequirement::InvalidMetadata);
    };
    let field = dex
        .field_id(index)
        .map_err(|_| MarkerRequirement::InvalidMetadata)?;
    if descriptor(dex, u32::from(field.class_idx))? != expected
        || descriptor(dex, u32::from(field.type_idx))? != expected
    {
        return Err(MarkerRequirement::InvalidMetadata);
    }
    standard_class(input_owners, platform, expected, false)?;
    if platform
        .class_access(expected)
        .is_none_or(|f| f & 0x4000 == 0)
    {
        return Err(MarkerRequirement::Platform(expected.into()));
    }
    let raw_name = dex
        .string(field.name_idx)
        .map_err(|_| MarkerRequirement::InvalidMetadata)?;
    let key = FieldKey {
        owner: expected.into(),
        name: raw_name.into(),
        descriptor: expected.into(),
    };
    let actual = platform
        .field(&key)
        .ok_or_else(|| MarkerRequirement::Platform(format!("{expected}->{raw_name}")))?;
    if actual.owner != key.owner
        || actual.name != key.name
        || actual.descriptor != key.descriptor
        || actual.access_flags != 0x4019
    {
        return Err(MarkerRequirement::InvalidMetadata);
    }
    let expected_source = expected
        .trim_start_matches('L')
        .trim_end_matches(';')
        .replace('/', ".");
    if name(u32::from(field.class_idx)).map_err(MarkerRequirement::Header)? != expected_source {
        return Err(MarkerRequirement::Platform("platform alias changed".into()));
    }
    Ok(format!("{expected_source}.{raw_name}"))
}
fn validate_raw_value(
    dex: &DexFile,
    value: &EncodedValue,
    depth: usize,
) -> Result<(), MarkerRequirement> {
    if depth > 16 {
        return Err(MarkerRequirement::InvalidMetadata);
    }
    match &value.value {
        Value::String(i) => {
            dex.string(*i)
                .map_err(|_| MarkerRequirement::InvalidMetadata)?;
        }
        Value::Type(i) => {
            dex.type_id(*i)
                .map_err(|_| MarkerRequirement::InvalidMetadata)?;
        }
        Value::Field(i) | Value::Enum(i) => {
            dex.field_id(*i)
                .map_err(|_| MarkerRequirement::InvalidMetadata)?;
        }
        Value::Method(i) => {
            dex.method_id(*i)
                .map_err(|_| MarkerRequirement::InvalidMetadata)?;
        }
        Value::MethodType(i) => {
            dex.proto_id(*i)
                .map_err(|_| MarkerRequirement::InvalidMetadata)?;
        }
        Value::MethodHandle(i) => {
            dex.method_handles
                .get(*i as usize)
                .ok_or(MarkerRequirement::InvalidMetadata)?;
        }
        Value::Array(v) => {
            for e in v {
                validate_raw_value(dex, e, depth + 1)?;
            }
        }
        Value::Annotation(a) => {
            descriptor(dex, a.type_idx)?;
            let mut names = BTreeSet::new();
            for e in &a.elements {
                let n = dex
                    .string(e.name_idx)
                    .map_err(|_| MarkerRequirement::InvalidMetadata)?;
                if !names.insert(n) {
                    return Err(MarkerRequirement::InvalidMetadata);
                }
                validate_raw_value(dex, &e.value, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Convenience wrapper for bounded clients. Large exports should reuse their
/// complete original input-owner index with `plan_marker_annotation_indexed`.
pub fn plan_marker_annotation(
    dex: &DexFile,
    class: &ClassDef,
    files: &[DexFile],
    platform: &dyn ExternalMetadata,
    name: &impl Fn(u32) -> Result<String, DeclarationUnsupported>,
) -> Result<MarkerAnnotationPlan, MarkerRequirement> {
    let mut input_owners = BTreeSet::new();
    for input in files {
        for original in &input.classes {
            input_owners.insert(descriptor(input, original.class_idx)?.to_owned());
        }
    }
    plan_marker_annotation_indexed(dex, class, &input_owners, platform, name)
}

/// Recover only genuine annotation markers with no fields, direct methods or
/// virtual methods and no AnnotationDefault wrapper. Standard JDK metadata uses
/// exact trusted declarations; unsupported raw entries remain explicit provenance.
/// No missing elements/defaults are inferred, invented, or silently discarded.
/// `input_owners` must be the complete, immutable original-owner inventory for
/// this export; filtering publication must never filter this shadow evidence.
pub fn plan_marker_annotation_indexed(
    dex: &DexFile,
    class: &ClassDef,
    input_owners: &BTreeSet<String>,
    platform: &dyn ExternalMetadata,
    name: &impl Fn(u32) -> Result<String, DeclarationUnsupported>,
) -> Result<MarkerAnnotationPlan, MarkerRequirement> {
    if class_kind(class.access_flags) != Ok(ClassKind::Annotation)
        || !dex.classes.iter().any(|original| original == class)
        || class.class_data.as_ref().is_some_and(|d| {
            !d.static_fields.is_empty()
                || !d.instance_fields.is_empty()
                || !d.direct_methods.is_empty()
                || !d.virtual_methods.is_empty()
        })
        || class
            .static_values
            .as_ref()
            .is_some_and(|values| !values.is_empty())
        || descriptor(dex, class.superclass_idx)? != "Ljava/lang/Object;"
        || class.interfaces.as_ref().is_none_or(|i| {
            i.types.len() != 1
                || descriptor(dex, u32::from(i.types[0])).ok()
                    != Some("Ljava/lang/annotation/Annotation;")
        })
    {
        return Err(MarkerRequirement::NotMarker);
    }
    standard_class(input_owners, platform, "Ljava/lang/Object;", false)?;
    let object = platform.class_hierarchy("Ljava/lang/Object;");
    if object[0].superclass.is_some() || object[0].is_interface {
        return Err(MarkerRequirement::Platform("untrusted Object root".into()));
    }
    standard_class(
        input_owners,
        platform,
        "Ljava/lang/annotation/Annotation;",
        false,
    )?;
    let marker = platform.class_hierarchy("Ljava/lang/annotation/Annotation;");
    if !marker[0].is_interface || marker[0].superclass.as_deref() != Some("java.lang.Object") {
        return Err(MarkerRequirement::Platform(
            "untrusted Annotation marker".into(),
        ));
    }
    let header = class_declaration_with(dex, class, name).map_err(MarkerRequirement::Header)?;
    let mut restored = Vec::new();
    let mut retained = Vec::new();
    if let Some(directory) = &class.annotations {
        if !directory.field_annotations.is_empty()
            || !directory.method_annotations.is_empty()
            || !directory.parameter_annotations.is_empty()
        {
            return Err(MarkerRequirement::InvalidMetadata);
        }
        let mut seen = BTreeSet::new();
        for item in directory.class_annotations.iter().flat_map(|s| &s.entries) {
            let owner = descriptor(dex, item.annotation.type_idx)?;
            if !seen.insert(owner) || owner == "Ldalvik/annotation/AnnotationDefault;" {
                return Err(MarkerRequirement::InvalidMetadata);
            }
            let mut element_names = BTreeSet::new();
            for element in &item.annotation.elements {
                let n = dex
                    .string(element.name_idx)
                    .map_err(|_| MarkerRequirement::InvalidMetadata)?;
                if !element_names.insert(n) {
                    return Err(MarkerRequirement::InvalidMetadata);
                }
                validate_raw_value(dex, &element.value, 0)?;
            }
            let rendered = match owner {
                "Ljava/lang/annotation/Retention;" | "Ljava/lang/annotation/Target;" => {
                    standard_class(input_owners, platform, owner, true)?;
                    if item.visibility != Visibility::Runtime
                        || item.annotation.elements.len() != 1
                        || dex.string(item.annotation.elements[0].name_idx).ok() != Some("value")
                    {
                        return Err(MarkerRequirement::InvalidMetadata);
                    }
                    let enumeration = if owner.ends_with("/Retention;") {
                        "Ljava/lang/annotation/RetentionPolicy;"
                    } else {
                        "Ljava/lang/annotation/ElementType;"
                    };
                    let return_type = if owner.ends_with("/Target;") {
                        format!("[{enumeration}")
                    } else {
                        enumeration.into()
                    };
                    let method = MethodKey {
                        owner: owner.into(),
                        name: "value".into(),
                        parameters: vec![],
                        return_type,
                    };
                    if platform.method_access(&method) != Some(0x401) {
                        return Err(MarkerRequirement::Platform(format!("{owner}->value")));
                    }
                    let v = &item.annotation.elements[0].value;
                    let expression = if owner.ends_with("/Target;") {
                        let Value::Array(values) = &v.value else {
                            return Err(MarkerRequirement::InvalidMetadata);
                        };
                        let mut targets = BTreeSet::new();
                        let mut expressions = Vec::new();
                        for v in values {
                            let expression =
                                enum_literal(dex, input_owners, platform, v, enumeration, name)?;
                            if !targets.insert(expression.clone()) {
                                return Err(MarkerRequirement::InvalidMetadata);
                            }
                            expressions.push(expression);
                        }
                        format!("{{{}}}", expressions.join(", "))
                    } else {
                        enum_literal(dex, input_owners, platform, v, enumeration, name)?
                    };
                    Some(format!(
                        "@{}({expression})",
                        owner
                            .trim_start_matches('L')
                            .trim_end_matches(';')
                            .replace('/', ".")
                    ))
                }
                "Ljava/lang/annotation/Inherited;" | "Ljava/lang/annotation/Documented;" => {
                    standard_class(input_owners, platform, owner, true)?;
                    if item.visibility != Visibility::Runtime
                        || !item.annotation.elements.is_empty()
                        || platform
                            .declared_methods(owner)
                            .is_none_or(|m| !m.is_empty())
                    {
                        return Err(MarkerRequirement::InvalidMetadata);
                    }
                    Some(format!(
                        "@{}",
                        owner
                            .trim_start_matches('L')
                            .trim_end_matches(';')
                            .replace('/', ".")
                    ))
                }
                _ => None,
            };
            if let Some(source) = rendered {
                restored.push(source);
            } else {
                retained.push(item.clone());
            }
        }
    }
    restored.push(header);
    Ok(MarkerAnnotationPlan {
        header: restored.join("\n"),
        original_annotations: class.annotations.clone(),
        retained_annotations: retained,
        descriptor: descriptor(dex, class.class_idx)?.into(),
        access_flags: class.access_flags,
    })
}
