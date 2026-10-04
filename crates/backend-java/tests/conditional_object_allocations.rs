//! Original allocation-before-null-selection bytecode and source equivalence.
#![allow(clippy::unwrap_used)]
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{CompareOp, Constant, InvokeKind, ManagedInsn},
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
    fn class(&self, class: ClassRef) -> Option<String> {
        Some(
            if class.index() == 0 {
                "Big"
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
    fn method(&self, method: MethodRef) -> Option<JavaMethod> {
        if method.index() == 2 {
            return Some(JavaMethod {
                owner: "Probe".into(),
                name: "effect".into(),
                parameters: vec![],
                return_type: JavaType::Class("java.lang.Object".into()),
                is_static: true,
                throws: vec![],
            });
        }
        Some(JavaMethod {
            owner: if self.mode == 5 && method.index() == 1 {
                "Other"
            } else {
                "Big"
            }
            .into(),
            name: "<init>".into(),
            parameters: if method.index() == 0 {
                vec![]
            } else {
                vec![JavaType::Class("java.lang.Object".into())]
            },
            return_type: JavaType::Void,
            is_static: false,
            throws: if self.mode == 7 {
                vec!["java.io.IOException".into()]
            } else {
                vec![]
            },
        })
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        if self.mode == 4 {
            Some(2)
        } else if self.mode == 10 {
            None
        } else {
            Some(1)
        }
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

fn fixture(mode: u8) -> (MethodIr<SsaForm>, ValueId) {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let (_, parameter) = def(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(1)),
        |value| ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![value],
            effects: EffectFlags::NONE,
        },
    );
    let (allocation, object) = def(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(0)),
        |result| ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        },
    );
    let (zero, null) = def(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(1)),
        |result| ManagedInsn::Const {
            result,
            value: Constant::Null,
        },
    );
    let (comparison, condition) = def(&mut insns, &mut values, ManagedType::Boolean, |result| {
        ManagedInsn::Compare {
            op: CompareOp::Equal,
            result,
            lhs: parameter,
            rhs: null,
        }
    });
    let branch = insns.insert(ManagedInsn::Return { value: None });
    let empty = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![object],
    });
    let supplied = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![object, if mode == 6 { null } else { parameter }],
    });
    let jump = insns.insert(ManagedInsn::Return { value: None });
    let ret = insns.insert(ManagedInsn::Return {
        value: Some(object),
    });
    let mut cfg = Cfg::new(allocation);
    let z = cfg.add_block(zero);
    let c = cfg.add_block(comparison);
    let b = cfg.add_block(branch);
    let a = cfg.add_block(empty);
    let d = cfg.add_block(supplied);
    let j = cfg.add_block(jump);
    let end = cfg.add_block(ret);
    cfg.add_edge(cfg.entry(), z, EdgeKind::Unconditional)
        .unwrap();
    if mode == 1 {
        let (effect, _) = def(
            &mut insns,
            &mut values,
            ManagedType::Reference(ClassRef::new(1)),
            |result| ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(result),
                method: MethodRef::new(2),
                args: vec![],
            },
        );
        let side = cfg.add_block(effect);
        cfg.add_edge(z, side, EdgeKind::Unconditional).unwrap();
        cfg.add_edge(side, c, EdgeKind::Unconditional).unwrap();
    } else {
        cfg.add_edge(z, c, EdgeKind::Unconditional).unwrap();
    }
    cfg.add_edge(c, b, EdgeKind::Unconditional).unwrap();
    let yes = cfg.add_edge(b, a, EdgeKind::BranchTrue).unwrap();
    let no = cfg.add_edge(b, d, EdgeKind::BranchFalse).unwrap();
    cfg.add_edge(a, j, EdgeKind::Unconditional).unwrap();
    let forward = cfg.add_edge(j, end, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(d, end, EdgeKind::Unconditional).unwrap();
    if mode == 2 || mode == 3 {
        cfg.add_edge(
            if mode == 2 { cfg.entry() } else { a },
            end,
            EdgeKind::Exception("java.lang.Throwable".into()),
        )
        .unwrap();
    }
    if mode == 8 {
        insns.insert(ManagedInsn::Opaque {
            family_tag: 44,
            reads: vec![object],
            writes: vec![],
            effects: EffectFlags::NONE,
        });
    }
    if mode == 9 {
        let (_, v) = def(
            &mut insns,
            &mut values,
            ManagedType::Reference(ClassRef::new(0)),
            |result| ManagedInsn::Phi {
                result,
                inputs: vec![(forward, object)],
            },
        );
        let _ = v;
    }
    *insns.get_mut(branch).unwrap() = ManagedInsn::Branch {
        condition: Some(condition),
        then_edge: yes,
        else_edge: Some(no),
    };
    *insns.get_mut(jump).unwrap() = ManagedInsn::Branch {
        condition: None,
        then_edge: forward,
        else_edge: None,
    };
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        parameter,
    )
}

fn emitted(mode: u8) -> Result<String, backend_java::Unsupported> {
    let (ir, parameter) = fixture(mode);
    let method = JavaMethod {
        owner: "Emitted".into(),
        name: "make".into(),
        parameters: vec![JavaType::Class("java.lang.Object".into())],
        return_type: JavaType::Class("Big".into()),
        is_static: true,
        throws: vec![],
    };
    match backend_java::emit_method(
        &ir,
        &method,
        &ParameterBindings {
            receiver: None,
            parameters: vec![parameter],
            names: vec![],
        },
        &R { mode },
    ) {
        backend_java::SourceResult::Complete { source, .. } => Ok(source),
        backend_java::SourceResult::Unsupported(error) => Err(error),
    }
}

#[test]
fn conditional_initializers_require_complete_pure_region_and_genuine_metadata() {
    let source = emitted(0).unwrap();
    assert_eq!(source.matches("new Big(").count(), 2);
    assert!(source.contains("? new Big() : new Big((java.lang.Object)"));
    for mode in 1..=10 {
        assert!(emitted(mode).is_err(), "unsound mode {mode} accepted");
    }
}

fn run(command: &mut Command) -> String {
    let out = command.output().unwrap();
    assert!(
        out.status.success(),
        "command {command:?}\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn original_new_before_branch_matches_emitted_source_on_jdk25_including_heap_oom() {
    let jdk = std::env::var_os("DECOMPHIFY_JDK25_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1"));
    assert!(
        jdk.join("bin/javac").is_file(),
        "Java25 differential proof requires genuine JDK25"
    );
    let dir = std::env::temp_dir().join(format!(
        "decomphify-conditional-original-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Emitted.java"),
        format!("public class Emitted {{ {} }}", emitted(0).unwrap()),
    )
    .unwrap();
    for name in ["Builder.java", "Harness.java", "Probe.java"] {
        std::fs::write(
            dir.join(name),
            std::fs::read(format!("tests/fixtures/conditional_allocation/{name}")).unwrap(),
        )
        .unwrap();
    }
    // JDK25 removed its internal ASM; build verifier-valid legacy bytecode with JDK17.
    let builder_jdk = PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11");
    run(Command::new(builder_jdk.join("bin/javac"))
        .args([
            "--add-exports",
            "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
        ])
        .arg("-d")
        .arg(&dir)
        .arg(dir.join("Builder.java")));
    run(Command::new(builder_jdk.join("bin/java"))
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
            dir.join("Emitted.java"),
            dir.join("Harness.java"),
        ]));
    for mode in [
        "normal-null",
        "normal-reference",
        "classfail-null",
        "classfail-reference",
        "ctorfail-null",
        "ctorfail-reference",
        "oom-null",
        "oom-reference",
    ] {
        let oracle = run(Command::new(jdk.join("bin/java"))
            .args(["-Xverify:all", "-Xmx16m", "-cp"])
            .arg(&dir)
            .args(["Harness", "Original", mode]));
        let recovered = run(Command::new(jdk.join("bin/java"))
            .args(["-Xverify:all", "-Xmx16m", "-cp"])
            .arg(&dir)
            .args(["Harness", "Emitted", mode]));
        assert_eq!(
            oracle, recovered,
            "original allocation event mismatch: {mode}"
        );
    }
    std::fs::write(
        dir.join("proof.txt"),
        "8 fresh JVM mode pairs matched, Java25, -Xverify:all, genuine -Xmx16m allocation OOM",
    )
    .unwrap();
}
