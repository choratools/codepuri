#![allow(clippy::unwrap_used)]
use super::*;
struct DefaultResolver;
impl JavaResolver for DefaultResolver {
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("C".into())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "I".into(),
            name: "m".into(),
            is_static: false,
            return_type: JavaType::Int,
            parameters: vec![],
            throws: vec!["java.io.IOException".into()],
        })
    }
    fn super_invocation(&self, _: &str, r: MethodRef) -> Option<SuperInvocation> {
        Some(SuperInvocation {
            method: self.method(r)?,
            access_flags: 1,
        })
    }
    fn super_interface_qualifier(&self, _: &str, _: MethodRef) -> Option<String> {
        Some("I".into())
    }
}
#[test]
fn original_interface_super_emission_does_not_virtual_dispatch() {
    let mut b = Builder::default();
    let receiver = b.parameter(ManagedType::Reference(ClassRef::new(0)));
    let result = b.value(ManagedType::Int, |result| ManagedInsn::Invoke {
        kind: InvokeKind::Super,
        result: Some(result),
        method: MethodRef::new(0),
        args: vec![receiver],
    });
    b.push(ManagedInsn::Return {
        value: Some(result),
    });
    let ir = b.finish();
    let signature = JavaMethod {
        owner: "C".into(),
        name: "recovered".into(),
        is_static: false,
        return_type: JavaType::Int,
        parameters: vec![],
        throws: vec!["java.io.IOException".into()],
    };
    let bindings = ParameterBindings {
        receiver: Some(receiver),
        parameters: vec![],
        names: vec![],
    };
    let SourceResult::Complete { source, .. } =
        emit_method(&ir, &signature, &bindings, &DefaultResolver)
    else {
        panic!("interface source rejected")
    };
    assert!(source.contains("I.super.m()"), "{source}");
    let mut bad = signature.clone();
    bad.throws.clear();
    assert!(matches!(
        emit_method(&ir, &bad, &bindings, &DefaultResolver),
        SourceResult::Unsupported(_)
    ));
    let mut static_signature = signature.clone();
    static_signature.is_static = true;
    static_signature.parameters = vec![JavaType::Class("C".into())];
    let static_bindings = ParameterBindings {
        receiver: None,
        parameters: vec![receiver],
        names: vec![],
    };
    assert!(matches!(
        emit_method(&ir, &static_signature, &static_bindings, &DefaultResolver),
        SourceResult::Unsupported(_)
    ));
    let text = format!(
        "interface I {{default int m() throws java.io.IOException {{if(C.fail)throw new java.io.IOException(\"exact\");C.effects++;return 7;}}}} class C implements I {{static int effects;static boolean fail;public int m(){{return 99;}} {source}}} class D extends C {{public int m(){{return 111;}}}} class Main {{public static void main(String[]args)throws Exception{{D d=new D();if(d.recovered()!=7||C.effects!=1||d.m()!=111)throw new AssertionError();C.fail=true;try{{d.recovered();throw new AssertionError();}}catch(java.io.IOException e){{if(!e.getMessage().equals(\"exact\")||C.effects!=1)throw new AssertionError();}}}}}} "
    );
    for home in [
        "/home/jtjisgod/tools/jdk-17.0.13+11",
        "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1",
    ] {
        let home = std::path::Path::new(home);
        if !home.join("bin/javac").exists() {
            continue;
        }
        let dir = std::env::temp_dir().join(format!(
            "interface-super-{}-{}",
            std::process::id(),
            home.file_name().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Main.java"), &text).unwrap();
        let output = Command::new(home.join("bin/javac"))
            .arg(dir.join("Main.java"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new(home.join("bin/java"))
            .arg("-cp")
            .arg(&dir)
            .arg("Main")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
