//! Conditional physical writes must merge with an incoming value even if the
//! lifter first encounters that register after the file-order write.
#![allow(clippy::unwrap_used)]
use dex_decompiler::{DexPasses, TypeContext};
use dex_format::{RawSpan, code::CodeItem};
use managed_ir::{
    insn::{Constant, ManagedInsn},
    types::ManagedType,
};
struct Incoming;
impl TypeContext for Incoming {
    fn initial_register_type(&self, register: u16, wide: bool) -> Option<ManagedType> {
        match (register, wide) {
            (0, false) | (1, false) => Some(ManagedType::Int),
            (0, true) => Some(ManagedType::Long),
            _ => None,
        }
    }
}
fn code(units: &[u16], registers: u16, incoming: u16) -> CodeItem {
    CodeItem {
        registers_size: registers,
        ins_size: incoming,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|v| v.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: RawSpan { offset: 0, size: 0 },
    }
}
fn phi_inputs(ssa: &dex_decompiler::SsaMethod) -> Vec<managed_ir::value::ValueId> {
    ssa.ir()
        .instructions()
        .values()
        .find_map(|i| match i {
            ManagedInsn::Phi { inputs, .. } => Some(inputs.iter().map(|(_, v)| *v).collect()),
            _ => None,
        })
        .unwrap()
}
#[test]
fn conditional_single_write_preserves_both_incoming_and_default() {
    // if-eqz v1,+3; const/4 v0,7; return v0.
    let ssa =
        DexPasses::run_ssa_with(&code(&[0x0138, 3, 0x7012, 0x000f], 2, 2), &Incoming).unwrap();
    let inputs = phi_inputs(&ssa);
    assert_eq!(inputs.len(), 2);
    assert_ne!(inputs[0], inputs[1]);
    assert!(inputs.iter().any(|v| matches!(
        ssa.ir().instructions()[ssa.ir().values()[*v].def()],
        ManagedInsn::Const {
            value: Constant::Int(7),
            ..
        }
    )));
    let initial = *inputs
        .iter()
        .find(|v| {
            matches!(
                ssa.ir().instructions()[ssa.ir().values()[**v].def()],
                ManagedInsn::Opaque {
                    family_tag: 0x8003,
                    ..
                }
            )
        })
        .unwrap();
    assert_eq!(ssa.initial_registers()[&initial], vec![0]);
    assert_eq!(ssa.ir().values()[initial].ty(), &ManagedType::Int);
    let returned = ssa
        .ir()
        .instructions()
        .values()
        .find_map(|i| match i {
            ManagedInsn::Return { value } => *value,
            _ => None,
        })
        .unwrap();
    assert!(matches!(
        ssa.ir().instructions()[ssa.ir().values()[returned].def()],
        ManagedInsn::Phi { .. }
    ));
}
#[test]
fn first_write_in_loop_merges_the_entry_path() {
    // Header first reads v0 after file-order loop-body assignment; backedge
    // re-enters the write test. Entry bypass must remain the original input.
    let ssa = DexPasses::run_ssa_with(
        &code(&[0x0312, 0x0138, 4, 0x7012, 0xfd28, 0x000f], 4, 4),
        &Incoming,
    )
    .unwrap();
    let inputs = phi_inputs(&ssa);
    assert_eq!(inputs.len(), 2);
    assert!(
        inputs
            .iter()
            .any(|v| ssa.initial_registers().get(v) == Some(&vec![0]))
    );
    assert!(inputs.iter().any(|v| matches!(
        ssa.ir().instructions()[ssa.ir().values()[*v].def()],
        ManagedInsn::Const {
            value: Constant::Int(7),
            ..
        }
    )));
}
#[test]
fn straight_line_single_write_needs_no_phi_or_incoming_value() {
    let ssa = DexPasses::run_ssa_with(&code(&[0x7012, 0x000f], 1, 1), &Incoming).unwrap();
    assert!(
        !ssa.ir()
            .instructions()
            .values()
            .any(|i| matches!(i, ManagedInsn::Phi { .. }))
    );
    assert!(ssa.initial_registers().is_empty());
}
#[test]
fn absent_input_evidence_retains_unresolved_entry_not_a_fabricated_constant() {
    let ssa = DexPasses::run_ssa_with(
        &code(&[0x0138, 3, 0x7012, 0x000f], 2, 0),
        &dex_decompiler::infer::NoTypes,
    )
    .unwrap();
    let inputs = phi_inputs(&ssa);
    assert_eq!(inputs.len(), 2);
    let unresolved=inputs.iter().find(|v|matches!(ssa.ir().instructions()[ssa.ir().values()[**v].def()],ManagedInsn::Opaque{family_tag:0x8003,ref reads,ref effects,..} if reads.is_empty()&&*effects==managed_ir::effects::EffectFlags::NONE)).unwrap();
    assert_eq!(ssa.initial_registers()[unresolved], vec![0]);
    // This evidence records a physical entry read, not a valid incoming
    // parameter: ins_size is zero. Publication must still check its signature.
    assert_eq!(code(&[], 2, 0).ins_size, 0);
}
#[test]
fn wide_single_write_merges_one_atomic_input_pair() {
    // if-eqz v2,+4; const-wide/16 v0,7; return-wide v0.
    let ssa =
        DexPasses::run_ssa_with(&code(&[0x0238, 4, 0x0016, 7, 0x0010], 3, 3), &Incoming).unwrap();
    let inputs = phi_inputs(&ssa);
    assert_eq!(inputs.len(), 2);
    let initial = inputs
        .iter()
        .find(|v| ssa.initial_registers().get(v) == Some(&vec![0, 1]))
        .unwrap();
    assert_eq!(ssa.ir().values()[*initial].ty(), &ManagedType::Long);
    assert!(
        ssa.ir()
            .register_slots()
            .values()
            .filter(|s| s.values().contains(initial))
            .map(|s| s.index())
            .eq([0, 1])
    );
}

#[test]
#[ignore = "requires retained Samsung Pass original APK"]
fn samsung_default_constructor_retains_incoming_and_allocated_argument() {
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let (dex, class, method) = inputs
        .iter()
        .find_map(|dex| {
            dex.classes.iter().find_map(|class| {
                let raw = dex.type_id(class.class_idx).ok()?;
                if dex.string(raw.descriptor_idx).ok()? != "Li1/d0;" {
                    return None;
                }
                class
                    .class_data
                    .as_ref()?
                    .direct_methods
                    .iter()
                    .find(|m| m.method_idx == 43967)
                    .map(|method| (dex, class, method))
            })
        })
        .unwrap();
    let code = method.code.as_ref().unwrap();
    assert_eq!((code.registers_size, code.ins_size), (4, 4));
    assert!(code.tries.is_empty());
    let ctx = dex_decompiler::DexTypes::new(dex, Some(method.method_idx));
    let mut types = Vec::new();
    assert!(ctx.method_params(method.method_idx, &mut types));
    assert_eq!(types.len(), 3);
    let mut seeds =
        std::collections::HashMap::from([((0, false), ctx.type_at(class.class_idx).unwrap())]);
    for (i, ty) in types.into_iter().enumerate() {
        assert_eq!(ty.category(), Some(managed_ir::types::ValueCategory::One));
        seeds.insert((i as u16 + 1, false), ty);
    }
    let ssa = DexPasses::run_ssa_with(code, &ctx.with_initial_registers(seeds)).unwrap();
    let argument = ssa
        .ir()
        .instructions()
        .values()
        .find_map(|i| match i {
            ManagedInsn::Invoke { method, args, .. } if method.index() == 43966 => Some(args[1]),
            _ => None,
        })
        .unwrap();
    let ManagedInsn::Phi { inputs, .. } =
        &ssa.ir().instructions()[ssa.ir().values()[argument].def()]
    else {
        panic!("this argument must retain an edge-selected Phi");
    };
    assert_eq!(inputs.len(), 2);
    assert!(
        inputs
            .iter()
            .any(|(_, v)| ssa.initial_registers().get(v) == Some(&vec![1]))
    );
    assert!(inputs.iter().any(|(_, v)| matches!(
        ssa.ir().instructions()[ssa.ir().values()[*v].def()],
        ManagedInsn::NewInstance { .. }
    )));
    assert!(ssa.diagnostics().is_empty(), "{:?}", ssa.diagnostics());
}
