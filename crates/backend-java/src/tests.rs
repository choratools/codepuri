// Fixture construction and successful-result assertions intentionally panic on failure.
#![allow(clippy::unwrap_used)]

use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    value::{InsnId, Value},
};
use std::{
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Resolver;

#[test]
fn protected_instance_receiver_preserves_attested_static_type_and_fails_closed() {
    struct Protected(bool);
    impl JavaResolver for Protected {
        fn method_access(&self, _: MethodRef) -> Option<u32> {
            Some(4)
        }
        fn field_access(&self, _: FieldRef) -> Option<u32> {
            Some(4)
        }
        fn retain_method_receiver(&self, _: MethodRef, caller: &str, receiver: &JavaType) -> bool {
            self.0 && caller == "q.Sub" && *receiver == JavaType::Class("q.Sub".into())
        }
        fn retain_field_receiver(&self, _: FieldRef, caller: &str, receiver: &JavaType) -> bool {
            self.retain_method_receiver(MethodRef::new(0), caller, receiver)
        }
        fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
            actual == expected
                || (*actual == JavaType::Class("q.Sub".into())
                    && *expected == JavaType::Class("p.Base".into()))
                || (*actual == JavaType::Class("java.lang.String".into())
                    && *expected == JavaType::Class("java.lang.Object".into()))
        }
        fn method(&self, _: MethodRef) -> Option<JavaMethod> {
            Some(JavaMethod {
                owner: "p.Base".into(),
                name: "pick".into(),
                is_static: false,
                return_type: JavaType::Int,
                parameters: vec![JavaType::Class("java.lang.Object".into())],
                throws: vec![],
            })
        }
        fn field(&self, _: FieldRef) -> Option<JavaField> {
            Some(JavaField {
                owner: "p.Base".into(),
                name: "field".into(),
                ty: JavaType::Int,
                is_static: false,
            })
        }
        fn class(&self, r: ClassRef) -> Option<String> {
            Some(
                if r.index() == 0 {
                    "q.Sub"
                } else {
                    "java.lang.String"
                }
                .into(),
            )
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
    }
    let mut b = Builder::default();
    let recv = b.parameter(ManagedType::Reference(ClassRef::new(0)));
    let arg = b.parameter(ManagedType::Reference(ClassRef::new(1)));
    let result = b.value(ManagedType::Int, |result| ManagedInsn::Invoke {
        kind: InvokeKind::Virtual,
        method: MethodRef::new(0),
        args: vec![recv, arg],
        result: Some(result),
    });
    b.push(ManagedInsn::Return {
        value: Some(result),
    });
    let ir = b.finish();
    let method = JavaMethod {
        owner: "q.Sub".into(),
        name: "call".into(),
        is_static: false,
        return_type: JavaType::Int,
        parameters: vec![JavaType::Class("java.lang.String".into())],
        throws: vec![],
    };
    let bindings = ParameterBindings {
        receiver: Some(recv),
        parameters: vec![arg],
        names: vec![],
    };
    let SourceResult::Complete { source, .. } =
        emit_method(&ir, &method, &bindings, &Protected(true))
    else {
        panic!("receiver proof rejected")
    };
    assert!(source.contains("this.pick((java.lang.Object) p0)"));
    assert!(!source.contains("((p.Base) this)"));
    let mut parameter_method = method.clone();
    parameter_method.name = "onOther".into();
    parameter_method.is_static = true;
    parameter_method
        .parameters
        .insert(0, JavaType::Class("q.Sub".into()));
    let parameter_bindings = ParameterBindings {
        receiver: None,
        parameters: vec![recv, arg],
        names: vec![],
    };
    let SourceResult::Complete {
        source: parameter_source,
        ..
    } = emit_method(
        &ir,
        &parameter_method,
        &parameter_bindings,
        &Protected(true),
    )
    else {
        panic!("static subclass receiver rejected")
    };
    assert!(parameter_source.contains("p0.pick((java.lang.Object) p1)"));
    assert!(matches!(
        emit_method(&ir, &method, &bindings, &Protected(false)),
        SourceResult::Unsupported(_)
    ));
    let mut b = Builder::default();
    let recv = b.parameter(ManagedType::Reference(ClassRef::new(0)));
    let field = b.value(ManagedType::Int, |result| ManagedInsn::FieldGet {
        result,
        object: Some(recv),
        field: FieldRef::new(0),
    });
    b.push(ManagedInsn::FieldPut {
        object: Some(recv),
        field: FieldRef::new(0),
        value: field,
    });
    b.push(ManagedInsn::Return { value: Some(field) });
    let ir = b.finish();
    let mut field_method = method.clone();
    field_method.name = "read".into();
    field_method.parameters.clear();
    let bindings = ParameterBindings {
        receiver: Some(recv),
        parameters: vec![],
        names: vec![],
    };
    let SourceResult::Complete { source: fields, .. } =
        emit_method(&ir, &field_method, &bindings, &Protected(true))
    else {
        panic!("field proof rejected")
    };
    assert!(fields.contains("this.field"));
    assert!(matches!(
        emit_method(&ir, &field_method, &bindings, &Protected(false)),
        SourceResult::Unsupported(_)
    ));
    let root = std::env::temp_dir().join(format!("protected-receiver-{}", std::process::id()));
    std::fs::create_dir_all(root.join("p")).unwrap();
    std::fs::create_dir_all(root.join("q")).unwrap();
    let base = root.join("p/Base.java");
    let sub = root.join("q/Sub.java");
    std::fs::write(&base, "package p; public class Base { protected int field; protected int pick(Object x) { return 1; } }").unwrap();
    std::fs::write(&sub, format!("package q; public class Sub extends p.Base {{ int pick(String x) {{ return 2; }} {source} {parameter_source} {fields} }}")).unwrap();
    let compile = || {
        Command::new("javac")
            .args(["--release", "17", "-d"])
            .arg(&root)
            .arg(&base)
            .arg(&sub)
            .output()
            .unwrap()
    };
    let result = compile();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for expression in [
        "((p.Base)this).pick(x)",
        "x.pick(null)",
        "x.field",
        "((p.Base)this).field",
    ] {
        std::fs::write(&sub, format!("package q; public class Sub extends p.Base {{ int bad(p.Base x) {{ return {expression}; }} }}")).unwrap();
        let result = compile();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("has protected access"));
    }
    std::fs::write(&sub, "package q; public class Sub extends p.Base { int good(Sub x) { x.field = 3; return x.pick(null); } }").unwrap();
    assert!(compile().status.success());
    std::fs::write(
        &sub,
        "package q; public class Sub { int bad(p.Base x) { return x.pick(null); } }",
    )
    .unwrap();
    assert!(!compile().status.success());
    std::fs::remove_dir_all(root).unwrap();
}
impl JavaResolver for Resolver {
    fn instruction_type(&self, r: TypeRef) -> Option<JavaType> {
        Some(match r.index() {
            0 => JavaType::Int,
            1 => JavaType::Array(Box::new(JavaType::Int)),
            2 => JavaType::Byte,
            3 => JavaType::Short,
            4 => JavaType::Char,
            5 => JavaType::Class("java.lang.String".into()),
            6 => JavaType::Array(Box::new(JavaType::Array(Box::new(JavaType::Int)))),
            7 => JavaType::Boolean,
            8 => JavaType::Long,
            9 => JavaType::Float,
            _ => return None,
        })
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        if r.index() == 1 {
            return Some(JavaMethod {
                owner: "java.lang.String".into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: vec![JavaType::Array(Box::new(JavaType::Char))],
                throws: vec![],
            });
        }
        Some(JavaMethod {
            owner: "Fixture".into(),
            name: "tick".into(),
            is_static: true,
            return_type: JavaType::Int,
            parameters: vec![JavaType::Int],
            throws: vec![],
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture".into(),
            name: "counter".into(),
            ty: JavaType::Int,
            is_static: true,
        })
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("java.lang.String".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        Some(vec![0x22, 0x5c, 0xa, 0xd800])
    }
}
#[derive(Default)]
struct Builder {
    insns: InsnArena,
    values: ValueArena,
    order: Vec<InsnId>,
}
impl Builder {
    fn value(&mut self, t: ManagedType, f: impl FnOnce(ValueId) -> ManagedInsn) -> ValueId {
        let mut result = None;
        let id = self.insns.insert_with_key(|id| {
            let v = self.values.insert(Value::new(id, t));
            result = Some(v);
            f(v)
        });
        self.order.push(id);
        result.unwrap()
    }
    fn parameter(&mut self, t: ManagedType) -> ValueId {
        let v = self.value(t, |v| ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        });
        self.order.pop();
        v
    }
    fn push(&mut self, i: ManagedInsn) {
        self.order.push(self.insns.insert(i));
    }
    fn finish(self) -> MethodIr<SsaForm> {
        let mut cfg = Cfg::new(self.order[0]);
        let mut last = cfg.entry();
        for id in &self.order[1..] {
            let next = cfg.add_block(*id);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}
fn signature(name: &str, result: JavaType, parameters: Vec<JavaType>) -> JavaMethod {
    JavaMethod {
        owner: "Fixture".into(),
        name: name.into(),
        is_static: true,
        return_type: result,
        parameters,
        throws: vec![],
    }
}
fn complete(ir: &MethodIr<SsaForm>, m: JavaMethod, b: ParameterBindings) -> String {
    match emit_method(ir, &m, &b, &Resolver) {
        SourceResult::Complete { source, .. } => source,
        other => panic!("{other:?}"),
    }
}
fn java(methods: &str, assertions: &str) {
    if Command::new("javac").arg("-version").output().is_err()
        || Command::new("java").arg("-version").output().is_err()
    {
        eprintln!("SKIP Java runtime assertions: javac/java unavailable");
        return;
    }
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "backend-java-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir).unwrap();
    let source = format!(
        "public class Fixture {{ static int counter; static int tick(int n) {{ counter++; return n + counter; }}\n{methods}\npublic static void main(String[] args) {{ {assertions} }} }}"
    );
    std::fs::write(dir.join("Fixture.java"), &source).unwrap();
    let compiled = Command::new("javac")
        .arg("Fixture.java")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let run = Command::new("java")
        .args(["-cp", ".", "Fixture"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn dex_wide_invoke_words_preserve_descriptor_and_effects() {
    struct Wide;
    impl JavaResolver for Wide {
        fn method(&self, _: MethodRef) -> Option<JavaMethod> {
            Some(JavaMethod {
                owner: "java.lang.Math".into(),
                name: "addExact".into(),
                is_static: true,
                return_type: JavaType::Long,
                parameters: vec![JavaType::Long, JavaType::Long],
                throws: vec![],
            })
        }
        fn class(&self, r: ClassRef) -> Option<String> {
            Resolver.class(r)
        }
        fn field(&self, r: FieldRef) -> Option<JavaField> {
            Resolver.field(r)
        }
        fn string(&self, r: StringRef) -> Option<Vec<u16>> {
            Resolver.string(r)
        }
    }
    let build = |bad: bool| {
        let mut b = Builder::default();
        let a = b.parameter(ManagedType::Long);
        let c = b.parameter(ManagedType::Long);
        let result = b.value(ManagedType::Long, |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(0),
            args: vec![a, if bad { c } else { a }, c, c],
        });
        b.push(ManagedInsn::Return {
            value: Some(result),
        });
        (
            b.finish(),
            ParameterBindings {
                receiver: None,
                parameters: vec![a, c],
                names: vec!["a".into(), "b".into()],
            },
        )
    };
    let m = signature(
        "wideAdd",
        JavaType::Long,
        vec![JavaType::Long, JavaType::Long],
    );
    let (ir, bindings) = build(false);
    let SourceResult::Complete { source, .. } = emit_method(&ir, &m, &bindings, &Wide) else {
        panic!("{:?}", emit_method(&ir, &m, &bindings, &Wide));
    };
    java(
        &source,
        "if(wideAdd(0x100000000L, 7L) != 0x100000007L) throw new AssertionError(); try { wideAdd(Long.MAX_VALUE, 1L); throw new AssertionError(); } catch(ArithmeticException expected) {}",
    );
    let (ir, bindings) = build(true);
    assert!(matches!(
        emit_method(&ir, &m, &bindings, &Wide),
        SourceResult::Unsupported(_)
    ));
}

#[test]
fn raw_zero_without_descriptor_consumer_is_category_one() {
    let mut b = Builder::default();
    let p = b.parameter(ManagedType::Int);
    let zero = b.value(ManagedType::Zero, |result| ManagedInsn::Const {
        result,
        value: Constant::Zero,
    });
    let sum = b.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: p,
        rhs: zero,
    });
    b.push(ManagedInsn::Return { value: Some(sum) });
    let source = complete(
        &b.finish(),
        signature("zeroAdd", JavaType::Int, vec![JavaType::Int]),
        ParameterBindings {
            receiver: None,
            parameters: vec![p],
            names: vec!["p".into()],
        },
    );
    java(
        &source,
        "if(zeroAdd(Integer.MIN_VALUE) != Integer.MIN_VALUE || zeroAdd(71) != 71) throw new AssertionError();",
    );

    let mut b = Builder::default();
    b.value(ManagedType::Zero, |result| ManagedInsn::Const {
        result,
        value: Constant::Zero,
    });
    b.push(ManagedInsn::Return { value: None });
    let source = complete(
        &b.finish(),
        signature("deadZero", JavaType::Void, vec![]),
        ParameterBindings::default(),
    );
    java(&source, "deadZero();");
}

#[test]
fn one_proven_zero_has_independent_numeric_and_reference_operands() {
    struct Fields;
    impl JavaResolver for Fields {
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            if r.index() == 3 {
                return Some(JavaMethod {
                    owner: "Fixture".into(),
                    name: "acceptByte".into(),
                    is_static: true,
                    return_type: JavaType::Void,
                    parameters: vec![JavaType::Byte],
                    throws: vec![],
                });
            }
            Resolver.method(r)
        }
        fn class(&self, r: ClassRef) -> Option<String> {
            Resolver.class(r)
        }
        fn string(&self, r: StringRef) -> Option<Vec<u16>> {
            Resolver.string(r)
        }
        fn field(&self, r: FieldRef) -> Option<JavaField> {
            let (name, ty) = match r.index() {
                0 => ("counter", JavaType::Int),
                1 => ("reference", JavaType::Class("java.lang.Object".into())),
                _ => ("floating", JavaType::Float),
            };
            Some(JavaField {
                owner: "Fixture".into(),
                name: name.into(),
                ty,
                is_static: true,
            })
        }
    }
    let mut b = Builder::default();
    let zero = b.value(ManagedType::Zero, |result| ManagedInsn::Const {
        result,
        value: Constant::Zero,
    });
    for field in 0..3 {
        b.push(ManagedInsn::FieldPut {
            field: FieldRef::new(field),
            object: None,
            value: zero,
        });
    }
    b.push(ManagedInsn::Return { value: None });
    let source = match emit_method(
        &b.finish(),
        &signature("reset", JavaType::Void, vec![]),
        &ParameterBindings::default(),
        &Fields,
    ) {
        SourceResult::Complete { source, .. } => source,
        other => panic!("{other:?}"),
    };
    java(
        &format!("static Object reference = new Object(); static float floating = 7;\n{source}"),
        "counter=9; reset(); if(counter!=0 || reference!=null || Float.floatToRawIntBits(floating)!=0) throw new AssertionError();",
    );
    let mut b = Builder::default();
    let zero = b.value(ManagedType::Zero, |result| ManagedInsn::Const {
        result,
        value: Constant::Zero,
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        method: MethodRef::new(3),
        args: vec![zero],
        result: None,
    });
    b.push(ManagedInsn::Return { value: None });
    let source = match emit_method(
        &b.finish(),
        &signature("narrow", JavaType::Void, vec![]),
        &ParameterBindings::default(),
        &Fields,
    ) {
        SourceResult::Complete { source, .. } => source,
        other => panic!("{other:?}"),
    };
    java(
        &format!("static void acceptByte(byte n) {{ counter=n; }}\n{source}"),
        "counter=7; narrow(); if(counter!=0) throw new AssertionError();",
    );
}

#[test]
fn arrays_materialize_effects_and_narrow_stores() {
    let mut b = Builder::default();
    let length = b.parameter(ManagedType::Int);
    let input = b.parameter(ManagedType::Int);
    let array = b.value(
        ManagedType::Array {
            element: Box::new(ManagedType::Byte),
            nullable: false,
        },
        |result| ManagedInsn::NewArray {
            result,
            element: TypeRef::new(2),
            length,
        },
    );
    let index = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(0),
    });
    b.push(ManagedInsn::ArrayPut {
        array,
        index,
        value: input,
    });
    let loaded = b.value(ManagedType::Int, |result| ManagedInsn::ArrayGet {
        result,
        array,
        index,
    });
    b.push(ManagedInsn::Return {
        value: Some(loaded),
    });
    let source = complete(
        &b.finish(),
        signature(
            "narrowStore",
            JavaType::Int,
            vec![JavaType::Int, JavaType::Int],
        ),
        ParameterBindings {
            parameters: vec![length, input],
            ..Default::default()
        },
    );
    assert!(source.contains("new byte[p0]"));
    assert!(source.contains("= (byte) p1;"));
    java(
        &source,
        "if (narrowStore(1, 255) != -1) throw new AssertionError(); try { narrowStore(0, 3); throw new AssertionError(); } catch (ArrayIndexOutOfBoundsException expected) {} try { narrowStore(-1, 3); throw new AssertionError(); } catch (NegativeArraySizeException expected) {}",
    );
}

#[test]
fn numeric_cast_preserves_short_narrowing_with_int_result() {
    let mut b = Builder::default();
    let input = b.parameter(ManagedType::Int);
    let result = b.value(ManagedType::Int, |result| ManagedInsn::Cast {
        result,
        value: input,
        target: TypeRef::new(3),
    });
    b.push(ManagedInsn::Return {
        value: Some(result),
    });
    let source = complete(
        &b.finish(),
        signature("narrow", JavaType::Int, vec![JavaType::Int]),
        ParameterBindings {
            parameters: vec![input],
            ..Default::default()
        },
    );
    assert!(source.contains("int v0 = (short) p0;"));
    java(
        &source,
        "if (narrow(65535) != -1 || narrow(65536) != 0) throw new AssertionError();",
    );
}

#[test]
fn reference_cast_and_instanceof_keep_source_type() {
    let mut b = Builder::default();
    let input = b.parameter(ManagedType::Unknown);
    let result = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::Cast {
            result,
            value: input,
            target: TypeRef::new(5),
        }
    });
    b.push(ManagedInsn::Return {
        value: Some(result),
    });
    let cast = complete(
        &b.finish(),
        signature(
            "checked",
            JavaType::Class("java.lang.String".into()),
            vec![JavaType::Class("java.lang.Object".into())],
        ),
        ParameterBindings {
            parameters: vec![input],
            ..Default::default()
        },
    );
    assert!(cast.contains("java.lang.Object p0"));
    assert!(cast.contains("(java.lang.String) (java.lang.Object) p0"));
    let mut b = Builder::default();
    let input = b.parameter(ManagedType::Unknown);
    let result = b.value(ManagedType::Boolean, |result| ManagedInsn::InstanceOf {
        result,
        value: input,
        target: TypeRef::new(5),
    });
    b.push(ManagedInsn::Return {
        value: Some(result),
    });
    let test = complete(
        &b.finish(),
        signature(
            "isString",
            JavaType::Boolean,
            vec![JavaType::Class("java.lang.Object".into())],
        ),
        ParameterBindings {
            parameters: vec![input],
            ..Default::default()
        },
    );
    java(
        &(cast + &test),
        "if (checked(null) != null || isString(null) || !isString(\"x\") || isString(new Object())) throw new AssertionError(); try { checked(new Object()); throw new AssertionError(); } catch (ClassCastException expected) {}",
    );
}

#[test]
fn multidimensional_array_allocates_outer_dimension_only() {
    let mut b = Builder::default();
    let length = b.parameter(ManagedType::Int);
    let ty = ManagedType::Array {
        element: Box::new(ManagedType::Array {
            element: Box::new(ManagedType::Int),
            nullable: true,
        }),
        nullable: false,
    };
    let result = b.value(ty, |result| ManagedInsn::NewArray {
        result,
        element: TypeRef::new(6),
        length,
    });
    b.push(ManagedInsn::Return {
        value: Some(result),
    });
    let source = complete(
        &b.finish(),
        signature(
            "matrix",
            JavaType::Array(Box::new(JavaType::Array(Box::new(JavaType::Int)))),
            vec![JavaType::Int],
        ),
        ParameterBindings {
            parameters: vec![length],
            ..Default::default()
        },
    );
    assert!(source.contains("new int[p0][]"));
    java(
        &source,
        "int[][] a = matrix(2); if (a.length != 2 || a[0] != null || a[1] != null) throw new AssertionError();",
    );
}

#[test]
fn fill_checks_before_mutation_and_length_is_materialized() {
    let mut b = Builder::default();
    let array = b.parameter(ManagedType::Array {
        element: Box::new(ManagedType::Int),
        nullable: true,
    });
    let one = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(11),
    });
    b.push(ManagedInsn::FillArray {
        array,
        values: vec![one, one],
    });
    let length = b.value(ManagedType::Int, |result| ManagedInsn::ArrayLength {
        result,
        array,
    });
    b.push(ManagedInsn::Return {
        value: Some(length),
    });
    let source = complete(
        &b.finish(),
        signature(
            "fill",
            JavaType::Int,
            vec![JavaType::Array(Box::new(JavaType::Int))],
        ),
        ParameterBindings {
            parameters: vec![array],
            ..Default::default()
        },
    );
    assert!(source.find("p0.length < 2").unwrap() < source.find("p0[0] =").unwrap());
    java(
        &source,
        "int[] a = {7}; try { fill(a); throw new AssertionError(); } catch (ArrayIndexOutOfBoundsException expected) {} if (a[0] != 7) throw new AssertionError(); try { fill(null); throw new AssertionError(); } catch (NullPointerException expected) {} int[] good = new int[3]; if (fill(good) != 3 || good[0] != 11 || good[1] != 11 || good[2] != 0) throw new AssertionError();",
    );
}

#[test]
fn allocation_uses_exact_adjacent_constructor_arguments() {
    let mut b = Builder::default();
    let chars = b.parameter(ManagedType::Array {
        element: Box::new(ManagedType::Char),
        nullable: true,
    });
    let result = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![result, chars],
    });
    b.push(ManagedInsn::Return {
        value: Some(result),
    });
    let source = complete(
        &b.finish(),
        signature(
            "construct",
            JavaType::Class("java.lang.String".into()),
            vec![JavaType::Array(Box::new(JavaType::Char))],
        ),
        ParameterBindings {
            parameters: vec![chars],
            ..Default::default()
        },
    );
    assert!(source.contains("new java.lang.String((char[]) p0)"));
    assert!(!source.contains(".<init>"));
    java(
        &source,
        "if (!construct(new char[] {'a', 'b'}).equals(\"ab\")) throw new AssertionError();",
    );
}

#[test]
fn invalid_managed_operations_fail_closed() {
    for target in [7, 100] {
        let mut b = Builder::default();
        let input = b.parameter(ManagedType::Int);
        let result = b.value(ManagedType::Int, |result| ManagedInsn::Cast {
            result,
            value: input,
            target: TypeRef::new(target),
        });
        b.push(ManagedInsn::Return {
            value: Some(result),
        });
        assert!(matches!(
            emit_method(
                &b.finish(),
                &signature("bad", JavaType::Int, vec![JavaType::Int]),
                &ParameterBindings {
                    parameters: vec![input],
                    ..Default::default()
                },
                &Resolver
            ),
            SourceResult::Unsupported(_)
        ));
    }
    let mut b = Builder::default();
    let result = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    b.push(ManagedInsn::Return {
        value: Some(result),
    });
    assert!(matches!(
        emit_method(
            &b.finish(),
            &signature(
                "uninitialized",
                JavaType::Class("java.lang.String".into()),
                vec![]
            ),
            &ParameterBindings::default(),
            &Resolver
        ),
        SourceResult::Unsupported(Unsupported::Instruction(_))
    ));
}

#[test]
fn primitive_array_roundtrips_preserve_bits_and_boolean_identity() {
    let cases = [
        (
            ManagedType::Boolean,
            JavaType::Boolean,
            "boolArray",
            "true",
            "!boolArray(true)",
        ),
        (
            ManagedType::Char,
            JavaType::Char,
            "charArray",
            "(char)65535",
            "charArray((char)65535) != 65535",
        ),
        (
            ManagedType::Long,
            JavaType::Long,
            "longArray",
            "Long.MIN_VALUE",
            "longArray(Long.MIN_VALUE) != Long.MIN_VALUE",
        ),
        (
            ManagedType::Float,
            JavaType::Float,
            "floatArray",
            "Float.intBitsToFloat(0x80000000)",
            "Float.floatToRawIntBits(floatArray(Float.intBitsToFloat(0x80000000))) != 0x80000000",
        ),
        (
            ManagedType::Double,
            JavaType::Double,
            "doubleArray",
            "Double.longBitsToDouble(0x8000000000000000L)",
            "Double.doubleToRawLongBits(doubleArray(Double.longBitsToDouble(0x8000000000000000L))) != 0x8000000000000000L",
        ),
    ];
    let mut sources = String::new();
    let mut assertions = String::new();
    for (managed, ty, name, expression, assertion) in cases {
        let mut b = Builder::default();
        let array = b.parameter(ManagedType::Array {
            element: Box::new(managed.clone()),
            nullable: true,
        });
        let input = b.parameter(managed.clone());
        let index = b.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(0),
        });
        b.push(ManagedInsn::ArrayPut {
            array,
            index,
            value: input,
        });
        let loaded = b.value(managed, |result| ManagedInsn::ArrayGet {
            result,
            array,
            index,
        });
        b.push(ManagedInsn::Return {
            value: Some(loaded),
        });
        let method_name = format!("{name}Impl");
        sources.push_str(&complete(
            &b.finish(),
            signature(
                &method_name,
                ty.clone(),
                vec![JavaType::Array(Box::new(ty.clone())), ty.clone()],
            ),
            ParameterBindings {
                parameters: vec![array, input],
                ..Default::default()
            },
        ));
        sources.push_str(&format!(
            "static {} {name}({} x) {{ return {method_name}(new {}[1], x); }}\n",
            ty.source().unwrap(),
            ty.source().unwrap(),
            ty.source().unwrap()
        ));
        assertions.push_str(&format!("if ({assertion}) throw new AssertionError(); try {{ {method_name}(null, {expression}); throw new AssertionError(); }} catch (NullPointerException expected) {{}}"));
    }
    java(&sources, &assertions);
}

#[test]
fn array_index_effects_execute_once_before_store_and_load() {
    let mut b = Builder::default();
    let array = b.parameter(ManagedType::Array {
        element: Box::new(ManagedType::Int),
        nullable: true,
    });
    let zero = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(0),
    });
    let index = b.value(ManagedType::Int, |result| ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: Some(result),
        method: MethodRef::new(0),
        args: vec![zero],
    });
    let value = b.value(ManagedType::Int, |result| ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: Some(result),
        method: MethodRef::new(0),
        args: vec![zero],
    });
    b.push(ManagedInsn::ArrayPut {
        array,
        index,
        value,
    });
    let result = b.value(ManagedType::Int, |result| ManagedInsn::ArrayGet {
        result,
        array,
        index,
    });
    b.push(ManagedInsn::Return {
        value: Some(result),
    });
    let source = complete(
        &b.finish(),
        signature(
            "arrayEffects",
            JavaType::Int,
            vec![JavaType::Array(Box::new(JavaType::Int))],
        ),
        ParameterBindings {
            parameters: vec![array],
            ..Default::default()
        },
    );
    assert_eq!(source.matches("Fixture.tick(").count(), 2);
    java(
        &source,
        "counter = 0; int[] a = new int[3]; if (arrayEffects(a) != 2 || a[1] != 2 || counter != 2) throw new AssertionError(); counter = 0; try { arrayEffects(null); throw new AssertionError(); } catch (NullPointerException expected) {} if (counter != 2) throw new AssertionError();",
    );
}

#[test]
fn sequenced_allocation_rejects_invalid_call_type_and_wrong_width_array_access() {
    let mut b = Builder::default();
    let chars = b.parameter(ManagedType::Array {
        element: Box::new(ManagedType::Char),
        nullable: true,
    });
    let result = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    b.value(ManagedType::Int, |result| ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: Some(result),
        method: MethodRef::new(0),
        args: vec![chars],
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![result, chars],
    });
    b.push(ManagedInsn::Return {
        value: Some(result),
    });
    assert!(matches!(
        emit_method(
            &b.finish(),
            &signature(
                "bad",
                JavaType::Class("java.lang.String".into()),
                vec![JavaType::Array(Box::new(JavaType::Char))]
            ),
            &ParameterBindings {
                parameters: vec![chars],
                ..Default::default()
            },
            &Resolver
        ),
        SourceResult::Unsupported(Unsupported::Type(_))
    ));
    let mut b = Builder::default();
    let array = b.parameter(ManagedType::Array {
        element: Box::new(ManagedType::Long),
        nullable: true,
    });
    let index = b.parameter(ManagedType::Long);
    let result = b.value(ManagedType::Long, |result| ManagedInsn::ArrayGet {
        result,
        array,
        index,
    });
    b.push(ManagedInsn::Return {
        value: Some(result),
    });
    assert!(matches!(
        emit_method(
            &b.finish(),
            &signature(
                "badIndex",
                JavaType::Long,
                vec![JavaType::Array(Box::new(JavaType::Long)), JavaType::Long]
            ),
            &ParameterBindings {
                parameters: vec![array, index],
                ..Default::default()
            },
            &Resolver
        ),
        SourceResult::Unsupported(Unsupported::Type(_))
    ));
}

#[test]
fn floating_payload_fill_preserves_raw_negative_zero_and_nan_bits() {
    let mut b = Builder::default();
    let array = b.parameter(ManagedType::Array {
        element: Box::new(ManagedType::Float),
        nullable: true,
    });
    let zero = b.value(ManagedType::Float, |result| ManagedInsn::Const {
        result,
        value: Constant::FloatBits(0x8000_0000),
    });
    let nan = b.value(ManagedType::Float, |result| ManagedInsn::Const {
        result,
        value: Constant::FloatBits(0x7fc0_0042),
    });
    b.push(ManagedInsn::FillArray {
        array,
        values: vec![zero, nan],
    });
    b.push(ManagedInsn::Return { value: None });
    let source = complete(
        &b.finish(),
        signature(
            "floatFill",
            JavaType::Void,
            vec![JavaType::Array(Box::new(JavaType::Float))],
        ),
        ParameterBindings {
            parameters: vec![array],
            ..Default::default()
        },
    );
    assert!(source.contains("intBitsToFloat(0x80000000)"));
    assert!(source.contains("intBitsToFloat(0x7fc00042)"));
    java(
        &source,
        "float[] a = new float[2]; floatFill(a); if (Float.floatToRawIntBits(a[0]) != 0x80000000 || Float.floatToRawIntBits(a[1]) != 0x7fc00042) throw new AssertionError();",
    );
}

#[test]
fn numeric_conversions_handle_nan_saturation_and_char_wrapping() {
    let mut sources = String::new();
    for (source_ty, java_source, result_ty, java_result, target, name) in [
        (
            ManagedType::Float,
            JavaType::Float,
            ManagedType::Int,
            JavaType::Int,
            0,
            "floatInt",
        ),
        (
            ManagedType::Long,
            JavaType::Long,
            ManagedType::Int,
            JavaType::Int,
            0,
            "longInt",
        ),
        (
            ManagedType::Int,
            JavaType::Int,
            ManagedType::Char,
            JavaType::Char,
            4,
            "intChar",
        ),
        (
            ManagedType::Int,
            JavaType::Int,
            ManagedType::Byte,
            JavaType::Byte,
            2,
            "intByte",
        ),
    ] {
        let mut b = Builder::default();
        let input = b.parameter(source_ty);
        let result = b.value(result_ty, |result| ManagedInsn::Cast {
            result,
            value: input,
            target: TypeRef::new(target),
        });
        b.push(ManagedInsn::Return {
            value: Some(result),
        });
        sources.push_str(&complete(
            &b.finish(),
            signature(name, java_result, vec![java_source]),
            ParameterBindings {
                parameters: vec![input],
                ..Default::default()
            },
        ));
    }
    java(
        &sources,
        "if (floatInt(Float.NaN) != 0 || floatInt(Float.POSITIVE_INFINITY) != Integer.MAX_VALUE || floatInt(Float.NEGATIVE_INFINITY) != Integer.MIN_VALUE || floatInt(-1.75f) != -1 || longInt(0x100000001L) != 1 || intChar(-1) != 65535 || intByte(255) != -1) throw new AssertionError();",
    );
}
#[test]
fn arithmetic_parameters_moves_and_widths_execute() {
    let mut b = Builder::default();
    let p = b.parameter(ManagedType::Unknown);
    let c = b.value(ManagedType::Int, |v| ManagedInsn::Const {
        result: v,
        value: Constant::Int(1),
    });
    let a = b.value(ManagedType::Int, |v| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result: v,
        lhs: p,
        rhs: c,
    });
    let mv = b.value(ManagedType::Int, |v| ManagedInsn::Move {
        result: v,
        value: a,
    });
    b.push(ManagedInsn::Return { value: Some(mv) });
    let source = complete(
        &b.finish(),
        signature("calc", JavaType::Int, vec![JavaType::Int]),
        ParameterBindings {
            parameters: vec![p],
            names: vec!["class".into()],
            ..Default::default()
        },
    );
    assert!(source.contains("int class_"));
    assert!(source.contains("return v1;"), "{source}");
    assert!(!source.contains("int v2 = v1;"), "{source}");
    let mut b = Builder::default();
    let p = b.parameter(ManagedType::Long);
    let c = b.value(ManagedType::Long, |v| ManagedInsn::Const {
        result: v,
        value: Constant::Long(1),
    });
    let a = b.value(ManagedType::Long, |v| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result: v,
        lhs: p,
        rhs: c,
    });
    b.push(ManagedInsn::Return { value: Some(a) });
    let wide = complete(
        &b.finish(),
        signature("wide", JavaType::Long, vec![JavaType::Long]),
        ParameterBindings {
            parameters: vec![p],
            ..Default::default()
        },
    );
    java(
        &(source + &wide),
        "if(calc(Integer.MAX_VALUE) != Integer.MIN_VALUE) throw new AssertionError(); if(wide(Long.MAX_VALUE) != Long.MIN_VALUE) throw new AssertionError();",
    );
}
#[test]
fn invokes_fields_and_unused_results_preserve_order() {
    let mut b = Builder::default();
    let c = b.value(ManagedType::Int, |v| ManagedInsn::Const {
        result: v,
        value: Constant::Int(10),
    });
    let call = b.value(ManagedType::Int, |v| ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: Some(v),
        method: MethodRef::new(0),
        args: vec![c],
    });
    b.push(ManagedInsn::FieldPut {
        object: None,
        field: FieldRef::new(0),
        value: call,
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(0),
        args: vec![call],
    });
    let f = b.value(ManagedType::Int, |v| ManagedInsn::FieldGet {
        result: v,
        object: None,
        field: FieldRef::new(0),
    });
    b.push(ManagedInsn::Return { value: Some(f) });
    let source = complete(
        &b.finish(),
        signature("effects", JavaType::Int, vec![]),
        ParameterBindings::default(),
    );
    assert_eq!(source.matches("Fixture.tick(").count(), 2);
    assert!(
        source.find("int v1 = Fixture.tick(v0);").unwrap()
            < source.find("Fixture.counter = v1;").unwrap()
    );
    assert!(
        source.find("Fixture.counter = v1;").unwrap() < source.find("Fixture.tick(v1);").unwrap()
    );
    assert!(
        source.find("Fixture.tick(v1);").unwrap()
            < source.find("int v2 = Fixture.counter;").unwrap()
    );
    java(
        &source,
        "if(effects() != 12 || counter != 12) throw new AssertionError();",
    );
}
#[test]
fn floating_bits_and_utf16_literals_execute() {
    let mut methods = String::new();
    for (name, t, j, c) in [
        (
            "negativeZero",
            ManagedType::Float,
            JavaType::Float,
            Constant::FloatBits(0x80000000),
        ),
        (
            "nan",
            ManagedType::Double,
            JavaType::Double,
            Constant::DoubleBits(0x7ff8000000000042),
        ),
    ] {
        let mut b = Builder::default();
        let v = b.value(t, |v| ManagedInsn::Const {
            result: v,
            value: c,
        });
        b.push(ManagedInsn::Return { value: Some(v) });
        methods += &complete(
            &b.finish(),
            signature(name, j, vec![]),
            ParameterBindings::default(),
        );
    }
    let mut b = Builder::default();
    let v = b.value(ManagedType::Reference(ClassRef::new(0)), |v| {
        ManagedInsn::Const {
            result: v,
            value: Constant::String(StringRef::new(0)),
        }
    });
    b.push(ManagedInsn::Return { value: Some(v) });
    methods += &complete(
        &b.finish(),
        signature("text", JavaType::Class("java.lang.String".into()), vec![]),
        ParameterBindings::default(),
    );
    assert!(methods.contains("Float.intBitsToFloat(0x80000000)"));
    assert!(methods.contains("Double.longBitsToDouble(0x7ff8000000000042L)"));
    // DEX string constants must not introduce fresh String allocations.
    // The runtime check below verifies the exact surrogate and intern identity.
    assert!(!methods.contains("new java.lang.String("));
    java(
        &methods,
        "if(Float.floatToRawIntBits(negativeZero()) != 0x80000000) throw new AssertionError(); if(Double.doubleToRawLongBits(nan()) != 0x7ff8000000000042L) throw new AssertionError(); if(text().length() != 4 || text().charAt(3) != (char)0xd800 || text() != text()) throw new AssertionError();",
    );
}
#[test]
fn unsupported_never_returns_stub() {
    let mut b = Builder::default();
    b.push(ManagedInsn::Opaque {
        family_tag: 12,
        reads: vec![],
        writes: vec![],
        effects: EffectFlags::NONE,
    });
    b.push(ManagedInsn::Return { value: None });
    assert!(matches!(
        emit_method(
            &b.finish(),
            &signature("bad", JavaType::Void, vec![]),
            &ParameterBindings::default(),
            &Resolver
        ),
        SourceResult::Unsupported(Unsupported::Instruction(_))
    ));
    let mut b = Builder::default();
    let v = b.value(ManagedType::Int, |v| ManagedInsn::Phi {
        result: v,
        inputs: vec![],
    });
    b.push(ManagedInsn::Return { value: Some(v) });
    assert!(matches!(
        emit_method(
            &b.finish(),
            &signature("bad", JavaType::Int, vec![]),
            &ParameterBindings::default(),
            &Resolver
        ),
        SourceResult::Unsupported(Unsupported::Instruction(_))
    ));
}
#[test]
fn naming_is_safe_and_unique() {
    assert_eq!(sanitize_identifier("1 bad-name"), "_1_bad_name");
    assert_eq!(sanitize_identifier("_"), "__");
    let mut b = Builder::default();
    let a = b.parameter(ManagedType::Int);
    let c = b.parameter(ManagedType::Int);
    b.push(ManagedInsn::Return { value: Some(a) });
    let source = complete(
        &b.finish(),
        signature("return", JavaType::Int, vec![JavaType::Int, JavaType::Int]),
        ParameterBindings {
            parameters: vec![a, c],
            names: vec!["v0".into(), "v0".into()],
            receiver: None,
        },
    );
    assert!(source.contains("return_(int p_v0, int p_v0_1)"));
}

#[test]
fn synthetic_parameter_in_cfg_and_metadata_failures() {
    let mut b = Builder::default();
    let p = b.value(ManagedType::Unknown, |v| ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![v],
        effects: EffectFlags::NONE,
    });
    b.push(ManagedInsn::Return { value: Some(p) });
    let ir = b.finish();
    let m = signature("identity", JavaType::Int, vec![JavaType::Int]);
    let bindings = ParameterBindings {
        parameters: vec![p],
        names: vec!["Fixture".into()],
        ..Default::default()
    };
    let source = complete(&ir, m.clone(), bindings.clone());
    assert!(source.contains("int Fixture_1"));
    assert!(matches!(
        emit_method(&ir, &m, &ParameterBindings::default(), &Resolver),
        SourceResult::Unsupported(Unsupported::Metadata(_))
    ));
    let wrong = signature("identity", JavaType::Long, vec![JavaType::Int]);
    assert!(matches!(
        emit_method(&ir, &wrong, &bindings, &Resolver),
        SourceResult::Unsupported(Unsupported::Type(_))
    ));
}

#[test]
fn malformed_edges_reject_and_normal_self_loop_emits() {
    for kind in [
        EdgeKind::Exception("java.lang.Exception".into()),
        EdgeKind::BranchTrue,
    ] {
        let mut b = Builder::default();
        let c = b.value(ManagedType::Int, |v| ManagedInsn::Const {
            result: v,
            value: Constant::Int(1),
        });
        b.push(ManagedInsn::Return { value: Some(c) });
        let mut cfg = Cfg::new(b.order[0]);
        let end = cfg.add_block(b.order[1]);
        cfg.add_edge(cfg.entry(), end, kind).unwrap();
        let ir = MethodIr::<SsaForm>::from_parts(cfg, b.insns, b.values);
        assert!(matches!(
            emit_method(
                &ir,
                &signature("bad", JavaType::Int, vec![]),
                &ParameterBindings::default(),
                &Resolver
            ),
            SourceResult::Unsupported(Unsupported::ControlFlow(_))
        ));
    }
    let mut b = Builder::default();
    b.push(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(b.order[0]);
    let edge = cfg
        .add_edge(cfg.entry(), cfg.entry(), EdgeKind::Unconditional)
        .unwrap();
    b.insns[b.order[0]] = ManagedInsn::Branch {
        condition: None,
        then_edge: edge,
        else_edge: None,
    };
    let ir = MethodIr::<SsaForm>::from_parts(cfg, b.insns, b.values);
    let result = emit_method(
        &ir,
        &signature("infinite", JavaType::Void, vec![]),
        &ParameterBindings::default(),
        &Resolver,
    );
    assert!(
        matches!(result, SourceResult::Complete { .. }),
        "{result:?}"
    );
}

#[test]
fn mixed_numeric_width_is_not_silently_promoted() {
    let mut b = Builder::default();
    let a = b.parameter(ManagedType::Int);
    let c = b.parameter(ManagedType::Long);
    let v = b.value(ManagedType::Long, |v| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result: v,
        lhs: a,
        rhs: c,
    });
    b.push(ManagedInsn::Return { value: Some(v) });
    assert!(matches!(
        emit_method(
            &b.finish(),
            &signature("bad", JavaType::Long, vec![JavaType::Int, JavaType::Long]),
            &ParameterBindings {
                parameters: vec![a, c],
                ..Default::default()
            },
            &Resolver
        ),
        SourceResult::Unsupported(Unsupported::Type(_))
    ));
}

#[test]
fn integrated_filled_array_and_contextual_zero_execute() {
    let mut b = Builder::default();
    let input = b.parameter(ManagedType::Int);
    b.push(ManagedInsn::NewFilledArray {
        result: None,
        target: TypeRef::new(1),
        elements: vec![input],
    });
    let array = b.value(
        ManagedType::Array {
            element: Box::new(ManagedType::Int),
            nullable: false,
        },
        |result| ManagedInsn::NewFilledArray {
            result: Some(result),
            target: TypeRef::new(1),
            elements: vec![input, input],
        },
    );
    b.push(ManagedInsn::Return { value: Some(array) });
    let arrays = complete(
        &b.finish(),
        signature(
            "filled",
            JavaType::Array(Box::new(JavaType::Int)),
            vec![JavaType::Int],
        ),
        ParameterBindings {
            parameters: vec![input],
            ..Default::default()
        },
    );
    let mut methods = arrays;
    for (name, managed, constant_value, expected) in [
        (
            "nil",
            ManagedType::Null,
            Constant::Null,
            JavaType::Class("java.lang.String".into()),
        ),
        (
            "zeroRef",
            ManagedType::Zero,
            Constant::Zero,
            JavaType::Class("java.lang.Object".into()),
        ),
        (
            "zeroBool",
            ManagedType::Zero,
            Constant::Zero,
            JavaType::Boolean,
        ),
    ] {
        let mut b = Builder::default();
        let value = b.value(managed, |result| ManagedInsn::Const {
            result,
            value: constant_value,
        });
        b.push(ManagedInsn::Return { value: Some(value) });
        methods.push_str(&complete(
            &b.finish(),
            signature(name, expected, vec![]),
            ParameterBindings::default(),
        ));
    }
    java(
        &methods,
        "int[] a = filled(7); if(a.length != 2 || a[0] != 7 || a[1] != 7 || nil() != null || zeroRef() != null || zeroBool()) throw new AssertionError();",
    );
    let mut b = Builder::default();
    let unknown = b.value(ManagedType::Unknown, |result| ManagedInsn::Const {
        result,
        value: Constant::Zero,
    });
    b.push(ManagedInsn::Return {
        value: Some(unknown),
    });
    assert!(matches!(
        emit_method(
            &b.finish(),
            &signature(
                "unknown",
                JavaType::Class("java.lang.Object".into()),
                vec![]
            ),
            &ParameterBindings::default(),
            &Resolver
        ),
        SourceResult::Unsupported(Unsupported::Type(_))
    ));
}

#[test]
fn integrated_hierarchy_argument_field_return_and_array_store_execute() {
    struct HierarchyResolver(bool);
    impl JavaResolver for HierarchyResolver {
        fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
            let mut h = type_compatibility::Hierarchy::default();
            if self.0 {
                h.insert(type_compatibility::Declaration {
                    name: "java.lang.String".into(),
                    superclass: Some("java.lang.Object".into()),
                    interfaces: vec!["java.lang.CharSequence".into()],
                    is_interface: false,
                })
                .unwrap();
            }
            h.assignable(actual, expected) == type_compatibility::Compatibility::Assignable
        }
        fn instruction_type(&self, _: TypeRef) -> Option<JavaType> {
            Some(JavaType::Array(Box::new(JavaType::Class(
                "java.lang.CharSequence".into(),
            ))))
        }
        fn method(&self, _: MethodRef) -> Option<JavaMethod> {
            Some(signature(
                "accept",
                JavaType::Void,
                vec![JavaType::Class("java.lang.CharSequence".into())],
            ))
        }
        fn field(&self, _: FieldRef) -> Option<JavaField> {
            Some(JavaField {
                owner: "Fixture".into(),
                name: "sequence".into(),
                ty: JavaType::Class("java.lang.CharSequence".into()),
                is_static: true,
            })
        }
        fn class(&self, reference: ClassRef) -> Option<String> {
            Some(
                if reference.index() == 0 {
                    "java.lang.String"
                } else {
                    "java.lang.CharSequence"
                }
                .into(),
            )
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
    }
    let mut b = Builder::default();
    let text = b.parameter(ManagedType::Reference(ClassRef::new(0)));
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(0),
        args: vec![text],
    });
    b.push(ManagedInsn::FieldPut {
        field: FieldRef::new(0),
        object: None,
        value: text,
    });
    let null = b.value(ManagedType::Null, |result| ManagedInsn::Const {
        result,
        value: Constant::Null,
    });
    let index = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(0),
    });
    let array = b.value(
        ManagedType::Array {
            element: Box::new(ManagedType::Reference(ClassRef::new(1))),
            nullable: false,
        },
        |result| ManagedInsn::NewFilledArray {
            result: Some(result),
            target: TypeRef::new(0),
            elements: vec![text, null],
        },
    );
    b.push(ManagedInsn::ArrayPut {
        array,
        index,
        value: text,
    });
    b.push(ManagedInsn::Return { value: Some(text) });
    let ir = b.finish();
    let method = signature(
        "upcast",
        JavaType::Class("java.lang.CharSequence".into()),
        vec![JavaType::Class("java.lang.String".into())],
    );
    let bindings = ParameterBindings {
        parameters: vec![text],
        ..Default::default()
    };
    let SourceResult::Complete { source, .. } =
        emit_method(&ir, &method, &bindings, &HierarchyResolver(true))
    else {
        panic!("proven upcast rejected")
    };
    java(
        &format!(
            "static CharSequence sequence; static void accept(CharSequence s) {{ if(s == null) throw new AssertionError(); }} static void accept(String s) {{ throw new AssertionError(\"wrong overload\"); }} {source}"
        ),
        "if(!upcast(\"ok\").toString().equals(\"ok\") || !sequence.toString().equals(\"ok\")) throw new AssertionError();",
    );
    assert!(matches!(
        emit_method(&ir, &method, &bindings, &HierarchyResolver(false)),
        SourceResult::Unsupported(Unsupported::Type(_))
    ));
}

#[path = "interface_super_tests.rs"]
mod interface_super_tests;
#[path = "pure_expression_tests.rs"]
mod pure_expression_tests;
