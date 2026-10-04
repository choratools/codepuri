//! SSA verifier policy and width checks.

use std::fmt::Debug;

use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::insn::{Constant, ManagedInsn, OrderingOp};
use managed_ir::method::{
    InsnArena, MethodIr, RegisterSlot, RegisterSlotArena, SsaForm, ValueArena,
};
use managed_ir::types::ManagedType;
use managed_ir::value::{InsnId, Value, ValueId};
use managed_ir::verify::SsaVerifier;

fn must<T, E: Debug>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("unexpected fixture error: {error:?}"),
    }
}

fn definition(
    insns: &mut InsnArena,
    values: &mut ValueArena,
    ty: ManagedType,
) -> (InsnId, ValueId) {
    let insn = insns.insert(ManagedInsn::Return { value: None });
    let value = values.insert(Value::new(insn, ty));
    (insn, value)
}

fn has_code(diagnostics: &[decomp_core::diagnostics::Diagnostic], code: &str) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code() == code)
}

#[test]
fn conflict_use_requires_explicit_opt_in() {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let (source_insn, source) = definition(&mut insns, &mut values, ManagedType::Conflict);
    insns[source_insn] = ManagedInsn::Const {
        result: source,
        value: Constant::Zero,
    };
    let (use_insn, result) = definition(&mut insns, &mut values, ManagedType::Int);
    insns[use_insn] = ManagedInsn::Move {
        result,
        value: source,
    };
    let mut cfg = Cfg::new(source_insn);
    let entry = cfg.entry();
    let use_block = cfg.add_block(use_insn);
    must(cfg.add_edge(entry, use_block, EdgeKind::Unconditional));
    let method = MethodIr::<SsaForm>::from_parts(cfg, insns, values);

    let strict = SsaVerifier::new().verify(&method);
    let permissive = SsaVerifier::new()
        .with_conflict_values_allowed(true)
        .verify(&method);

    assert!(has_code(&strict, "MIR_SSA_CONFLICT_USE"));
    assert!(!has_code(&permissive, "MIR_SSA_CONFLICT_USE"));
}

#[test]
fn width_preserving_instruction_rejects_split_category_two_value() {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let (source_insn, source) = definition(&mut insns, &mut values, ManagedType::Long);
    insns[source_insn] = ManagedInsn::Const {
        result: source,
        value: Constant::Long(1),
    };
    let (move_insn, result) = definition(&mut insns, &mut values, ManagedType::Int);
    insns[move_insn] = ManagedInsn::Move {
        result,
        value: source,
    };
    let mut cfg = Cfg::new(source_insn);
    let entry = cfg.entry();
    let move_block = cfg.add_block(move_insn);
    must(cfg.add_edge(entry, move_block, EdgeKind::Unconditional));
    let method = MethodIr::<SsaForm>::from_parts(cfg, insns, values);

    let diagnostics = SsaVerifier::new().verify(&method);

    assert!(has_code(&diagnostics, "MIR_SSA_WIDE_SPLIT"));
}

#[test]
fn retained_register_history_rejects_half_of_a_wide_value() {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let (insn, value) = definition(&mut insns, &mut values, ManagedType::Double);
    insns[insn] = ManagedInsn::Const {
        result: value,
        value: Constant::DoubleBits(0),
    };
    let cfg = Cfg::new(insn);
    let mut slots = RegisterSlotArena::with_key();
    let mut low = RegisterSlot::new(4);
    low.append(value);
    slots.insert(low);
    let method = MethodIr::<SsaForm>::from_parts_with_registers(cfg, insns, values, slots);

    let diagnostics = SsaVerifier::new().verify(&method);

    assert!(has_code(&diagnostics, "MIR_SSA_WIDE_SPLIT"));
}

#[test]
fn ordering_compare_result_must_be_category_one() {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let (left_insn, left) = definition(&mut insns, &mut values, ManagedType::Long);
    insns[left_insn] = ManagedInsn::Const {
        result: left,
        value: Constant::Long(1),
    };
    let (right_insn, right) = definition(&mut insns, &mut values, ManagedType::Long);
    insns[right_insn] = ManagedInsn::Const {
        result: right,
        value: Constant::Long(2),
    };
    let (cmp_insn, result) = definition(&mut insns, &mut values, ManagedType::Long);
    insns[cmp_insn] = ManagedInsn::OrderingCompare {
        op: OrderingOp::CmpLong,
        result,
        lhs: left,
        rhs: right,
    };
    let mut cfg = Cfg::new(left_insn);
    let second = cfg.add_block(right_insn);
    must(cfg.add_edge(cfg.entry(), second, EdgeKind::Unconditional));
    let third = cfg.add_block(cmp_insn);
    must(cfg.add_edge(second, third, EdgeKind::Unconditional));
    let method = MethodIr::<SsaForm>::from_parts(cfg, insns, values);

    let diagnostics = SsaVerifier::new().verify(&method);

    assert!(has_code(&diagnostics, "MIR_SSA_WIDE_SPLIT"));
}

#[test]
fn ordering_compare_operands_must_have_matching_width() {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let (left_insn, left) = definition(&mut insns, &mut values, ManagedType::Float);
    insns[left_insn] = ManagedInsn::Const {
        result: left,
        value: Constant::FloatBits(0),
    };
    let (right_insn, right) = definition(&mut insns, &mut values, ManagedType::Double);
    insns[right_insn] = ManagedInsn::Const {
        result: right,
        value: Constant::DoubleBits(0),
    };
    let (cmp_insn, result) = definition(&mut insns, &mut values, ManagedType::Int);
    insns[cmp_insn] = ManagedInsn::OrderingCompare {
        op: OrderingOp::CmpLowFloat,
        result,
        lhs: left,
        rhs: right,
    };
    let mut cfg = Cfg::new(left_insn);
    let second = cfg.add_block(right_insn);
    must(cfg.add_edge(cfg.entry(), second, EdgeKind::Unconditional));
    let third = cfg.add_block(cmp_insn);
    must(cfg.add_edge(second, third, EdgeKind::Unconditional));
    let method = MethodIr::<SsaForm>::from_parts(cfg, insns, values);

    let diagnostics = SsaVerifier::new().verify(&method);

    assert!(has_code(&diagnostics, "MIR_SSA_WIDE_SPLIT"));
}

#[test]
fn stale_arena_references_produce_diagnostics_without_panicking() {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let (insn, value) = definition(&mut insns, &mut values, ManagedType::Int);
    insns[insn] = ManagedInsn::Const {
        result: value,
        value: Constant::Int(1),
    };
    let cfg = Cfg::new(insn);
    let removed = values.remove(value);
    assert!(removed.is_some());
    let method = MethodIr::<SsaForm>::from_parts(cfg, insns, values);

    let diagnostics = SsaVerifier::new().verify(&method);

    assert!(has_code(&diagnostics, "MIR_SSA_MISSING_VALUE"));
}
