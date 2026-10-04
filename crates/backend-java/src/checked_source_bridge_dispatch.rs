//! An outer source boundary around the unchanged original exception dispatcher.
use super::*;
use crate::exceptions::{RegionMetadata, dispatch};
use decomp_core::ids::BlockId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ScopeIdentity {
    protected: Vec<BlockId>,
    continuation: Option<BlockId>,
    catches: Vec<(String, BlockId, JavaType, bool)>,
    original: (usize, u32, u32, u16, Vec<(String, u32)>),
    offsets: Vec<(InsnId, u32)>,
}
fn scopes(metadata: &[&dyn RegionMetadata]) -> Result<Vec<ScopeIdentity>, Unsupported> {
    if metadata.is_empty() {
        return Err(failure("missing original dispatch scopes"));
    }
    metadata
        .iter()
        .map(|md| {
            let original = md
                .original_scope()
                .ok_or_else(|| failure("missing original dispatch range"))?;
            let mut protected = md.region_scope().protected.clone();
            protected.sort();
            let mut offsets: Vec<_> = original
                .offsets
                .iter()
                .map(|(id, offset)| (*id, *offset))
                .collect();
            offsets.sort_by_key(|(id, _)| *id);
            Ok(ScopeIdentity {
                protected,
                continuation: md.region_scope().continuation,
                catches: md
                    .region_scope()
                    .catches
                    .iter()
                    .map(|catch| {
                        Ok((
                            catch.key.clone(),
                            catch.entry,
                            md.catch_type(&catch.key)
                                .ok_or_else(|| failure("missing original dispatch catch type"))?,
                            md.is_catch_all(&catch.key),
                        ))
                    })
                    .collect::<Result<_, Unsupported>>()?,
                original: (
                    original.ordinal,
                    original.start,
                    original.end,
                    original.handler_offset,
                    original.catches.clone(),
                ),
                offsets,
            })
        })
        .collect()
}
/// Prove an outer checked-source boundary while retaining every original handler.
/// The frontend must attest an ordinary, nonsynchronized class override and a
/// collision-free same-owner synthetic identity-rethrow helper.
pub fn plan_dispatch_body(
    ir: &MethodIr<SsaForm>,
    original: &JavaMethod,
    runtime_escaping: Option<&[String]>,
    helper: &str,
    resolver: &impl JavaResolver,
    bindings: &DeclarationBindings,
    metadata: &[&dyn RegionMetadata],
) -> Result<Option<Plan>, Unsupported> {
    plan_dispatch_body_bound_targets(
        ir,
        original,
        runtime_escaping,
        helper,
        resolver,
        BoundDispatchInputs {
            bindings,
            targets: &[],
        },
        metadata,
    )
}

/// Descriptor bindings and exact selected target witnesses for a dispatch boundary.
pub struct BoundDispatchInputs<'a> {
    /// Original caller parameter identities.
    pub bindings: &'a DeclarationBindings,
    /// Exact selected original targets, never blanket source-domain permission.
    pub targets: &'a [BoundTarget],
}

/// Exact-target outer boundary preserving the complete original encoded scopes.
/// Broader Throwable domains require a selected genuine field-throw witness.
pub fn plan_dispatch_body_bound_targets(
    ir: &MethodIr<SsaForm>,
    original: &JavaMethod,
    runtime_escaping: Option<&[String]>,
    helper: &str,
    resolver: &impl JavaResolver,
    inputs: BoundDispatchInputs<'_>,
    metadata: &[&dyn RegionMetadata],
) -> Result<Option<Plan>, Unsupported> {
    let BoundDispatchInputs { bindings, targets } = inputs;
    let snapshot = scopes(metadata)?;
    let Some(mut recipe) = plan_inner_bound(
        ir,
        original,
        runtime_escaping,
        helper,
        resolver,
        false,
        (Mode::DispatchBody, targets),
    )?
    else {
        return Ok(None);
    };
    let declarations =
        dispatch::declarations_for_scopes(ir, &original.return_type, resolver, metadata)?;
    let incoming: Vec<_> = bindings
        .parameters
        .iter()
        .flatten()
        .copied()
        .chain(bindings.receiver)
        .collect();
    dispatch::plan(ir, metadata, &incoming, resolver, &declarations).map_err(|err| {
        Unsupported::ControlFlow(format!("dispatch bridge original scope: {err}"))
    })?;
    recipe.dispatch_scopes = Some(snapshot);
    Ok(Some(recipe))
}
/// Render the proved original dispatcher in place, wrapped by one outer source
/// boundary. No original body is moved into a helper and no throw clause is
/// published as a new runtime fact.
pub fn emit_dispatch_body(
    ir: &MethodIr<SsaForm>,
    bindings: &DeclarationBindings,
    resolver: &impl JavaResolver,
    recipe: &Plan,
    metadata: &[&dyn RegionMetadata],
) -> SourceResult {
    let checked = || -> Result<(), Unsupported> {
        if recipe.mode != Mode::DispatchBody
            || recipe.dispatch_scopes.as_ref() != Some(&scopes(metadata)?)
        {
            return Err(failure("original dispatch scope changed"));
        }
        validate_ownership(ir, resolver, recipe)
    };
    if let Err(err) = checked() {
        return SourceResult::Unsupported(err);
    }
    dispatch::emit_method(
        ir,
        recipe.rendering_method(),
        bindings,
        &Local {
            inner: resolver,
            plan: recipe,
        },
        metadata,
    )
}

#[cfg(test)]
#[path = "checked_source_bridge_dispatch_tests.rs"]
mod tests;
