//! Filled allocation decoding, SSA normalization, and type contracts.
#![allow(clippy::unwrap_used)]
use dex_decompiler::{DexPasses, TypeContext};
use dex_format::{RawSpan, code::CodeItem};
use managed_ir::{insn::ManagedInsn, types::ManagedType};

struct Arrays(ManagedType);
impl TypeContext for Arrays {
    fn type_at(&self, _: u32) -> Option<ManagedType> {
        Some(ManagedType::Array {
            element: Box::new(self.0.clone()),
            nullable: true,
        })
    }
}
fn code(units: &[u16]) -> CodeItem {
    CodeItem {
        registers_size: 8,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|unit| unit.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: RawSpan { offset: 0, size: 0 },
    }
}
#[test]
fn packed_and_range_preserve_elements_and_fuse_object_result() {
    for instruction in [[0x2024, 0, 0x0032], [0x0225, 0, 2]] {
        let mut units = vec![0x1212, 0x2312]; // v2=1, v3=2
        units.extend(instruction);
        units.extend([0x040c, 0x0411]);
        let ssa = DexPasses::run_ssa_with(&code(&units), &Arrays(ManagedType::Int)).unwrap();
        let ir = ssa.ir();
        let allocation = ir
            .cfg()
            .block_ids()
            .find_map(|block| {
                let insn = ir
                    .instruction(*ir.cfg().block(block).unwrap().payload())
                    .unwrap();
                match insn {
                    ManagedInsn::NewFilledArray {
                        result, elements, ..
                    } => Some((result, elements)),
                    _ => None,
                }
            })
            .unwrap();
        assert!(allocation.0.is_some());
        assert_eq!(allocation.1.len(), 2);
        for (value, expected) in allocation.1.iter().zip([1, 2]) {
            let def = ir.value(*value).unwrap().def();
            assert!(
                matches!(ir.instruction(def).unwrap(), ManagedInsn::Const { value: managed_ir::ops::Constant::Int(n), .. } if *n == expected)
            );
        }
    }
}
#[test]
fn discarded_allocation_survives_and_wide_components_fail() {
    let body = code(&[0x0025, 0, 7, 0x000e]);
    let ssa = DexPasses::run_ssa_with(&body, &Arrays(ManagedType::Int)).unwrap();
    assert!(ssa.ir().cfg().block_ids().any(|block| matches!(ssa.ir().instruction(*ssa.ir().cfg().block(block).unwrap().payload()).unwrap(), ManagedInsn::NewFilledArray { result: None, elements, .. } if elements.is_empty())));
    for ty in [ManagedType::Long, ManagedType::Double] {
        let error = DexPasses::run_ssa_with(&body, &Arrays(ty)).unwrap_err();
        assert!(error.to_string().contains("category-one"));
    }
}

#[test]
fn component_constraints_interpret_raw_bits_and_zero() {
    for (component, units, expected) in [
        (
            ManagedType::Float,
            vec![0x0014, 0, 0x3f80],
            ManagedType::Float,
        ),
        (ManagedType::Boolean, vec![0x1012], ManagedType::Boolean),
        (ManagedType::Byte, vec![0x7012], ManagedType::Byte),
        (ManagedType::Short, vec![0x7012], ManagedType::Short),
        (ManagedType::Char, vec![0x7012], ManagedType::Char),
        (
            ManagedType::Reference(managed_ir::refs::ClassRef::new(1)),
            vec![0x0012],
            ManagedType::Null,
        ),
    ] {
        let mut units = units;
        units.extend([0x1024, 0, 0, 0x000e]);
        let ssa = DexPasses::run_ssa_with(&code(&units), &Arrays(component)).unwrap();
        let ir = ssa.ir();
        let value = ir
            .cfg()
            .block_ids()
            .find_map(|block| {
                match ir
                    .instruction(*ir.cfg().block(block).unwrap().payload())
                    .unwrap()
                {
                    ManagedInsn::NewFilledArray { elements, .. } => elements.first().copied(),
                    _ => None,
                }
            })
            .unwrap();
        assert_eq!(ir.value(value).unwrap().ty(), &expected);
        if expected == ManagedType::Float {
            assert!(matches!(
                ir.instruction(ir.value(value).unwrap().def()).unwrap(),
                ManagedInsn::Const {
                    value: managed_ir::ops::Constant::FloatBits(0x3f800000),
                    ..
                }
            ));
        }
    }
}

#[test]
fn range_expands_all_six_words_in_register_order() {
    let mut units = vec![0x0012, 0x1112, 0x2212, 0x3312, 0x4412, 0x5512];
    units.extend([0x0625, 0, 0, 0x000e]);
    let ssa = DexPasses::run_ssa_with(&code(&units), &Arrays(ManagedType::Int)).unwrap();
    let ir = ssa.ir();
    let elements = ir
        .cfg()
        .block_ids()
        .find_map(|block| {
            match ir
                .instruction(*ir.cfg().block(block).unwrap().payload())
                .unwrap()
            {
                ManagedInsn::NewFilledArray {
                    result: None,
                    elements,
                    ..
                } => Some(elements),
                _ => None,
            }
        })
        .unwrap();
    assert_eq!(elements.len(), 6);
    for (element, expected) in elements.iter().zip(0..6) {
        let literal = ir.instruction(ir.value(*element).unwrap().def()).unwrap();
        assert!(
            matches!(literal, ManagedInsn::Const { value: managed_ir::ops::Constant::Zero, .. } if expected == 0)
                || matches!(literal, ManagedInsn::Const { value: managed_ir::ops::Constant::Int(n), .. } if *n == expected)
        );
    }
}
