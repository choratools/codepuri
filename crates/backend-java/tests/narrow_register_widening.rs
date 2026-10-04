//! Narrow scalar registers retain sign/zero extension and descriptor dispatch.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, SourceResult,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::InvokeKind,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};
struct Resolver(JavaType);
impl JavaResolver for Resolver {
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "NarrowRegister".into(),
            name: "input".into(),
            ty: self.0.clone(),
            is_static: true,
        })
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "NarrowRegister".into(),
            name: "pick".into(),
            parameters: vec![JavaType::Int],
            return_type: JavaType::Int,
            is_static: true,
            throws: vec![],
        })
    }
}
fn source(ty: JavaType, managed: ManagedType) -> SourceResult {
    let mut instructions = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut input = None;
    let entry = instructions.insert_with_key(|id| {
        let result = values.insert(Value::new(id, managed));
        input = Some(result);
        ManagedInsn::FieldGet {
            result,
            object: None,
            field: FieldRef::new(0),
        }
    });
    let mut output = None;
    let call = instructions.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Int));
        output = Some(result);
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(0),
            args: vec![input.unwrap()],
        }
    });
    let ret = instructions.insert(ManagedInsn::Return { value: output });
    let mut cfg = Cfg::new(entry);
    let mut previous = cfg.entry();
    for id in [call, ret] {
        let next = cfg.add_block(id);
        cfg.add_edge(previous, next, EdgeKind::Unconditional)
            .unwrap();
        previous = next;
    }
    let ir = MethodIr::<SsaForm>::from_parts(cfg, instructions, values);
    let signature = JavaMethod {
        owner: "NarrowRegister".into(),
        name: "check".into(),
        parameters: vec![],
        return_type: JavaType::Int,
        is_static: true,
        throws: vec![],
    };
    backend_java::emit_method(
        &ir,
        &signature,
        &ParameterBindings {
            receiver: None,
            parameters: vec![],
            names: vec![],
        },
        &Resolver(ty),
    )
}
#[test]
fn narrow_fields_use_int_descriptor_and_preserve_boundary_values() {
    for (ty, managed, literal, expected) in [
        (JavaType::Byte, ManagedType::Byte, "-128", -128),
        (JavaType::Short, ManagedType::Short, "-32768", -32768),
        (JavaType::Char, ManagedType::Char, "65535", 65535),
    ] {
        let SourceResult::Complete { source, .. } = source(ty.clone(), managed) else {
            panic!("narrow field rejected");
        };
        let root = std::env::temp_dir().join(format!(
            "decomphify-narrow-register-{}-{}",
            std::process::id(),
            match ty {
                JavaType::Byte => "byte",
                JavaType::Short => "short",
                JavaType::Char => "char",
                _ => unreachable!(),
            }
        ));
        std::fs::create_dir_all(&root).unwrap();
        let java = format!(
            "public class NarrowRegister {{ static {} input = ({} ){}; static int pick(int x){{return x;}} static int pick({} x){{throw new AssertionError(\"wrong overload\");}} {} public static void main(String[] args){{if(check()!={})throw new AssertionError();}} }}",
            match ty {
                JavaType::Byte => "byte",
                JavaType::Short => "short",
                JavaType::Char => "char",
                _ => unreachable!(),
            },
            match ty {
                JavaType::Byte => "byte",
                JavaType::Short => "short",
                JavaType::Char => "char",
                _ => unreachable!(),
            },
            literal,
            match ty {
                JavaType::Byte => "byte",
                JavaType::Short => "short",
                JavaType::Char => "char",
                _ => unreachable!(),
            },
            source,
            expected
        );
        std::fs::write(root.join("NarrowRegister.java"), &java).unwrap();
        let compile = std::process::Command::new("javac")
            .arg("NarrowRegister.java")
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}\n{java}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let run = std::process::Command::new("java")
            .args(["-cp", ".", "NarrowRegister"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
}
#[test]
fn wider_boolean_and_reference_domains_are_not_numeric_widening() {
    use backend_java::type_compatibility::{Compatibility, Hierarchy, narrow_register_widening};
    for ty in [
        JavaType::Boolean,
        JavaType::Long,
        JavaType::Float,
        JavaType::Double,
        JavaType::Class("java.lang.Object".into()),
    ] {
        assert!(!narrow_register_widening(&ty, &JavaType::Int));
        assert_eq!(
            Hierarchy::default().assignable(&ty, &JavaType::Int),
            Compatibility::Incompatible
        );
    }
    assert!(!narrow_register_widening(&JavaType::Int, &JavaType::Byte));
    assert!(!narrow_register_widening(&JavaType::Byte, &JavaType::Long));
}
