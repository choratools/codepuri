//! Primitive switch scaffolding preserves allocation and original caller frames.
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
            let result = self.values.insert(Value::new(id, ty));
            value = Some(result);
            f(result)
        });
        self.order.push(id);
        value.unwrap()
    }
    fn push(&mut self, i: ManagedInsn) {
        self.order.push(self.insns.insert(i));
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
            if r.index() == 0 {
                "SequenceRuntime.Outer"
            } else {
                "java.lang.String"
            }
            .into(),
        )
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        Some("context".encode_utf16().collect())
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let (name, params, ret) = match r.index() {
            0 => (
                "<init>",
                vec![JavaType::Int, JavaType::Class("java.lang.String".into())],
                JavaType::Void,
            ),
            1 => ("model", vec![], JavaType::Int),
            2 => (
                "context",
                vec![],
                JavaType::Class("java.lang.String".into()),
            ),
            3 => (
                "check",
                vec![JavaType::Class("java.lang.String".into()); 2],
                JavaType::Void,
            ),
            4 => ("<init>", vec![], JavaType::Void),
            5 => (
                "consume",
                vec![JavaType::Class("java.lang.String".into())],
                JavaType::Void,
            ),
            6 => ("<init>", vec![JavaType::Int; 2], JavaType::Void),
            _ => return None,
        };
        Some(JavaMethod {
            owner: if matches!(r.index(), 0 | 4 | 6) {
                "SequenceRuntime.Outer"
            } else {
                "SequenceRuntime"
            }
            .into(),
            name: name.into(),
            is_static: !matches!(r.index(), 0 | 4 | 6),
            return_type: ret,
            parameters: params,
            throws: if r.index() == 3 {
                vec!["java.io.IOException".into()]
            } else {
                vec![]
            },
        })
    }
}
fn fixture(mode: u8) -> SourceResult {
    let mut b = Builder::default();
    let outer = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    let model = b.value(ManagedType::Int, |result| ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: Some(result),
        method: MethodRef::new(1),
        args: vec![],
    });
    let context = b.value(ManagedType::Reference(ClassRef::new(1)), |result| {
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(2),
            args: vec![],
        }
    });
    let name = b.value(ManagedType::Reference(ClassRef::new(1)), |result| {
        ManagedInsn::Const {
            result,
            value: Constant::String(StringRef::new(0)),
        }
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(if mode == 3 { 99 } else { 3 }),
        args: vec![if mode == 2 { outer } else { context }, name],
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(if mode == 1 {
            4
        } else if mode == 4 {
            6
        } else {
            0
        }),
        args: if mode == 1 {
            vec![outer]
        } else if mode == 4 {
            vec![outer, model, model]
        } else {
            vec![outer, model, context]
        },
    });
    if mode == 5 {
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(5),
            args: vec![context],
        });
    }
    b.push(ManagedInsn::Return { value: Some(outer) });
    emit_method(
        &b.finish(),
        &JavaMethod {
            owner: "SequenceRuntime".into(),
            name: "build".into(),
            is_static: true,
            return_type: JavaType::Class("SequenceRuntime.Outer".into()),
            parameters: vec![],
            throws: if mode == 6 {
                vec![]
            } else {
                vec!["java.io.IOException".into()]
            },
        },
        &ParameterBindings::default(),
        &Resolver,
    )
}
#[test]
fn switch_sequence_preserves_genuine_void_call_and_typed_captures() {
    for mode in [0, 4, 5] {
        let result = fixture(mode);
        let SourceResult::Complete { source, .. } = result else {
            panic!("mode{mode}:{result:?}")
        };
        assert!(source.contains("new SequenceRuntime.Outer("));
        assert_eq!(source.matches("switch (0)").count(), 1);
        assert_eq!(source.matches("SequenceRuntime.model(").count(), 1);
        assert_eq!(source.matches("SequenceRuntime.context(").count(), 1);
        assert_eq!(source.matches("SequenceRuntime.check(").count(), 1);
        assert!(source.contains("compiler scaffold: original allocation argument sequence"));
        compile(&source, false, false);
    }
}
#[test]
fn switch_sequence_denies_zero_arg_uninitialized_escape_unknown_and_uncovered_throws() {
    for mode in [1, 2, 3, 6] {
        assert!(
            matches!(fixture(mode), SourceResult::Unsupported(_)),
            "mode{mode}"
        );
    }
}
#[test]
fn switch_sequence_source17_preserves_frame_effects_and_failures() {
    let SourceResult::Complete { source, .. } = fixture(0) else {
        panic!("{:?}", fixture(0))
    };
    compile(&source, true, false);
}
#[test]
#[ignore = "requires genuine JDK25 public ClassFile API"]
fn switch_sequence_original_bytecode_differential() {
    let SourceResult::Complete { source, .. } = fixture(0) else {
        panic!("{:?}", fixture(0))
    };
    compile(&source, true, true);
}
fn compile(source: &str, runtime: bool, original: bool) {
    let home = if original {
        std::env::var("DECOMPHIFY_JDK25_HOME")
            .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into())
    } else {
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into())
    };
    let dir = std::env::temp_dir().join(format!(
        "decomphify-object-switch-{}-{}-{}",
        std::process::id(),
        runtime,
        original
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SequenceRuntime.java"),format!(r#"public class SequenceRuntime {{
static int mode;static String events="";static final RuntimeException failure=new IllegalArgumentException("original");static final java.io.IOException checked=new java.io.IOException("checked");
static class Outer {{static {{events+="A";if(mode==6)throw new OutOfMemoryError("explicit allocation-site class-init boundary");}} final int model;final String context;Outer(int m,String c){{events+="C";if(mode==5)throw failure;model=m;context=c;}}Outer(int m,int n){{this(m,"ctx");if(m!=n)throw new AssertionError();}}}}
static int model(){{events+="M";if(mode==2)throw failure;return 7;}}
static String context(){{events+="X";if(mode==3)throw failure;return mode==1?null:"ctx";}}
static void check(String context,String name)throws java.io.IOException{{events+="K";if(!Thread.currentThread().getStackTrace()[2].getMethodName().equals("build"))throw new AssertionError("caller frame changed");if(!name.equals("context"))throw new AssertionError();if(context==null)throw new NullPointerException(name);if(mode==4)throw checked;}}
static void consume(String c){{if(!c.equals("ctx"))throw new AssertionError();}}
{source}
public static void main(String[]args){{mode=Integer.parseInt(args[0]);try {{Outer value=build();if(value.model!=7||!value.context.equals("ctx"))throw new AssertionError();System.out.println(events+":ok");}}catch(Throwable e){{System.out.println(events+":"+e.getClass().getSimpleName()+":"+(e==failure)+":"+(e==checked));}}}}
}}"#)).unwrap();
    let run = |tool: &str, args: &[&str]| {
        let out = std::process::Command::new(format!("{home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{tool}:{}\n{source}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    };
    run(
        "javac",
        &[
            "--release",
            if original { "25" } else { "17" },
            "SequenceRuntime.java",
        ],
    );
    if !runtime {
        return;
    }
    let mut after = Vec::new();
    for mode in 0..7 {
        after.extend(run(
            "java",
            &["-Xverify:all", "SequenceRuntime", &mode.to_string()],
        ));
    }
    assert_eq!(
        String::from_utf8(after.clone()).unwrap(),
        "AMXKC:ok\nAMXK:NullPointerException:false:false\nAM:IllegalArgumentException:true:false\nAMX:IllegalArgumentException:true:false\nAMXK:IOException:false:true\nAMXKC:IllegalArgumentException:true:false\nA:OutOfMemoryError:false:false\n"
    );
    if original {
        std::fs::write(dir.join("Original.java"),r#"import java.lang.classfile.*;import java.lang.constant.*;import java.nio.file.*;
public class Original {public static void main(String[]args)throws Exception{ClassFile cf=ClassFile.of();ClassDesc self=ClassDesc.of("SequenceRuntime"),outer=ClassDesc.of("SequenceRuntime$Outer");Path path=Path.of("SequenceRuntime.class");Files.write(path,cf.transformClass(cf.parse(path),(cb,element)->{if(element instanceof MethodModel mm&&mm.methodName().equalsString("build"))cb.transformMethod(mm,(mb,me)->{if(me instanceof CodeModel)mb.withCode(c->{c.new_(outer).astore(0);c.invokestatic(self,"model",MethodTypeDesc.ofDescriptor("()I")).istore(1);c.invokestatic(self,"context",MethodTypeDesc.ofDescriptor("()Ljava/lang/String;")).astore(2);c.ldc("context").astore(3);c.aload(2).aload(3).invokestatic(self,"check",MethodTypeDesc.ofDescriptor("(Ljava/lang/String;Ljava/lang/String;)V"));c.aload(0).iload(1).aload(2).invokespecial(outer,"<init>",MethodTypeDesc.ofDescriptor("(ILjava/lang/String;)V"));c.aload(0).areturn();});else mb.with(me);});else cb.with(element);}));}}
"#).unwrap();
        run("javac", &["--release", "25", "Original.java"]);
        run("java", &["Original"]);
        let mut before = Vec::new();
        for mode in 0..7 {
            before.extend(run(
                "java",
                &["-Xverify:all", "SequenceRuntime", &mode.to_string()],
            ));
        }
        assert_eq!(before, after);
    }
    std::fs::write(dir.join("runtime-traces.txt"), after).unwrap();
}

/// Compile newly representable legacy fixtures to check block scope, overloads,
/// definite assignment and typed captures against Java17's actual compiler.
pub(in crate::object_plan) fn compile_fixture(source: &str, declarations: &str) {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hash);
    declarations.hash(&mut hash);
    let dir = std::env::temp_dir().join(format!(
        "decomphify-sequence-compile-{}-{}",
        std::process::id(),
        hash.finish()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Fixture.java"),
        format!("public class Fixture {{{declarations}\n{source}\n}}\n"),
    )
    .unwrap();
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let out = std::process::Command::new(format!("{home}/bin/javac"))
        .args(["--release", "17", "Fixture.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&out.stderr)
    );
}
