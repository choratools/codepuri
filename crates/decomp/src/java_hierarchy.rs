//! Hierarchy ingestion from all APK DEX pools and explicitly supplied classpath declarations.
use crate::type_compatibility::{Declaration, Hierarchy, UnsupportedType};
use backend_java::JavaType;
use dex_format::DexFile;
use std::collections::BTreeMap;

fn class_name(dex: &DexFile, index: u32) -> Result<String, UnsupportedType> {
    let descriptor = dex
        .type_id(index)
        .ok()
        .and_then(|t| dex.string(t.descriptor_idx).ok())
        .ok_or(UnsupportedType::InvalidType)?;
    if !descriptor.starts_with('L') || !descriptor.ends_with(';') || descriptor.len() < 3 {
        return Err(UnsupportedType::InvalidType);
    }
    Ok(descriptor[1..descriptor.len() - 1].replace('/', "."))
}

/// Pass every DEX extracted from an APK, plus declarations obtained from the
/// supplied classpath. References in a type pool are not treated as definitions.
pub fn build_hierarchy(
    dexes: &[&DexFile],
    classpath: impl IntoIterator<Item = Declaration>,
) -> Result<Hierarchy, UnsupportedType> {
    let mut hierarchy = Hierarchy::default();
    let mut inputs = BTreeMap::new();
    for dex in dexes {
        for class in &dex.classes {
            let declaration = Declaration {
                name: class_name(dex, class.class_idx)?,
                superclass: if class.superclass_idx == u32::MAX {
                    None
                } else {
                    Some(class_name(dex, class.superclass_idx)?)
                },
                interfaces: class
                    .interfaces
                    .as_ref()
                    .map(|list| {
                        list.types
                            .iter()
                            .map(|i| class_name(dex, u32::from(*i)))
                            .collect()
                    })
                    .transpose()?
                    .unwrap_or_default(),
                is_interface: class.access_flags & 0x200 != 0,
            };
            if inputs
                .insert(declaration.name.clone(), declaration.clone())
                .is_some()
            {
                return Err(UnsupportedType::ConflictingDeclaration(declaration.name));
            }
        }
    }
    for declaration in classpath {
        if !inputs.contains_key(&declaration.name) {
            hierarchy.insert(declaration)?;
        }
    }
    for declaration in inputs.into_values() {
        hierarchy.insert(declaration)?;
    }
    Ok(hierarchy)
}

/// Convenience for facade callers holding source-level class names.
pub fn class_type(name: impl Into<String>) -> JavaType {
    JavaType::Class(name.into())
}

/// Adapt trusted external declarations and all input DEX definitions into one
/// hierarchy, applying the same input-owner precedence and duplicate rejection.
pub fn build_with_metadata(
    dexes: &[&DexFile],
    metadata: &dyn crate::java_external_metadata::ExternalMetadata,
) -> Result<Hierarchy, UnsupportedType> {
    build_hierarchy(dexes, metadata.hierarchy())
}
