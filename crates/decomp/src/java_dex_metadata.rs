//! Explicit declaration-only metadata from genuine external DEX files.
//!
//! A runtime DEX is not a javac classpath. Callers separately attest that the
//! selected API is available to their target; no JVM Code, ConstantValue, class
//! file version, or Object bytecode proof is inferred from DEX declarations.
use super::{Classpath, ExternalMetadata, FieldMetadata, MethodMetadata, ParsedMethod};
use crate::{
    java_aliases::{FieldKey, MethodKey},
    type_compatibility::Declaration,
};
use dex_format::{
    DexFile,
    annotations::{AnnotationItem, AnnotationSet, Visibility},
    class::ClassDef,
    encoded_value::EncodedValueKind,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::{Cursor, Read},
    path::Path,
};

/// External DEX metadata ingestion failed; no partially built provider escapes.
#[derive(Debug)]
pub enum DexMetadataError {
    /// An explicit input could not be read.
    Io(std::io::Error),
    /// An explicit ZIP could not be decoded.
    Zip(zip::result::ZipError),
    /// A genuine DEX could not be decoded.
    Dex(dex_format::DexError),
    /// A decoded structure is incomplete, contradictory, or malformed.
    Invalid(&'static str),
    /// Two input DEX files define the same owner.
    DuplicateOwner(String),
    /// A class defines the same exact method more than once.
    DuplicateMethod(MethodKey),
    /// A class defines the same exact field more than once.
    DuplicateField(FieldKey),
}
impl fmt::Display for DexMetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for DexMetadataError {}
impl From<std::io::Error> for DexMetadataError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<zip::result::ZipError> for DexMetadataError {
    fn from(e: zip::result::ZipError) -> Self {
        Self::Zip(e)
    }
}
impl From<dex_format::DexError> for DexMetadataError {
    fn from(e: dex_format::DexError) -> Self {
        Self::Dex(e)
    }
}

#[derive(Clone, Debug)]
struct DexClass {
    access_flags: u32,
    declaration: Declaration,
    methods: Vec<ParsedMethod>,
    fields: Vec<FieldMetadata>,
    // Only genuine member-class relationships, never binary-name splitting.
    member: Option<(String, String)>,
    signature: Option<String>,
    code_presence: BTreeMap<MethodKey, bool>,
    covariant_bridges: BTreeMap<MethodKey, MethodKey>,
}

/// Immutable, export-local original external DEX declarations.
#[derive(Clone, Debug, Default)]
pub struct DexMetadata {
    classes: BTreeMap<String, DexClass>,
}
impl DexMetadata {
    /// Index exact ClassData definitions. Pool references are never declarations.
    ///
    /// # Errors
    /// Rejects duplicate owners/members and incomplete or malformed metadata.
    pub fn from_dex_files(files: &[DexFile]) -> Result<Self, DexMetadataError> {
        let mut classes = BTreeMap::new();
        for dex in files {
            for class in &dex.classes {
                let owner = descriptor(dex, class.class_idx)?.to_owned();
                let name = class_name(&owner)?;
                if classes.contains_key(&owner) {
                    return Err(DexMetadataError::DuplicateOwner(owner));
                }
                check_complete(class)?;
                let superclass = (class.superclass_idx != u32::MAX)
                    .then(|| descriptor(dex, class.superclass_idx).and_then(class_name))
                    .transpose()?;
                let interfaces = class
                    .interfaces
                    .as_ref()
                    .map(|list| {
                        list.types
                            .iter()
                            .map(|t| descriptor(dex, u32::from(*t)).and_then(class_name))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                let declaration = Declaration {
                    name,
                    superclass,
                    interfaces,
                    is_interface: class.access_flags & 0x200 != 0,
                };
                let member = member_relationship(dex, class)?;
                // Validate recognized class Signature metadata as well, without
                // manufacturing a ClassMetadata/JVM version from it.
                let class_signature = signature(
                    dex,
                    class
                        .annotations
                        .as_ref()
                        .and_then(|d| d.class_annotations.as_ref()),
                )?;
                let mut methods = Vec::new();
                let mut fields = Vec::new();
                let mut method_keys = BTreeSet::new();
                let mut field_keys = BTreeSet::new();
                let mut method_indices = BTreeSet::new();
                let mut code_presence = BTreeMap::new();
                let mut defined_keys = BTreeMap::new();
                let mut field_indices = BTreeSet::new();
                if let Some(data) = &class.class_data {
                    for (direct, method) in data
                        .direct_methods
                        .iter()
                        .map(|m| (true, m))
                        .chain(data.virtual_methods.iter().map(|m| (false, m)))
                    {
                        let raw = dex.method_id(method.method_idx)?;
                        if u32::from(raw.class_idx) != class.class_idx {
                            return Err(DexMetadataError::Invalid(
                                "method owner differs from ClassData owner",
                            ));
                        }
                        let proto = dex.proto_id(u32::from(raw.proto_idx))?;
                        if proto.parameters_off != 0 && proto.parameters.is_none() {
                            return Err(DexMetadataError::Invalid("missing parameter type list"));
                        }
                        let parameters = proto
                            .parameters
                            .as_ref()
                            .map(|p| {
                                p.types
                                    .iter()
                                    .map(|t| descriptor(dex, u32::from(*t)).map(str::to_owned))
                                    .collect::<Result<Vec<_>, _>>()
                            })
                            .transpose()?
                            .unwrap_or_default();
                        let key = MethodKey {
                            owner: owner.clone(),
                            name: dex.string(raw.name_idx)?.into(),
                            parameters,
                            return_type: descriptor(dex, proto.return_type_idx)?.into(),
                        };
                        let descriptor =
                            format!("({}){}", key.parameters.join(""), key.return_type);
                        for parameter in &key.parameters {
                            validate_type(parameter, false)?;
                        }
                        validate_type(&key.return_type, true)?;
                        if !method_keys.insert(key.clone())
                            || !method_indices.insert(method.method_idx)
                        {
                            return Err(DexMetadataError::DuplicateMethod(key));
                        }
                        if !direct
                            && (method.access_flags & (2 | 8 | 0x10000) != 0
                                || key.name.starts_with('<'))
                        {
                            return Err(DexMetadataError::Invalid("invalid virtual method flags"));
                        }
                        if direct
                            && method.access_flags & (2 | 8 | 0x10000) == 0
                            && key.name != "<init>"
                            && key.name != "<clinit>"
                        {
                            return Err(DexMetadataError::Invalid("invalid direct method flags"));
                        }
                        if (method.code_off != 0) != method.code.is_some() {
                            return Err(DexMetadataError::Invalid("missing method code item"));
                        }
                        let annotations = member_annotations(class, method.method_idx, false)?;
                        let exceptions = throws(dex, annotations)?;
                        code_presence.insert(key.clone(), method.code.is_some());
                        defined_keys.insert(method.method_idx, key.clone());
                        methods.push(ParsedMethod {
                            key,
                            metadata: MethodMetadata {
                                access_flags: method.access_flags,
                                exceptions: Some(exceptions),
                            },
                            descriptor,
                            signature: signature(dex, annotations)?,
                            code: None,
                        });
                    }
                    for (is_static, field) in data
                        .static_fields
                        .iter()
                        .map(|f| (true, f))
                        .chain(data.instance_fields.iter().map(|f| (false, f)))
                    {
                        let raw = dex.field_id(field.field_idx)?;
                        if u32::from(raw.class_idx) != class.class_idx {
                            return Err(DexMetadataError::Invalid(
                                "field owner differs from ClassData owner",
                            ));
                        }
                        if (field.access_flags & 8 != 0) != is_static {
                            return Err(DexMetadataError::Invalid(
                                "field staticness differs from ClassData list",
                            ));
                        }
                        let key = FieldKey {
                            owner: owner.clone(),
                            name: dex.string(raw.name_idx)?.into(),
                            descriptor: descriptor(dex, u32::from(raw.type_idx))?.into(),
                        };
                        validate_type(&key.descriptor, false)?;
                        if !field_keys.insert(key.clone()) || !field_indices.insert(field.field_idx)
                        {
                            return Err(DexMetadataError::DuplicateField(key));
                        }
                        fields.push(FieldMetadata {
                            owner: owner.clone(),
                            name: key.name,
                            descriptor: key.descriptor,
                            access_flags: field.access_flags,
                            signature: signature(
                                dex,
                                member_annotations(class, field.field_idx, true)?,
                            )?,
                            constant_value: None,
                            attributes: vec![],
                        });
                    }
                }
                if let Some(directory) = &class.annotations {
                    let mut observed_methods = BTreeSet::new();
                    let mut observed_fields = BTreeSet::new();
                    let mut observed_parameters = BTreeSet::new();
                    for a in &directory.method_annotations {
                        if !method_indices.contains(&a.member_idx)
                            || !observed_methods.insert(a.member_idx)
                        {
                            return Err(DexMetadataError::Invalid(
                                "undeclared or duplicate method annotation target",
                            ));
                        }
                    }
                    for a in &directory.field_annotations {
                        if !field_indices.contains(&a.member_idx)
                            || !observed_fields.insert(a.member_idx)
                        {
                            return Err(DexMetadataError::Invalid(
                                "undeclared or duplicate field annotation target",
                            ));
                        }
                    }
                    for a in &directory.parameter_annotations {
                        if !method_indices.contains(&a.method_idx)
                            || !observed_parameters.insert(a.method_idx)
                        {
                            return Err(DexMetadataError::Invalid(
                                "undeclared or duplicate parameter annotation target",
                            ));
                        }
                    }
                }
                let covariant_bridges = dex_covariant_bridges(dex, class, &methods, &defined_keys);
                classes.insert(
                    owner,
                    DexClass {
                        access_flags: class.access_flags,
                        declaration,
                        methods,
                        fields,
                        member,
                        signature: class_signature,
                        code_presence,
                        covariant_bridges,
                    },
                );
            }
        }
        Ok(Self { classes })
    }
    /// Parse a caller-selected raw DEX or manifest-free DEX JAR/ZIP.
    ///
    /// # Errors
    /// Rejects unreadable/malformed inputs, ambiguous ZIP member order, and
    /// archives without conventional root classes*.dex entries.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, DexMetadataError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)?;
        if bytes.starts_with(b"dex\n") {
            return Self::from_dex_files(&DexFile::from_container(&bytes)?);
        }
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
        let mut entries = BTreeMap::new();
        for i in 0..archive.len() {
            let name = archive.by_index(i)?.name().to_owned();
            let order = if name == "classes.dex" {
                Some(1)
            } else {
                name.strip_prefix("classes")
                    .and_then(|s| s.strip_suffix(".dex"))
                    .and_then(|s| {
                        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                            .then(|| s.parse::<u32>().ok())
                            .flatten()
                            .filter(|n| *n >= 2)
                    })
            };
            if let Some(order) = order
                && entries.insert(order, name).is_some()
            {
                return Err(DexMetadataError::Invalid("duplicate DEX ZIP ordinal"));
            }
        }
        if entries.is_empty() {
            return Err(DexMetadataError::Invalid(
                "archive has no conventional DEX entries",
            ));
        }
        let mut files = Vec::new();
        for name in entries.values() {
            let mut data = Vec::new();
            archive.by_name(name)?.read_to_end(&mut data)?;
            files.extend(DexFile::from_container(&data)?);
        }
        Self::from_dex_files(&files)
    }
    /// Original owner descriptors, once each, in stable lexical order.
    pub fn owners(&self) -> impl Iterator<Item = &str> {
        self.classes.keys().map(String::as_str)
    }
    fn source_name(&self, owner: &str, seen: &mut BTreeSet<String>) -> Option<String> {
        if !seen.insert(owner.into()) {
            return None;
        }
        let class = self.classes.get(owner)?;
        let Some((outer, name)) = &class.member else {
            return Some(class.declaration.name.clone());
        };
        Some(format!("{}.{}", self.source_name(outer, seen)?, name))
    }
}
fn check_complete(class: &ClassDef) -> Result<(), DexMetadataError> {
    if (class.class_data_off != 0) != class.class_data.is_some()
        || (class.annotations_off != 0) != class.annotations.is_some()
        || (class.interfaces_off != 0) != class.interfaces.is_some()
    {
        return Err(DexMetadataError::Invalid("incomplete class-owned metadata"));
    }
    Ok(())
}
fn descriptor(dex: &DexFile, index: u32) -> Result<&str, DexMetadataError> {
    Ok(dex.string(dex.type_id(index)?.descriptor_idx)?)
}
fn class_name(descriptor: &str) -> Result<String, DexMetadataError> {
    let body = descriptor
        .strip_prefix('L')
        .and_then(|s| s.strip_suffix(';'))
        .filter(|s| !s.is_empty())
        .ok_or(DexMetadataError::Invalid("invalid class descriptor"))?;
    if body.bytes().any(|b| b == b';' || b == b'[' || b == b'.')
        || body.split('/').any(str::is_empty)
    {
        return Err(DexMetadataError::Invalid("invalid class descriptor"));
    }
    Ok(body.replace('/', "."))
}
// DEX permits binary names that are not source identifiers (for example
// desugared -CC classes). Preserve those exact declarations; Java name
// representability is a separate facade proof, never a metadata alias guess.
fn validate_type(descriptor: &str, allow_void: bool) -> Result<(), DexMetadataError> {
    let rank = descriptor.bytes().take_while(|b| *b == b'[').count();
    if rank > 255 {
        return Err(DexMetadataError::Invalid("array rank exceeds 255"));
    }
    let base = &descriptor[rank..];
    if matches!(base, "Z" | "B" | "S" | "C" | "I" | "J" | "F" | "D")
        || (base == "V" && rank == 0 && allow_void)
    {
        return Ok(());
    }
    let _ = class_name(base)?;
    Ok(())
}
fn annotation<'a>(
    dex: &DexFile,
    set: Option<&'a AnnotationSet>,
    kind: &str,
) -> Result<Option<&'a AnnotationItem>, DexMetadataError> {
    let mut selected = None;
    let mut types = BTreeSet::new();
    if let Some(set) = set {
        for item in &set.entries {
            let name = descriptor(dex, item.annotation.type_idx)?;
            if !types.insert(name) {
                return Err(DexMetadataError::Invalid("duplicate annotation type"));
            }
            if name == kind {
                if item.visibility != Visibility::System {
                    return Err(DexMetadataError::Invalid(
                        "wrong system annotation visibility",
                    ));
                }
                selected = Some(item);
            }
        }
    }
    Ok(selected)
}
fn value<'a>(
    dex: &DexFile,
    item: &'a AnnotationItem,
) -> Result<&'a EncodedValueKind, DexMetadataError> {
    let elements = &item.annotation.elements;
    if elements.len() != 1 || dex.string(elements[0].name_idx)? != "value" {
        return Err(DexMetadataError::Invalid(
            "malformed system annotation value",
        ));
    }
    Ok(&elements[0].value.value)
}
fn signature(
    dex: &DexFile,
    set: Option<&AnnotationSet>,
) -> Result<Option<String>, DexMetadataError> {
    let Some(item) = annotation(dex, set, "Ldalvik/annotation/Signature;")? else {
        return Ok(None);
    };
    let EncodedValueKind::Array(values) = value(dex, item)? else {
        return Err(DexMetadataError::Invalid("Signature is not an array"));
    };
    let mut signature = String::new();
    for value in values {
        let EncodedValueKind::String(index) = value.value else {
            return Err(DexMetadataError::Invalid(
                "Signature fragment is not a string",
            ));
        };
        signature.push_str(dex.string(index)?);
    }
    if signature.is_empty() {
        return Err(DexMetadataError::Invalid("empty Signature"));
    }
    Ok(Some(signature))
}
fn throws(dex: &DexFile, set: Option<&AnnotationSet>) -> Result<Vec<String>, DexMetadataError> {
    let Some(item) = annotation(dex, set, "Ldalvik/annotation/Throws;")? else {
        return Ok(vec![]);
    };
    let EncodedValueKind::Array(values) = value(dex, item)? else {
        return Err(DexMetadataError::Invalid("Throws is not an array"));
    };
    values
        .iter()
        .map(|value| {
            let EncodedValueKind::Type(index) = value.value else {
                return Err(DexMetadataError::Invalid("Throws element is not a type"));
            };
            let descriptor = descriptor(dex, index)?;
            let _ = class_name(descriptor)?;
            Ok(descriptor.into())
        })
        .collect()
}
fn member_annotations(
    class: &ClassDef,
    index: u32,
    field: bool,
) -> Result<Option<&AnnotationSet>, DexMetadataError> {
    let Some(directory) = &class.annotations else {
        return Ok(None);
    };
    let annotations = if field {
        &directory.field_annotations
    } else {
        &directory.method_annotations
    };
    let mut selected = annotations.iter().filter(|m| m.member_idx == index);
    let first = selected.next();
    if selected.next().is_some() {
        return Err(DexMetadataError::Invalid(
            "duplicate member annotation target",
        ));
    }
    Ok(first.map(|m| &m.annotations))
}
fn member_relationship(
    dex: &DexFile,
    class: &ClassDef,
) -> Result<Option<(String, String)>, DexMetadataError> {
    let set = class
        .annotations
        .as_ref()
        .and_then(|d| d.class_annotations.as_ref());
    let inner = annotation(dex, set, "Ldalvik/annotation/InnerClass;")?;
    let enclosing = annotation(dex, set, "Ldalvik/annotation/EnclosingClass;")?;
    let method = annotation(dex, set, "Ldalvik/annotation/EnclosingMethod;")?;
    if enclosing.is_some() && method.is_some() {
        return Err(DexMetadataError::Invalid(
            "contradictory enclosing annotations",
        ));
    }
    let Some(inner) = inner else {
        if enclosing.is_some() || method.is_some() {
            return Err(DexMetadataError::Invalid(
                "enclosing annotation without InnerClass",
            ));
        }
        return Ok(None);
    };
    if inner.annotation.elements.len() != 2 {
        return Err(DexMetadataError::Invalid("malformed InnerClass"));
    }
    let mut name = None;
    let mut flags = None;
    for e in &inner.annotation.elements {
        match dex.string(e.name_idx)? {
            "name" if name.is_none() => {
                name = Some(match e.value.value {
                    EncodedValueKind::String(s) => Some(dex.string(s)?.to_owned()),
                    EncodedValueKind::Null => None,
                    _ => return Err(DexMetadataError::Invalid("invalid InnerClass name")),
                })
            }
            "accessFlags" if flags.is_none() => {
                let EncodedValueKind::Int(f) = e.value.value else {
                    return Err(DexMetadataError::Invalid("invalid InnerClass flags"));
                };
                flags = Some(f);
            }
            _ => return Err(DexMetadataError::Invalid("invalid InnerClass elements")),
        }
    }
    if flags.is_none() {
        return Err(DexMetadataError::Invalid("missing InnerClass flags"));
    }
    let name = name.ok_or(DexMetadataError::Invalid("missing InnerClass name"))?;
    if let Some(method) = method {
        let EncodedValueKind::Method(index) = value(dex, method)? else {
            return Err(DexMetadataError::Invalid("invalid EnclosingMethod"));
        };
        let _ = dex.method_id(*index)?;
        return Ok(None);
    }
    let Some(enclosing) = enclosing else {
        return Ok(None);
    };
    let EncodedValueKind::Type(index) = value(dex, enclosing)? else {
        return Err(DexMetadataError::Invalid("invalid EnclosingClass"));
    };
    let outer = descriptor(dex, *index)?.to_owned();
    let _ = class_name(&outer)?;
    let Some(name) = name else { return Ok(None) };
    // Unsupported source names remain unavailable, without guessing aliases.
    if crate::java_descriptors::source_identifier(&name).is_err()
        || descriptor(dex, class.class_idx)?
            != format!(
                "{}${name};",
                outer
                    .strip_suffix(';')
                    .ok_or(DexMetadataError::Invalid("invalid enclosing descriptor"))?
            )
    {
        return Ok(None);
    }
    Ok(Some((outer, name)))
}
// This attests only straight-through DEX forwarding, never JVM Code or
// return assignability. SuperIndex independently proves reference covariance.
fn dex_covariant_bridges(
    dex: &DexFile,
    class: &ClassDef,
    methods: &[ParsedMethod],
    defined_keys: &BTreeMap<u32, MethodKey>,
) -> BTreeMap<MethodKey, MethodKey> {
    let mut result = BTreeMap::new();
    let Some(data) = &class.class_data else {
        return result;
    };
    for encoded in &data.virtual_methods {
        if encoded.access_flags & 0x1040 != 0x1040 {
            continue;
        }
        let Some(bridge_key) = defined_keys.get(&encoded.method_idx) else {
            continue;
        };
        let Some(bridge) = methods.iter().find(|m| &m.key == bridge_key) else {
            continue;
        };
        let Some(code) = &encoded.code else {
            continue;
        };
        let Some(index) = dex_bridge_target(code, &bridge.key, encoded.access_flags) else {
            continue;
        };
        let Some(target_encoded) = data.virtual_methods.iter().find(|m| m.method_idx == index)
        else {
            continue;
        };
        let Ok(target_raw) = dex.method_id(index) else {
            continue;
        };
        if u32::from(target_raw.class_idx) != class.class_idx {
            continue;
        }
        let Ok(proto) = dex.proto_id(u32::from(target_raw.proto_idx)) else {
            continue;
        };
        let Ok(return_type) = descriptor(dex, proto.return_type_idx) else {
            continue;
        };
        let Some(target_key) = defined_keys.get(&index) else {
            continue;
        };
        let Some(target) = methods.iter().find(|m| &m.key == target_key) else {
            continue;
        };
        if target.key.name != bridge.key.name
            || target.key.parameters != bridge.key.parameters
            || target.key.owner != bridge.key.owner
            || target.key.return_type != return_type
        {
            continue;
        }
        if target_encoded.access_flags & (2 | 8 | 0x40 | 0x100 | 0x400) != 0
            || target_encoded.code.is_none()
            || target.key.return_type == bridge.key.return_type
            || !reference_return(&target.key.return_type)
            || bridge.signature.is_some()
            || target.signature.is_some()
        {
            continue;
        }
        result.insert(bridge.key.clone(), target.key.clone());
    }
    result
}
fn reference_return(ty: &str) -> bool {
    ty.starts_with('L') || ty.starts_with('[')
}
fn dex_bridge_target(
    code: &dex_format::code::CodeItem,
    key: &MethodKey,
    flags: u32,
) -> Option<u32> {
    if flags & 0x1040 != 0x1040
        || flags & (2 | 8 | 0x100 | 0x400 | 0x20 | 0x20000) != 0
        || !reference_return(&key.return_type)
        || !code.tries.is_empty()
        || !code.handlers.is_empty()
        || code.insns_size != 5
        || code.insns.len() != 10
    {
        return None;
    }
    let units: Vec<u16> = code
        .insns
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let words = key.parameters.iter().try_fold(1u16, |n, p| {
        n.checked_add(if p == "J" || p == "D" { 2 } else { 1 })
    })?;
    if code.ins_size != words || code.outs_size < words {
        return None;
    }
    let first = code.registers_size.checked_sub(words)?;
    match units[0] & 255 {
        0x6e => {
            if words > 5 || units[0] >> 12 != words {
                return None;
            }
            let registers = [
                units[2] & 15,
                (units[2] >> 4) & 15,
                (units[2] >> 8) & 15,
                units[2] >> 12,
                (units[0] >> 8) & 15,
            ];
            for i in 0..words {
                if registers[usize::from(i)] != first.checked_add(i)? {
                    return None;
                }
            }
        }
        0x74 => {
            if units[0] >> 8 != words || units[2] != first {
                return None;
            }
        }
        _ => return None,
    }
    let register = units[3] >> 8;
    if units[3] & 255 != 0x0c
        || units[4] & 255 != 0x11
        || units[4] >> 8 != register
        || register >= code.registers_size
    {
        return None;
    }
    Some(u32::from(units[1]))
}

impl ExternalMetadata for DexMetadata {
    fn attests_covariant_bridge(&self, bridge: &MethodKey, target: &MethodKey) -> bool {
        self.classes
            .get(&bridge.owner)
            .is_some_and(|class| class.covariant_bridges.get(bridge) == Some(target))
    }

    fn inherited_public_instance(&self, key: &MethodKey) -> Option<super::InheritedPublicMethod> {
        let chain = MetadataChain::new(vec![MetadataProvider::Dex(self)]);
        super::inherited_methods::resolve_runtime_class_instance(&chain, key).ok()
    }
    fn method(&self, key: &MethodKey) -> Option<MethodMetadata> {
        self.classes
            .get(&key.owner)?
            .methods
            .iter()
            .find(|m| &m.key == key)
            .map(|m| m.metadata.clone())
    }
    fn declared_methods(&self, owner: &str) -> Option<Vec<ParsedMethod>> {
        Some(self.classes.get(owner)?.methods.clone())
    }
    fn class_generic_signature(&self, owner: &str) -> Option<Option<String>> {
        Some(self.classes.get(owner)?.signature.clone())
    }
    fn class_access(&self, owner: &str) -> Option<u32> {
        Some(self.classes.get(owner)?.access_flags)
    }
    fn class_hierarchy(&self, owner: &str) -> Vec<Declaration> {
        self.classes
            .get(owner)
            .map(|c| vec![c.declaration.clone()])
            .unwrap_or_default()
    }
    fn hierarchy(&self) -> Vec<Declaration> {
        self.classes
            .values()
            .map(|c| c.declaration.clone())
            .collect()
    }
    fn field(&self, key: &FieldKey) -> Option<FieldMetadata> {
        self.classes
            .get(&key.owner)?
            .fields
            .iter()
            .find(|f| f.name == key.name && f.descriptor == key.descriptor)
            .cloned()
    }
    fn declared_fields(&self, owner: &str) -> Option<Vec<FieldMetadata>> {
        Some(self.classes.get(owner)?.fields.clone())
    }
    fn member_source_names(&self) -> BTreeMap<String, String> {
        self.classes
            .iter()
            .filter_map(|(owner, class)| {
                class.member.as_ref()?;
                let source = self.source_name(owner, &mut BTreeSet::new())?;
                (source != class.declaration.name).then(|| (owner.clone(), source))
            })
            .collect()
    }
}

/// One original metadata provider; precedence selects an entire owner.
#[derive(Clone, Copy)]
pub enum MetadataProvider<'a> {
    /// Genuine external DEX declaration catalog.
    Dex(&'a DexMetadata),
    /// Genuine JVM class catalog, retaining its original Code/object proof.
    Jvm(&'a Classpath),
}
impl<'a> MetadataProvider<'a> {
    fn metadata(self) -> &'a dyn ExternalMetadata {
        match self {
            Self::Dex(m) => m,
            Self::Jvm(m) => m,
        }
    }
}
/// Ordered whole-owner selection without member fallback or merging.
///
/// Provider order is explicitly supplied by the caller. The first provider
/// defining an owner supplies its entire declaration and byte evidence; missing
/// members are never filled from a shadowed SDK or another provider.
pub struct MetadataChain<'a> {
    providers: Vec<MetadataProvider<'a>>,
    owners: BTreeMap<String, usize>,
    // Both successful and unavailable proofs are immutable for this frozen
    // provider order. Never share this cache between metadata input sets.
    inherited: std::sync::RwLock<BTreeMap<MethodKey, Option<super::InheritedPublicMethod>>>,
}
impl<'a> MetadataChain<'a> {
    /// Freeze first-provider ownership for every declaration.
    pub fn new(providers: Vec<MetadataProvider<'a>>) -> Self {
        let mut owners = BTreeMap::new();
        for (index, provider) in providers.iter().enumerate() {
            match provider {
                MetadataProvider::Dex(m) => {
                    for owner in m.owners() {
                        owners.entry(owner.into()).or_insert(index);
                    }
                }
                MetadataProvider::Jvm(m) => {
                    for class in m.classes() {
                        owners.entry(class.descriptor.clone()).or_insert(index);
                    }
                }
            }
        }
        Self {
            providers,
            owners,
            inherited: std::sync::RwLock::new(BTreeMap::new()),
        }
    }
    fn selected(&self, owner: &str) -> Option<&dyn ExternalMetadata> {
        Some(self.providers[*self.owners.get(owner)?].metadata())
    }
    /// Access the selected genuine classfile without copying or rebuilding its
    /// private object-origin attestation. DEX owners never yield JVM evidence.
    pub fn jvm_class(&self, owner: &str) -> Option<&'a super::ClassMetadata> {
        match self.providers[*self.owners.get(owner)?] {
            MetadataProvider::Jvm(m) => m.class(owner),
            MetadataProvider::Dex(_) => None,
        }
    }
    pub(super) fn instance_class(&self, owner: &str) -> Option<InstanceClass<'a>> {
        match self.providers[*self.owners.get(owner)?] {
            MetadataProvider::Dex(m) => {
                let class = m.classes.get(owner)?;
                Some(InstanceClass {
                    flags: class.access_flags,
                    declaration: &class.declaration,
                    signature: class.signature.as_deref(),
                    methods: &class.methods,
                    dex_code: Some(&class.code_presence),
                })
            }
            MetadataProvider::Jvm(m) => {
                let class = m.class(owner)?;
                Some(InstanceClass {
                    flags: class.access_flags,
                    declaration: &class.declaration,
                    signature: class.signature.as_deref(),
                    methods: &class.methods,
                    dex_code: None,
                })
            }
        }
    }
}
/// Private view of an actually selected parsed owner, never a JVM conversion.
pub(super) struct InstanceClass<'a> {
    pub(super) flags: u32,
    pub(super) declaration: &'a Declaration,
    pub(super) signature: Option<&'a str>,
    pub(super) methods: &'a [ParsedMethod],
    pub(super) dex_code: Option<&'a BTreeMap<MethodKey, bool>>,
}
impl ExternalMetadata for MetadataChain<'_> {
    fn inherited_public_instance(&self, key: &MethodKey) -> Option<super::InheritedPublicMethod> {
        if let Ok(cache) = self.inherited.read()
            && let Some(proof) = cache.get(key)
        {
            return proof.clone();
        }
        let proof = super::inherited_methods::resolve_inherited_public_instance(self, key)
            .ok()
            .or_else(|| super::inherited_methods::resolve_runtime_class_instance(self, key).ok());
        if let Ok(mut cache) = self.inherited.write() {
            cache.insert(key.clone(), proof.clone());
        }
        proof
    }

    fn attests_covariant_bridge(&self, bridge: &MethodKey, target: &MethodKey) -> bool {
        self.selected(&bridge.owner)
            .is_some_and(|provider| provider.attests_covariant_bridge(bridge, target))
    }
    fn method(&self, key: &MethodKey) -> Option<MethodMetadata> {
        self.selected(&key.owner)?.method(key)
    }
    fn method_access(&self, key: &MethodKey) -> Option<u32> {
        self.selected(&key.owner)?.method_access(key)
    }
    fn class_generic_signature(&self, owner: &str) -> Option<Option<String>> {
        self.selected(owner)?.class_generic_signature(owner)
    }
    fn declared_methods(&self, owner: &str) -> Option<Vec<ParsedMethod>> {
        self.selected(owner)?.declared_methods(owner)
    }
    fn class_access(&self, owner: &str) -> Option<u32> {
        self.selected(owner)?.class_access(owner)
    }
    fn class_hierarchy(&self, owner: &str) -> Vec<Declaration> {
        self.selected(owner)
            .map(|m| m.class_hierarchy(owner))
            .unwrap_or_default()
    }
    fn hierarchy(&self) -> Vec<Declaration> {
        self.owners
            .keys()
            .flat_map(|owner| self.class_hierarchy(owner))
            .collect()
    }
    fn field(&self, key: &FieldKey) -> Option<FieldMetadata> {
        self.selected(&key.owner)?.field(key)
    }
    fn declared_fields(&self, owner: &str) -> Option<Vec<FieldMetadata>> {
        self.selected(owner)?.declared_fields(owner)
    }
    fn member_source_names(&self) -> BTreeMap<String, String> {
        self.providers
            .iter()
            .enumerate()
            .flat_map(|(index, p)| {
                p.metadata()
                    .member_source_names()
                    .into_iter()
                    .filter(move |(owner, _)| self.owners.get(owner) == Some(&index))
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "java_dex_metadata_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "java_runtime_inherited_class_tests.rs"]
mod runtime_inherited_tests;

#[cfg(test)]
#[path = "java_dex_covariant_bridge_tests.rs"]
mod covariant_bridge_tests;
