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
    fn class(&self, c: ClassRef) -> Option<String> {
        Some(
            match c.index() {
                0 => "Big",
                2 => "Child",
                _ => "java.lang.Object",
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
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        if m.index() == 4 || m.index() == 5 {
            return Some(JavaMethod {
                owner: if self.mode == 7 && m.index() == 4 {
                    "Other"
                } else {
                    "Child"
                }
                .into(),
                name: if m.index() == 4 { "<init>" } else { "touch" }.into(),
                parameters: if m.index() == 4 {
                    vec![]
                } else {
                    vec![JavaType::Class("java.lang.Object".into())]
                },
                return_type: if m.index() == 4 {
                    JavaType::Void
                } else if self.mode == 12 {
                    JavaType::Class("java.lang.Object".into())
                } else {
                    JavaType::Class("Child".into())
                },
                is_static: false,
                throws: vec![],
            });
        }
        Some(if m.index() == 0 {
            JavaMethod {
                owner: "Big".into(),
                name: "<init>".into(),
                parameters: vec![JavaType::Class("java.lang.Object".into()); 2],
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            }
        } else {
            JavaMethod {
                owner: "Probe".into(),
                name: match m.index() {
                    1 => "first",
                    2 => "next",
                    _ => "extra",
                }
                .into(),
                parameters: if m.index() == 3 {
                    vec![JavaType::Class("java.lang.Object".into())]
                } else {
                    vec![]
                },
                return_type: JavaType::Class("java.lang.Object".into()),
                is_static: true,
                throws: vec![],
            }
        })
    }
    fn discarded_invoke_source_target(&self, reference: MethodRef) -> bool {
        reference.index() == 5 && self.mode != 12
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        if self.mode == 5 { None } else { Some(1) }
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
    let (f, first) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(1)),
        |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(1),
            args: vec![],
        },
    );
    let (child_new, child) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(2)),
        |result| ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(2),
        },
    );
    let child_init = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(4),
        args: vec![child],
    });
    let before = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Virtual,
        result: None,
        method: MethodRef::new(5),
        args: vec![child, first],
    });
    let (n, selected) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(1)),
        |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(2),
            args: vec![],
        },
    );
    let (z, null) = def(&mut i, &mut v, ManagedType::Null, |result| {
        ManagedInsn::Const {
            result,
            value: Constant::Null,
        }
    });
    let (c, condition) = def(&mut i, &mut v, ManagedType::Boolean, |result| {
        ManagedInsn::Compare {
            op: CompareOp::Equal,
            result,
            lhs: selected,
            rhs: null,
        }
    });
    let b = i.insert(ManagedInsn::Return { value: None });
    let (again, next) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(1)),
        |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(2),
            args: vec![],
        },
    );
    let (e, extra) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(1)),
        |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(3),
            args: vec![next],
        },
    );
    let (p, phi) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(1)),
        |result| ManagedInsn::Phi {
            result,
            inputs: vec![],
        },
    );
    let after = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Virtual,
        result: None,
        method: MethodRef::new(5),
        args: vec![child, phi],
    });
    let init = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: if mode == 11 {
            vec![obj, phi, first]
        } else {
            vec![obj, first, phi]
        },
    });
    let ret = i.insert(ManagedInsn::Return { value: Some(obj) });
    let prelude = if mode >= 13 {
        Some(def(&mut i, &mut v, ManagedType::Boolean, |result| {
            ManagedInsn::Const {
                result,
                value: Constant::Boolean(false),
            }
        }))
    } else {
        None
    };
    let mut cfg = Cfg::new(prelude.map_or(a, |p| p.0));
    let root = if prelude.is_some() {
        cfg.add_block(a)
    } else {
        cfg.entry()
    };
    let fb = cfg.add_block(f);
    let cn = cfg.add_block(child_new);
    let ci = cfg.add_block(child_init);
    let bf = cfg.add_block(before);
    let af = cfg.add_block(after);
    let nb = cfg.add_block(n);
    let zb = cfg.add_block(z);
    let cb = cfg.add_block(c);
    let bb = cfg.add_block(b);
    let ab = cfg.add_block(again);
    let eb = cfg.add_block(e);
    let pb = cfg.add_block(p);
    let ib = cfg.add_block(init);
    let rb = cfg.add_block(ret);
    for (x, y) in [
        (root, fb),
        (fb, cn),
        (cn, ci),
        (ci, bf),
        (bf, nb),
        (nb, zb),
        (zb, cb),
        (cb, bb),
        (ab, eb),
        (pb, af),
        (af, ib),
        (ib, rb),
    ] {
        cfg.add_edge(x, y, EdgeKind::Unconditional).unwrap();
    }
    let yes = cfg.add_edge(bb, pb, EdgeKind::BranchTrue).unwrap();
    let no = cfg.add_edge(bb, ab, EdgeKind::BranchFalse).unwrap();
    let end = cfg.add_edge(eb, pb, EdgeKind::Unconditional).unwrap();
    *i.get_mut(b).unwrap() = ManagedInsn::Branch {
        condition: Some(condition),
        then_edge: yes,
        else_edge: Some(no),
    };
    *i.get_mut(p).unwrap() = ManagedInsn::Phi {
        result: phi,
        inputs: vec![(yes, null), (end, extra)],
    };
    if mode == 1 {
        cfg.add_edge(
            cfg.entry(),
            rb,
            EdgeKind::Exception("java.lang.Throwable".into()),
        )
        .unwrap();
    }
    if mode == 2 {
        i.insert(ManagedInsn::Opaque {
            family_tag: 9,
            reads: vec![phi],
            writes: vec![],
            effects: EffectFlags::NONE,
        });
    }
    if mode == 3 {
        i.insert(ManagedInsn::Opaque {
            family_tag: 9,
            reads: vec![obj],
            writes: vec![],
            effects: EffectFlags::NONE,
        });
    }
    if mode == 4 {
        i.insert(ManagedInsn::Branch {
            condition: Some(condition),
            then_edge: yes,
            else_edge: Some(no),
        });
    }
    if mode == 6 {
        i.insert(ManagedInsn::Opaque {
            family_tag: 77,
            reads: vec![child],
            writes: vec![],
            effects: EffectFlags::NONE,
        });
    }
    if mode == 8 {
        i.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(4),
            args: vec![child],
        });
    }
    if mode == 9 {
        cfg.add_edge(cn, rb, EdgeKind::Exception("java.lang.Throwable".into()))
            .unwrap();
    }
    if mode == 10 {
        *i.get_mut(before).unwrap() = ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(4),
            args: vec![child],
        };
    }
    if mode >= 13 {
        let original = root;
        let (_, selected) = prelude.unwrap();
        let (true_insn, true_value) = def(&mut i, &mut v, ManagedType::Boolean, |result| {
            ManagedInsn::Const {
                result,
                value: Constant::Boolean(true),
            }
        });
        let outer = i.insert(ManagedInsn::Return { value: None });
        let entry = cfg.add_block(outer);
        cfg.add_edge(cfg.entry(), entry, EdgeKind::Unconditional)
            .unwrap();
        let second_insn = i.insert(ManagedInsn::Return { value: None });
        let second = cfg.add_block(second_insn);
        let goto_insn = i.insert(ManagedInsn::Return { value: None });
        let tail = cfg.add_block(goto_insn);
        let (null_insn, null_value) = def(&mut i, &mut v, ManagedType::Null, |result| {
            ManagedInsn::Const {
                result,
                value: Constant::Null,
            }
        });
        let null_block = cfg.split_block(entry, null_insn).unwrap();
        cfg.split_block(null_block, true_insn).unwrap();
        let yes = cfg.add_edge(entry, tail, EdgeKind::BranchTrue).unwrap();
        let no = cfg.add_edge(entry, second, EdgeKind::BranchFalse).unwrap();
        *i.get_mut(outer).unwrap() = ManagedInsn::Branch {
            condition: Some(selected),
            then_edge: yes,
            else_edge: Some(no),
        };
        let yes = cfg
            .add_edge(
                second,
                if mode == 15 { cn } else { original },
                EdgeKind::BranchTrue,
            )
            .unwrap();
        let no = cfg.add_edge(second, tail, EdgeKind::BranchFalse).unwrap();
        *i.get_mut(second_insn).unwrap() = ManagedInsn::Branch {
            condition: Some(true_value),
            then_edge: yes,
            else_edge: Some(no),
        };
        *i.get_mut(goto_insn).unwrap() = ManagedInsn::Return {
            value: Some(null_value),
        };

        assert!(decomp_structure::structure_extended(&cfg).is_err());
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
fn argument_region_exact_calls_and_negative_scope() {
    let source = emitted(13).unwrap();
    println!("{source}");
    assert!(source.contains("switch (0)"));
    assert_eq!(source.matches("Probe.next(").count(), 2);
    let phi_first = emitted(14).unwrap();
    assert!(phi_first.contains("switch (0)"));
    for mode in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 15] {
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
fn original_argument_diamond_runtime_and_heap_oom() {
    let jdk = PathBuf::from("/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1");
    let builder = PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11");
    let dir = std::env::temp_dir().join(format!(
        "decomphify-nested-argument-diamond-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Emitted.java"),
        format!("public class Emitted {{ {} }}", emitted(13).unwrap()),
    )
    .unwrap();
    for name in ["Builder.java", "Probe.java", "Harness.java"] {
        std::fs::copy(
            format!("tests/fixtures/nested_argument_regions/{name}"),
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
    for (runtime, release) in [(&builder, "17"), (&jdk, "25")] {
        run(Command::new(runtime.join("bin/javac"))
            .args(["--release", release, "-cp"])
            .arg(&dir)
            .arg("-d")
            .arg(&dir)
            .args([
                dir.join("Probe.java"),
                dir.join("Harness.java"),
                dir.join("Emitted.java"),
            ]));
        for mode in [
            "normal",
            "null",
            "alternate",
            "F",
            "N1",
            "N2",
            "E",
            "T",
            "I",
            "U1",
            "U2",
            "class",
            "innerclass",
            "oom",
            "inneroom",
        ] {
            let original = run(Command::new(runtime.join("bin/java"))
                .args(["-Xverify:all", "-XX:+UseSerialGC", "-Xmx16m", "-cp"])
                .arg(&dir)
                .args(["Harness", "original", mode]));
            let emitted = run(Command::new(runtime.join("bin/java"))
                .args(["-Xverify:all", "-XX:+UseSerialGC", "-Xmx16m", "-cp"])
                .arg(&dir)
                .args(["Harness", "emitted", mode]));
            assert_eq!(original, emitted, "mode {mode}");
        }
    }
}
