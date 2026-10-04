//! Exact input-only source selection for privately proved final bridge aliases.
use super::*;
use crate::java_aliases::FinalBridgeAliasProof;

pub(super) fn select(
    caller: &str,
    reference: &MethodKey,
    provider: &impl SuperMetadata,
    aliases: &JavaAliases,
    hierarchy: &Hierarchy,
    proof: &FinalBridgeAliasProof,
) -> Result<SuperSelection, SuperError> {
    if !provider.attests_final_bridge_alias(proof.bridge(), proof.bridge_code(), proof.target()) {
        return Err(SuperError::Aliases);
    }
    let class = provider.class(caller).ok_or(SuperError::Metadata)?;
    if class.access_flags & 0x200 != 0 {
        return Err(SuperError::Interface);
    }
    let mut next = class.superclass;
    let mut ancestry = Vec::new();
    let mut seen = BTreeSet::new();
    let mut owner_seen = false;
    let mut selected = None;
    while let Some(owner) = next {
        if seen.len() >= 256 || !seen.insert(owner.clone()) {
            return Err(SuperError::Metadata);
        }
        let class = provider.class(&owner).ok_or(SuperError::Metadata)?;
        if class.access_flags & 0x200 != 0 {
            return Err(SuperError::Interface);
        }
        ancestry.push(owner.clone());
        owner_seen |= owner == reference.owner;
        let mut candidates = Vec::new();
        for method in &class.methods {
            // DEX resolution still uses the original narrow descriptor. The
            // source spelling cannot hide a closer original override.
            if method.key.name == proof.target().name
                && method.key.parameters == proof.target().parameters
                && method.key.return_type == proof.target().return_type
                && method.key != *proof.target()
            {
                return Err(SuperError::Selection);
            }
            match aliases.method_name(&method.key) {
                Some(MethodAlias::Ordinary(name)) if name.as_str() == proof.typed_name() => {
                    candidates.push(method)
                }
                None if method.key.name == proof.typed_name() => return Err(SuperError::Aliases),
                _ => {}
            }
        }
        if !candidates.is_empty() {
            let [method] = candidates.as_slice() else {
                return Err(SuperError::Selection);
            };
            if method.key != *proof.target()
                || method.metadata.access_flags != 0x11
                || method.key.parameters != reference.parameters
                || method.key.return_type != reference.return_type
            {
                return Err(SuperError::Selection);
            }
            if method.generic
                && !provider.source_super_signature_is_erased(caller, &ancestry, &method.key)
            {
                return Err(SuperError::Overload);
            }
            selected = Some((*method).clone());
            break;
        }
        next = class.superclass;
    }
    if !owner_seen {
        return Err(SuperError::NotAncestor);
    }
    let method = selected.ok_or(SuperError::Selection)?;
    let original_caller = dotted(caller)?;
    let original_owner = dotted(&method.key.owner)?;
    let emitted_caller = aliases.class_name(caller).ok_or(SuperError::Aliases)?;
    let emitted_owner = aliases
        .class_name(&method.key.owner)
        .ok_or(SuperError::Aliases)?;
    AccessContext {
        caller: &original_caller,
        owner: &original_owner,
        receiver: Some(&original_caller),
        hierarchy,
        nests: None,
        emitted_caller,
        emitted_owner,
    }
    .method_access(method.metadata.access_flags)
    .map_err(|_| SuperError::Access)?;
    Ok(SuperSelection {
        target: method.key,
        metadata: method.metadata,
        ancestry,
        interface_qualifier: None,
    })
}
