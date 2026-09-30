#![allow(clippy::unwrap_used)]

use super::*;
use decomp_core::ids::EdgeId;
use slotmap::{Key, SlotMap};

// Keep the old allocation-heavy traversal as a differential oracle for the
// optimized leaf path. Diagnostics are compared verbatim, including repeats.
fn legacy_meet(engine: &mut Engine<'_>, value: ValueId, target: ManagedType, insn: InsnId) {
    let mut pending = vec![value];
    let mut seen = HashSet::new();
    while let Some(value) = pending.pop() {
        if !seen.insert(value) {
            continue;
        }
        if let Some(phi) = engine.phi_results.get(&value) {
            if let Some(ManagedInsn::Phi { inputs, .. }) = engine.insns.get(*phi) {
                pending.extend(inputs.iter().map(|(_, value)| *value));
            }
        } else {
            engine.meet_leaf(value, target.clone(), insn);
        }
    }
}

fn legacy_run(engine: &mut Engine<'_>) {
    while let Some(insn) = engine.pending.pop_front() {
        engine.queued.remove(&insn);
        let record = engine.insns.get(insn).unwrap();
        let mut refinements = Vec::new();
        engine.collect(record, insn, &mut refinements);
        for refinement in refinements {
            match refinement {
                Refinement::Meet(value, target) => legacy_meet(engine, value, target, insn),
                Refinement::Join => engine.join_phi(insn),
            }
        }
    }
}

fn solve(
    insns: &InsnArena,
    mut types: HashMap<ValueId, ManagedType>,
    legacy: bool,
) -> (HashMap<ValueId, ManagedType>, Vec<Diagnostic>) {
    let mut phi_results = HashMap::new();
    let mut definitions = HashMap::new();
    let mut users: HashMap<_, Vec<_>> = HashMap::new();
    for (id, record) in insns.iter() {
        for_each_definition(record, |value| {
            definitions.insert(value, id);
            users.entry(value).or_default().push(id);
            if matches!(record, ManagedInsn::Phi { .. }) {
                phi_results.insert(value, id);
            }
        });
        for_each_use(record, |value| users.entry(value).or_default().push(id));
    }
    let offsets = insns
        .keys()
        .enumerate()
        .map(|(n, id)| (id, n as u32))
        .collect();
    let mut engine = Engine {
        insns,
        types: &mut types,
        offsets: &offsets,
        ctx: &NoTypes,
        phi_results: &phi_results,
        definitions: &definitions,
        raw_constants: &HashSet::new(),
        users: &users,
        pending: insns.keys().collect(),
        queued: insns.keys().collect(),
        diagnostics: Vec::new(),
        conflicts_overflow: 0,
        arity_overflow: 0,
        arity_diagnostics: 0,
        params: Vec::new(),
    };
    if legacy {
        legacy_run(&mut engine);
    } else {
        engine.run();
    }
    let diagnostics = engine.finish();
    (types, diagnostics)
}

#[test]
fn worklist_matches_legacy_for_cycles_fanout_and_conflicts() {
    for leaf_type in [
        ManagedType::Int,
        ManagedType::Long,
        ManagedType::Float,
        ManagedType::Double,
    ] {
        let mut values: SlotMap<ValueId, ()> = SlotMap::with_key();
        let leaf = values.insert(());
        let phi = values.insert(());
        let copy = values.insert(());
        let mut insns = InsnArena::with_key();
        insns.insert(ManagedInsn::Phi {
            result: phi,
            inputs: vec![
                (EdgeId::null(), leaf),
                (EdgeId::null(), copy),
                (EdgeId::null(), leaf),
            ],
        });
        insns.insert(ManagedInsn::Move {
            result: copy,
            value: phi,
        });
        let mut seeds = HashMap::from([
            (leaf, leaf_type.clone()),
            (phi, ManagedType::Unknown),
            (copy, ManagedType::Unknown),
        ]);
        // Repeated operands create duplicate dependency edges. Late consumers
        // force backward propagation and repeated conflict diagnostics.
        for n in 0..48 {
            let result = values.insert(());
            seeds.insert(
                result,
                if n % 3 == 0 {
                    if matches!(leaf_type, ManagedType::Long | ManagedType::Double) {
                        ManagedType::Int
                    } else {
                        ManagedType::Long
                    }
                } else {
                    leaf_type.clone()
                },
            );
            insns.insert(ManagedInsn::Arithmetic {
                op: ArithOp::Add,
                result,
                lhs: copy,
                rhs: copy,
            });
        }
        let expected = solve(&insns, seeds.clone(), true);
        let actual = solve(&insns, seeds, false);
        assert_eq!(actual, expected);
        assert_eq!(actual.0[&leaf], leaf_type);
        assert_eq!(actual.0[&phi], leaf_type);
        assert_eq!(actual.0[&copy], leaf_type);
        assert!(actual.1.iter().any(|d| d.code == "DEX_TYPE_CONFLICT"));
        assert!(
            actual
                .1
                .last()
                .unwrap()
                .message
                .contains("additional type conflict(s) suppressed")
        );
    }
}

struct ReturnType(ManagedType);
impl TypeContext for ReturnType {
    fn enclosing_return(&self) -> Option<ManagedType> {
        Some(self.0.clone())
    }
}

#[test]
fn refinement_collection_preserves_pre_update_operand_snapshot() {
    let mut values: SlotMap<ValueId, ()> = SlotMap::with_key();
    let source = values.insert(());
    let result = values.insert(());
    let mut insns = InsnArena::with_key();
    insns.insert(ManagedInsn::Move {
        result,
        value: source,
    });
    // Byte meets Int by narrowing the destination, but the reverse constraint
    // must still use the collected Int rather than a freshly read Byte.
    let seeds = HashMap::from([(source, ManagedType::Byte), (result, ManagedType::Int)]);
    let actual = solve(&insns, seeds.clone(), false);
    assert_eq!(actual, solve(&insns, seeds, true));
    assert!(actual.1.is_empty());
    assert_eq!(actual.0[&source], ManagedType::Byte);
    assert_eq!(actual.0[&result], ManagedType::Byte);
}

#[test]
fn raw_bits_propagate_through_long_copy_chains_without_integer_conversion() {
    for (constant, target) in [
        (Constant::Int(0x3f800000), ManagedType::Float),
        (Constant::Long(0x3ff0000000000000), ManagedType::Double),
        (Constant::Long(7), ManagedType::Long),
    ] {
        let mut values: SlotMap<ValueId, ()> = SlotMap::with_key();
        let first = values.insert(());
        let mut insns = InsnArena::with_key();
        let def = insns.insert(ManagedInsn::Const {
            result: first,
            value: constant,
        });
        let mut types = HashMap::from([(first, ManagedType::Unknown)]);
        let mut last = first;
        for _ in 0..128 {
            let result = values.insert(());
            types.insert(result, ManagedType::Unknown);
            insns.insert(ManagedInsn::Move {
                result,
                value: last,
            });
            last = result;
        }
        insns.insert(ManagedInsn::Return { value: Some(last) });
        let diagnostics = infer_types(
            &Cfg::new(def),
            &insns,
            &mut types,
            &HashMap::from([(def, 0)]),
            &ReturnType(target.clone()),
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(types.values().all(|ty| *ty == target));
        assert_eq!(
            insns.get(def),
            Some(&ManagedInsn::Const {
                result: first,
                value: constant
            })
        );
    }
}

#[test]
fn array_payload_bits_and_index_width_remain_exact() {
    for element in [ManagedType::Float, ManagedType::Double] {
        let mut values: SlotMap<ValueId, ()> = SlotMap::with_key();
        let array = values.insert(());
        let payload = values.insert(());
        let index = values.insert(());
        let result = values.insert(());
        let mut insns = InsnArena::with_key();
        insns.insert(ManagedInsn::Const {
            result: payload,
            value: Constant::Long(0),
        });
        insns.insert(ManagedInsn::FillArray {
            array,
            values: vec![payload],
        });
        insns.insert(ManagedInsn::ArrayGet {
            result,
            array,
            index,
        });
        let shape = ManagedType::Array {
            element: Box::new(element.clone()),
            nullable: true,
        };
        let mut types = HashMap::from([
            (array, shape.clone()),
            (payload, ManagedType::Long),
            (index, ManagedType::Unknown),
            (result, ManagedType::Unknown),
        ]);
        let diagnostics = infer_types(
            &Cfg::new(insns.keys().next().unwrap()),
            &insns,
            &mut types,
            &HashMap::new(),
            &NoTypes,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(types[&array], shape);
        assert_eq!(types[&payload], element);
        assert_eq!(types[&result], element);
        assert_eq!(types[&index], ManagedType::Int);
    }
}
