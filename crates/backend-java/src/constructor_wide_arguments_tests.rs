//! Physical-wide initializer words are descriptor-normalized at every boundary.
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
    fn push(&mut self, insn: ManagedInsn) {
        self.order.push(self.insns.insert(insn));
    }
    fn finish(self) -> MethodIr<SsaForm> {
        let mut cfg = Cfg::new(self.order[0]);
        let mut last = cfg.entry();
        for id in &self.order[1..] {
            let next = cfg.add_block(*id);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}

struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Fixture".into())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        Some(if r.index() == 3 { 9 } else { 1 })
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        if r.index() == 3 {
            Some(JavaMethod {
                owner: "Fixture".into(),
                name: "tick".into(),
                is_static: true,
                return_type: JavaType::Long,
                parameters: vec![JavaType::Long],
                throws: vec![],
            })
        } else {
            self.initializer(r).map(|i| i.method)
        }
    }
}
impl ConstructorResolver for Resolver {
    fn initializer(&self, r: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: if r.index() == 2 {
                    "java.lang.Object"
                } else {
                    "Fixture"
                }
                .into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: match r.index() {
                    0 => vec![JavaType::Int],
                    1 => vec![JavaType::Long, JavaType::Long, JavaType::Int],
                    2 => vec![],
                    _ => return None,
                },
                throws: vec![],
            },
            delegates_to: (r.index() == 0).then_some(MethodRef::new(1)),
        })
    }
}
fn fixture(mode: u8) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut b = Builder::default();
    let receiver = b.parameter();
    let p = b.value(ManagedType::Int, |v| ManagedInsn::Opaque {
        family_tag: 32769,
        reads: vec![],
        writes: vec![v],
        effects: EffectFlags::NONE,
    });
    b.order.pop();
    let mut args = vec![receiver];
    let mut results = vec![];
    for n in 1..=2 {
        if mode == 4 && n == 2 {
            continue;
        }
        let literal = b.value(ManagedType::Long, |result| ManagedInsn::Const {
            result,
            value: Constant::Long(n),
        });
        let v = if mode == 3 {
            literal
        } else {
            b.value(ManagedType::Long, |result| ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(result),
                method: MethodRef::new(3),
                args: vec![literal, literal],
            })
        };
        results.push(v);
    }
    match mode {
        1 => args.extend([results[0], results[1], results[1], results[1]]),
        2 => args.extend([results[0], results[0], results[1]]),
        3 => args.extend([results[0], results[0]]),
        4 => args.extend([results[0], results[0], results[0], results[0]]),
        _ => args.extend([results[0], results[0], results[1], results[1]]),
    };
    args.push(p);
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args,
    });
    b.push(ManagedInsn::Return { value: None });
    (
        b.finish(),
        ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![p],
            names: vec![],
        },
    )
}
#[test]
fn wide_chain_words_preserve_effect_order_and_initializer_evidence() {
    let (ir, bindings) = fixture(0);
    let plan = plan_constructor(
        &ir,
        MethodRef::new(0),
        "java.lang.Object",
        &bindings,
        &Resolver,
    )
    .unwrap();
    assert_eq!(plan.chain, ChainKind::This);
    let input = ConstructorInput {
        reference: MethodRef::new(0),
        ir: &ir,
        bindings: &bindings,
    };
    let evidence =
        constructor_initialization_evidence(&input, "java.lang.Object", &[], &Resolver).unwrap();
    assert!(
        evidence
            .normal_chains
            .contains(&(ChainKind::This, MethodRef::new(1)))
    );
    let java = format!(
        "public class Fixture {{static String trace=\"\";static long tick(long n){{trace+=n;return n;}} Fixture(long a,long b,int c){{if(a!=1||b!=2||c!=7)throw new AssertionError();trace+=\"C\";}} {} public static void main(String[]args){{new Fixture(7);if(!trace.equals(\"12C\"))throw new AssertionError(trace);}}}}",
        plan.source
    );
    let dir = std::env::temp_dir().join(format!("wide-constructor-chain-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), java).unwrap();
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (tool, args) in [
        ("javac", vec!["--release", "17", "Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let output = std::process::Command::new(format!("{home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn wide_chain_rejects_distinct_high_word_missing_word_and_duplicate_effect() {
    for mode in [1, 2] {
        let (ir, bindings) = fixture(mode);
        assert!(
            plan_constructor(
                &ir,
                MethodRef::new(0),
                "java.lang.Object",
                &bindings,
                &Resolver
            )
            .is_err()
        );
        let input = ConstructorInput {
            reference: MethodRef::new(0),
            ir: &ir,
            bindings: &bindings,
        };
        assert!(
            constructor_initialization_evidence(&input, "java.lang.Object", &[], &Resolver)
                .is_err()
        );
    }
    let (ir, bindings) = fixture(3); // Same pure SSA value in two LOGICAL wide parameters remains two args.
    let (duplicate, duplicate_bindings) = fixture(4);
    assert!(
        plan_constructor(
            &duplicate,
            MethodRef::new(0),
            "java.lang.Object",
            &duplicate_bindings,
            &Resolver
        )
        .is_err()
    );
    let result = plan_constructor(
        &ir,
        MethodRef::new(0),
        "java.lang.Object",
        &bindings,
        &Resolver,
    );
    assert!(result.is_ok(), "{result:?}");
}

struct SamsungShape;
impl JavaResolver for SamsungShape {
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Fixture".into())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        self.initializer(r).map(|i| i.method)
    }
}
impl ConstructorResolver for SamsungShape {
    fn initializer(&self, r: MethodRef) -> Option<Initializer> {
        let array = JavaType::Array(Box::new(JavaType::Byte));
        Some(Initializer {
            method: JavaMethod {
                owner: if r.index() == 2 {
                    "java.lang.Object"
                } else {
                    "Fixture"
                }
                .into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: match r.index() {
                    0 => vec![JavaType::Int, JavaType::Int, array],
                    1 => vec![JavaType::Int, JavaType::Int, JavaType::Long, array],
                    2 => vec![],
                    _ => return None,
                },
                throws: vec![],
            },
            delegates_to: (r.index() == 0).then_some(MethodRef::new(1)),
        })
    }
}
#[test]
fn actual_samsung_ctor111_to110_layout_preserves_long_and_following_array() {
    // La4/a$c #111 off6 -> #110: [this,int,int,longlow,longhigh,byte[]].
    let mut b = Builder::default();
    let receiver = b.parameter();
    let a = b.parameter();
    let c = b.parameter();
    let array = b.value(
        ManagedType::Array {
            element: Box::new(ManagedType::Byte),
            nullable: true,
        },
        |v| ManagedInsn::Opaque {
            family_tag: 32769,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        },
    );
    b.order.pop();
    let long = b.value(ManagedType::Long, |result| ManagedInsn::Const {
        result,
        value: Constant::Long(-1),
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![receiver, a, c, long, long, array],
    });
    b.push(ManagedInsn::Return { value: None });
    let ir = b.finish();
    let bindings = ParameterBindings {
        receiver: Some(receiver),
        parameters: vec![a, c, array],
        names: vec![],
    };
    let plan = plan_constructor(
        &ir,
        MethodRef::new(0),
        "java.lang.Object",
        &bindings,
        &SamsungShape,
    )
    .unwrap();
    let input = ConstructorInput {
        reference: MethodRef::new(0),
        ir: &ir,
        bindings: &bindings,
    };
    assert!(
        constructor_initialization_evidence(&input, "java.lang.Object", &[], &SamsungShape)
            .unwrap()
            .normal_chains
            .contains(&(ChainKind::This, MethodRef::new(1)))
    );
    let java = format!(
        "public class Fixture {{final int a,b;final long c;final byte[] d;Fixture(int a,int b,long c,byte[]d){{this.a=a;this.b=b;this.c=c;this.d=d;}} {} public static void main(String[]args){{byte[]d={{4}};Fixture f=new Fixture(2,3,d);if(f.a!=2||f.b!=3||f.c!=-1||f.d!=d)throw new AssertionError();}}}}",
        plan.source
    );
    let dir = std::env::temp_dir().join(format!("samsung-wide-chain-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), java).unwrap();
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (tool, args) in [
        ("javac", vec!["--release", "17", "Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let output = std::process::Command::new(format!("{home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
