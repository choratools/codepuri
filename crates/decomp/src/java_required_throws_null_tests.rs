#![allow(clippy::unwrap_used)]
use super::*;
use dex_decompiler::{DexPasses, TypeContext};
use managed_ir::refs::{FieldRef, StringRef};

struct Context;
fn identity() -> OriginalMethodIdentity {
    OriginalMethodIdentity {
        dex: 0,
        index: 0,
        key: MethodKey {
            owner: "LThrowFixture;".into(),
            name: "throwIt".into(),
            parameters: vec![],
            return_type: "V".into(),
        },
    }
}
impl JavaResolver for Context {
    fn method(&self, _: MethodRef) -> Option<backend_java::JavaMethod> {
        None
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(8)
    }
    fn field(&self, _: FieldRef) -> Option<backend_java::JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("java.io.IOException".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
impl ThrowsHierarchy for Context {
    fn exception_kind(&self, name: &str) -> Option<ExceptionKind> {
        match name {
            "java.lang.NullPointerException" => Some(ExceptionKind::Unchecked),
            "java.io.IOException" => Some(ExceptionKind::Checked),
            _ => None,
        }
    }
}
impl RequiredThrowsContext for Context {
    fn invocation_contract(&self, _: MethodRef) -> Option<InvocationContract> {
        Some(InvocationContract {
            source_target: Some(identity().key),
            declared: vec![],
        })
    }
    fn override_contract(&self, _: &OriginalMethodIdentity) -> OverrideContract {
        OverrideContract::NoInheritedDeclaration
    }
}
struct Seed(ManagedType);
impl TypeContext for Seed {
    fn initial_register_type(&self, register: u16, wide: bool) -> Option<ManagedType> {
        (register == 0 && !wide).then(|| self.0.clone())
    }
}
fn code(units: &[u16], incoming: u16) -> CodeItem {
    CodeItem {
        registers_size: 1,
        ins_size: incoming,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|n| n.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: dex_format::RawSpan {
            offset: 0,
            size: units.len() as u32 * 2,
        },
    }
}
#[test]
fn literal_zero_is_an_unchecked_implicit_npe_in_source_contracts() {
    let code = code(&[0x0012, 0x0027], 0);
    let ssa = DexPasses::run_ssa_with(&code, &Seed(ManagedType::Unknown)).unwrap();
    let requirements = inspect_method(identity(), &ssa, &code, 8, vec![], &Context).unwrap();
    let plan = derive_program(&[requirements], &Context).unwrap();
    assert!(plan.unavailable.is_empty());
    assert!(plan.methods[&identity().key].derived.is_empty());
}
#[test]
fn nominal_zero_entry_and_checked_reference_do_not_become_null_literals() {
    let code = code(&[0x0027], 1);
    let zero = DexPasses::run_ssa_with(&code, &Seed(ManagedType::Zero)).unwrap();
    assert!(matches!(
        inspect_method(identity(), &zero, &code, 8, vec![], &Context),
        Err(ThrowsError::Unknown(_))
    ));
    let checked =
        DexPasses::run_ssa_with(&code, &Seed(ManagedType::Reference(ClassRef::new(0)))).unwrap();
    let requirements = inspect_method(identity(), &checked, &code, 8, vec![], &Context).unwrap();
    let plan = derive_program(&[requirements], &Context).unwrap();
    assert_eq!(
        plan.methods[&identity().key].derived,
        vec!["java.io.IOException"]
    );
}
#[test]
fn original_samsung_component_and_media_controller_null_offsets_are_proven() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
    if !path.exists() {
        return;
    }
    let files = dex_format::apk::parse_apk_path(&path).unwrap();
    let dex = &files[0];
    for (index, offset) in [(10702, 6), (2913, 45)] {
        let (class, method) = dex
            .classes
            .iter()
            .find_map(|class| {
                class.class_data.as_ref().and_then(|data| {
                    data.direct_methods
                        .iter()
                        .chain(&data.virtual_methods)
                        .find(|m| m.method_idx == index)
                        .map(|m| (class, m))
                })
            })
            .unwrap();
        assert_eq!(
            crate::java_members::MemberResolver { dex }.declared_throws_for_class(class, index),
            Some(vec![])
        );
        let code = method.code.as_ref().unwrap();
        let types = dex_decompiler::DexTypes::new(dex, Some(index));
        let ssa = DexPasses::run_ssa_with(code, &types).unwrap();
        let value = ssa
            .ir()
            .instructions()
            .iter()
            .find_map(|(id, insn)| match insn {
                ManagedInsn::Throw { value } if ssa.offset(id) == Some(offset) => Some(*value),
                _ => None,
            })
            .unwrap();
        assert!(
            backend_java::proven_null_throw(ssa.ir(), value),
            "original method {index} throw@{offset}"
        );
    }
}
