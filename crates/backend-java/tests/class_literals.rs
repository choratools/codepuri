//! Exact class identity and non-initializing Java semantics.
#![allow(clippy::unwrap_used)]
#[path = "../src/class_literal.rs"]
mod class_literal;
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType, Unsupported};
use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef};
struct Types;
impl JavaResolver for Types {
    fn instruction_type(&self, r: TypeRef) -> Option<JavaType> {
        match r.index() {
            0 => Some(JavaType::Int),
            1 => Some(JavaType::Void),
            2 => Some(JavaType::Array(Box::new(JavaType::Array(Box::new(
                JavaType::Float,
            ))))),
            3 => Some(JavaType::Class("LiteralTest.Target".into())),
            4 => Some(JavaType::Array(Box::new(JavaType::Void))),
            _ => None,
        }
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        panic!("primitive/array targets are not class-only references")
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
fn render(ty: &JavaType) -> Result<String, Unsupported> {
    Ok(match ty {
        JavaType::Int => "int".into(),
        JavaType::Void => "void".into(),
        JavaType::Float => "float".into(),
        JavaType::Class(name) => name.clone(),
        JavaType::Array(element) => format!("{}[]", render(element)?),
        _ => unreachable!(),
    })
}
#[test]
fn rejects_missing_and_invalid_array_targets() {
    assert!(class_literal::expression(&Types, TypeRef::new(4), render).is_err());
    assert!(class_literal::expression(&Types, TypeRef::new(5), render).is_err());
}
#[test]
fn emitted_literals_compile_and_preserve_identity_without_initialization() {
    let literals: Vec<_> = (0..4)
        .map(|index| class_literal::expression(&Types, TypeRef::new(index), render).unwrap())
        .collect();
    assert_eq!(
        literals[..3],
        ["int.class", "void.class", "float[][].class"]
    );
    let root =
        std::env::temp_dir().join(format!("decomphify-class-literal-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = format!(
        "public class LiteralTest {{ static int initialized; static class Target {{ static {{ initialized++; }} }} public static void main(String[] args) {{ Class<?>[] values = {{ {} }}; if(values[0] != Integer.TYPE || values[1] != Void.TYPE || values[2] != float[][].class || values[3] != Target.class || initialized != 0) throw new AssertionError(); }} }}",
        literals.join(", ")
    );
    std::fs::write(root.join("LiteralTest.java"), source).unwrap();
    let jdk = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
    assert!(
        std::process::Command::new(format!("{jdk}/javac"))
            .arg(root.join("LiteralTest.java"))
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new(format!("{jdk}/java"))
            .args(["-cp", root.to_str().unwrap(), "LiteralTest"])
            .status()
            .unwrap()
            .success()
    );
}
