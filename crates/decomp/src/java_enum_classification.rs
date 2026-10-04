//! Original enum ownership. Constant-specific subclasses own method bodies, not
//! the enclosing enum's constant creation or static initializer.
use super::descriptor;
use dex_format::{DexFile, class::ClassDef};

/// Original source representation that a parent plan must account for.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum EnumOwnership {
    /// Actual declaration directly extending java.lang.Enum.
    Direct,
    /// Exact child methods require the enclosing original enum's constant plan.
    ConstantBody {
        /// Original child descriptor, before aliases.
        child: String,
        /// Actual declaring parent descriptor, before aliases.
        parent: String,
        /// Every original method identity in the child's class-data domain.
        methods: Vec<u32>,
    },
}
/// Default-closed metadata boundary. This never approves a constructor/body.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum OwnershipError {
    /// Child flags or superclass descriptor unavailable/inconsistent.
    InvalidChild,
    /// Exact parent declaration absent or duplicated across complete inputs.
    ParentUnavailable,
    /// Exact parent is not an original direct enum declaration.
    ParentNotDirectEnum,
    /// Duplicate child method definitions cannot be assigned identities.
    DuplicateMethods,
}
/// Classify actual declarations before demanding an independent enum initializer.
pub fn classify(
    inputs: &[DexFile],
    dex: &DexFile,
    class: &ClassDef,
) -> Result<EnumOwnership, OwnershipError> {
    if class.access_flags & 0x4000 == 0 || class.access_flags & 0x200 != 0 {
        return Err(OwnershipError::InvalidChild);
    }
    let child = descriptor(dex, class.class_idx).map_err(|_| OwnershipError::InvalidChild)?;
    let parent = descriptor(dex, class.superclass_idx).map_err(|_| OwnershipError::InvalidChild)?;
    if parent == "Ljava/lang/Enum;" {
        return Ok(EnumOwnership::Direct);
    }
    let mut declarations = inputs
        .iter()
        .flat_map(|file| file.classes.iter().map(move |c| (file, c)))
        .filter(|(file, c)| descriptor(file, c.class_idx).ok() == Some(parent));
    let (parent_dex, declaration) = declarations
        .next()
        .ok_or(OwnershipError::ParentUnavailable)?;
    if declarations.next().is_some() {
        return Err(OwnershipError::ParentUnavailable);
    }
    if declaration.access_flags & 0x4000 == 0
        || declaration.access_flags & 0x200 != 0
        || descriptor(parent_dex, declaration.superclass_idx).ok() != Some("Ljava/lang/Enum;")
    {
        return Err(OwnershipError::ParentNotDirectEnum);
    }
    let mut methods = class
        .class_data
        .as_ref()
        .map(|d| {
            d.direct_methods
                .iter()
                .chain(&d.virtual_methods)
                .map(|m| m.method_idx)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let count = methods.len();
    methods.sort_unstable();
    methods.dedup();
    if methods.len() != count {
        return Err(OwnershipError::DuplicateMethods);
    }
    Ok(EnumOwnership::ConstantBody {
        child: child.into(),
        parent: parent.into(),
        methods,
    })
}
#[cfg(test)]
#[path = "java_enum_classification_tests.rs"]
mod tests;
