//! Concrete invocation return contracts across ordinary acyclic copy families.
//! This module never selects a subtype from its consumers or inserts a cast.
use super::{MethodIndex, consumer_constraints_indexed};
use crate::{JavaResolver, JavaType};
use managed_ir::{
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
    value::ValueId,
};
use std::collections::{HashMap, HashSet};

#[cfg(test)]
pub(super) fn declarations(
    ir: &MethodIr<SsaForm>,
    return_type: &JavaType,
    resolver: &impl JavaResolver,
) -> HashMap<ValueId, JavaType> {
    declarations_indexed(ir, &MethodIndex::new(ir), return_type, resolver)
}

pub(super) fn declarations_indexed(
    ir: &MethodIr<SsaForm>,
    index: &MethodIndex<'_>,
    return_type: &JavaType,
    resolver: &impl JavaResolver,
) -> HashMap<ValueId, JavaType> {
    let mut result = HashMap::new();
    for (id, instruction) in &index.instructions {
        let ManagedInsn::Invoke {
            result: Some(seed),
            method,
            kind,
            ..
        } = instruction
        else {
            continue;
        };
        let Some(target) = resolver.method(*method) else {
            continue;
        };
        if target.is_static != (*kind == managed_ir::ops::InvokeKind::Static)
            || !target.return_type.reference()
        {
            continue;
        }
        let concrete = target.return_type;
        let mut family = Vec::new();
        let mut seen = HashSet::new();
        let mut pending = vec![*seed];
        let mut valid = true;
        let mut needs_refinement = false;
        while let Some(value) = pending.pop() {
            if !seen.insert(value) {
                continue;
            }
            let Ok(record) = ir.value(value) else {
                valid = false;
                break;
            };
            if index.definitions.get(&value) != Some(&1) {
                valid = false;
                break;
            }
            let Ok(definition) = ir.instruction(record.def()) else {
                valid = false;
                break;
            };
            if value == *seed {
                if record.def() != *id {
                    valid = false;
                    break;
                }
            } else if !matches!(definition,ManagedInsn::Move{result,value:source} if *result==value && seen.contains(source))
            {
                valid = false;
                break;
            }
            let Ok(nominal) = crate::managed_type(record.ty(), resolver) else {
                valid = false;
                break;
            };
            if !nominal.reference() || !resolver.assignable(&concrete, &nominal) {
                valid = false;
                break;
            }
            needs_refinement |= nominal != concrete;
            family.push(value);
            pending.extend(index.copies.get(&value).into_iter().flatten().copied());
        }
        if !valid || !needs_refinement || seen.iter().any(|v| index.phi_members.contains(v)) {
            continue;
        }
        let proposed: HashMap<_, _> = family.iter().map(|v| (*v, concrete.clone())).collect();
        let Ok(consumers) =
            consumer_constraints_indexed(ir, index, &family, return_type, resolver, &proposed)
        else {
            continue;
        };
        if consumers
            .iter()
            .any(|expected| !resolver.assignable(&concrete, expected))
        {
            continue;
        }
        if proposed
            .iter()
            .any(|(v, t)| result.get(v).is_some_and(|old| old != t))
        {
            continue;
        }
        result.extend(proposed);
    }
    result
}
#[cfg(test)]
#[path = "producer_reference_locals_tests.rs"]
mod tests;
