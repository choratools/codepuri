//! Prove compiler-owned enum parameters by SSA identities, not source spelling.
use super::*;
fn copies(ir: &MethodIr<SsaForm>, root: ValueId) -> Option<HashSet<ValueId>> {
    ir.value(root).ok()?;
    let mut aliases = HashSet::from([root]);
    loop {
        let mut changed = false;
        for (id, insn) in ir.instructions() {
            if let ManagedInsn::Move { result, value } = insn
                && aliases.contains(value)
                && !aliases.contains(result)
            {
                if ir.value(*result).ok()?.def() != id {
                    return None;
                }
                changed |= aliases.insert(*result);
            }
        }
        if !changed {
            return Some(aliases);
        }
    }
}
pub(super) fn chain_only(
    input: &EnumConstructorDeclarationInput<'_>,
    target: MethodRef,
    parameters: &[JavaType],
) -> bool {
    let Some(receiver) = input.bindings.receiver else {
        return false;
    };
    let [Some(name), Some(ordinal), ..] = input.bindings.parameters.as_slice() else {
        return false;
    };
    let Some(names) = copies(input.ir, *name) else {
        return false;
    };
    let Some(ordinals) = copies(input.ir, *ordinal) else {
        return false;
    };
    let Some(receivers) = copies(input.ir, receiver) else {
        return false;
    };
    if !names.is_disjoint(&ordinals)
        || !names.is_disjoint(&receivers)
        || !ordinals.is_disjoint(&receivers)
    {
        return false;
    }
    let hidden = |value: &ValueId| names.contains(value) || ordinals.contains(value);
    let mut chains = 0;
    for insn in input.ir.instructions().values() {
        if !super::enum_suffix_uses(insn).iter().any(hidden) {
            continue;
        }
        match insn {
            ManagedInsn::Move { value, .. } if hidden(value) => {}
            ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                result: None,
                method,
                args,
            } if *method == target => {
                let Ok(args) = super::invocation_operands(args, parameters) else {
                    return false;
                };
                if args.len() < 3
                    || !receivers.contains(&args[0])
                    || !names.contains(&args[1])
                    || !ordinals.contains(&args[2])
                    || args[3..].iter().any(hidden)
                {
                    return false;
                }
                chains += 1;
            }
            _ => return false,
        }
    }
    chains == 1
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use managed_ir::{
        effects::EffectFlags,
        method::{InsnArena, ValueArena},
        types::ManagedType,
        value::Value,
    };
    fn case(mode: u8) -> (MethodIr<SsaForm>, DeclarationBindings, Vec<JavaType>) {
        let mut insns = InsnArena::with_key();
        let mut values = ValueArena::with_key();
        let mut order = vec![];
        let mut initial = |ty| {
            let mut value = None;
            let id = insns.insert_with_key(|id| {
                let v = values.insert(Value::new(id, ty));
                value = Some(v);
                ManagedInsn::Opaque {
                    family_tag: 0x8001,
                    reads: vec![],
                    writes: vec![v],
                    effects: EffectFlags::NONE,
                }
            });
            order.push(id);
            value.unwrap()
        };
        let receiver = initial(ManagedType::Unknown);
        let name = initial(ManagedType::Unknown);
        let ordinal = initial(ManagedType::Int);
        let payload = initial(ManagedType::Int);
        let mut chain_name = name;
        if mode == 5 {
            let id = insns.insert_with_key(|id| {
                let v = values.insert(Value::new(id, ManagedType::Unknown));
                chain_name = v;
                ManagedInsn::Move {
                    result: v,
                    value: name,
                }
            });
            order.push(id);
        }
        let mut args = vec![
            if mode == 2 { payload } else { receiver },
            chain_name,
            ordinal,
        ];
        let mut types = vec![JavaType::Class("java.lang.String".into()), JavaType::Int];
        if mode == 4 {
            args.push(ordinal);
            types.push(JavaType::Int);
        }
        let invoke = ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(0),
            args,
        };
        order.push(insns.insert(invoke.clone()));
        if mode == 3 {
            order.push(insns.insert(invoke));
        }
        order.push(insns.insert(if mode == 6 {
            ManagedInsn::Opaque {
                family_tag: 0x9999,
                reads: vec![ordinal],
                writes: vec![],
                effects: EffectFlags::NONE,
            }
        } else {
            ManagedInsn::FieldPut {
                object: Some(receiver),
                field: managed_ir::refs::FieldRef::new(0),
                value: if mode == 1 { ordinal } else { payload },
            }
        }));
        let mut cfg = decomp_core::cfg::Cfg::new(order[0]);
        let mut last = cfg.entry();
        for id in &order[1..] {
            let next = cfg.add_block(*id);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        (
            MethodIr::<SsaForm>::from_parts(cfg, insns, values),
            DeclarationBindings {
                receiver: Some(receiver),
                parameters: vec![Some(name), Some(ordinal), Some(payload)],
                names: vec![],
            },
            types,
        )
    }
    #[test]
    fn hidden_parameter_proof_preserves_copies_but_rejects_escape_and_duplicate_chain() {
        for mode in 0..=6 {
            let (ir, bindings, types) = case(mode);
            let input = EnumConstructorDeclarationInput {
                reference: MethodRef::new(1),
                ir: &ir,
                bindings: &bindings,
            };
            assert_eq!(
                chain_only(&input, MethodRef::new(0), &types),
                matches!(mode, 0 | 5),
                "mode={mode}"
            );
        }
    }
}
