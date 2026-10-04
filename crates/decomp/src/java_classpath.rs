//! Bounds-checked JVM declarations and bounded bytecode evidence; never executes code.
use super::{ExternalMetadata, MethodMetadata};
use crate::{
    java_aliases::MethodKey,
    type_compatibility::{Declaration, Hierarchy, UnsupportedType},
};
use std::{
    collections::BTreeMap,
    fmt,
    io::{Cursor, Read},
    path::Path,
    sync::Arc,
};
#[path = "java_classpath_covariant_bridge.rs"]
mod covariant_bridge;

/// Failure to ingest an explicit classpath.
#[derive(Debug)]
pub enum ClasspathError {
    /// Filesystem or archive read failed.
    Io(std::io::Error),
    /// Invalid ZIP archive.
    Archive(zip::result::ZipError),
    /// Truncated or structurally invalid JVM input.
    Malformed {
        /// Byte offset where parsing failed (zero for pool-reference errors).
        offset: usize,
        /// Structural failure category.
        reason: &'static str,
    },
    /// Multiple definitions of the same class are ambiguous, even if identical.
    DuplicateClass(String),
    /// Unsupported explicit path kind.
    UnsupportedPath(String),
}
impl fmt::Display for ClasspathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ClasspathError {}
impl From<std::io::Error> for ClasspathError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<zip::result::ZipError> for ClasspathError {
    fn from(e: zip::result::ZipError) -> Self {
        Self::Archive(e)
    }
}

/// Exact method declaration, including optional generic Signature text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedMethod {
    /// Descriptor-based identity, including exact return type.
    pub key: MethodKey,
    /// JVM declaration flags and known Exceptions attribute (absent means empty).
    pub metadata: MethodMetadata,
    /// Exact JVM method descriptor.
    pub descriptor: String,
    /// Uninterpreted generic Signature attribute.
    pub signature: Option<String>,
    /// Exact bounded JVM Code attribute. Metadata-only DEX providers also return
    /// `None`; absence by itself does not prove a native or abstract method.
    pub code: Option<Arc<CodeMetadata>>,
}
/// One original exception-table entry; catch type zero means catch-all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeException {
    /// Protected start byte offset, inclusive.
    pub start_pc: u16,
    /// Protected end byte offset, exclusive.
    pub end_pc: u16,
    /// Handler instruction byte offset.
    pub handler_pc: u16,
    /// Genuine catch descriptor, or None for catch-all.
    pub catch_type: Option<String>,
}
/// Bounded method Code evidence; this is not a general JVM verifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeMetadata {
    /// Absolute original Code attribute payload offset.
    pub attribute_offset: usize,
    /// Exact original Code attribute payload length.
    pub attribute_length: usize,
    /// Original max_stack.
    pub max_stack: u16,
    /// Original max_locals.
    pub max_locals: u16,
    /// Exact original JVM instructions, bounded to 65,535 bytes.
    pub bytes: Vec<u8>,
    /// Original ordered exception table.
    pub exceptions: Vec<CodeException>,
    /// Bounded original nested attributes, including debug information.
    pub attributes: Vec<(String, Vec<u8>)>,
}
/// Actual Java17 Object constructor/finalizer Code evidence.
///
/// Obtain through ClassMetadata::object_return_only_evidence; descriptor names
/// alone and declarations without Code cannot create this evidence.
#[derive(Clone, Copy, Debug)]
pub struct ObjectReturnOnlyEvidence<'a> {
    class: &'a ClassMetadata,
    constructor: &'a ParsedMethod,
    finalizer: &'a ParsedMethod,
}
impl<'a> ObjectReturnOnlyEvidence<'a> {
    /// Complete parsed original Object declaration.
    pub fn class(&self) -> &'a ClassMetadata {
        self.class
    }
    /// Exact parsed public no-argument constructor, with return-only Code.
    pub fn constructor(&self) -> &'a ParsedMethod {
        self.constructor
    }
    /// Exact parsed protected finalizer, with return-only Code.
    pub fn finalizer(&self) -> &'a ParsedMethod {
        self.finalizer
    }
}
/// Exact field declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldMetadata {
    /// Actual declaring class descriptor, never a reference-site alias.
    pub owner: String,
    /// Original field name.
    pub name: String,
    /// JVM type descriptor.
    pub descriptor: String,
    /// JVM access flags.
    pub access_flags: u32,
    /// Uninterpreted generic Signature attribute.
    pub signature: Option<String>,
    /// Genuine ConstantValue attribute, not a guessed static initializer.
    pub constant_value: Option<ConstantValue>,
    /// Exact bounded attribute payloads, including unknown attributes.
    pub attributes: Vec<(String, Vec<u8>)>,
}
/// JVM constants retain floating-point bits and UTF-16 string units exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstantValue {
    /// CONSTANT_Integer.
    Integer(i32),
    /// CONSTANT_Long.
    Long(i64),
    /// CONSTANT_Float raw bits.
    Float(u32),
    /// CONSTANT_Double raw bits.
    Double(u64),
    /// CONSTANT_String, including isolated surrogates.
    String(Vec<u16>),
}
impl FieldMetadata {
    /// Java constant-variable semantics require static final primitive/String.
    /// Other fields must remain owner-qualified reads to preserve class init.
    pub fn inline_constant(&self) -> Option<&ConstantValue> {
        (self.access_flags & 0x18 == 0x18).then_some(())?;
        matches!(
            self.descriptor.as_str(),
            "Z" | "B" | "C" | "S" | "I" | "J" | "F" | "D" | "Ljava/lang/String;"
        )
        .then_some(())?;
        self.constant_value.as_ref()
    }
}
/// One genuine class-file declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassMetadata {
    /// Original class descriptor.
    pub descriptor: String,
    /// JVM class access flags.
    pub access_flags: u32,
    /// Class-file major version.
    pub major_version: u16,
    /// Attested direct hierarchy edges.
    pub declaration: Declaration,
    /// Uninterpreted generic Signature attribute.
    pub signature: Option<String>,
    /// Declared methods, not inferred inherited methods.
    pub methods: Vec<ParsedMethod>,
    /// Declared fields.
    pub fields: Vec<FieldMetadata>,
    /// Genuine InnerClasses entries: inner descriptor, outer descriptor, name.
    /// Empty outer/name records denote local/anonymous classes, not members.
    pub inner_classes: Vec<(String, String, String)>,
    // Private, byte-derived origin prevents public metadata edits fabricating a root proof.
    object_origin: Option<Box<ObjectCodeOrigin>>,
    // Byte-derived bridge identity, unaffected by public metadata mutation.
    bridge_origins: BTreeMap<MethodKey, covariant_bridge::BridgeOrigin>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct ObjectCodeOrigin {
    access_flags: u32,
    major_version: u16,
    declaration: Declaration,
    constructor: ParsedMethod,
    finalizer: ParsedMethod,
}

/// Explicit caller-provided classpath. Empty by default; no platform fabrication.
#[derive(Clone, Debug, Default)]
pub struct Classpath {
    classes: BTreeMap<String, ClassMetadata>,
}
impl Classpath {
    /// Parse one actual class file, rejecting duplicate definitions atomically.
    pub fn add_class(&mut self, bytes: &[u8]) -> Result<(), ClasspathError> {
        self.insert(ClassMetadata::parse(bytes)?)
    }
    fn insert(&mut self, class: ClassMetadata) -> Result<(), ClasspathError> {
        if self.classes.contains_key(&class.descriptor) {
            return Err(ClasspathError::DuplicateClass(class.descriptor));
        }
        self.classes.insert(class.descriptor.clone(), class);
        Ok(())
    }
    /// Load only the paths explicitly opted into by a caller/CLI. Supports class,
    /// JAR/ZIP, and local JMOD files; no subprocesses, downloads, or ambient JDK.
    /// Explicit path order is classpath precedence: the first complete class
    /// declaration wins. Members from shadowed platform classes are never merged.
    pub fn from_paths<P: AsRef<Path>>(
        paths: impl IntoIterator<Item = P>,
    ) -> Result<Self, ClasspathError> {
        let mut result = Self::default();
        for path in paths {
            let mut entry = Self::default();
            entry.add_path(path)?;
            for (name, class) in entry.classes {
                result.classes.entry(name).or_insert(class);
            }
        }
        Ok(result)
    }
    /// Transactionally add a class/JAR/JMOD path. Multi-release JARs use base
    /// declarations only; version overlays are never guessed without a target.
    pub fn add_path(&mut self, path: impl AsRef<Path>) -> Result<(), ClasspathError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)?;
        let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        let mut staged = Self::default();
        match extension {
            "class" => staged.add_class(&bytes)?,
            "jar" | "zip" | "jmod" => {
                let data = if extension == "jmod" {
                    if !bytes.starts_with(b"JM\x01\x00") {
                        return Err(bad(0, "invalid JMOD header"));
                    }
                    &bytes[4..]
                } else {
                    &bytes[..]
                };
                let mut archive = zip::ZipArchive::new(Cursor::new(data))?;
                for i in 0..archive.len() {
                    let mut entry = archive.by_index(i)?;
                    let name = entry.name();
                    if !name.ends_with(".class")
                        || name.starts_with("META-INF/")
                        || name.ends_with("module-info.class")
                    {
                        continue;
                    }
                    if extension == "jmod" && !name.starts_with("classes/") {
                        continue;
                    }
                    let mut class = Vec::new();
                    entry.read_to_end(&mut class)?;
                    staged.add_class(&class)?;
                }
            }
            _ => return Err(ClasspathError::UnsupportedPath(path.display().to_string())),
        }
        for name in staged.classes.keys() {
            if self.classes.contains_key(name) {
                return Err(ClasspathError::DuplicateClass(name.clone()));
            }
        }
        self.classes.extend(staged.classes);
        Ok(())
    }
    /// Original descriptor lookup; no inferred placeholder classes.
    pub fn class(&self, descriptor: &str) -> Option<&ClassMetadata> {
        self.classes.get(descriptor)
    }
    /// Exact constructor access flags; constructors are never inherited.
    pub fn constructor_access(&self, key: &MethodKey) -> Option<u32> {
        if key.name != "<init>" {
            return None;
        }
        self.class(&key.owner)?
            .methods
            .iter()
            .find(|m| &m.key == key)
            .map(|m| m.metadata.access_flags)
    }
    /// All explicitly loaded, trusted classes in deterministic order.
    pub fn classes(&self) -> impl Iterator<Item = &ClassMetadata> {
        self.classes.values()
    }
    /// Build the existing hierarchy adapter from trusted class declarations.
    pub fn to_hierarchy(&self) -> Result<Hierarchy, UnsupportedType> {
        let mut h = Hierarchy::default();
        for c in self.classes.values() {
            h.insert(c.declaration.clone())?;
        }
        Ok(h)
    }
}
impl ExternalMetadata for Classpath {
    fn inherited_public_instance(&self, key: &MethodKey) -> Option<super::InheritedPublicMethod> {
        super::inherited_methods::resolve_inherited_public_instance(self, key).ok()
    }

    fn attests_covariant_bridge(&self, bridge: &MethodKey, target: &MethodKey) -> bool {
        self.class(&bridge.owner)
            .is_some_and(|class| class.attests_covariant_bridge(bridge, target))
    }
    fn class_generic_signature(&self, owner: &str) -> Option<Option<String>> {
        Some(self.class(owner)?.signature.clone())
    }
    fn declared_methods(&self, owner: &str) -> Option<Vec<ParsedMethod>> {
        Some(self.class(owner)?.methods.clone())
    }
    fn class_hierarchy(&self, descriptor: &str) -> Vec<Declaration> {
        self.class(descriptor)
            .map(|c| vec![c.declaration.clone()])
            .unwrap_or_default()
    }
    fn declared_fields(&self, owner: &str) -> Option<Vec<FieldMetadata>> {
        Some(self.class(owner)?.fields.clone())
    }
    fn field(&self, key: &crate::java_aliases::FieldKey) -> Option<FieldMetadata> {
        self.class(&key.owner)?
            .fields
            .iter()
            .find(|f| f.name == key.name && f.descriptor == key.descriptor)
            .cloned()
    }
    fn member_source_names(&self) -> BTreeMap<String, String> {
        fn source(cp: &Classpath, descriptor: &str, seen: &mut Vec<String>) -> Option<String> {
            if seen.iter().any(|d| d == descriptor) {
                return None;
            }
            seen.push(descriptor.into());
            let class = cp.class(descriptor)?;
            let entries: Vec<_> = class
                .inner_classes
                .iter()
                .filter(|e| e.0 == descriptor)
                .collect();
            match entries.as_slice() {
                [] => Some(class.declaration.name.clone()),
                [entry] => {
                    let (_, outer, name) = entry;
                    // Reject malformed relationships and names, without inventing aliases.
                    if crate::java_descriptors::source_identifier(name).is_err()
                        || descriptor != format!("{}${name};", outer.strip_suffix(';')?)
                    {
                        return None;
                    }
                    Some(format!("{}.{name}", source(cp, outer, seen)?))
                }
                _ => None,
            }
        }
        self.classes
            .keys()
            .filter_map(|d| {
                let name = source(self, d, &mut Vec::new())?;
                (name != self.classes[d].declaration.name).then(|| (d.clone(), name))
            })
            .collect()
    }
    fn class_access(&self, descriptor: &str) -> Option<u32> {
        self.class(descriptor).map(|c| c.access_flags)
    }
    fn hierarchy(&self) -> Vec<Declaration> {
        self.classes
            .values()
            .map(|c| c.declaration.clone())
            .collect()
    }
    fn method(&self, key: &MethodKey) -> Option<MethodMetadata> {
        self.class(&key.owner)?
            .methods
            .iter()
            .find(|m| &m.key == key)
            .map(|m| m.metadata.clone())
    }
}

fn bad(offset: usize, reason: &'static str) -> ClasspathError {
    ClasspathError::Malformed { offset, reason }
}
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    base: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ClasspathError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| bad(self.base + self.pos, "length overflow"))?;
        let result = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| bad(self.base + self.pos, "truncated input"))?;
        self.pos = end;
        Ok(result)
    }
    fn u1(&mut self) -> Result<u8, ClasspathError> {
        Ok(self.take(1)?[0])
    }
    fn u2(&mut self) -> Result<u16, ClasspathError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u4(&mut self) -> Result<u32, ClasspathError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn done(&self) -> Result<(), ClasspathError> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(bad(self.base + self.pos, "trailing attribute/input bytes"))
        }
    }
}
#[derive(Clone)]
enum Cp {
    Empty,
    Utf(Option<String>),
    Class(u16),
    Ref(u8, u16, u16),
    One(u8, u16),
    Handle(u8, u16),
    Value(ConstantValue),
    StringUnits(Vec<u16>),
}
fn utf(cp: &[Cp], i: u16) -> Result<&str, ClasspathError> {
    match cp.get(usize::from(i)) {
        Some(Cp::Utf(Some(s))) => Ok(s),
        Some(Cp::Utf(None)) => Err(bad(0, "unrepresentable declaration UTF16")),
        _ => Err(bad(0, "invalid UTF8 pool reference")),
    }
}
fn internal(cp: &[Cp], i: u16) -> Result<String, ClasspathError> {
    let Some(Cp::Class(n)) = cp.get(usize::from(i)) else {
        return Err(bad(0, "invalid class pool reference"));
    };
    let s = utf(cp, *n)?;
    if s.is_empty()
        || s.starts_with('/')
        || s.ends_with('/')
        || s.split('/').any(|p| p.is_empty())
        || s.contains(['.', ';', '['])
    {
        return Err(bad(0, "invalid internal class name"));
    }
    Ok(s.to_owned())
}
// JVM CONSTANT_Utf8 is modified UTF-8 over UTF-16, not standard UTF-8.
fn mutf_units(bytes: &[u8]) -> Result<Vec<u16>, ClasspathError> {
    let mut units = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        i += 1;
        let unit = if (1..128).contains(&b) {
            u16::from(b)
        } else if b & 0xe0 == 0xc0 {
            let c = *bytes
                .get(i)
                .ok_or_else(|| bad(i, "truncated modified UTF8"))?;
            i += 1;
            if c & 0xc0 != 0x80 {
                return Err(bad(i, "invalid modified UTF8"));
            }
            let v = (u16::from(b & 31) << 6) | u16::from(c & 63);
            if v < 128 && !(b == 0xc0 && c == 0x80) {
                return Err(bad(i, "overlong modified UTF8"));
            }
            v
        } else if b & 0xf0 == 0xe0 {
            let c = *bytes
                .get(i)
                .ok_or_else(|| bad(i, "truncated modified UTF8"))?;
            let d = *bytes
                .get(i + 1)
                .ok_or_else(|| bad(i, "truncated modified UTF8"))?;
            i += 2;
            if c & 0xc0 != 0x80 || d & 0xc0 != 0x80 {
                return Err(bad(i, "invalid modified UTF8"));
            }
            let v = (u16::from(b & 15) << 12) | (u16::from(c & 63) << 6) | u16::from(d & 63);
            if v < 2048 {
                return Err(bad(i, "overlong modified UTF8"));
            }
            v
        } else {
            return Err(bad(i, "invalid modified UTF8"));
        };
        units.push(unit);
    }
    // Legal string literals may contain isolated surrogates. They are opaque
    // metadata-irrelevant constants; never replace characters in declarations.
    Ok(units)
}
fn type_end(bytes: &[u8], pos: &mut usize, void: bool) -> Result<(), ClasspathError> {
    let start = *pos;
    let mut arrays = 0;
    while bytes.get(*pos) == Some(&b'[') {
        arrays += 1;
        *pos += 1;
    }
    if arrays > 255 {
        return Err(bad(start, "too many array dimensions"));
    }
    match bytes.get(*pos).copied() {
        Some(b'V') if void && arrays == 0 => *pos += 1,
        Some(b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z') => *pos += 1,
        Some(b'L') => {
            *pos += 1;
            let begin = *pos;
            while let Some(b) = bytes.get(*pos) {
                if *b == b';' {
                    break;
                }
                *pos += 1;
            }
            let name = std::str::from_utf8(&bytes[begin..*pos])
                .map_err(|_| bad(start, "invalid descriptor"))?;
            if bytes.get(*pos) != Some(&b';')
                || name.is_empty()
                || name.contains(['.', '[', '(', ')'])
                || name.split('/').any(|p| p.is_empty())
            {
                return Err(bad(start, "invalid class descriptor"));
            }
            *pos += 1;
        }
        _ => return Err(bad(start, "invalid type descriptor")),
    }
    Ok(())
}
fn method_descriptor(s: &str) -> Result<(Vec<String>, String), ClasspathError> {
    let b = s.as_bytes();
    if b.first() != Some(&b'(') {
        return Err(bad(0, "invalid method descriptor"));
    }
    let mut pos = 1;
    let mut args = Vec::new();
    while b.get(pos) != Some(&b')') {
        let start = pos;
        type_end(b, &mut pos, false)?;
        args.push(s[start..pos].to_owned());
    }
    pos += 1;
    let start = pos;
    type_end(b, &mut pos, true)?;
    if pos != b.len() {
        return Err(bad(pos, "trailing descriptor"));
    }
    Ok((args, s[start..pos].to_owned()))
}
struct Attributes {
    signature: Option<String>,
    exceptions: Option<Vec<String>>,
    inner_classes: Vec<(String, String, String)>,
    constant_index: Option<u16>,
    raw: Vec<(String, Vec<u8>)>,
    code: Option<Arc<CodeMetadata>>,
}
fn attributes(
    r: &mut Reader<'_>,
    cp: &[Cp],
    method: bool,
    field: bool,
) -> Result<Attributes, ClasspathError> {
    let mut out = Attributes {
        signature: None,
        exceptions: None,
        inner_classes: Vec::new(),
        constant_index: None,
        raw: Vec::new(),
        code: None,
    };
    for _ in 0..r.u2()? {
        let name = utf(cp, r.u2()?)?;
        let len = r.u4()? as usize;
        let base = r.base + r.pos;
        let mut a = Reader {
            bytes: r.take(len)?,
            pos: 0,
            base,
        };
        if field {
            out.raw.push((name.to_owned(), a.bytes.to_vec()));
        }
        match name {
            "Code" => {
                if !method || out.code.is_some() {
                    return Err(bad(base, "invalid or duplicate Code"));
                }
                out.code = Some(Arc::new(parse_code_attribute(&mut a, cp)?));
            }
            "ConstantValue" if field => {
                if out.constant_index.is_some() {
                    return Err(bad(base, "duplicate ConstantValue"));
                }
                out.constant_index = Some(a.u2()?);
                a.done()?;
            }
            "InnerClasses" => {
                for _ in 0..a.u2()? {
                    let inner = a.u2()?;
                    let outer = a.u2()?;
                    let name = a.u2()?;
                    let _flags = a.u2()?;
                    // Anonymous/local classes have no source-level member name.
                    if inner != 0 {
                        out.inner_classes.push((
                            format!("L{};", internal(cp, inner)?),
                            if outer == 0 {
                                String::new()
                            } else {
                                format!("L{};", internal(cp, outer)?)
                            },
                            if name == 0 {
                                String::new()
                            } else {
                                utf(cp, name)?.to_owned()
                            },
                        ));
                    }
                }
                a.done()?;
            }
            "Signature" => {
                if out.signature.is_some() {
                    return Err(bad(base, "duplicate Signature"));
                }
                out.signature = Some(utf(cp, a.u2()?)?.to_owned());
                a.done()?;
            }
            "Exceptions" if method => {
                if out.exceptions.is_some() {
                    return Err(bad(base, "duplicate Exceptions"));
                }
                let mut exceptions = Vec::new();
                for _ in 0..a.u2()? {
                    exceptions.push(format!("L{};", internal(cp, a.u2()?)?));
                }
                a.done()?;
                out.exceptions = Some(exceptions);
            }
            _ => {} // Other attributes are bounded, opaque payloads.
        }
    }
    Ok(out)
}

fn parse_code_attribute(r: &mut Reader<'_>, cp: &[Cp]) -> Result<CodeMetadata, ClasspathError> {
    let attribute_offset = r.base;
    let attribute_length = r.bytes.len();
    let max_stack = r.u2()?;
    let max_locals = r.u2()?;
    let length = r.u4()? as usize;
    if !(1..=65_535).contains(&length) {
        return Err(bad(r.base + r.pos, "invalid Code length"));
    }
    let bytes = r.take(length)?.to_vec();
    let mut exceptions = Vec::new();
    for _ in 0..r.u2()? {
        let start_pc = r.u2()?;
        let end_pc = r.u2()?;
        let handler_pc = r.u2()?;
        let catch = r.u2()?;
        if start_pc >= end_pc || usize::from(end_pc) > length || usize::from(handler_pc) >= length {
            return Err(bad(r.base + r.pos, "invalid Code exception range"));
        }
        exceptions.push(CodeException {
            start_pc,
            end_pc,
            handler_pc,
            catch_type: if catch == 0 {
                None
            } else {
                Some(format!("L{};", internal(cp, catch)?))
            },
        });
    }
    let mut attributes = Vec::new();
    for _ in 0..r.u2()? {
        let name = utf(cp, r.u2()?)?.to_owned();
        let size = r.u4()? as usize;
        if name == "Code" {
            return Err(bad(r.base + r.pos, "nested Code"));
        }
        attributes.push((name, r.take(size)?.to_vec()));
    }
    r.done()?;
    Ok(CodeMetadata {
        attribute_offset,
        attribute_length,
        max_stack,
        max_locals,
        bytes,
        exceptions,
        attributes,
    })
}
impl ClassMetadata {
    /// Attest only actual Java17 Object return-only method Code.
    ///
    /// This attests the explicit parsed classpath artifact. It never attests an
    /// Android ART runtime, an implicit platform, or an input DEX pool reference.
    pub fn object_return_only_evidence(&self) -> Option<ObjectReturnOnlyEvidence<'_>> {
        let origin = self.object_origin.as_ref()?;
        let evidence = self.object_return_only_shape()?;
        (self.access_flags == origin.access_flags
            && self.major_version == origin.major_version
            && self.declaration == origin.declaration
            && evidence.constructor == &origin.constructor
            && evidence.finalizer == &origin.finalizer)
            .then_some(evidence)
    }
    fn object_return_only_shape(&self) -> Option<ObjectReturnOnlyEvidence<'_>> {
        if self.descriptor != "Ljava/lang/Object;"
            || self.major_version != 61
            || self.access_flags != 0x21
            || self.declaration.superclass.is_some()
            || !self.declaration.interfaces.is_empty()
            || !self.fields.is_empty()
        {
            return None;
        }
        let constructor = self
            .methods
            .iter()
            .find(|m| m.key.name == "<init>" && m.descriptor == "()V")?;
        let finalizer = self
            .methods
            .iter()
            .find(|m| m.key.name == "finalize" && m.descriptor == "()V")?;
        if self
            .methods
            .iter()
            .filter(|m| m.key.name == "<init>" || m.key.name == "finalize")
            .count()
            != 2
            || constructor.metadata.access_flags != 1
            || finalizer.metadata.access_flags != 4
        {
            return None;
        }
        for method in [constructor, finalizer] {
            let code = method.code.as_ref()?;
            if code.bytes != [0xb1]
                || code.max_stack != 0
                || code.max_locals != 1
                || !code.exceptions.is_empty()
                || code.attributes.iter().any(|(name, _)| {
                    !matches!(
                        name.as_str(),
                        "LineNumberTable" | "LocalVariableTable" | "LocalVariableTypeTable"
                    )
                })
            {
                return None;
            }
        }
        Some(ObjectReturnOnlyEvidence {
            class: self,
            constructor,
            finalizer,
        })
    }
    /// Parse class-file declarations with checked lengths, pool references, and
    /// descriptors and bounded Code attributes; this is not a bytecode verifier.
    pub fn parse(bytes: &[u8]) -> Result<Self, ClasspathError> {
        let mut r = Reader {
            bytes,
            pos: 0,
            base: 0,
        };
        if r.u4()? != 0xcafebabe {
            return Err(bad(0, "invalid class magic"));
        }
        let _minor = r.u2()?;
        let major_version = r.u2()?;
        if major_version < 45 {
            return Err(bad(6, "invalid class version"));
        }
        let count = r.u2()?;
        if count == 0 {
            return Err(bad(8, "empty constant pool"));
        }
        let mut cp = vec![Cp::Empty];
        while cp.len() < usize::from(count) {
            let tag = r.u1()?;
            let value = match tag {
                1 => {
                    let n = r.u2()?;
                    let units = mutf_units(r.take(usize::from(n))?)?;
                    match String::from_utf16(&units) {
                        Ok(s) => Cp::Utf(Some(s)),
                        Err(_) => Cp::StringUnits(units),
                    }
                }
                3 => Cp::Value(ConstantValue::Integer(r.u4()? as i32)),
                4 => Cp::Value(ConstantValue::Float(r.u4()?)),
                5 | 6 => {
                    let bits = (u64::from(r.u4()?) << 32) | u64::from(r.u4()?);
                    cp.push(Cp::Value(if tag == 5 {
                        ConstantValue::Long(bits as i64)
                    } else {
                        ConstantValue::Double(bits)
                    }));
                    if cp.len() >= usize::from(count) {
                        return Err(bad(r.pos, "missing wide constant slot"));
                    }
                    Cp::Empty
                }
                7 => Cp::Class(r.u2()?),
                8 | 16 | 19 | 20 => Cp::One(tag, r.u2()?),
                9 | 10 | 11 | 12 | 17 | 18 => Cp::Ref(tag, r.u2()?, r.u2()?),
                15 => Cp::Handle(r.u1()?, r.u2()?),
                _ => return Err(bad(r.pos - 1, "unknown constant pool tag")),
            };
            cp.push(value);
        }
        let entry = |i: u16| {
            cp.get(usize::from(i))
                .filter(|v| !matches!(v, Cp::Empty))
                .ok_or_else(|| bad(0, "invalid constant pool reference"))
        };
        for c in &cp {
            match c {
                Cp::Class(i) | Cp::One(16 | 19 | 20, i) => {
                    utf(&cp, *i)?;
                }
                Cp::One(8, i) if !matches!(entry(*i)?, Cp::Utf(_) | Cp::StringUnits(_)) => {
                    return Err(bad(0, "invalid string pool reference"));
                }
                Cp::Ref(tag, a, b) => {
                    if *tag == 12 {
                        utf(&cp, *a)?;
                        utf(&cp, *b)?;
                    } else {
                        if *tag != 17 && *tag != 18 && !matches!(entry(*a)?, Cp::Class(_)) {
                            return Err(bad(0, "invalid member owner"));
                        }
                        if !matches!(entry(*b)?, Cp::Ref(12, _, _)) {
                            return Err(bad(0, "invalid name/type reference"));
                        }
                    }
                }
                Cp::Handle(kind, index)
                    if !(1..=9).contains(kind)
                        || !matches!(entry(*index)?, Cp::Ref(9..=11, _, _)) =>
                {
                    return Err(bad(0, "invalid method handle"));
                }
                _ => {}
            }
        }
        let access_flags = u32::from(r.u2()?);
        let name = internal(&cp, r.u2()?)?;
        let descriptor = format!("L{name};");
        let super_index = r.u2()?;
        let superclass = if super_index == 0 {
            None
        } else {
            Some(internal(&cp, super_index)?.replace('/', "."))
        };
        let mut interfaces = Vec::new();
        for _ in 0..r.u2()? {
            interfaces.push(internal(&cp, r.u2()?)?.replace('/', "."));
        }
        let mut fields = Vec::new();
        let field_owner = descriptor.clone();
        for _ in 0..r.u2()? {
            let flags = u32::from(r.u2()?);
            let name = utf(&cp, r.u2()?)?.to_owned();
            let descriptor = utf(&cp, r.u2()?)?.to_owned();
            let mut p = 0;
            type_end(descriptor.as_bytes(), &mut p, false)?;
            if p != descriptor.len() {
                return Err(bad(r.pos, "invalid field descriptor"));
            }
            if fields
                .iter()
                .any(|f: &FieldMetadata| f.name == name && f.descriptor == descriptor)
            {
                return Err(bad(r.pos, "duplicate field"));
            }
            let a = attributes(&mut r, &cp, false, true)?;
            let constant_value = a
                .constant_index
                .map(|index| {
                    let value = match cp.get(usize::from(index)) {
                        Some(Cp::Value(v)) => v.clone(),
                        Some(Cp::One(8, i)) => match cp.get(usize::from(*i)) {
                            Some(Cp::Utf(Some(s))) => {
                                ConstantValue::String(s.encode_utf16().collect())
                            }
                            Some(Cp::StringUnits(s)) => ConstantValue::String(s.clone()),
                            _ => return Err(bad(0, "invalid ConstantValue string")),
                        },
                        _ => return Err(bad(0, "invalid ConstantValue pool reference")),
                    };
                    let valid = matches!(
                        (descriptor.as_str(), &value),
                        ("Z" | "B" | "C" | "S" | "I", ConstantValue::Integer(_))
                            | ("J", ConstantValue::Long(_))
                            | ("F", ConstantValue::Float(_))
                            | ("D", ConstantValue::Double(_))
                            | ("Ljava/lang/String;", ConstantValue::String(_))
                    );
                    if !valid {
                        return Err(bad(0, "ConstantValue descriptor mismatch"));
                    }
                    Ok(value)
                })
                .transpose()?;
            fields.push(FieldMetadata {
                owner: field_owner.clone(),
                name,
                descriptor,
                access_flags: flags,
                signature: a.signature,
                constant_value,
                attributes: a.raw,
            });
        }
        let mut methods = Vec::new();
        for _ in 0..r.u2()? {
            let flags = u32::from(r.u2()?);
            let name = utf(&cp, r.u2()?)?.to_owned();
            let desc = utf(&cp, r.u2()?)?.to_owned();
            let (parameters, return_type) = method_descriptor(&desc)?;
            if (name == "<init>" && (return_type != "V" || flags & 8 != 0))
                || (name == "<clinit>" && (desc != "()V" || flags & 8 == 0))
            {
                return Err(bad(r.pos, "invalid initializer"));
            }
            let key = MethodKey {
                owner: descriptor.clone(),
                name,
                parameters,
                return_type,
            };
            if methods.iter().any(|m: &ParsedMethod| m.key == key) {
                return Err(bad(r.pos, "duplicate method"));
            }
            let a = attributes(&mut r, &cp, true, false)?;
            if (flags & (0x100 | 0x400) != 0) == a.code.is_some() {
                return Err(bad(r.pos, "method Code/access mismatch"));
            }
            methods.push(ParsedMethod {
                key,
                descriptor: desc,
                signature: a.signature,
                code: a.code,
                metadata: MethodMetadata {
                    access_flags: flags,
                    exceptions: Some(a.exceptions.unwrap_or_default()),
                },
            });
        }
        let a = attributes(&mut r, &cp, false, false)?;
        r.done()?;
        let mut parsed = Self {
            descriptor,
            access_flags,
            major_version,
            declaration: Declaration {
                name: name.replace('/', "."),
                superclass,
                interfaces,
                is_interface: access_flags & 0x200 != 0,
            },
            signature: a.signature,
            methods,
            fields,
            inner_classes: a.inner_classes,
            object_origin: None,
            bridge_origins: BTreeMap::new(),
        };
        parsed.bridge_origins = covariant_bridge::origins(&parsed, &cp);
        if let Some(evidence) = parsed.object_return_only_shape() {
            parsed.object_origin = Some(Box::new(ObjectCodeOrigin {
                access_flags: parsed.access_flags,
                major_version: parsed.major_version,
                declaration: parsed.declaration.clone(),
                constructor: evidence.constructor.clone(),
                finalizer: evidence.finalizer.clone(),
            }));
        }
        Ok(parsed)
    }
}
