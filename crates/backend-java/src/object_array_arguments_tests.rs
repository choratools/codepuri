//! Allocation-order runtime and closure checks for array constructor arguments.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
};

struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Fixture.Outer".into())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn instruction_type(&self, t: TypeRef) -> Option<JavaType> {
        Some(if t.index() == 0 {
            JavaType::Int
        } else if t.index() == 1 {
            JavaType::Array(Box::new(JavaType::Int))
        } else {
            JavaType::Array(Box::new(JavaType::Long))
        })
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let array = JavaType::Array(Box::new(JavaType::Int));
        Some(match r.index() {
            0 => JavaMethod {
                owner: "Fixture.Outer".into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: vec![array.clone(), array],
                throws: vec![],
            },
            1 => JavaMethod {
                owner: "Fixture".into(),
                name: "length".into(),
                is_static: true,
                return_type: JavaType::Int,
                parameters: vec![JavaType::Int],
                throws: vec![],
            },
            _ => JavaMethod {
                owner: "Fixture".into(),
                name: "extra".into(),
                is_static: true,
                return_type: JavaType::Void,
                parameters: vec![],
                throws: vec![],
            },
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
        let mut last = cfg.entry();
        for id in &self.order[1..] {
            let next = cfg.add_block(*id);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}
fn array_type() -> ManagedType {
    ManagedType::Array {
        element: Box::new(ManagedType::Int),
        nullable: true,
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
    let length = b.value(ManagedType::Int, |result| ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: Some(result),
        method: MethodRef::new(1),
        args: vec![n],
    });
    let sized = b.value(array_type(), |result| ManagedInsn::NewArray {
        result,
        element: TypeRef::new(0),
        length,
    });
    let filled_element = if mode == 4 {
        b.value(ManagedType::Int, |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(1),
            args: vec![n],
        })
    } else {
        n
    };
    let filled = b.value(array_type(), |result| ManagedInsn::NewFilledArray {
        result: Some(result),
        target: TypeRef::new(if mode == 5 { 2 } else { 1 }),
        elements: vec![filled_element],
    });
    if mode == 3 {
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(2),
            args: vec![],
        });
    }
    if mode == 6 {
        b.value(ManagedType::Int, |result| ManagedInsn::ArrayLength {
            result,
            array: filled,
        });
    }
    if mode == 7 {
        b.push(ManagedInsn::Opaque {
            family_tag: 91,
            reads: vec![outer],
            writes: vec![],
            effects: managed_ir::effects::EffectFlags::NONE,
        });
    }
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: match mode {
            1 => vec![outer, filled, sized],
            2 => vec![outer, sized, sized],
            _ => vec![outer, sized, filled],
        },
    });
    if mode == 8 {
        b.value(ManagedType::Int, |result| ManagedInsn::ArrayLength {
            result,
            array: filled,
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
        &Resolver,
    )
}

#[test]
fn array_arguments_preserve_allocation_length_effect_throw_and_constructor_trace() {
    let result = emit(0);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert!(
        source.lines().any(|l| l.contains("new Fixture.Outer(")
            && l.contains("new int[")
            && l.contains("new int[] {")),
        "{source}"
    );
    let dir = std::env::temp_dir().join(format!("object-array-arguments-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"),format!(r#"public class Fixture {{
        static String log=""; static boolean failOuter;
        static class Outer {{ static {{log+="A";if(failOuter)throw new OutOfMemoryError("outer init");}} Outer(int[] a,int[] b){{if(a.length!=2||b.length!=1||b[0]!=2)throw new AssertionError();log+="C";}} }}
        static int length(int n){{if(!log.equals("A"))throw new AssertionError(log);log+="L";if(n==7)throw new IllegalArgumentException();return n;}}
        {source}
        public static void main(String[] args) {{
            int mode=Integer.parseInt(args[0]);failOuter=mode==4;
            try{{construct(mode==0?2:mode==1?-1:mode==2?Integer.MAX_VALUE:7);if(mode!=0)throw new AssertionError();}}
            catch(NegativeArraySizeException e){{if(mode!=1)throw e;}}
            catch(OutOfMemoryError e){{if(mode!=2&&mode!=4)throw e;}}
            catch(IllegalArgumentException e){{if(mode!=3)throw e;}}
            if(!log.equals(mode==0?"ALC":mode==4?"A":"AL"))throw new AssertionError(log);
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
    for mode in 0..=4 {
        let output = std::process::Command::new(format!("{home}/bin/java"))
            .current_dir(&dir)
            .args(["-Xmx32m", "-cp", ".", "Fixture", &mode.to_string()])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "mode{mode}:{}",
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
    let outer = body.find("// class Fixture$Outer").unwrap();
    let length = body.find("// Method length:").unwrap();
    let arrays: Vec<_> = body.match_indices("newarray").map(|(i, _)| i).collect();
    let ctor = body.find("// Method Fixture$Outer.\"<init>\"").unwrap();
    assert_eq!(arrays.len(), 2);
    assert!(
        outer < length && length < arrays[0] && arrays[0] < arrays[1] && arrays[1] < ctor,
        "{body}"
    );
    std::fs::write(dir.join("javap.txt"), bytecode).unwrap();
}
#[test]
fn sequenced_array_effects_preserve_order_and_reject_descriptor_and_receiver_escape() {
    for mode in 1..=8 {
        let result = emit(mode);
        if matches!(mode, 5 | 7) {
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
                "static class Outer { Outer(int[] a,int[] b) {} } static int length(int n) { return n; } static void extra() {}",
            );
        }
    }
}
