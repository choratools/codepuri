//! Exact original dispatch suffix after a proved, completed receiver chain.
use super::*;
use decomp_core::cfg::EdgeKind;
use decomp_core::ids::BlockId;

pub(super) fn emit<'a, R: ConstructorResolver>(
    e: &mut Emitter<'a, R>,
    method: &'a JavaMethod,
    bindings: &DeclarationBindings,
    prefix: &HashSet<BlockId>,
    metadata: &dyn ConstructorExceptionMetadata,
) -> Result<(), Unsupported> {
    let fail = || Unsupported::Metadata("unproved constructor exception suffix".into());
    let scopes = metadata.scopes();
    if scopes.is_empty() || !e.lines.is_empty() || bindings.receiver.is_none() {
        return Err(fail());
    }
    let mut initializer = None;
    for block in prefix {
        let node = e.ir.cfg().block(*block).map_err(|_| fail())?;
        if *block == e.ir.cfg().entry() {
            if !node.incoming().is_empty() {
                return Err(fail());
            }
        } else if node.incoming().len() != 1 {
            return Err(fail());
        } else {
            let incoming = e.ir.cfg().edge(node.incoming()[0]).map_err(|_| fail())?;
            if !prefix.contains(&incoming.source())
                || !matches!(incoming.kind(), EdgeKind::Unconditional)
            {
                return Err(fail());
            }
        }
        if scopes.iter().any(|scope| {
            scope
                .region_scope()
                .catches
                .iter()
                .any(|catch| catch.entry == *block)
        }) {
            return Err(fail());
        }
        let id = *node.payload();
        if scopes
            .iter()
            .any(|scope| scope.region_scope().protected.contains(block))
        {
            return Err(fail());
        }
        let instruction = e.ir.instruction(id).map_err(|_| fail())?;
        match instruction {
            ManagedInsn::Opaque {
                family_tag,
                reads,
                writes,
                effects,
            } if matches!(*family_tag, 0x8001 | 0x8003)
                && reads.is_empty()
                && !writes.is_empty()
                && *effects == managed_ir::effects::EffectFlags::NONE
                && writes.iter().all(|value| e.parameters.contains(value)) => {}
            // Only provenance-attested inert literal definitions may precede
            // the chain. They are not incoming slots or initializer operands;
            // the original dispatcher still renders their exact identities
            // after the real chain has completed.
            ManagedInsn::Const {
                result,
                value: Constant::Zero,
            } if !e.parameters.contains(result)
                && e.ir.value(*result).is_ok_and(|value| {
                    value.def() == id && *value.ty() == managed_ir::types::ManagedType::Zero
                })
                && scopes.iter().all(|scope| {
                    scope
                        .original_scope()
                        .is_some_and(|original| !original.offsets.contains_key(&id))
                }) => {}
            ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                result: None,
                method: target,
                args,
            } if initializer.is_none() => {
                let target = e.resolver.initializer(*target).ok_or_else(fail)?;
                let operands = invoke_arguments::operands(&target.method, args)?;
                if operands.first().copied() != bindings.receiver
                    || operands.iter().any(|value| !e.parameters.contains(value))
                {
                    return Err(fail());
                }
                initializer = Some(id);
            }
            _ => return Err(fail()),
        }
        if node.outgoing().len() != 1
            || !matches!(
                e.ir.cfg()
                    .edge(node.outgoing()[0])
                    .map_err(|_| fail())?
                    .kind(),
                EdgeKind::Unconditional
            )
        {
            return Err(fail());
        }
    }
    let initializer = initializer.ok_or_else(fail)?;
    for (id, instruction) in e.ir.instructions().iter() {
        if id != initializer
            && let ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                method: target,
                args,
                ..
            } = instruction
            && let Some(target) = e.resolver.initializer(*target)
            && invoke_arguments::operands(&target.method, args)?
                .first()
                .copied()
                == bindings.receiver
        {
            return Err(Unsupported::Metadata(
                "repeated receiver initializer in exception suffix".into(),
            ));
        }
    }
    for instruction in e.ir.instructions().values() {
        if let ManagedInsn::FieldPut { field, .. } = instruction
            && !e.resolver.constructor_exception_field_store(*field)
        {
            return Err(Unsupported::Metadata(
                "constructor exception field store requires non-final proof".into(),
            ));
        }
    }
    let bound = e.parameters.iter().copied().collect::<Vec<_>>();
    let plan =
        crate::exceptions::dispatch::plan(e.ir, &scopes, &bound, e.resolver, &e.declarations)
            .map_err(|error| Unsupported::ControlFlow(error.to_string()))?;
    if plan
        .capture_blocks
        .iter()
        .any(|block| prefix.contains(block))
    {
        return Err(fail());
    }
    crate::exceptions::dispatch::render_initialized_into(e, method, &scopes, &plan, initializer)
}
