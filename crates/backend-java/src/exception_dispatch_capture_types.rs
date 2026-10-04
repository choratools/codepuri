//! Original caught-object evidence narrows only compatible capture lineages.
use super::*;

pub(super) fn domain(
    caught: &JavaType,
    previous: Option<&JavaType>,
    inferred: Option<&JavaType>,
    hierarchy: &crate::type_compatibility::Hierarchy,
) -> Result<JavaType, Error> {
    let assignable = |a, b| hierarchy.assignable(a, b) == Compatibility::Assignable;
    let throwable = JavaType::Class("java.lang.Throwable".into());
    if !assignable(caught, &throwable) || inferred.is_some_and(|bound| !assignable(caught, bound)) {
        return Err(Error::Unsupported("handler capture type mismatch"));
    }
    let domain = match previous {
        None => caught.clone(),
        Some(bound) if assignable(caught, bound) => bound.clone(),
        Some(bound) if assignable(bound, caught) => caught.clone(),
        Some(bound) if assignable(bound, &throwable) => {
            if let Some(common) = inferred
                && assignable(bound, common)
                && assignable(common, &throwable)
            {
                common.clone()
            } else {
                throwable.clone()
            }
        }
        Some(_) => return Err(Error::Unsupported("handler capture ancestry unavailable")),
    };
    if inferred.is_some_and(|bound| !assignable(&domain, bound)) {
        return Err(Error::Unsupported("shared handler capture type mismatch"));
    }
    Ok(domain)
}

pub(super) fn compatible_lineage(
    ir: &MethodIr<SsaForm>,
    result: ValueId,
    ty: &JavaType,
    hierarchy: &crate::type_compatibility::Hierarchy,
    resolver: &impl JavaResolver,
) -> Result<bool, Error> {
    let inferred = ir.value(result).map_err(invalid)?.ty();
    if inferred == &managed_ir::types::ManagedType::Unknown {
        return Ok(true);
    }
    let inferred =
        crate::managed_type(inferred, resolver).map_err(|e| Error::Invalid(format!("{e:?}")))?;
    Ok(hierarchy.assignable(ty, &inferred) == Compatibility::Assignable)
}
