//! Descriptor-attested class literals. Resolution uses instruction types,
//! rather than class-only lookup, so primitive and array targets are retained.
use crate::{JavaResolver, JavaType, Unsupported};
use managed_ir::refs::TypeRef;

/// Produces the exact target's class literal using the caller's validated type
/// renderer. No reflective lookup or constructor is substituted. Java class
/// literals do not initialize the target class.
pub fn expression(
    resolver: &impl JavaResolver,
    target: TypeRef,
    render_type: impl FnOnce(&JavaType) -> Result<String, Unsupported>,
) -> Result<String, Unsupported> {
    let ty = resolver
        .instruction_type(target)
        .ok_or_else(|| Unsupported::Metadata("unresolved class-literal target".into()))?;
    let mut component = &ty;
    while let JavaType::Array(element) = component {
        component = element;
        if *component == JavaType::Void {
            return Err(Unsupported::Metadata("void array class literal".into()));
        }
    }
    Ok(format!("{}.class", render_type(&ty)?))
}
