//! Ordinary constructed throw objects in initialized constructor branch suffixes.
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
struct R;
impl JavaResolver for R {
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || (actual == &JavaType::Class("java.lang.IllegalArgumentException".into())
                && matches!(expected, JavaType::Class(name) if name == "java.lang.Throwable" || name == "java.lang.RuntimeException"))
    }
    fn class(&self, c: ClassRef) -> Option<String> {
        Some(
            match c.index() {
                0 => "SimpleBigDecimal",
                1 => "java.math.BigInteger",
                2 => "java.lang.IllegalArgumentException",
                3 => "java.lang.String",
                _ => return None,
            }
            .into(),
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        Some("scale may not be negative".encode_utf16().collect())
    }
    fn field(&self, f: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "SimpleBigDecimal".into(),
            name: if f.index() == 0 { "bigInt" } else { "scale" }.into(),
            ty: if f.index() == 0 {
                JavaType::Class("java.math.BigInteger".into())
            } else {
                JavaType::Int
            },
            is_static: false,
        })
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        self.initializer(m).map(|i| i.method)
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(1)
    }
}
impl ConstructorResolver for R {
    fn initializer(&self, m: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: match m.index() {
                    0 => "SimpleBigDecimal",
                    1 => "java.lang.Object",
                    2 => "java.lang.IllegalArgumentException",
                    3 => "java.lang.IllegalStateException",
                    _ => return None,
                }
                .into(),
                name: "<init>".into(),
                parameters: match m.index() {
                    0 => vec![
                        JavaType::Class("java.math.BigInteger".into()),
                        JavaType::Int,
                    ],
                    1 => vec![],
                    _ => vec![JavaType::Class("java.lang.String".into())],
                },
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: None,
        })
    }
}
impl TrustedConstructorResolver for R {
    fn constructor_class(&self, n: &str) -> Option<ConstructorClass> {
        Some(ConstructorClass {
            name: n.into(),
            superclass: if n == "java.lang.Object" {
                None
            } else {
                Some("java.lang.Object".into())
            },
            access_flags: 1,
        })
    }
    fn constructor_access(&self, _: MethodRef) -> Option<u32> {
        Some(1)
    }
    fn declared_constructors(&self, _: &str) -> Option<Vec<MethodRef>> {
        Some(vec![MethodRef::new(0)])
    }
}
fn define(
    insns: &mut InsnArena,
    values: &mut ValueArena,
    ty: ManagedType,
    f: impl FnOnce(ValueId) -> ManagedInsn,
) -> (InsnId, ValueId) {
    let mut v = None;
    let id = insns.insert_with_key(|id| {
        let value = values.insert(Value::new(id, ty));
        v = Some(value);
        f(value)
    });
    (id, v.unwrap())
}
fn param(insns: &mut InsnArena, values: &mut ValueArena, ty: ManagedType) -> ValueId {
    define(insns, values, ty, |v| ManagedInsn::Opaque {
        family_tag: 0,
        reads: vec![],
        writes: vec![v],
        effects: EffectFlags::NONE,
    })
    .1
}
fn body(mutation: u8) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let this = param(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(0)),
    );
    let big = param(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(1)),
    );
    let scale = param(&mut insns, &mut values, ManagedType::Int);
    let init = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![this],
    });
    let (zero, z) = define(&mut insns, &mut values, ManagedType::Int, |v| {
        ManagedInsn::Const {
            result: v,
            value: Constant::Zero,
        }
    });
    let (cmp, condition) = define(&mut insns, &mut values, ManagedType::Boolean, |v| {
        ManagedInsn::Compare {
            op: CompareOp::LessThan,
            result: v,
            lhs: scale,
            rhs: z,
        }
    });
    let branch = insns.insert(ManagedInsn::Return { value: None });
    let store0 = insns.insert(ManagedInsn::FieldPut {
        object: Some(this),
        field: FieldRef::new(0),
        value: big,
    });
    let store1 = insns.insert(ManagedInsn::FieldPut {
        object: Some(this),
        field: FieldRef::new(1),
        value: scale,
    });
    let ret = insns.insert(ManagedInsn::Return { value: None });
    let (alloc, object) = define(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(2)),
        |v| {
            if mutation == 2 {
                ManagedInsn::Opaque {
                    family_tag: 0,
                    reads: vec![],
                    writes: vec![v],
                    effects: EffectFlags::NONE,
                }
            } else {
                ManagedInsn::NewInstance {
                    result: v,
                    class: ClassRef::new(2),
                }
            }
        },
    );
    let (literal, text) = define(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(3)),
        |v| ManagedInsn::Const {
            result: v,
            value: Constant::String(StringRef::new(0)),
        },
    );
    let object_init = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(if mutation == 3 { 3 } else { 2 }),
        args: vec![object, text],
    });
    let throw = insns.insert(ManagedInsn::Throw { value: object });
    let mut cfg = Cfg::new(init);
    let mut prev = cfg.entry();
    for id in [zero, cmp, branch] {
        let b = cfg.add_block(id);
        cfg.add_edge(prev, b, EdgeKind::Unconditional).unwrap();
        prev = b;
    }
    let b = prev;
    let valid = cfg.add_block(if mutation == 6 { store1 } else { store0 });
    let t = cfg.add_block(alloc);
    let yes = cfg.add_edge(b, t, EdgeKind::BranchTrue).unwrap();
    let no = cfg.add_edge(b, valid, EdgeKind::BranchFalse).unwrap();
    insns[branch] = ManagedInsn::Branch {
        condition: Some(condition),
        then_edge: yes,
        else_edge: Some(no),
    };
    prev = valid;
    if mutation != 6 {
        let s = cfg.add_block(store1);
        cfg.add_edge(prev, s, EdgeKind::Unconditional).unwrap();
        prev = s;
    }
    let r = cfg.add_block(ret);
    cfg.add_edge(prev, r, EdgeKind::Unconditional).unwrap();
    prev = t;
    let mut path = vec![literal];
    if mutation == 1 {
        path.push(insns.insert(ManagedInsn::Opaque {
            family_tag: 99,
            reads: vec![object],
            writes: vec![],
            effects: EffectFlags::NONE,
        }));
    }
    path.push(object_init);
    if mutation == 4 {
        path.push(insns.insert(insns[object_init].clone()));
    }
    if mutation == 5 {
        path.push(insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(1),
            args: vec![this],
        }));
    }
    path.push(throw);
    for id in path {
        let n = cfg.add_block(id);
        cfg.add_edge(prev, n, EdgeKind::Unconditional).unwrap();
        prev = n;
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        ParameterBindings {
            receiver: Some(this),
            parameters: vec![big, scale],
            names: vec!["value".into(), "scale".into()],
        },
    )
}
fn plans(
    ir: &MethodIr<SsaForm>,
    b: &ParameterBindings,
) -> Result<Vec<ConstructorPlan>, backend_java::Unsupported> {
    plan_constructor_family(
        "SimpleBigDecimal",
        &[ConstructorInput {
            reference: MethodRef::new(0),
            ir,
            bindings: b,
        }],
        &[
            BlankFinal {
                reference: FieldRef::new(0),
                writes: [(MethodRef::new(0), 1)].into(),
            },
            BlankFinal {
                reference: FieldRef::new(1),
                writes: [(MethodRef::new(0), 1)].into(),
            },
        ],
        &R,
    )
}
#[test]
fn exact_branch_throw_identity_message_and_normal_final_assignment() {
    let (ir, b) = body(0);
    let p = plans(&ir, &b).unwrap().remove(0);
    let source = format!(
        "public class SimpleBigDecimal {{private final java.math.BigInteger bigInt;private final int scale;{}public static void main(String[]a){{java.math.BigInteger value=java.math.BigInteger.TEN;SimpleBigDecimal x=new SimpleBigDecimal(value,2);if(x.bigInt!=value||x.scale!=2)throw new AssertionError();try{{new SimpleBigDecimal(value,-1);throw new AssertionError();}}catch(IllegalArgumentException e){{if(!e.getMessage().equals(\"scale may not be negative\"))throw new AssertionError();}}}}}}",
        p.source
    );
    let dir = std::env::temp_dir().join(format!("constructor-throw-suffix-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SimpleBigDecimal.java"), source).unwrap();
    for (tool, args) in [
        ("javac", vec!["SimpleBigDecimal.java"]),
        ("java", vec!["-cp", ".", "SimpleBigDecimal"]),
    ] {
        let o =
            std::process::Command::new(format!("/home/jtjisgod/tools/jdk-17.0.13+11/bin/{tool}"))
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
}
#[test]
fn malformed_or_escaping_initializers_and_missing_final_path_reject() {
    for mutation in 1..=6 {
        let (ir, b) = body(mutation);
        assert!(plans(&ir, &b).is_err(), "accepted mutation{mutation}");
    }
}
