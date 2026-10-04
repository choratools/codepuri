//! Use genuine JDK overloads to check contextual expressions, not only IR types.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, SourceResult,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::{Constant, InvokeKind},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};

struct Jdk;
impl JavaResolver for Jdk {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let (owner, name, parameters, return_type) = match r.index() {
            0 => (
                "java.util.Arrays",
                "hashCode",
                vec![JavaType::Array(Box::new(JavaType::Int))],
                JavaType::Int,
            ),
            1 => (
                "java.lang.String",
                "valueOf",
                vec![JavaType::Class("java.lang.Object".into())],
                JavaType::Class("java.lang.String".into()),
            ),
            _ => return None,
        };
        Some(JavaMethod {
            owner: owner.into(),
            name: name.into(),
            parameters,
            return_type,
            is_static: true,
            throws: vec![],
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("java.lang.String".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}

fn fixture(constant: Constant) -> SourceResult {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut zero = None;
    let first = insns.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Zero));
        zero = Some(result);
        ManagedInsn::Const {
            result,
            value: constant,
        }
    });
    let zero = zero.unwrap();
    let hash = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(0),
        args: vec![zero],
    });
    let mut string = None;
    let stringify = insns.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
        string = Some(result);
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(1),
            args: vec![zero],
        }
    });
    let ret = insns.insert(ManagedInsn::Return { value: string });
    let mut cfg = Cfg::new(first);
    let mut previous = cfg.entry();
    for instruction in [hash, stringify, ret] {
        let next = cfg.add_block(instruction);
        cfg.add_edge(previous, next, EdgeKind::Unconditional)
            .unwrap();
        previous = next;
    }
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    let method = JavaMethod {
        owner: "ContextualNullDispatch".into(),
        name: "check".into(),
        parameters: vec![],
        return_type: JavaType::Class("java.lang.String".into()),
        is_static: true,
        throws: vec![],
    };
    backend_java::emit_method(
        &ir,
        &method,
        &ParameterBindings {
            receiver: None,
            parameters: vec![],
            names: vec![],
        },
        &Jdk,
    )
}

#[test]
fn null_is_typed_at_each_descriptor_use_and_invalid_bits_reject() {
    let SourceResult::Complete { source, .. } = fixture(Constant::Zero) else {
        panic!("contextual zero rejected")
    };
    assert!(
        source.contains("java.util.Arrays.hashCode(((int[]) null))"),
        "{source}"
    );
    assert!(
        source.contains("java.lang.String.valueOf(((java.lang.Object) null))"),
        "{source}"
    );
    let rejected = [Constant::Int(1), Constant::Int(-1), Constant::Int(i32::MAX)]
        .into_iter()
        .filter(|constant| matches!(fixture(*constant), SourceResult::Unsupported(_)))
        .count();
    assert_eq!(rejected, 3, "nonzero bits must never become null");
    let dir = std::env::temp_dir().join(format!("decomphify-null-dispatch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ContextualNullDispatch.java");
    let java = format!(
        "public class ContextualNullDispatch {{ {source} public static void main(String[] args) {{ if (!\"null\".equals(check())) throw new AssertionError(); }} }}"
    );
    std::fs::write(&path, &java).unwrap();
    let compile = std::process::Command::new("javac")
        .args(["--release", "17", "-d"])
        .arg(&dir)
        .arg(&path)
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
        .arg("ContextualNullDispatch")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    // Differential oracle: the old bare expression is ambiguous in the real
    // JDK overload family. No substitute declarations or stub classpath.
    std::fs::write(
        &path,
        java.replace("((int[]) null)", "null")
            .replace("((java.lang.Object) null)", "null"),
    )
    .unwrap();
    let before = std::process::Command::new("javac")
        .args(["--release", "17", "-d"])
        .arg(&dir)
        .arg(&path)
        .output()
        .unwrap();
    assert!(!before.status.success());
    assert!(String::from_utf8_lossy(&before.stderr).contains("reference to hashCode is ambiguous"));
    // Fixing only the ambiguous array family is insufficient: bare null also
    // silently selects valueOf(char[]) instead of the original Object descriptor.
    std::fs::write(&path, java.replace("((java.lang.Object) null)", "null")).unwrap();
    let partial = std::process::Command::new("javac")
        .args(["--release", "17", "-d"])
        .arg(&dir)
        .arg(&path)
        .output()
        .unwrap();
    assert!(partial.status.success());
    let wrong_dispatch = std::process::Command::new("java")
        .arg("-cp")
        .arg(&dir)
        .arg("ContextualNullDispatch")
        .output()
        .unwrap();
    assert!(!wrong_dispatch.status.success());
    assert!(String::from_utf8_lossy(&wrong_dispatch.stderr).contains("NullPointerException"));
    std::fs::remove_dir_all(dir).unwrap();
}
