//! Contextual zero and immutable signature identity regressions.
#![allow(clippy::unwrap_used)]
use super::*;
use managed_ir::{
    effects::EffectFlags,
    refs::{ClassRef, FieldRef},
};
use slotmap::{Key, SlotMap};

struct Context {
    initial: ValueId,
}
impl TypeContext for Context {
    fn initial_type(&self, v: ValueId) -> Option<ManagedType> {
        (v == self.initial).then_some(ManagedType::Int)
    }
    fn field_type(&self, index: u32) -> Option<ManagedType> {
        Some(match index {
            0 => ManagedType::Int,
            1 => ManagedType::Float,
            _ => ManagedType::Reference(ClassRef::new(0)),
        })
    }
}

#[test]
fn shared_zero_copy_has_no_global_numeric_reference_contradiction() {
    for order in [vec![0, 1, 2], vec![2, 1, 0]] {
        let mut values: SlotMap<ValueId, ()> = SlotMap::with_key();
        let zero = values.insert(());
        let copy = values.insert(());
        let mut insns = InsnArena::with_key();
        insns.insert(ManagedInsn::Const {
            result: zero,
            value: Constant::Zero,
        });
        insns.insert(ManagedInsn::Move {
            result: copy,
            value: zero,
        });
        for index in order {
            insns.insert(ManagedInsn::FieldPut {
                field: FieldRef::new(index),
                object: None,
                value: copy,
            });
        }
        let mut types = HashMap::from([(zero, ManagedType::Zero), (copy, ManagedType::Unknown)]);
        let offsets = insns.keys().map(|id| (id, 0)).collect();
        let diagnostics = infer_types(
            &Cfg::new(InsnId::null()),
            &insns,
            &mut types,
            &offsets,
            &Context {
                initial: ValueId::null(),
            },
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(types[&zero], ManagedType::Zero);
        assert_eq!(types[&copy], ManagedType::Zero);
    }
}

#[test]
fn signature_seed_uses_exact_initial_identity_and_retains_conflicts() {
    let mut values: SlotMap<ValueId, ()> = SlotMap::with_key();
    let initial = values.insert(());
    let later = values.insert(());
    let mut insns = InsnArena::with_key();
    insns.insert(ManagedInsn::Opaque {
        family_tag: 0x8003,
        reads: vec![],
        writes: vec![initial],
        effects: EffectFlags::NONE,
    });
    insns.insert(ManagedInsn::Const {
        result: later,
        value: Constant::Null,
    });
    insns.insert(ManagedInsn::FieldPut {
        field: FieldRef::new(2),
        object: None,
        value: initial,
    });
    insns.insert(ManagedInsn::FieldPut {
        field: FieldRef::new(2),
        object: None,
        value: later,
    });
    let mut types = HashMap::from([
        (initial, ManagedType::Unknown),
        (later, ManagedType::Unknown),
    ]);
    let diagnostics = infer_types(
        &Cfg::new(InsnId::null()),
        &insns,
        &mut types,
        &HashMap::new(),
        &Context { initial },
    );
    assert_eq!(types[&initial], ManagedType::Int);
    assert_eq!(types[&later], ManagedType::Null);
    assert!(diagnostics.iter().any(|d| d.code == "DEX_TYPE_CONFLICT"));
}

/// A narrow read-only diagnostic probe: caller specifies one DEX entry and a
/// handful of exact method pool IDs from retained-source/report metadata.
#[test]
#[ignore = "requires TYPE_APK, TYPE_METHOD_IDS and optional TYPE_DEX"]
fn selected_real_method_type_provenance() {
    let apk = std::env::var("TYPE_APK").unwrap();
    let entry = std::env::var("TYPE_DEX").unwrap_or_else(|_| "classes.dex".into());
    let ids: Vec<u32> = std::env::var("TYPE_METHOD_IDS")
        .unwrap()
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    assert!(
        !ids.is_empty() && ids.len() <= 12,
        "bounded diagnostic only"
    );
    let bytes = std::process::Command::new("unzip")
        .args(["-p", &apk, &entry])
        .output()
        .unwrap();
    assert!(bytes.status.success());
    let dex = dex_format::DexFile::from_bytes(&bytes.stdout).unwrap();
    let mut found = 0;
    for method in dex
        .classes
        .iter()
        .filter_map(|c| c.class_data.as_ref())
        .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
        .filter(|m| ids.contains(&m.method_idx))
    {
        found += 1;
        let code = method.code.as_ref().unwrap();
        let ctx = DexTypes::new(&dex, Some(method.method_idx));
        let ssa = crate::DexPasses::run_ssa_with(code, &ctx).unwrap();
        let id = &dex.method_ids[method.method_idx as usize];
        let owner = dex
            .string(dex.type_ids[id.class_idx as usize].descriptor_idx)
            .unwrap();
        let name = dex.string(id.name_idx).unwrap();
        eprintln!("IDENTITY {owner}->{name}");
        let mut declared = Vec::new();
        if method.access_flags & 0x0008 == 0 {
            declared.push(ManagedType::Reference(ClassRef::new(u32::from(
                id.class_idx,
            ))));
        }
        let mut params = Vec::new();
        assert!(ctx.method_params(method.method_idx, &mut params));
        declared.extend(params);
        let mut reg = code.registers_size - code.ins_size;
        let mut seeds = HashMap::new();
        for ty in declared {
            let width = if ty.is_wide() { 2 } else { 1 };
            let candidates = |register| -> HashSet<_> {
                ssa.ir().register_slots().values().filter(|s| s.index() == register)
                    .flat_map(|s| s.values()).copied().filter(|v| {
                        let record = ssa.ir().value(*v).unwrap();
                        matches!(ssa.ir().instruction(record.def()).unwrap(), ManagedInsn::Opaque { family_tag: 0x8001 | 0x8003, reads, writes, effects }
                            if reads.is_empty() && writes.as_slice() == [*v] && *effects == EffectFlags::NONE)
                    }).collect()
            };
            let low = candidates(reg);
            if low.len() == 1 && (width == 1 || candidates(reg + 1) == low) {
                let value = *low.iter().next().unwrap();
                eprintln!(
                    "  SIGNATURE register={reg} {value:?} declared={ty:?} inferred={:?}",
                    ssa.ir().value(value).unwrap().ty()
                );
                seeds.insert(value, ty);
            } else {
                eprintln!("  SIGNATURE register={reg} missing/ambiguous initial evidence {low:?}");
            }
            reg += width;
        }
        assert_eq!(reg, code.registers_size);
        let offsets = ssa
            .ir()
            .instructions()
            .keys()
            .filter_map(|id| ssa.offset(id).map(|offset| (id, offset)))
            .collect();
        let mut seeded_types = ssa
            .ir()
            .values()
            .iter()
            .map(|(v, record)| (v, record.ty().clone()))
            .collect();
        let seeded_ctx =
            DexTypes::new(&dex, Some(method.method_idx)).with_initial_types(seeds.clone());
        let seeded_diagnostics = infer_types(
            ssa.ir().cfg(),
            ssa.ir().instructions(),
            &mut seeded_types,
            &offsets,
            &seeded_ctx,
        );
        for (value, ty) in seeds {
            assert_eq!(seeded_types[&value], ty);
        }
        eprintln!("  SEEDED diagnostics={seeded_diagnostics:?}");
        eprintln!(
            "METHOD {} frame={} incoming={} diagnostics={:?}",
            method.method_idx,
            code.registers_size,
            code.ins_size,
            ssa.diagnostics()
        );
        for (value, record) in ssa.ir().values() {
            let def = ssa.ir().instruction(record.def()).unwrap();
            if matches!(
                def,
                ManagedInsn::Const {
                    value: Constant::Zero | Constant::Null,
                    ..
                } | ManagedInsn::Opaque {
                    family_tag: 0x8001 | 0x8003,
                    ..
                }
            ) {
                let slots: Vec<_> = ssa
                    .ir()
                    .register_slots()
                    .values()
                    .filter(|s| s.values().contains(&value))
                    .map(|s| s.index())
                    .collect();
                let mut consumers = Vec::new();
                for (id, insn) in ssa.ir().instructions() {
                    for_each_use(insn, |used| {
                        if used == value {
                            consumers.push((id, insn));
                        }
                    });
                }
                eprintln!(
                    "  {value:?} {:?} slots={slots:?} def={def:?} uses={consumers:?}",
                    record.ty()
                );
            }
        }
    }
    assert_eq!(found, ids.len());
}
