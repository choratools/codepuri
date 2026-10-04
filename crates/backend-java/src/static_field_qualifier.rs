//! Static field spelling when a lexical variable shadows a package root.
//!
//! JLS 15.11.1 discards a static field's primary, even when it is null. The
//! exact owner is placed in a type context; no receiver value, declaration name
//! or field flags change. JVMS checkcast does not resolve its type for null.
use crate::{JavaField, JavaType, Unsupported, qualified};

#[cfg(test)]
#[path = "static_field_qualifier_tests.rs"]
mod tests;

pub(crate) fn owner(field: &JavaField, type_context: bool) -> Result<String, Unsupported> {
    if !field.is_static || field.ty == JavaType::Void {
        return Err(Unsupported::Metadata(
            "static field qualifier dispatch".into(),
        ));
    }
    let owner = qualified(&field.owner)?;
    Ok(if type_context {
        format!("(({owner}) null)")
    } else {
        owner
    })
}
