//! Conservative, pool-independent implementation evidence for library classes.
//!
//! Register allocation, instruction layout, handler ordering, declarations and
//! runtime/system annotations are retained. This is exact evidence, not an
//! obfuscation-tolerant guess. Unsupported references yield no fingerprint.
use dex_decompiler::decode::{Opcode, decode_method};
use dex_format::{
    DexFile,
    class::ClassDef,
    code::CodeItem,
    encoded_value::{EncodedValue, EncodedValueKind},
};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Versioned format used by the evidence generator and CLI verifier.
pub const KIND: &str = "codepuri-class-implementation-v1";

/// An exact implementation digest and its nontrivial method-body anchors.
#[derive(Clone, Debug, Serialize)]
pub struct ClassFingerprint {
    /// Original DEX class descriptor.
    pub owner: String,
    /// SHA-256 of the canonical class definition and original method bodies.
    pub sha256: String,
    /// Body fingerprints with at least eight decoded instructions, excluding constructors.
    pub anchors: Vec<String>,
}

/// SHA-256 bytes, in lowercase hexadecimal.
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn ty(dex: &DexFile, index: u32) -> Option<String> {
    Some(
        dex.string(dex.type_id(index).ok()?.descriptor_idx)
            .ok()?
            .to_owned(),
    )
}

fn string(dex: &DexFile, index: u32) -> Option<String> {
    let span = dex.strings.get(index as usize)?.span;
    Some(digest(dex.raw_bytes().get(
        span.offset as usize..(span.offset + span.size) as usize,
    )?))
}

fn proto(dex: &DexFile, index: u32) -> Option<String> {
    let proto = dex.proto_id(index).ok()?;
    let mut result = String::from("(");
    if let Some(parameters) = &proto.parameters {
        for parameter in &parameters.types {
            result.push_str(&ty(dex, u32::from(*parameter))?);
        }
    }
    result.push(')');
    result.push_str(&ty(dex, proto.return_type_idx)?);
    Some(result)
}

fn method(dex: &DexFile, index: u32) -> Option<String> {
    let method = dex.method_id(index).ok()?;
    Some(format!(
        "{}->{}{}",
        ty(dex, u32::from(method.class_idx))?,
        dex.string(method.name_idx).ok()?,
        proto(dex, u32::from(method.proto_idx))?
    ))
}

fn field(dex: &DexFile, index: u32) -> Option<String> {
    let field = dex.field_id(index).ok()?;
    Some(format!(
        "{}->{}:{}",
        ty(dex, u32::from(field.class_idx))?,
        dex.string(field.name_idx).ok()?,
        ty(dex, u32::from(field.type_idx))?
    ))
}

fn reference(dex: &DexFile, opcode: u8, index: u32) -> Option<String> {
    match opcode {
        0x1a | 0x1b => Some(format!("string:{}", string(dex, index)?)),
        0x1c | 0x1f | 0x20 | 0x22..=0x25 => Some(format!("type:{}", ty(dex, index)?)),
        0x52..=0x6d => Some(format!("field:{}", field(dex, index)?)),
        0x6e..=0x72 | 0x74..=0x78 | 0xfa | 0xfb => Some(format!("method:{}", method(dex, index)?)),
        0xff => Some(format!("proto:{}", proto(dex, index)?)),
        _ => None,
    }
}

fn body(dex: &DexFile, code: &CodeItem) -> Option<(String, usize)> {
    let instructions = decode_method(code).ok()?;
    let count = instructions
        .iter()
        .filter(|insn| !matches!(insn.opcode, Opcode::Payload { .. }))
        .count();
    let mut masked = code.insns.clone();
    let mut references = Vec::new();
    for insn in instructions {
        let Opcode::Supported(info) = insn.opcode else {
            if !matches!(insn.opcode, Opcode::Payload { .. }) {
                return None;
            }
            continue;
        };
        if let Some(index) = insn.pool_index {
            references.push((insn.offset, reference(dex, info.opcode, index)?));
            let start = insn.offset as usize * 2 + 2;
            let width = if info.opcode == 0x1b { 4 } else { 2 };
            masked.get_mut(start..start + width)?.fill(0);
        }
        if let Some(index) = insn.secondary_pool_index {
            references.push((insn.offset, format!("secondary:{}", proto(dex, index)?)));
            let start = insn.offset as usize * 2 + 6;
            masked.get_mut(start..start + 2)?.fill(0);
        }
    }
    // Map handler identities to ordered semantic content, not encoded offsets.
    let mut handlers = Vec::new();
    for handler in &code.handlers {
        let mut typed = Vec::new();
        for item in &handler.handlers {
            typed.push((ty(dex, item.type_idx)?, item.address));
        }
        handlers.push((handler.relative_offset, typed, handler.catch_all_addr));
    }
    let mut tries = Vec::new();
    for scope in &code.tries {
        let (_, typed, catch_all) = handlers
            .iter()
            .find(|(offset, _, _)| *offset == u32::from(scope.handler_off))?;
        tries.push((scope.start_addr, scope.insn_count, typed, catch_all));
    }
    let canonical = serde_json::to_vec(&(
        code.registers_size,
        code.ins_size,
        code.outs_size,
        masked,
        references,
        tries,
    ))
    .ok()?;
    Some((digest(&canonical), count))
}

fn annotation(
    dex: &DexFile,
    annotation: &dex_format::encoded_value::EncodedAnnotation,
    depth: usize,
) -> Option<serde_json::Value> {
    if depth > 64 {
        return None;
    }
    let mut elements = Vec::new();
    for element in &annotation.elements {
        elements.push((
            dex.string(element.name_idx).ok()?,
            value(dex, &element.value, depth + 1)?,
        ));
    }
    elements.sort_by(|a, b| a.0.cmp(b.0));
    Some(serde_json::json!([ty(dex, annotation.type_idx)?, elements]))
}

fn value(dex: &DexFile, value: &EncodedValue, depth: usize) -> Option<serde_json::Value> {
    if depth > 64 {
        return None;
    }
    use EncodedValueKind::*;
    Some(match &value.value {
        Byte(n) => serde_json::json!(["byte", n]),
        Short(n) => serde_json::json!(["short", n]),
        Char(n) => serde_json::json!(["char", n]),
        Int(n) => serde_json::json!(["int", n]),
        Long(n) => serde_json::json!(["long", n]),
        Float(n) => serde_json::json!(["float", n.to_bits()]),
        Double(n) => serde_json::json!(["double", n.to_bits()]),
        String(index) => serde_json::json!(["string", string(dex, *index)?]),
        Type(index) => serde_json::json!(["type", ty(dex, *index)?]),
        Field(index) => serde_json::json!(["field", field(dex, *index)?]),
        Enum(index) => serde_json::json!(["enum", field(dex, *index)?]),
        Method(index) => serde_json::json!(["method", method(dex, *index)?]),
        MethodType(index) => serde_json::json!(["proto", proto(dex, *index)?]),
        MethodHandle(_) => return None,
        Array(values) => serde_json::json!([
            "array",
            values
                .iter()
                .map(|item| self::value(dex, item, depth + 1))
                .collect::<Option<Vec<_>>>()?
        ]),
        Annotation(item) => serde_json::json!(["annotation", annotation(dex, item, depth + 1)?]),
        Null => serde_json::json!(["null"]),
        Boolean(n) => serde_json::json!(["boolean", n]),
    })
}

fn annotations(
    dex: &DexFile,
    set: &dex_format::annotations::AnnotationSet,
) -> Option<Vec<serde_json::Value>> {
    let mut result = Vec::new();
    for item in &set.entries {
        // Build-only annotations differ between compiler/shrinker configurations.
        if item.visibility == dex_format::annotations::Visibility::Build {
            continue;
        }
        result.push(serde_json::json!([
            format!("{:?}", item.visibility),
            annotation(dex, &item.annotation, 0)?
        ]));
    }
    result.sort_by_cached_key(serde_json::Value::to_string);
    Some(result)
}

/// Fingerprint original declarations and all original bodies. Source/debug
/// metadata and build-only annotations do not contribute to this origin evidence.
pub fn class_fingerprint(dex: &DexFile, class: &ClassDef) -> Option<ClassFingerprint> {
    let owner = ty(dex, class.class_idx)?;
    let parent = if class.superclass_idx == u32::MAX {
        None
    } else {
        Some(ty(dex, class.superclass_idx)?)
    };
    let interfaces = class
        .interfaces
        .as_ref()
        .map(|items| {
            items
                .types
                .iter()
                .map(|id| ty(dex, u32::from(*id)))
                .collect::<Option<Vec<_>>>()
        })
        .transpose_option()?;
    let data = class.class_data.as_ref()?;
    let mut fields = Vec::new();
    for (index, item) in data.static_fields.iter().enumerate() {
        let initial = class
            .static_values
            .as_ref()
            .and_then(|values| values.get(index))
            .map(|item| value(dex, item, 0))
            .transpose_option()?;
        fields.push((field(dex, item.field_idx)?, item.access_flags, initial));
    }
    for item in &data.instance_fields {
        fields.push((field(dex, item.field_idx)?, item.access_flags, None));
    }
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    let mut methods = Vec::new();
    let mut anchors = Vec::new();
    for item in data.direct_methods.iter().chain(&data.virtual_methods) {
        let name = method(dex, item.method_idx)?;
        let fingerprint = item
            .code
            .as_ref()
            .map(|code| body(dex, code))
            .transpose_option()?;
        if let Some((hash, count)) = &fingerprint
            && *count >= 8
            && !name.contains("-><init>")
            && !name.contains("-><clinit>")
        {
            anchors.push(hash.clone());
        }
        methods.push((name, item.access_flags, fingerprint.map(|(hash, _)| hash)));
    }
    methods.sort_by(|a, b| a.0.cmp(&b.0));
    let mut annotation_items = Vec::new();
    if let Some(directory) = &class.annotations {
        if let Some(set) = &directory.class_annotations {
            let retained = annotations(dex, set)?;
            if !retained.is_empty() {
                annotation_items.push(serde_json::json!(["class", retained]));
            }
        }
        for item in &directory.field_annotations {
            let retained = annotations(dex, &item.annotations)?;
            if !retained.is_empty() {
                annotation_items.push(serde_json::json!([field(dex, item.member_idx)?, retained]));
            }
        }
        for item in &directory.method_annotations {
            let retained = annotations(dex, &item.annotations)?;
            if !retained.is_empty() {
                annotation_items.push(serde_json::json!([method(dex, item.member_idx)?, retained]));
            }
        }
        for item in &directory.parameter_annotations {
            let sets = item
                .annotations
                .entries
                .iter()
                .map(|set| {
                    set.as_ref()
                        .map(|set| annotations(dex, set))
                        .transpose_option()
                        .map(|set| set.filter(|values| !values.is_empty()))
                })
                .collect::<Option<Vec<_>>>()?;
            if sets.iter().any(Option::is_some) {
                annotation_items.push(serde_json::json!([
                    "parameters",
                    method(dex, item.method_idx)?,
                    sets
                ]));
            }
        }
    }
    annotation_items.sort_by_cached_key(serde_json::Value::to_string);
    let canonical = serde_json::to_vec(&(
        KIND,
        &owner,
        class.access_flags,
        parent,
        interfaces,
        fields,
        methods,
        annotation_items,
    ))
    .ok()?;
    anchors.sort();
    anchors.dedup();
    Some(ClassFingerprint {
        owner,
        sha256: digest(&canonical),
        anchors,
    })
}

/// Nontrivial supported method bodies, even if another method in the class lacks
/// a complete fingerprint. Used to count ambiguous anchors across the whole APK.
pub fn class_anchors(dex: &DexFile, class: &ClassDef) -> Vec<String> {
    let Some(data) = &class.class_data else {
        return Vec::new();
    };
    let mut anchors = Vec::new();
    for item in data.direct_methods.iter().chain(&data.virtual_methods) {
        let Some(name) = method(dex, item.method_idx) else {
            continue;
        };
        if name.contains("-><init>") || name.contains("-><clinit>") {
            continue;
        }
        if let Some((hash, count)) = item.code.as_ref().and_then(|code| body(dex, code))
            && count >= 8
        {
            anchors.push(hash);
        }
    }
    anchors.sort();
    anchors.dedup();
    anchors
}

trait TransposeOption<T> {
    fn transpose_option(self) -> Option<Option<T>>;
}
impl<T> TransposeOption<T> for Option<Option<T>> {
    fn transpose_option(self) -> Option<Option<T>> {
        match self {
            None => Some(None),
            Some(Some(value)) => Some(Some(value)),
            Some(None) => None,
        }
    }
}
