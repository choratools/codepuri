//! Shared raw literal one is contextual; arbitrary runtime Int/Boolean is not.
#![allow(clippy::unwrap_used)]
use dex_decompiler::{DexPasses, TypeContext};
use dex_format::{RawSpan, code::CodeItem};
use managed_ir::{insn::ManagedInsn, refs::ClassRef, types::ManagedType};
struct Types;
impl TypeContext for Types {
    fn method_return(&self, m: u32) -> Option<ManagedType> {
        match m {
            1 => Some(ManagedType::Int),
            2 => Some(ManagedType::Boolean),
            _ => None,
        }
    }
    fn method_params(&self, m: u32, p: &mut Vec<ManagedType>) -> bool {
        p.clear();
        if m == 0 {
            p.push(ManagedType::Boolean);
        }
        true
    }
    fn enclosing_return(&self) -> Option<ManagedType> {
        Some(ManagedType::Int)
    }
    fn type_at(&self, _: u32) -> Option<ManagedType> {
        Some(ManagedType::Reference(ClassRef::new(0)))
    }
}
fn run(units: &[u16]) -> dex_decompiler::ssa::SsaMethod {
    let code = CodeItem {
        registers_size: 3,
        ins_size: 0,
        outs_size: 1,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|v| v.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: RawSpan { offset: 0, size: 0 },
    };
    DexPasses::run_ssa_with(&code, &Types).unwrap()
}
#[test]
fn numeric_result_compared_with_shared_boolean_one_remains_int() {
    let ssa = run(&[
        0x1112, 0x1071, 0, 1, 0x0071, 1, 0, 0x000a, 0x1032, 3, 0x000f, 0x000f,
    ]);
    assert!(
        !ssa.diagnostics()
            .iter()
            .any(|d| d.code() == "DEX_TYPE_CONFLICT"),
        "{:?}",
        ssa.diagnostics()
    );
    for (_, insn) in ssa.ir().instructions() {
        if let ManagedInsn::Const { value, .. } = insn {
            assert!(!matches!(value, managed_ir::ops::Constant::Zero));
        }
        if let ManagedInsn::Invoke {
            method,
            result: Some(result),
            ..
        } = insn
            && method.index() == 1
        {
            assert_eq!(ssa.ir().value(*result).unwrap().ty(), &ManagedType::Int);
        }
    }
}
#[test]
fn arbitrary_boolean_result_compared_with_integer_two_still_conflicts() {
    let ssa = run(&[0x2112, 0x0071, 2, 0, 0x000a, 0x1032, 3, 0x000f, 0x000f]);
    assert!(
        ssa.diagnostics()
            .iter()
            .any(|d| d.code() == "DEX_TYPE_CONFLICT")
    );
}

#[test]
fn copied_raw_one_keeps_literal_identity_and_numeric_domain() {
    let ssa = run(&[
        0x1112, 0x1201, 0x1071, 0, 2, 0x0071, 1, 0, 0x000a, 0x2032, 3, 0x000f, 0x000f,
    ]);
    assert!(
        !ssa.diagnostics()
            .iter()
            .any(|d| d.code() == "DEX_TYPE_CONFLICT"),
        "{:?}",
        ssa.diagnostics()
    );
    assert!(ssa.ir().instructions().values().any(|i| matches!(
        i,
        ManagedInsn::Const {
            value: managed_ir::ops::Constant::Int(1),
            ..
        }
    )));
}
#[test]
fn arbitrary_int_invocation_into_boolean_parameter_remains_conflict() {
    let ssa = run(&[0x0071, 1, 0, 0x000a, 0x1071, 0, 0, 0x000f]);
    assert!(
        ssa.diagnostics()
            .iter()
            .any(|d| d.code() == "DEX_TYPE_CONFLICT")
    );
}

struct IncompatibleTypes(ManagedType);
impl TypeContext for IncompatibleTypes {
    fn method_params(&self, method: u32, params: &mut Vec<ManagedType>) -> bool {
        params.clear();
        params.push(if method == 0 {
            ManagedType::Boolean
        } else {
            self.0.clone()
        });
        true
    }
}
#[test]
fn raw_one_does_not_erase_float_or_reference_constraints_in_either_order() {
    for conflicting in [ManagedType::Float, ManagedType::Reference(ClassRef::new(0))] {
        for first in [0u16, 1u16] {
            let wide = matches!(conflicting, ManagedType::Long | ManagedType::Double);
            let mut units = vec![0x0112, 0x1012]; // v0=1; v1=0 (high word only for negative wide fixture)
            for method in [first, 1 - first] {
                if method == 1 && wide {
                    units.extend([0x2071, method, 0x0010]);
                } else {
                    units.extend([0x1071, method, 0]);
                }
            }
            units.push(0x000e);
            let code = CodeItem {
                registers_size: 2,
                ins_size: 0,
                outs_size: 2,
                debug_info_off: 0,
                insns_size: units.len() as u32,
                insns: units.iter().flat_map(|v| v.to_le_bytes()).collect(),
                tries: vec![],
                handlers: vec![],
                span: RawSpan { offset: 0, size: 0 },
            };
            let ssa =
                DexPasses::run_ssa_with(&code, &IncompatibleTypes(conflicting.clone())).unwrap();
            assert!(
                ssa.diagnostics()
                    .iter()
                    .any(|d| d.code() == "DEX_TYPE_CONFLICT" || d.code() == "DEX_INVOKE_ARITY"),
                "{conflicting:?},first={first}: {:?}",
                ssa.diagnostics()
            );
            // Original v0 raw1 may change typed view, never become literal0.
            assert!(ssa.ir().instructions().values().any(|i| matches!(
                i,
                ManagedInsn::Const {
                    value: managed_ir::ops::Constant::Int(1)
                        | managed_ir::ops::Constant::Boolean(true)
                        | managed_ir::ops::Constant::FloatBits(1),
                    ..
                }
            )));
        }
    }
}

#[test]
fn raw_one_mixed_narrow_boolean_views_reach_fixedpoint_in_both_orders() {
    for narrow in [ManagedType::Byte, ManagedType::Short, ManagedType::Char] {
        for first in [0u16, 1u16] {
            let units = [0x1012, 0x1071, first, 0, 0x1071, 1 - first, 0, 0x000e];
            let code = CodeItem {
                registers_size: 1,
                ins_size: 0,
                outs_size: 1,
                debug_info_off: 0,
                insns_size: units.len() as u32,
                insns: units.iter().flat_map(|v| v.to_le_bytes()).collect(),
                tries: vec![],
                handlers: vec![],
                span: RawSpan { offset: 0, size: 0 },
            };
            let ssa = DexPasses::run_ssa_with(&code, &IncompatibleTypes(narrow.clone())).unwrap();
            assert!(
                !ssa.diagnostics()
                    .iter()
                    .any(|d| d.code() == "DEX_TYPE_CONFLICT"),
                "{narrow:?}: {:?}",
                ssa.diagnostics()
            );
            assert!(ssa.ir().instructions().values().any(|i| matches!(
                i,
                ManagedInsn::Const {
                    value: managed_ir::ops::Constant::Int(1),
                    ..
                }
            )));
        }
    }
}
