//! Structural SSA verifier scenarios.

use std::fmt::Debug;

use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::insn::{ArithOp, Constant, ManagedInsn};
use managed_ir::method::{InsnArena, MethodIr, SsaForm, ValueArena};
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
    let insn = insns.insert(ManagedInsn::Opaque {
        family_tag: 0,
        reads: Vec::new(),
        writes: Vec::new(),
        effects: managed_ir::effects::EffectFlags::NONE,
    });
    let value = values.insert(Value::new(insn, ty));
    (insn, value)
}

fn has_code(diagnostics: &[decomp_core::diagnostics::Diagnostic], code: &str) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code() == code)
}

#[test]
fn valid_diamond_with_edge_keyed_phi_passes() {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let (entry_insn, condition) = definition(&mut insns, &mut values, ManagedType::Boolean);
    insns[entry_insn] = ManagedInsn::Const {
        result: condition,
        value: Constant::Boolean(true),
    };
    let (left_insn, left_value) = definition(&mut insns, &mut values, ManagedType::Int);
    insns[left_insn] = ManagedInsn::Const {
        result: left_value,
        value: Constant::Int(1),
    };
    let (right_insn, right_value) = definition(&mut insns, &mut values, ManagedType::Int);
    insns[right_insn] = ManagedInsn::Const {
        result: right_value,
        value: Constant::Int(2),
    };
    let (phi_insn, phi_value) = definition(&mut insns, &mut values, ManagedType::Int);
    let mut cfg = Cfg::new(entry_insn);
    let entry = cfg.entry();
    let left = cfg.add_block(left_insn);
    let right = cfg.add_block(right_insn);
    let join_block = cfg.add_block(phi_insn);
    must(cfg.add_edge(entry, left, EdgeKind::BranchTrue));
    must(cfg.add_edge(entry, right, EdgeKind::BranchFalse));
    let left_edge = must(cfg.add_edge(left, join_block, EdgeKind::Unconditional));
    let right_edge = must(cfg.add_edge(right, join_block, EdgeKind::Unconditional));
    insns[phi_insn] = ManagedInsn::Phi {
        result: phi_value,
        inputs: vec![(left_edge, left_value), (right_edge, right_value)],
    };
    let method = MethodIr::<SsaForm>::from_parts(cfg, insns, values);

    let diagnostics = SsaVerifier::new().verify(&method);

    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

#[test]
fn use_from_non_dominating_sibling_fails() {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let (entry_insn, entry_value) = definition(&mut insns, &mut values, ManagedType::Int);
    insns[entry_insn] = ManagedInsn::Const {
        result: entry_value,
        value: Constant::Int(0),
    };
    let (left_insn, left_value) = definition(&mut insns, &mut values, ManagedType::Int);
    insns[left_insn] = ManagedInsn::Const {
        result: left_value,
        value: Constant::Int(1),
    };
    let (right_insn, right_value) = definition(&mut insns, &mut values, ManagedType::Int);
    insns[right_insn] = ManagedInsn::Move {
        result: right_value,
        value: left_value,
    };
    let mut cfg = Cfg::new(entry_insn);
    let entry = cfg.entry();
    let left = cfg.add_block(left_insn);
    let right = cfg.add_block(right_insn);
    must(cfg.add_edge(entry, left, EdgeKind::BranchTrue));
    must(cfg.add_edge(entry, right, EdgeKind::BranchFalse));
    let method = MethodIr::<SsaForm>::from_parts(cfg, insns, values);

    let diagnostics = SsaVerifier::new().verify(&method);

    assert!(has_code(&diagnostics, "MIR_SSA_DOMINANCE"));
}

#[test]
fn phi_arity_mismatch_fails() {
    let mut fixture = diamond_fixture();
    let ManagedInsn::Phi { inputs, .. } = &mut fixture.1[fixture.3] else {
        panic!("fixture join must be a phi");
    };
    inputs.pop();
    let method = MethodIr::<SsaForm>::from_parts(fixture.0, fixture.1, fixture.2);

    let diagnostics = SsaVerifier::new().verify(&method);

    assert!(has_code(&diagnostics, "MIR_SSA_PHI_ARITY"));
}

#[test]
fn one_value_defined_by_two_instructions_fails() {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let (first_insn, value) = definition(&mut insns, &mut values, ManagedType::Int);
    insns[first_insn] = ManagedInsn::Const {
        result: value,
        value: Constant::Int(1),
    };
    let second_insn = insns.insert(ManagedInsn::Move {
        result: value,
        value,
    });
    let mut cfg = Cfg::new(first_insn);
    let entry = cfg.entry();
    let second = cfg.add_block(second_insn);
    must(cfg.add_edge(entry, second, EdgeKind::Unconditional));
    let method = MethodIr::<SsaForm>::from_parts(cfg, insns, values);

    let diagnostics = SsaVerifier::new().verify(&method);

    assert!(has_code(&diagnostics, "MIR_SSA_DEFINITION_COUNT"));
}

#[test]
fn valid_loop_with_back_edge_phi_passes() {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let (entry_insn, initial) = definition(&mut insns, &mut values, ManagedType::Int);
    insns[entry_insn] = ManagedInsn::Const {
        result: initial,
        value: Constant::Int(0),
    };
    let (phi_insn, induction) = definition(&mut insns, &mut values, ManagedType::Int);
    let (body_insn, next) = definition(&mut insns, &mut values, ManagedType::Int);
    insns[body_insn] = ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result: next,
        lhs: induction,
        rhs: initial,
    };
    let mut cfg = Cfg::new(entry_insn);
    let entry = cfg.entry();
    let header = cfg.add_block(phi_insn);
    let body = cfg.add_block(body_insn);
    let entry_edge = must(cfg.add_edge(entry, header, EdgeKind::Unconditional));
    must(cfg.add_edge(header, body, EdgeKind::BranchTrue));
    let back_edge = must(cfg.add_edge(body, header, EdgeKind::Unconditional));
    insns[phi_insn] = ManagedInsn::Phi {
        result: induction,
        inputs: vec![(entry_edge, initial), (back_edge, next)],
    };
    let method = MethodIr::<SsaForm>::from_parts(cfg, insns, values);

    let diagnostics = SsaVerifier::new().verify(&method);

    assert!(diagnostics.is_empty(), "{diagnostics:#?}");
}

fn diamond_fixture() -> (Cfg<InsnId>, InsnArena, ValueArena, InsnId) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let (entry_insn, entry_value) = definition(&mut insns, &mut values, ManagedType::Int);
    insns[entry_insn] = ManagedInsn::Const {
        result: entry_value,
        value: Constant::Int(0),
    };
    let (left_insn, left_value) = definition(&mut insns, &mut values, ManagedType::Int);
    insns[left_insn] = ManagedInsn::Move {
        result: left_value,
        value: entry_value,
    };
    let (right_insn, right_value) = definition(&mut insns, &mut values, ManagedType::Int);
    insns[right_insn] = ManagedInsn::Move {
        result: right_value,
        value: entry_value,
    };
    let (phi_insn, phi_value) = definition(&mut insns, &mut values, ManagedType::Int);
    let mut cfg = Cfg::new(entry_insn);
    let entry = cfg.entry();
    let left = cfg.add_block(left_insn);
    let right = cfg.add_block(right_insn);
    let join_block = cfg.add_block(phi_insn);
    must(cfg.add_edge(entry, left, EdgeKind::BranchTrue));
    must(cfg.add_edge(entry, right, EdgeKind::BranchFalse));
    let left_edge = must(cfg.add_edge(left, join_block, EdgeKind::Unconditional));
    let right_edge = must(cfg.add_edge(right, join_block, EdgeKind::Unconditional));
    insns[phi_insn] = ManagedInsn::Phi {
        result: phi_value,
        inputs: vec![(left_edge, left_value), (right_edge, right_value)],
    };
    (cfg, insns, values, phi_insn)
}

#[test]
fn phi_cluster_checks_shared_edges_and_each_predecessor_definition() {
    for invalid in 0..=3 {
        let (mut cfg, mut insns, mut values, head_insn) = diamond_fixture();
        let Some(head) = cfg
            .block_ids()
            .find(|block| *must(cfg.block(*block)).payload() == head_insn)
        else {
            panic!("missing phi head");
        };
        let ManagedInsn::Phi {
            result: first,
            inputs,
        } = insns[head_insn].clone()
        else {
            panic!("phi")
        };
        let (second_insn, second) = definition(&mut insns, &mut values, ManagedType::Int);
        let tail = cfg.add_block(second_insn);
        let internal = must(cfg.add_edge(head, tail, EdgeKind::Unconditional));
        let mut second_inputs = inputs.clone();
        match invalid {
            1 => second_inputs[0].0 = internal, // internal edge is not a selector
            2 => second_inputs[0].1 = inputs[1].1, // wrong predecessor value
            3 => second_inputs[0].1 = first,    // cluster result unavailable on entry
            _ => {}
        }
        insns[second_insn] = ManagedInsn::Phi {
            result: second,
            inputs: second_inputs,
        };
        let (sum_insn, sum) = definition(&mut insns, &mut values, ManagedType::Int);
        insns[sum_insn] = ManagedInsn::Arithmetic {
            op: ArithOp::Add,
            result: sum,
            lhs: first,
            rhs: second,
        };
        let continuation = cfg.add_block(sum_insn);
        must(cfg.add_edge(tail, continuation, EdgeKind::Unconditional));
        let method = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
        let diagnostics = SsaVerifier::new().verify(&method);
        if invalid == 0 {
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
        } else if invalid == 1 {
            assert!(has_code(&diagnostics, "MIR_SSA_PHI_EDGES"));
        } else {
            assert!(has_code(&diagnostics, "MIR_SSA_DOMINANCE"));
        }
    }
}

#[test]
fn long_shifts_require_a_narrow_count_and_wide_lhs() {
    for count_type in [ManagedType::Int, ManagedType::Long] {
        let mut insns = InsnArena::with_key();
        let mut values = ValueArena::with_key();
        let (lhs_insn, lhs) = definition(&mut insns, &mut values, ManagedType::Long);
        insns[lhs_insn] = ManagedInsn::Const {
            result: lhs,
            value: Constant::Long(1),
        };
        let (count_insn, count) = definition(&mut insns, &mut values, count_type.clone());
        insns[count_insn] = ManagedInsn::Const {
            result: count,
            value: if count_type.is_wide() {
                Constant::Long(2)
            } else {
                Constant::Int(2)
            },
        };
        let (shift_insn, result) = definition(&mut insns, &mut values, ManagedType::Long);
        insns[shift_insn] = ManagedInsn::Arithmetic {
            op: ArithOp::ShiftLeft,
            result,
            lhs,
            rhs: count,
        };
        let mut cfg = Cfg::new(lhs_insn);
        let count_block = cfg.add_block(count_insn);
        let shift = cfg.add_block(shift_insn);
        must(cfg.add_edge(cfg.entry(), count_block, EdgeKind::Unconditional));
        must(cfg.add_edge(count_block, shift, EdgeKind::Unconditional));
        let method = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
        let diagnostics = SsaVerifier::new().verify(&method);
        assert_eq!(
            has_code(&diagnostics, "MIR_SSA_WIDE_SPLIT"),
            count_type.is_wide(),
            "{diagnostics:?}"
        );
    }
}

#[test]
fn branching_phi_starts_a_separate_cluster() {
    for kind in [EdgeKind::BranchTrue, EdgeKind::Unconditional] {
        let (mut cfg, mut insns, mut values, head_insn) = diamond_fixture();
        let head = cfg
            .block_ids()
            .find(|&id| *must(cfg.block(id)).payload() == head_insn);
        let Some(head) = head else {
            panic!("missing head")
        };
        let ManagedInsn::Phi { result: first, .. } = insns[head_insn] else {
            panic!("missing phi")
        };
        let (next_insn, next) = definition(&mut insns, &mut values, ManagedType::Int);
        let next_block = cfg.add_block(next_insn);
        let edge = must(cfg.add_edge(head, next_block, kind.clone()));
        insns[next_insn] = ManagedInsn::Phi {
            result: next,
            inputs: vec![(edge, first)],
        };
        let method = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
        let diagnostics = SsaVerifier::new().verify(&method);
        if kind == EdgeKind::BranchTrue {
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
        } else {
            assert!(has_code(&diagnostics, "MIR_SSA_PHI_ARITY"));
            assert!(has_code(&diagnostics, "MIR_SSA_PHI_EDGES"));
            assert!(has_code(&diagnostics, "MIR_SSA_DOMINANCE"));
        }
    }
}

#[test]
fn duplicate_phi_selector_is_rejected_even_with_matching_arity() {
    let (cfg, mut insns, values, phi) = diamond_fixture();
    let ManagedInsn::Phi { inputs, .. } = &mut insns[phi] else {
        panic!("missing phi")
    };
    inputs[1] = inputs[0];
    let method = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    let diagnostics = SsaVerifier::new().verify(&method);
    assert!(has_code(&diagnostics, "MIR_SSA_PHI_EDGES"));
    assert!(!has_code(&diagnostics, "MIR_SSA_PHI_ARITY"));
    assert!(!has_code(&diagnostics, "MIR_SSA_DOMINANCE"));
}
