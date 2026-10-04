//! Inert-chain evidence and Java17 ordering regressions.
#![allow(clippy::unwrap_used)]
#[path = "../src/constructor_prechecks.rs"]
mod proof;
use proof::*;
use std::collections::{BTreeMap, BTreeSet};

fn identity(owner: &str) -> ConstructorIdentity {
    ConstructorIdentity {
        owner: owner.into(),
        descriptor: "()V".into(),
    }
}
fn declaration(owner: &str, parent: Option<&str>, flags: u32) -> ClassEvidence {
    ClassEvidence {
        owner: owner.into(),
        flags,
        superclass: parent.map(str::to_owned),
        methods: BTreeSet::from([("<init>".into(), "()V".into())]),
    }
}
fn root() -> ObjectRootEvidence {
    ObjectRootEvidence {
        declaration: declaration("Ljava/lang/Object;", None, 1),
        finalizer_operations: None,
        constructor: ConstructorEvidence {
            identity: identity("Ljava/lang/Object;"),
            operations: vec![Operation::Return],
            has_control_or_handlers: false,
        },
    }
}
fn evidence() -> (
    BTreeMap<String, ClassEvidence>,
    BTreeMap<ConstructorIdentity, ConstructorEvidence>,
) {
    let classes = BTreeMap::from([
        ("LC;".into(), declaration("LC;", Some("LP;"), 0x11)),
        (
            "LP;".into(),
            declaration("LP;", Some("Ljava/lang/Object;"), 1),
        ),
    ]);
    let method = ConstructorEvidence {
        identity: identity("LP;"),
        operations: vec![
            Operation::Pure,
            Operation::Chain(identity("Ljava/lang/Object;")),
            Operation::Return,
        ],
        has_control_or_handlers: false,
    };
    (classes, BTreeMap::from([(method.identity.clone(), method)]))
}
#[test]
fn exact_inert_parent_chain_accepts() {
    let (classes, constructors) = evidence();
    let proved =
        prove_inert_chain("LC;", &identity("LP;"), &classes, &constructors, &root()).unwrap();
    assert_eq!(proved.caller(), "LC;");
    assert_eq!(
        proved.chain(),
        &[identity("LP;"), identity("Ljava/lang/Object;")]
    );
}
#[test]
fn finalizers_effects_missing_metadata_and_cycles_reject() {
    let (mut classes, constructors) = evidence();
    classes.get_mut("LC;").unwrap().flags = 1;
    assert_eq!(
        prove_inert_chain("LC;", &identity("LP;"), &classes, &constructors, &root()).unwrap_err(),
        Rejection::NonFinalCaller
    );
    classes.get_mut("LC;").unwrap().flags = 0x11;
    classes
        .get_mut("LP;")
        .unwrap()
        .methods
        .insert(("finalize".into(), "()V".into()));
    assert_eq!(
        prove_inert_chain("LC;", &identity("LP;"), &classes, &constructors, &root()).unwrap_err(),
        Rejection::Finalizer
    );
    classes
        .get_mut("LP;")
        .unwrap()
        .methods
        .remove(&("finalize".into(), "()V".into()));
    let mut altered = constructors.clone();
    altered
        .get_mut(&identity("LP;"))
        .unwrap()
        .operations
        .insert(0, Operation::Observable);
    assert_eq!(
        prove_inert_chain("LC;", &identity("LP;"), &classes, &altered, &root()).unwrap_err(),
        Rejection::ObservableChain
    );
    altered.get_mut(&identity("LP;")).unwrap().operations =
        vec![Operation::Chain(identity("LP;")), Operation::Return];
    assert_eq!(
        prove_inert_chain("LC;", &identity("LP;"), &classes, &altered, &root()).unwrap_err(),
        Rejection::ObservableChain
    );
    let mut untrusted = root();
    untrusted
        .constructor
        .operations
        .insert(0, Operation::Observable);
    assert_eq!(
        prove_inert_chain("LC;", &identity("LP;"), &classes, &constructors, &untrusted)
            .unwrap_err(),
        Rejection::UntrustedRoot
    );
    classes.remove("LP;");
    assert_eq!(
        prove_inert_chain("LC;", &identity("LP;"), &classes, &constructors, &root()).unwrap_err(),
        Rejection::MissingDeclaration
    );
}
#[test]
fn class_initialization_throwing_cast_and_handler_summaries_are_not_inert() {
    let (classes, constructors) = evidence();
    // All of these must be summarized as Observable, never Pure: invoking a
    // static initializer, writing a field, allocating, or evaluating a throwing cast.
    for at in [0, 1, 2] {
        let mut changed = constructors.clone();
        changed
            .get_mut(&identity("LP;"))
            .unwrap()
            .operations
            .insert(at, Operation::Observable);
        assert_eq!(
            prove_inert_chain("LC;", &identity("LP;"), &classes, &changed, &root()).unwrap_err(),
            Rejection::ObservableChain
        );
    }
    let mut changed = constructors;
    changed
        .get_mut(&identity("LP;"))
        .unwrap()
        .has_control_or_handlers = true;
    assert_eq!(
        prove_inert_chain("LC;", &identity("LP;"), &classes, &changed, &root()).unwrap_err(),
        Rejection::ObservableChain
    );
}

#[test]
fn java17_preserves_original_check_frame_and_order_without_helper() {
    let dir = std::env::temp_dir().join(format!("decomphify-precheck-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = r#"
public class PrecheckOrder {
 static String trace="";
 static class Check {
  static { trace+="C"; }
  static void check(Object a,String name) {
   trace+=name;
   if(a==null) {
    StackTraceElement e=new Throwable().getStackTrace()[1];
    throw new IllegalArgumentException(e.getClassName()+"."+e.getMethodName()+":"+name);
   }
  }
 }
 static class Inert { Inert() {} }
 static final class Child extends Inert {
  Child(Object a,Object b) { super(); Check.check(a,"a"); Check.check(b,"b"); trace+="W"; }
 }
 static String run(Object a,Object b) {
  trace="";
  try { new Child(a,b); return trace; }
  catch(IllegalArgumentException e) { return trace+"|"+e.getMessage(); }
 }
 public static void main(String[] args) {
  String first=run(new Object(),new Object());
  if(!first.equals("CabW")) throw new AssertionError(first);
  String a=run(null,new Object());
  if(!a.equals("a|PrecheckOrder$Child.<init>:a")) throw new AssertionError(a);
  String b=run(new Object(),null);
  if(!b.equals("ab|PrecheckOrder$Child.<init>:b")) throw new AssertionError(b);
  if(!run(new Object(),new Object()).equals("abW")) throw new AssertionError();
 }
}
"#;
    std::fs::write(dir.join("PrecheckOrder.java"), source).unwrap();
    let compile = std::process::Command::new("javac")
        .args(["--release", "17", "PrecheckOrder.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new("java")
        .args(["-cp", ".", "PrecheckOrder"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
