//! Receiver identity columns never expose uninitialized this as a Java local.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    value::Value,
};
#[derive(Default)]
struct B {
    insns: InsnArena,
    values: ValueArena,
}
impl B {
    fn value(
        &mut self,
        ty: ManagedType,
        f: impl FnOnce(ValueId) -> ManagedInsn,
    ) -> (InsnId, ValueId) {
        let mut v = None;
        let id = self.insns.insert_with_key(|id| {
            let x = self.values.insert(Value::new(id, ty));
            v = Some(x);
            f(x)
        });
        (id, v.unwrap())
    }
    fn param(&mut self, ty: ManagedType) -> ValueId {
        self.value(ty, |v| ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        })
        .1
    }
}
struct R;
impl JavaResolver for R {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r.index() == 0 {
                "FcmPrefixFixture"
            } else {
                "java.lang.Object"
            }
            .into(),
        )
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        if r.index() == 2 {
            Some(JavaMethod {
                owner: "FcmPrefixFixture".into(),
                name: "eval".into(),
                is_static: true,
                parameters: vec![JavaType::Class("java.lang.Object".into())],
                return_type: JavaType::Class("java.lang.Object".into()),
                throws: vec![],
            })
        } else {
            self.initializer(r).map(|x| x.method)
        }
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        Some(if r.index() == 2 { 9 } else { 1 })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
impl ConstructorResolver for R {
    fn constructor_source_level(&self) -> u16 {
        25
    }
    fn initializer(&self, r: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: if r.index() == 0 {
                    "FcmPrefixFixture"
                } else {
                    "Base"
                }
                .into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: if r.index() == 0 {
                    vec![
                        JavaType::Boolean,
                        JavaType::Class("java.lang.Object".into()),
                        JavaType::Class("java.lang.Object".into()),
                    ]
                } else {
                    vec![JavaType::Class("java.lang.Object".into()); 2]
                },
                throws: vec![],
            },
            delegates_to: None,
        })
    }
}
fn fixture(
    foreign: bool,
    escape: bool,
    exceptional: bool,
) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut b = B::default();
    let own = ManagedType::Reference(ClassRef::new(0));
    let obj = ManagedType::Reference(ClassRef::new(1));
    let receiver = b.param(own.clone());
    let cond = b.param(ManagedType::Boolean);
    let a = b.param(obj.clone());
    let c = b.param(obj.clone());
    let branch = b.insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(branch);
    let entry = cfg.entry();
    let mut arms = Vec::new();
    for input in [a, c] {
        let (mi, alias) = b.value(own.clone(), |result| ManagedInsn::Move {
            result,
            value: if foreign && input == c { a } else { receiver },
        });
        let first = cfg.add_block(mi);
        let (ei, result) = b.value(obj.clone(), |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(2),
            args: vec![if escape { alias } else { input }],
        });
        let end = cfg.add_block(ei);
        cfg.add_edge(first, end, EdgeKind::Unconditional).unwrap();
        arms.push((first, end, alias, result));
    }
    let (ri, rphi) = b.value(own, |_| ManagedInsn::Return { value: None });
    let merge = cfg.add_block(ri);
    let e1 = cfg
        .add_edge(arms[0].1, merge, EdgeKind::Unconditional)
        .unwrap();
    let e2 = cfg
        .add_edge(arms[1].1, merge, EdgeKind::Unconditional)
        .unwrap();
    b.insns[ri] = ManagedInsn::Phi {
        result: rphi,
        inputs: vec![(e1, arms[0].2), (e2, arms[1].2)],
    };
    let (ai, aphi) = b.value(obj.clone(), |result| ManagedInsn::Phi {
        result,
        inputs: vec![(e1, arms[0].3), (e2, arms[1].3)],
    });
    let am = cfg.add_block(ai);
    cfg.add_edge(merge, am, EdgeKind::Unconditional).unwrap();
    let (ci, cphi) = b.value(obj, |result| ManagedInsn::Phi {
        result,
        inputs: vec![(e1, c), (e2, a)],
    });
    let cm = cfg.add_block(ci);
    cfg.add_edge(am, cm, EdgeKind::Unconditional).unwrap();
    let chain = cfg.add_block(b.insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![rphi, aphi, cphi],
    }));
    cfg.add_edge(cm, chain, EdgeKind::Unconditional).unwrap();
    let end = cfg.add_block(b.insns.insert(ManagedInsn::Return { value: None }));
    cfg.add_edge(chain, end, EdgeKind::Unconditional).unwrap();
    let yes = cfg
        .add_edge(entry, arms[0].0, EdgeKind::BranchTrue)
        .unwrap();
    let no = cfg
        .add_edge(entry, arms[1].0, EdgeKind::BranchFalse)
        .unwrap();
    b.insns[branch] = ManagedInsn::Branch {
        condition: Some(cond),
        then_edge: yes,
        else_edge: Some(no),
    };
    if exceptional {
        cfg.add_edge(arms[0].1, end, EdgeKind::Exception("all".into()))
            .unwrap();
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, b.insns, b.values),
        ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![cond, a, c],
            names: vec![],
        },
    )
}
#[test]
fn receiver_column_and_argument_snapshots_preserve_original_order() {
    let (ir, b) = fixture(false, false, false);
    let plan = plan_constructor(&ir, MethodRef::new(0), "Base", &b, &R).unwrap();
    assert!(
        !plan.source.contains("FcmPrefixFixture v"),
        "{}",
        plan.source
    );
    let root = std::env::temp_dir().join(format!("fcm-prefix-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = format!(
        r#"class Base {{ Object a,b; Base(Object a,Object b){{FcmPrefixFixture.trace+="P";this.a=a;this.b=b;}} }} public class FcmPrefixFixture extends Base {{static String trace="";static RuntimeException marker=new RuntimeException();static Object eval(Object x){{trace+="E";if(x==marker)throw marker;return x;}} {} public static void main(String[]args){{Object a=new Object(),b=new Object();for(boolean choice:new boolean[]{{true,false}})for(boolean original:new boolean[]{{false,true}}){{trace="";Base x=original?new OriginalPrefix(choice,a,b):new FcmPrefixFixture(choice,a,b);if(x.a!=(choice?a:b)||x.b!=(choice?b:a)||!trace.equals("EP"))throw new AssertionError(trace);trace="";x=original?new OriginalPrefix(choice,null,null):new FcmPrefixFixture(choice,null,null);if(x.a!=null||x.b!=null||!trace.equals("EP"))throw new AssertionError(trace);trace="";try{{if(original)new OriginalPrefix(choice,marker,marker);else new FcmPrefixFixture(choice,marker,marker);throw new AssertionError();}}catch(RuntimeException e){{if(e!=marker||!trace.equals("E"))throw new AssertionError(trace);}}}}}} }}"#,
        plan.source
    );
    std::fs::write(root.join("FcmPrefixFixture.java"), source).unwrap();
    let oracle = r#"import jdk.internal.org.objectweb.asm.*; import java.nio.file.*;
public class OriginalGenerator implements Opcodes {public static void main(String[]args)throws Exception {
 ClassWriter w=new ClassWriter(ClassWriter.COMPUTE_FRAMES|ClassWriter.COMPUTE_MAXS);w.visit(69,ACC_PUBLIC,"OriginalPrefix",null,"Base",null);
 MethodVisitor m=w.visitMethod(ACC_PUBLIC,"<init>","(ZLjava/lang/Object;Ljava/lang/Object;)V",null,null);m.visitCode();Label no=new Label(),join=new Label();m.visitVarInsn(ILOAD,1);m.visitJumpInsn(IFEQ,no);
 m.visitVarInsn(ALOAD,0);m.visitVarInsn(ASTORE,4);m.visitVarInsn(ALOAD,2);m.visitMethodInsn(INVOKESTATIC,"FcmPrefixFixture","eval","(Ljava/lang/Object;)Ljava/lang/Object;",false);m.visitVarInsn(ASTORE,5);m.visitVarInsn(ALOAD,3);m.visitVarInsn(ASTORE,6);m.visitJumpInsn(GOTO,join);
 m.visitLabel(no);m.visitVarInsn(ALOAD,0);m.visitVarInsn(ASTORE,4);m.visitVarInsn(ALOAD,3);m.visitMethodInsn(INVOKESTATIC,"FcmPrefixFixture","eval","(Ljava/lang/Object;)Ljava/lang/Object;",false);m.visitVarInsn(ASTORE,5);m.visitVarInsn(ALOAD,2);m.visitVarInsn(ASTORE,6);
 m.visitLabel(join);m.visitVarInsn(ALOAD,4);m.visitVarInsn(ALOAD,5);m.visitVarInsn(ALOAD,6);m.visitMethodInsn(INVOKESPECIAL,"Base","<init>","(Ljava/lang/Object;Ljava/lang/Object;)V",false);m.visitInsn(RETURN);m.visitMaxs(0,0);m.visitEnd();w.visitEnd();Files.write(Path.of("OriginalPrefix.class"),w.toByteArray());}}
"#;
    std::fs::write(root.join("OriginalGenerator.java"), oracle).unwrap();
    let bin = "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1/bin";
    for (tool, args) in [
        (
            "javac",
            vec![
                "--add-exports",
                "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
                "OriginalGenerator.java",
            ],
        ),
        (
            "java",
            vec![
                "--add-exports",
                "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
                "-cp",
                ".",
                "OriginalGenerator",
            ],
        ),
    ] {
        let output =
            std::process::Command::new(format!("/home/jtjisgod/tools/jdk-17.0.13+11/bin/{tool}"))
                .current_dir(&root)
                .args(args)
                .output()
                .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for (tool, args) in [
        ("javac", vec!["FcmPrefixFixture.java"]),
        ("java", vec!["-cp", ".", "FcmPrefixFixture"]),
    ] {
        let out = std::process::Command::new(format!("{bin}/{tool}"))
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
#[test]
fn foreign_receiver_escape_and_exception_edges_stay_closed() {
    for (foreign, escape, exceptional) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let (ir, b) = fixture(foreign, escape, exceptional);
        assert!(plan_constructor(&ir, MethodRef::new(0), "Base", &b, &R).is_err());
    }
}
