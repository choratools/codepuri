//! Execute small SSA programs, including simultaneous phi selection, rather
//! than treating verifier acceptance as proof of preserved reaching values.
use std::collections::HashMap;

use dex_decompiler::{DexPasses, SsaMethod, TypeContext};
use dex_format::{RawSpan, code::CodeItem};
use managed_ir::{
    insn::ManagedInsn,
    ops::{ArithOp, CompareOp, Constant},
    types::ManagedType,
};

fn must<T, E: std::fmt::Debug>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("fixture failed: {error:?}"),
    }
}

fn some<T>(value: Option<T>) -> T {
    match value {
        Some(value) => value,
        None => panic!("missing fixture value"),
    }
}

fn code(registers: u16, units: &[u16]) -> CodeItem {
    CodeItem {
        registers_size: registers,
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

struct ThrowingParse;
impl TypeContext for ThrowingParse {
    fn string_type(&self) -> Option<ManagedType> {
        Some(ManagedType::Reference(managed_ir::refs::ClassRef::new(0)))
    }
    fn method_return(&self, index: u32) -> Option<ManagedType> {
        (index == 0).then_some(ManagedType::Float)
    }
    fn method_params(&self, _: u32, params: &mut Vec<ManagedType>) -> bool {
        params.clear();
        params.push(ManagedType::Reference(managed_ir::refs::ClassRef::new(0)));
        true
    }
    fn enclosing_return(&self) -> Option<ManagedType> {
        Some(ManagedType::Float)
    }
}

struct FloatArray;
impl TypeContext for FloatArray {
    fn field_type(&self, _: u32) -> Option<ManagedType> {
        Some(ManagedType::Array {
            element: Box::new(ManagedType::Float),
            nullable: true,
        })
    }
    fn enclosing_return(&self) -> Option<ManagedType> {
        Some(ManagedType::Float)
    }
}

struct FloatConsumer;
impl TypeContext for FloatConsumer {
    fn method_params(&self, _: u32, params: &mut Vec<ManagedType>) -> bool {
        params.clear();
        params.push(ManagedType::Float);
        true
    }
    fn enclosing_return(&self) -> Option<ManagedType> {
        Some(ManagedType::Float)
    }
}

#[test]
fn numeric_conversion_has_no_exception_edge_to_handler() {
    use dex_format::code::{EncodedCatchHandler, TryItem, TypeAddrPair};
    let mut body = code(
        1,
        &[
            0x7012, // 0: const/4 v0, 7
            0x0082, // 1: int-to-float v0, v0
            0x1071, 0, 0, // 2: invoke-static {v0}, method@0 (void)
            0x000f, 0x000f, // 5: return v0; 6: handler: return v0
        ],
    );
    body.tries.push(TryItem {
        start_addr: 0,
        insn_count: 6,
        handler_off: 1,
        span: RawSpan { offset: 0, size: 0 },
    });
    body.handlers.push(EncodedCatchHandler {
        relative_offset: 1,
        handlers: vec![TypeAddrPair {
            type_idx: 1,
            address: 6,
        }],
        catch_all_addr: None,
        span: RawSpan { offset: 0, size: 0 },
    });
    let ssa = must(DexPasses::run_ssa_with(&body, &FloatConsumer));
    assert!(ssa.diagnostics().is_empty(), "{:?}", ssa.diagnostics());
    let handler_value = ssa
        .ir()
        .instructions()
        .iter()
        .find_map(|(id, insn)| match insn {
            ManagedInsn::Return { value } if ssa.offset(id) == Some(6) => *value,
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing fixture value"));
    assert!(matches!(
        ssa.ir().instructions()[ssa.ir().values()[handler_value].def()],
        ManagedInsn::Cast { .. }
    ));
    let cfg = ssa.ir().cfg();
    let exceptional: Vec<_> = cfg
        .edge_ids()
        .filter(|id| {
            matches!(
                must(cfg.edge(*id)).kind(),
                decomp_core::cfg::EdgeKind::Exception(_)
            )
        })
        .collect();
    assert_eq!(exceptional.len(), 1);
    let source = must(cfg.block(must(cfg.edge(exceptional[0])).source()));
    assert_eq!(ssa.offset(*source.payload()), Some(2));
}

#[test]
fn failed_array_get_preserves_array_for_handler_retry() {
    use dex_format::code::{EncodedCatchHandler, TryItem, TypeAddrPair};
    let mut body = code(
        2,
        &[
            0x0062, 0,      // 0: sget-object v0, field@0 ([F)
            0x0112, // 2: const/4 v1, 0
            0x0044, 0x0100, // 3: aget v0, v0, v1
            0x000f, // 5: return v0
            0x0044, 0x0100, // 6: handler: retry aget v0, v0, v1
            0x000f, // 8: return v0
        ],
    );
    body.tries.push(TryItem {
        start_addr: 3,
        insn_count: 3,
        handler_off: 1,
        span: RawSpan { offset: 0, size: 0 },
    });
    body.handlers.push(EncodedCatchHandler {
        relative_offset: 1,
        handlers: vec![TypeAddrPair {
            type_idx: 1,
            address: 6,
        }],
        catch_all_addr: None,
        span: RawSpan { offset: 0, size: 0 },
    });
    let ssa = must(DexPasses::run_ssa_with(&body, &FloatArray));
    assert!(ssa.diagnostics().is_empty(), "{:?}", ssa.diagnostics());
    let array = ssa
        .ir()
        .instructions()
        .iter()
        .find_map(|(id, insn)| match insn {
            ManagedInsn::ArrayGet { array, .. } if ssa.offset(id) == Some(6) => Some(*array),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing fixture value"));
    assert!(matches!(
        ssa.ir().instructions()[ssa.ir().values()[array].def()],
        ManagedInsn::FieldGet { .. }
    ));
}

#[test]
fn protected_invoke_result_does_not_overwrite_handler_register() {
    use dex_format::code::{EncodedCatchHandler, TryItem, TypeAddrPair};
    // v0 = string; try { v0 = parse(v0); return v0; }
    // catch { log(v0); return 0f; }. On exception v0 is still the string.
    let mut body = code(
        1,
        &[
            0x001a, 0, // 0: const-string v0, string@0
            0x1071, 0, 0,      // 2: invoke-static {v0}, method@0
            0x000a, // 5: move-result v0
            0x000f, // 6: return v0
            0x1071, 1, 0, // 7: invoke-static {v0}, method@1
            0x0012, 0x000f, // 10: const/4 v0, 0; return v0
        ],
    );
    body.tries.push(TryItem {
        start_addr: 2,
        insn_count: 4,
        handler_off: 1,
        span: RawSpan { offset: 0, size: 0 },
    });
    body.handlers.push(EncodedCatchHandler {
        relative_offset: 1,
        handlers: vec![TypeAddrPair {
            type_idx: 1,
            address: 7,
        }],
        catch_all_addr: None,
        span: RawSpan { offset: 0, size: 0 },
    });
    let ssa = must(DexPasses::run_ssa_with(&body, &ThrowingParse));
    assert!(ssa.diagnostics().is_empty(), "{:?}", ssa.diagnostics());
    let handler_arg = ssa
        .ir()
        .instructions()
        .iter()
        .find_map(|(id, insn)| match insn {
            ManagedInsn::Invoke { args, .. } if ssa.offset(id) == Some(7) => Some(args[0]),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing fixture value"));
    assert!(matches!(
        ssa.ir().instructions()[ssa.ir().values()[handler_arg].def()],
        ManagedInsn::Const {
            value: Constant::String(_),
            ..
        }
    ));
    let returned = ssa
        .ir()
        .instructions()
        .iter()
        .find_map(|(id, insn)| match insn {
            ManagedInsn::Return { value } if ssa.offset(id) == Some(6) => *value,
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing fixture value"));
    assert_eq!(ssa.ir().values()[returned].ty(), &ManagedType::Float);
    assert_ne!(handler_arg, returned);
}

fn run(ssa: &SsaMethod) -> i64 {
    let ir = ssa.ir();
    let cfg = ir.cfg();
    let mut block = cfg.entry();
    let mut incoming = None;
    let mut values = HashMap::new();
    for _ in 0..500 {
        // Select the entire cluster from the predecessor snapshot before
        // committing any result (essential for swap loops).
        let mut selected = vec![];
        while let ManagedInsn::Phi { result, inputs } =
            &ir.instructions()[*must(cfg.block(block)).payload()]
        {
            let input = some(inputs.iter().find(|(edge, _)| Some(*edge) == incoming)).1;
            selected.push((*result, values[&input]));
            let record = must(cfg.block(block));
            assert_eq!(record.outgoing().len(), 1);
            block = must(cfg.edge(record.outgoing()[0])).target();
        }
        values.extend(selected);
        let record = must(cfg.block(block));
        let insn = &ir.instructions()[*record.payload()];
        let mut next = record.outgoing().first().copied();
        match insn {
            ManagedInsn::Const { result, value } => {
                let value = match value {
                    Constant::Int(n) => i64::from(*n),
                    Constant::Long(n) => *n,
                    Constant::Zero => 0,
                    _ => panic!("unsupported constant"),
                };
                values.insert(*result, value);
            }
            ManagedInsn::Move { result, value } => {
                values.insert(*result, values[value]);
            }
            ManagedInsn::Not { result, value } => {
                values.insert(*result, i64::from(values[value] == 0));
            }
            ManagedInsn::Arithmetic {
                op,
                result,
                lhs,
                rhs,
            } => {
                let (a, b) = (values[lhs], values[rhs]);
                let value = match op {
                    ArithOp::Add => a + b,
                    ArithOp::Sub => a - b,
                    ArithOp::Mul => a * b,
                    ArithOp::ShiftLeft => a << b,
                    _ => panic!("unsupported arithmetic"),
                };
                values.insert(*result, value);
            }
            ManagedInsn::Compare {
                op,
                result,
                lhs,
                rhs,
            } => {
                let (a, b) = (values[lhs], values[rhs]);
                let value = match op {
                    CompareOp::Equal => a == b,
                    CompareOp::NotEqual => a != b,
                    CompareOp::LessThan => a < b,
                    CompareOp::LessEqual => a <= b,
                    CompareOp::GreaterThan => a > b,
                    CompareOp::GreaterEqual => a >= b,
                };
                values.insert(*result, i64::from(value));
            }
            ManagedInsn::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                next = if condition.is_none_or(|value| values[&value] != 0) {
                    Some(*then_edge)
                } else {
                    *else_edge
                };
            }
            ManagedInsn::Return { value: Some(value) } => return values[value],
            other => panic!("unexpected instruction {other:?}"),
        }
        let edge = some(next);
        incoming = Some(edge);
        block = must(cfg.edge(edge)).target();
    }
    panic!("SSA program did not terminate")
}

#[test]
fn two_variable_diamond_selects_both_arm_values() {
    for (condition, expected) in [(0, 11), (1, 7)] {
        let item = code(
            4,
            &[
                0x1012,
                0x2112,
                (condition << 12) | 0x0212,
                0x0238,
                5,
                0x3012,
                0x4112,
                0x0328,
                0x5012,
                0x6112,
                0x0390,
                0x0100,
                0x030f,
            ],
        );
        let ssa = must(DexPasses::run_merged(&item));
        let phis: Vec<_> = ssa
            .ir()
            .instructions()
            .values()
            .filter_map(|insn| match insn {
                ManagedInsn::Phi { inputs, .. } => Some(inputs),
                _ => None,
            })
            .collect();
        assert_eq!(phis.len(), 2);
        assert_eq!(
            phis[0].iter().map(|(edge, _)| *edge).collect::<Vec<_>>(),
            phis[1].iter().map(|(edge, _)| *edge).collect::<Vec<_>>()
        );
        assert_eq!(run(&ssa), expected);
    }
}

#[test]
fn join_reads_remain_live_before_later_overwrites_in_the_same_chain() {
    for (condition, expected) in [(0, 11), (1, 7)] {
        let item = code(
            4,
            &[
                0x1012,
                0x2112,
                (condition << 12) | 0x0212,
                0x0238,
                5,
                0x3012,
                0x4112,
                0x0328,
                0x5012,
                0x6112,
                0x0390,
                0x0100,
                0x0012,
                0x0112,
                0x030f,
            ],
        );
        let ssa = must(DexPasses::run_merged(&item));
        assert_eq!(run(&ssa), expected);
        assert_eq!(
            ssa.ir()
                .instructions()
                .values()
                .filter(|insn| matches!(insn, ManagedInsn::Phi { .. }))
                .count(),
            2
        );
    }
}

#[test]
fn loop_phis_swap_two_reaching_values_simultaneously() {
    for iterations in 1..=4 {
        let item = code(
            4,
            &[
                0x1012,
                0x2112,
                (iterations << 12) | 0x0212,
                0x7312,
                0x0238,
                8,
                0x0301,
                0x1001,
                0x3101,
                0x02d8,
                0xff02,
                0xf928,
                0x03da,
                0x0a00,
                0x13b0,
                0x030f,
            ],
        );
        let ssa = must(DexPasses::run_merged(&item));
        assert!(ssa.stats().phis_placed >= 3);
        for insn in ssa.ir().instructions().values() {
            if let ManagedInsn::Phi { result, .. } = insn {
                assert_eq!(ssa.ir().values()[*result].ty(), &ManagedType::Int);
            }
        }
        assert_eq!(run(&ssa), if iterations % 2 == 0 { 12 } else { 21 });
    }
}

#[test]
fn narrow_and_wide_register_reuse_preserves_definition_widths() {
    // Both halves of v0:v1 are subsequently reused as independent integers;
    // v2:v3 retains the copied wide value and v4:v5 receives a later wide def.
    let item = code(
        6,
        &[
            0x0016, 42, 0x0204, 0x7012, 0x3112, 0x0490, 0x0100, 0x0416, 9, 0x0210,
        ],
    );
    let ssa = must(DexPasses::run_merged(&item));
    assert_eq!(run(&ssa), 42);
    for (id, value) in ssa.ir().values() {
        let slots: Vec<_> = ssa
            .ir()
            .register_slots()
            .values()
            .filter(|slot| slot.values().contains(&id))
            .map(|slot| slot.index())
            .collect();
        if value.ty().is_wide() && !slots.is_empty() {
            assert_eq!(slots.len(), 2);
            assert_eq!(slots[1], slots[0] + 1);
        }
    }
    assert!(
        !ssa.ir()
            .instructions()
            .values()
            .any(|insn| matches!(insn, ManagedInsn::Cast { .. }))
    );
}

struct Fields;
impl TypeContext for Fields {
    fn field_type(&self, index: u32) -> Option<ManagedType> {
        Some(if index == 0 {
            ManagedType::Long
        } else {
            ManagedType::Int
        })
    }
}

#[test]
fn wide_field_opcodes_bind_pairs_but_boolean_fields_do_not() {
    let item = code(5, &[0x0053, 0, 0x0255, 1, 0x0061, 0, 0x0263, 1, 0x0010]);
    let ssa = must(DexPasses::run_ssa_with(&item, &Fields));
    for (id, insn) in ssa.ir().instructions() {
        if let ManagedInsn::FieldGet { result, field, .. } = insn {
            let slots: Vec<_> = ssa
                .ir()
                .register_slots()
                .values()
                .filter(|slot| slot.values().contains(result))
                .map(|slot| slot.index())
                .collect();
            assert_eq!(
                slots.len(),
                if field.index() == 0 { 2 } else { 1 },
                "{id:?}"
            );
        }
    }
}

struct Calls;
impl TypeContext for Calls {
    fn method_params(&self, index: u32, params: &mut Vec<ManagedType>) -> bool {
        params.clear();
        params.push(ManagedType::Long);
        if index == 0 {
            params.push(ManagedType::Int);
        }
        true
    }
    fn method_return(&self, index: u32) -> Option<ManagedType> {
        (index == 0).then_some(ManagedType::Boolean)
    }
}

#[test]
fn invoke_constraints_skip_receiver_and_wide_high_word() {
    let item = code(
        5,
        &[
            0x0412, 0x0116, 42, 0x7312, 0x406e, 0, 0x3214, 0x000a, 0x000f,
        ],
    );
    let ssa = must(DexPasses::run_ssa_with(&item, &Calls));
    assert!(ssa.diagnostics().is_empty(), "{:?}", ssa.diagnostics());
    for insn in ssa.ir().instructions().values() {
        if let ManagedInsn::Invoke { args, result, .. } = insn {
            assert_eq!(args.len(), 4);
            assert_eq!(args[1], args[2]);
            assert_eq!(ssa.ir().values()[args[1]].ty(), &ManagedType::Long);
            assert_eq!(ssa.ir().values()[args[3]].ty(), &ManagedType::Int);
            assert_eq!(ssa.ir().values()[some(*result)].ty(), &ManagedType::Boolean);
            assert!(!ssa.ir().values()[args[0]].ty().is_wide());
        }
    }
}

#[test]
fn consumer_constraint_reaches_the_defining_phi_inputs() {
    let item = code(
        5,
        &[
            0x0412, 0x1212, 0x0238, 5, 0x4053, 0, 0x0328, 0x4053, 1, 0x2071, 1, 0x0010, 0x0010,
        ],
    );
    let ssa = must(DexPasses::run_ssa_with(&item, &Calls));
    assert!(ssa.diagnostics().is_empty(), "{:?}", ssa.diagnostics());
    let mut phis = 0;
    for insn in ssa.ir().instructions().values() {
        if let ManagedInsn::Phi { result, inputs } = insn {
            phis += 1;
            assert_eq!(ssa.ir().values()[*result].ty(), &ManagedType::Long);
            for (_, input) in inputs {
                assert_eq!(ssa.ir().values()[*input].ty(), &ManagedType::Long);
            }
        }
    }
    assert_eq!(phis, 1);
}

struct FloatingCall;
impl TypeContext for FloatingCall {
    fn method_params(&self, _: u32, params: &mut Vec<ManagedType>) -> bool {
        *params = vec![ManagedType::Float, ManagedType::Double];
        true
    }
}

#[test]
fn raw_literals_keep_float_and_double_bits_through_late_copy_constraints() {
    let item = code(
        6,
        &[
            0x0015, 0x3f80, 0x0219, 0x3ff0, 0x0101, 0x2404, 0x3071, 0, 0x0541, 0x000e,
        ],
    );
    let ssa = must(DexPasses::run_ssa_with(&item, &FloatingCall));
    assert!(ssa.diagnostics().is_empty(), "{:?}", ssa.diagnostics());
    let constants: Vec<_> = ssa
        .ir()
        .instructions()
        .values()
        .filter_map(|insn| match insn {
            ManagedInsn::Const { value, .. } => Some(*value),
            _ => None,
        })
        .collect();
    assert!(constants.contains(&Constant::FloatBits(1.0f32.to_bits())));
    assert!(constants.contains(&Constant::DoubleBits(1.0f64.to_bits())));
    assert!(
        !ssa.ir()
            .instructions()
            .values()
            .any(|insn| matches!(insn, ManagedInsn::Cast { .. }))
    );
}

struct Arrays;
impl TypeContext for Arrays {
    fn type_at(&self, _: u32) -> Option<ManagedType> {
        Some(ManagedType::Array {
            element: Box::new(ManagedType::Float),
            nullable: true,
        })
    }
}

#[test]
fn new_array_preserves_the_descriptor_dimension_and_element_type() {
    let item = code(2, &[0x2112, 0x1023, 0, 0x0011]);
    let ssa = must(DexPasses::run_ssa_with(&item, &Arrays));
    assert!(ssa.diagnostics().is_empty(), "{:?}", ssa.diagnostics());
    let mut arrays = 0;
    for insn in ssa.ir().instructions().values() {
        if let ManagedInsn::NewArray { result, .. } = insn {
            arrays += 1;
            assert_eq!(
                ssa.ir().values()[*result].ty(),
                &ManagedType::Array {
                    element: Box::new(ManagedType::Float),
                    nullable: true
                }
            );
        }
    }
    assert_eq!(arrays, 1);
}

#[test]
fn fill_array_payload_uses_element_bit_interpretation() {
    let item = code(
        2,
        &[
            0x2112, 0x1023, 0, 0x0026, 5, 0, 0x0011, 0, 0x0300, 4, 2, 0, 0, 0, 0, 0x3f80,
        ],
    );
    let ssa = must(DexPasses::run_ssa_with(&item, &Arrays));
    assert!(ssa.diagnostics().is_empty(), "{:?}", ssa.diagnostics());
    let mut fills = 0;
    for insn in ssa.ir().instructions().values() {
        if let ManagedInsn::FillArray { values, .. } = insn {
            fills += 1;
            let constants: Vec<_> = values
                .iter()
                .map(|value| {
                    assert_eq!(ssa.ir().values()[*value].ty(), &ManagedType::Float);
                    match ssa.ir().instructions()[ssa.ir().values()[*value].def()] {
                        ManagedInsn::Const { value, .. } => value,
                        ref other => panic!("unexpected payload definition {other:?}"),
                    }
                })
                .collect();
            assert_eq!(
                constants,
                vec![
                    Constant::FloatBits(0),
                    Constant::FloatBits(1.0f32.to_bits())
                ]
            );
        }
    }
    assert_eq!(fills, 1);
}
