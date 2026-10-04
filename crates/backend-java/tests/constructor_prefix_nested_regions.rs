//! Nested and sequential Java25 pre-constructor selectors keep exact CFG edges.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, constructors::*,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{CompareOp, Constant, InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};
use std::process::Command;
struct R;
impl JavaResolver for R {
    fn class(&self, c: ClassRef) -> Option<String> {
        Some(if c.index() == 0 { "Fixture" } else { "Base" }.into())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method_access(&self, m: MethodRef) -> Option<u32> {
        Some(if m.index() < 2 { 1 } else { 9 })
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        if m.index() < 2 {
            return self.initializer(m).map(|i| i.method);
        }
        Some(JavaMethod {
            owner: "Trace".into(),
            name: match m.index() {
                2 => "get1",
                3 => "fallback1",
                4 => "get2",
                5 => "fallback2",
                _ => "token",
            }
            .into(),
            parameters: vec![],
            return_type: JavaType::Int,
            is_static: true,
            throws: vec![],
        })
    }
}
impl ConstructorResolver for R {
    fn constructor_source_level(&self) -> u16 {
        25
    }
    fn initializer(&self, m: MethodRef) -> Option<Initializer> {
        if m.index() > 1 {
            return None;
        }
        Some(Initializer {
            method: JavaMethod {
                owner: if m.index() == 0 { "Fixture" } else { "Base" }.into(),
                name: "<init>".into(),
                parameters: if m.index() == 0 {
                    vec![JavaType::Boolean]
                } else {
                    vec![JavaType::Int, JavaType::Int, JavaType::Int]
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
    i: &mut InsnArena,
    v: &mut ValueArena,
    ty: ManagedType,
    f: impl FnOnce(ValueId) -> ManagedInsn,
) -> (InsnId, ValueId) {
    let mut result = None;
    let id = i.insert_with_key(|id| {
        let value = v.insert(Value::new(id, ty));
        result = Some(value);
        f(value)
    });
    (id, result.unwrap())
}
fn body(mode: u8) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut i = InsnArena::default();
    let mut v = ValueArena::default();
    let (_, this) = def(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(0)),
        |result| ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![result],
            effects: EffectFlags::NONE,
        },
    );
    let (_, present) = def(&mut i, &mut v, ManagedType::Boolean, |result| {
        ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![result],
            effects: EffectFlags::NONE,
        }
    });
    let (token_id, token) = def(&mut i, &mut v, ManagedType::Int, |result| {
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(6),
            args: vec![],
        }
    });
    let mut cfg = Cfg::new(token_id);
    let mut previous = cfg.entry();
    let mut args = vec![this, token];
    for stage in 0..2 {
        let branch = i.insert(ManagedInsn::Return { value: None });
        let b = cfg.add_block(branch);
        cfg.add_edge(previous, b, EdgeKind::Unconditional).unwrap();
        let (get_id, get) = def(&mut i, &mut v, ManagedType::Int, |result| {
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(result),
                method: MethodRef::new(2 + stage * 2),
                args: vec![],
            }
        });
        let g = cfg.add_block(get_id);
        let (zero_id, zero) = def(&mut i, &mut v, ManagedType::Int, |result| {
            ManagedInsn::Const {
                result,
                value: Constant::Int(0),
            }
        });
        let z = cfg.add_block(zero_id);
        let (compare_id, condition) = def(&mut i, &mut v, ManagedType::Boolean, |result| {
            ManagedInsn::Compare {
                op: CompareOp::NotEqual,
                result,
                lhs: get,
                rhs: zero,
            }
        });
        let c = cfg.add_block(compare_id);
        let nested = i.insert(ManagedInsn::Return { value: None });
        let n = cfg.add_block(nested);
        let (fallback_id, fallback) = def(&mut i, &mut v, ManagedType::Int, |result| {
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(result),
                method: MethodRef::new(3 + stage * 2),
                args: vec![],
            }
        });
        let f = cfg.add_block(fallback_id);
        let (phi_id, result) = def(&mut i, &mut v, ManagedType::Int, |result| {
            ManagedInsn::Phi {
                result,
                inputs: vec![],
            }
        });
        let p = cfg.add_block(phi_id);
        let yes = cfg.add_edge(b, g, EdgeKind::BranchTrue).unwrap();
        let no = cfg.add_edge(b, f, EdgeKind::BranchFalse).unwrap();
        cfg.add_edge(g, z, EdgeKind::Unconditional).unwrap();
        cfg.add_edge(z, c, EdgeKind::Unconditional).unwrap();
        cfg.add_edge(c, n, EdgeKind::Unconditional).unwrap();
        let selected = cfg.add_edge(n, p, EdgeKind::BranchTrue).unwrap();
        let missed = cfg.add_edge(n, f, EdgeKind::BranchFalse).unwrap();
        let supplied = cfg.add_edge(f, p, EdgeKind::Unconditional).unwrap();
        *i.get_mut(branch).unwrap() = ManagedInsn::Branch {
            condition: Some(if mode == 3 { this } else { present }),
            then_edge: yes,
            else_edge: Some(no),
        };
        *i.get_mut(nested).unwrap() = ManagedInsn::Branch {
            condition: Some(condition),
            then_edge: selected,
            else_edge: Some(missed),
        };
        *i.get_mut(phi_id).unwrap() = ManagedInsn::Phi {
            result,
            inputs: vec![
                (selected, get),
                (if mode == 4 { selected } else { supplied }, fallback),
            ],
        };
        if stage == 0 {
            match mode {
                1 => {
                    cfg.add_edge(n, g, EdgeKind::Unconditional).unwrap();
                }
                2 => {
                    cfg.add_edge(cfg.entry(), f, EdgeKind::Unconditional)
                        .unwrap();
                }
                5 => {
                    cfg.add_edge(g, f, EdgeKind::Exception("java.lang.Exception".into()))
                        .unwrap();
                }
                _ => {}
            }
        }
        args.push(result);
        previous = p;
    }
    let chain = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args,
    });
    let c = cfg.add_block(chain);
    cfg.add_edge(previous, c, EdgeKind::Unconditional).unwrap();
    let ret = i.insert(ManagedInsn::Return { value: None });
    let r = cfg.add_block(ret);
    cfg.add_edge(c, r, EdgeKind::Unconditional).unwrap();
    (
        MethodIr::<SsaForm>::from_parts(cfg, i, v),
        ParameterBindings {
            receiver: Some(this),
            parameters: vec![present],
            names: vec![],
        },
    )
}
#[test]
fn original_jvm_nested_sequential_selectors_match_java25_without_default_locals() {
    let (ir, b) = body(0);
    let p = plan_constructor(&ir, MethodRef::new(0), "Base", &b, &R).unwrap();
    assert!(p.source.contains("preInitRegion"));
    let dir = std::env::temp_dir().join(format!("preinit-nested-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = r#"
class Trace {
 static String log="";static boolean first,second;static int stop;
 static final RuntimeException error=new RuntimeException("exact");
 static void event(String s){log+=s;if(--stop==0)throw error;}
 static int get1(){event("G1");return first?1:0;}static int get2(){event("G2");return second?2:0;}
 static int fallback1(){event("R1");return 3;}static int fallback2(){event("R2");return 4;}
 static int token(){event("T");return 5;}
}
class Base {final int a,b;Base(int t,int a,int b){Trace.event("S");this.a=a;this.b=b;}}
public class Fixture extends Base {
 @CTOR@
 static String observe(boolean original,boolean p,int at) {
  Trace.log="";Trace.stop=at;String outcome;
  try {Base f=original?new FixtureOriginal(p):new Fixture(p);outcome="ok:"+f.a+":"+f.b;}
  catch(RuntimeException error){if(error!=Trace.error)throw new AssertionError("identity");outcome="same";}
  return outcome+":"+Trace.log;
 }
 public static void main(String[]x) {
  for(boolean p:new boolean[]{false,true})for(boolean a:new boolean[]{false,true})for(boolean b:new boolean[]{false,true}){
   Trace.first=a;Trace.second=b;
   for(int at=0;at<=8;at++) {String before=observe(true,p,at),after=observe(false,p,at);if(!before.equals(after))throw new AssertionError(before+" != "+after);}
  }
 }
}
"#.replace("@CTOR@", &p.source);
    std::fs::write(
        dir.join("ReferenceGenerator.java"),
        include_str!("fixtures/constructor_prefix_nested_regions/ReferenceGenerator.java"),
    )
    .unwrap();
    let jdk17 = "/home/jtjisgod/tools/jdk-17.0.13+11";
    for (tool, args) in [
        (
            "javac",
            vec![
                "--add-exports",
                "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
                "ReferenceGenerator.java",
            ],
        ),
        (
            "java",
            vec![
                "--add-exports",
                "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
                "ReferenceGenerator",
            ],
        ),
    ] {
        let out = Command::new(format!("{jdk17}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::write(dir.join("Fixture.java"), source).unwrap();
    let jdk = std::env::var("DECOMPHIFY_JDK25_HOME")
        .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into());
    for (tool, args) in [
        ("javac", vec!["--release", "25", "Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let out = Command::new(format!("{jdk}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&out.stderr),
            p.source
        );
    }
}
#[test]
fn nested_prefix_rejects_cycles_foreign_entries_early_this_and_bad_phi() {
    for mode in 1..=5 {
        let (ir, b) = body(mode);
        assert!(
            plan_constructor(&ir, MethodRef::new(0), "Base", &b, &R).is_err(),
            "accepted{mode}"
        );
    }
}
