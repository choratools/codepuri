//! Indexed provenance under many inputs and long write histories.
#![allow(clippy::unwrap_used)]
#[path = "../src/java_initial_parameters.rs"]
mod parameters;
use backend_java::{JavaMethod, JavaType};
use parameters::{InitialParameter, parameter_bindings};

#[test]
fn many_initial_inputs_with_long_write_histories_keep_exact_bindings() {
    let mut words = Vec::new();
    // Read every incoming register, then overwrite it repeatedly. Binding must
    // keep the synthetic initial, rather than any of these later writes.
    for r in 1..101u16 {
        words.extend([0x0002, r]); // move/from16 v0, vR
    }
    for _ in 0..16 {
        for r in 1..101u16 {
            words.extend([0x0013 | (r << 8), 7]); // const/16 vR, 7
        }
    }
    words.push(0x000e);
    let code = dex_format::code::CodeItem {
        registers_size: 101,
        ins_size: 100,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: words.len() as u32,
        insns: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: dex_format::RawSpan {
            offset: 0,
            size: (words.len() * 2) as u32,
        },
    };
    let ssa = dex_decompiler::DexPasses::run_merged(&code).unwrap();
    let method = JavaMethod {
        owner: "x".into(),
        name: "many".into(),
        is_static: true,
        parameters: vec![JavaType::Int; 100],
        return_type: JavaType::Void,
        throws: vec![],
    };
    let plan = parameter_bindings(ssa.ir(), &method, 101, 100).unwrap();
    for (r, bound) in plan.parameters.iter().enumerate() {
        let InitialParameter::Bound(value) = bound else {
            panic!("missing initial {r}");
        };
        assert!(
            ssa.ir()
                .register_slots()
                .values()
                .find(|s| usize::from(s.index()) == r + 1)
                .unwrap()
                .values()
                .contains(value)
        );
    }
}
