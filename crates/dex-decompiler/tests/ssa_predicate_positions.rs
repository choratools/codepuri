//! Synthetic handler inputs must not move branch predicates ahead of their
//! original register producers when both are inserted in one placement pass.
use dex_decompiler::{DexPasses, SsaMethod};
use dex_format::{
    RawSpan,
    code::{CodeItem, EncodedCatchHandler, TryItem},
};
use managed_ir::{insn::ManagedInsn, value::ValueId};

fn fixture(reassigned: bool) -> CodeItem {
    // try { v3 = 1 / 1; return; } catch (...) {
    //   v3 = 7; v3:v4 = 10L; v5:v6 = 20L;
    //   v3 = cmp-long(v3:v4, v5:v6); [v3 = 2;] if-ltz v3 ...
    // }
    let mut units: Vec<u16> = vec![
        0x1312, 0x0393, 0x0303, 0x000e, 0x000d, 0x7312, 0x0316, 10, 0x0516, 20, 0x0331, 0x0503,
    ];
    if reassigned {
        units.push(0x2312);
    }
    units.extend([0x033a, 3, 0x000e, 0x000e]);
    CodeItem {
        registers_size: 7,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: u32::try_from(units.len()).unwrap_or_else(|_| panic!("fixture size")),
        insns: units.iter().flat_map(|unit| unit.to_le_bytes()).collect(),
        tries: vec![TryItem {
            start_addr: 1,
            insn_count: 2,
            handler_off: 0,
            span: RawSpan { offset: 0, size: 0 },
        }],
        handlers: vec![EncodedCatchHandler {
            relative_offset: 0,
            handlers: vec![],
            catch_all_addr: Some(4),
            span: RawSpan { offset: 0, size: 0 },
        }],
        span: RawSpan { offset: 0, size: 0 },
    }
}

fn comparison_input(ssa: &SsaMethod) -> ValueId {
    ssa.ir()
        .instructions()
        .iter()
        .find_map(|(_, insn)| match insn {
            ManagedInsn::Compare { lhs, .. } => Some(*lhs),
            _ => None,
        })
        .unwrap_or_else(|| panic!("branch comparison absent"))
}

#[test]
fn handler_input_insertion_preserves_cmp_long_before_branch_predicate() {
    let ssa = DexPasses::run_merged(&fixture(false))
        .unwrap_or_else(|error| panic!("fixture rejected: {error:?}"));
    let (cmp_id, cmp_value) = ssa
        .ir()
        .instructions()
        .iter()
        .find_map(|(id, insn)| match insn {
            ManagedInsn::OrderingCompare { result, .. } => Some((id, *result)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("cmp-long absent"));
    assert_eq!(ssa.offset(cmp_id), Some(10));
    assert_eq!(
        comparison_input(&ssa),
        cmp_value,
        "if-ltz must read the cmp result, never the old category-one v3=7"
    );
    let ir = ssa.ir();
    let cmp_block = ir
        .cfg()
        .block_ids()
        .find(|b| ir.cfg().block(*b).is_ok_and(|b| *b.payload() == cmp_id))
        .unwrap_or_else(|| panic!("cmp block absent"));
    let compare_id = ir
        .value(comparison_input(&ssa))
        .unwrap_or_else(|error| panic!("missing value: {error:?}"))
        .def();
    assert_eq!(compare_id, cmp_id);
    assert!(
        !ir.cfg()
            .block(cmp_block)
            .unwrap_or_else(|error| panic!("missing block: {error:?}"))
            .outgoing()
            .is_empty()
    );
}

#[test]
fn branch_redefinition_after_cmp_is_retained() {
    let ssa = DexPasses::run_merged(&fixture(true))
        .unwrap_or_else(|error| panic!("fixture rejected: {error:?}"));
    let value = comparison_input(&ssa);
    let id = ssa
        .ir()
        .value(value)
        .unwrap_or_else(|error| panic!("missing value: {error:?}"))
        .def();
    assert_eq!(
        ssa.offset(id),
        Some(12),
        "the later const/4 wins, not cmp-long"
    );
    assert!(matches!(
        ssa.ir().instruction(id),
        Ok(ManagedInsn::Const {
            value: managed_ir::ops::Constant::Int(2),
            ..
        })
    ));
}
