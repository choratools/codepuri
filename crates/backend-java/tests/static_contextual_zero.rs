//! Static replay retains the ordinary emitter's exact contextual declaration plan.
#![allow(clippy::unwrap_used)]
use backend_java::static_initializer::{
    StaticInitializerContext, emit_static_initializer_body_subset,
};
use backend_java::*;
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    insn::{Constant, InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};
struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, class: ClassRef) -> Option<String> {
        Some(
            if class.index() == 0 {
                "Fixture.Box"
            } else {
                "java.lang.Object"
            }
            .into(),
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture".into(),
            name: "BOX".into(),
            ty: JavaType::Class("Fixture.Box".into()),
            is_static: true,
        })
    }
    fn instruction_type(&self, t: TypeRef) -> Option<JavaType> {
        Some(if t.index() == 0 {
            JavaType::Int
        } else {
            JavaType::Class("java.lang.Object".into())
        })
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let (name, parameters, return_type, is_static, owner) = match r.index() {
            0 => (
                "<init>",
                vec![
                    JavaType::Int,
                    JavaType::Array(Box::new(JavaType::Int)),
                    JavaType::Array(Box::new(JavaType::Class("java.lang.Object".into()))),
                    JavaType::Boolean,
                ],
                JavaType::Void,
                false,
                "Fixture.Box",
            ),
            1 => (
                "<init>",
                vec![JavaType::Boolean],
                JavaType::Void,
                false,
                "Fixture.Box",
            ),
            2 => (
                "bit",
                vec![JavaType::Boolean],
                JavaType::Boolean,
                true,
                "Fixture",
            ),
            _ => ("number", vec![], JavaType::Int, true, "Fixture"),
        };
        Some(JavaMethod {
            owner: owner.into(),
            name: name.into(),
            parameters,
            return_type,
            is_static,
            throws: vec![],
        })
    }
}
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
    fn push(&mut self, insn: ManagedInsn) {
        self.order.push(self.insns.insert(insn));
    }
    fn finish(self) -> MethodIr<SsaForm> {
        let mut cfg = Cfg::new(self.order[0]);
        let mut previous = cfg.entry();
        for insn in self.order.into_iter().skip(1) {
            let next = cfg.add_block(insn);
            cfg.add_edge(previous, next, EdgeKind::Unconditional)
                .unwrap();
            previous = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}
fn signature(name: &str, ty: JavaType, parameters: Vec<JavaType>) -> JavaMethod {
    JavaMethod {
        owner: "Fixture".into(),
        name: name.into(),
        return_type: ty,
        parameters,
        is_static: true,
        throws: vec![],
    }
}

fn source() -> String {
    let mut b = Builder::default();
    let object = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    let zero = b.value(ManagedType::Zero, |result| ManagedInsn::Const {
        result,
        value: Constant::Zero,
    });
    let ints = b.value(
        ManagedType::Array {
            element: Box::new(ManagedType::Int),
            nullable: true,
        },
        |result| ManagedInsn::NewArray {
            result,
            length: zero,
            element: TypeRef::new(0),
        },
    );
    let objects = b.value(
        ManagedType::Array {
            element: Box::new(ManagedType::Reference(ClassRef::new(1))),
            nullable: true,
        },
        |result| ManagedInsn::NewArray {
            result,
            length: zero,
            element: TypeRef::new(1),
        },
    );
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![object, zero, ints, objects, zero],
    });
    b.push(ManagedInsn::FieldPut {
        object: None,
        field: FieldRef::new(0),
        value: object,
    });
    b.push(ManagedInsn::Return { value: None });
    let ir = b.finish();
    emit_static_initializer_body_subset(
        &ir,
        &signature("<clinit>", JavaType::Void, vec![]),
        &ParameterBindings::default(),
        &Resolver,
        &StaticInitializerContext {
            owner: "Fixture".into(),
            writable_static_fields: vec![],
        },
        &[FieldRef::new(0)],
    )
    .unwrap()
}
#[test]
fn exact_zero_shared_uses_survive_static_replay() {
    let block = source();
    assert!(block.contains("!= 0"), "{block}");
    assert!(block.contains("new int["), "{block}");
    assert!(block.contains("BOX ="), "{block}");
}
#[test]
fn static_replay_compiles_and_runs_on_java17_and25() {
    let block = source();
    let text = format!(
        "public class Fixture {{ static final Box BOX; {block} static class Box {{ final int n; final int[] a; final Object[] b; final boolean bit; Box(int n,int[] a,Object[] b,boolean bit) {{this.n=n;this.a=a;this.b=b;this.bit=bit;}} }} public static void main(String[] x) {{if(BOX.n!=0||BOX.a.length!=0||BOX.b.length!=0||BOX.bit)throw new AssertionError(); System.out.println(\"shared zero exact\");}} }}"
    );
    for (release, home) in [
        ("17", "/home/jtjisgod/tools/jdk-17.0.13+11"),
        ("25", "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1"),
    ] {
        let dir =
            std::env::temp_dir().join(format!("static-zero-{}-{release}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Fixture.java"), &text).unwrap();
        for (tool, args) in [
            ("javac", vec!["--release", release, "Fixture.java"]),
            ("java", vec!["-cp", ".", "Fixture"]),
        ] {
            let out = std::process::Command::new(format!("{home}/bin/{tool}"))
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{text}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}
