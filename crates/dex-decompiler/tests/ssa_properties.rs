//! Property and directed tests for SSA construction: random valid Dalvik
//! streams through the full 2.4 pipeline checked against brute-force
//! oracles, plus diamond, loop-header, exception-handler, and wide-value
//! scenarios exercised end to end.

use dex_decompiler::{DexPasses, SsaMethod};
use dex_format::RawSpan;
use dex_format::code::{CodeItem, EncodedCatchHandler, TryItem, TypeAddrPair};
use managed_ir::insn::ManagedInsn;
use managed_ir::value::{InsnId, ValueId};
use proptest::prelude::*;
use std::collections::BTreeSet;

#[path = "ssa_phi_check.rs"]
mod phi_check;

fn must<T, E: std::fmt::Debug>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("unexpected error: {error:?}"),
    }
}

fn must_some<T: std::fmt::Debug>(option: Option<T>) -> T {
    match option {
        Some(value) => value,
        None => panic!("unexpected absence"),
    }
}

fn code(registers: u16, units: &[u16]) -> CodeItem {
    let insns = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    CodeItem {
        registers_size: registers,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns,
        tries: Vec::new(),
        handlers: Vec::new(),
        span: RawSpan { offset: 0, size: 0 },
    }
}

fn with_try(code: &CodeItem, start: u32, count: u16, handler: u32) -> CodeItem {
    let mut item = code.clone();
    item.tries = vec![TryItem {
        start_addr: start,
        insn_count: count,
        handler_off: 0,
        span: RawSpan {
            offset: 0,
            size: u32::from(count),
        },
    }];
    item.handlers = vec![EncodedCatchHandler {
        relative_offset: 0,
        handlers: vec![TypeAddrPair {
            type_idx: 77,
            address: handler,
        }],
        catch_all_addr: None,
        span: RawSpan { offset: 0, size: 8 },
    }];
    item
}

fn phis_of(ssa: &SsaMethod) -> Vec<(decomp_core::ids::BlockId, InsnId, ValueId)> {
    let cfg = ssa.ir().cfg();
    cfg.block_ids()
        .filter_map(|block| {
            let insn = *cfg.block(block).ok()?.payload();
            match ssa.ir().instructions().get(insn)? {
                ManagedInsn::Phi { result, .. } => Some((block, insn, *result)),
                _ => None,
            }
        })
        .collect()
}

/// A random instruction-stream generator: fixed-width atoms with in-range
/// branch targets, mirrored from the corpus-proven fuzz shape.
fn stream() -> impl Strategy<Value = Vec<u16>> {
    (2_usize..=18, any::<u64>()).prop_map(|(n, seed)| {
        let mut state = seed | 1;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut below = |bound: u64| next() % bound.max(1);
        let kinds: Vec<u8> = (0..n).map(|_| below(8) as u8).collect();
        let targets: Vec<usize> = (0..n).map(|_| below(n as u64) as usize).collect();
        let mut offsets = Vec::with_capacity(n);
        let mut cursor = 0_usize;
        for kind in &kinds {
            offsets.push(cursor);
            cursor += match kind {
                5 | 7 => 2,
                _ => 1,
            };
        }
        let mut units = Vec::with_capacity(cursor);
        for (atom, kind) in kinds.iter().enumerate() {
            match kind {
                0 => units.push(0x0012),
                1 => units.push(0x1012),
                2 => units.push(0x2301),
                3 => units.push(0x000e),
                4 => units.push(0x000f),
                5 => {
                    units.push(0x00d8);
                    units.push(0x0100 | ((below(3) as u16) << 8));
                }
                6 => {
                    let delta = offsets[targets[atom]] as i64 - units.len() as i64;
                    let Ok(off) = i8::try_from(delta) else {
                        units.push(0x000e);
                        continue;
                    };
                    units.push(0x2800 | ((off as u8 as u16) << 8));
                }
                _ => {
                    let delta = offsets[targets[atom]] as i64 - (units.len() + 2) as i64;
                    let Ok(off) = i16::try_from(delta) else {
                        units.push(0x000e);
                        continue;
                    };
                    units.push(0x0038 | ((below(2) as u16) << 8));
                    units.push(off as u16);
                }
            }
        }
        units
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn random_streams_survive_the_full_pipeline(units in stream()) {
        let item = code(8, &units);
        match DexPasses::run_merged(&item) {
            Ok(ssa) => {
                prop_assert_eq!(
                    ssa.stats().ssa_blocks,
                    ssa.ir().cfg().block_count(),
                    "stats must match the published CFG"
                );
                phi_check::check(&ssa).map_err(TestCaseError::fail)?;
            }
            Err(dex_decompiler::StageError::Ssa(dex_decompiler::SsaError::Verification(
                diagnostics,
            ))) => {
                return Err(TestCaseError::fail(format!(
                    "ssa verification rejected units {units:?}: {diagnostics:?}"
                )));
            }
            Err(_) => {
                // Merge/structural errors abort with typed diagnostics;
                // they must exist but random streams rarely reach them.
                prop_assume!(false, "typed stage error");
            }
        }
    }

    #[test]
    fn ssa_preserves_offsets_and_never_grows_blocks(units in stream()) {
        let item = code(8, &units);
        let Ok(lifted) = DexPasses::run(&item) else {
            prop_assume!(false, "lift rejected a decodable stream");
            return Ok(());
        };
        let lifted_blocks = lifted.ir().cfg().block_count();
        let original_offsets: std::collections::HashMap<_, _> = lifted
            .ir()
            .instructions()
            .keys()
            .filter_map(|id| lifted.offset(id).map(|offset| (id, offset)))
            .collect();
        let merged = dex_decompiler::merge_method(lifted).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(merged.cfg().block_count() <= lifted_blocks);
        let merged_blocks = merged.cfg().block_count();
        let ssa = dex_decompiler::to_ssa(merged).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
        prop_assert!(ssa.stats().merged_blocks <= merged_blocks);
        // Surviving original insns keep their code-unit provenance; newly
        // created synthetic definitions (phis, hoisted constants, entry
        // initials) legitimately have none.
        for block in ssa.ir().cfg().block_ids() {
            let insn = *ssa
                .ir()
                .cfg()
                .block(block)
                .map_err(|e| TestCaseError::fail(e.to_string()))?
                .payload();
            if let (Some(lifted_offset), Some(ssa_offset)) = (original_offsets.get(&insn), ssa.offset(insn)) {
                prop_assert_eq!(*lifted_offset, ssa_offset, "insn moved");
            }
        }
    }
}

/// Diamond: both arms define v1, the join consumes it — exactly one phi
/// must sit on the two-predecessor join and select both arm values.
#[test]
fn diamond_join_carries_one_well_formed_phi() {
    // 0 const/4 v0, 1 ; 1-2 if-eqz v0, +4 ; 3 const/4 v1, 2 ; 4 goto +2 ;
    // 5 const/4 v1, 3 ; 6 return v1
    let item = code(4, &[0x1012, 0x0038, 0x0004, 0x2112, 0x0228, 0x3112, 0x010f]);
    let ssa = match DexPasses::run_merged(&item) {
        Ok(ssa) => ssa,
        Err(error) => panic!("pipeline rejected diamond: {error:?}"),
    };
    must(phi_check::check(&ssa));
    let phis = phis_of(&ssa);
    assert_eq!(phis.len(), 1, "exactly one join phi expected");
    let (block, insn, _) = phis[0];
    let record = must(ssa.ir().cfg().block(block)).clone();
    assert_eq!(record.incoming().len(), 2, "diamond join has two preds");
    let Some(ManagedInsn::Phi { inputs, .. }) = ssa.ir().instructions().get(insn) else {
        panic!("payload must be the phi");
    };
    assert_eq!(inputs.len(), 2);
    let values: BTreeSet<ValueId> = inputs.iter().map(|(_, value)| *value).collect();
    assert_eq!(values.len(), 2, "both arm values must be selected");
    // Each input is defined on its edge's source side: either exactly in
    // the source block or in a block dominating it (fusion folds the arm's
    // definition into a longer chain whose tail is the edge source).
    for (edge, value) in inputs {
        let edge_record = must(ssa.ir().cfg().edge(*edge));
        let def_insn = must_some(ssa.ir().values().get(*value)).def();
        let def_block = must_some(ssa.ir().cfg().block_ids().find(|block| {
            ssa.ir()
                .cfg()
                .block(*block)
                .is_ok_and(|record| *record.payload() == def_insn)
        }));
        let source = edge_record.source();
        assert!(
            def_block == source || phi_check::dominates(&ssa, def_block, source),
            "phi input {value:?} must be defined on its edge source side"
        );
    }
}

/// Loop: the induction phi at the loop header must merge the initializer
/// from the preheader with the body-computed value on the back edge — the
/// loop-carried dependence. (A phi whose only non-self input is the
/// initializer is trivial and correctly eliminated instead.)
#[test]
fn loop_header_phi_carries_the_induction_variable() {
    // 0 const/4 v0, 0 ; 1 const/4 v1, 3 ; 2-3 if-ge v0, v1, +3 ;
    // 4 add-int/2addr v0, v1 ; 5 goto -2 ; 6 return v0
    let item = code(4, &[0x0012, 0x3112, 0x1035, 0x0004, 0x10b0, 0xfd28, 0x000f]);
    let ssa = match DexPasses::run_merged(&item) {
        Ok(ssa) => ssa,
        Err(error) => panic!("pipeline rejected loop: {error:?}"),
    };
    must(phi_check::check(&ssa));
    let phis = phis_of(&ssa);
    assert_eq!(phis.len(), 1, "exactly the induction phi expected");
    let (block, insn, _) = phis[0];
    let record = must(ssa.ir().cfg().block(block)).clone();
    assert_eq!(record.incoming().len(), 2, "loop header has two preds");
    let Some(ManagedInsn::Phi { inputs, .. }) = ssa.ir().instructions().get(insn) else {
        panic!("payload must be the phi");
    };
    assert_eq!(inputs.len(), 2);
    let values: BTreeSet<ValueId> = inputs.iter().map(|(_, value)| *value).collect();
    assert_eq!(values.len(), 2, "initializer and body value must differ");
    let header = block;
    // The body-side input must be defined strictly inside the loop: its
    // definition block is the header's own successor chain, not the
    // preheader (which has exactly one incoming edge of its own).
    let def_blocks: Vec<_> = inputs
        .iter()
        .map(|(_, value)| {
            let def = must_some(ssa.ir().values().get(*value)).def();
            must_some(ssa.ir().cfg().block_ids().find(|block| {
                ssa.ir()
                    .cfg()
                    .block(*block)
                    .is_ok_and(|record| *record.payload() == def)
            }))
        })
        .collect();
    let pre = must_some(
        def_blocks
            .iter()
            .copied()
            .find(|block| must(ssa.ir().cfg().block(*block)).incoming().len() == 1),
    );
    let body = must_some(def_blocks.iter().copied().find(|block| *block != pre));
    assert_ne!(pre, body, "the two inputs come from different loops sides");
    assert_ne!(body, header, "the body input is computed below the header");
}

/// Try/catch: the guarded instruction's exceptional edge enters the
/// handler, move-exception receives its synthetic input, and the join
/// after the handler merges both paths under verified dominance.
#[test]
fn try_catch_handler_keeps_exception_flow_verifiable() {
    // 0 const/4 v0, 1 (try) ; 1 div-int/2addr v0, v0 (try) ; 2 goto +2 ;
    // 3 move-exception v1 (handler) ; 4 return v0
    let item = code(4, &[0x1012, 0x00b3, 0x0228, 0x010d, 0x000f]);
    let item = with_try(&item, 0, 2, 3);
    let ssa = match DexPasses::run_merged(&item) {
        Ok(ssa) => ssa,
        Err(error) => panic!("pipeline rejected try/catch: {error:?}"),
    };
    must(phi_check::check(&ssa));
    let cfg = ssa.ir().cfg();
    let exception_edges: Vec<_> = cfg
        .edge_ids()
        .filter(|edge| {
            matches!(
                must(cfg.edge(*edge)).kind(),
                decomp_core::cfg::EdgeKind::Exception(_)
            )
        })
        .collect();
    assert!(
        !exception_edges.is_empty(),
        "exceptional edges must survive materialization"
    );
    for edge in &exception_edges {
        let record = must(cfg.edge(*edge));
        let target = must(cfg.block(record.target()));
        let handler_insn = *target.payload();
        assert!(
            ssa.ir().instructions().get(handler_insn).is_some(),
            "handler instruction must be placed"
        );
    }
    // The handler's move-exception chain is present: an Opaque with the
    // TAG_HANDLER family written by hoisted placement, consumed by a Move.
    let mut handler_opaques = 0;
    let mut handler_moves = 0;
    for (id, insn) in ssa.ir().instructions() {
        match insn {
            ManagedInsn::Opaque { family_tag, .. } if *family_tag == 0x8002 => {
                handler_opaques += 1;
            }
            ManagedInsn::Move { .. } if ssa.offset(id) == Some(3) => handler_moves += 1,
            _ => {}
        }
    }
    assert_eq!(handler_opaques, 1, "one synthetic handler input");
    assert_eq!(handler_moves, 1, "move-exception lowers to a Move");
}

/// Wide atomicity: both arms define the same category-two register pair;
/// the join phi must select the two atomic values, and the register-slot
/// histories must keep each of them as one adjacent pair.
#[test]
fn wide_pair_stays_atomic_across_phi() {
    // 0 const/4 v2, 1 ; 1-2 if-eqz v2, +5 ; 3-4 const-wide/16 v0, 7 ;
    // 5 goto +3 ; 6-7 const-wide/16 v0, -2 ; 8 return-wide v0
    let item = code(
        4,
        &[
            0x1212, 0x0238, 0x0005, 0x0016, 0x0007, 0x0328, 0x0016, 0xfffe, 0x0010,
        ],
    );
    let ssa = match DexPasses::run_merged(&item) {
        Ok(ssa) => ssa,
        Err(error) => panic!("pipeline rejected wide diamond: {error:?}"),
    };
    must(phi_check::check(&ssa));
    let phis = phis_of(&ssa);
    assert_eq!(phis.len(), 1, "exactly the wide join phi expected");
    let (block, insn, _) = phis[0];
    let record = must(ssa.ir().cfg().block(block)).clone();
    assert_eq!(record.incoming().len(), 2);
    let Some(ManagedInsn::Phi { inputs, .. }) = ssa.ir().instructions().get(insn) else {
        panic!("payload must be the phi");
    };
    // Each phi input is one atomic wide value whose def is a Const in the
    // corresponding arm, and whose register history is the adjacent pair.
    for (edge, value) in inputs {
        let edge_record = must(ssa.ir().cfg().edge(*edge));
        let def_insn = must_some(ssa.ir().values().get(*value)).def();
        let Some(ManagedInsn::Const {
            value: constant, ..
        }) = ssa.ir().instructions().get(def_insn)
        else {
            panic!("wide arm must define the pair with a constant");
        };
        assert!(
            matches!(constant, managed_ir::insn::Constant::Long(_)),
            "arm constant must be a wide literal, got {constant:?}"
        );
        let mut slots: Vec<u16> = Vec::new();
        for (_, slot) in ssa.ir().register_slots().iter() {
            if slot.values().contains(value) {
                slots.push(slot.index());
            }
        }
        slots.sort_unstable();
        assert_eq!(
            slots.as_slice(),
            &[0, 1],
            "wide value {value:?} must own register pair v0:v1"
        );
        assert_ne!(
            edge_record.source(),
            block,
            "arm definitions live in their own blocks"
        );
    }
}
