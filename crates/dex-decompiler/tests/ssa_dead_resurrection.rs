//! Dead arena instructions must not become synthetic reaching definitions.
use dex_decompiler::DexPasses;
use dex_format::{RawSpan, code::CodeItem};
use managed_ir::{insn::ManagedInsn, ops::Constant};

fn run(units: &[u16]) -> dex_decompiler::SsaMethod {
    let body = CodeItem {
        registers_size: 2,
        ins_size: 1,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|word| word.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: RawSpan { offset: 0, size: 0 },
    };
    DexPasses::run_ssa_with(&body, &dex_decompiler::infer::NoTypes).expect("verified SSA")
}

#[test]
fn dead_move_object_does_not_replace_live_register() {
    let ssa = run(&[0x7012, 0x0228, 0x1007, 0x000f]);
    assert!(
        !ssa.ir()
            .instructions()
            .iter()
            .any(|(id, _)| ssa.offset(id) == Some(2))
    );
    let returned = ssa
        .ir()
        .instructions()
        .iter()
        .find_map(|(_, insn)| match insn {
            ManagedInsn::Return { value } => *value,
            _ => None,
        })
        .expect("return value");
    assert!(matches!(
        ssa.ir().instructions()[ssa.ir().values()[returned].def()],
        ManagedInsn::Const {
            value: Constant::Int(7),
            ..
        }
    ));
}

#[test]
fn dead_iget_does_not_replace_live_register() {
    let ssa = run(&[0x7012, 0x0328, 0x1052, 0, 0x000f]);
    assert!(
        !ssa.ir()
            .instructions()
            .iter()
            .any(|(_, insn)| matches!(insn, ManagedInsn::FieldGet { .. }))
    );
    assert!(ssa.diagnostics().is_empty(), "{:?}", ssa.diagnostics());
}

#[test]
fn synthetic_parameter_survives_dead_definition() {
    let ssa = run(&[0x0228, 0x7112, 0x010f]);
    let returned = ssa
        .ir()
        .instructions()
        .iter()
        .find_map(|(_, insn)| match insn {
            ManagedInsn::Return { value } => *value,
            _ => None,
        })
        .expect("return value");
    assert!(matches!(
        ssa.ir().instructions()[ssa.ir().values()[returned].def()],
        ManagedInsn::Opaque {
            family_tag: 0x8001 | 0x8003,
            ..
        }
    ));
    assert!(
        !ssa.ir()
            .instructions()
            .iter()
            .any(|(id, _)| ssa.offset(id) == Some(1))
    );
}

#[test]
fn dead_invoke_result_does_not_replace_live_register() {
    let ssa = run(&[0x7012, 0x0528, 0x1071, 0, 1, 0x000a, 0x000f]);
    assert!(
        !ssa.ir()
            .instructions()
            .iter()
            .any(|(_, insn)| matches!(insn, ManagedInsn::Invoke { .. }))
    );
    let returned = ssa
        .ir()
        .instructions()
        .iter()
        .find_map(|(_, insn)| match insn {
            ManagedInsn::Return { value } => *value,
            _ => None,
        })
        .expect("return value");
    assert!(matches!(
        ssa.ir().instructions()[ssa.ir().values()[returned].def()],
        ManagedInsn::Const {
            value: Constant::Int(7),
            ..
        }
    ));
}

#[test]
fn handler_input_and_normal_success_commit_survive() {
    use dex_format::code::{EncodedCatchHandler, TryItem, TypeAddrPair};
    let units: [u16; 10] = [
        0x7012, // v0 = 7
        0x0071, 0, 0,      // invoke-static {}, method@0
        0x000a, // move-result v0
        0x000f, // normal return v0
        0x010d, // handler: move-exception v1
        0x000f, // handler return v0 (still 7)
        0x0112, // unreachable const
        0x000e,
    ];
    let body = CodeItem {
        registers_size: 2,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|word| word.to_le_bytes()).collect(),
        tries: vec![TryItem {
            start_addr: 1,
            insn_count: 4,
            handler_off: 1,
            span: RawSpan { offset: 0, size: 0 },
        }],
        handlers: vec![EncodedCatchHandler {
            relative_offset: 1,
            handlers: vec![TypeAddrPair {
                type_idx: 0,
                address: 6,
            }],
            catch_all_addr: None,
            span: RawSpan { offset: 0, size: 0 },
        }],
        span: RawSpan { offset: 0, size: 0 },
    };
    let ssa =
        DexPasses::run_ssa_with(&body, &dex_decompiler::infer::NoTypes).expect("verified SSA");
    assert!(ssa.ir().instructions().iter().any(|(_, insn)| matches!(
        insn,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            ..
        }
    )));
    for (offset, expected_move) in [(5, true), (7, false)] {
        let value = ssa
            .ir()
            .instructions()
            .iter()
            .find_map(|(id, insn)| match insn {
                ManagedInsn::Return { value } if ssa.offset(id) == Some(offset) => *value,
                _ => None,
            })
            .expect("return value");
        let def = &ssa.ir().instructions()[ssa.ir().values()[value].def()];
        if expected_move {
            assert!(matches!(def, ManagedInsn::Move { .. }));
        } else {
            assert!(matches!(
                def,
                ManagedInsn::Const {
                    value: Constant::Int(7),
                    ..
                }
            ));
        }
    }
}
