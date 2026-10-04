#![allow(clippy::unwrap_used)]
use super::*;
use backend_java::{JavaField, JavaMethod};
use dex_decompiler::{DexPasses, TypeContext};
use managed_ir::refs::{FieldRef, StringRef};
struct Context {
    incoming: ManagedType,
    attest: bool,
}
fn identity() -> OriginalMethodIdentity {
    OriginalMethodIdentity {
        dex: 0,
        index: 10,
        key: MethodKey {
            owner: "LCloneFixture;".into(),
            name: "copy".into(),
            parameters: vec!["[I".into()],
            return_type: "V".into(),
        },
    }
}
impl JavaResolver for Context {
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "java.lang.Object".into(),
            name: "clone".into(),
            is_static: false,
            return_type: JavaType::Class("java.lang.Object".into()),
            parameters: vec![],
            throws: vec!["java.lang.CloneNotSupportedException".into()],
        })
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        Some(if r.index() == 10 { 8 } else { 0x104 })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("java.lang.Object".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
impl ThrowsHierarchy for Context {
    fn exception_kind(&self, name: &str) -> Option<ExceptionKind> {
        (name == "java.lang.CloneNotSupportedException").then_some(ExceptionKind::Checked)
    }
}
impl RequiredThrowsContext for Context {
    fn invocation_contract(&self, reference: MethodRef) -> Option<InvocationContract> {
        Some(if reference.index() == 10 {
            InvocationContract {
                source_target: Some(identity().key),
                declared: vec![],
            }
        } else {
            InvocationContract {
                source_target: None,
                declared: vec!["java.lang.CloneNotSupportedException".into()],
            }
        })
    }
    fn array_clone_contract(&self, _: MethodRef, _: &ManagedType) -> Option<InvocationContract> {
        self.attest.then_some(InvocationContract {
            source_target: None,
            declared: vec![],
        })
    }
    fn override_contract(&self, _: &OriginalMethodIdentity) -> OverrideContract {
        OverrideContract::NoInheritedDeclaration
    }
}
impl TypeContext for Context {
    fn initial_register_type(&self, r: u16, wide: bool) -> Option<ManagedType> {
        (r == 1 && !wide).then(|| self.incoming.clone())
    }
    fn method_params(&self, _: u32, _: &mut Vec<ManagedType>) -> bool {
        true
    }
    fn method_return(&self, _: u32) -> Option<ManagedType> {
        Some(ManagedType::Reference(ClassRef::new(0)))
    }
}
#[test]
fn only_virtual_array_with_attested_binding_uses_array_clone_contract() {
    let array = ManagedType::Array {
        element: Box::new(ManagedType::Int),
        nullable: true,
    };
    for (incoming, attest, opcode, expected) in [
        (array.clone(), true, 0x106eu16, false),
        (ManagedType::Reference(ClassRef::new(0)), true, 0x106e, true),
        (ManagedType::Unknown, true, 0x106e, true),
        (array.clone(), true, 0x1070, true),
        (array, false, 0x106e, true),
    ] {
        let context = Context { incoming, attest };
        let units = [opcode, 0, 1, 0x000e];
        let code = CodeItem {
            registers_size: 2,
            ins_size: 1,
            outs_size: 1,
            debug_info_off: 0,
            insns_size: 4,
            insns: units.into_iter().flat_map(u16::to_le_bytes).collect(),
            tries: vec![],
            handlers: vec![],
            span: dex_format::RawSpan { offset: 0, size: 8 },
        };
        let ssa = DexPasses::run_ssa_with(&code, &context).unwrap();
        let body = inspect_method(identity(), &ssa, &code, 8, vec![], &context).unwrap();
        let plan = derive_program(&[body], &context).unwrap();
        assert!(plan.unavailable.is_empty());
        assert_eq!(
            plan.methods[&identity().key].source,
            if expected {
                vec!["java.lang.CloneNotSupportedException"]
            } else {
                vec![]
            }
        );
    }
}
