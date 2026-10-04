//! Exact caught-value shapes do not hide unresolved enum-side effects.
#![allow(clippy::unwrap_used)]
use super::*;
use managed_ir::{
    effects::EffectFlags,
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    types::ManagedType,
    value::Value,
};
fn fixture(
    tag: u16,
    reads: bool,
    count: usize,
    effects: EffectFlags,
    wrong_definition: bool,
) -> MethodIr<SsaForm> {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let ret = insns.insert(ManagedInsn::Return { value: None });
    insns.insert_with_key(|id| {
        let value = values.insert(Value::new(
            if wrong_definition { ret } else { id },
            ManagedType::Unknown,
        ));
        ManagedInsn::Opaque {
            family_tag: tag,
            reads: if reads { vec![value] } else { vec![] },
            writes: vec![value; count],
            effects,
        }
    });
    MethodIr::<SsaForm>::from_parts(decomp_core::cfg::Cfg::new(ret), insns, values)
}
#[test]
fn enum_caught_binder_requires_exact_tag_shape_and_definition() {
    let dex = DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let class = &dex.classes[0];
    let plan = super::super::EnumInitializationPlan {
        header: String::new(),
        constants: vec![],
        constants_source: String::new(),
        encoded_declaration_initializers: BTreeMap::new(),
        absorbed_constructor: u32::MAX,
        absorbed_constructors: vec![],
        constructor_sources: BTreeMap::new(),
        absorbed_clinit: u32::MAX,
        absorbed_values_field: u32::MAX,
        absorbed_values_helper: None,
        static_initializer_source: None,
        static_initializer_ir: None,
        suffix_runtime_capabilities: 0,
    };
    assert!(
        validate_enum_member_ir(
            class,
            &plan,
            &fixture(0x8002, false, 1, EffectFlags::MAY_THROW, false)
        )
        .is_ok()
    );
    for (tag, reads, count, effects, wrong) in [
        (0x8002, true, 1, EffectFlags::MAY_THROW, false),
        (0x8002, false, 0, EffectFlags::MAY_THROW, false),
        (0x8002, false, 2, EffectFlags::MAY_THROW, false),
        (0x8002, false, 1, EffectFlags::NONE, false),
        (0x8002, false, 1, EffectFlags::MAY_THROW, true),
        (0x8888, false, 1, EffectFlags::MAY_THROW, false),
    ] {
        assert!(
            validate_enum_member_ir(class, &plan, &fixture(tag, reads, count, effects, wrong))
                .is_err()
        );
    }
}
