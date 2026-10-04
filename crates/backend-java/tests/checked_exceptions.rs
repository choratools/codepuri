//! Coverage proofs and genuine JVM thrown-object identity.
#![allow(clippy::unwrap_used)]
#[path = "../src/checked_exceptions.rs"]
mod checked_exceptions;
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType};
use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
struct Hierarchy;
impl JavaResolver for Hierarchy {
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        let (JavaType::Class(a), JavaType::Class(e)) = (actual, expected) else {
            return false;
        };
        a == e
            || matches!(
                (a.as_str(), e.as_str()),
                (
                    "java.io.FileNotFoundException",
                    "java.io.IOException" | "java.lang.Exception" | "java.lang.Throwable"
                ) | (
                    "java.lang.IllegalArgumentException",
                    "java.lang.RuntimeException" | "java.lang.Exception" | "java.lang.Throwable"
                ) | (
                    "java.lang.AssertionError",
                    "java.lang.Error" | "java.lang.Throwable"
                )
            )
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).into()).collect()
}
#[test]
fn exact_subtype_unchecked_and_missing_proofs() {
    assert!(
        checked_exceptions::check(
            &Hierarchy,
            &names(&["java.io.IOException"]),
            &names(&["java.io.IOException"])
        )
        .is_ok()
    );
    assert!(
        checked_exceptions::check(
            &Hierarchy,
            &names(&["java.io.FileNotFoundException"]),
            &names(&["java.io.IOException"])
        )
        .is_ok()
    );
    assert!(
        checked_exceptions::check(
            &Hierarchy,
            &names(&[
                "java.lang.IllegalArgumentException",
                "java.lang.AssertionError",
                "java.lang.RuntimeException",
                "java.lang.Error"
            ]),
            &[]
        )
        .is_ok()
    );
    assert_eq!(
        checked_exceptions::check(
            &Hierarchy,
            &names(&[
                "java.io.FileNotFoundException",
                "missing.Type",
                "java.io.IOException"
            ]),
            &names(&["java.lang.RuntimeException"])
        )
        .unwrap_err()
        .types,
        names(&[
            "java.io.FileNotFoundException",
            "missing.Type",
            "java.io.IOException"
        ])
    );
}
#[test]
fn subtype_coverage_preserves_original_throw_identity() {
    assert!(
        checked_exceptions::check(
            &Hierarchy,
            &names(&["java.io.FileNotFoundException"]),
            &names(&["java.io.IOException"])
        )
        .is_ok()
    );
    let root = std::env::temp_dir().join(format!(
        "decomphify-checked-exceptions-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let source = r#"public class ExceptionCoverage {
 static final java.io.FileNotFoundException checked = new java.io.FileNotFoundException();
 static final IllegalArgumentException unchecked = new IllegalArgumentException();
 static final AssertionError error = new AssertionError();
 static void callee() throws java.io.FileNotFoundException { throw checked; }
 static void caller() throws java.io.IOException { callee(); }
 static void uncheckedCallee() throws IllegalArgumentException { throw unchecked; }
 static void uncheckedCaller() { uncheckedCallee(); }
 static void errorCallee() throws AssertionError { throw error; }
 static void errorCaller() { errorCallee(); }
 public static void main(String[] args) {
  try { caller(); throw new AssertionError(); } catch(java.io.IOException x) { if(x != checked) throw new AssertionError(); }
  try { uncheckedCaller(); throw new AssertionError(); } catch(IllegalArgumentException x) { if(x != unchecked) throw new AssertionError(); }
  try { errorCaller(); throw new AssertionError(); } catch(AssertionError x) { if(x != error) throw new AssertionError(); }
 }
}"#;
    std::fs::write(root.join("ExceptionCoverage.java"), source).unwrap();
    let jdk = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
    assert!(
        std::process::Command::new(format!("{jdk}/javac"))
            .arg(root.join("ExceptionCoverage.java"))
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new(format!("{jdk}/java"))
            .args(["-cp", root.to_str().unwrap(), "ExceptionCoverage"])
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(
        root.join("BadCoverage.java"),
        "class BadCoverage { void bad() { throw new java.io.IOException(); } }",
    )
    .unwrap();
    let bad = std::process::Command::new(format!("{jdk}/javac"))
        .arg(root.join("BadCoverage.java"))
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("unreported exception"));
}
