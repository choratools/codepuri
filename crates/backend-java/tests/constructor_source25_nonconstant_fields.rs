//! A final primitive/String source declaration may be a nonconstant variable.
//! Retain its original getstatic class-initialization boundary before super.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, constructors::*,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{Value, ValueId},
};
struct R {
    release: u16,
    attested: u8,
}
fn types() -> Vec<JavaType> {
    vec![
        JavaType::Int,
        JavaType::Int,
        JavaType::Long,
        JavaType::Class("java.lang.String".into()),
    ]
}
impl JavaResolver for R {
    fn class(&self, c: ClassRef) -> Option<String> {
        Some(
            if c.index() == 0 {
                "Fixture"
            } else {
                "java.lang.String"
            }
            .into(),
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn field(&self, f: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fields".into(),
            name: ["c", "d", "e", "a"][f.index() as usize].into(),
            ty: types()[f.index() as usize].clone(),
            is_static: true,
        })
    }
    fn field_access(&self, _: FieldRef) -> Option<u32> {
        Some(0x19)
    }
    fn field_source_nonconstant(&self, f: FieldRef) -> bool {
        self.attested & (1 << f.index()) != 0
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        self.initializer(m).map(|i| i.method)
    }
}
impl ConstructorResolver for R {
    fn constructor_source_level(&self) -> u16 {
        self.release
    }
    fn initializer(&self, m: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: if m.index() == 0 { "Fixture" } else { "Parent" }.into(),
                name: "<init>".into(),
                parameters: if m.index() == 0 { vec![] } else { types() },
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: None,
        })
    }
}
fn define(
    i: &mut InsnArena,
    v: &mut ValueArena,
    ty: ManagedType,
    f: impl FnOnce(ValueId) -> ManagedInsn,
) -> (managed_ir::value::InsnId, ValueId) {
    let mut value = None;
    let id = i.insert_with_key(|id| {
        let x = v.insert(Value::new(id, ty));
        value = Some(x);
        f(x)
    });
    (id, value.unwrap())
}
fn body(receiver_read: bool) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut i = InsnArena::default();
    let mut v = ValueArena::default();
    let (_, this) = define(
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
    let mut ids = Vec::new();
    let mut args = vec![this];
    for (index, ty) in [
        ManagedType::Int,
        ManagedType::Int,
        ManagedType::Long,
        ManagedType::Reference(ClassRef::new(1)),
    ]
    .into_iter()
    .enumerate()
    {
        let (id, x) = define(&mut i, &mut v, ty, |result| ManagedInsn::FieldGet {
            result,
            object: (receiver_read && index == 0).then_some(this),
            field: FieldRef::new(index as u32),
        });
        ids.push(id);
        args.push(x);
    }
    ids.push(i.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args,
    }));
    ids.push(i.insert(ManagedInsn::Return { value: None }));
    let mut cfg = Cfg::new(ids[0]);
    let mut previous = cfg.entry();
    for id in ids.into_iter().skip(1) {
        let next = cfg.add_block(id);
        cfg.add_edge(previous, next, EdgeKind::Unconditional)
            .unwrap();
        previous = next;
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, i, v),
        ParameterBindings {
            receiver: Some(this),
            parameters: vec![],
            names: vec![],
        },
    )
}
#[test]
fn exact_attested_nonconstant_fields_remain_eager_before_parent() {
    let (ir, b) = body(false);
    let p = plan_constructor(
        &ir,
        MethodRef::new(0),
        "Parent",
        &b,
        &R {
            release: 25,
            attested: 15,
        },
    )
    .unwrap();
    let super_at = p.source.find("super(").unwrap();
    let mut previous = 0;
    for name in ["Fields.c", "Fields.d", "Fields.e", "Fields.a"] {
        let at = p.source.find(name).unwrap();
        assert!(at >= previous && at < super_at, "{}", p.source);
        assert_eq!(p.source.matches(name).count(), 1);
        previous = at;
    }
    assert_eq!(p.prologue.len(), 4);
}
#[test]
fn unknown_constant_fields_and_uninitialized_receiver_stay_denied() {
    let (ir, b) = body(false);
    for mask in [0, 1, 3, 7, 14] {
        assert!(
            plan_constructor(
                &ir,
                MethodRef::new(0),
                "Parent",
                &b,
                &R {
                    release: 25,
                    attested: mask
                }
            )
            .is_err(),
            "unknown field mask {mask}"
        );
    }
    let (ir, b) = body(true);
    assert!(
        plan_constructor(
            &ir,
            MethodRef::new(0),
            "Parent",
            &b,
            &R {
                release: 25,
                attested: 15
            }
        )
        .is_err()
    );
}
#[test]
fn default_source17_argument_lowering_stays_unchanged() {
    let (ir, b) = body(false);
    let unknown = plan_constructor(
        &ir,
        MethodRef::new(0),
        "Parent",
        &b,
        &R {
            release: 17,
            attested: 0,
        },
    )
    .unwrap();
    let known = plan_constructor(
        &ir,
        MethodRef::new(0),
        "Parent",
        &b,
        &R {
            release: 17,
            attested: 15,
        },
    )
    .unwrap();
    assert_eq!(unknown.source, known.source);
    assert!(unknown.prologue.is_empty());
    for name in ["Fields.c", "Fields.d", "Fields.e", "Fields.a"] {
        assert_eq!(unknown.source.matches(name).count(), 1);
    }
}
fn run(tool: &str, args: &[&str], dir: &std::path::Path) -> Vec<u8> {
    let home = std::env::var("DECOMPHIFY_JDK25_HOME")
        .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into());
    let out = std::process::Command::new(format!("{home}/bin/{tool}"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{tool}:{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
#[test]
fn source_blank_finals_preserve_getstatic_initialization_and_throw_order() {
    let (ir, b) = body(false);
    let p = plan_constructor(
        &ir,
        MethodRef::new(0),
        "Parent",
        &b,
        &R {
            release: 25,
            attested: 15,
        },
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!(
        "constructor-nonconstant-field-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Fixture.java"),
        format!("public class Fixture extends Parent {{{}}}", p.source),
    )
    .unwrap();
    std::fs::write(dir.join("Driver.java"),r#"class Trace {static int mode;static String log="";}
class Fields {static final int c,d;static final long e;static final String a;static {Trace.log+="I";c=integer("c",3);d=integer("d",5);e=wide();a=text();}static int integer(String s,int n){Trace.log+=s;if(Trace.mode==1&&s.equals("d"))throw new IllegalArgumentException();return n;}static long wide(){Trace.log+="e";return 4294967297L;}static String text(){Trace.log+="a";return Trace.mode==2?null:"value";}}
class Parent {final int c,d;final long e;final String a;Parent(int c,int d,long e,String a){Trace.log+="P";this.c=c;this.d=d;this.e=e;this.a=a;}}
public class Driver {public static void main(String[]args){Trace.mode=Integer.parseInt(args[0]);try{Fixture f=new Fixture();if(f.c!=3||f.d!=5||f.e!=4294967297L||!java.util.Objects.equals(f.a,Trace.mode==2?null:"value"))throw new AssertionError();System.out.println(Trace.log+":"+f.a);}catch(ExceptionInInitializerError t){if(!(t.getCause()instanceof IllegalArgumentException))throw t;System.out.println(Trace.log+":"+t.getCause().getClass().getSimpleName());}}}"#).unwrap();
    run(
        "javac",
        &["--release", "25", "Fixture.java", "Driver.java"],
        &dir,
    );
    let mut after = Vec::new();
    for mode in ["0", "1", "2"] {
        after.extend(run("java", &["-Xverify:all", "Driver", mode], &dir));
    }
    std::fs::write(dir.join("Original.java"),r#"import java.nio.file.*;import java.lang.classfile.*;import java.lang.constant.*;
public class Original {public static void main(String[]args)throws Exception{ClassFile cf=ClassFile.of();ClassDesc fields=ClassDesc.of("Fields");ClassModel model=cf.parse(Path.of("Fixture.class"));int[] reads={0};for(MethodModel m:model.methods())if(m.methodName().equalsString("<init>"))for(CodeElement e:m.code().orElseThrow())if(e instanceof java.lang.classfile.instruction.FieldInstruction f&&f.opcode()==Opcode.GETSTATIC)reads[0]++;if(reads[0]!=4)throw new AssertionError("source field reads erased:"+reads[0]);Files.write(Path.of("Fixture.class"),cf.transformClass(model,(cb,e)->{if(e instanceof MethodModel mm&&mm.methodName().equalsString("<init>"))cb.transformMethod(mm,(mb,me)->{if(me instanceof CodeModel)mb.withCode(c->{c.getstatic(fields,"c",ConstantDescs.CD_int).istore(1);c.getstatic(fields,"d",ConstantDescs.CD_int).istore(2);c.getstatic(fields,"e",ConstantDescs.CD_long).lstore(3);c.getstatic(fields,"a",ConstantDescs.CD_String).astore(5);c.aload(0).iload(1).iload(2).lload(3).aload(5).invokespecial(ClassDesc.of("Parent"),"<init>",MethodTypeDesc.ofDescriptor("(IIJLjava/lang/String;)V")).return_();});else mb.with(me);});else cb.with(e);}));}}"#).unwrap();
    run("javac", &["--release", "25", "Original.java"], &dir);
    run("java", &["Original"], &dir);
    let mut before = Vec::new();
    for mode in ["0", "1", "2"] {
        before.extend(run("java", &["-Xverify:all", "Driver", mode], &dir));
    }
    assert_eq!(before, after);
    assert_eq!(
        String::from_utf8(after).unwrap(),
        "IcdeaP:value\nIcd:IllegalArgumentException\nIcdeaP:null\n"
    );
}
