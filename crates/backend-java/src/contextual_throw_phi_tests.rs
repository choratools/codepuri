//! Selected throwable locals retain concrete constructor input types.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{JavaField, JavaMethod};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    method::{InsnArena, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};

fn io() -> JavaType {
    JavaType::Class("java.io.IOException".into())
}
fn child() -> JavaType {
    JavaType::Class("Fixture.Child".into())
}
struct Resolver;
impl JavaResolver for Resolver {
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "Fixture.Child".into(),
            name: "<init>".into(),
            is_static: false,
            parameters: vec![],
            return_type: JavaType::Void,
            throws: vec![],
        })
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        match r.index() {
            0 => Some("java.io.IOException".into()),
            1 => Some("Fixture.Child".into()),
            _ => None,
        }
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || (*actual == child() && *expected == io())
            || ([io(), child()].contains(actual)
                && *expected == JavaType::Class("java.lang.Throwable".into()))
    }
}
fn fixture(bad: bool) -> (MethodIr<SsaForm>, ValueId, ValueId) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let entry = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(entry);
    let new_id = insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(1))));
        ManagedInsn::NewInstance {
            result: v,
            class: ClassRef::new(1),
        }
    });
    let source = match insns[new_id] {
        ManagedInsn::NewInstance { result, .. } => result,
        _ => unreachable!(),
    };
    let mut copied = None;
    let cp_id = insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(1))));
        copied = Some(v);
        ManagedInsn::Move {
            result: v,
            value: source,
        }
    });
    let copied = copied.unwrap();
    insns.insert(ManagedInsn::Invoke {
        result: None,
        kind: managed_ir::ops::InvokeKind::Special,
        method: MethodRef::new(0),
        args: vec![copied],
    });
    let zero_id = insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Zero));
        ManagedInsn::Const {
            result: v,
            value: Constant::Zero,
        }
    });
    let zero = match insns[zero_id] {
        ManagedInsn::Const { result, .. } => result,
        _ => unreachable!(),
    };
    let y = cfg.add_block(cp_id);
    let n = cfg.add_block(zero_id);
    cfg.add_edge(cfg.entry(), y, EdgeKind::BranchTrue).unwrap();
    cfg.add_edge(cfg.entry(), n, EdgeKind::BranchFalse).unwrap();
    let join_id = insns.insert(ManagedInsn::Return { value: None });
    let join = cfg.add_block(join_id);
    let ye = cfg.add_edge(y, join, EdgeKind::Unconditional).unwrap();
    let ne = cfg.add_edge(n, join, EdgeKind::Unconditional).unwrap();
    let phi = values.insert(Value::new(join_id, ManagedType::Unknown));
    insns[join_id] = ManagedInsn::Phi {
        result: phi,
        inputs: vec![(ye, copied), (ne, zero)],
    };
    insns.insert(ManagedInsn::Throw { value: phi });
    if bad {
        insns.insert(ManagedInsn::Invoke {
            result: None,
            kind: managed_ir::ops::InvokeKind::Special,
            method: MethodRef::new(0),
            args: vec![phi],
        });
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        phi,
        copied,
    )
}
#[test]
fn selected_throw_phi_keeps_concrete_constructor_receiver_and_checked_type() {
    let (ir, phi, receiver) = fixture(false);
    let proof = checked_phi_plan(
        &ir,
        phi,
        &io(),
        &JavaType::Void,
        &Hierarchy::default(),
        &Resolver,
    )
    .unwrap();
    assert!(proof.locals.contains(&phi));
    assert!(!proof.locals.contains(&receiver));
    let plan = declaration_plan(&ir, &JavaType::Void, &Resolver).unwrap();
    // With one concrete Child producer, the narrower Child is also legal.
    assert!([io(), child()].contains(&plan[&phi]));
    assert!(!plan.contains_key(&receiver));
    assert_eq!(
        evidence(&ir, receiver, &Resolver).unwrap(),
        ValueType::Known(child())
    );
}
#[test]
fn selected_throw_phi_cannot_narrow_for_a_constructor_consumer() {
    let (ir, phi, _) = fixture(true);
    assert!(
        checked_phi_plan(
            &ir,
            phi,
            &io(),
            &JavaType::Void,
            &Hierarchy::default(),
            &Resolver
        )
        .is_err()
    );
}
#[test]
fn typed_throw_phi_jvm_retains_subtype_constructor_and_null_identity() {
    let (ir, phi, _) = fixture(false);
    checked_phi_plan(
        &ir,
        phi,
        &io(),
        &JavaType::Void,
        &Hierarchy::default(),
        &Resolver,
    )
    .unwrap();
    let source = r#"public class Fixture {
      static class Child extends java.io.IOException { final int marker; Child() { marker = 19; } }
      static Child child;
      static void select(boolean choose) throws java.io.IOException {
        Child concrete = new Child(); child = concrete;
        java.io.IOException selected;
        if (choose) selected = concrete; else selected = null;
        throw selected;
      }
      public static void main(String[] args) {
        try { select(true); throw new AssertionError(); } catch(java.io.IOException e) { if (e != child || child.marker != 19) throw new AssertionError(); }
        try { select(false); throw new AssertionError(); } catch(NullPointerException e) {} catch(java.io.IOException e) { throw new AssertionError(); }
        System.out.print("identity:constructor:null");
      }
    }"#;
    let dir = std::env::temp_dir().join(format!("decomphify-throw-phi-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), source).unwrap();
    let jdk =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let compiled = std::process::Command::new(format!("{jdk}/bin/javac"))
        .args(["--release", "17", "Fixture.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let run = std::process::Command::new(format!("{jdk}/bin/java"))
        .args(["-cp", ".", "Fixture"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "identity:constructor:null"
    );
}

#[test]
fn concrete_copy_rooted_in_phi_remains_atomic_and_checks_off_cfg_consumers() {
    let (ir, phi, _) = fixture(false);
    let mut instructions = ir.instructions().clone();
    let mut values = ir.values().clone();
    let mut copied = None;
    instructions.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
        copied = Some(v);
        ManagedInsn::Move {
            result: v,
            value: phi,
        }
    });
    let copied = copied.unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(
        Cfg::new(*ir.cfg().block(ir.cfg().entry()).unwrap().payload()),
        instructions.clone(),
        values.clone(),
    );
    assert!(preserved_reference_copy(&ir, copied, &io(), &Resolver).is_none());
    let plan = checked_phi_plan(
        &ir,
        phi,
        &io(),
        &JavaType::Void,
        &Hierarchy::default(),
        &Resolver,
    )
    .unwrap();
    assert!(plan.locals.contains(&copied));
    // An off-CFG subtype-only consumer must reject the complete selected family.
    instructions.insert(ManagedInsn::Invoke {
        result: None,
        kind: managed_ir::ops::InvokeKind::Special,
        method: MethodRef::new(0),
        args: vec![copied],
    });
    let ir = MethodIr::<SsaForm>::from_parts(
        Cfg::new(*ir.cfg().block(ir.cfg().entry()).unwrap().payload()),
        instructions,
        values,
    );
    assert!(
        checked_phi_plan(
            &ir,
            phi,
            &io(),
            &JavaType::Void,
            &Hierarchy::default(),
            &Resolver
        )
        .is_err()
    );
}

#[test]
fn ordinary_reference_phi_preserves_concrete_producer_copy() {
    let (ir, phi, receiver) = fixture(false);
    let mut instructions = ir.instructions().clone();
    for (_, instruction) in instructions.iter_mut() {
        if matches!(instruction, ManagedInsn::Throw { value } if *value == phi) {
            *instruction = ManagedInsn::Return { value: Some(phi) };
        }
    }
    let ir = MethodIr::<SsaForm>::from_parts(
        Cfg::new(*ir.cfg().block(ir.cfg().entry()).unwrap().payload()),
        instructions,
        ir.values().clone(),
    );
    let plan = checked_phi_plan(
        &ir,
        phi,
        &child(),
        &child(),
        &Hierarchy::default(),
        &Resolver,
    )
    .unwrap();
    // A concrete producer-copy retains its subtype for ordinary uses as well
    // as Throw; only selected Phi/copy locals adopt the checked declaration.
    assert!(!plan.locals.contains(&receiver));
    assert!(plan.locals.contains(&phi));
    assert!(checked_phi_plan(&ir, phi, &io(), &child(), &Hierarchy::default(), &Resolver).is_err());
}
