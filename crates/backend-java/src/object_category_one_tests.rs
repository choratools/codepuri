#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
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
        None
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
fn mixed_literal(bit: i32) -> SourceResult {
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
    let literal = if bit == 0 {
        zero
    } else {
        b.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(bit),
        })
    };
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
        args: vec![object, literal, ints, objects, literal],
    });
    b.push(ManagedInsn::Return {
        value: Some(object),
    });
    emit_method(
        &b.finish(),
        &signature(
            if bit == 1 { "mixedOne" } else { "mixed" },
            JavaType::Class("Fixture.Box".into()),
            vec![],
        ),
        &ParameterBindings::default(),
        &Resolver,
    )
}
fn xor_argument() -> SourceResult {
    let mut b = Builder::default();
    let flag = b.value(ManagedType::Boolean, |v| ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![v],
        effects: managed_ir::effects::EffectFlags::NONE,
    });
    let object = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    let one = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    let flipped = b.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Xor,
        result,
        lhs: flag,
        rhs: one,
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![object, flipped],
    });
    b.push(ManagedInsn::Return {
        value: Some(object),
    });
    emit_method(
        &b.finish(),
        &signature(
            "xorArgument",
            JavaType::Class("Fixture.Box".into()),
            vec![JavaType::Boolean],
        ),
        &ParameterBindings {
            parameters: vec![flag],
            ..Default::default()
        },
        &Resolver,
    )
}
fn return_bit(genuine_boolean: bool) -> SourceResult {
    let mut b = Builder::default();
    let flag = b.value(ManagedType::Boolean, |v| ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![v],
        effects: managed_ir::effects::EffectFlags::NONE,
    });
    let value = b.value(
        if genuine_boolean {
            ManagedType::Int
        } else {
            ManagedType::Boolean
        },
        |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(if genuine_boolean { 2 } else { 3 }),
            args: if genuine_boolean { vec![flag] } else { vec![] },
        },
    );
    let one = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    let result = b.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Xor,
        result,
        lhs: value,
        rhs: one,
    });
    b.push(ManagedInsn::Return {
        value: Some(result),
    });
    emit_method(
        &b.finish(),
        &signature("returnBit", JavaType::Boolean, vec![JavaType::Boolean]),
        &ParameterBindings {
            parameters: vec![flag],
            ..Default::default()
        },
        &Resolver,
    )
}
fn source(result: SourceResult) -> String {
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    source
}
#[test]
fn category_one_argument_capture_and_boolean_call_keep_identity_and_effects() {
    let mixed = source(mixed_literal(0));
    let mixed_one = source(mixed_literal(1));
    let xor = source(xor_argument());
    let returned = source(return_bit(true));
    let assertions = "Box m = mixed(); if(m.n != 0 || m.b || m.ints.length != 0 || m.objects.length != 0) throw new AssertionError(); Box m1 = mixedOne(); if(m1.n != 1 || !m1.b || m1.ints.length != 0 || m1.objects.length != 0) throw new AssertionError(); for(boolean b:new boolean[]{false,true}) { if(xorArgument(b).b == b || returnBit(b) == b) throw new AssertionError(); } if(calls != 2) throw new AssertionError(calls);";
    let code = format!(
        "public class Fixture {{ static int calls; static boolean bit(boolean b) {{ calls++; return b; }} static int number() {{ return 2; }} static class Box {{ int n; boolean b; int[] ints; Object[] objects; Box(boolean b) {{this.b=b;}} Box(int n,int[] a,Object[] o,boolean b){{this.n=n;ints=a;objects=o;this.b=b;}} }} {mixed} {mixed_one} {xor} {returned} public static void main(String[] a) {{ {assertions} }} }}"
    );
    for (index, home) in [
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into()),
        std::env::var("DECOMPHIFY_JDK25_HOME")
            .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into()),
    ]
    .into_iter()
    .enumerate()
    {
        let dir = std::env::temp_dir().join(format!("category-one-{}-{index}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Fixture.java"), &code).unwrap();
        for (tool, args) in [
            ("javac", vec!["Fixture.java"]),
            ("java", vec!["-cp", ".", "Fixture"]),
        ] {
            let out = std::process::Command::new(format!("{home}/bin/{tool}"))
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{tool}: {}\n{code}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}
#[test]
fn category_one_argument_and_return_reject_nonbit_integers() {
    assert!(matches!(mixed_literal(2), SourceResult::Unsupported(_)));
    assert!(matches!(return_bit(false), SourceResult::Unsupported(_)));
    let mut b = Builder::default();
    let integer = b.value(ManagedType::Int, |result| ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: Some(result),
        method: MethodRef::new(3),
        args: vec![],
    });
    let copied_boolean = b.value(ManagedType::Boolean, |result| ManagedInsn::Move {
        result,
        value: integer,
    });
    b.push(ManagedInsn::Return {
        value: Some(copied_boolean),
    });
    assert!(matches!(
        emit_method(
            &b.finish(),
            &signature("copiedNumber", JavaType::Boolean, vec![]),
            &ParameterBindings::default(),
            &Resolver
        ),
        SourceResult::Unsupported(_)
    ));
}
