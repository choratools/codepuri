//! Static call spelling retaining original fields that shadow a package root.
//! The pure null primary is discarded; its cast is exclusively a type context.
//! JVM null checkcast performs no resolution before argument evaluation.
use crate::{JavaMethod, Unsupported, qualified};

/// Exact declaring-type evidence supplied by the metadata owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeclaringType {
    Class,
    Interface,
    Unknown,
}

/// Only an attested class may use a primary for static invocation. Static
/// interface methods require a type qualifier and cannot use this spelling.
pub(crate) fn owner(
    method: &JavaMethod,
    type_context: bool,
    declaring_type: DeclaringType,
) -> Result<String, Unsupported> {
    if !method.is_static || matches!(method.name.as_str(), "<init>" | "<clinit>") {
        return Err(Unsupported::Metadata(
            "static method qualifier dispatch".into(),
        ));
    }
    let owner = qualified(&method.owner)?;
    if !type_context {
        return Ok(owner);
    }
    if declaring_type != DeclaringType::Class {
        return Err(Unsupported::Metadata(
            "static method qualifier requires declaring class".into(),
        ));
    }
    Ok(format!("(({owner}) null)"))
}

#[cfg(test)]
#[path = "static_method_qualifier_tests.rs"]
mod tests;

/// Preserve independent lexical and exact-owner metadata evidence.
pub(crate) fn resolved_owner(
    method: &JavaMethod,
    reference: managed_ir::refs::MethodRef,
    caller: &str,
    resolver: &impl crate::JavaResolver,
) -> Result<String, Unsupported> {
    let declaring_type = match resolver.static_method_owner_access(reference) {
        Some(flags) if flags & 0x200 == 0 => DeclaringType::Class,
        Some(_) => DeclaringType::Interface,
        None => DeclaringType::Unknown,
    };
    owner(
        method,
        resolver.static_method_type_context(reference, caller),
        declaring_type,
    )
}
