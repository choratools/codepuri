//! Independently wired contextual typing helper and JDK17 behavior checks.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use backend_java::contextual_locals;
pub use backend_java::{JavaResolver, JavaType, type_compatibility};
use contextual_locals::*;
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::Constant,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{Value, ValueId},
};
use type_compatibility::{Coercion, Hierarchy};

struct Resolver;
impl JavaResolver for Resolver {
    fn method(&self, _: MethodRef) -> Option<backend_java::JavaMethod> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<backend_java::JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("java.lang.String".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
fn fixture(left: Constant, right: ManagedType) -> (MethodIr<SsaForm>, ValueId, ValueId, ValueId) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut alloc = |ty, f: &dyn Fn(ValueId) -> ManagedInsn| {
        let mut value = None;
        insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ty));
            value = Some(v);
            f(v)
        });
        value.unwrap()
    };
    let a = alloc(ManagedType::Unknown, &|result| ManagedInsn::Const {
        result,
        value: left,
    });
    let b = alloc(right, &|v| ManagedInsn::Opaque {
        family_tag: 0x8003,
        reads: vec![],
        writes: vec![v],
        effects: managed_ir::effects::EffectFlags::NONE,
    });
    let phi = alloc(ManagedType::Unknown, &|result| ManagedInsn::Phi {
        result,
        inputs: vec![(Default::default(), a), (Default::default(), b)],
    });
    let id = values[phi].def();
    (
        MethodIr::<SsaForm>::from_parts(decomp_core::cfg::Cfg::new(id), insns, values),
        a,
        b,
        phi,
    )
}
#[test]
fn shared_zero_has_use_site_expressions_and_phi_checks_real_inputs() {
    let (ir, zero, _, phi) = fixture(Constant::Zero, ManagedType::Int);
    let h = Hierarchy::default();
    let reference = JavaType::Class("java.lang.String".into());
    assert_eq!(
        expression(&ir, zero, &reference, &h, &Resolver),
        Ok(Coercion::NullLiteral)
    );
    assert_eq!(
        expression(&ir, zero, &JavaType::Float, &h, &Resolver),
        Ok(Coercion::ZeroLiteral("0.0f"))
    );
    assert_eq!(
        expression(&ir, zero, &JavaType::Int, &h, &Resolver),
        Ok(Coercion::ZeroLiteral("0"))
    );
    assert!(
        phi_plan(
            &ir,
            phi,
            &reference,
            std::slice::from_ref(&reference),
            &h,
            &Resolver
        )
        .is_err()
    );
    let plan = phi_plan(&ir, phi, &JavaType::Int, &[JavaType::Int], &h, &Resolver).unwrap();
    assert_eq!(plan.inputs[0], (zero, Coercion::ZeroLiteral("0")));
    assert_eq!(plan.inputs[1].1, Coercion::Identity);
    assert_eq!(plan.locals, vec![phi]);
    assert!(phi_plan(&ir, phi, &JavaType::Int, &[reference], &h, &Resolver).is_err());
    assert!(expression(&ir, phi, &JavaType::Int, &h, &Resolver).is_err());
}
#[test]
fn explicit_null_cannot_be_numeric_and_unknown_is_not_evidence() {
    let (ir, null, unknown, phi) = fixture(Constant::Null, ManagedType::Unknown);
    let h = Hierarchy::default();
    let reference = JavaType::Class("java.lang.String".into());
    assert!(expression(&ir, null, &JavaType::Int, &h, &Resolver).is_err());
    assert!(expression(&ir, unknown, &reference, &h, &Resolver).is_err());
    assert!(phi_plan(&ir, phi, &reference, &[], &h, &Resolver).is_err());
    let (ir, zero, fake_zero, _) = fixture(Constant::Zero, ManagedType::Zero);
    assert!(expression(&ir, zero, &JavaType::Long, &h, &Resolver).is_err());
    assert!(expression(&ir, fake_zero, &JavaType::Int, &h, &Resolver).is_err());
}

fn comparison_fixture(
    op: managed_ir::ops::CompareOp,
    peer: Constant,
    input_type: ManagedType,
) -> (MethodIr<SsaForm>, ValueId) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut alloc = |ty, f: &dyn Fn(ValueId) -> ManagedInsn| {
        let mut value = None;
        insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ty));
            value = Some(v);
            f(v)
        });
        value.unwrap()
    };
    let zero = alloc(ManagedType::Zero, &|result| ManagedInsn::Const {
        result,
        value: Constant::Zero,
    });
    let nonzero = alloc(input_type.clone(), &|result| ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![result],
        effects: managed_ir::effects::EffectFlags::NONE,
    });
    let phi = alloc(input_type, &|result| ManagedInsn::Phi {
        result,
        inputs: vec![(Default::default(), zero), (Default::default(), nonzero)],
    });
    let rhs = alloc(ManagedType::Zero, &|result| ManagedInsn::Const {
        result,
        value: peer,
    });
    let _ = alloc(ManagedType::Boolean, &|result| ManagedInsn::Compare {
        op,
        result,
        lhs: phi,
        rhs,
    });
    let root = values[phi].def();
    (
        MethodIr::<SsaForm>::from_parts(decomp_core::cfg::Cfg::new(root), insns, values),
        phi,
    )
}

#[test]
fn integer_phi_accepts_definition_proven_zero_in_relational_comparisons() {
    use managed_ir::ops::CompareOp;
    for op in [
        CompareOp::LessThan,
        CompareOp::LessEqual,
        CompareOp::GreaterThan,
        CompareOp::GreaterEqual,
    ] {
        let (ir, phi) = comparison_fixture(op, Constant::Zero, ManagedType::Int);
        let plan = checked_phi_plan(
            &ir,
            phi,
            &JavaType::Int,
            &JavaType::Void,
            &Hierarchy::default(),
            &Resolver,
        )
        .unwrap();
        assert_eq!(plan.inputs[0].1, Coercion::ZeroLiteral("0"));
        assert_eq!(plan.inputs[1].1, Coercion::Identity);
        assert_eq!(
            declaration_plan(&ir, &JavaType::Void, &Resolver)
                .unwrap()
                .get(&phi),
            Some(&JavaType::Int)
        );
    }
}

#[test]
fn relational_comparisons_do_not_reinterpret_null_or_boolean_values() {
    use managed_ir::ops::CompareOp;
    let (ir, phi) = comparison_fixture(CompareOp::LessThan, Constant::Null, ManagedType::Int);
    assert!(
        checked_phi_plan(
            &ir,
            phi,
            &JavaType::Int,
            &JavaType::Void,
            &Hierarchy::default(),
            &Resolver
        )
        .is_err()
    );
    let (ir, phi) = comparison_fixture(CompareOp::LessThan, Constant::Zero, ManagedType::Boolean);
    assert!(
        checked_phi_plan(
            &ir,
            phi,
            &JavaType::Boolean,
            &JavaType::Void,
            &Hierarchy::default(),
            &Resolver
        )
        .is_err()
    );
}

#[test]
#[ignore = "requires TYPE_REAL_JAVA pointing to retained ErrorUtils.java"]
fn retained_real_errorutils_jdk17_differential() {
    let original = std::env::var("TYPE_REAL_JAVA").unwrap();
    let dir =
        std::env::temp_dir().join(format!("decomphify-real-contextual-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("ErrorUtils.java");
    std::fs::copy(original, &source).unwrap();
    let harness = dir.join("RealContextualCheck.java");
    std::fs::write(&harness, "package androidx.biometric; public class RealContextualCheck { public static void main(String[] args) { for(int i=-10;i<=30;i++) { boolean expected=i>=1 && i<=15 && i!=6; if(ErrorUtils.isKnownError(i)!=expected) throw new AssertionError(i); } if(ErrorUtils.isKnownError(Integer.MIN_VALUE) || ErrorUtils.isKnownError(Integer.MAX_VALUE)) throw new AssertionError(); } }").unwrap();
    let compile = std::process::Command::new("javac")
        .args(["--release", "17", "-d"])
        .arg(&dir)
        .arg(&source)
        .arg(&harness)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new("java")
        .arg("-cp")
        .arg(&dir)
        .arg("androidx.biometric.RealContextualCheck")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn jdk17_constant_expressions_preserve_selected_value_and_exceptions() {
    let (ir, zero, _, _) = fixture(Constant::Zero, ManagedType::Int);
    let h = Hierarchy::default();
    let literal = |ty| match expression(&ir, zero, &ty, &h, &Resolver).unwrap() {
        Coercion::NullLiteral => "null",
        Coercion::ZeroLiteral(s) => s,
        _ => panic!("literal expected"),
    };
    let source = format!(
        "public class ContextualConstants {{ static int selected(boolean b) {{ int p; if (b) p={}; else p=7; return p; }} public static void main(String[] a) {{ Object n={}; float f={}; boolean b={}; if(n!=null || Float.floatToRawIntBits(f)!=0 || b || selected(true)!=0 || selected(false)!=7) throw new AssertionError(); try {{ n.toString(); throw new AssertionError(); }} catch(NullPointerException expected) {{}} }} }}",
        literal(JavaType::Int),
        literal(JavaType::Class("java.lang.Object".into())),
        literal(JavaType::Float),
        literal(JavaType::Boolean)
    );
    let dir = std::env::temp_dir().join(format!("decomphify-contextual-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("ContextualConstants.java");
    std::fs::write(&file, source).unwrap();
    let compiled = std::process::Command::new("javac")
        .args(["--release", "17"])
        .arg(&file)
        .output()
        .expect("JDK17 javac");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let ran = std::process::Command::new("java")
        .arg("-cp")
        .arg(&dir)
        .arg("ContextualConstants")
        .output()
        .unwrap();
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}
