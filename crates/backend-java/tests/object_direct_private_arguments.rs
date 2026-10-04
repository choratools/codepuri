//! Exact private producers remain after NEW; Java overload/override dispatch stays bound.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, SourceResult, SuperInvocation,
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
use std::process::Command;
struct R(u8);
fn method(owner: &str, name: &str, parameters: Vec<JavaType>, return_type: JavaType) -> JavaMethod {
    JavaMethod {
        owner: owner.into(),
        name: name.into(),
        parameters,
        return_type,
        is_static: false,
        throws: vec![],
    }
}
impl JavaResolver for R {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let mut target = if r.index() == 0 {
            method(
                if self.0 == 2 { "Base" } else { "Child" },
                "produce",
                vec![JavaType::Class("java.lang.Object".into())],
                JavaType::Int,
            )
        } else {
            method("Box", "<init>", vec![JavaType::Int], JavaType::Void)
        };
        if self.0 == 5 && r.index() == 0 {
            target.throws = vec!["java.io.IOException".into()];
        }
        if self.0 == 6 && r.index() == 0 {
            target.parameters.push(JavaType::Int);
        }
        Some(target)
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        if r.index() == 0 {
            if self.0 == 1 {
                None
            } else {
                Some(if self.0 == 3 { 1 } else { 2 })
            }
        } else {
            Some(1)
        }
    }
    fn super_invocation(&self, caller: &str, r: MethodRef) -> Option<SuperInvocation> {
        if self.0 == 1 || caller != "Child" || r.index() != 0 {
            return None;
        }
        Some(SuperInvocation {
            method: method(
                if self.0 == 2 { "Child" } else { "Base" },
                "produce",
                vec![],
                JavaType::Int,
            ),
            access_flags: if self.0 == 3 { 2 } else { 1 },
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            match r.index() {
                0 => "Child",
                1 => "Box",
                _ => "java.lang.Object",
            }
            .into(),
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
fn value(
    i: &mut InsnArena,
    v: &mut ValueArena,
    ty: ManagedType,
    f: impl FnOnce(ValueId) -> ManagedInsn,
) -> (managed_ir::value::InsnId, ValueId) {
    let mut out = None;
    let id = i.insert_with_key(|id| {
        let x = v.insert(Value::new(id, ty));
        out = Some(x);
        f(x)
    });
    (id, out.unwrap())
}
fn emit(mode: u8) -> SourceResult {
    let mut i = InsnArena::with_key();
    let mut v = ValueArena::with_key();
    let (_, this) = value(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(0)),
        |x| ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![x],
            effects: EffectFlags::NONE,
        },
    );
    let (_, flag) = value(&mut i, &mut v, ManagedType::Boolean, |x| {
        ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![x],
            effects: EffectFlags::NONE,
        }
    });
    let (_, argument) = value(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(2)),
        |x| ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![x],
            effects: EffectFlags::NONE,
        },
    );
    let branch = i.insert(ManagedInsn::Return { value: None });
    let (new, obj) = value(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(1)),
        |x| ManagedInsn::NewInstance {
            result: x,
            class: ClassRef::new(1),
        },
    );
    let (call, arg) = value(&mut i, &mut v, ManagedType::Int, |x| ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: Some(x),
        method: MethodRef::new(0),
        args: vec![if mode == 7 { obj } else { this }, argument],
    });
    let init = i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![obj, arg],
    });
    let (zero, nil) = value(&mut i, &mut v, ManagedType::Null, |x| ManagedInsn::Const {
        result: x,
        value: Constant::Zero,
    });
    let (phi, result) = value(
        &mut i,
        &mut v,
        ManagedType::Reference(ClassRef::new(1)),
        |x| ManagedInsn::Return { value: Some(x) },
    );
    let ret = i.insert(ManagedInsn::Return {
        value: Some(result),
    });
    let mut cfg = Cfg::new(branch);
    let a = cfg.add_block(new);
    let b = cfg.add_block(call);
    let c = cfg.add_block(init);
    let z = cfg.add_block(zero);
    let p = cfg.add_block(phi);
    let r = cfg.add_block(ret);
    let yes = cfg.add_edge(cfg.entry(), a, EdgeKind::BranchTrue).unwrap();
    let no = cfg.add_edge(cfg.entry(), z, EdgeKind::BranchFalse).unwrap();
    i[branch] = ManagedInsn::Branch {
        condition: Some(flag),
        then_edge: yes,
        else_edge: Some(no),
    };
    cfg.add_edge(a, b, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(b, c, EdgeKind::Unconditional).unwrap();
    let from = if mode == 4 {
        let duplicate = i.insert(i[init].clone());
        let d = cfg.add_block(duplicate);
        cfg.add_edge(c, d, EdgeKind::Unconditional).unwrap();
        d
    } else {
        c
    };
    let left = cfg.add_edge(from, p, EdgeKind::Unconditional).unwrap();
    let right = cfg.add_edge(z, p, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(p, r, EdgeKind::Unconditional).unwrap();
    i[phi] = ManagedInsn::Phi {
        result,
        inputs: vec![(left, obj), (right, nil)],
    };
    backend_java::emit_method(
        &MethodIr::<SsaForm>::from_parts(cfg, i, v),
        &method(
            "Child",
            "make",
            vec![
                JavaType::Boolean,
                JavaType::Class("java.lang.Object".into()),
            ],
            JavaType::Class("Box".into()),
        ),
        &ParameterBindings {
            receiver: Some(this),
            parameters: vec![flag, argument],
            ..Default::default()
        },
        &R(mode),
    )
}
#[test]
fn absent_or_illegal_direct_private_stays_denied() {
    for mode in 1..=7 {
        assert!(
            matches!(emit(mode), SourceResult::Unsupported(_)),
            "mode{mode}: {:?}",
            emit(mode)
        );
    }
}
#[test]
fn original_new_private_init_order_and_phi_execute() {
    let SourceResult::Complete { source, .. } = emit(0) else {
        panic!("{:?}", emit(0))
    };
    assert!(source.contains("produce("), "{source}");
    let dir = std::env::temp_dir().join(format!("private-argument-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Child.java"),
        format!("public class Child extends Base {{ private int produce(Object arg) {{ return Harness.produce(arg); }} {source} }} class Shadow extends Child {{ public int produce(Object arg) {{ throw new AssertionError(\"wrong virtual target\"); }} }}"),
    )
    .unwrap();
    std::fs::write(
        dir.join("Base.java"),
        "public class Base {public int shadow(){throw new AssertionError(\"wrong target\");}}",
    )
    .unwrap();
    std::fs::write(dir.join("Box.java"),"public class Box {static {Harness.log+=\"N\";} int value;Box(int x){Harness.log+=\"C\";value=x;}}").unwrap();
    let mut box_source = std::fs::read_to_string(dir.join("Box.java")).unwrap();
    let fields = (0..4096).map(|n| format!("long f{n};")).collect::<String>();
    box_source = box_source.replacen(
        "public class Box {",
        &format!("public class Box {{{fields}"),
        1,
    );
    std::fs::write(dir.join("Box.java"), box_source).unwrap();
    std::fs::write(dir.join("Generate.java"),r#"import jdk.internal.org.objectweb.asm.*;public class Generate implements Opcodes {public static void main(String[] a)throws Exception{ClassWriter c=new ClassWriter(ClassWriter.COMPUTE_FRAMES);c.visit(V17,ACC_PUBLIC,"Original",null,"Base",null);MethodVisitor m=c.visitMethod(ACC_PUBLIC,"<init>","()V",null,null);m.visitCode();m.visitVarInsn(ALOAD,0);m.visitMethodInsn(INVOKESPECIAL,"Base","<init>","()V",false);m.visitInsn(RETURN);m.visitMaxs(0,0);m.visitEnd();m=c.visitMethod(ACC_PRIVATE,"produce","(Ljava/lang/Object;)I",null,null);m.visitCode();m.visitVarInsn(ALOAD,1);m.visitMethodInsn(INVOKESTATIC,"Harness","produce","(Ljava/lang/Object;)I",false);m.visitInsn(IRETURN);m.visitMaxs(0,0);m.visitEnd();m=c.visitMethod(ACC_PUBLIC,"make","(ZLjava/lang/Object;)LBox;",null,null);m.visitCode();Label no=new Label(),end=new Label();m.visitVarInsn(ILOAD,1);m.visitJumpInsn(IFEQ,no);m.visitTypeInsn(NEW,"Box");m.visitInsn(DUP);m.visitVarInsn(ALOAD,0);m.visitVarInsn(ALOAD,2);m.visitMethodInsn(INVOKESPECIAL,"Original","produce","(Ljava/lang/Object;)I",false);m.visitMethodInsn(INVOKESPECIAL,"Box","<init>","(I)V",false);m.visitJumpInsn(GOTO,end);m.visitLabel(no);m.visitInsn(ACONST_NULL);m.visitLabel(end);m.visitInsn(ARETURN);m.visitMaxs(0,0);m.visitEnd();c.visitEnd();java.nio.file.Files.write(java.nio.file.Path.of(a[0],"Original.class"),c.toByteArray());}}"#).unwrap();
    std::fs::write(dir.join("Harness.java"),r#"public class Harness {static int produce(Object arg){if(arg!=null)throw new AssertionError();calls++;log+="S";if(failure!=null)throw failure;return 7;}static String log="";static RuntimeException failure;static int calls;static java.util.LinkedList<byte[]> keep;public static void main(String[] args)throws Exception{boolean generated=args[0].equals("generated");boolean yes=!args[1].equals("false");if(args[1].equals("oom")){Class.forName("Box");Child child=new Child();Original original=new Original();log="";keep=new java.util.LinkedList<>();try{while(true)keep.add(new byte[8192]);}catch(OutOfMemoryError expected){}boolean fit=false;try{if(generated)child.make(true,null);else original.make(true,null);fit=true;}catch(OutOfMemoryError expected){}finally{keep=null;}System.gc();if(fit||calls!=0)throw new AssertionError("super ran or allocation fit");System.out.println("OOM:0");return;}if(args[1].equals("throw"))failure=new RuntimeException("exact");try{Box b=generated?new Shadow().make(yes,null):new Original().make(yes,null);if(failure!=null)throw new AssertionError();if(yes){if(b.value!=7||!log.equals("NSC"))throw new AssertionError(log);}else if(b!=null||!log.equals(""))throw new AssertionError(log);}catch(RuntimeException e){if(e!=failure||!log.equals("NS"))throw new AssertionError(log);}System.out.println(log);}}"#).unwrap();
    let jdk = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
    let o = Command::new(format!("{jdk}/javac"))
        .args([
            "--add-exports",
            "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
        ])
        .arg(dir.join("Generate.java"))
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(
        Command::new(format!("{jdk}/java"))
            .args([
                "--add-exports",
                "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
                "-cp",
                dir.to_str().unwrap(),
                "Generate",
                dir.to_str().unwrap()
            ])
            .status()
            .unwrap()
            .success()
    );
    let o = Command::new(format!("{jdk}/javac"))
        .args(["-cp", dir.to_str().unwrap(), "-d", dir.to_str().unwrap()])
        .args([
            dir.join("Child.java"),
            dir.join("Base.java"),
            dir.join("Box.java"),
            dir.join("Harness.java"),
        ])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    for jdk in [jdk, "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1/bin"] {
        for mode in ["false", "true", "throw", "oom"] {
            let run = |which| {
                Command::new(format!("{jdk}/java"))
                    .args([
                        "-Xmx32m",
                        "-cp",
                        dir.to_str().unwrap(),
                        "Harness",
                        which,
                        mode,
                    ])
                    .output()
                    .unwrap()
            };
            let a = run("generated");
            let b = run("original");
            assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
            assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
            assert_eq!(a.stdout, b.stdout);
        }
    }
}
