//! The implicit invocation edge makes original entry distinct from a loop seam.
#![allow(clippy::unwrap_used)]
use dex_decompiler::{DexPasses, TypeContext, merge_method};
use dex_format::{RawSpan, code::CodeItem};
use managed_ir::{insn::ManagedInsn, refs::ClassRef, types::ManagedType};
use std::collections::HashMap;
fn code(units: &[u16], registers: u16, incoming: u16) -> CodeItem {
    CodeItem {
        registers_size: registers,
        ins_size: incoming,
        outs_size: 2,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|w| w.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: RawSpan { offset: 0, size: 0 },
    }
}
struct ViewTypes;
impl TypeContext for ViewTypes {
    fn initial_register_type(&self, r: u16, wide: bool) -> Option<ManagedType> {
        (!wide && matches!(r, 1 | 2)).then_some(ManagedType::Reference(ClassRef::new(0)))
    }
    fn type_at(&self, _: u32) -> Option<ManagedType> {
        Some(ManagedType::Reference(ClassRef::new(0)))
    }
    fn method_return(&self, _: u32) -> Option<ManagedType> {
        Some(ManagedType::Reference(ClassRef::new(0)))
    }
    fn method_params(&self, m: u32, p: &mut Vec<ManagedType>) -> bool {
        p.clear();
        if m == 1 {
            p.push(ManagedType::Reference(ClassRef::new(0)));
        }
        true
    }
    fn enclosing_return(&self) -> Option<ManagedType> {
        Some(ManagedType::Reference(ClassRef::new(0)))
    }
}
fn view_loop() -> CodeItem {
    code(
        &[
            0x106e, 0, 2, 0x000c, 0x1032, 12, 0x0220, 0, 0x0239, 4, 0x0112, 0x0111, 0x0207, 0x021f,
            0, 0xf128, 0x206e, 1, 0x0021, 0x010c, 0x0111,
        ],
        3,
        2,
    )
}
#[test]
fn original_entry_operation_survives_sole_backedge_and_normal_fusion() {
    let lift = DexPasses::run_with(&view_loop(), &ViewTypes).unwrap();
    let entry = *lift
        .ir()
        .cfg()
        .block(lift.ir().cfg().entry())
        .unwrap()
        .payload();
    assert_eq!(lift.offset(entry), Some(0));
    let offsets: HashMap<_, _> = lift
        .ir()
        .instructions()
        .keys()
        .map(|id| (id, lift.offset(id)))
        .collect();
    let merged = merge_method(lift).unwrap();
    let e = merged.cfg().block(merged.cfg().entry()).unwrap();
    assert_eq!(e.payload().first(), Some(&entry));
    assert_eq!(
        e.incoming().len(),
        1,
        "retain actual loop backedge into original entry"
    );
    assert!(
        merged
            .cfg()
            .block_ids()
            .filter(|b| *b != merged.cfg().entry())
            .all(|b| !merged.cfg().block(b).unwrap().payload().contains(&entry))
    );
    assert_eq!(
        e.payload().iter().map(|i| offsets[i]).collect::<Vec<_>>(),
        vec![Some(0), Some(4)]
    );
    assert!(
        merged
            .cfg()
            .block_ids()
            .any(|b| merged.cfg().block(b).unwrap().payload().len() > 1),
        "ordinary seams still fuse"
    );
    merged.cfg().validate().unwrap();
}
#[test]
fn literal_entry_goto_retained_but_nonentry_goto_still_fuses() {
    let lift = DexPasses::run(&code(&[0x0228, 0x7012, 0x1012, 0x000f], 1, 0)).unwrap();
    let entry = *lift
        .ir()
        .cfg()
        .block(lift.ir().cfg().entry())
        .unwrap()
        .payload();
    let merged = merge_method(lift).unwrap();
    assert_eq!(
        merged.cfg().block(merged.cfg().entry()).unwrap().payload(),
        &vec![entry]
    );
    assert!(matches!(
        merged.instructions()[entry],
        ManagedInsn::Branch {
            condition: None,
            ..
        }
    ));
    assert_eq!(merged.dead_blocks(), 1);
    assert_eq!(merged.dropped_gotos(), 0);
    let item = code(&[0x1012, 0x0128, 0x2112, 0x010f], 2, 0);
    let lift = DexPasses::run(&item).unwrap();
    let entry = *lift
        .ir()
        .cfg()
        .block(lift.ir().cfg().entry())
        .unwrap()
        .payload();
    let merged = merge_method(lift).unwrap();
    assert_eq!(merged.cfg().block_count(), 1);
    assert_eq!(merged.dropped_gotos(), 1);
    assert_eq!(
        merged
            .cfg()
            .block(merged.cfg().entry())
            .unwrap()
            .payload()
            .first(),
        Some(&entry)
    );
    assert!(DexPasses::run_merged(&item).is_ok());
}
#[test]
fn entry_loop_phi_includes_actual_invocation_input_and_backedge_update() {
    let ssa = DexPasses::run_ssa_with(&view_loop(), &ViewTypes).unwrap();
    let ir = ssa.ir();
    let operand = ir
        .instructions()
        .iter()
        .find_map(|(id, insn)| match insn {
            ManagedInsn::Invoke { method, args, .. }
                if method.index() == 0 && ssa.offset(id) == Some(0) =>
            {
                Some(args[0])
            }
            _ => None,
        })
        .unwrap();
    let def = ir.value(operand).unwrap().def();
    let ManagedInsn::Phi { inputs, .. } = ir.instruction(def).unwrap() else {
        panic!(
            "original getParent must select child on invocation and cast parent on backedge; got {:?}",
            ir.instruction(def)
        )
    };
    assert_eq!(inputs.len(), 2);
    assert!(
        inputs
            .iter()
            .any(|(_, v)| ssa.initial_registers().get(v) == Some(&vec![2]))
    );
    assert!(inputs.iter().any(|(_, v)| matches!(
        ir.instruction(ir.value(*v).unwrap().def()),
        Ok(ManagedInsn::Cast { .. })
    )));
}

#[test]
#[ignore = "retained original Samsung APK; bounded exact ViewPager/ArrayEncoder entry recurrences"]
fn original_samsung_entry_effects_and_initial_phi_inputs_are_retained() {
    let files = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    for (pool, index, expected_owner, expected_name, input_register) in [
        (
            0,
            21016,
            "Landroidx/viewpager/widget/ViewPager;",
            "infoForAnyChild",
            2,
        ),
        (
            1,
            5433,
            "Lcom/google/protobuf/CodedOutputStream$ArrayEncoder;",
            "writeUInt32NoTag",
            4,
        ),
    ] {
        let dex = &files[pool];
        let raw = dex.method_id(index).unwrap();
        assert_eq!(
            dex.string(
                dex.type_id(u32::from(raw.class_idx))
                    .unwrap()
                    .descriptor_idx
            )
            .unwrap(),
            expected_owner
        );
        assert_eq!(dex.string(raw.name_idx).unwrap(), expected_name);
        let encoded = dex
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|c| c.direct_methods.iter().chain(&c.virtual_methods))
            .find(|m| m.method_idx == index)
            .unwrap();
        let item = encoded.code.as_ref().unwrap();
        let ctx = dex_decompiler::DexTypes::new(dex, Some(index));
        let mut register = item.registers_size - item.ins_size;
        let mut seeds = HashMap::new();
        if encoded.access_flags & 8 == 0 {
            seeds.insert(
                (register, false),
                ctx.type_at(u32::from(raw.class_idx)).unwrap(),
            );
            register += 1;
        }
        let mut parameters = Vec::new();
        assert!(ctx.method_params(index, &mut parameters));
        for ty in parameters {
            let wide = ty.category() == Some(managed_ir::types::ValueCategory::Two);
            seeds.insert((register, wide), ty);
            register += if wide { 2 } else { 1 };
        }
        assert_eq!(register, item.registers_size);
        let seeded = ctx.with_initial_registers(seeds);
        let lift = DexPasses::run_with(item, &seeded).unwrap();
        let entry = *lift
            .ir()
            .cfg()
            .block(lift.ir().cfg().entry())
            .unwrap()
            .payload();
        assert_eq!(lift.offset(entry), Some(0));
        let merged = merge_method(lift).unwrap();
        assert_eq!(
            merged
                .cfg()
                .block(merged.cfg().entry())
                .unwrap()
                .payload()
                .first(),
            Some(&entry)
        );
        let ssa = DexPasses::run_ssa_with(item, &seeded).unwrap();
        let ir = ssa.ir();
        let operand = match ir.instruction(entry).unwrap() {
            ManagedInsn::Invoke { args, .. } => args[0],
            ManagedInsn::Arithmetic { lhs, .. } => *lhs,
            other => panic!("unexpected original entry {other:?}"),
        };
        let ManagedInsn::Phi { inputs, .. } =
            ir.instruction(ir.value(operand).unwrap().def()).unwrap()
        else {
            panic!("{expected_name} original input recurrence has no Phi")
        };
        assert_eq!(inputs.len(), 2);
        assert!(
            inputs
                .iter()
                .any(|(_, v)| ssa.initial_registers().get(v) == Some(&vec![input_register]))
        );
        assert!(
            inputs
                .iter()
                .any(|(_, v)| ssa.offset(ir.value(*v).unwrap().def()).is_some()),
            "retain actual offset-bearing backedge update"
        );
        // Synthetic entry has no catchable effect; the first original operation
        // is still offset0, after only initial definitions and header Phis.
        let mut block = ir.cfg().entry();
        for _ in 0..ir.cfg().block_count() {
            let b = ir.cfg().block(block).unwrap();
            let id = *b.payload();
            if let Some(offset) = ssa.offset(id) {
                assert_eq!(offset, 0);
                break;
            }
            assert!(matches!(
                ir.instruction(id).unwrap(),
                ManagedInsn::Const { .. }
                    | ManagedInsn::Phi { .. }
                    | ManagedInsn::Branch {
                        condition: None,
                        ..
                    }
                    | ManagedInsn::Opaque {
                        effects: managed_ir::effects::EffectFlags::NONE,
                        ..
                    }
            ));
            assert_eq!(b.outgoing().len(), 1);
            block = ir.cfg().edge(b.outgoing()[0]).unwrap().target();
        }
        println!(
            "ORIGINAL ENTRY/Phi VERIFIED pool={pool} method={index} owner={expected_owner} name={expected_name} merged_chains={} ssa_blocks={} initial_inputs={inputs:?}",
            ssa.stats().merged_blocks,
            ssa.stats().ssa_blocks
        );
    }
}

#[test]
fn entry_loop_exception_edges_keep_handler_prethrow_input() {
    use dex_format::code::{EncodedCatchHandler, TryItem};
    struct IntInputs;
    impl TypeContext for IntInputs {
        fn initial_register_type(&self, r: u16, w: bool) -> Option<ManagedType> {
            (!w && r < 2).then_some(ManagedType::Int)
        }
        fn enclosing_return(&self) -> Option<ManagedType> {
            Some(ManagedType::Int)
        }
    }
    // div-int at original entry, normal quotient test, decrement dividend,
    // backedge to entry. The handler returns the current PRETHROW dividend.
    let mut item = code(
        &[
            0x0293, 0x0100, 0x0238, 5, 0x00d8, 0xff00, 0xfa28, 0x020f, 0x020d, 0x000f,
        ],
        3,
        2,
    );
    let span = RawSpan { offset: 0, size: 0 };
    item.tries.push(TryItem {
        start_addr: 0,
        insn_count: 2,
        handler_off: 0,
        span,
    });
    item.handlers.push(EncodedCatchHandler {
        relative_offset: 0,
        handlers: vec![],
        catch_all_addr: Some(8),
        span,
    });
    let lift = DexPasses::run_with(&item, &IntInputs).unwrap();
    let original_exception_edges = lift
        .ir()
        .cfg()
        .edge_ids()
        .filter(|e| {
            matches!(
                lift.ir().cfg().edge(*e).unwrap().kind(),
                decomp_core::cfg::EdgeKind::Exception(_)
            )
        })
        .count();
    let ssa = DexPasses::run_ssa_with(&item, &IntInputs).unwrap();
    let ir = ssa.ir();
    assert_eq!(
        ir.cfg()
            .edge_ids()
            .filter(|e| matches!(
                ir.cfg().edge(*e).unwrap().kind(),
                decomp_core::cfg::EdgeKind::Exception(_)
            ))
            .count(),
        original_exception_edges
    );
    let dividend = ir
        .instructions()
        .iter()
        .find_map(|(id, i)| match i {
            ManagedInsn::Arithmetic {
                op: managed_ir::ops::ArithOp::Div,
                lhs,
                ..
            } if ssa.offset(id) == Some(0) => Some(*lhs),
            _ => None,
        })
        .unwrap();
    let ManagedInsn::Phi { inputs, .. } =
        ir.instruction(ir.value(dividend).unwrap().def()).unwrap()
    else {
        panic!("missing original entry recurrence")
    };
    assert_eq!(inputs.len(), 2);
    assert!(
        inputs
            .iter()
            .any(|(_, v)| ssa.initial_registers().get(v) == Some(&vec![0]))
    );
    let caught_return = ir
        .instructions()
        .iter()
        .find_map(|(id, i)| match i {
            ManagedInsn::Return { value } if ssa.offset(id) == Some(9) => *value,
            _ => None,
        })
        .unwrap();
    assert_eq!(
        caught_return, dividend,
        "handler sees selected dividend before failed div, not quotient or only initial value"
    );
}
