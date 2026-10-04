#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::ids::EdgeId;
use slotmap::{Key, SlotMap};

pub(super) fn legacy_join(engine: &mut Engine<'_>, insn: InsnId) {
    let Some(ManagedInsn::Phi { result, inputs }) = engine.insns.get(insn) else {
        return;
    };
    let result = *result;
    let mut pending: Vec<_> = inputs.iter().map(|(_, v)| *v).collect();
    let mut seen = HashSet::from([result]);
    let mut joined = ManagedType::Conflict;
    let mut leaves = 0;
    while let Some(value) = pending.pop() {
        if !seen.insert(value) {
            continue;
        }
        match engine
            .definitions
            .get(&value)
            .and_then(|id| engine.insns.get(*id))
        {
            Some(ManagedInsn::Phi { inputs, .. }) => {
                pending.extend(inputs.iter().map(|(_, v)| *v));
                continue;
            }
            Some(ManagedInsn::Move { value, .. }) => {
                pending.push(*value);
                continue;
            }
            _ => {}
        }
        leaves += 1;
        let input = engine
            .types
            .get(&value)
            .cloned()
            .unwrap_or(ManagedType::Unknown);
        joined = types::join(&joined, &input);
    }
    if leaves == 0 {
        joined = ManagedType::Unknown;
    }
    if engine.types.get(&result) != Some(&joined) {
        engine.types.insert(result, joined);
        engine.push_users(result);
    }
}

#[test]
fn randomized_dependency_graphs_exact_differential() {
    // Fixed PRNG makes failures reproducible without external randomness.
    let mut seed = 0x7bc4_0231_u64;
    for case in 0..500 {
        let mut random = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as usize % n
        };
        let mut slots: SlotMap<ValueId, ()> = SlotMap::with_key();
        let values: Vec<_> = (0..16).map(|_| slots.insert(())).collect();
        let mut insns = InsnArena::with_key();
        let mut initial = HashMap::new();
        for &value in &values {
            initial.insert(
                value,
                match random(9) {
                    0 => ManagedType::Int,
                    1 => ManagedType::Long,
                    2 => ManagedType::Float,
                    3 => ManagedType::Null,
                    4 => ManagedType::Reference(managed_ir::refs::ClassRef::new(1)),
                    5 => ManagedType::Array {
                        element: Box::new(ManagedType::Float),
                        nullable: true,
                    },
                    6 => ManagedType::Array {
                        element: Box::new(ManagedType::Array {
                            element: Box::new(ManagedType::Int),
                            nullable: true,
                        }),
                        nullable: true,
                    },
                    _ => ManagedType::Unknown,
                },
            );
        }
        for (i, &result) in values.iter().enumerate() {
            let record = if i < 4 {
                ManagedInsn::Const {
                    result,
                    value: Constant::Int(i as i32),
                }
            } else if random(3) == 0 {
                ManagedInsn::Move {
                    result,
                    value: values[random(values.len())],
                }
            } else {
                let inputs = (0..random(6))
                    .map(|_| (EdgeId::null(), values[random(values.len())]))
                    .collect();
                ManagedInsn::Phi { result, inputs }
            };
            insns.insert(record);
        }
        for _ in 0..8 {
            let result = slots.insert(());
            initial.insert(
                result,
                if random(2) == 0 {
                    ManagedType::Int
                } else {
                    ManagedType::Long
                },
            );
            insns.insert(ManagedInsn::Arithmetic {
                op: ArithOp::Add,
                result,
                lhs: values[random(values.len())],
                rhs: values[random(values.len())],
            });
        }
        let old = tests::solve(&insns, initial.clone(), true);
        let new = tests::solve(&insns, initial, false);
        assert_eq!(old.0, new.0, "types case {case}");
        assert_eq!(old.1, new.1, "diagnostics case {case}");
    }
}

#[test]
fn scc_root_order_copy_boundary_and_no_exit() {
    let mut slots: SlotMap<ValueId, ()> = SlotMap::with_key();
    let [a, b, x, y, copy, closed] = std::array::from_fn(|_| slots.insert(()));
    let mut insns = InsnArena::with_key();
    let mut definitions = HashMap::new();
    let mut phis = HashMap::new();
    for (result, inputs) in [
        (a, vec![x, b]),
        (b, vec![copy, a]),
        (closed, vec![closed, closed]),
    ] {
        let id = insns.insert(ManagedInsn::Phi {
            result,
            inputs: inputs.into_iter().map(|v| (EdgeId::null(), v)).collect(),
        });
        definitions.insert(result, id);
        phis.insert(result, id);
    }
    definitions.insert(
        copy,
        insns.insert(ManagedInsn::Move {
            result: copy,
            value: y,
        }),
    );
    let mut cache = expansion::Expansions::default();
    for _ in 0..2 {
        for (root, join, expected) in [
            (a, false, vec![copy, x]),
            (b, false, vec![x, copy]),
            (a, true, vec![y, x]),
            (b, true, vec![x, y]),
            (closed, false, vec![]),
            (closed, true, vec![]),
        ] {
            assert_eq!(
                &*cache.leaves(root, join, &insns, &definitions, &phis),
                expected
            );
        }
    }
    assert_eq!(cache.builds, 6);
    let seeds = HashMap::from([
        (a, ManagedType::Unknown),
        (b, ManagedType::Unknown),
        (x, ManagedType::Int),
        (y, ManagedType::Byte),
        (copy, ManagedType::Unknown),
        (closed, ManagedType::Unknown),
    ]);
    let old = tests::solve(&insns, seeds.clone(), true);
    let new = tests::solve(&insns, seeds, false);
    assert_eq!(old, new);
    assert_eq!(new.0[&closed], ManagedType::Unknown);
}

#[test]
fn repeated_expansion_operation_counts() {
    let mut slots: SlotMap<ValueId, ()> = SlotMap::with_key();
    let values: Vec<_> = (0..65).map(|_| slots.insert(())).collect();
    let mut insns = InsnArena::with_key();
    let mut definitions = HashMap::new();
    for i in 1..65 {
        let id = insns.insert(ManagedInsn::Phi {
            result: values[i],
            inputs: vec![
                (EdgeId::null(), values[i - 1]),
                (EdgeId::null(), values[i - 1]),
            ],
        });
        definitions.insert(values[i], id);
    }
    for join in [false, true] {
        let mut cache = expansion::Expansions::default();
        let mut old_visits = 0;
        for _ in 0..1000 {
            // Execute the original traversal, counting every stack pop,
            // including repeated dependency edges.
            let mut pending = if join {
                vec![values[63], values[63]]
            } else {
                vec![values[64]]
            };
            let mut seen = if join {
                HashSet::from([values[64]])
            } else {
                HashSet::new()
            };
            let mut old_leaves = Vec::new();
            while let Some(value) = pending.pop() {
                old_visits += 1;
                if !seen.insert(value) {
                    continue;
                }
                if let Some(ManagedInsn::Phi { inputs, .. }) =
                    definitions.get(&value).and_then(|id| insns.get(*id))
                {
                    pending.extend(inputs.iter().map(|(_, v)| *v));
                } else {
                    old_leaves.push(value);
                }
            }
            assert_eq!(old_leaves, vec![values[0]]);
            assert_eq!(
                &*cache.leaves(values[64], join, &insns, &definitions, &definitions),
                &[values[0]]
            );
        }
        println!(
            "join={join}: old traversal pops={old_visits}, new={}, old seen sets=1000, new builds={}",
            cache.visits, cache.builds
        );
        assert_eq!(cache.builds, 1);
        assert_eq!(cache.visits, if join { 128 } else { 129 });
    }
}
