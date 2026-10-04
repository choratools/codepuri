//! Class-literal targets and resolution-side exception state.
#![allow(clippy::unwrap_used)]
use decomp_core::cfg::EdgeKind;
use dex_decompiler::{DexPasses, TypeContext, lift_method};
use dex_format::{
    RawSpan,
    code::{CodeItem, EncodedCatchHandler, TryItem, TypeAddrPair},
};
use managed_ir::{
    insn::{Constant, ManagedInsn},
    refs::ClassRef,
    types::ManagedType,
};
struct Types;
impl TypeContext for Types {
    fn class_literal_type(&self) -> Option<ManagedType> {
        Some(ManagedType::Reference(ClassRef::new(42)))
    }
}
fn code(units: &[u16]) -> CodeItem {
    CodeItem {
        registers_size: 1,
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
fn class_literal_preserves_type_pool_identity_and_class_result() {
    // Targets may name primitive, reference, or array descriptors; the semantic
    // result is always Class, never a value of that target's type.
    for target in [0, 1, 2, 65535] {
        let ssa = DexPasses::run_ssa_with(&code(&[0x001c, target, 0x0011]), &Types).unwrap();
        let literal = ssa
            .ir()
            .cfg()
            .block_ids()
            .find_map(|block| {
                match ssa
                    .ir()
                    .instruction(*ssa.ir().cfg().block(block).unwrap().payload())
                    .unwrap()
                {
                    ManagedInsn::Const {
                        result,
                        value: Constant::ClassLiteral(target),
                    } => Some((*result, *target)),
                    _ => None,
                }
            })
            .unwrap();
        assert_eq!(literal.1.index(), u32::from(target));
        assert_eq!(
            ssa.ir().value(literal.0).unwrap().ty(),
            &ManagedType::Reference(ClassRef::new(42))
        );
    }
}
#[test]
fn resolution_exception_precedes_destination_commit() {
    let mut item = code(&[0x0012, 0x001c, 7, 0x0011, 0x000d, 0x0011]);
    item.tries.push(TryItem {
        start_addr: 1,
        insn_count: 2,
        handler_off: 0,
        span: RawSpan { offset: 0, size: 0 },
    });
    item.handlers.push(EncodedCatchHandler {
        relative_offset: 0,
        handlers: vec![TypeAddrPair {
            type_idx: 8,
            address: 4,
        }],
        catch_all_addr: None,
        span: RawSpan { offset: 0, size: 0 },
    });
    let lifted = lift_method(&item).unwrap();
    let cfg = lifted.ir().cfg();
    let edge = cfg
        .edge_ids()
        .find_map(|id| {
            let edge = cfg.edge(id).unwrap();
            matches!(edge.kind(), EdgeKind::Exception(_)).then_some(edge)
        })
        .unwrap();
    let insn = lifted
        .ir()
        .instruction(*cfg.block(edge.source()).unwrap().payload())
        .unwrap();
    assert!(matches!(
        insn,
        ManagedInsn::Const {
            value: Constant::ClassLiteral(_),
            ..
        }
    ));
    assert!(cfg.edge_ids().any(|id| {
        let normal = cfg.edge(id).unwrap();
        normal.source() == edge.source()
            && matches!(normal.kind(), EdgeKind::Unconditional)
            && matches!(
                lifted
                    .ir()
                    .instruction(*cfg.block(normal.target()).unwrap().payload())
                    .unwrap(),
                ManagedInsn::Move { .. }
            )
    }));
}
#[test]
fn discarded_class_resolution_is_retained() {
    let ssa = DexPasses::run_ssa_with(&code(&[0x001c, 3, 0x000e]), &Types).unwrap();
    assert!(ssa.ir().cfg().block_ids().any(|id| {
        matches!(
            ssa.ir()
                .instruction(*ssa.ir().cfg().block(id).unwrap().payload())
                .unwrap(),
            ManagedInsn::Const {
                value: Constant::ClassLiteral(_),
                ..
            }
        )
    }));
}
