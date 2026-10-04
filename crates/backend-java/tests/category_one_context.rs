//! Definition-proven category-one bits; arbitrary numerics are not Booleans.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, contextual_locals::*,
    type_compatibility::Hierarchy,
};
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::Constant,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{Value, ValueId},
};
struct Resolver;
impl JavaResolver for Resolver {
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
    }
    fn field(&self, reference: FieldRef) -> Option<JavaField> {
        (reference.index() == 0).then(|| JavaField {
            owner: "CategoryOne".into(),
            name: "flag".into(),
            ty: JavaType::Boolean,
            is_static: true,
        })
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
fn fixture(a: Constant, b: Constant) -> (MethodIr<SsaForm>, ValueId, ValueId, ValueId) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut add = |ty, f: &dyn Fn(ValueId) -> ManagedInsn| {
        let mut value = None;
        insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ty));
            value = Some(v);
            f(v)
        });
        value.unwrap()
    };
    let left = add(ManagedType::Int, &|result| ManagedInsn::Const {
        result,
        value: a,
    });
    let right = add(ManagedType::Boolean, &|result| ManagedInsn::Const {
        result,
        value: b,
    });
    let copy = add(ManagedType::Int, &|result| ManagedInsn::Move {
        result,
        value: left,
    });
    let phi = add(ManagedType::Int, &|result| ManagedInsn::Phi {
        result,
        inputs: vec![(Default::default(), copy), (Default::default(), right)],
    });
    let entry = values[phi].def();
    (
        MethodIr::<SsaForm>::from_parts(decomp_core::cfg::Cfg::new(entry), insns, values),
        copy,
        right,
        phi,
    )
}
#[test]
fn all_phi_inputs_and_consumers_keep_category_one_proofs() {
    let (ir, copy, boolean, phi) = fixture(Constant::Int(1), Constant::Boolean(false));
    assert_eq!(
        category_one_expression(&ir, copy, &JavaType::Int, &JavaType::Boolean, &Resolver),
        Some(CategoryOneCoercion::Literal("true"))
    );
    assert_eq!(
        category_one_expression(&ir, boolean, &JavaType::Boolean, &JavaType::Int, &Resolver),
        Some(CategoryOneCoercion::Literal("0"))
    );
    assert!(boolean_bits(&ir, phi, &Resolver));
    assert_eq!(
        category_one_expression(&ir, phi, &JavaType::Int, &JavaType::Boolean, &Resolver),
        Some(CategoryOneCoercion::IntBitsToBoolean)
    );
    let plan = phi_plan(
        &ir,
        phi,
        &JavaType::Boolean,
        &[JavaType::Boolean, JavaType::Int],
        &Hierarchy::default(),
        &Resolver,
    )
    .unwrap();
    assert!(plan.locals.contains(&phi) && plan.locals.contains(&copy));
    assert!(
        plan.inputs
            .iter()
            .any(|(_, c)| *c == backend_java::type_compatibility::Coercion::ZeroLiteral("true"))
    );
    let (bad_ir, _, _, bad_phi) = fixture(Constant::Zero, Constant::Int(2));
    assert!(!boolean_bits(&bad_ir, bad_phi, &Resolver));
    assert!(
        phi_plan(
            &bad_ir,
            bad_phi,
            &JavaType::Boolean,
            &[JavaType::Boolean],
            &Hierarchy::default(),
            &Resolver
        )
        .is_err()
    );
    for bad in [
        Constant::Int(2),
        Constant::Int(-1),
        Constant::Long(0),
        Constant::Null,
    ] {
        let (ir, value, _, phi) = fixture(bad, Constant::Boolean(false));
        assert!(!boolean_bits(&ir, phi, &Resolver));
        assert!(
            category_one_expression(&ir, value, &JavaType::Int, &JavaType::Boolean, &Resolver)
                .is_none()
        );
        assert!(
            phi_plan(
                &ir,
                phi,
                &JavaType::Boolean,
                &[JavaType::Boolean],
                &Hierarchy::default(),
                &Resolver
            )
            .is_err()
        );
    }
}
#[test]
fn jdk17_bit_domain_and_single_evaluation_runtime() {
    let (ir, copy, right, phi) = fixture(Constant::Int(1), Constant::Boolean(false));
    let Some(CategoryOneCoercion::Literal(one)) =
        category_one_expression(&ir, copy, &JavaType::Int, &JavaType::Boolean, &Resolver)
    else {
        panic!()
    };
    let Some(CategoryOneCoercion::Literal(zero)) =
        category_one_expression(&ir, right, &JavaType::Boolean, &JavaType::Int, &Resolver)
    else {
        panic!()
    };
    assert_eq!(
        category_one_expression(&ir, phi, &JavaType::Boolean, &JavaType::Int, &Resolver),
        Some(CategoryOneCoercion::BooleanToInt)
    );
    let dir = std::env::temp_dir().join(format!("decomphify-category-one-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("CategoryOne.java");
    std::fs::write(&file,format!("public class CategoryOne {{ static int n; static boolean next(boolean b){{n++;return b;}} static int bits(boolean b){{return next(b)?1:0;}} static boolean selected(boolean b){{int v=b?1:0;return v!=0;}} public static void main(String[] args){{if(!{one} || {zero}!=0)throw new AssertionError();for(boolean b:new boolean[]{{false,true}}){{n=0;if(bits(b)!=(b?1:0)||n!=1||selected(b)!=b)throw new AssertionError();}} if((1&0)!=0 || (1|0)!=1 || (1^1)!=0)throw new AssertionError();}} }}")).unwrap();
    let result = std::process::Command::new("javac")
        .args(["--release", "17"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = std::process::Command::new("java")
        .arg("-cp")
        .arg(&dir)
        .arg("CategoryOne")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    // Genuine arbitrary int-to-Boolean assignment is still a javac error.
    std::fs::write(
        &file,
        "class CategoryOne { static boolean invalid(int unknown) { return unknown; } }",
    )
    .unwrap();
    let result = std::process::Command::new("javac")
        .arg(&file)
        .output()
        .unwrap();
    assert!(!result.status.success());
}

#[test]
fn descriptor_boolean_and_bitwise_flags_are_not_arbitrary_ints() {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut field = None;
    let entry = insns.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Boolean));
        field = Some(result);
        ManagedInsn::FieldGet {
            result,
            object: None,
            field: FieldRef::new(0),
        }
    });
    let field = field.unwrap();
    let mut one = None;
    insns.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Int));
        one = Some(result);
        ManagedInsn::Const {
            result,
            value: Constant::Int(1),
        }
    });
    let mut bits = None;
    insns.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Int));
        bits = Some(result);
        ManagedInsn::Arithmetic {
            result,
            op: managed_ir::ops::ArithOp::Xor,
            lhs: field,
            rhs: one.unwrap(),
        }
    });
    let ir = MethodIr::<SsaForm>::from_parts(decomp_core::cfg::Cfg::new(entry), insns, values);
    assert_eq!(
        category_one_expression(&ir, field, &JavaType::Boolean, &JavaType::Int, &Resolver),
        Some(CategoryOneCoercion::BooleanToInt)
    );
    assert!(boolean_bits(&ir, bits.unwrap(), &Resolver));
    assert_eq!(
        category_one_expression(
            &ir,
            bits.unwrap(),
            &JavaType::Int,
            &JavaType::Boolean,
            &Resolver
        ),
        Some(CategoryOneCoercion::IntBitsToBoolean)
    );
}
