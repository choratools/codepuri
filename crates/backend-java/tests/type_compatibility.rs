//! Directed assignment and real Java contextual typing checks.
#![allow(clippy::unwrap_used)]
pub use backend_java::JavaType;
use backend_java::type_compatibility;
use type_compatibility::*;
fn c(n: &str) -> JavaType {
    JavaType::Class(n.into())
}
fn a(t: JavaType) -> JavaType {
    JavaType::Array(Box::new(t))
}
fn hierarchy() -> Hierarchy {
    let mut h = Hierarchy::default();
    for (name, superclass, interfaces, is_interface) in [
        ("I", None, vec![], true),
        ("Base", Some("java.lang.Object"), vec!["I"], false),
        ("Child", Some("Base"), vec![], false),
        ("Other", Some("java.lang.Object"), vec![], false),
    ] {
        h.insert(Declaration {
            name: name.into(),
            superclass: superclass.map(str::to_owned),
            interfaces: interfaces.into_iter().map(str::to_owned).collect(),
            is_interface,
        })
        .unwrap();
    }
    h
}
#[test]
fn directed_and_array_rules() {
    let h = hierarchy();
    for (from, to) in [
        (c("Child"), c("Base")),
        (c("Child"), c("I")),
        (a(c("Child")), a(c("I"))),
        (a(JavaType::Int), c("java.lang.Cloneable")),
        (a(a(JavaType::Int)), a(c("java.lang.Object"))),
        (a(JavaType::Int), c("java.io.Serializable")),
    ] {
        assert_eq!(h.assignable(&from, &to), Compatibility::Assignable);
    }
    for (from, to) in [
        (c("Base"), c("Child")),
        (c("Child"), c("Other")),
        (a(JavaType::Int), a(JavaType::Long)),
        (JavaType::Int, c("I")),
        (a(JavaType::Int), a(c("java.lang.Object"))),
    ] {
        assert_eq!(h.assignable(&from, &to), Compatibility::Incompatible);
    }
    assert!(matches!(
        h.assignable(&c("Missing"), &c("I")),
        Compatibility::Indeterminate(UnsupportedType::MissingHierarchy(_))
    ));
}
#[test]
fn contextual_values_preserve_conflicts() {
    let h = hierarchy();
    assert_eq!(
        expected_type(&h, &ValueType::RawZero, &JavaType::Int),
        Ok(Coercion::ZeroLiteral("0"))
    );
    assert_eq!(
        expected_type(&h, &ValueType::RawZero, &c("I")),
        Ok(Coercion::NullLiteral)
    );
    assert!(expected_type(&h, &ValueType::Null, &JavaType::Int).is_err());
    assert!(expected_type(&h, &ValueType::Unknown, &c("I")).is_err());
    assert!(expected_type(&h, &ValueType::Known(JavaType::Int), &c("I")).is_err());
    assert!(
        check_local_plan(
            &h,
            &c("Base"),
            &[ValueType::Known(c("Child")), ValueType::Null],
            &[c("I")]
        )
        .is_ok()
    );
    assert!(
        check_local_plan(
            &h,
            &c("Base"),
            &[ValueType::Known(c("Other"))],
            &[c("java.lang.Object")]
        )
        .is_err()
    );
}
#[test]
fn cycle_guard() {
    let mut h = Hierarchy::default();
    for (name, parent) in [("A", "B"), ("B", "A")] {
        h.insert(Declaration {
            name: name.into(),
            superclass: Some(parent.into()),
            interfaces: vec![],
            is_interface: false,
        })
        .unwrap();
    }
    assert!(matches!(
        h.assignable(&c("A"), &c("I")),
        Compatibility::Indeterminate(UnsupportedType::Cycle(_))
    ));
}
#[test]
fn generated_java_compiles_and_runs() {
    let h = hierarchy();
    let null = match expected_type(&h, &ValueType::RawZero, &c("I")).unwrap() {
        Coercion::NullLiteral => "null",
        _ => panic!(),
    };
    let zero = match expected_type(&h, &ValueType::RawZero, &JavaType::Int).unwrap() {
        Coercion::ZeroLiteral(s) => s,
        _ => panic!(),
    };
    let dir = std::env::temp_dir().join(format!("decomphify-types-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = format!(
        "interface I {{}} class Base implements I {{}} class Child extends Base {{}} public class CompatibilityMain {{ static I arg(I x) {{ return x; }} public static void main(String[] a) {{ Base local = new Child(); I i = arg(local); I nil = {null}; int n = {zero}; I[] array = new Child[1]; if(i == null || nil != null || n != 0 || array.length != 1) throw new AssertionError(); }} }}"
    );
    std::fs::write(dir.join("CompatibilityMain.java"), source).unwrap();
    assert!(
        std::process::Command::new("javac")
            .arg("CompatibilityMain.java")
            .current_dir(&dir)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new("java")
            .args(["-cp", ".", "CompatibilityMain"])
            .current_dir(&dir)
            .status()
            .unwrap()
            .success()
    );
    std::fs::remove_dir_all(dir).unwrap();
}
