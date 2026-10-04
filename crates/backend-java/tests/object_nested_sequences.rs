//! Original allocation-before-null-selection bytecode and source equivalence.
#![allow(clippy::unwrap_used)]
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{Constant, InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};
use std::{path::PathBuf, process::Command};

struct R {
    mode: u8,
}
impl JavaResolver for R {
    fn class(&self, c: ClassRef) -> Option<String> {
        Some(
            match c.index() {
                0 => "Big",
                2 => "Inner",
                3 => "java.lang.String",
                _ => "java.lang.Object",
            }
            .into(),
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        Some("late".encode_utf16().collect())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        Some(match m.index() {
            0 => JavaMethod {
                owner: "Big".into(),
                name: "<init>".into(),
                parameters: vec![
                    JavaType::Class("java.lang.Object".into()),
                    JavaType::Class("java.lang.String".into()),
                    JavaType::Class("java.lang.Object".into()),
                ],
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            4 => JavaMethod {
                owner: if self.mode == 2 { "Other" } else { "Inner" }.into(),
                name: "<init>".into(),
                parameters: vec![],
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            n => JavaMethod {
                owner: "Probe".into(),
                name: match n {
                    1 => "touch",
                    2 => "value",
                    _ => "file",
                }
                .into(),
                parameters: if n == 3 {
                    vec![]
                } else {
                    vec![JavaType::Class("Inner".into())]
                },
                return_type: if n == 1 {
                    JavaType::Void
                } else {
                    JavaType::Class("java.lang.Object".into())
                },
                is_static: true,
                throws: vec![],
            },
        })
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(1)
    }
}

fn def(
    insns: &mut InsnArena,
    values: &mut ValueArena,
    ty: ManagedType,
    f: impl FnOnce(ValueId) -> ManagedInsn,
) -> (InsnId, ValueId) {
    let mut value = None;
    let id = insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ty));
        value = Some(v);
        f(v)
    });
    (id, value.unwrap())
}

fn emitted(mode: u8) -> Result<String, backend_java::Unsupported> {
    let mut i = InsnArena::default();
    let mut v = ValueArena::default();
    let (a, obj) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(0)),
        |result| ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        },
    );
    let (n, inner) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(2)),
        |result| ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(2),
        },
    );
    let ni = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(4),
        args: vec![inner],
    });
    let touch = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(1),
        args: vec![inner],
    });
    let (val, first) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(1)),
        |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(2),
            args: vec![inner],
        },
    );
    let (file, third) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(1)),
        |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(3),
            args: vec![],
        },
    );
    let (literal, second) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(3)),
        |result| ManagedInsn::Const {
            result,
            value: Constant::String(StringRef::new(0)),
        },
    );
    let init = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![obj, first, second, third],
    });
    let ret = i.insert(ManagedInsn::Return { value: Some(obj) });
    let mut cfg = Cfg::new(a);
    let mut previous = cfg.entry();
    let mut inner_block = None;
    for id in [n, ni, touch, val, file, literal, init, ret] {
        let next = cfg.add_block(id);
        cfg.add_edge(previous, next, EdgeKind::Unconditional)
            .unwrap();
        if id == n {
            inner_block = Some(next);
        }
        previous = next;
    }
    if mode == 1 {
        cfg.add_edge(
            inner_block.unwrap(),
            previous,
            EdgeKind::Exception("java.lang.Throwable".into()),
        )
        .unwrap();
    }
    if mode == 3 {
        i.insert(ManagedInsn::Opaque {
            family_tag: 9,
            reads: vec![inner],
            writes: vec![],
            effects: EffectFlags::NONE,
        });
    }
    if mode == 4 {
        i.insert(ManagedInsn::Opaque {
            family_tag: 9,
            reads: vec![obj],
            writes: vec![],
            effects: EffectFlags::NONE,
        });
    }
    let ir = MethodIr::<SsaForm>::from_parts(cfg, i, v);
    let m = JavaMethod {
        owner: "Emitted".into(),
        name: "make".into(),
        parameters: vec![],
        return_type: JavaType::Class("Big".into()),
        is_static: true,
        throws: vec![],
    };
    match backend_java::emit_method(
        &ir,
        &m,
        &ParameterBindings {
            receiver: None,
            parameters: vec![],
            names: vec![],
        },
        &R { mode },
    ) {
        backend_java::SourceResult::Complete { source, .. } => Ok(source),
        backend_java::SourceResult::Unsupported(e) => Err(e),
    }
}
#[test]
fn nested_statement_sequence_preserves_real_new_and_denies_unproved_escapes() {
    let source = emitted(0).unwrap();
    assert!(source.contains("switch (0)"));
    assert_eq!(source.matches("new Inner(").count(), 1);
    for mode in 1..=4 {
        assert!(emitted(mode).is_err(), "mode {mode}");
    }
}

fn run(command: &mut Command) -> String {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
#[test]
fn nested_statement_sequence_original_jvm_runtime() {
    let jdk = PathBuf::from("/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1");
    let builder = PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11");
    let dir =
        std::env::temp_dir().join(format!("decomphify-nested-sequence-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Emitted.java"),
        format!("public class Emitted {{ {} }}", emitted(0).unwrap()),
    )
    .unwrap();
    for name in ["Builder.java", "Probe.java", "Harness.java"] {
        std::fs::copy(
            format!("tests/fixtures/nested_sequences/{name}"),
            dir.join(name),
        )
        .unwrap();
    }
    run(Command::new(builder.join("bin/javac"))
        .args([
            "--add-exports",
            "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
            "-d",
        ])
        .arg(&dir)
        .arg(dir.join("Builder.java")));
    run(Command::new(builder.join("bin/java"))
        .args([
            "--add-exports",
            "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
            "-cp",
        ])
        .arg(&dir)
        .arg("Builder")
        .arg(&dir));
    run(Command::new(jdk.join("bin/javac"))
        .args(["--release", "25", "-cp"])
        .arg(&dir)
        .arg("-d")
        .arg(&dir)
        .args([
            dir.join("Probe.java"),
            dir.join("Harness.java"),
            dir.join("Emitted.java"),
        ]));
    for mode in [
        "normal", "OC", "IC", "IT", "A", "V", "F", "OT", "outerOOM", "innerOOM",
    ] {
        let original = run(Command::new(jdk.join("bin/java"))
            .args(["-Xverify:all", "-Xmx16m", "-cp"])
            .arg(&dir)
            .args(["Harness", "original", mode]));
        let emitted = run(Command::new(jdk.join("bin/java"))
            .args(["-Xverify:all", "-Xmx16m", "-cp"])
            .arg(&dir)
            .args(["Harness", "emitted", mode]));
        assert_eq!(original, emitted, "mode {mode}");
    }
}
