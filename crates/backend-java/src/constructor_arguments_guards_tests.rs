//! Exact constructor-prefix operands keep reference identity and proven zero context.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    effects::EffectFlags,
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
    fn parameter(&mut self) -> ValueId {
        let v = self.value(ManagedType::Unknown, |v| ManagedInsn::Opaque {
            family_tag: 0,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        });
        self.order.pop();
        v
    }
}

struct Resolver {
    release: u16,
    access: bool,
}
impl JavaResolver for Resolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        if r.index() == 2 {
            Some(JavaMethod {
                owner: "Fixture".into(),
                name: "d".into(),
                is_static: true,
                parameters: vec![JavaType::Int],
                return_type: JavaType::Void,
                throws: vec![],
            })
        } else {
            self.initializer(r).map(|i| i.method)
        }
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        if self.access {
            Some(if r.index() == 2 { 9 } else { 1 })
        } else {
            None
        }
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("java.lang.Object".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
impl ConstructorResolver for Resolver {
    fn constructor_source_level(&self) -> u16 {
        self.release
    }
    fn initializer(&self, r: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: if r.index() == 0 { "Fixture" } else { "Base" }.into(),
                name: "<init>".into(),
                is_static: false,
                parameters: vec![
                    JavaType::Class("java.lang.Object".into()),
                    JavaType::Class("java.lang.Object".into()),
                ],
                return_type: JavaType::Void,
                throws: vec![],
            },
            delegates_to: None,
        })
    }
}
fn fixture(receiver_call: bool) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut b = Builder::default();
    let receiver = b.parameter();
    let a = b.parameter();
    let z = b.parameter();
    for input in [a, z] {
        b.values[input] = Value::new(
            b.values[input].def(),
            ManagedType::Reference(ClassRef::new(1)),
        );
    }
    let zero = b.value(ManagedType::Zero, |result| ManagedInsn::Const {
        result,
        value: Constant::Zero,
    });
    let mut cfg = Cfg::new(b.order[0]);
    let mut last = cfg.entry();
    for (index, input) in [a, z].into_iter().enumerate() {
        let guard = b.value(ManagedType::Boolean, |result| ManagedInsn::Compare {
            op: CompareOp::Equal,
            result,
            lhs: input,
            rhs: zero,
        });
        let cmp = cfg.add_block(*b.order.last().unwrap());
        cfg.add_edge(last, cmp, EdgeKind::Unconditional).unwrap();
        let id = b.insns.insert(ManagedInsn::Return { value: None });
        let branch = cfg.add_block(id);
        cfg.add_edge(cmp, branch, EdgeKind::Unconditional).unwrap();
        let number = b.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(index as i32),
        });
        let constant = cfg.add_block(*b.order.last().unwrap());
        let call = cfg.add_block(b.insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(2),
            args: vec![if receiver_call { receiver } else { number }],
        }));
        cfg.add_edge(constant, call, EdgeKind::Unconditional)
            .unwrap();
        let _dummy = b.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(7),
        });
        let merge = cfg.add_block(*b.order.last().unwrap());
        cfg.add_edge(call, merge, EdgeKind::Unconditional).unwrap();
        let te = cfg
            .add_edge(branch, constant, EdgeKind::BranchTrue)
            .unwrap();
        let fe = cfg.add_edge(branch, merge, EdgeKind::BranchFalse).unwrap();
        b.insns[id] = ManagedInsn::Branch {
            condition: Some(guard),
            then_edge: te,
            else_edge: Some(fe),
        };
        last = merge;
    }
    let chain = cfg.add_block(b.insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![receiver, a, z],
    }));
    cfg.add_edge(last, chain, EdgeKind::Unconditional).unwrap();
    let end = cfg.add_block(b.insns.insert(ManagedInsn::Return { value: None }));
    cfg.add_edge(chain, end, EdgeKind::Unconditional).unwrap();
    (
        MethodIr::<SsaForm>::from_parts(cfg, b.insns, b.values),
        ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![a, z],
            names: vec![],
        },
    )
}
#[test]
fn sequential_guards_preserve_failure_identity_order_and_returning_calls() {
    let (ir, bindings) = fixture(false);
    let plan = plan_constructor(
        &ir,
        MethodRef::new(0),
        "Base",
        &bindings,
        &Resolver {
            release: 25,
            access: true,
        },
    )
    .unwrap();
    let root = std::env::temp_dir().join(format!("constructor-sequential-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = format!(
        "class Base {{ Base(Object a,Object b){{if(a!=Fixture.expectedA||b!=Fixture.expectedB)throw new AssertionError();Fixture.trace+=\"P\";}} }} class Fixture extends Base {{static String trace=\"\";static int fail=-1;static Object expectedA,expectedB;static RuntimeException[] markers={{new RuntimeException(),new RuntimeException()}};public static void d(int i){{trace+=i;if(fail==i)throw markers[i];}} {} public static void main(String[] args){{for(int f=-1;f<2;f++)for(int mask=0;mask<4;mask++){{trace=\"\";fail=f;Object a=(mask&1)==0?new Object():null,b=(mask&2)==0?new Object():null;expectedA=a;expectedB=b;String expected=\"\";int selected=-1;if(a==null){{expected+=\"0\";if(f==0)selected=0;}}if(selected<0&&b==null){{expected+=\"1\";if(f==1)selected=1;}}if(selected<0)expected+=\"P\";try{{if(args.length>0)new Raw(a,b);else new Fixture(a,b);if(selected>=0)throw new AssertionError();}}catch(RuntimeException e){{if(selected<0||e!=markers[selected])throw new AssertionError();}}if(!trace.equals(expected))throw new AssertionError(trace+\" != \"+expected);}}}}}}",
        plan.source
    );
    std::fs::write(root.join("Fixture.java"), source).unwrap();
    let jdk = "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1/bin";
    // Raw JVM constructor retains the original Dalvik ordering: two
    // IFNONNULL/static-call guards before invokespecial. It has no Java
    // constructor source, so javac cannot normalize the reference ordering.
    let raw = r#"import jdk.internal.org.objectweb.asm.*;
class BuildRaw implements Opcodes {
 public static void main(String[] args) throws Exception {
  ClassWriter c=new ClassWriter(ClassWriter.COMPUTE_FRAMES|ClassWriter.COMPUTE_MAXS);
  c.visit(V17,ACC_PUBLIC,"Raw",null,"Base",null);
  MethodVisitor m=c.visitMethod(ACC_PUBLIC,"<init>","(Ljava/lang/Object;Ljava/lang/Object;)V",null,null);m.visitCode();
  for(int i=0;i<2;i++){Label next=new Label();m.visitVarInsn(ALOAD,i+1);m.visitJumpInsn(IFNONNULL,next);m.visitInsn(i==0?ICONST_0:ICONST_1);m.visitMethodInsn(INVOKESTATIC,"Fixture","d","(I)V",false);m.visitLabel(next);}
  m.visitVarInsn(ALOAD,0);m.visitVarInsn(ALOAD,1);m.visitVarInsn(ALOAD,2);m.visitMethodInsn(INVOKESPECIAL,"Base","<init>","(Ljava/lang/Object;Ljava/lang/Object;)V",false);m.visitInsn(RETURN);m.visitMaxs(0,0);m.visitEnd();c.visitEnd();java.nio.file.Files.write(java.nio.file.Path.of(args[0],"Raw.class"),c.toByteArray());
 }
}"#;
    std::fs::write(root.join("BuildRaw.java"), raw).unwrap();
    let exports = "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED";
    let raw_jdk = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
    for (tool, args) in [
        (
            "javac",
            vec![
                "--add-exports".to_owned(),
                exports.into(),
                root.join("BuildRaw.java").to_str().unwrap().into(),
            ],
        ),
        (
            "java",
            vec![
                "--add-exports".to_owned(),
                exports.into(),
                "-cp".into(),
                root.to_str().unwrap().into(),
                "BuildRaw".into(),
                root.to_str().unwrap().into(),
            ],
        ),
    ] {
        let out = std::process::Command::new(format!("{raw_jdk}/{tool}"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    for (tool, args) in [
        (
            "javac",
            vec![
                "-cp".into(),
                root.to_str().unwrap().into(),
                root.join("Fixture.java").to_str().unwrap().to_owned(),
            ],
        ),
        (
            "java",
            vec![
                "-cp".into(),
                root.to_str().unwrap().into(),
                "Fixture".into(),
            ],
        ),
        (
            "java",
            vec![
                "-cp".into(),
                root.to_str().unwrap().into(),
                "Fixture".into(),
                "raw".into(),
            ],
        ),
    ] {
        let out = std::process::Command::new(format!("{jdk}/{tool}"))
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
fn guards_reject_java17_missing_access_and_receiver_reads() {
    for (release, access, receiver) in [(17, true, false), (25, false, false), (25, true, true)] {
        let (ir, bindings) = fixture(receiver);
        assert!(
            plan_constructor(
                &ir,
                MethodRef::new(0),
                "Base",
                &bindings,
                &Resolver { release, access }
            )
            .is_err()
        );
    }
}
