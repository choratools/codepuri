//! A post-chain NEW must precede its selected argument Phi and effects.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, constructors::*,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{Constant, InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{Value, ValueId},
};
struct R {
    missing: bool,
}
impl JavaResolver for R {
    fn class(&self, c: ClassRef) -> Option<String> {
        if self.missing && c.index() == 1 {
            None
        } else {
            Some(if c.index() == 0 { "Fixture" } else { "Payload" }.into())
        }
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture".into(),
            name: "value".into(),
            ty: JavaType::Class("Payload".into()),
            is_static: false,
        })
    }
    fn field_access(&self, _: FieldRef) -> Option<u32> {
        Some(0x10)
    }
    fn method_access(&self, m: MethodRef) -> Option<u32> {
        Some(if m.index() == 3 { 9 } else { 1 })
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        if m.index() == 3 {
            Some(JavaMethod {
                owner: "Fixture".into(),
                name: "select".into(),
                parameters: vec![JavaType::Boolean],
                return_type: JavaType::Boolean,
                is_static: true,
                throws: vec![],
            })
        } else {
            self.initializer(m).map(|x| x.method)
        }
    }
}
impl ConstructorResolver for R {
    fn initializer(&self, m: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: match m.index() {
                    0 => "Fixture",
                    1 => "java.lang.Object",
                    2 => "Payload",
                    _ => return None,
                }
                .into(),
                name: "<init>".into(),
                parameters: match m.index() {
                    0 => vec![JavaType::Boolean],
                    2 => vec![JavaType::Int],
                    _ => vec![],
                },
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: None,
        })
    }
}
fn def(
    insns: &mut InsnArena,
    values: &mut ValueArena,
    ty: ManagedType,
    f: impl FnOnce(ValueId) -> ManagedInsn,
) -> (managed_ir::value::InsnId, ValueId) {
    let mut value = None;
    let id = insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ty));
        value = Some(v);
        f(v)
    });
    (id, value.unwrap())
}
fn input(i: &mut InsnArena, v: &mut ValueArena, t: ManagedType) -> ValueId {
    def(i, v, t, |x| ManagedInsn::Opaque {
        family_tag: 0,
        reads: vec![],
        writes: vec![x],
        effects: EffectFlags::NONE,
    })
    .1
}
fn body(mode: u8) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut i = InsnArena::default();
    let mut v = ValueArena::default();
    let this = input(&mut i, &mut v, ManagedType::Reference(ClassRef::new(0)));
    let flag = input(&mut i, &mut v, ManagedType::Boolean);
    let chain = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![this],
    });
    let (new, obj) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(1)),
        |result| ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(1),
        },
    );
    let (selector, condition) = def(&mut i, &mut v, ManagedType::Boolean, |result| {
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(3),
            args: vec![flag],
        }
    });
    let branch = i.insert(ManagedInsn::Return { value: None });
    let (a, av) = def(&mut i, &mut v, ManagedType::Int, |result| {
        ManagedInsn::Const {
            result,
            value: Constant::Int(23),
        }
    });
    let (b, bv) = def(&mut i, &mut v, ManagedType::Int, |result| {
        ManagedInsn::Const {
            result,
            value: Constant::Int(12),
        }
    });
    let (phi, pv) = def(&mut i, &mut v, ManagedType::Int, |result| {
        ManagedInsn::Phi {
            result,
            inputs: vec![],
        }
    });
    let init = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(2),
        args: vec![obj, pv],
    });
    let store = i.insert(ManagedInsn::FieldPut {
        object: Some(this),
        field: FieldRef::new(0),
        value: obj,
    });
    let ret = i.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(chain);
    let nb = cfg.add_block(new);
    let sb = cfg.add_block(selector);
    let cb = cfg.add_block(branch);
    let ab = cfg.add_block(a);
    let bb = cfg.add_block(b);
    let pb = cfg.add_block(phi);
    let ib = cfg.add_block(init);
    let fb = cfg.add_block(store);
    let rb = cfg.add_block(ret);
    for (x, y) in [
        (cfg.entry(), nb),
        (nb, sb),
        (sb, cb),
        (pb, ib),
        (ib, fb),
        (fb, rb),
    ] {
        cfg.add_edge(x, y, EdgeKind::Unconditional).unwrap();
    }
    let yes = cfg.add_edge(cb, ab, EdgeKind::BranchTrue).unwrap();
    let no = cfg.add_edge(cb, bb, EdgeKind::BranchFalse).unwrap();
    let ae = cfg.add_edge(ab, pb, EdgeKind::Unconditional).unwrap();
    let be = cfg.add_edge(bb, pb, EdgeKind::Unconditional).unwrap();
    *i.get_mut(branch).unwrap() = ManagedInsn::Branch {
        condition: Some(condition),
        then_edge: yes,
        else_edge: Some(no),
    };
    *i.get_mut(phi).unwrap() = ManagedInsn::Phi {
        result: pv,
        inputs: vec![(ae, av), (be, bv)],
    };
    if mode == 1 {
        cfg.redirect_edge(ae, cb).unwrap();
    } // A real cycle, not a shared join.
    if mode == 2 {
        cfg.add_edge(nb, pb, EdgeKind::Exception("java.lang.Exception".into()))
            .unwrap();
    }
    if mode == 3 {
        *i.get_mut(init).unwrap() = ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(2),
            args: vec![flag, pv],
        };
    }
    if mode == 4 {
        *i.get_mut(chain).unwrap() = ManagedInsn::FieldPut {
            object: Some(this),
            field: FieldRef::new(0),
            value: obj,
        };
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, i, v),
        ParameterBindings {
            receiver: Some(this),
            parameters: vec![flag],
            names: vec!["p0".into()],
        },
    )
}
#[test]
fn original_allocation_precedes_selector_phi_then_final_store() {
    let (ir, bindings) = body(0);
    let plan = plan_constructor(
        &ir,
        MethodRef::new(0),
        "java.lang.Object",
        &bindings,
        &R { missing: false },
    )
    .unwrap();
    let source = plan.source;
    assert!(source.contains("new Payload"));
    assert_eq!(source.matches("this.value =").count(), 1);
    let java = format!(
        r#"public class Fixture {{ static String log=""; static final RuntimeException ERROR=new RuntimeException(); final Payload value; {source}
 static boolean select(boolean flag) {{ log+="S"; if(System.getProperty("mode").equals("throw")) throw ERROR; return flag; }}
 static class Payload {{ static {{ log+="A"; if(System.getProperty("mode").equals("init")) throw ERROR; if(System.getProperty("mode").equals("oom")) {{ byte[] tooBig=new byte[32*1024*1024]; log+=tooBig.length; }} }} final int number; Payload(int n) {{ log+=n; number=n; }} }}
 static Payload original(boolean flag) {{ return new Payload(select(flag)?23:12); }}
 public static void main(String[]args) {{ Throwable error=null; try {{ Payload p=args[0].equals("generated")?new Fixture(Boolean.parseBoolean(args[1])).value:original(Boolean.parseBoolean(args[1])); log+="="+p.number; }} catch(Throwable t) {{ error=t; }} System.out.print(log+":"+(error==null?"normal":error==ERROR?"same":error instanceof ExceptionInInitializerError && error.getCause()==ERROR?"initSame":error.getClass().getName())); }} }}"#
    );
    for home in [
        "/home/jtjisgod/tools/jdk-17.0.13+11",
        "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1",
    ] {
        let root = std::env::temp_dir().join(format!(
            "ctor-argument-phi-{}-{}",
            std::process::id(),
            home.contains("jdk25")
        ));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("Fixture.java");
        std::fs::write(&file, &java).unwrap();
        let compile = std::process::Command::new(format!("{home}/bin/javac"))
            .arg(file)
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        for mode in ["normal", "throw", "init", "oom"] {
            for flag in ["true", "false"] {
                let run = |who| {
                    std::process::Command::new(format!("{home}/bin/java"))
                        .args([
                            "-Xmx16m",
                            &format!("-Dmode={mode}"),
                            "-cp",
                            root.to_str().unwrap(),
                            "Fixture",
                            who,
                            flag,
                        ])
                        .output()
                        .unwrap()
                };
                let a = run("original");
                let b = run("generated");
                assert!(a.status.success() && b.status.success());
                assert_eq!(a.stdout, b.stdout, "{mode} {flag}");
                let trace = String::from_utf8_lossy(&b.stdout);
                if mode == "oom" {
                    assert_eq!(trace, "A:java.lang.OutOfMemoryError");
                }
                if mode == "init" {
                    assert_eq!(trace, "A:initSame");
                }
            }
        }
    }
}
#[test]
fn missing_type_cycle_handler_wrong_pair_and_early_this_stay_rejected() {
    for mode in 0..=4 {
        let (ir, b) = body(mode);
        assert!(
            plan_constructor(
                &ir,
                MethodRef::new(0),
                "java.lang.Object",
                &b,
                &R { missing: mode == 0 }
            )
            .is_err(),
            "mode{mode}"
        );
    }
}
