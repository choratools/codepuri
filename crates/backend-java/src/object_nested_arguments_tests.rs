//! Nested allocations keep the complete original argument event trace.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
};

#[derive(Default)]
struct Builder {
    insns: InsnArena,
    values: ValueArena,
    order: Vec<InsnId>,
}
impl Builder {
    fn value(&mut self, ty: ManagedType, f: impl FnOnce(ValueId) -> ManagedInsn) -> ValueId {
        let mut value = None;
        let id = self.insns.insert_with_key(|id| {
            let result = self.values.insert(Value::new(id, ty));
            value = Some(result);
            f(result)
        });
        self.order.push(id);
        value.unwrap()
    }
    fn push(&mut self, insn: ManagedInsn) -> InsnId {
        let id = self.insns.insert(insn);
        self.order.push(id);
        id
    }
    fn finish(self) -> MethodIr<SsaForm> {
        let mut cfg = Cfg::new(self.order[0]);
        let mut last = cfg.entry();
        for insn in self.order.iter().skip(1) {
            let next = cfg.add_block(*insn);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}
struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, c: ClassRef) -> Option<String> {
        Some(
            match c.index() {
                0 => "Fixture.Outer",
                1 => "Fixture.Inner",
                2 => "java.lang.String",
                _ => return None,
            }
            .into(),
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        Some("message".encode_utf16().collect())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        let (owner, name, is_static, parameters, return_type) = match m.index() {
            0 => (
                "Fixture.Outer",
                "<init>",
                false,
                vec![JavaType::Class("Fixture.Inner".into())],
                JavaType::Void,
            ),
            1 => (
                "Fixture.Inner",
                "<init>",
                false,
                vec![JavaType::Class("java.lang.String".into())],
                JavaType::Void,
            ),
            2 | 3 => (
                "Fixture",
                if m.index() == 2 { "first" } else { "second" },
                true,
                vec![],
                JavaType::Class("java.lang.String".into()),
            ),
            4 => (
                "Fixture.Inner",
                "<init>",
                false,
                vec![
                    JavaType::Class("java.lang.String".into()),
                    JavaType::Class("java.lang.String".into()),
                ],
                JavaType::Void,
            ),
            5 => (
                "Fixture.Inner",
                "<init>",
                false,
                vec![JavaType::Class("Fixture.Inner".into())],
                JavaType::Void,
            ),
            _ => return None,
        };
        Some(JavaMethod {
            owner: owner.into(),
            name: name.into(),
            is_static,
            parameters,
            return_type,
            throws: vec![],
        })
    }
}
fn new(b: &mut Builder, class: u32) -> ValueId {
    b.value(ManagedType::Reference(ClassRef::new(class)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(class),
        }
    })
}
fn string(b: &mut Builder) -> ValueId {
    b.value(ManagedType::Reference(ClassRef::new(2)), |result| {
        ManagedInsn::Const {
            result,
            value: Constant::String(StringRef::new(0)),
        }
    })
}
fn init(b: &mut Builder, method: u32, args: Vec<ValueId>) {
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(method),
        args,
    });
}
fn emit(b: Builder) -> SourceResult {
    emit_method(
        &b.finish(),
        &JavaMethod {
            owner: "Fixture".into(),
            name: "construct".into(),
            is_static: true,
            parameters: vec![],
            return_type: JavaType::Class("Fixture.Outer".into()),
            throws: vec![],
        },
        &ParameterBindings::default(),
        &Resolver,
    )
}
fn fixture(mode: u8) -> Builder {
    let mut b = Builder::default();
    let outer = new(&mut b, 0);
    let early = (mode == 1).then(|| string(&mut b));
    let inner = new(&mut b, 1);
    if mode == 2 {
        // A producer may never receive an uninitialized inner reference.
        b.value(ManagedType::Reference(ClassRef::new(1)), |result| {
            ManagedInsn::Move {
                result,
                value: inner,
            }
        });
    }
    let value = early.unwrap_or_else(|| string(&mut b));
    init(&mut b, 1, vec![inner, value]);
    init(&mut b, 0, vec![outer, inner]);
    b.push(ManagedInsn::Return { value: Some(outer) });
    b
}
#[test]
fn nested_string_argument_preserves_new_new_string_init_init() {
    let result = emit(fixture(0));
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert_eq!(source.matches("new Fixture.Outer(").count(), 1, "{source}");
    assert_eq!(source.matches("new Fixture.Inner(").count(), 1, "{source}");
    assert_eq!(source.matches("\"message\"").count(), 1, "{source}");
    runtime(&source, false);
}
#[test]
fn resolving_string_before_inner_allocation_and_uninitialized_alias_reject() {
    let SourceResult::Complete { source, .. } = emit(fixture(1)) else {
        panic!("ordered nested sequence must emit");
    };
    runtime(&source, true);
    assert!(source.contains("switch (0)"));
    assert!(source.find("\"message\"").unwrap() < source.find("new Fixture.Inner(").unwrap());
    assert!(!matches!(emit(fixture(2)), SourceResult::Complete { .. }));
}
#[test]
fn effectful_nested_arguments_require_original_left_to_right_order() {
    for reversed in [false, true] {
        let mut b = Builder::default();
        let outer = new(&mut b, 0);
        let inner = new(&mut b, 1);
        let first = b.value(ManagedType::Reference(ClassRef::new(2)), |result| {
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(result),
                method: MethodRef::new(2),
                args: vec![],
            }
        });
        let second = b.value(ManagedType::Reference(ClassRef::new(2)), |result| {
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(result),
                method: MethodRef::new(3),
                args: vec![],
            }
        });
        init(
            &mut b,
            4,
            if reversed {
                vec![inner, second, first]
            } else {
                vec![inner, first, second]
            },
        );
        init(&mut b, 0, vec![outer, inner]);
        b.push(ManagedInsn::Return { value: Some(outer) });
        assert_eq!(matches!(emit(b), SourceResult::Complete { .. }), !reversed);
    }
}
#[test]
fn multiple_nested_allocations_close_in_stack_order() {
    for reordered in [false, true] {
        let mut b = Builder::default();
        let outer = new(&mut b, 0);
        let inner = new(&mut b, 1);
        let deepest = new(&mut b, 1);
        let value = string(&mut b);
        if reordered {
            init(&mut b, 1, vec![inner, value]);
            init(&mut b, 1, vec![deepest, value]);
        } else {
            init(&mut b, 1, vec![deepest, value]);
            init(&mut b, 5, vec![inner, deepest]);
        }
        init(&mut b, 0, vec![outer, inner]);
        b.push(ManagedInsn::Return { value: Some(outer) });
        let result = emit(b);
        assert_eq!(
            matches!(result, SourceResult::Complete { .. }),
            !reordered,
            "{result:?}"
        );
    }
}
#[test]
fn inner_initializer_exception_range_cannot_absorb_prior_allocations() {
    let mut b = fixture(0);
    let initializer = b.order[3];
    let handler = b.insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(b.order[0]);
    let mut last = cfg.entry();
    let mut protected = None;
    for id in b.order.iter().skip(1) {
        let next = cfg.add_block(*id);
        cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
        if *id == initializer {
            protected = Some(next);
        }
        last = next;
    }
    let catch = cfg.add_block(handler);
    cfg.add_edge(
        protected.unwrap(),
        catch,
        EdgeKind::Exception("java.lang.RuntimeException".into()),
    )
    .unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, b.insns, b.values);
    let resolver = Resolver;
    let emitter = Emitter {
        ir: &ir,
        resolver: &resolver,
        values: HashMap::new(),
        lines: vec![],
        next: 0,
        parameters: HashSet::new(),
        reserved: HashSet::new(),
        current: None,
        hoisted: HashSet::new(),
        declarations: HashMap::new(),
        allocation_readable: None,
        object_planning: Default::default(),
    };
    assert!(emitter.argument_object_plan(b.order[0]).is_err());
}
#[test]
fn nested_producer_capture_is_evaluated_once_and_later_reads_slot() {
    let mut b = Builder::default();
    let outer = new(&mut b, 0);
    let inner = new(&mut b, 1);
    let value = b.value(ManagedType::Reference(ClassRef::new(2)), |result| {
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(2),
            args: vec![],
        }
    });
    init(&mut b, 1, vec![inner, value]);
    init(&mut b, 0, vec![outer, inner]);
    b.push(ManagedInsn::Return { value: Some(value) });
    let result = emit_method(
        &b.finish(),
        &JavaMethod {
            owner: "Fixture".into(),
            name: "capture".into(),
            is_static: true,
            parameters: vec![],
            return_type: JavaType::Class("java.lang.String".into()),
            throws: vec![],
        },
        &ParameterBindings::default(),
        &Resolver,
    );
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert_eq!(source.matches("Fixture.first(").count(), 1, "{source}");
    assert!(
        source
            .lines()
            .any(|line| line.contains("new Fixture.Inner(") && line.contains(" = ")),
        "{source}"
    );
}
fn runtime(source: &str, early: bool) {
    let root = std::env::temp_dir().join(format!(
        "decomphify-nested-args-{}-{early}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Fixture.java"),format!(r#"public class Fixture {{
    static int mode;static String log="";
    static class Outer {{static {{log+="O";if(mode==1)throw new OutOfMemoryError();}} Inner value;Outer(Inner value){{log+="C";this.value=value;}}}}
    static class Inner {{static {{log+="I";if(mode==2)throw new OutOfMemoryError();}}String value;Inner(String value){{log+="N";if(mode==3)throw new IllegalArgumentException();this.value=value;}}}}
    {source}
    public static void main(String[]args){{mode=Integer.parseInt(args[0]);try{{Outer o=construct();if(mode!=0||!o.value.value.equals("message")||!log.equals("OINC"))throw new AssertionError(log);}}catch(OutOfMemoryError e){{if(!(mode==1&&log.equals("O")||mode==2&&log.equals("OI")))throw new AssertionError(log);}}catch(IllegalArgumentException e){{if(mode!=3||!log.equals("OIN"))throw new AssertionError(log);}}}}
    }}"#)).unwrap();
    let home = std::env::var_os("JAVA_HOME").map(std::path::PathBuf::from);
    let tool = |name: &str| {
        home.as_ref()
            .map(|h| h.join("bin").join(name))
            .unwrap_or_else(|| name.into())
    };
    let out = std::process::Command::new(tool("javac"))
        .args(["--release", "17", "Fixture.java"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytecode = std::process::Command::new(tool("javap"))
        .args(["-c", "-p", "Fixture"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(bytecode.status.success());
    let bytecode = String::from_utf8(bytecode.stdout).unwrap();
    let construct = bytecode
        .split(" construct();")
        .nth(1)
        .unwrap()
        .split("public static void main")
        .next()
        .unwrap();
    let events = construct
        .lines()
        .filter_map(|line| {
            if line.contains(" new ") && line.contains("Fixture$Outer") {
                Some("outer-new")
            } else if line.contains(" new ") && line.contains("Fixture$Inner") {
                Some("inner-new")
            } else if line.contains(" ldc ") && line.contains("message") {
                Some("string")
            } else if line.contains("invokespecial") && line.contains("Method Fixture$Inner.") {
                Some("inner-init")
            } else if line.contains("invokespecial") && line.contains("Method Fixture$Outer.") {
                Some("outer-init")
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        events,
        if early {
            [
                "outer-new",
                "string",
                "inner-new",
                "inner-init",
                "outer-init",
            ]
        } else {
            [
                "outer-new",
                "inner-new",
                "string",
                "inner-init",
                "outer-init",
            ]
        },
        "{construct}"
    );
    for mode in ["0", "1", "2", "3"] {
        let out = std::process::Command::new(tool("java"))
            .args(["-Xverify:all", "Fixture", mode])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "mode={mode}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
