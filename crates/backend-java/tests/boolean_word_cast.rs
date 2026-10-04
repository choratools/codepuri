//! Original Dalvik int-word conversions of descriptor-proven booleans.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, SourceResult,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
    types::ManagedType,
    value::Value,
};
struct R(Option<JavaType>);
impl JavaResolver for R {
    fn instruction_type(&self, _: TypeRef) -> Option<JavaType> {
        self.0.clone()
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "Fixture".into(),
            name: "tick".into(),
            parameters: vec![JavaType::Boolean],
            return_type: JavaType::Boolean,
            is_static: true,
            throws: vec![],
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
fn emit(
    op: u32,
    target: Option<JavaType>,
    mt: ManagedType,
    jt: JavaType,
    name: &str,
) -> SourceResult {
    let mut i = InsnArena::with_key();
    let mut v = ValueArena::with_key();
    let mut p = None;
    i.insert_with_key(|id| {
        let x = v.insert(Value::new(id, ManagedType::Boolean));
        p = Some(x);
        ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![x],
            effects: EffectFlags::NONE,
        }
    });
    let mut input = p.unwrap();
    let effect = (name == "effect").then(|| {
        i.insert_with_key(|id| {
            let out = v.insert(Value::new(id, ManagedType::Boolean));
            input = out;
            ManagedInsn::Invoke {
                kind: managed_ir::ops::InvokeKind::Static,
                result: Some(out),
                method: MethodRef::new(0),
                args: vec![p.unwrap()],
            }
        })
    });
    let mut r = None;
    let c = i.insert_with_key(|id| {
        let x = v.insert(Value::new(id, mt));
        r = Some(x);
        ManagedInsn::Cast {
            result: x,
            value: input,
            target: TypeRef::new(op),
        }
    });
    let ret = i.insert(ManagedInsn::Return { value: r });
    let mut cfg = Cfg::new(effect.unwrap_or(c));
    if effect.is_some() {
        let cast = cfg.add_block(c);
        cfg.add_edge(cfg.entry(), cast, EdgeKind::Unconditional)
            .unwrap();
    }

    let end = cfg.add_block(ret);
    let predecessor = cfg
        .block_ids()
        .find(|id| cfg.block(*id).unwrap().payload() == &c)
        .unwrap();
    cfg.add_edge(predecessor, end, EdgeKind::Unconditional)
        .unwrap();
    backend_java::emit_method(
        &MethodIr::<SsaForm>::from_parts(cfg, i, v),
        &JavaMethod {
            owner: "Fixture".into(),
            name: name.into(),
            parameters: vec![JavaType::Boolean],
            return_type: jt,
            is_static: true,
            throws: vec![],
        },
        &ParameterBindings {
            parameters: vec![p.unwrap()],
            ..Default::default()
        },
        &R(target),
    )
}
#[test]
fn exact_int_word_opcodes_compile_and_execute() {
    let mut source = "public class Fixture {".to_string();
    for (op, mt, jt, name) in [
        (0x81, ManagedType::Long, JavaType::Long, "l"),
        (0x82, ManagedType::Float, JavaType::Float, "f"),
        (0x83, ManagedType::Double, JavaType::Double, "d"),
        (0x8d, ManagedType::Byte, JavaType::Byte, "b"),
        (0x8e, ManagedType::Char, JavaType::Char, "c"),
        (0x8f, ManagedType::Short, JavaType::Short, "s"),
    ] {
        let SourceResult::Complete { source: s, .. } =
            emit(0x80000000 | op, Some(jt.clone()), mt, jt, name)
        else {
            panic!("conversion denied")
        };
        source.push_str(&s);
    }
    let SourceResult::Complete { source: effect, .. } = emit(
        0x8000008d,
        Some(JavaType::Byte),
        ManagedType::Byte,
        JavaType::Byte,
        "effect",
    ) else {
        panic!("effect cast denied")
    };
    source.push_str(&effect);
    source.push_str("static RuntimeException failure; static int calls; static boolean tick(boolean x){calls++;if(failure!=null)throw failure;return x;} public static void main(String[] a){for(boolean x:new boolean[]{false,true}){int n=x?1:0;if(l(x)!=Original.l(x)||f(x)!=Original.f(x)||d(x)!=Original.d(x)||b(x)!=Original.b(x)||c(x)!=Original.c(x)||s(x)!=Original.s(x))throw new AssertionError();if(effect(x)!=n)throw new AssertionError();}if(calls!=2)throw new AssertionError();failure=new RuntimeException();try{effect(true);throw new AssertionError();}catch(RuntimeException e){if(e!=failure||calls!=3)throw new AssertionError();}}}");
    let dir = std::env::temp_dir().join(format!("boolean-word-cast-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), source).unwrap();
    let generator = r#"import jdk.internal.org.objectweb.asm.*;
public class Generate implements Opcodes {
 public static void main(String[] args)throws Exception{
 ClassWriter c=new ClassWriter(0);c.visit(V17,ACC_PUBLIC,"Original",null,"java/lang/Object",null);
 String[] names={"l","f","d","b","c","s"};String[] results={"J","F","D","B","C","S"};
 int[] cast={I2L,I2F,I2D,I2B,I2C,I2S},ret={LRETURN,FRETURN,DRETURN,IRETURN,IRETURN,IRETURN};
 for(int i=0;i<6;i++){MethodVisitor m=c.visitMethod(ACC_PUBLIC|ACC_STATIC,names[i],"(Z)"+results[i],null,null);m.visitCode();m.visitVarInsn(ILOAD,0);m.visitInsn(cast[i]);m.visitInsn(ret[i]);m.visitMaxs(2,1);m.visitEnd();}
 c.visitEnd();java.nio.file.Files.write(java.nio.file.Path.of(args[0],"Original.class"),c.toByteArray());}}
"#;
    std::fs::write(dir.join("Generate.java"), generator).unwrap();
    let jdk = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
    assert!(
        std::process::Command::new(format!("{jdk}/javac"))
            .args([
                "--add-exports",
                "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED"
            ])
            .arg(dir.join("Generate.java"))
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new(format!("{jdk}/java"))
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

    let jdk = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
    let output = std::process::Command::new(format!("{jdk}/javac"))
        .args(["-cp", dir.to_str().unwrap()])
        .arg(dir.join("Fixture.java"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        std::process::Command::new(format!("{jdk}/java"))
            .args(["-cp", dir.to_str().unwrap(), "Fixture"])
            .status()
            .unwrap()
            .success()
    );
}
#[test]
fn wrong_opcode_target_and_missing_metadata_reject() {
    for (op, target) in [
        (0x80000084, Some(JavaType::Byte)),
        (0x8d, Some(JavaType::Byte)),
        (0x8000008d, Some(JavaType::Long)),
        (0x8000008d, None),
    ] {
        assert!(matches!(
            emit(op, target, ManagedType::Byte, JavaType::Byte, "bad"),
            SourceResult::Unsupported(_)
        ));
    }
}
