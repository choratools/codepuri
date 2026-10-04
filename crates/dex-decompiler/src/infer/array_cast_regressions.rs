//! Regressions for descriptor ranks, cast boundaries, and shared null literals.
#![allow(clippy::unwrap_used)]

use super::*;
use decomp_core::ids::EdgeId;
use managed_ir::refs::{ClassRef, FieldRef, TypeRef};
use slotmap::{Key, SlotMap};

fn array(element: ManagedType) -> ManagedType {
    ManagedType::Array {
        element: Box::new(element),
        nullable: true,
    }
}

struct Context;
impl TypeContext for Context {
    fn type_at(&self, _: u32) -> Option<ManagedType> {
        descriptor::parse("[[F", &|_| None)
    }
    fn field_type(&self, index: u32) -> Option<ManagedType> {
        Some(match index {
            0 => array(ManagedType::Float),
            1 => array(array(ManagedType::Float)),
            _ => ManagedType::Reference(ClassRef::new(index)),
        })
    }
    fn enclosing_return(&self) -> Option<ManagedType> {
        Some(ManagedType::Float)
    }
}

fn solve(insns: &InsnArena, types: &mut HashMap<ValueId, ManagedType>) -> Vec<Diagnostic> {
    let offsets = insns
        .keys()
        .enumerate()
        .map(|(n, id)| (id, n as u32))
        .collect();
    infer_types(&Cfg::new(InsnId::null()), insns, types, &offsets, &Context)
}

#[test]
fn shared_null_preserves_multidimensional_phi_and_element_types() {
    for constant in [Constant::Zero, Constant::Null] {
        let mut values: SlotMap<ValueId, ()> = SlotMap::with_key();
        let [null, object, cast, phi, index, row, scalar] =
            std::array::from_fn(|_| values.insert(()));
        let mut insns = InsnArena::with_key();
        insns.insert(ManagedInsn::Const {
            result: null,
            value: constant,
        });
        // The same null is consumed as [F, [[F, and unrelated nominal refs.
        for field in 0..4 {
            insns.insert(ManagedInsn::FieldPut {
                field: FieldRef::new(field),
                value: null,
                object: None,
            });
        }
        insns.insert(ManagedInsn::Cast {
            result: cast,
            value: object,
            target: TypeRef::new(0),
        });
        insns.insert(ManagedInsn::Phi {
            result: phi,
            inputs: vec![(EdgeId::null(), null), (EdgeId::null(), cast)],
        });
        insns.insert(ManagedInsn::ArrayGet {
            result: row,
            array: phi,
            index,
        });
        insns.insert(ManagedInsn::ArrayGet {
            result: scalar,
            array: row,
            index,
        });
        insns.insert(ManagedInsn::Return {
            value: Some(scalar),
        });
        let mut types: HashMap<_, _> = values.keys().map(|v| (v, ManagedType::Unknown)).collect();
        types.insert(object, ManagedType::Reference(ClassRef::new(10)));
        types.insert(index, ManagedType::Int);
        let diagnostics = solve(&insns, &mut types);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(types[&null], ManagedType::Null);
        assert_eq!(types[&object], ManagedType::Reference(ClassRef::new(10)));
        assert_eq!(types[&cast], array(array(ManagedType::Float)));
        assert_eq!(types[&phi], array(array(ManagedType::Float)));
        assert_eq!(types[&row], array(ManagedType::Float));
        assert_eq!(types[&scalar], ManagedType::Float);
    }
}

#[test]
fn lifted_cast_preserves_descriptor_rank() {
    let units = [
        0x001f_u16, 0, 0x0112, 0x0246, 0x0100, 0x0344, 0x0102, 0x030f,
    ];
    let code = dex_format::code::CodeItem {
        registers_size: 4,
        ins_size: 1,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|word| word.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: dex_format::RawSpan { offset: 0, size: 0 },
    };
    let ssa = crate::DexPasses::run_ssa_with(&code, &Context).unwrap();
    assert!(ssa.diagnostics().is_empty(), "{:?}", ssa.diagnostics());
    let cast = ssa
        .ir()
        .instructions()
        .iter()
        .find_map(|(_, insn)| match insn {
            ManagedInsn::Cast { result, .. } => Some(*result),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        ssa.ir().values()[cast].ty(),
        &array(array(ManagedType::Float))
    );
}

#[test]
fn raw_zero_still_supports_float_bit_interpretation() {
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
    insns.insert(ManagedInsn::Return { value: Some(copy) });
    let mut types = HashMap::from([(zero, ManagedType::Zero), (copy, ManagedType::Unknown)]);
    let diagnostics = solve(&insns, &mut types);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(types[&zero], ManagedType::Float);
    assert_eq!(types[&copy], ManagedType::Float);
}

#[test]
fn genuine_scalar_cast_and_array_element_mismatches_remain_errors() {
    for scalar_cast in [false, true] {
        let mut values: SlotMap<ValueId, ()> = SlotMap::with_key();
        let source = values.insert(());
        let result = values.insert(());
        let index = values.insert(());
        let mut insns = InsnArena::with_key();
        if scalar_cast {
            insns.insert(ManagedInsn::Cast {
                result,
                value: source,
                target: TypeRef::new(0),
            });
        } else {
            insns.insert(ManagedInsn::ArrayGet {
                result,
                array: source,
                index,
            });
            insns.insert(ManagedInsn::Return {
                value: Some(result),
            });
        }
        let source_type = if scalar_cast {
            ManagedType::Float
        } else {
            array(ManagedType::Long)
        };
        let mut types = HashMap::from([
            (source, source_type.clone()),
            (result, ManagedType::Unknown),
            (index, ManagedType::Int),
        ]);
        let diagnostics = solve(&insns, &mut types);
        assert!(
            diagnostics.iter().any(|d| d.code() == "DEX_TYPE_CONFLICT"),
            "{diagnostics:?}"
        );
        assert_eq!(types[&source], source_type);
    }
}
