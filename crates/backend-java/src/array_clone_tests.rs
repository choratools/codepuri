//! JVM array clone intrinsic emission.
#![allow(clippy::unwrap_used)]

use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    value::Value,
};
use std::{fs, process::Command};

#[derive(Default)]
struct Builder {
    insns: InsnArena,
    values: ValueArena,
    order: Vec<InsnId>,
}
impl Builder {
    fn value(&mut self, ty: ManagedType, f: impl FnOnce(ValueId) -> ManagedInsn) -> ValueId {
        let mut value = None;
        let id = self.insns.insert_with_key(|id| {
            let v = self.values.insert(Value::new(id, ty));
            value = Some(v);
            f(v)
        });
        self.order.push(id);
        value.unwrap()
    }
    fn parameter(&mut self, ty: ManagedType) -> ValueId {
        let v = self.value(ty, |v| ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        });
        self.order.pop();
        v
    }
    fn push(&mut self, insn: ManagedInsn) {
        self.order.push(self.insns.insert(insn));
    }
    fn finish(self) -> MethodIr<SsaForm> {
        let mut cfg = Cfg::new(self.order[0]);
        let mut last = cfg.entry();
        for id in &self.order[1..] {
            let next = cfg.add_block(*id);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}

#[derive(Clone, Copy)]
struct Resolver {
    intrinsic: bool,
    expected_ref: u32,
}
impl JavaResolver for Resolver {
    fn array_clone_method(&self, reference: MethodRef, receiver: &JavaType) -> Option<JavaMethod> {
        (self.intrinsic
            && reference.index() == 7
            && *receiver == JavaType::Array(Box::new(JavaType::Byte)))
        .then(|| JavaMethod {
            owner: "java.lang.Object".into(),
            name: "clone".into(),
            is_static: false,
            return_type: JavaType::Class("java.lang.Object".into()),
            parameters: vec![],
            throws: vec![],
        })
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        // Ordinary Object.clone metadata must not be used for the intrinsic path.
        Some(0x104)
    }
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
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, reference: ClassRef) -> Option<String> {
        (reference.index() == self.expected_ref).then(|| "java.lang.Object".into())
    }
    fn instruction_type(&self, reference: TypeRef) -> Option<JavaType> {
        (reference.index() == 9).then(|| JavaType::Array(Box::new(JavaType::Byte)))
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || matches!((actual, expected),
                (JavaType::Array(_), JavaType::Class(name)) if name == "java.lang.Object")
    }
}

fn array_clone_ir(
    kind: InvokeKind,
    receiver_ty: ManagedType,
    method: u32,
) -> (MethodIr<SsaForm>, ValueId) {
    let mut b = Builder::default();
    let p0 = b.parameter(receiver_ty);
    let raw = b.value(ManagedType::Reference(ClassRef::new(1)), |result| {
        ManagedInsn::Invoke {
            kind,
            result: Some(result),
            method: MethodRef::new(method),
            args: vec![p0],
        }
    });
    let cast = b.value(
        ManagedType::Array {
            element: Box::new(ManagedType::Byte),
            nullable: true,
        },
        |result| ManagedInsn::Cast {
            result,
            value: raw,
            target: TypeRef::new(9),
        },
    );
    b.push(ManagedInsn::Return { value: Some(cast) });
    (b.finish(), p0)
}

fn method() -> JavaMethod {
    JavaMethod {
        owner: "ArrayCloneHarness".into(),
        name: "copy".into(),
        is_static: true,
        return_type: JavaType::Array(Box::new(JavaType::Byte)),
        parameters: vec![JavaType::Array(Box::new(JavaType::Byte))],
        throws: vec![],
    }
}

#[test]
fn array_owner_clone_emits_receiver_clone_and_runtime_shallow_copy() {
    let (ir, p0) = array_clone_ir(
        InvokeKind::Virtual,
        ManagedType::Array {
            element: Box::new(ManagedType::Byte),
            nullable: true,
        },
        7,
    );
    let SourceResult::Complete { source, .. } = emit_method(
        &ir,
        &method(),
        &ParameterBindings {
            receiver: None,
            parameters: vec![p0],
            names: vec!["input".into()],
        },
        &Resolver {
            intrinsic: true,
            expected_ref: 1,
        },
    ) else {
        panic!("array clone rejected")
    };
    assert!(source.contains("input.clone()"), "{source}");
    assert!(!source.contains("CloneNotSupportedException"), "{source}");
    assert!(
        !source.contains("((java.lang.Object) input).clone()"),
        "{source}"
    );

    let dir = std::env::temp_dir().join(format!("decomphify-array-clone-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("ArrayCloneHarness.java");
    fs::write(
        &file,
        format!(
            "public class ArrayCloneHarness {{\n{source}\n  public static void main(String[] args) {{ byte[] a = new byte[]{{1,2}}; byte[] b = copy(a); if (a == b || b[0] != 1 || b[1] != 2) throw new AssertionError(); a[0] = 9; if (b[0] != 1) throw new AssertionError(); try {{ copy(null); throw new AssertionError(); }} catch (NullPointerException expected) {{}} }}\n}}"
        ),
    )
    .unwrap();
    let javac = "/home/jtjisgod/tools/jdk-17.0.13+11/bin/javac";
    let java = "/home/jtjisgod/tools/jdk-17.0.13+11/bin/java";
    assert!(Command::new(javac).arg(&file).status().unwrap().success());
    assert!(
        Command::new(java)
            .arg("-cp")
            .arg(&dir)
            .arg("ArrayCloneHarness")
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn array_clone_intrinsic_rejects_wrong_dispatch_receiver_and_reference() {
    let array = ManagedType::Array {
        element: Box::new(ManagedType::Byte),
        nullable: true,
    };
    for (kind, ty, method_ref) in [
        (InvokeKind::Static, array.clone(), 7),
        (InvokeKind::Special, array.clone(), 7),
        (
            InvokeKind::Virtual,
            ManagedType::Reference(ClassRef::new(1)),
            7,
        ),
        (InvokeKind::Virtual, array, 8),
    ] {
        let (ir, p0) = array_clone_ir(kind, ty, method_ref);
        assert!(matches!(
            emit_method(
                &ir,
                &method(),
                &ParameterBindings {
                    receiver: None,
                    parameters: vec![p0],
                    names: vec![],
                },
                &Resolver {
                    intrinsic: true,
                    expected_ref: 1,
                },
            ),
            SourceResult::Unsupported(_)
        ));
    }
}
