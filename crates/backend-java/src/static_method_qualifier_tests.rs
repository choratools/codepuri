#![allow(clippy::unwrap_used)]
use super::*;
use crate::JavaType;
use std::{fs, path::Path, process::Command};

fn target() -> JavaMethod {
    JavaMethod {
        owner: "d1.Target".into(),
        name: "a".into(),
        is_static: true,
        return_type: JavaType::Long,
        parameters: vec![JavaType::Int],
        throws: vec![],
    }
}
#[test]
fn exact_class_evidence_only_for_primary_static_calls() {
    let mut method = target();
    assert_eq!(
        owner(&method, true, DeclaringType::Class).unwrap(),
        "((d1.Target) null)"
    );
    for kind in [DeclaringType::Interface, DeclaringType::Unknown] {
        assert!(owner(&method, true, kind).is_err());
        assert_eq!(owner(&method, false, kind).unwrap(), "d1.Target");
    }
    method.is_static = false;
    assert!(owner(&method, true, DeclaringType::Class).is_err());
    method.is_static = true;
    for name in ["<init>", "<clinit>"] {
        method.name = name.into();
        assert!(owner(&method, true, DeclaringType::Class).is_err());
    }
    method.name = "a".into();
    method.owner = "d1.Target; injected".into();
    assert!(owner(&method, true, DeclaringType::Class).is_err());
}
fn invoke(tool: &Path, args: &[&str]) -> std::process::Output {
    Command::new(tool).args(args).output().unwrap()
}
#[test]
fn original_shadow_fields_retained_and_null_primary_has_exact_argument_linkage_order() {
    let qualifier = owner(&target(), true, DeclaringType::Class).unwrap();
    for (version, home) in [
        ("17", "/home/jtjisgod/tools/jdk-17.0.13+11"),
        ("25", "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1"),
    ] {
        let root = std::env::temp_dir().join(format!(
            "static-method-qualifier-{}-{version}",
            std::process::id()
        ));
        let src = root.join("src");
        let out = root.join("classes");
        fs::create_dir_all(src.join("sample")).unwrap();
        fs::create_dir_all(src.join("d1")).unwrap();
        fs::create_dir_all(&out).unwrap();
        let mut files = Vec::new();
        for (rel, source) in [
            (
                "sample/Trace.java",
                r#"package sample;public class Trace{public static String log="";public static int mode;public static final RuntimeException failure=new IllegalStateException("identity");public static int arg(){log+="A";if(mode==1)throw failure;return 7;}}"#,
            ),
            (
                "sample/Control.java",
                r#"package sample;public class Control{public static long call(){return d1.Target.a(Trace.arg());}}"#,
            ),
            (
                "sample/Run.java",
                r#"package sample; public class Run{public static void main(String[]a){Trace.mode=Integer.parseInt(a[1]);try{long n=a[0].equals("original")?Control.call():a[0].equals("boolean")?BooleanShadow.call():Shadow.call();System.out.println(n+":"+Trace.log);}catch(Throwable e){boolean same=e==Trace.failure||e.getCause()==Trace.failure;System.out.println(e.getClass().getName()+":"+same+":"+Trace.log);}}}"#,
            ),
            (
                "d1/Target.java",
                r#"package d1; public class Target {static {sample.Trace.log+="I";if(sample.Trace.mode==2)throw sample.Trace.failure;} public static long a(int n){sample.Trace.log+="C";return n;} }"#,
            ),
        ] {
            let to = src.join(rel);
            fs::write(&to, source).unwrap();
            files.push(to);
        }
        for (name, field) in [("Shadow", "long"), ("BooleanShadow", "boolean")] {
            let path = src.join(format!("sample/{name}.java"));
            fs::write(&path, format!("package sample; public class {name} {{ public static {field} d1; public static long call() {{ return {qualifier}.a(Trace.arg()); }} }}")).unwrap();
            files.push(path);
        }
        let javac = Path::new(home).join("bin/javac");
        let java = Path::new(home).join("bin/java");
        let compiled = Command::new(&javac)
            .arg("-d")
            .arg(&out)
            .args(&files)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        for (name, source, diagnostic) in [
            (
                "Bad",
                "package sample; class Bad {static long d1; static long call(){return d1.Target.a(7);}}",
                "long cannot be dereferenced",
            ),
            (
                "InterfaceBad",
                "package sample; class InterfaceBad {static long call(){return ((d1.Face)null).a(7);}}",
                "illegal static interface method call",
            ),
        ] {
            let face = src.join("d1/Face.java");
            fs::write(
                &face,
                "package d1; public interface Face {static long a(int n){return n;}}",
            )
            .unwrap();
            let bad = src.join(format!("sample/{name}.java"));
            fs::write(&bad, source).unwrap();
            let check = Command::new(&javac)
                .arg("-cp")
                .arg(&out)
                .arg("-d")
                .arg(&out)
                .arg(&face)
                .arg(&bad)
                .output()
                .unwrap();
            assert!(!check.status.success());
            assert!(String::from_utf8_lossy(&check.stderr).contains(diagnostic));
        }
        for (mode, expected) in [
            ("0", "7:AIC"),
            ("1", "java.lang.IllegalStateException:true:A"),
            ("2", "java.lang.ExceptionInInitializerError:true:AI"),
        ] {
            for variant in ["original", "fixed", "boolean"] {
                let run = invoke(
                    &java,
                    &["-cp", out.to_str().unwrap(), "sample.Run", variant, mode],
                );
                assert!(run.status.success());
                assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), expected);
            }
        }
        let dump = invoke(
            &Path::new(home).join("bin/javap"),
            &["-cp", out.to_str().unwrap(), "-c", "sample.Shadow"],
        );
        let bytecode = String::from_utf8(dump.stdout).unwrap();
        let cast = bytecode.find("checkcast").unwrap();
        let argument = bytecode.find("Trace.arg").unwrap();
        assert!(cast < argument);
        assert!(bytecode.contains("Target.a:(I)J"));
        assert!(bytecode.contains("pop"));
        fs::remove_file(out.join("d1/Target.class")).unwrap();
        for (mode, expected) in [
            ("0", "java.lang.NoClassDefFoundError:false:A"),
            ("1", "java.lang.IllegalStateException:true:A"),
        ] {
            for variant in ["original", "fixed", "boolean"] {
                let run = invoke(
                    &java,
                    &["-cp", out.to_str().unwrap(), "sample.Run", variant, mode],
                );
                assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), expected);
            }
        }
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn resolver_attestations_remain_independent_and_default_closed() {
    use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
    struct Evidence {
        shadow: bool,
        flags: Option<u32>,
    }
    impl crate::JavaResolver for Evidence {
        fn method(&self, _: MethodRef) -> Option<JavaMethod> {
            Some(target())
        }
        fn field(&self, _: FieldRef) -> Option<crate::JavaField> {
            None
        }
        fn class(&self, _: ClassRef) -> Option<String> {
            None
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
        fn static_method_type_context(&self, _: MethodRef, _: &str) -> bool {
            self.shadow
        }
        fn static_method_owner_access(&self, _: MethodRef) -> Option<u32> {
            self.flags
        }
    }
    let reference = MethodRef::new(1);
    for flags in [None, Some(0x201)] {
        assert!(
            resolved_owner(
                &target(),
                reference,
                "sample.C",
                &Evidence {
                    shadow: true,
                    flags
                }
            )
            .is_err()
        );
        assert_eq!(
            resolved_owner(
                &target(),
                reference,
                "sample.C",
                &Evidence {
                    shadow: false,
                    flags
                }
            )
            .unwrap(),
            "d1.Target"
        );
    }
    assert_eq!(
        resolved_owner(
            &target(),
            reference,
            "sample.C",
            &Evidence {
                shadow: true,
                flags: Some(1)
            }
        )
        .unwrap(),
        "((d1.Target) null)"
    );
}
