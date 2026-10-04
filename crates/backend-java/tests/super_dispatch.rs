//! Exact superclass dispatch must retain the incoming receiver and opcode.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, SourceResult,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::InvokeKind,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};
struct Resolver {
    proof: bool,
    flags: u32,
}
impl JavaResolver for Resolver {
    fn java_super_selects_dex_target(&self, caller: &str, _: MethodRef) -> bool {
        self.proof && caller == "SuperDispatch"
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(self.flags)
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || matches!((actual, expected), (JavaType::Class(a), JavaType::Class(b)) if a == "SuperDispatch" && b == "java.util.ArrayList")
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "java.util.ArrayList".into(),
            name: "size".into(),
            parameters: vec![],
            return_type: JavaType::Int,
            is_static: false,
            throws: vec![],
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("SuperDispatch".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
fn emit(kind: InvokeKind, proof: bool, this: bool, flags: u32) -> SourceResult {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut receiver = None;
    insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
        receiver = Some(v);
        ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        }
    });
    let receiver = receiver.unwrap();
    let mut result = None;
    let call = insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Int));
        result = Some(v);
        ManagedInsn::Invoke {
            kind,
            method: MethodRef::new(0),
            args: vec![receiver],
            result: Some(v),
        }
    });
    let ret = insns.insert(ManagedInsn::Return { value: result });
    let mut cfg = Cfg::new(call);
    let tail = cfg.add_block(ret);
    cfg.add_edge(cfg.entry(), tail, EdgeKind::Unconditional)
        .unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    backend_java::emit_method(
        &ir,
        &JavaMethod {
            owner: "SuperDispatch".into(),
            name: "originalSize".into(),
            parameters: if this {
                vec![]
            } else {
                vec![JavaType::Class("SuperDispatch".into())]
            },
            return_type: JavaType::Int,
            is_static: !this,
            throws: vec![],
        },
        &ParameterBindings {
            receiver: if this { Some(receiver) } else { None },
            parameters: if this { vec![] } else { vec![receiver] },
            names: vec![],
        },
        &Resolver { proof, flags },
    )
}
#[test]
fn exact_super_dispatch_compiles_and_bypasses_override() {
    let SourceResult::Complete { source, .. } = emit(InvokeKind::Super, true, true, 1) else {
        panic!("super unsupported");
    };
    assert!(source.contains("super.size()"), "{source}");
    let dir =
        std::env::temp_dir().join(format!("decomphify-super-dispatch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("SuperDispatch.java");
    std::fs::write(&file,format!("public class SuperDispatch extends java.util.ArrayList<String> {{ {source} @Override public int size() {{ return 99; }} public static void main(String[] a) {{ SuperDispatch x=new SuperDispatch(); if(x.originalSize()!=0 || x.size()!=99) throw new AssertionError(); x.add(\"a\"); if(x.originalSize()!=1) throw new AssertionError(); }} }}")).unwrap();
    let c = std::process::Command::new("javac")
        .arg("--release")
        .arg("17")
        .arg(&file)
        .output()
        .unwrap();
    assert!(c.status.success(), "{}", String::from_utf8_lossy(&c.stderr));
    let r = std::process::Command::new("java")
        .arg("-cp")
        .arg(&dir)
        .arg("SuperDispatch")
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
}
#[test]
fn provenance_receiver_and_declaration_conflicts_reject() {
    for (kind, proof, this, flags) in [
        (InvokeKind::Special, true, true, 1),
        (InvokeKind::Super, false, true, 1),
        (InvokeKind::Super, true, false, 1),
        (InvokeKind::Super, true, true, 0x401),
        (InvokeKind::Super, true, true, 0x81),
        (InvokeKind::Super, true, true, 0x41),
        (InvokeKind::Super, true, true, 2),
    ] {
        assert!(matches!(
            emit(kind, proof, this, flags),
            SourceResult::Unsupported(_)
        ));
    }
}
