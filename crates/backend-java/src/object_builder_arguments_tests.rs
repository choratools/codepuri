//! Trusted discarded-result standard-library builder chain checks.
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
    fn discarded_invoke_source_target(&self, reference: MethodRef) -> bool {
        !(self.mode == 3 && reference.index() == 2)
    }
    fn trusted_string_builder_class(&self, r: ClassRef) -> bool {
        self.mode != 1 && r.index() == 1
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            match r.index() {
                0 => "Fixture.Outer",
                1 => "java.lang.StringBuilder",
                2 => "java.lang.Object",
                _ => "java.lang.String",
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
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        Some(if self.mode == 2 && r.index() == 2 {
            2
        } else {
            1
        })
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let string = JavaType::Class("java.lang.String".into());
        let builder = JavaType::Class("java.lang.StringBuilder".into());
        let mut m = match r.index() {
            0 => JavaMethod {
                owner: "Fixture.Outer".into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: vec![string.clone()],
                throws: vec![],
            },
            1 => JavaMethod {
                owner: "java.lang.StringBuilder".into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: vec![JavaType::Int],
                throws: vec![],
            },
            2 | 3 | 10 => JavaMethod {
                owner: "java.lang.StringBuilder".into(),
                name: "append".into(),
                is_static: false,
                return_type: builder.clone(),
                parameters: vec![if r.index() == 2 {
                    string.clone()
                } else if r.index() == 10 {
                    JavaType::Long
                } else {
                    JavaType::Class("java.lang.Object".into())
                }],
                throws: vec![],
            },
            4 => JavaMethod {
                owner: "java.lang.StringBuilder".into(),
                name: "toString".into(),
                is_static: false,
                return_type: string.clone(),
                parameters: vec![],
                throws: vec![],
            },
            5 | 6 => JavaMethod {
                owner: "Fixture".into(),
                name: if r.index() == 5 { "first" } else { "last" }.into(),
                is_static: true,
                return_type: string.clone(),
                parameters: vec![],
                throws: vec![],
            },
            _ => JavaMethod {
                owner: "Fixture".into(),
                name: "consume".into(),
                is_static: true,
                return_type: JavaType::Void,
                parameters: vec![builder],
                throws: vec![],
            },
        };
        if r.index() == 2 {
            match self.mode {
                3 => m.return_type = JavaType::Class("java.lang.Object".into()),
                4 => m.owner = "Fixture.Fake".into(),
                5 => m.throws = vec!["java.io.IOException".into()],
                _ => {}
            }
        }
        Some(m)
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
    let object = b.value(ManagedType::Reference(ClassRef::new(2)), |v| {
        ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![v],
            effects: managed_ir::effects::EffectFlags::NONE,
        }
    });
    b.order.pop();
    let wide = b.value(ManagedType::Long, |v| ManagedInsn::Const {
        result: v,
        value: Constant::Long(41),
    });
    let high = b.value(ManagedType::Long, |v| ManagedInsn::Const {
        result: v,
        value: Constant::Long(42),
    });
    let outer = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    let builder = b.value(ManagedType::Reference(ClassRef::new(1)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(1),
        }
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![builder, n],
    });
    let first = b.value(ManagedType::Reference(ClassRef::new(3)), |result| {
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(5),
            args: vec![],
        }
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Virtual,
        result: None,
        method: MethodRef::new(2),
        args: vec![builder, first],
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Virtual,
        result: None,
        method: MethodRef::new(3),
        args: vec![
            if mode == 7 { object } else { builder },
            if mode == 8 { builder } else { object },
        ],
    });
    if mode == 6 {
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(7),
            args: vec![builder],
        });
    }
    if mode == 9 || mode == 10 {
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: None,
            method: MethodRef::new(10),
            args: vec![builder, wide, if mode == 10 { high } else { wide }],
        });
    }
    let last = b.value(ManagedType::Reference(ClassRef::new(3)), |result| {
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(6),
            args: vec![],
        }
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Virtual,
        result: None,
        method: MethodRef::new(2),
        args: vec![builder, last],
    });
    let text = b.value(ManagedType::Reference(ClassRef::new(3)), |result| {
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: Some(result),
            method: MethodRef::new(4),
            args: vec![builder],
        }
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![outer, text],
    });
    b.push(ManagedInsn::Return { value: Some(outer) });
    emit_method(
        &b.finish(),
        &JavaMethod {
            owner: "Fixture".into(),
            name: "construct".into(),
            is_static: true,
            return_type: JavaType::Class("Fixture.Outer".into()),
            parameters: vec![JavaType::Int, JavaType::Class("java.lang.Object".into())],
            throws: vec![],
        },
        &ParameterBindings {
            parameters: vec![n, object],
            ..Default::default()
        },
        &Resolver { mode },
    )
}
fn runtime(mode: u8) {
    let result = emit(mode);
    let SourceResult::Complete { source, .. } = result else {
        panic!("mode{mode}: {result:?}")
    };
    let dir = std::env::temp_dir().join(format!(
        "object-builder-chain-{}-{mode}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"),format!(r#"public class Fixture {{
        static String log="";static int mode;static StringBuilder escaped;static boolean selfArgument={self_argument};
        static void consume(StringBuilder b){{if(!log.equals("A1O"))throw new AssertionError(log);escaped=b;log+="B";if(mode==7)throw new IllegalArgumentException();}}
        static class Outer {{static{{log+="A";if(mode==4)throw new OutOfMemoryError("outer class init");}} Outer(String s){{if(!s.equals("one{}end"))throw new AssertionError(s);log+="C";if(mode==5)throw new IllegalArgumentException();}}}}
        static String first(){{if(!log.equals("A"))throw new AssertionError(log);log+="1";return "one";}}
        static String last(){{if(!log.equals(selfArgument?"A1":escaped==null?"A1O":"A1OB"))throw new AssertionError(log);log+="2";if(mode==3)throw new IllegalArgumentException();return "end";}}
        static class Probe {{public String toString(){{if(!log.equals("A1"))throw new AssertionError(log);log+="O";if(mode==2)throw new IllegalArgumentException();return "value";}}}}
        {source}
        public static void main(String[] args){{mode=Integer.parseInt(args[0]);
            try{{construct(mode==1?-1:mode==6?Integer.MAX_VALUE:16,new Probe());if(mode!=0)throw new AssertionError();}}
            catch(NegativeArraySizeException e){{if(mode!=1)throw e;}}
            catch(OutOfMemoryError e){{if(mode!=4&&mode!=6)throw e;}}
            catch(IllegalArgumentException e){{if(mode!=2&&mode!=3&&mode!=5&&mode!=7)throw e;}}
            String expected=mode==0||mode==5?"A1O2C":mode==2?"A1O":mode==3?"A1O2":"A";
            if(selfArgument)expected=expected.replace("A1O", "A1");
            if (escaped!=null) {{
                expected=mode==7?"A1OB":expected.replace("A1O2", "A1OB2");
                if(mode==0&&!escaped.toString().equals("onevalueend"))throw new AssertionError();
            }}
            if(!log.equals(expected))throw new AssertionError(log);
        }}
    }}"#,if mode==9{"value41"}else if mode==8{"one"}else{"value"}, self_argument=if mode==8{"true"}else{"false"})).unwrap();
    let (home, release) = if matches!(mode, 1 | 6 | 8) {
        (
            std::env::var("DECOMPHIFY_JDK25_HOME")
                .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into()),
            "25",
        )
    } else {
        (
            std::env::var("JAVA_HOME")
                .unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into()),
            "17",
        )
    };
    let output = std::process::Command::new(format!("{home}/bin/javac"))
        .current_dir(&dir)
        .args(["--release", release, "Fixture.java"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    for scenario in 0..=if mode == 6 { 7 } else { 6 } {
        if mode == 8 && scenario == 2 {
            continue; // Original append argument is the builder, not Probe.
        }
        let output = std::process::Command::new(format!("{home}/bin/java"))
            .current_dir(&dir)
            .args(["-Xmx32m", "-cp", ".", "Fixture", &scenario.to_string()])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "scenario{scenario}:{}\n{source}",
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
        .split("static Fixture$Outer construct(int, java.lang.Object);")
        .nth(1)
        .unwrap()
        .split("public static void main")
        .next()
        .unwrap();
    assert_eq!(
        body.lines()
            .filter(|l| l.split_whitespace().nth(1) == Some("new"))
            .count(),
        2,
        "{body}"
    );
    assert!(
        body.find("// class Fixture$Outer").unwrap()
            < body.find("// class java/lang/StringBuilder").unwrap()
    );
    assert_eq!(body.matches("StringBuilder.toString:").count(), 1);
    assert_eq!(
        body.matches("StringBuilder.append:").count(),
        if mode == 9 { 4 } else { 3 }
    );
    std::fs::write(dir.join("javap.txt"), bytecode).unwrap();
}
#[test]
fn trusted_builder_chain_preserves_private_receiver_effects_and_abrupt_completion() {
    runtime(0);
}
#[test]
fn builder_chain_rejects_invalid_metadata_receiver_escape_and_distinct_high_word() {
    for mode in [2, 3, 4, 5, 7, 10] {
        let result = emit(mode);
        assert!(
            matches!(result, SourceResult::Unsupported(_)),
            "mode{mode}:{result:?}"
        );
    }
}
#[test]
fn trusted_builder_chain_preserves_attested_wide_append_word_identity() {
    runtime(9);
}

#[test]
fn ordinary_builder_calls_preserve_effects_without_fluent_attestation() {
    let SourceResult::Complete { source, .. } = emit(1) else {
        panic!("ordinary builder sequence unavailable");
    };
    assert!(
        source.contains("original allocation argument sequence"),
        "{source}"
    );
    runtime(1);
}

#[test]
fn ordinary_builder_escape_preserves_identity_and_abrupt_effects() {
    let SourceResult::Complete { source, .. } = emit(6) else {
        panic!("ordinary escaping builder sequence unavailable");
    };
    assert!(source.contains("Fixture.consume("), "{source}");
    runtime(6);
}

#[test]
fn ordinary_builder_self_argument_retains_object_overload_and_effects() {
    let SourceResult::Complete { source, .. } = emit(8) else {
        panic!("ordinary builder self argument unavailable");
    };
    assert!(source.contains("java.lang.Object"), "{source}");
    runtime(8);
}
