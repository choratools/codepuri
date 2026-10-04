//! Preserve concrete producer-copy array types through a wider selected join.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    refs::ClassRef,
    types::ManagedType,
    value::Value,
};
struct Resolver;
impl JavaResolver for Resolver {
    fn string(&self, _: managed_ir::refs::StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method(&self, _: managed_ir::refs::MethodRef) -> Option<crate::JavaMethod> {
        None
    }
    fn field(&self, _: managed_ir::refs::FieldRef) -> Option<crate::JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("java.lang.Object".into())
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        a == b || a.reference() && *b == JavaType::Class("java.lang.Object".into())
    }
}
fn fixture(negative: u8) -> (MethodIr<SsaForm>, ValueId, ValueId) {
    let mut i = InsnArena::with_key();
    let mut v = ValueArena::with_key();
    let incoming = i.insert(ManagedInsn::Return { value: None });
    let arr = v.insert(Value::new(
        incoming,
        ManagedType::Array {
            element: Box::new(ManagedType::Double),
            nullable: true,
        },
    ));
    i[incoming] = ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![arr],
        effects: EffectFlags::NONE,
    };
    let m = i.insert(ManagedInsn::Return { value: None });
    let cp = v.insert(Value::new(
        m,
        ManagedType::Array {
            element: Box::new(ManagedType::Double),
            nullable: true,
        },
    ));
    i[m] = ManagedInsn::Move {
        result: cp,
        value: arr,
    };
    let mut cfg = Cfg::new(m);
    let root = cfg.entry();
    let null = i.insert(ManagedInsn::Return { value: None });
    let n = v.insert(Value::new(null, ManagedType::Null));
    i[null] = ManagedInsn::Const {
        result: n,
        value: managed_ir::ops::Constant::Null,
    };
    let nb = cfg.add_block(null);
    let p = i.insert(ManagedInsn::Return { value: None });
    let phi = v.insert(Value::new(p, ManagedType::Unknown));
    let pb = cfg.add_block(p);
    let e = cfg.add_edge(root, pb, EdgeKind::Unconditional).unwrap();
    let ne = cfg.add_edge(nb, pb, EdgeKind::Unconditional).unwrap();
    i[p] = ManagedInsn::Phi {
        result: phi,
        inputs: vec![(e, cp), (ne, n)],
    };
    let length = i.insert(ManagedInsn::Return { value: None });
    let size = v.insert(Value::new(length, ManagedType::Int));
    i[length] = ManagedInsn::ArrayLength {
        result: size,
        array: cp,
    };
    i.insert(ManagedInsn::Return { value: Some(phi) });
    if negative == 1 {
        i.insert(ManagedInsn::Opaque {
            family_tag: 7,
            reads: vec![cp],
            writes: vec![],
            effects: EffectFlags::NONE,
        });
    }
    if negative == 2 {
        let k = i.insert(ManagedInsn::Return { value: None });
        let scalar = v.insert(Value::new(k, ManagedType::Int));
        i[k] = ManagedInsn::Const {
            result: scalar,
            value: managed_ir::ops::Constant::Int(3),
        };
        i[p] = ManagedInsn::Phi {
            result: phi,
            inputs: vec![(e, cp), (ne, scalar)],
        };
    }
    if negative == 3 {
        i.insert(ManagedInsn::Opaque {
            family_tag: 7,
            reads: vec![phi],
            writes: vec![],
            effects: EffectFlags::NONE,
        });
    }
    (MethodIr::<SsaForm>::from_parts(cfg, i, v), phi, cp)
}
#[test]
fn array_copy_remains_array_when_selected_phi_is_object() {
    let (ir, phi, copy) = fixture(0);
    let object = JavaType::Class("java.lang.Object".into());
    let plan =
        checked_phi_plan(&ir, phi, &object, &object, &Hierarchy::default(), &Resolver).unwrap();
    assert!(plan.locals.contains(&phi));
    assert!(!plan.locals.contains(&copy));
    assert_eq!(
        evidence(&ir, copy, &Resolver).unwrap(),
        ValueType::Known(JavaType::Array(Box::new(JavaType::Double)))
    );
    assert_eq!(
        declaration_plan(&ir, &object, &Resolver).unwrap().get(&phi),
        Some(&object)
    );
    for mode in 1..=3 {
        let (ir, phi, _) = fixture(mode);
        assert!(
            checked_phi_plan(&ir, phi, &object, &object, &Hierarchy::default(), &Resolver).is_err(),
            "mode {mode}"
        );
    }
}

#[test]
fn checked_array_join_identity_compiles_and_executes() {
    let (ir, phi, producer) = fixture(0);
    let object = JavaType::Class("java.lang.Object".into());
    let proof =
        checked_phi_plan(&ir, phi, &object, &object, &Hierarchy::default(), &Resolver).unwrap();
    assert!(!proof.locals.contains(&producer));
    let producer_type = match evidence(&ir, producer, &Resolver).unwrap() {
        ValueType::Known(t) => t.source().unwrap(),
        _ => panic!("concrete producer required"),
    };
    let dir = std::env::temp_dir().join(format!("decomphify-array-join-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = format!(
        "public class ArrayJoin {{ static Object select(boolean yes,double[] input) {{ {producer_type} copy=input; Object selected; if(yes){{ int length=copy.length; selected=copy; }}else{{selected=null;}} return selected; }} public static void main(String[] args){{double[] original={{1,2}};if(select(true,original)!=original||select(false,original)!=null)throw new AssertionError(); boolean threw=false;try{{select(true,null);}}catch(NullPointerException expected){{threw=true;}}if(!threw||select(false,null)!=null)throw new AssertionError();}} }}"
    );
    std::fs::write(dir.join("ArrayJoin.java"), source).unwrap();
    let javac = std::process::Command::new("/home/jtjisgod/tools/jdk-17.0.13+11/bin/javac")
        .arg("--release")
        .arg("17")
        .arg(dir.join("ArrayJoin.java"))
        .output()
        .unwrap();
    assert!(
        javac.status.success(),
        "{}",
        String::from_utf8_lossy(&javac.stderr)
    );
    let run = std::process::Command::new("/home/jtjisgod/tools/jdk-17.0.13+11/bin/java")
        .arg("-cp")
        .arg(&dir)
        .arg("ArrayJoin")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
