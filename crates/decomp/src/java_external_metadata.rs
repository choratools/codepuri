//! Explicit, trusted classpath declarations for one Java export input.
use crate::{
    dex_java::{JavaOptions, JavaProfile, JavaReport},
    java_aliases::MethodKey,
    output::{OutputError, SourceSink},
};
use dex_format::DexFile;
use std::collections::BTreeMap;

#[path = "java_classpath.rs"]
pub mod classpath;
pub use classpath::ConstantValue;
pub use classpath::{ClassMetadata, Classpath, ClasspathError, FieldMetadata, ParsedMethod};
#[path = "java_dex_metadata.rs"]
pub mod dex_metadata;
pub use dex_metadata::{DexMetadata, DexMetadataError, MetadataChain, MetadataProvider};
#[path = "java_external_fields.rs"]
pub mod fields;
pub use fields::{
    FieldAccessContext, FieldOrigin, FieldRequirement, ResolvedField, resolve_field,
    retain_field_receiver,
};

#[path = "java_external_inherited_methods.rs"]
pub mod inherited_methods;
pub use inherited_methods::InheritedPublicMethod;

/// Declaration flags and exact exception descriptors. `None` means unknown,
/// whereas `Some(vec![])` explicitly asserts no declared exceptions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodMetadata {
    /// Original DEX/JVM declaration access flags, including staticness.
    pub access_flags: u32,
    /// Exact exception class descriptors, or unknown metadata.
    pub exceptions: Option<Vec<String>>,
}

/// Genuine declarations from explicit compiler or runtime metadata inputs.
/// Keys are original DEX descriptors, never renamed Java source identifiers.
/// Runtime DEX metadata proves declarations, not availability to `javac`;
/// consumers must assess compilation dependencies separately.
pub trait ExternalMetadata: Sync {
    /// Distinct, descriptor-exact inherited public instance method evidence.
    /// Exact `method()`/`method_access()` remain declared-only. The proof retains
    /// the symbolic reference and actual declaring class/interface; callers must not
    /// change symbolic receiver typing, devirtualize its body, or change its opcode.
    /// Missing/cyclic/ambiguous/generic metadata stays closed. Runtime DEX can
    /// separately prove class-instance declarations, never JVM interface defaults,
    /// concrete receiver dispatch, or availability to the Java compiler.
    fn inherited_public_instance(&self, _key: &MethodKey) -> Option<InheritedPublicMethod> {
        None
    }

    /// Attest an exact synthetic return-covariance bridge from genuine body
    /// bytes. Flags or a shadowed provider's declaration alone are insufficient.
    fn attests_covariant_bridge(&self, _bridge: &MethodKey, _target: &MethodKey) -> bool {
        false
    }
    /// Complete genuine method declarations, including generic/varargs metadata.
    fn declared_methods(&self, _owner: &str) -> Option<Vec<ParsedMethod>> {
        None
    }
    /// Exact declared field only; inheritance is resolved separately.
    fn field(&self, _key: &crate::java_aliases::FieldKey) -> Option<FieldMetadata> {
        None
    }
    /// Complete declared field list needed to prove Java name-based lookup.
    fn declared_fields(&self, _owner: &str) -> Option<Vec<FieldMetadata>> {
        None
    }
    /// Attested source names for member classes, derived from genuine nesting
    /// metadata rather than splitting binary names on `$`.
    fn member_source_names(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }
    /// Exact declaration access without requiring exception or body metadata.
    fn method_access(&self, key: &MethodKey) -> Option<u32> {
        self.method(key).map(|method| method.access_flags)
    }
    /// Genuine class Signature: outer None is missing/ambiguous evidence;
    /// Some(None) is an attested nongeneric declaration. Runtime metadata
    /// defaults closed because it does not prove compiler class-file spelling.
    fn class_generic_signature(&self, _descriptor: &str) -> Option<Option<String>> {
        None
    }
    /// Trusted original class access flags, when available.
    fn class_access(&self, _descriptor: &str) -> Option<u32> {
        None
    }
    /// Exact constructor access; constructors cannot be inherited.
    fn constructor_access(&self, key: &MethodKey) -> Option<u32> {
        (key.name == "<init>")
            .then(|| self.method(key))
            .flatten()
            .map(|m| m.access_flags)
    }
    /// Explicit classpath hierarchy declarations in original dotted class names.
    fn hierarchy(&self) -> Vec<crate::type_compatibility::Declaration> {
        Vec::new()
    }
    /// Exact original hierarchy lookup; multiple entries remain ambiguous.
    /// Providers may override this to avoid cloning a whole platform catalog.
    fn class_hierarchy(&self, descriptor: &str) -> Vec<crate::type_compatibility::Declaration> {
        self.hierarchy()
            .into_iter()
            .filter(|d| format!("L{};", d.name.replace('.', "/")) == descriptor)
            .collect()
    }
    /// Return an exact declaration; a pool reference alone is insufficient.
    fn method(&self, key: &MethodKey) -> Option<MethodMetadata>;
}
impl ExternalMetadata for BTreeMap<MethodKey, MethodMetadata> {
    fn method(&self, key: &MethodKey) -> Option<MethodMetadata> {
        self.get(key).cloned()
    }
}

/// Typed information required before an external call can be emitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetadataRequirement {
    /// Invalid method-pool reference.
    InvalidReference(u32),
    /// No unambiguous trusted declaration exists.
    Declaration(MethodKey),
    /// The declaration exists but its exception list is unavailable.
    Exceptions(MethodKey),
}

/// Resolve original method keys without guessing exceptions or dispatch flags.
/// Input declarations take precedence over classpath declarations. This also
/// provides the typed requirement underlying an export's diagnostic text.
pub fn resolve_method(
    files: &[DexFile],
    key: &MethodKey,
    external: &dyn ExternalMetadata,
) -> Result<MethodMetadata, MetadataRequirement> {
    let catalog = crate::java_integration::method_catalog(files);
    let input_owners = files
        .iter()
        .flat_map(|dex| {
            dex.classes.iter().filter(move |class| {
                dex.type_id(class.class_idx)
                    .ok()
                    .and_then(|ty| dex.string(ty.descriptor_idx).ok())
                    == Some(key.owner.as_str())
            })
        })
        .count();
    if input_owners > 1 {
        return Err(MetadataRequirement::Declaration(key.clone()));
    }
    let declaration = match catalog.get(key) {
        Some(value) => value.clone(),
        None if input_owners != 0 => None,
        None => external.method(key),
    }
    .ok_or_else(|| MetadataRequirement::Declaration(key.clone()))?;
    if declaration.exceptions.is_none() {
        return Err(MetadataRequirement::Exceptions(key.clone()));
    }
    Ok(declaration)
}

/// Export with declarations from all supplied DEX files (one trusted APK/input)
/// and caller-provided classpath metadata. Referenced verified classes may still
/// need compile dependencies when their bodies fall outside the honest subset.
pub fn emit_with_metadata(
    files: &[DexFile],
    options: &JavaOptions,
    sink: &mut impl SourceSink,
    profile: Option<&mut JavaProfile>,
    metadata: &dyn ExternalMetadata,
) -> Result<JavaReport, OutputError> {
    crate::java_integration::emit_with_metadata(files, options, sink, profile, metadata)
}
