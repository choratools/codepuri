//! Standalone parent-integration contracts and real JDK17 access oracles.
#![allow(clippy::unwrap_used)]
#[path = "../src/java_access.rs"]
mod access;
#[path = "../../backend-java/src/special_invokes.rs"]
mod dispatch;
#[path = "../src/java_dependency_publication.rs"]
mod publication;
mod java_aliases {
    pub use decomp::java_aliases::*;
}
use backend_java::type_compatibility::{Declaration, Hierarchy};
use managed_ir::ops::InvokeKind;

#[test]
fn alias_protected_access_requires_both_original_and_source_graphs() {
    let mut original = Hierarchy::default();
    original
        .insert(Declaration {
            name: "q.a".into(),
            superclass: Some("p.Base".into()),
            interfaces: vec![],
            is_interface: false,
        })
        .unwrap();
    let source = original
        .map_names(|name| {
            if name == "q.a" {
                "q.a$type".into()
            } else {
                name.into()
            }
        })
        .unwrap();
    let context = access::AccessContext {
        caller: "q.a",
        owner: "p.Base",
        receiver: Some("q.a"),
        hierarchy: &original,
        nests: None,
        emitted_caller: "q.a$type",
        emitted_owner: "p.Base",
    };
    assert_eq!(
        context.method_access_with_source_hierarchy(4, &source, Some("q.a$type")),
        Ok(())
    );
    assert_eq!(
        context.method_access_with_source_hierarchy(4, &source, Some("p.Base")),
        Err(access::AccessBlocker::ProtectedReceiver)
    );
    assert!(
        context
            .method_access_with_source_hierarchy(4, &Hierarchy::default(), Some("q.a$type"))
            .is_err()
    );
    let missing_original = access::AccessContext {
        hierarchy: &Hierarchy::default(),
        ..context
    };
    assert!(
        missing_original
            .method_access_with_source_hierarchy(4, &source, Some("q.a$type"))
            .is_err()
    );
}

#[test]
fn original_access_and_receiver_restriction() {
    let mut h = Hierarchy::default();
    h.insert(Declaration {
        name: "q.Child".into(),
        superclass: Some("p.Base".into()),
        interfaces: vec![],
        is_interface: false,
    })
    .unwrap();
    let mut c = access::AccessContext {
        caller: "q.Child",
        owner: "p.Base",
        receiver: Some("p.Base"),
        hierarchy: &h,
        nests: None,
        emitted_caller: "q.Child",
        emitted_owner: "p.Base",
    };
    assert_eq!(
        c.method_access(4),
        Err(access::AccessBlocker::ProtectedReceiver)
    );
    c.receiver = Some("q.Child");
    assert_eq!(c.method_access(4), Ok(()));
    assert_eq!(c.method_access(2), Err(access::AccessBlocker::Private));
    c.caller = "p.Other";
    c.emitted_caller = "renamed.Other";
    assert_eq!(
        c.method_access(0),
        Err(access::AccessBlocker::AliasPackageChanged)
    );
    c.caller = "p.Base";
    c.emitted_caller = "p.Base";
    assert_eq!(c.method_access(2 | 0x100), Ok(()));
    assert_eq!(c.class_access(1), Ok(()));
    assert_eq!(access::package("p.Outer$Inner"), Ok("p"));
}

#[test]
fn special_dispatch_requires_exact_proof() {
    let mut c = dispatch::DispatchContext {
        caller: "p.C",
        declaration_owner: "p.C",
        declaration_name: "f",
        access_flags: 2 | 0x100,
        receiver_is_this: false,
        super_resolver: None,
    };
    assert_eq!(
        c.plan(InvokeKind::Special),
        Ok(dispatch::InvocationPlan::DirectPrivate)
    );
    assert_eq!(
        c.plan(InvokeKind::Virtual),
        Err(dispatch::DispatchBlocker::PrivateVirtual)
    );
    assert_eq!(
        c.plan(InvokeKind::Static),
        Err(dispatch::DispatchBlocker::StaticMismatch)
    );
    c.declaration_owner = "p.B";
    c.access_flags = 4;
    c.receiver_is_this = true;
    assert_eq!(
        c.plan(InvokeKind::Special),
        Err(dispatch::DispatchBlocker::UnprovenSpecial)
    );
    let proof = dispatch::SuperProof(true);
    c.super_resolver = Some(&proof);
    assert_eq!(
        c.plan(InvokeKind::Super),
        Ok(dispatch::InvocationPlan::Super)
    );
}

#[test]
fn declaration_reference_is_not_body_completion() {
    use publication::*;
    let key = java_aliases::MethodKey {
        owner: "Lp/C;".into(),
        name: "nativeCall".into(),
        parameters: vec![],
        return_type: "V".into(),
    };
    let mut p = DependencyPublication::default();
    for status in [
        DeclarationStatus::BodylessDeclaration,
        DeclarationStatus::ExternalClasspath,
    ] {
        p.declarations.insert(key.clone(), status);
        p.require_reference(&key).unwrap();
        assert!(p.body_closure.is_empty());
    }
    p.declarations
        .insert(key.clone(), DeclarationStatus::SourceSubsetAbsent);
    assert_eq!(
        p.require_reference(&key),
        Err(DependencyBlocker::SourceSubsetAbsent(key.clone()))
    );
    let mut report = BodyPublicationReport::default();
    let mut caller = key.clone();
    caller.name = "caller".into();
    p.record_emitted_body(&mut report, caller.clone(), [key.clone()]);
    assert!(!report.declaration_closed());
    p.declarations
        .insert(key.clone(), DeclarationStatus::CompleteBody);
    p.refresh(&mut report);
    assert!(report.declaration_closed());
    assert!(report.complete_bodies.contains(&caller));
    p.require_reference(&key).unwrap();
    assert!(p.body_closure.contains(&key));
}

#[test]
fn javac17_access_and_super_runtime() {
    use std::{fs, process::Command};
    let root = std::env::temp_dir().join(format!("decomp-access-{}", std::process::id()));
    fs::create_dir_all(root.join("p")).unwrap();
    fs::create_dir_all(root.join("q")).unwrap();
    let javac = "/home/jtjisgod/tools/jdk-17.0.13+11/bin/javac";
    let java = "/home/jtjisgod/tools/jdk-17.0.13+11/bin/java";
    fs::write(root.join("p/Base.java"), "package p; public class Base { protected int f(){return 1;} int pkg(){return 3;} private int priv(){return 4;} public native int nativeCall(); }").unwrap();
    fs::write(root.join("q/Child.java"), "package q; public class Child extends p.Base { private int own(){return 7;} protected int f(){return 2;} int call(){return own()+super.f();} public static void main(String[] args){if(new Child().call()!=8)throw new AssertionError();} }").unwrap();
    let compile = |source: &str| {
        Command::new(javac)
            .args(["--release", "17", "-cp"])
            .arg(&root)
            .arg("-d")
            .arg(&root)
            .arg(root.join(source))
            .output()
            .unwrap()
    };
    assert!(compile("p/Base.java").status.success());
    let out = compile("q/Child.java");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        Command::new(java)
            .arg("-cp")
            .arg(&root)
            .arg("q.Child")
            .status()
            .unwrap()
            .success()
    );
    for expression in ["b.priv()", "b.pkg()", "b.f()"] {
        fs::write(root.join("q/Bad.java"), format!("package q; public class Bad extends p.Base {{ int bad(p.Base b) {{return {expression};}} }}")).unwrap();
        assert!(
            !compile("q/Bad.java").status.success(),
            "accepted {expression}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_local_android_sdk_declarations() {
    use decomp::java_external_metadata::{Classpath, ExternalMetadata};
    let cp = Classpath::from_paths(["/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"])
        .unwrap();
    let key = java_aliases::MethodKey {
        owner: "Landroid/app/Activity;".into(),
        name: "onCreate".into(),
        parameters: vec!["Landroid/os/Bundle;".into()],
        return_type: "V".into(),
    };
    assert_eq!(cp.method_access(&key).unwrap() & 7, 4);
    assert_eq!(cp.class_access("Landroid/app/Activity;").unwrap() & 1, 1);
}
