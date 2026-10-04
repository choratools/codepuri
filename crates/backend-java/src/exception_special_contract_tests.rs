//! Exact Special contracts never become symbolic Super/virtual dispatch.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{JavaField, JavaMethod};
use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
struct Resolver {
    flags: Option<u32>,
    method: JavaMethod,
}
impl JavaResolver for Resolver {
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Owner".into())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(self.method.clone())
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        self.flags
    }
}
fn resolver(name: &str, flags: u32) -> Resolver {
    Resolver {
        flags: Some(flags),
        method: JavaMethod {
            owner: "Owner".into(),
            name: name.into(),
            is_static: false,
            parameters: vec![JavaType::Int],
            return_type: JavaType::Void,
            throws: vec!["java.io.IOException".into()],
        },
    }
}
fn invoke(args: usize) -> ManagedInsn {
    ManagedInsn::Invoke {
        kind: managed_ir::ops::InvokeKind::Special,
        method: MethodRef::new(0),
        args: vec![ValueId::default(); args],
        result: None,
    }
}
#[test]
fn exact_constructor_contract_requires_genuine_nonstatic_flags_and_arity() {
    let mut r = resolver("<init>", 1);
    let i = invoke(2);
    assert_eq!(
        contract(&i, "Caller", &r),
        Some(vec!["java.io.IOException".into()])
    );
    assert_eq!(contract(&invoke(1), "Caller", &r), None);
    r.flags = None;
    assert_eq!(contract(&i, "Caller", &r), None);
    r.flags = Some(8);
    assert_eq!(contract(&i, "Caller", &r), None);
    r.flags = Some(0x401);
    assert_eq!(contract(&i, "Caller", &r), None);
    r.flags = Some(0x101);
    assert_eq!(contract(&i, "Caller", &r), None);
    r.flags = Some(2);
    assert_eq!(contract(&i, "Caller", &r), None);
    assert_eq!(contract(&i, "Owner", &r), Some(r.method.throws.clone()));
    r.method.return_type = JavaType::Int;
    assert_eq!(contract(&i, "Owner", &r), None);
}
#[test]
fn direct_private_contract_preserves_exact_owner_and_checked_throws() {
    let mut r = resolver("callback", 2);
    let i = invoke(2);
    assert_eq!(contract(&i, "Owner", &r), Some(r.method.throws.clone()));
    assert_eq!(contract(&i, "Other", &r), None);
    r.flags = Some(1);
    assert_eq!(contract(&i, "Owner", &r), None);
    r.flags = Some(2);
    r.method.is_static = true;
    assert_eq!(contract(&i, "Owner", &r), None);
    r.method.is_static = false;
    r.method.name = "<clinit>".into();
    assert_eq!(contract(&i, "Owner", &r), None);
}
#[test]
fn special_constructor_result_and_void_parameter_cannot_supply_source_contract() {
    let mut r = resolver("<init>", 1);
    let mut i = invoke(2);
    if let ManagedInsn::Invoke { result, .. } = &mut i {
        *result = Some(ValueId::default());
    }
    assert_eq!(contract(&i, "Owner", &r), None);
    r.method.parameters = vec![JavaType::Void];
    assert_eq!(contract(&invoke(2), "Owner", &r), None);
}

#[test]
fn emitted_constructor_contract_requires_exact_cached_allocation_initializer() {
    use decomp_core::cfg::{Cfg, EdgeKind};
    use managed_ir::{
        method::{InsnArena, ValueArena},
        types::ManagedType,
        value::Value,
    };
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let allocation = insns.insert(ManagedInsn::Return { value: None });
    let object = values.insert(Value::new(
        allocation,
        ManagedType::Reference(ClassRef::new(0)),
    ));
    insns[allocation] = ManagedInsn::NewInstance {
        result: object,
        class: ClassRef::new(0),
    };
    let constructor = insns.insert(ManagedInsn::Invoke {
        kind: managed_ir::ops::InvokeKind::Special,
        method: MethodRef::new(0),
        args: vec![object],
        result: None,
    });
    let terminal = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(allocation);
    let init = cfg.add_block(constructor);
    let exit = cfg.add_block(terminal);
    cfg.add_edge(cfg.entry(), init, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(init, exit, EdgeKind::Unconditional).unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    let mut r = resolver("<init>", 1);
    r.method.parameters.clear();
    let emitter = crate::Emitter {
        allocation_readable: None,
        object_planning: Default::default(),
        ir: &ir,
        resolver: &r,
        values: HashMap::new(),
        lines: vec![],
        next: 0,
        parameters: HashSet::new(),
        reserved: HashSet::new(),
        current: None,
        hoisted: HashSet::new(),
        declarations: HashMap::new(),
    };
    let original = ir.instruction(constructor).unwrap();
    assert_eq!(
        emitted_contract(&emitter, "Caller", constructor, original),
        Some(r.method.throws.clone())
    );
    assert_eq!(
        emitted_contract(&emitter, "Caller", terminal, original),
        None
    );
    let unpaired = ManagedInsn::Invoke {
        kind: managed_ir::ops::InvokeKind::Special,
        method: MethodRef::new(0),
        args: vec![ValueId::default()],
        result: None,
    };
    assert_eq!(
        emitted_contract(&emitter, "Caller", constructor, &unpaired),
        None
    );
    let mut wrong = resolver("<init>", 1);
    wrong.method.parameters.clear();
    wrong.method.owner = "Other".into();
    let wrong_emitter = crate::Emitter {
        allocation_readable: None,
        object_planning: Default::default(),
        ir: &ir,
        resolver: &wrong,
        values: HashMap::new(),
        lines: vec![],
        next: 0,
        parameters: HashSet::new(),
        reserved: HashSet::new(),
        current: None,
        hoisted: HashSet::new(),
        declarations: HashMap::new(),
    };
    assert_eq!(
        emitted_contract(&wrong_emitter, "Caller", constructor, original),
        None
    );
}
