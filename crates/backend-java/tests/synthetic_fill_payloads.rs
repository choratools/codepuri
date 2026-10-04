//! Arena-only folded fill-array payload definitions retain exact bit evidence.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, SourceResult, emit_method,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{Constant, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};
struct R {
    target: Option<JavaType>,
}
impl JavaResolver for R {
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("java.lang.Object".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
    }
    fn instruction_type(&self, _: TypeRef) -> Option<JavaType> {
        self.target.clone()
    }
}
fn def(
    i: &mut InsnArena,
    v: &mut ValueArena,
    t: ManagedType,
    f: impl FnOnce(ValueId) -> ManagedInsn,
) -> (InsnId, ValueId) {
    let mut value = None;
    let id = i.insert_with_key(|id| {
        let x = v.insert(Value::new(id, t));
        value = Some(x);
        f(x)
    });
    (id, value.unwrap())
}
fn fixture(
    component: ManagedType,
    payloads: &[Constant],
    filled: bool,
    escape: bool,
) -> (MethodIr<SsaForm>, ParameterBindings) {
    fixture_placement(component, payloads, filled, escape, false)
}
fn fixture_placement(
    component: ManagedType,
    payloads: &[Constant],
    filled: bool,
    escape: bool,
    prelude: bool,
) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut i = InsnArena::default();
    let mut v = ValueArena::default();
    let (_, length) = def(&mut i, &mut v, ManagedType::Int, |x| ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![x],
        effects: EffectFlags::NONE,
    });
    let array_type = ManagedType::Array {
        element: Box::new(component.clone()),
        nullable: true,
    };
    let mut ids = Vec::new();
    let mut args = Vec::new();
    let mut array = None;
    if !filled {
        let (id, a) = def(&mut i, &mut v, array_type.clone(), |result| {
            ManagedInsn::NewArray {
                result,
                element: TypeRef::new(0),
                length,
            }
        });
        ids.push(id);
        array = Some(a);
    }
    for payload in payloads {
        let (id, x) = def(&mut i, &mut v, component.clone(), |result| {
            ManagedInsn::Const {
                result,
                value: *payload,
            }
        });
        if filled {
            ids.push(id);
        } else if prelude {
            ids.insert(args.len(), id);
        }
        args.push(x);
    }
    if filled {
        let (id, a) = def(&mut i, &mut v, array_type, |result| {
            ManagedInsn::NewFilledArray {
                result: Some(result),
                target: TypeRef::new(0),
                elements: args.clone(),
            }
        });
        ids.push(id);
        array = Some(a);
    } else {
        ids.push(i.insert(ManagedInsn::FillArray {
            array: array.unwrap(),
            values: args.clone(),
        }));
    }
    ids.push(i.insert(ManagedInsn::Return {
        value: Some(if escape { args[0] } else { array.unwrap() }),
    }));
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
            parameters: vec![length],
            ..Default::default()
        },
    )
}
fn sig(name: &str, component: JavaType, escape: bool) -> JavaMethod {
    JavaMethod {
        owner: "Fixture".into(),
        name: name.into(),
        parameters: vec![JavaType::Int],
        return_type: if escape {
            JavaType::Int
        } else {
            JavaType::Array(Box::new(component))
        },
        is_static: true,
        throws: vec![],
    }
}
fn source(name: &str, managed: ManagedType, java: JavaType, payload: &[Constant]) -> String {
    let (ir, b) = fixture(managed, payload, false, false);
    match emit_method(
        &ir,
        &sig(name, java.clone(), false),
        &b,
        &R {
            target: Some(JavaType::Array(Box::new(java))),
        },
    ) {
        SourceResult::Complete { source, .. } => source,
        error => panic!("{error:?}"),
    }
}
#[test]
fn unsigned_payload_words_become_exact_signed_literals() {
    let b = source(
        "bytes",
        ManagedType::Byte,
        JavaType::Byte,
        &[Constant::Int(255), Constant::Int(216), Constant::Int(128)],
    );
    assert!(b.contains("v0[0] = -1;"), "{b}");
    assert!(b.contains("= -40;"));
    assert!(b.contains("= -128;"));
    let s = source(
        "shorts",
        ManagedType::Short,
        JavaType::Short,
        &[Constant::Int(65535), Constant::Int(32768)],
    );
    assert!(s.contains("= -1;"));
    assert!(s.contains("= -32768;"));
}
#[test]
fn width_descriptor_and_non_payload_consumers_stay_denied() {
    for (managed, java, payload, filled, escape, target) in [
        (
            ManagedType::Byte,
            JavaType::Byte,
            Constant::Int(256),
            false,
            false,
            Some(JavaType::Array(Box::new(JavaType::Byte))),
        ),
        (
            ManagedType::Short,
            JavaType::Short,
            Constant::Int(65536),
            false,
            false,
            Some(JavaType::Array(Box::new(JavaType::Short))),
        ),
        (
            ManagedType::Byte,
            JavaType::Byte,
            Constant::Long(255),
            false,
            false,
            Some(JavaType::Array(Box::new(JavaType::Byte))),
        ),
        (
            ManagedType::Byte,
            JavaType::Byte,
            Constant::Int(255),
            false,
            true,
            Some(JavaType::Array(Box::new(JavaType::Byte))),
        ),
        (
            ManagedType::Byte,
            JavaType::Byte,
            Constant::Int(255),
            true,
            false,
            Some(JavaType::Array(Box::new(JavaType::Byte))),
        ),
        (
            ManagedType::Byte,
            JavaType::Byte,
            Constant::Int(255),
            false,
            false,
            None,
        ),
        (
            ManagedType::Byte,
            JavaType::Byte,
            Constant::Int(255),
            false,
            false,
            Some(JavaType::Array(Box::new(JavaType::Short))),
        ),
        (
            ManagedType::Reference(ClassRef::new(0)),
            JavaType::Class("java.lang.Object".into()),
            Constant::Int(1),
            true,
            false,
            Some(JavaType::Array(Box::new(JavaType::Class(
                "java.lang.Object".into(),
            )))),
        ),
        (
            ManagedType::Long,
            JavaType::Long,
            Constant::Long(1),
            true,
            false,
            Some(JavaType::Array(Box::new(JavaType::Long))),
        ),
        (
            ManagedType::Double,
            JavaType::Double,
            Constant::DoubleBits(0),
            true,
            false,
            Some(JavaType::Array(Box::new(JavaType::Double))),
        ),
    ] {
        let (ir, b) = fixture(managed, &[payload], filled, escape);
        assert!(
            matches!(
                emit_method(&ir, &sig("bad", java, escape), &b, &R { target }),
                SourceResult::Unsupported(_)
            ),
            "accepted {payload:?}, filled={filled}, escape={escape}"
        );
    }
}
fn filled_float_source() -> String {
    let (ir, bindings) = fixture(
        ManagedType::Float,
        &[
            Constant::FloatBits(0x8000_0000),
            Constant::FloatBits(0x7fc0_0042),
        ],
        true,
        false,
    );
    let mut instructions = ir.instructions().clone();
    let mut values = ir.values().clone();
    for (id, insn) in instructions.iter_mut() {
        if let ManagedInsn::Const {
            result,
            value: payload @ Constant::FloatBits(_),
        } = insn
        {
            let Constant::FloatBits(bits) = *payload else {
                unreachable!()
            };
            *payload = Constant::Int(bits as i32);
            values[*result] = Value::new(id, ManagedType::Int);
        }
    }
    let original = ir.cfg();
    let mut at = original.entry();
    let mut cfg = Cfg::new(*original.block(at).unwrap().payload());
    let mut tail = cfg.entry();
    while let Some(edge) = original.block(at).unwrap().outgoing().first() {
        at = original.edge(*edge).unwrap().target();
        let next = cfg.add_block(*original.block(at).unwrap().payload());
        cfg.add_edge(tail, next, EdgeKind::Unconditional).unwrap();
        tail = next;
    }
    let ir = MethodIr::<SsaForm>::from_parts(cfg, instructions, values);
    match emit_method(
        &ir,
        &sig("filledFloats", JavaType::Float, false),
        &bindings,
        &R {
            target: Some(JavaType::Array(Box::new(JavaType::Float))),
        },
    ) {
        SourceResult::Complete { source, .. } => source,
        result => panic!("{result:?}"),
    }
}
#[test]
fn category_one_register_bits_use_exact_filled_array_descriptor() {
    let source = filled_float_source();
    assert_eq!(source.matches("java.lang.Float.intBitsToFloat(").count(), 2);
    assert!(source.contains("new float[]"));
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
fn java_runtime_preserves_signed_payloads_float_bits_and_allocation_failures() {
    let mut methods = source(
        "bytes",
        ManagedType::Byte,
        JavaType::Byte,
        &[Constant::Int(255), Constant::Int(216), Constant::Int(255)],
    );
    methods += &source(
        "shorts",
        ManagedType::Short,
        JavaType::Short,
        &[Constant::Int(65535), Constant::Int(32768)],
    );
    methods += &source(
        "chars",
        ManagedType::Char,
        JavaType::Char,
        &[Constant::Int(65535)],
    );
    methods += &source(
        "floats",
        ManagedType::Float,
        JavaType::Float,
        &[
            Constant::Int(0x80000000_u32 as i32),
            Constant::Int(0x7fc00042),
        ],
    );
    methods += &source(
        "doubles",
        ManagedType::Double,
        JavaType::Double,
        &[
            Constant::Long(0x8000000000000000_u64 as i64),
            Constant::Long(0x7ff8000000000042),
        ],
    );
    methods += &filled_float_source();
    let dir = std::env::temp_dir().join(format!("array-payload-constants-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Fixture.java"),
        format!("public class Fixture {{{methods}}}"),
    )
    .unwrap();
    std::fs::write(dir.join("Driver.java"),r#"public class Driver {public static void main(String[]args){for(int n:new int[]{3,-1,1,100000000})try{System.out.println(java.util.Arrays.toString(Fixture.bytes(n)));}catch(NegativeArraySizeException|ArrayIndexOutOfBoundsException|OutOfMemoryError t){System.out.println(t.getClass().getSimpleName());}if(!java.util.Arrays.equals(Fixture.shorts(2),new short[]{-1,-32768})||Fixture.chars(1)[0]!=65535)throw new AssertionError();float[] f=Fixture.floats(2),filled=Fixture.filledFloats(0);if(Float.floatToRawIntBits(filled[0])!=0x80000000||Float.floatToRawIntBits(filled[1])!=0x7fc00042)throw new AssertionError("category-one filled float bits");double[]d=Fixture.doubles(2);if(Float.floatToRawIntBits(f[0])!=0x80000000||Float.floatToRawIntBits(f[1])!=0x7fc00042||Double.doubleToRawLongBits(d[0])!=0x8000000000000000L||Double.doubleToRawLongBits(d[1])!=0x7ff8000000000042L)throw new AssertionError("bit patterns");}}"#).unwrap();
    run(
        "javac",
        &["--release", "17", "Fixture.java", "Driver.java"],
        &dir,
    );
    let after = run("java", &["-Xmx16m", "-Xverify:all", "Driver"], &dir);
    std::fs::write(dir.join("Original.java"),r#"import java.nio.file.*;import java.lang.classfile.*;import java.lang.constant.*;
public class Original {public static void main(String[]args)throws Exception{ClassFile cf=ClassFile.of();Path p=Path.of("Fixture.class");Files.write(p,cf.transformClass(cf.parse(p),(cb,e)->{if(e instanceof MethodModel mm&&mm.methodName().equalsString("bytes"))cb.transformMethod(mm,(mb,me)->{if(me instanceof CodeModel)mb.withCode(c->{Label valid=c.newLabel();c.iload(0).newarray(TypeKind.BYTE).astore(1);c.aload(1).arraylength().iconst_3().if_icmpge(valid);c.new_(ClassDesc.of("java.lang.ArrayIndexOutOfBoundsException")).dup().invokespecial(ClassDesc.of("java.lang.ArrayIndexOutOfBoundsException"),"<init>",MethodTypeDesc.ofDescriptor("()V")).athrow();c.labelBinding(valid);for(int i=0;i<3;i++)c.aload(1).loadConstant(i).loadConstant(i==1?216:255).bastore();c.aload(1).areturn();});else mb.with(me);});else cb.with(e);}));}}"#).unwrap();
    run("javac", &["--release", "25", "Original.java"], &dir);
    run("java", &["Original"], &dir);
    let before = run("java", &["-Xmx16m", "-Xverify:all", "Driver"], &dir);
    assert_eq!(before, after);
    assert_eq!(
        String::from_utf8(after).unwrap(),
        "[-1, -40, -1]\nNegativeArraySizeException\nArrayIndexOutOfBoundsException\nOutOfMemoryError\n"
    );
}

#[test]
fn pure_entry_payload_prelude_does_not_require_premature_array_allocation() {
    let (ir, bindings) = fixture_placement(
        ManagedType::Byte,
        &[Constant::Int(255), Constant::Int(225)],
        false,
        false,
        true,
    );
    let resolver = R {
        target: Some(JavaType::Array(Box::new(JavaType::Byte))),
    };
    let method = sig("preludeBytes", JavaType::Byte, false);
    let SourceResult::Complete { source, .. } = emit_method(&ir, &method, &bindings, &resolver)
    else {
        panic!("pure payload prelude must emit");
    };
    assert!(source.find("byte v0 = -1;").unwrap() < source.find("new byte[p0]").unwrap());
    assert!(source.contains(".length < 2"));
    let dir = std::env::temp_dir().join(format!("synthetic-prelude-array-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Prelude.java"),format!("public class Prelude {{{source} public static void main(String[] a){{ if(!java.util.Arrays.equals(preludeBytes(2),new byte[]{{-1,-31}}))throw new AssertionError(); try{{preludeBytes(-1);throw new AssertionError();}}catch(NegativeArraySizeException ok){{}} try{{preludeBytes(1);throw new AssertionError();}}catch(ArrayIndexOutOfBoundsException ok){{}} try{{preludeBytes(100000000);throw new AssertionError();}}catch(OutOfMemoryError ok){{}} }} }}")).unwrap();
    run("javac", &["--release", "17", "Prelude.java"], &dir);
    run("java", &["-Xmx16m", "-Xverify:all", "Prelude"], &dir);
}
