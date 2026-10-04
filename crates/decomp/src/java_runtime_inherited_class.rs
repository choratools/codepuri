//! Runtime declaration-only class lookup; JVM defaults remain a separate proof.
use super::*;
use crate::java_external_metadata::dex_metadata::InstanceClass;

pub(crate) fn resolve_runtime_class_instance(
    metadata: &MetadataChain<'_>,
    reference: &MethodKey,
) -> Result<InheritedPublicMethod, InheritedMethodError> {
    if reference.name.starts_with('<') || !valid_owner(&reference.owner) {
        return Err(InheritedMethodError::NoMethod(reference.clone()));
    }
    // Do not reopen failed JVM-only proofs through this declaration capability.
    if metadata
        .instance_class(&reference.owner)
        .is_none_or(|c| c.dex_code.is_none())
    {
        return Err(InheritedMethodError::Missing(reference.owner.clone()));
    }
    let mut owners = Vec::new();
    let mut seen = BTreeSet::new();
    let mut selected = None;
    let mut current = Some(reference.owner.clone());
    while let Some(owner) = current {
        if owners.len() >= 256 || !seen.insert(owner.clone()) || !valid_owner(&owner) {
            return Err(InheritedMethodError::Invalid(owner));
        }
        let class = metadata
            .instance_class(&owner)
            .ok_or_else(|| InheritedMethodError::Missing(owner.clone()))?;
        validate_class(&owner, &class)?;
        if owner == reference.owner && class.flags & 1 == 0 {
            return Err(InheritedMethodError::NoMethod(reference.clone()));
        }
        owners.push(owner.clone());
        if selected.is_none() {
            let family = class
                .methods
                .iter()
                .filter(|m| same_parameters(&m.key, reference))
                .collect::<Vec<_>>();
            if !family.is_empty() {
                if owner == reference.owner {
                    return Err(InheritedMethodError::ClassDominance(family[0].key.clone()));
                }
                let [method] = family.as_slice() else {
                    return Err(InheritedMethodError::Generic(owner));
                };
                if !same_descriptor(&method.key, reference) || method.signature.is_some() {
                    return Err(InheritedMethodError::Generic(owner));
                }
                let flags = method.metadata.access_flags;
                let bodyless = flags & (0x100 | 0x400);
                // DEX declared-synchronized is metadata, not a manufactured JVM flag.
                let allowed = 1 | 0x10 | 0x20 | 0x80 | 0x100 | 0x400 | 0x800 | 0x20000;
                let has_code = match class.dex_code {
                    Some(code) => *code
                        .get(&method.key)
                        .ok_or_else(|| InheritedMethodError::Invalid(owner.clone()))?,
                    None => method.code.as_ref().is_some_and(|c| !c.bytes.is_empty()),
                };
                if class.flags & 1 == 0
                    || flags & 1 == 0
                    || flags & !allowed != 0
                    || bodyless == 0x500
                    || method.metadata.exceptions.is_none()
                    || has_code != (bodyless == 0)
                    || (class.dex_code.is_some() && method.code.is_some())
                {
                    return Err(InheritedMethodError::NoMethod(reference.clone()));
                }
                selected = Some((*method).clone());
            }
        }
        current = class.declaration.superclass.as_deref().map(descriptor);
    }
    let declaration = selected.ok_or_else(|| InheritedMethodError::NoMethod(reference.clone()))?;
    Ok(InheritedPublicMethod::new(
        reference.clone(),
        declaration,
        owners,
        InheritedMethodKind::RuntimeClassInstance,
    ))
}

fn validate_class(owner: &str, class: &InstanceClass<'_>) -> Result<(), InheritedMethodError> {
    let h = class.declaration;
    if descriptor(&h.name) != owner
        || h.is_interface
        || class.flags & (0x200 | 0x8000) != 0
        || (owner == "Ljava/lang/Object;") != h.superclass.is_none()
        || h.interfaces.iter().collect::<BTreeSet<_>>().len() != h.interfaces.len()
        || h.interfaces.iter().any(|i| !valid_owner(&descriptor(i)))
        || class.methods.iter().any(|m| {
            m.key.owner != owner
                || m.descriptor != format!("({}){}", m.key.parameters.join(""), m.key.return_type)
        })
        || class
            .methods
            .iter()
            .map(|m| &m.key)
            .collect::<BTreeSet<_>>()
            .len()
            != class.methods.len()
    {
        return Err(InheritedMethodError::Invalid(owner.into()));
    }
    if class.signature.is_some() {
        return Err(InheritedMethodError::Generic(owner.into()));
    }
    Ok(())
}
