//! Lifting decoded Dalvik instructions into managed IR with a validated CFG.

use std::fmt::Debug;

use decomp_core::cfg::{Cfg, EdgeKind};
use dex_decompiler::{LiftError, lift_method};
use dex_format::RawSpan;
use dex_format::code::{CodeItem, EncodedCatchHandler, TryItem, TypeAddrPair};
use managed_ir::insn::{Constant, InvokeKind, ManagedInsn, OrderingOp};
use managed_ir::value::InsnId;

fn must<T, E: Debug>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("unexpected error: {error:?}"),
    }
}

fn must_some<T: Debug>(option: Option<T>) -> T {
    match option {
        Some(value) => value,
        None => panic!("unexpected absence"),
    }
}

fn code(registers_size: u16, units: &[u16]) -> CodeItem {
    let insns = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    CodeItem {
        registers_size,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: must(u32::try_from(units.len())),
        insns,
        tries: Vec::new(),
        handlers: Vec::new(),
        span: RawSpan { offset: 0, size: 0 },
    }
}

fn payload_of<'a>(
    cfg: &Cfg<InsnId>,
    lifted: &'a dex_decompiler::LiftedMethod,
    block: decomp_core::ids::BlockId,
) -> &'a ManagedInsn {
    let insn = must(cfg.block(block)).payload();
    must_some(lifted.ir().instructions().get(*insn))
}

#[test]
fn invoke_fuses_following_move_result() {
    // 0: const/4 v0, 1 ; 1: invoke-static {} ; 2: move-result v0 ; 3: return-void
    let item = code(16, &[0x1012, 0x0071, 0x0000, 0x0000, 0x000a, 0x000e]);
    let lifted = must(lift_method(&item));
    let cfg = lifted.ir().cfg();
    assert_eq!(cfg.block_count(), 3, "const, fused invoke, return");
    let mut invokes = 0;
    let mut moves = 0;
    for id in lifted.ir().instructions().keys() {
        match lifted.ir().instructions().get(id) {
            Some(ManagedInsn::Invoke {
                kind,
                result,
                method,
                args,
            }) => {
                invokes += 1;
                assert_eq!(*kind, InvokeKind::Static);
                assert_eq!(method.index(), 0);
                assert!(args.is_empty());
                assert!(result.is_some(), "move-result must be fused");
            }
            Some(ManagedInsn::Move { .. }) => moves += 1,
            _ => {}
        }
    }
    assert_eq!(invokes, 1);
    assert_eq!(moves, 0, "fused move-result must not become its own block");
    let _ = payload_of(cfg, &lifted, cfg.entry());
}

#[test]
fn diamond_yields_four_blocks_four_edges() {
    // 0: if-eqz v0, +4 ; 2: const/4 v0, 1 ; 3: goto +1 ; 4: return-void
    let item = code(16, &[0x0032, 0x0004, 0x1012, 0x0128, 0x000e]);
    let lifted = must(lift_method(&item));
    let cfg = lifted.ir().cfg();
    assert_eq!(cfg.block_count(), 4);
    assert_eq!(cfg.edge_count(), 4);
    let mut true_edges = 0;
    let mut false_edges = 0;
    let mut unconditional = 0;
    for edge_id in cfg.edge_ids() {
        match must(cfg.edge(edge_id)).kind() {
            EdgeKind::BranchTrue => true_edges += 1,
            EdgeKind::BranchFalse => false_edges += 1,
            EdgeKind::Unconditional => unconditional += 1,
            other => panic!("unexpected edge {other:?}"),
        }
    }
    assert_eq!((true_edges, false_edges, unconditional), (1, 1, 2));
    match payload_of(cfg, &lifted, cfg.entry()) {
        ManagedInsn::Branch {
            condition: Some(_),
            then_edge: _,
            else_edge: Some(_),
        } => {}
        other => panic!("expected conditional branch at entry, got {other:?}"),
    }
}

#[test]
fn packed_switch_resolves_payload_cases() {
    // 0: packed-switch v0, payload@4 ; 3: return-void ; 4..: payload (keys 10..12, targets +3)
    let mut units = vec![
        0x002b, 0x0004, 0x0000, 0x000e, 0x0100, 0x0003, 0x000a, 0x0000,
    ];
    for _ in 0..3 {
        units.extend([0x0003, 0x0000]);
    }
    let item = code(16, &units);
    let lifted = must(lift_method(&item));
    let cfg = lifted.ir().cfg();
    assert_eq!(cfg.block_count(), 2, "payloads must not become blocks");
    assert_eq!(cfg.edge_count(), 4, "three cases plus default fallthrough");
    let mut keys = Vec::new();
    for edge_id in cfg.edge_ids() {
        match must(cfg.edge(edge_id)).kind() {
            EdgeKind::Switch(key) => keys.push(*key),
            EdgeKind::Unconditional => {}
            other => panic!("unexpected edge {other:?}"),
        }
    }
    keys.sort_unstable();
    assert_eq!(keys, vec![10, 11, 12]);
    match payload_of(cfg, &lifted, cfg.entry()) {
        ManagedInsn::Switch {
            key: _,
            default_edge: _,
            cases,
        } => {
            let mut values: Vec<i64> = cases.iter().map(|(value, _)| *value).collect();
            values.sort_unstable();
            assert_eq!(values, vec![10, 11, 12]);
        }
        other => panic!("expected switch at entry, got {other:?}"),
    }
}

#[test]
fn three_way_compares_lift_to_typed_ordering_ir() {
    // 0: cmp-long v0, v2, v4 ; 2: cmpl-float v1, v6, v7 ; 4: cmpg-double v8, v10, v12
    let item = code(
        16,
        &[0x0031, 0x0402, 0x012d, 0x0706, 0x0830, 0x0c0a, 0x000e],
    );
    let lifted = must(lift_method(&item));
    let mut ops = Vec::new();
    let mut opaque_cmp = 0;
    for id in lifted.ir().instructions().keys() {
        match lifted.ir().instructions().get(id) {
            Some(ManagedInsn::OrderingCompare {
                op,
                result,
                lhs,
                rhs,
            }) => {
                assert_eq!(
                    lifted.ir().values().get(*result).map(|v| v.ty()),
                    Some(&managed_ir::types::ManagedType::Int)
                );
                assert_ne!(lhs, rhs, "operands remain distinct values");
                ops.push(*op);
            }
            Some(ManagedInsn::Opaque { family_tag, .. }) if (0x2d..=0x31).contains(family_tag) => {
                opaque_cmp += 1;
            }
            _ => {}
        }
    }
    ops.sort_by_key(|op| match op {
        OrderingOp::CmpLowFloat => 0,
        OrderingOp::CmpHighFloat => 1,
        OrderingOp::CmpLowDouble => 2,
        OrderingOp::CmpHighDouble => 3,
        OrderingOp::CmpLong => 4,
    });
    assert_eq!(
        ops,
        vec![
            OrderingOp::CmpLowFloat,
            OrderingOp::CmpHighDouble,
            OrderingOp::CmpLong,
        ]
    );
    assert_eq!(opaque_cmp, 0, "cmp* opcodes must not degrade to opaque");
}

#[test]
fn try_catch_creates_exception_edges() {
    // 0: monitor-enter v0; 1: monitor-exit v0; 2: return-void;
    // 3: move-exception v0; 4: return-void. Only the monitors can throw.
    let mut item = code(16, &[0x001d, 0x001e, 0x000e, 0x000d, 0x000e]);
    item.tries.push(TryItem {
        start_addr: 0,
        insn_count: 3,
        handler_off: 0,
        span: RawSpan { offset: 0, size: 0 },
    });
    item.handlers.push(EncodedCatchHandler {
        relative_offset: 0,
        handlers: vec![TypeAddrPair {
            type_idx: 7,
            address: 3,
        }],
        catch_all_addr: None,
        span: RawSpan { offset: 0, size: 0 },
    });
    let lifted = must(lift_method(&item));
    let cfg = lifted.ir().cfg();
    let mut exception_edges = 0;
    for edge_id in cfg.edge_ids() {
        let edge = must(cfg.edge(edge_id));
        if let EdgeKind::Exception(name) = edge.kind() {
            exception_edges += 1;
            assert_eq!(name, "7");
        }
    }
    assert_eq!(
        exception_edges, 2,
        "both monitors, but not return, reach the handler"
    );
    let mut move_count = 0;
    for id in lifted.ir().instructions().keys() {
        if let Some(ManagedInsn::Move { value, .. }) = lifted.ir().instructions().get(id) {
            let def = must_some(lifted.ir().values().get(*value)).def();
            assert!(matches!(
                lifted.ir().instructions().get(def),
                Some(ManagedInsn::Opaque { .. })
            ));
            move_count += 1;
        }
    }
    assert_eq!(move_count, 1, "move-exception becomes one Move");
}

#[test]
fn wide_register_pair_is_one_atomic_value() {
    // 0: const-wide/16 v0, 42 ; 1: return-wide v0
    let item = code(16, &[0x0016, 0x002a, 0x0010]);
    let lifted = must(lift_method(&item));
    let mut wide_value = None;
    for id in lifted.ir().instructions().keys() {
        if let Some(ManagedInsn::Const { result, value }) = lifted.ir().instructions().get(id) {
            assert_eq!(*value, Constant::Long(42));
            wide_value = Some(*result);
        }
    }
    let wide_value = must_some(wide_value);
    assert!(
        lifted
            .ir()
            .values()
            .get(wide_value)
            .map(|record| record.ty().is_wide())
            == Some(true)
    );
    let slots = lifted.ir().register_slots();
    let history = |index: u16| {
        slots
            .values()
            .find_map(|slot| (slot.index() == index).then(|| slot.values().to_vec()))
    };
    let v0_history = must_some(history(0));
    let v1_history = must_some(history(1));
    assert_eq!(v0_history, vec![wide_value]);
    assert_eq!(v1_history, vec![wide_value]);
}

#[test]
fn adversarial_bins_error_or_degrade_not_panic() {
    // Two reserved opcodes exhaust the recovery budget.
    let aborted = code(0, &[0x0073, 0x0073]);
    assert!(matches!(
        lift_method(&aborted),
        Err(LiftError::DecodeAborted { .. })
    ));
    // Branch target outside the stream degrades to a conservative opaque.
    let oob = code(4, &[0x0505, 0x000e]);
    let lifted = must(lift_method(&oob));
    assert!(lifted.ir().cfg().validate().is_ok());
    // Lying insns_size is clamped by the recovering decoder.
    let mut lying = code(0, &[0x0000, 0x000e]);
    lying.insns_size = 8;
    assert!(lift_method(&lying).is_ok());
}

#[test]
fn empty_body_synthesizes_return() {
    let item = code(4, &[]);
    let lifted = must(lift_method(&item));
    let cfg = lifted.ir().cfg();
    assert_eq!(cfg.block_count(), 1);
    assert!(matches!(
        payload_of(cfg, &lifted, cfg.entry()),
        ManagedInsn::Return { value: None }
    ));
}

#[test]
fn offsets_side_table_tracks_code_units() {
    // 0: const/4 v0, 1 ; 1: return v0
    let item = code(16, &[0x1012, 0x000f]);
    let lifted = must(lift_method(&item));
    let mut offsets: Vec<Option<u32>> = lifted
        .ir()
        .instructions()
        .keys()
        .map(|id| lifted.offset(id))
        .collect();
    offsets.sort_unstable();
    assert_eq!(offsets, vec![Some(0), Some(1)]);
    // Every CFG block payload carries code-unit provenance.
    let cfg = lifted.ir().cfg();
    let mut provenance = 0;
    for block_id in cfg.block_ids() {
        let payload = must(cfg.block(block_id)).payload();
        assert!(
            lifted.offset(*payload).is_some(),
            "block missing provenance"
        );
        provenance += 1;
    }
    assert_eq!(provenance, 2);
}

#[test]
fn const_kinds_map_literals() {
    // 0: const/4 v0, 0 ; 1: const-string v1, string@3 ; 3: return-void
    let item = code(16, &[0x0012, 0x011a, 0x0003, 0x000e]);
    let lifted = must(lift_method(&item));
    let mut zeros = 0;
    let mut strings = 0;
    for id in lifted.ir().instructions().keys() {
        if let Some(ManagedInsn::Const { value, .. }) = lifted.ir().instructions().get(id) {
            match value {
                Constant::Zero => zeros += 1,
                Constant::String(reference) => {
                    strings += 1;
                    assert_eq!(reference.index(), 3);
                }
                other => panic!("unexpected constant {other:?}"),
            }
        }
    }
    assert_eq!(zeros, 1);
    assert_eq!(strings, 1);
    assert_eq!(lifted.ir().cfg().block_count(), 3);
}
