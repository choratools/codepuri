//! Descriptor-typed pre-chain values and preserved allocation/field effects.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, constructors::*,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{Constant, InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
    types::ManagedType,
    value::{Value, ValueId},
};
struct R {
    kind: u8,
    missing: bool,
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
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        self.initializer(m).map(|i| i.method)
    }
    fn field(&self, f: FieldRef) -> Option<JavaField> {
        (!self.missing).then(|| JavaField {
            owner: "Holder".into(),
            name: if f.index() == 0 { "A" } else { "B" }.into(),
            ty: JavaType::Int,
            is_static: true,
        })
    }
    fn field_access(&self, _: FieldRef) -> Option<u32> {
        Some(9)
    }
    fn instruction_type(&self, _: TypeRef) -> Option<JavaType> {
        Some(JavaType::Array(Box::new(JavaType::Byte)))
    }
}
impl ConstructorResolver for R {
    fn initializer(&self, m: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: if m.index() == 0 { "Fixture" } else { "Base" }.into(),
                name: "<init>".into(),
                parameters: if m.index() == 0 {
                    vec![]
                } else {
                    match self.kind {
                        0 => vec![JavaType::Class("java.lang.String".into())],
                        1 => vec![JavaType::Array(Box::new(JavaType::Array(Box::new(
                            JavaType::Byte,
                        ))))],
                        _ => vec![JavaType::Int, JavaType::Int],
                    }
                },
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: None,
        })
    }
}
fn define(
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
fn body(kind: u8, reverse: bool, extra: bool) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let (_, this) = define(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(0)),
        |v| ManagedInsn::Opaque {
            family_tag: 0,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        },
    );
    let mut ids = vec![];
    let mut args = vec![this];
    match kind {
        0 | 4 => {
            let (id, v) = define(&mut insns, &mut values, ManagedType::Null, |v| {
                ManagedInsn::Const {
                    result: v,
                    value: if kind == 4 {
                        Constant::Int(1)
                    } else {
                        Constant::Null
                    },
                }
            });
            ids.push(id);
            args.push(v);
        }
        1 | 3 => {
            let (id, n) = define(&mut insns, &mut values, ManagedType::Int, |v| {
                ManagedInsn::Const {
                    result: v,
                    value: Constant::Int(if kind == 3 { -1 } else { 11 }),
                }
            });
            ids.push(id);
            let (id, v) = define(
                &mut insns,
                &mut values,
                ManagedType::Array {
                    element: Box::new(ManagedType::Array {
                        element: Box::new(ManagedType::Byte),
                        nullable: true,
                    }),
                    nullable: false,
                },
                |v| ManagedInsn::NewArray {
                    result: v,
                    element: TypeRef::new(0),
                    length: n,
                },
            );
            ids.push(id);
            args.push(v);
        }
        _ => {
            for index in 0..2 {
                let (id, v) = define(&mut insns, &mut values, ManagedType::Int, |v| {
                    ManagedInsn::FieldGet {
                        result: v,
                        field: FieldRef::new(index),
                        object: None,
                    }
                });
                ids.push(id);
                args.push(v);
            }
            if reverse {
                args[1..].reverse();
            }
        }
    }
    if extra {
        insns.insert(ManagedInsn::Opaque {
            family_tag: 99,
            reads: vec![args[1]],
            writes: vec![],
            effects: EffectFlags::NONE,
        });
    }
    ids.push(insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        method: MethodRef::new(1),
        args,
        result: None,
    }));
    ids.push(insns.insert(ManagedInsn::Return { value: None }));
    let mut cfg = Cfg::new(ids[0]);
    let mut prev = cfg.entry();
    for id in &ids[1..] {
        let b = cfg.add_block(*id);
        cfg.add_edge(prev, b, EdgeKind::Unconditional).unwrap();
        prev = b;
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        ParameterBindings {
            receiver: Some(this),
            parameters: vec![],
            names: vec![],
        },
    )
}
fn plan(
    kind: u8,
    reverse: bool,
    extra: bool,
    missing: bool,
) -> Result<ConstructorPlan, backend_java::Unsupported> {
    let (ir, b) = body(kind, reverse, extra);
    plan_constructor(&ir, MethodRef::new(0), "Base", &b, &R { kind, missing })
}
fn run(source: String, name: &str) {
    let dir =
        std::env::temp_dir().join(format!("constructor-prefix-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), source).unwrap();
    let home = "/home/jtjisgod/tools/jdk-17.0.13+11";
    for (tool, args) in [
        ("javac", vec!["Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let o = std::process::Command::new(format!("{home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
}
#[test]
fn contextual_null_selects_original_overload() {
    let p = plan(0, false, false, false).unwrap();
    run(
        format!(
            "class Base {{static int which;Base(String s){{which=1;}}Base(char[]c){{which=2;}}}}public class Fixture extends Base {{{}public static void main(String[]a){{new Fixture();if(which!=1)throw new AssertionError();}}}}",
            p.source
        ),
        "null",
    );
}
#[test]
fn array_argument_retains_rank_allocation_and_throw_order() {
    let p = plan(1, false, false, false).unwrap();
    assert!(p.first_statement.contains("new byte[11][]"));
    run(
        format!(
            "class Base {{static int calls;Base(byte[][]a){{if(a.length!=11||a[0]!=null)throw new AssertionError();calls++;}}}}public class Fixture extends Base {{{}public static void main(String[]a){{new Fixture();if(calls!=1)throw new AssertionError();}}}}",
            p.source
        ),
        "array",
    );
    assert!(plan(1, false, true, false).is_err());
}
#[test]
fn static_field_reads_keep_order_and_class_initialization() {
    let p = plan(2, false, false, false).unwrap();
    run(
        format!(
            "class Trace {{static String log=\"\";}}class Holder {{static int A=init(\"A\"),B=init(\"B\");static int init(String s){{Trace.log+=s;return Trace.log.length();}}}}class Base {{Base(int a,int b){{if(a!=1||b!=2||!Trace.log.equals(\"AB\"))throw new AssertionError();Trace.log+=\"C\";}}}}public class Fixture extends Base {{{}public static void main(String[]a){{new Fixture();if(!Trace.log.equals(\"ABC\"))throw new AssertionError();}}}}",
            p.source
        ),
        "field",
    );
    assert!(plan(2, true, false, false).is_err());
    assert!(plan(2, false, true, false).is_err());
    assert!(plan(2, false, false, true).is_err());
}

#[test]
fn array_failure_prevents_parent_and_nonzero_is_not_null() {
    let (ir, b) = body(3, false, false);
    let p = plan_constructor(
        &ir,
        MethodRef::new(0),
        "Base",
        &b,
        &R {
            kind: 1,
            missing: false,
        },
    )
    .unwrap();
    run(
        format!(
            "class Base {{static int calls;Base(byte[][]a){{calls++;}}}}public class Fixture extends Base {{{}public static void main(String[]a){{try{{new Fixture();throw new AssertionError();}}catch(NegativeArraySizeException expected){{}}if(calls!=0)throw new AssertionError();}}}}",
            p.source
        ),
        "array-throw",
    );
    let (ir, b) = body(4, false, false);
    assert!(
        plan_constructor(
            &ir,
            MethodRef::new(0),
            "Base",
            &b,
            &R {
                kind: 0,
                missing: false
            }
        )
        .is_err()
    );
}
