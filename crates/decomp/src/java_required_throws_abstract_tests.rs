#![allow(clippy::unwrap_used)]
use super::*;
use backend_java::{JavaField, JavaMethod};
use managed_ir::refs::{FieldRef, StringRef};

struct Context {
    inherited: Option<Vec<String>>,
}
fn identity(index: u32) -> OriginalMethodIdentity {
    OriginalMethodIdentity {
        dex: 0,
        index,
        key: MethodKey {
            owner: if index == 0 {
                "LContract;".into()
            } else {
                format!("LImplementation{index};")
            },
            name: "run".into(),
            parameters: vec![],
            return_type: "V".into(),
        },
    }
}
impl JavaResolver for Context {
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        Some(if r.index() == 0 { 0x401 } else { 1 })
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
    fn assignable(&self, a: &JavaType, e: &JavaType) -> bool {
        a == e
            || matches!((a, e), (JavaType::Class(a), JavaType::Class(e))
            if ["java.io.IOException", "java.sql.SQLException"].contains(&a.as_str())
            && ["java.lang.Exception", "java.lang.Throwable"].contains(&e.as_str()))
    }
}
impl ThrowsHierarchy for Context {
    fn exception_kind(&self, name: &str) -> Option<ExceptionKind> {
        [
            "java.io.IOException",
            "java.sql.SQLException",
            "java.lang.Exception",
            "java.lang.Throwable",
        ]
        .contains(&name)
        .then_some(ExceptionKind::Checked)
    }
}
impl RequiredThrowsContext for Context {
    fn invocation_contract(&self, r: MethodRef) -> Option<InvocationContract> {
        Some(InvocationContract {
            source_target: Some(identity(r.index()).key),
            declared: vec![],
        })
    }
    fn override_contract(&self, i: &OriginalMethodIdentity) -> OverrideContract {
        if i.index == 0 {
            self.inherited
                .as_ref()
                .map(|throws| {
                    OverrideContract::Declarations(vec![OverrideDeclaration {
                        key: MethodKey {
                            owner: "LExternal;".into(),
                            ..identity(0).key
                        },
                        source_target: None,
                        throws: throws.clone(),
                    }])
                })
                .unwrap_or(OverrideContract::NoInheritedDeclaration)
        } else {
            OverrideContract::Declarations(vec![OverrideDeclaration {
                key: identity(0).key.clone(),
                source_target: Some(identity(0).key),
                throws: vec![],
            }])
        }
    }
}
fn body(index: u32, name: &str, context: &Context) -> MethodRequirements {
    let identity = identity(index);
    MethodRequirements {
        overrides: context.override_contract(&identity),
        identity,
        original: vec![],
        flags: 1,
        events: vec![Event::Throw(
            name.into(),
            Coverage {
                catches: vec![],
                all: false,
            },
        )],
    }
}
fn declaration(context: &Context, implementations: &[u32]) -> MethodRequirements {
    inspect_abstract_declaration(
        identity(0),
        0x401,
        vec![],
        implementations.iter().map(|i| identity(*i).key).collect(),
        context,
    )
    .unwrap()
}
#[test]
fn abstract_union_preserves_original_and_propagates_to_callers() {
    let context = Context {
        inherited: Some(vec!["java.lang.Exception".into()]),
    };
    let mut caller = body(3, "java.io.IOException", &context);
    caller.overrides = OverrideContract::NoInheritedDeclaration;
    caller.events = vec![Event::Invoke(
        InvocationContract {
            source_target: Some(identity(0).key),
            declared: vec![],
        },
        Coverage {
            catches: vec![],
            all: false,
        },
    )];
    let plan = derive_program(
        &[
            declaration(&context, &[1, 2]),
            body(1, "java.io.IOException", &context),
            body(2, "java.sql.SQLException", &context),
            caller,
        ],
        &context,
    )
    .unwrap();
    assert!(plan.unavailable.is_empty(), "{:?}", plan.unavailable);
    assert!(plan.methods[&identity(0).key].original.is_empty());
    assert_eq!(
        plan.methods[&identity(0).key].source,
        ["java.io.IOException", "java.sql.SQLException"]
    );
    assert_eq!(
        plan.methods[&identity(3).key].source,
        plan.methods[&identity(0).key].source
    );
}
#[test]
fn external_empty_bound_and_missing_implementation_remain_unavailable() {
    let context = Context {
        inherited: Some(vec![]),
    };
    let plan = derive_program(
        &[
            declaration(&context, &[1]),
            body(1, "java.io.IOException", &context),
        ],
        &context,
    )
    .unwrap();
    assert_eq!(
        plan.unavailable[&identity(0).key],
        ThrowsError::OverrideConflict("java.io.IOException".into())
    );
    assert!(plan.methods.is_empty());
    let context = Context { inherited: None };
    let plan = derive_program(&[declaration(&context, &[1])], &context).unwrap();
    assert_eq!(
        plan.unavailable[&identity(0).key],
        ThrowsError::MissingSourceTarget(identity(1).key)
    );
    assert!(
        inspect_abstract_declaration(identity(0), 0x101, vec![], vec![identity(1).key], &context)
            .is_err()
    );
}
#[test]
fn reconstructed_interface_compiles_preserves_identity_and_rejects_external_bound() {
    let context = Context { inherited: None };
    let plan = derive_program(
        &[
            declaration(&context, &[1]),
            body(1, "java.io.IOException", &context),
        ],
        &context,
    )
    .unwrap();
    let throws = plan.methods[&identity(0).key].source.join(", ");
    let source = format!(
        "public class AbstractContractFixture {{ interface Contract {{ void run() throws {throws}; }} static final java.io.IOException original = new java.io.IOException(); static class Implementation implements Contract {{ public void run() throws {throws} {{ throw original; }} }} public static void main(String[] a) {{ Contract c = new Implementation(); try {{ c.run(); throw new AssertionError(); }} catch (java.io.IOException e) {{ if (e != original) throw new AssertionError(); System.out.print(\"IO:identity\"); }} }} }}"
    );
    let dir =
        std::env::temp_dir().join(format!("decomphify-abstract-throws-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("AbstractContractFixture.java"), source).unwrap();
    let jdk =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let javac = |file| {
        std::process::Command::new(format!("{jdk}/bin/javac"))
            .args(["--release", "17", file])
            .current_dir(&dir)
            .output()
            .unwrap()
    };
    let compiled = javac("AbstractContractFixture.java");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let run = std::process::Command::new(format!("{jdk}/bin/java"))
        .args(["-cp", ".", "AbstractContractFixture"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(run.status.success());
    assert_eq!(run.stdout, b"IO:identity");
    std::fs::write(dir.join("InvalidBound.java"), "class InvalidBound implements Runnable { public void run() throws java.io.IOException {} }").unwrap();
    let invalid = javac("InvalidBound.java");
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("overridden method does not throw"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn original_checked_annotations_do_not_bypass_external_bounds_or_diamonds() {
    let context = Context { inherited: None };
    let mut implementation = body(1, "java.io.IOException", &context);
    implementation.original = vec!["java.io.IOException".into()];
    implementation.events.clear();
    let external = |owner: &str, throws: Vec<String>| OverrideDeclaration {
        key: MethodKey {
            owner: owner.into(),
            ..identity(1).key
        },
        source_target: None,
        throws,
    };
    implementation.overrides = OverrideContract::Declarations(vec![external("LExternal;", vec![])]);
    let plan = derive_program(&[implementation.clone()], &context).unwrap();
    assert_eq!(
        plan.unavailable[&identity(1).key],
        ThrowsError::OverrideConflict("java.io.IOException".into())
    );
    implementation.overrides = OverrideContract::Declarations(vec![
        external("LPermissive;", vec!["java.lang.Exception".into()]),
        external("LExternal;", vec![]),
    ]);
    let plan = derive_program(&[implementation], &context).unwrap();
    assert_eq!(
        plan.unavailable[&identity(1).key],
        ThrowsError::OverrideConflict("java.io.IOException".into())
    );
    let mut abstract_method = declaration(&context, &[1]);
    abstract_method.overrides = OverrideContract::Unknown;
    let plan = derive_program(
        &[abstract_method, body(1, "java.io.IOException", &context)],
        &context,
    )
    .unwrap();
    assert_eq!(
        plan.unavailable[&identity(0).key],
        ThrowsError::Unknown("inherited throws contract".into())
    );
    assert!(plan.methods.is_empty());
}
