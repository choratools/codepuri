#![allow(clippy::unwrap_used)]
use super::*;
fn classes(owner: &str, mode: u8) -> Option<ProtectedClass> {
    let (parent, named, interface, interfaces, declaration) = match owner {
        "Child" => (Some("Middle"), mode == 1, false, vec!["I"], None),
        "Middle" => (
            Some(if mode == 2 { "Child" } else { "Base" }),
            mode == 3,
            false,
            vec![],
            None,
        ),
        "Base" => (None, false, false, vec![], Some(4)),
        "I" => (
            None,
            mode == 4,
            true,
            if mode == 5 { vec!["I"] } else { vec![] },
            None,
        ),
        _ => return None,
    };
    if mode == 6 && owner == "Middle" {
        return None;
    }
    Some(ProtectedClass {
        parent: parent.map(str::to_owned),
        named,
        interface,
        interfaces: interfaces.into_iter().map(str::to_owned).collect(),
        declaration,
    })
}
#[test]
fn protected_ancestor_requires_complete_unshadowed_class_and_interface_path() {
    assert!(protected_path("Child", "Base", |o| classes(o, 0)));
    for mode in 1..=6 {
        assert!(
            !protected_path("Child", "Base", |o| classes(o, mode)),
            "mode {mode}"
        );
    }
    assert!(!protected_path("Child", "Foreign", |o| classes(o, 0)));
}
#[test]
fn genuine_multilevel_protected_receiver_source_compiles_and_illegal_base_receiver_fails() {
    let dir =
        std::env::temp_dir().join(format!("decomp-protected-ancestor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(dir.join("base")).unwrap();
    std::fs::create_dir_all(dir.join("child")).unwrap();
    std::fs::write(
        dir.join("base/Base.java"),
        "package base; public class Base {protected int size(){return 37;}}",
    )
    .unwrap();
    std::fs::write(
        dir.join("base/Middle.java"),
        "package base; public class Middle extends Base {}",
    )
    .unwrap();
    let source = "package child; public class Child extends base.Middle {public int good(Child receiver){return receiver.size();} public static void main(String[]a){Child c=new Child();if(c.good(c)!=37)throw new AssertionError();}}";
    std::fs::write(dir.join("child/Child.java"), source).unwrap();
    for jdk in [
        "/home/jtjisgod/tools/jdk-17.0.13+11",
        "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1",
    ] {
        let compile = std::process::Command::new(format!("{jdk}/bin/javac"))
            .arg("-d")
            .arg(&dir)
            .args([
                dir.join("base/Base.java"),
                dir.join("base/Middle.java"),
                dir.join("child/Child.java"),
            ])
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let run = std::process::Command::new(format!("{jdk}/bin/java"))
            .arg("-cp")
            .arg(&dir)
            .arg("child.Child")
            .output()
            .unwrap();
        assert!(run.status.success());
        let bad = source.replace("good(Child receiver)", "good(base.Base receiver)");
        std::fs::write(dir.join("child/Child.java"), bad).unwrap();
        let compile = std::process::Command::new(format!("{jdk}/bin/javac"))
            .arg("-cp")
            .arg(&dir)
            .arg("-d")
            .arg(&dir)
            .arg(dir.join("child/Child.java"))
            .output()
            .unwrap();
        assert!(!compile.status.success());
        assert!(String::from_utf8_lossy(&compile.stderr).contains("protected access"));
        std::fs::write(dir.join("child/Child.java"), source).unwrap();
    }
}

#[test]
fn multilevel_receiver_bridge_keeps_virtual_dispatch_effect_order_and_exception_identity() {
    let dir = std::env::temp_dir().join(format!("decomp-protected-bridge-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("base")).unwrap();
    std::fs::create_dir_all(dir.join("child")).unwrap();
    std::fs::write(
        dir.join("base/Base.java"),
        r#"package base;
public class Base {
 public static String log=""; public static Throwable failure;
 public void step() throws java.io.IOException {
  log+="step;";
  if(failure instanceof java.io.IOException)throw (java.io.IOException)failure;
  if(failure instanceof RuntimeException)throw (RuntimeException)failure;
  if(failure instanceof Error)throw (Error)failure;
 }
 protected int size(){log+="base;";return 37;}
}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("base/Middle.java"),
        "package base; public class Middle extends Base {}",
    )
    .unwrap();
    // The baseline has the genuine checked declaration. The source projection
    // keeps its fixed no-throws override using the existing identity rethrow
    // recipe; protected dispatch is still ordinary invokevirtual, never super.
    std::fs::write(dir.join("child/Child.java"), r#"package child;
public class Child extends base.Middle {
 public int original(Child receiver) throws java.io.IOException {
  step();return receiver.size();
 }
 public int projected(Child receiver) {
  try {step();}catch(Exception e){throw Child.<RuntimeException>raise(e);}
  return receiver.size();
 }
 @SuppressWarnings("unchecked") static <T extends Throwable> RuntimeException raise(Throwable t)throws T{throw(T)t;}
 public static class Derived extends Child {
  protected int size(){log+="derived;";return 91;}
 }
 public static void main(String[]args)throws Exception {
  for(Throwable failure:new Throwable[]{null,new java.io.IOException("io"),new IllegalStateException("runtime"),new AssertionError("error")}) {
   for(Child receiver:new Child[]{new Child(),new Derived(),null}) {
    Child caller=new Child();base.Base.failure=failure;log="";
    Object before;try{before=Integer.valueOf(caller.original(receiver));}catch(Throwable t){before=t;}
    String beforeLog=log;log="";
    Object after;try{after=Integer.valueOf(caller.projected(receiver));}catch(Throwable t){after=t;}
    if(!beforeLog.equals(log))throw new AssertionError(beforeLog+" != "+log);
    if(failure!=null){if(before!=failure||after!=failure)throw new AssertionError("identity");}
    else if(receiver==null){if(!(before instanceof NullPointerException)||!(after instanceof NullPointerException))throw new AssertionError("null");}
    else if(!before.equals(after))throw new AssertionError("dispatch");
   }
  }
 }
}"#).unwrap();
    for jdk in [
        "/home/jtjisgod/tools/jdk-17.0.13+11",
        "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1",
    ] {
        let compile = std::process::Command::new(format!("{jdk}/bin/javac"))
            .arg("-d")
            .arg(&dir)
            .args([
                dir.join("base/Base.java"),
                dir.join("base/Middle.java"),
                dir.join("child/Child.java"),
            ])
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let run = std::process::Command::new(format!("{jdk}/bin/java"))
            .arg("-cp")
            .arg(&dir)
            .arg("child.Child")
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
}
