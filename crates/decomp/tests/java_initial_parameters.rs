//! Focused input-provenance tests, independently wired until facade integration.
// Fixture construction and successful-result assertions intentionally panic on failure.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "../src/java_initial_parameters.rs"]
mod java_initial_parameters;

use backend_java::{JavaMethod, JavaType};
use java_initial_parameters::{InitialParameter::*, InitialParameterError, parameter_bindings};

fn code(words: &[u16], registers: u16, incoming: u16) -> dex_format::code::CodeItem {
    dex_format::code::CodeItem {
        registers_size: registers,
        ins_size: incoming,
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
    }
}

fn signature(is_static: bool, parameters: Vec<JavaType>) -> JavaMethod {
    JavaMethod {
        owner: "fixture.Parameters".into(),
        name: "test".into(),
        is_static,
        return_type: JavaType::Void,
        parameters,
        throws: vec![],
    }
}

#[test]
fn unused_inputs_keep_signature_positions_and_names() {
    let ssa = dex_decompiler::DexPasses::run_merged(&code(&[0x000e], 5, 5)).unwrap();
    let plan = parameter_bindings(
        ssa.ir(),
        &signature(true, vec![JavaType::Long, JavaType::Int, JavaType::Double]),
        5,
        5,
    )
    .unwrap();
    assert_eq!(plan.receiver, None);
    assert_eq!(
        plan.parameters,
        vec![
            Unused { name: "p0".into() },
            Unused { name: "p1".into() },
            Unused { name: "p2".into() }
        ]
    );
}

#[test]
fn overwritten_input_is_unused_not_the_returned_constant() {
    let ssa = dex_decompiler::DexPasses::run_merged(&code(&[0x1012, 0x000f], 1, 1)).unwrap();
    let plan = parameter_bindings(ssa.ir(), &signature(true, vec![JavaType::Int]), 1, 1).unwrap();
    assert_eq!(plan.parameters, vec![Unused { name: "p0".into() }]);
    assert!(
        !ssa.ir()
            .register_slots()
            .values()
            .next()
            .unwrap()
            .values()
            .is_empty()
    );
}

#[test]
fn receiver_wide_and_narrow_bind_initials_in_signature_order() {
    let ssa =
        dex_decompiler::DexPasses::run_merged(&code(&[0x3007, 0x4104, 0x060f], 7, 4)).unwrap();
    let plan = parameter_bindings(
        ssa.ir(),
        &signature(false, vec![JavaType::Long, JavaType::Int]),
        7,
        4,
    )
    .unwrap();
    let history = |r| {
        ssa.ir()
            .register_slots()
            .values()
            .find(|s| s.index() == r)
            .unwrap()
            .values()
    };
    let Some(Bound(receiver)) = plan.receiver else {
        panic!("bound receiver")
    };
    let [Bound(wide), Bound(narrow)] = plan.parameters.as_slice() else {
        panic!("bound args")
    };
    assert!(history(3).contains(&receiver));
    assert!(history(4).contains(wide) && history(5).contains(wide));
    assert!(history(6).contains(narrow));
    assert_ne!(receiver, *wide);
    assert_ne!(wide, narrow);
}

#[test]
fn unused_receiver_does_not_shift_used_argument() {
    let ssa = dex_decompiler::DexPasses::run_merged(&code(&[0x010f], 2, 2)).unwrap();
    let plan = parameter_bindings(ssa.ir(), &signature(false, vec![JavaType::Int]), 2, 2).unwrap();
    assert_eq!(
        plan.receiver,
        Some(Unused {
            name: "this".into()
        })
    );
    assert!(matches!(plan.parameters.as_slice(), [Bound(_)]));
}

#[test]
fn initial_read_then_overwrite_keeps_original_identity() {
    let ssa =
        dex_decompiler::DexPasses::run_merged(&code(&[0x1001, 0x1112, 0x000f], 2, 1)).unwrap();
    let plan = parameter_bindings(ssa.ir(), &signature(true, vec![JavaType::Int]), 2, 1).unwrap();
    let Bound(value) = plan.parameters[0] else {
        panic!("bound initial")
    };
    let history = ssa
        .ir()
        .register_slots()
        .values()
        .find(|s| s.index() == 1)
        .unwrap()
        .values();
    assert_eq!(history[0], value);
    assert_ne!(*history.last().unwrap(), value);
}

#[test]
fn split_wide_and_narrow_aliases_are_ambiguities() {
    let split = dex_decompiler::DexPasses::run_merged(&code(&[0x000f], 2, 2)).unwrap();
    assert!(matches!(
        parameter_bindings(split.ir(), &signature(true, vec![JavaType::Long]), 2, 2),
        Err(InitialParameterError::InitialParameterAmbiguity(_))
    ));
    let wide = dex_decompiler::DexPasses::run_merged(&code(&[0x0010], 2, 2)).unwrap();
    assert!(matches!(
        parameter_bindings(
            wide.ir(),
            &signature(true, vec![JavaType::Int, JavaType::Int]),
            2,
            2
        ),
        Err(InitialParameterError::InitialParameterAmbiguity(_))
    ));
}

#[test]
fn frame_and_descriptor_widths_are_validated() {
    let ssa = dex_decompiler::DexPasses::run_merged(&code(&[0x000e], 0, 0)).unwrap();
    for (method, registers, incoming) in [
        (signature(true, vec![JavaType::Long]), 2, 1),
        (signature(false, vec![]), 0, 1),
        (signature(true, vec![JavaType::Void]), 1, 1),
    ] {
        assert!(matches!(
            parameter_bindings(ssa.ir(), &method, registers, incoming),
            Err(InitialParameterError::InvalidLayout(_))
        ));
    }
}

#[test]
fn overwritten_wide_input_and_receiver_are_unused() {
    // const-wide/16 v1, 1; const/4 v0, 0; return-wide v1
    let ssa =
        dex_decompiler::DexPasses::run_merged(&code(&[0x0116, 1, 0x0012, 0x0110], 3, 3)).unwrap();
    let plan = parameter_bindings(ssa.ir(), &signature(false, vec![JavaType::Long]), 3, 3).unwrap();
    assert_eq!(
        plan.receiver,
        Some(Unused {
            name: "this".into()
        })
    );
    assert_eq!(plan.parameters, vec![Unused { name: "p0".into() }]);
}

#[test]
fn missing_ssa_entry_provenance_is_not_declared_unused() {
    use managed_ir::{
        effects::EffectFlags,
        insn::ManagedInsn,
        method::{MethodIr, SsaForm},
        types::ManagedType,
        value::Value,
    };
    let ssa = dex_decompiler::DexPasses::run_merged(&code(&[0x000e], 1, 1)).unwrap();
    let ir = ssa.ir();
    let mut insns = ir.instructions().clone();
    let mut values = ir.values().clone();
    let def = insns.insert(ManagedInsn::Return { value: None });
    let value = values.insert(Value::new(def, ManagedType::Unknown));
    insns[def] = ManagedInsn::Opaque {
        family_tag: 0x8003,
        reads: vec![],
        writes: vec![value],
        effects: EffectFlags::NONE,
    };
    let ir = MethodIr::<SsaForm>::from_parts_with_registers(
        decomp_core::cfg::Cfg::new(def),
        insns,
        values,
        ir.register_slots().clone(),
    );
    assert!(matches!(
        parameter_bindings(&ir, &signature(true, vec![JavaType::Int]), 1, 1),
        Err(InitialParameterError::InitialParameterAmbiguity(_))
    ));
}

#[test]
#[ignore = "local OKcashbag corpus probe; requires unzip and device APK fixture"]
fn okcashbag_smallest_input_methods_probe() {
    use backend_java::JavaResolver;
    use managed_ir::refs::MethodRef;
    let apk = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk"
    );
    let output = std::process::Command::new("unzip")
        .args(["-p", apk, "classes.dex"])
        .output()
        .expect("unzip available");
    assert!(output.status.success());
    let dex = dex_format::DexFile::from_bytes(&output.stdout).unwrap();
    let resolver = decomp::dex_java::DexJavaResolver { dex: &dex };
    let mut methods: Vec<_> = dex
        .classes
        .iter()
        .filter_map(|c| c.class_data.as_ref())
        .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
        .filter(|m| {
            m.code
                .as_ref()
                .is_some_and(|c| c.ins_size > 0 && c.insns_size <= 16)
        })
        .collect();
    methods.sort_by_key(|m| (m.code.as_ref().unwrap().insns_size, m.method_idx));
    let mut accepted = 0;
    let mut unused = 0;
    let mut ambiguous = 0;
    let mut frontend_failures = 0;
    let mut inspected = 0;
    for method in methods.into_iter().take(8) {
        let signature = resolver.method(MethodRef::new(method.method_idx)).unwrap();
        let code = method.code.as_ref().unwrap();
        inspected += 1;
        let ssa = match dex_decompiler::DexPasses::run_ssa_with(
            code,
            &dex_decompiler::DexTypes::new(&dex, Some(method.method_idx)),
        ) {
            Ok(ssa) => ssa,
            Err(_) => {
                frontend_failures += 1;
                continue;
            }
        };
        match parameter_bindings(ssa.ir(), &signature, code.registers_size, code.ins_size) {
            Ok(plan) => {
                accepted += 1;
                assert_eq!(plan.parameters.len(), signature.parameters.len());
                assert_eq!(plan.receiver.is_none(), signature.is_static);
                unused += plan
                    .parameters
                    .iter()
                    .filter(|p| matches!(p, Unused { .. }))
                    .count();
                eprintln!(
                    "method {} {}.{}: {:?}",
                    method.method_idx, signature.owner, signature.name, plan
                );
            }
            Err(InitialParameterError::InitialParameterAmbiguity(reason)) => {
                ambiguous += 1;
                eprintln!("method {}: ambiguity: {reason}", method.method_idx);
            }
            Err(e) => panic!("real signature layout rejected: {e}"),
        }
    }
    assert!(inspected > 0 && accepted > 0);
    eprintln!(
        "OKcashbag: inspected={inspected}, accepted={accepted}, unused_arguments={unused}, ambiguous={ambiguous}, frontend_failures={frontend_failures}"
    );
}
