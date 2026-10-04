//! Exact class-literal resolution event inside allocation arguments.
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
struct Resolver {
    mode: u8,
}
impl JavaResolver for Resolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r.index() == 0 {
                "Fixture.Outer"
            } else {
                "java.lang.Class"
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
        if self.mode == 4 {
            None
        } else if self.mode == 5 {
            Some(JavaType::Int)
        } else if self.mode == 6 {
            Some(JavaType::Array(Box::new(JavaType::Int)))
        } else {
            Some(JavaType::Class("Fixture.Target".into()))
        }
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let class = JavaType::Class("java.lang.Class".into());
        Some(match r.index() {
            0 => JavaMethod {
                owner: "Fixture.Outer".into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: if self.mode == 2 {
                    vec![class.clone(), class]
                } else if self.mode == 3 {
                    vec![JavaType::Int, class]
                } else {
                    vec![class, JavaType::Int]
                },
                throws: vec![],
            },
            1 => JavaMethod {
                owner: "Fixture".into(),
                name: "tick".into(),
                is_static: true,
                return_type: JavaType::Int,
                parameters: vec![JavaType::Int],
                throws: vec![],
            },
            _ => JavaMethod {
                owner: "Fixture".into(),
                name: "consume".into(),
                is_static: true,
                return_type: JavaType::Void,
                parameters: vec![class],
                throws: vec![],
            },
        })
    }
}
fn emit(mode: u8) -> SourceResult {
    let mut b = Builder::default();
    let n = b.value(ManagedType::Int, |v| ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![v],
        effects: managed_ir::effects::EffectFlags::NONE,
    });
    b.order.pop();
    let outer = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    let class = b.value(ManagedType::Reference(ClassRef::new(1)), |result| {
        ManagedInsn::Const {
            result,
            value: Constant::ClassLiteral(TypeRef::new(0)),
        }
    });
    let number = b.value(ManagedType::Int, |result| ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: Some(result),
        method: MethodRef::new(1),
        args: vec![n],
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: if mode == 2 {
            vec![outer, class, class]
        } else if mode == 3 {
            vec![outer, number, class]
        } else {
            vec![outer, class, number]
        },
    });
    if mode == 1 {
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(2),
            args: vec![class],
        });
    }
    b.push(ManagedInsn::Return { value: Some(outer) });
    emit_method(
        &b.finish(),
        &JavaMethod {
            owner: "Fixture".into(),
            name: "construct".into(),
            is_static: true,
            return_type: JavaType::Class("Fixture.Outer".into()),
            parameters: vec![JavaType::Int],
            throws: vec![],
        },
        &ParameterBindings {
            parameters: vec![n],
            ..Default::default()
        },
        &Resolver { mode },
    )
}
#[test]
fn class_literal_argument_preserves_allocation_resolution_and_target_noninitialization() {
    let result = emit(0);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    let dir = std::env::temp_dir().join(format!("object-class-literal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"),format!(r#"public class Fixture {{
        static String log="";static int mode;
        static class Outer {{static{{log+="A";if(mode==1)throw new OutOfMemoryError("outer init");}} Outer(Class<?> c,int n){{if(c!=Target.class||n!=2||!log.equals("A1"))throw new AssertionError(log);log+="C";if(mode==3)throw new IllegalArgumentException();}}}}
        static class Target {{static{{log+="T";}} static void touch(){{}}}}
        static int tick(int n){{if(!log.equals("A"))throw new AssertionError(log);log+="1";if(mode==2)throw new IllegalArgumentException();return n;}}
        {source}
        public static void main(String[] args){{mode=Integer.parseInt(args[0]);
            try{{construct(2);if(mode!=0)throw new AssertionError();}}
            catch(OutOfMemoryError e){{if(mode!=1)throw e;}}
            catch(IllegalArgumentException e){{if(mode!=2&&mode!=3)throw e;}}
            catch(NoClassDefFoundError e){{if(mode!=4)throw e;}}
            if(!log.equals(mode==0||mode==3?"A1C":mode==2?"A1":"A"))throw new AssertionError(log);
            if(mode==0){{Target.touch();if(!log.equals("A1CT"))throw new AssertionError(log);}}
        }}
    }}"#)).unwrap();
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let output = std::process::Command::new(format!("{home}/bin/javac"))
        .current_dir(&dir)
        .args(["--release", "17", "Fixture.java"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    for scenario in 0..=4 {
        if scenario == 4 {
            std::fs::remove_file(dir.join("Fixture$Target.class")).unwrap();
        }
        let output = std::process::Command::new(format!("{home}/bin/java"))
            .current_dir(&dir)
            .args(["-cp", ".", "Fixture", &scenario.to_string()])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "scenario{scenario}:{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = std::process::Command::new(format!("{home}/bin/javap"))
        .current_dir(&dir)
        .args(["-c", "-p", "Fixture"])
        .output()
        .unwrap();
    let bytecode = String::from_utf8(output.stdout).unwrap();
    let body = bytecode
        .split("static Fixture$Outer construct(int);")
        .nth(1)
        .unwrap()
        .split("public static void main")
        .next()
        .unwrap();
    assert!(
        body.find("// class Fixture$Outer").unwrap()
            < body.find("// class Fixture$Target").unwrap()
    );
    assert_eq!(body.matches("// class Fixture$Target").count(), 1);
    assert!(body.find("// class Fixture$Target").unwrap() < body.find("// Method tick:").unwrap());
    std::fs::write(dir.join("javap.txt"), bytecode).unwrap();
}
#[test]
fn sequenced_class_literals_capture_once_and_reject_unknown_and_primitive_wrapper_proof() {
    for mode in 1..=5 {
        let result = emit(mode);
        if mode >= 4 {
            assert!(
                matches!(result, SourceResult::Unsupported(_)),
                "mode{mode}:{result:?}"
            );
        } else {
            let SourceResult::Complete { source, .. } = result else {
                panic!("mode{mode}:{result:?}")
            };
            crate::object_plan::sequence_tests::compile_fixture(
                &source,
                "static class Target {} static class Outer { Outer(Class<?> c,int n) {} Outer(int n,Class<?> c) {} Outer(Class<?> a,Class<?> b) {} } static int tick(int n) { return n; } static void consume(Class<?> c) {}",
            );
        }
    }
}

#[test]
fn class_literal_array_argument_is_ldc_without_array_allocation_or_wrapper_initialization() {
    let result = emit(6);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    let dir =
        std::env::temp_dir().join(format!("object-array-class-literal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), format!(r#"public class Fixture {{
        static class Outer {{ Outer(Class<?> c,int n){{if(c != int[].class || n != 2)throw new AssertionError();}} }}
        static int tick(int n){{return n;}}
        {source}
        public static void main(String[] args){{construct(2);}}
    }}"#)).unwrap();
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (tool, args) in [
        ("javac", vec!["--release", "17", "Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let output = std::process::Command::new(format!("{home}/bin/{tool}"))
            .current_dir(&dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{source}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = std::process::Command::new(format!("{home}/bin/javap"))
        .current_dir(&dir)
        .args(["-c", "-p", "Fixture"])
        .output()
        .unwrap();
    let bytecode = String::from_utf8(output.stdout).unwrap();
    let body = bytecode
        .split("static Fixture$Outer construct(int);")
        .nth(1)
        .unwrap()
        .split("public static void main")
        .next()
        .unwrap();
    let literal = body
        .lines()
        .find(|line| line.contains("ldc") && line.contains("[I"))
        .unwrap();
    assert!(body.find("// class Fixture$Outer").unwrap() < body.find(literal).unwrap());
    assert!(!body.contains("newarray") && !body.contains("getstatic") && !body.contains("TYPE"));
    std::fs::write(dir.join("javap.txt"), bytecode).unwrap();
}
