//! Typed original register lifting keeps source offsets and fails closed.
#![allow(clippy::unwrap_used)]
use dex_decompiler::{DexPasses, LiftError, TypeContext};
use dex_format::{RawSpan, code::CodeItem};
use managed_ir::{insn::ManagedInsn, types::ManagedType};

struct Descriptor(Option<ManagedType>);
impl TypeContext for Descriptor {
    fn type_at(&self, index: u32) -> Option<ManagedType> {
        (index == 7).then(|| self.0.clone()).flatten()
    }
}
fn body() -> CodeItem {
    let units: [u16; 8] = [0x1012, 0x1024, 7, 0, 0x010c, 0x0169, 41, 0x000e];
    CodeItem {
        registers_size: 2,
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
fn typed_raw_filled_array_preserves_allocation_store_and_return_offsets() {
    let descriptor = Descriptor(Some(ManagedType::Array {
        element: Box::new(ManagedType::Int),
        nullable: true,
    }));
    let raw = DexPasses::run_with(&body(), &descriptor).unwrap();
    raw.ir().cfg().validate().unwrap();
    let mut allocation = None;
    let mut store = None;
    let mut normal_return = None;
    for (id, instruction) in raw.ir().instructions() {
        match instruction {
            ManagedInsn::NewFilledArray {
                result: Some(_),
                elements,
                ..
            } => {
                assert_eq!(elements.len(), 1);
                allocation = raw.offset(id);
            }
            ManagedInsn::FieldPut {
                object: None,
                field,
                ..
            } if field.index() == 41 => store = raw.offset(id),
            ManagedInsn::Return { value: None } => normal_return = raw.offset(id),
            _ => {}
        }
    }
    assert_eq!(
        (allocation, store, normal_return),
        (Some(1), Some(5), Some(7))
    );
}
#[test]
fn unresolved_nonarray_and_wide_array_descriptors_still_fail() {
    assert!(matches!(
        DexPasses::run(&body()),
        Err(LiftError::InvalidFilledArray {
            offset: 1,
            type_index: 7,
            ..
        })
    ));
    for ty in [
        None,
        Some(ManagedType::Int),
        Some(ManagedType::Array {
            element: Box::new(ManagedType::Long),
            nullable: true,
        }),
        Some(ManagedType::Array {
            element: Box::new(ManagedType::Double),
            nullable: true,
        }),
    ] {
        assert!(matches!(
            DexPasses::run_with(&body(), &Descriptor(ty)),
            Err(LiftError::InvalidFilledArray {
                offset: 1,
                type_index: 7,
                ..
            })
        ));
    }
}
