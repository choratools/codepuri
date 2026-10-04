//! Evidence-only DEX nested-class/outer-capture recognition. No source publication.
use dex_decompiler::decode::{DecodedInsn, Opcode, decode_method};
use dex_format::{
    DexFile,
    annotations::{AnnotationItem, Visibility},
    class::ClassDef,
    encoded_value::EncodedValueKind,
};
use std::collections::{BTreeMap, BTreeSet};

/// Genuine enclosing metadata classification; binary-name spelling is irrelevant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InnerClassKind {
    /// Named class with EnclosingClass metadata.
    Member {
        /// Genuine simple name.
        name: String,
    },
    /// Named class with EnclosingMethod metadata.
    Local {
        /// Genuine simple name.
        name: String,
        /// DEX-local enclosing method identity.
        method_index: u32,
    },
    /// Null InnerClass name, with genuine enclosing context.
    Anonymous {
        /// Optional DEX-local enclosing method identity.
        method_index: Option<u32>,
    },
}
/// Descriptor-exact field identity, independent of local pool numbering.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CaptureField {
    /// Actual declaring class descriptor.
    pub owner: String,
    /// Original field name.
    pub name: String,
    /// Exact outer-reference descriptor.
    pub ty: String,
}
/// A checked pattern candidate, never a global exclusivity/initialization token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OuterCaptureCandidate {
    /// Exact class descriptor.
    pub descriptor: String,
    /// Annotation-backed classification.
    pub kind: InnerClassKind,
    /// Genuine enclosing class descriptor.
    pub enclosing_class: String,
    /// Original InnerClass access flags.
    pub inner_access_flags: u32,
    /// Exact capture declaration.
    pub field: CaptureField,
    /// DEX-local capture field index.
    pub field_index: u32,
    /// DEX-local original constructor index.
    pub constructor_index: u32,
    /// First capture-store code-unit offset.
    pub capture_offset: u32,
    /// Exact direct-super reference index.
    pub super_method_index: u32,
    /// Original direct-super call offset.
    pub super_offset: u32,
    /// Native, unavailable, or undecodable bodies retained as arbitrary uncertainty.
    pub unknown_write_bodies: usize,
    /// Unresolved field operands retained as conservative uncertainty.
    pub unresolved_field_writes: usize,
}
/// Typed denial; no annotation/name or register guesses are made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InnerClassError {
    /// Absent, duplicate, inconsistent, or out-of-bounds genuine metadata.
    Metadata(&'static str),
    /// Static nested classes do not carry this outer-instance pattern.
    Static,
    /// The original constructor does not have the bounded prefix shape.
    Prefix(&'static str),
    /// Known stores include an additional write or incomplete field resolution.
    Writes,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Store {
    dex: usize,
    method: u32,
    offset: u32,
}
/// Reusable all-input known-write inventory. Arbitrary unknown capabilities
/// remain explicit; this catalog does not certify complete initialization.
#[derive(Default)]
pub struct OuterCaptureInventory {
    catalog: FieldCatalog,
    writes: BTreeMap<CaptureField, Vec<Store>>,
    unresolved: BTreeSet<(String, String)>,
    unknown_bodies: usize,
    unresolved_writes: usize,
}
impl OuterCaptureInventory {
    /// Scan every supplied original body once, before any source filtering.
    pub fn scan(files: &[DexFile]) -> Self {
        let mut out = Self {
            catalog: FieldCatalog::new(files),
            ..Self::default()
        };
        for (dex_index, dex) in files.iter().enumerate() {
            for class in &dex.classes {
                if let Some(data) = &class.class_data {
                    for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                        let Some(code) = &method.code else {
                            if method.access_flags & 0x400 == 0 {
                                out.unknown_bodies += 1;
                            }
                            continue;
                        };
                        let Ok(decoded) = decode_method(code) else {
                            out.unknown_bodies += 1;
                            continue;
                        };
                        if decoded.iter().any(|insn| {
                            matches!(byte(insn), Some(0x6e..=0x78))
                                && insn
                                    .pool_index
                                    .and_then(|i| dex.method_ids.get(i as usize))
                                    .and_then(|m| descriptor(dex, u32::from(m.class_idx)).ok())
                                    .is_some_and(|owner| {
                                        owner.starts_with("Ljava/lang/reflect/")
                                            || owner.starts_with("Ljava/lang/invoke/")
                                    })
                        }) {
                            out.unknown_bodies += 1;
                        }
                        for insn in decoded {
                            let Some(op) = byte(&insn) else {
                                continue;
                            };
                            if !matches!(op,0x59..=0x5f|0x67..=0x6d) {
                                continue;
                            }
                            let Some(index) = insn.pool_index else {
                                out.unresolved_writes += 1;
                                continue;
                            };
                            match field_identity(&out.catalog, dex, index) {
                                Some(field) => out.writes.entry(field).or_default().push(Store {
                                    dex: dex_index,
                                    method: method.method_idx,
                                    offset: insn.offset,
                                }),
                                None => {
                                    out.unresolved_writes += 1;
                                    if let Some(id) = dex.field_ids.get(index as usize)
                                        && let (Ok(name), Ok(ty)) = (
                                            dex.string(id.name_idx),
                                            descriptor(dex, u32::from(id.type_idx)),
                                        )
                                    {
                                        out.unresolved.insert((name.into(), ty));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }
}
fn byte(insn: &DecodedInsn) -> Option<u8> {
    match insn.opcode {
        Opcode::Supported(info) => Some(info.opcode),
        _ => None,
    }
}
fn descriptor(dex: &DexFile, index: u32) -> Result<String, InnerClassError> {
    let id = dex
        .type_ids
        .get(index as usize)
        .ok_or(InnerClassError::Metadata("type index"))?;
    dex.string(id.descriptor_idx)
        .map(str::to_owned)
        .map_err(|_| InnerClassError::Metadata("type string"))
}
#[derive(Default)]
struct FieldCatalog {
    declarations: BTreeMap<CaptureField, usize>,
    parents: BTreeMap<String, (Option<String>, bool)>,
    duplicates: BTreeSet<String>,
}
impl FieldCatalog {
    fn new(files: &[DexFile]) -> Self {
        let mut out = Self::default();
        for dex in files {
            for class in &dex.classes {
                let Ok(owner) = descriptor(dex, class.class_idx) else {
                    continue;
                };
                let parent = descriptor(dex, class.superclass_idx).ok();
                if out
                    .parents
                    .insert(
                        owner.clone(),
                        (
                            parent,
                            class
                                .interfaces
                                .as_ref()
                                .is_some_and(|i| !i.types.is_empty()),
                        ),
                    )
                    .is_some()
                {
                    out.duplicates.insert(owner.clone());
                }
                if let Some(data) = &class.class_data {
                    for field in data.instance_fields.iter().chain(&data.static_fields) {
                        let Some(id) = dex.field_ids.get(field.field_idx as usize) else {
                            continue;
                        };
                        let (Ok(name), Ok(ty)) = (
                            dex.string(id.name_idx),
                            descriptor(dex, u32::from(id.type_idx)),
                        ) else {
                            continue;
                        };
                        if u32::from(id.class_idx) != class.class_idx {
                            continue;
                        }
                        *out.declarations
                            .entry(CaptureField {
                                owner: owner.clone(),
                                name: name.into(),
                                ty,
                            })
                            .or_default() += 1;
                    }
                }
            }
        }
        out
    }
}
fn field_identity(catalog: &FieldCatalog, dex: &DexFile, index: u32) -> Option<CaptureField> {
    let id = dex.field_ids.get(index as usize)?;
    let mut key = CaptureField {
        owner: descriptor(dex, u32::from(id.class_idx)).ok()?,
        name: dex.string(id.name_idx).ok()?.into(),
        ty: descriptor(dex, u32::from(id.type_idx)).ok()?,
    };
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(key.owner.clone()) || catalog.duplicates.contains(&key.owner) {
            return None;
        }
        if let Some(count) = catalog.declarations.get(&key) {
            return (*count == 1).then_some(key);
        }
        let (parent, interfaces) = catalog.parents.get(&key.owner)?;
        if *interfaces {
            return None;
        }
        key.owner = parent.clone()?;
    }
}
fn annotation<'a>(
    dex: &DexFile,
    class: &'a ClassDef,
    wanted: &str,
) -> Result<Option<&'a AnnotationItem>, InnerClassError> {
    let Some(set) = class
        .annotations
        .as_ref()
        .and_then(|a| a.class_annotations.as_ref())
    else {
        return Ok(None);
    };
    let mut found = None;
    for item in &set.entries {
        if descriptor(dex, item.annotation.type_idx)? == wanted {
            if found.is_some() || item.visibility != Visibility::System {
                return Err(InnerClassError::Metadata(
                    "system annotation visibility/duplicate",
                ));
            }
            found = Some(item);
        }
    }
    Ok(found)
}
fn elements<'a>(
    dex: &DexFile,
    item: &'a AnnotationItem,
    expected: &[&str],
) -> Result<BTreeMap<String, &'a EncodedValueKind>, InnerClassError> {
    let mut out = BTreeMap::new();
    for e in &item.annotation.elements {
        let name = dex
            .string(e.name_idx)
            .map_err(|_| InnerClassError::Metadata("annotation element name"))?
            .to_owned();
        if !expected.contains(&name.as_str()) || out.insert(name, &e.value.value).is_some() {
            return Err(InnerClassError::Metadata("annotation elements"));
        }
    }
    if out.len() != expected.len() {
        return Err(InnerClassError::Metadata("missing annotation element"));
    }
    Ok(out)
}
/// Recognize one original capture constructor using genuine metadata and a
/// reusable complete-input known-write inventory. Returned uncertainty must not
/// be interpreted as a complete-class, access, or global initialization proof.
pub fn recognize_outer_capture(
    files: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructor_index: u32,
    inventory: &OuterCaptureInventory,
) -> Result<OuterCaptureCandidate, InnerClassError> {
    let dex = files
        .get(dex_index)
        .ok_or(InnerClassError::Metadata("dex index"))?;
    let own = descriptor(dex, class.class_idx)?;
    let inner = annotation(dex, class, "Ldalvik/annotation/InnerClass;")?
        .ok_or(InnerClassError::Metadata("InnerClass absent"))?;
    let inner = elements(dex, inner, &["accessFlags", "name"])?;
    let EncodedValueKind::Int(flags) = inner["accessFlags"] else {
        return Err(InnerClassError::Metadata("inner access flags"));
    };
    let flags =
        u32::try_from(*flags).map_err(|_| InnerClassError::Metadata("inner access flags"))?;
    if flags & 8 != 0 || class.access_flags & 8 != 0 {
        return Err(InnerClassError::Static);
    }
    let name = match inner["name"] {
        EncodedValueKind::Null => None,
        EncodedValueKind::String(i) => Some(
            dex.string(*i)
                .map_err(|_| InnerClassError::Metadata("inner name"))?
                .to_owned(),
        ),
        _ => return Err(InnerClassError::Metadata("inner name")),
    };
    let enclosing_class = annotation(dex, class, "Ldalvik/annotation/EnclosingClass;")?;
    let enclosing_method = annotation(dex, class, "Ldalvik/annotation/EnclosingMethod;")?;
    let (outer, method) = match (enclosing_class, enclosing_method) {
        (Some(a), None) => {
            let e = elements(dex, a, &["value"])?;
            let EncodedValueKind::Type(i) = e["value"] else {
                return Err(InnerClassError::Metadata("enclosing class value"));
            };
            (descriptor(dex, *i)?, None)
        }
        (None, Some(a)) => {
            let e = elements(dex, a, &["value"])?;
            let EncodedValueKind::Method(i) = e["value"] else {
                return Err(InnerClassError::Metadata("enclosing method value"));
            };
            let id = dex
                .method_ids
                .get(*i as usize)
                .ok_or(InnerClassError::Metadata("enclosing method index"))?;
            (descriptor(dex, u32::from(id.class_idx))?, Some(*i))
        }
        _ => {
            return Err(InnerClassError::Metadata(
                "exactly one enclosing annotation required",
            ));
        }
    };
    if outer == own {
        return Err(InnerClassError::Metadata("self enclosing class"));
    }
    let kind = match (name, method) {
        (Some(name), None) => InnerClassKind::Member { name },
        (Some(name), Some(method_index)) => InnerClassKind::Local { name, method_index },
        (None, method_index) => InnerClassKind::Anonymous { method_index },
    };
    for d in files {
        for c in &d.classes {
            if descriptor(d, c.class_idx)? == outer
                && let Some(a) = annotation(d, c, "Ldalvik/annotation/MemberClasses;")?
            {
                let e = elements(d, a, &["value"])?;
                let EncodedValueKind::Array(v) = e["value"] else {
                    return Err(InnerClassError::Metadata("MemberClasses value"));
                };
                let mut members = BTreeSet::new();
                for value in v {
                    let EncodedValueKind::Type(i) = value.value else {
                        return Err(InnerClassError::Metadata("MemberClasses element"));
                    };
                    if !members.insert(descriptor(d, i)?) {
                        return Err(InnerClassError::Metadata("duplicate member class"));
                    }
                }
                if matches!(kind, InnerClassKind::Member { .. }) && !members.contains(&own) {
                    return Err(InnerClassError::Metadata("MemberClasses inconsistent"));
                }
            }
        }
    }
    let data = class
        .class_data
        .as_ref()
        .ok_or(InnerClassError::Metadata("class data"))?;
    let method = data
        .direct_methods
        .iter()
        .find(|m| m.method_idx == constructor_index)
        .ok_or(InnerClassError::Metadata("constructor declaration"))?;
    if method.access_flags & 0x10000 == 0 || method.access_flags & 8 != 0 {
        return Err(InnerClassError::Metadata("constructor flags"));
    }
    let id = dex
        .method_ids
        .get(constructor_index as usize)
        .ok_or(InnerClassError::Metadata("method index"))?;
    if u32::from(id.class_idx) != class.class_idx || dex.string(id.name_idx).ok() != Some("<init>")
    {
        return Err(InnerClassError::Metadata("constructor identity"));
    }
    let proto = dex
        .proto_id(u32::from(id.proto_idx))
        .map_err(|_| InnerClassError::Metadata("constructor prototype"))?;
    let params = proto
        .parameters
        .as_ref()
        .ok_or(InnerClassError::Metadata("outer parameter absent"))?;
    if descriptor(
        dex,
        u32::from(
            *params
                .types
                .first()
                .ok_or(InnerClassError::Metadata("outer parameter absent"))?,
        ),
    )? != outer
        || descriptor(dex, proto.return_type_idx)? != "V"
    {
        return Err(InnerClassError::Metadata("outer parameter/return type"));
    }
    let original = dex
        .classes
        .iter()
        .find(|c| c.class_idx == class.class_idx)
        .and_then(|c| c.class_data.as_ref())
        .and_then(|d| {
            d.direct_methods
                .iter()
                .find(|m| m.method_idx == constructor_index)
        })
        .ok_or(InnerClassError::Metadata("original constructor missing"))?;
    if original.code != method.code {
        return Err(InnerClassError::Metadata("inventory input/body mismatch"));
    }
    let code = method
        .code
        .as_ref()
        .ok_or(InnerClassError::Prefix("body absent"))?;
    if !code.tries.is_empty() {
        return Err(InnerClassError::Prefix("constructor catches"));
    }
    let mut widths = Vec::new();
    for t in &params.types {
        widths.push(
            if matches!(descriptor(dex, u32::from(*t))?.as_str(), "J" | "D") {
                2
            } else {
                1
            },
        );
    }
    let incoming = 1 + widths.iter().sum::<u16>();
    if incoming != code.ins_size {
        return Err(InnerClassError::Metadata("incoming signature width"));
    }
    let receiver = code
        .registers_size
        .checked_sub(incoming)
        .ok_or(InnerClassError::Prefix("incoming frame"))?;
    let decoded =
        decode_method(code).map_err(|_| InnerClassError::Prefix("malformed instructions"))?;
    let first = decoded
        .first()
        .ok_or(InnerClassError::Prefix("empty body"))?;
    if byte(first) != Some(0x5b) || first.registers != [receiver + 1, receiver] {
        return Err(InnerClassError::Prefix(
            "first store not incoming outer to this",
        ));
    }
    let field_index = first
        .pool_index
        .ok_or(InnerClassError::Metadata("capture field index"))?;
    let declaration = data
        .instance_fields
        .iter()
        .find(|f| f.field_idx == field_index)
        .ok_or(InnerClassError::Metadata("capture declaration"))?;
    if declaration.access_flags & 0x1010 != 0x1010 || declaration.access_flags & !(0x1010 | 7) != 0
    {
        return Err(InnerClassError::Metadata(
            "capture must be synthetic final nonstatic",
        ));
    }
    let field = field_identity(&inventory.catalog, dex, field_index)
        .ok_or(InnerClassError::Metadata("capture owner"))?;
    if field.owner != own || field.ty != outer {
        return Err(InnerClassError::Metadata("capture type/owner"));
    }
    if inventory
        .unresolved
        .contains(&(field.name.clone(), field.ty.clone()))
        || inventory.writes.get(&field)
            != Some(&vec![Store {
                dex: dex_index,
                method: constructor_index,
                offset: first.offset,
            }])
    {
        return Err(InnerClassError::Writes);
    }
    let mut origins = (0..code.registers_size)
        .map(|r| (r >= receiver).then_some(r))
        .collect::<Vec<_>>();
    let mut register_types = vec![None; usize::from(code.registers_size)];
    register_types[usize::from(receiver)] = Some(own.clone());
    let mut word = receiver + 1;
    for (ty, width) in params.types.iter().zip(&widths) {
        register_types[usize::from(word)] = Some(descriptor(dex, u32::from(*ty))?);
        word += width;
    }
    let mut super_call = None;
    for insn in decoded.iter().skip(1) {
        let op = byte(insn).ok_or(InnerClassError::Prefix("unknown instruction"))?;
        if matches!(op,0x04..=0x06|0x16..=0x19) {
            return Err(InnerClassError::Prefix("wide prefix arguments unsupported"));
        }
        if matches!(op, 0x70 | 0x76) {
            let target = insn
                .pool_index
                .ok_or(InnerClassError::Metadata("super method index"))?;
            let id = dex
                .method_ids
                .get(target as usize)
                .ok_or(InnerClassError::Metadata("super method index"))?;
            if u32::from(id.class_idx) != class.superclass_idx
                || dex.string(id.name_idx).ok() != Some("<init>")
                || insn
                    .registers
                    .first()
                    .and_then(|r| origins.get(*r as usize))
                    .copied()
                    .flatten()
                    != Some(receiver)
            {
                return Err(InnerClassError::Prefix("not direct superclass initializer"));
            }
            if insn
                .registers
                .iter()
                .skip(1)
                .any(|r| origins.get(*r as usize).copied().flatten() == Some(receiver))
            {
                return Err(InnerClassError::Prefix("uninitialized this argument"));
            }
            let target_proto = dex
                .proto_id(u32::from(id.proto_idx))
                .map_err(|_| InnerClassError::Metadata("super prototype"))?;
            if descriptor(dex, target_proto.return_type_idx)? != "V" {
                return Err(InnerClassError::Metadata("super return type"));
            }
            let target_params = target_proto
                .parameters
                .as_ref()
                .map(|p| p.types.as_slice())
                .unwrap_or(&[]);
            if insn.registers.len() != target_params.len() + 1
                || insn
                    .registers
                    .iter()
                    .any(|r| origins.get(*r as usize).copied().flatten().is_none())
            {
                return Err(InnerClassError::Prefix("super argument availability/arity"));
            }
            for (register, ty) in insn.registers.iter().skip(1).zip(target_params) {
                let expected = descriptor(dex, u32::from(*ty))?;
                if matches!(expected.as_str(), "J" | "D") {
                    return Err(InnerClassError::Prefix("wide super arguments unsupported"));
                }
                if register_types[usize::from(*register)].as_ref() != Some(&expected) {
                    return Err(InnerClassError::Prefix("unproved super argument type"));
                }
            }
            super_call = Some((target, insn.offset));
            break;
        }
        if matches!(op, 0x01..=0x09) {
            if insn.registers.len() != 2 {
                return Err(InnerClassError::Prefix("copy operands"));
            }
            let d = insn.registers[0] as usize;
            let s = insn.registers[1] as usize;
            if d == receiver as usize {
                return Err(InnerClassError::Prefix("overwrites this"));
            }
            origins[d] = origins[s];
            register_types[d] = register_types[s].clone();
            if matches!(op, 0x04..=0x06) {
                if d + 1 >= origins.len() || s + 1 >= origins.len() {
                    return Err(InnerClassError::Prefix("wide copy"));
                }
                origins[d + 1] = origins[s + 1];
            }
        } else if matches!(op, 0x12..=0x19) {
            let d = *insn
                .registers
                .first()
                .ok_or(InnerClassError::Prefix("constant operands"))? as usize;
            if d == receiver as usize {
                return Err(InnerClassError::Prefix("overwrites this"));
            }
            origins[d] = Some(u16::MAX);
            register_types[d] = Some("I".into());
            if op >= 0x16 {
                if d + 1 >= origins.len() {
                    return Err(InnerClassError::Prefix("wide constant"));
                }
                origins[d + 1] = Some(u16::MAX);
            }
        } else {
            return Err(InnerClassError::Prefix(
                "effect/control before superclass call",
            ));
        }
    }
    let (super_method_index, super_offset) =
        super_call.ok_or(InnerClassError::Prefix("super initializer absent"))?;
    Ok(OuterCaptureCandidate {
        descriptor: own,
        kind,
        enclosing_class: outer,
        inner_access_flags: flags,
        field,
        field_index,
        constructor_index,
        capture_offset: first.offset,
        super_method_index,
        super_offset,
        unknown_write_bodies: inventory.unknown_bodies,
        unresolved_field_writes: inventory.unresolved_writes,
    })
}
