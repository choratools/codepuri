//! Genuine empty interface declarations; never constructor/body recovery.
use super::{DeclarationUnsupported, class_declaration_with};
use dex_format::{DexFile, class::ClassDef};

/// A closed boundary preventing empty declaration publication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmptyTypeRequirement {
    /// This is not a pure, representable original interface.
    NotPureInterface,
    /// The original referenced metadata is absent or inconsistent.
    InvalidMetadata,
    /// The existing original-name/flag header cannot be rendered.
    Header(DeclarationUnsupported),
}

/// Original declaration provenance; no original executable identity is added.
#[derive(Clone, Debug)]
pub struct EmptyInterfacePlan {
    /// Existing validated header using the frozen source alias inventory.
    pub header: String,
    /// Genuine original descriptor.
    pub descriptor: String,
    /// Genuine original flags, which contain no synthetic/final/annotation bits.
    pub access_flags: u32,
}
impl EmptyInterfacePlan {
    /// This plan restores a declaration only, never an executable body.
    pub const fn declaration_only(&self) -> bool {
        true
    }
    /// A pure interface has no original method/constructor identities to recover.
    pub const fn original_defined_methods(&self) -> usize {
        0
    }
}

/// Validate a genuine methodless marker interface without inventing constructors,
/// inherited declarations, annotation semantics, static values or source flags.
pub fn plan_empty_interface(
    dex: &DexFile,
    class: &ClassDef,
    name: impl Fn(u32) -> Result<String, DeclarationUnsupported>,
) -> Result<EmptyInterfacePlan, EmptyTypeRequirement> {
    if class.access_flags != 0x600 && class.access_flags != 0x601 {
        return Err(EmptyTypeRequirement::NotPureInterface);
    }
    if class.annotations_off != 0
        || class.annotations.is_some()
        || class.static_values_off != 0
        || class.static_values.is_some()
        || class.interfaces_off != 0
        || class.interfaces.is_some()
    {
        return Err(EmptyTypeRequirement::NotPureInterface);
    }
    match (&class.class_data, class.class_data_off) {
        (None, 0) => {}
        (Some(data), offset)
            if offset != 0
                && data.static_fields.is_empty()
                && data.instance_fields.is_empty()
                && data.direct_methods.is_empty()
                && data.virtual_methods.is_empty() => {}
        _ => return Err(EmptyTypeRequirement::InvalidMetadata),
    }
    let raw = |index| {
        dex.type_id(index)
            .ok()
            .and_then(|t| dex.string(t.descriptor_idx).ok())
    };
    if raw(class.superclass_idx) != Some("Ljava/lang/Object;")
        || name(class.superclass_idx).ok().as_deref() != Some("java.lang.Object")
    {
        return Err(EmptyTypeRequirement::InvalidMetadata);
    }
    let descriptor = raw(class.class_idx)
        .filter(|s| s.starts_with('L') && s.ends_with(';'))
        .ok_or(EmptyTypeRequirement::InvalidMetadata)?
        .to_owned();
    let header = class_declaration_with(dex, class, name).map_err(EmptyTypeRequirement::Header)?;
    Ok(EmptyInterfacePlan {
        header,
        descriptor,
        access_flags: class.access_flags,
    })
}

#[cfg(test)]
#[path = "java_empty_type_publication_tests.rs"]
mod tests;
