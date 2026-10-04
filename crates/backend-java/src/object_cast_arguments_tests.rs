//! Throwing reference casts retain allocation-before-CHECKCAST ordering.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
};
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
        let mut last = cfg.entry();
        for id in &self.order[1..] {
            let next = cfg.add_block(*id);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}

struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            match r.index() {
                0 => "Fixture.Outer",
                2 => "java.lang.String",
                _ => "java.lang.Object",
            }
            .into(),
        )
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn instruction_type(&self, _: TypeRef) -> Option<JavaType> {
        Some(JavaType::Class("java.lang.String".into()))
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "Fixture.Outer".into(),
            name: "<init>".into(),
            is_static: false,
            return_type: JavaType::Void,
            parameters: vec![JavaType::Class("java.lang.String".into())],
            throws: vec![],
        })
    }
}
fn emit(mode: u8) -> SourceResult {
    let mut b = Builder::default();
    let input = b.value(ManagedType::Reference(ClassRef::new(1)), |v| {
        ManagedInsn::Opaque {
            family_tag: 32769,
            reads: vec![],
            writes: vec![v],
            effects: managed_ir::effects::EffectFlags::NONE,
        }
    });
    b.order.pop();
    let outer = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    let moved = b.value(ManagedType::Reference(ClassRef::new(1)), |result| {
        ManagedInsn::Move {
            result,
            value: input,
        }
    });
    // Use a resolved String ClassRef for the cast result.
    let cast = b.value(
        if mode == 3 {
            ManagedType::Int
        } else {
            ManagedType::Reference(ClassRef::new(2))
        },
        |result| ManagedInsn::Cast {
            result,
            value: moved,
            target: TypeRef::new(0),
        },
    );
    if mode == 1 {
        b.push(ManagedInsn::Opaque {
            family_tag: 1,
            reads: vec![cast],
            writes: vec![],
            effects: managed_ir::effects::EffectFlags::NONE,
        });
    }
    if mode == 2 {
        b.push(ManagedInsn::Opaque {
            family_tag: 1,
            reads: vec![outer],
            writes: vec![],
            effects: managed_ir::effects::EffectFlags::NONE,
        });
    }
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![outer, cast],
    });
    b.push(ManagedInsn::Return { value: Some(outer) });
    emit_method(
        &b.finish(),
        &JavaMethod {
            owner: "Fixture".into(),
            name: "construct".into(),
            is_static: true,
            return_type: JavaType::Class("Fixture.Outer".into()),
            parameters: vec![JavaType::Class("java.lang.Object".into())],
            throws: vec![],
        },
        &ParameterBindings {
            parameters: vec![input],
            ..Default::default()
        },
        &Resolver,
    )
}
#[test]
fn reference_cast_argument_retains_failure_order_and_null_identity() {
    let result = emit(0);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    let java = format!(
        "public class Fixture {{static String trace=\"\";static class Outer{{static{{trace+=\"A\";}} final String value;Outer(String s){{value=s;trace+=\"C\";}}}} {source} public static void main(String[]args){{try{{construct(new Object());throw new AssertionError();}}catch(ClassCastException expected){{if(!trace.equals(\"A\"))throw new AssertionError(trace);}}trace=\"\";String s=new String(\"x\");if(construct(s).value!=s||!trace.equals(\"C\"))throw new AssertionError();trace=\"\";if(construct(null).value!=null||!trace.equals(\"C\"))throw new AssertionError();}}}}"
    );
    let dir = std::env::temp_dir().join(format!("object-cast-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), java).unwrap();
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (tool, args) in [
        ("javac", vec!["--release", "17", "Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let o = std::process::Command::new(format!("{home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    let bytecode = std::process::Command::new(format!("{home}/bin/javap"))
        .args(["-c", "-p", "Fixture"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(bytecode.status.success());
    let bytecode = String::from_utf8(bytecode.stdout).unwrap();
    let construct = bytecode
        .split("construct(java.lang.Object)")
        .nth(1)
        .unwrap()
        .split("public static void main")
        .next()
        .unwrap();
    // JVM allocation (including its OOM boundary) precedes CHECKCAST and init.
    let allocation = construct.find("new ").unwrap();
    let cast = construct.find("checkcast").unwrap();
    let init = construct.find("invokespecial").unwrap();
    assert!(allocation < cast && cast < init, "{construct}");
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn reference_cast_argument_rejects_escape_extra_read_and_wrong_result_type() {
    for mode in 1..=3 {
        assert!(
            matches!(emit(mode), SourceResult::Unsupported(_)),
            "mode {mode}"
        );
    }
}
