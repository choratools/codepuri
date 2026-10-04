//! Effect traces and capture availability for general object argument producers.
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
        for id in &self.order[1..] {
            let next = cfg.add_block(*id);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}
struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            match r.index() {
                0 => "Fixture.Outer",
                1 => "Fixture.Inner",
                2 => "Fixture.Holder",
                3 => "Fixture.ArrayBox",
                4 => "java.lang.Object",
                _ => return None,
            }
            .into(),
        )
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture.Holder".into(),
            name: "CURRENT".into(),
            ty: JavaType::Class("Fixture.Holder".into()),
            is_static: true,
        })
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn instruction_type(&self, r: TypeRef) -> Option<JavaType> {
        (r.index() == 0)
            .then(|| JavaType::Array(Box::new(JavaType::Class("java.lang.Object".into()))))
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let (owner, name, is_static, parameters, return_type) = match r.index() {
            0 => (
                "Fixture.Outer",
                "<init>",
                false,
                vec![JavaType::Boolean],
                JavaType::Void,
            ),
            1 => ("Fixture.Holder", "flag", false, vec![], JavaType::Boolean),
            2 => (
                "Fixture",
                "both",
                true,
                vec![JavaType::Boolean, JavaType::Boolean],
                JavaType::Boolean,
            ),
            3 => (
                "Fixture.Outer",
                "<init>",
                false,
                vec![
                    JavaType::Int,
                    JavaType::Class("Fixture.Inner".into()),
                    JavaType::Int,
                ],
                JavaType::Void,
            ),
            4 => ("Fixture.Inner", "<init>", false, vec![], JavaType::Void),
            5 => (
                "Fixture",
                "consume",
                true,
                vec![JavaType::Class("Fixture.Holder".into())],
                JavaType::Void,
            ),
            8 => ("Fixture", "number", true, vec![], JavaType::Int),
            9 => (
                "Fixture.ArrayBox",
                "<init>",
                false,
                vec![
                    JavaType::Array(Box::new(JavaType::Class("java.lang.Object".into()))),
                    JavaType::Int,
                ],
                JavaType::Void,
            ),
            10 => (
                "Fixture.ArrayBox",
                "<init>",
                false,
                vec![
                    JavaType::Array(Box::new(JavaType::Class("java.lang.Object".into()))),
                    JavaType::Boolean,
                ],
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
fn allocation(b: &mut Builder, class: u32) -> ValueId {
    b.value(ManagedType::Reference(ClassRef::new(class)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(class),
        }
    })
}
fn invoke(b: &mut Builder, method: u32, args: Vec<ValueId>) -> InsnId {
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(method),
        args,
    })
}
fn emit(ir: &MethodIr<SsaForm>, return_type: JavaType) -> SourceResult {
    emit_method(
        ir,
        &JavaMethod {
            owner: "Fixture".into(),
            name: "construct".into(),
            is_static: true,
            parameters: vec![],
            return_type,
            throws: vec![],
        },
        &ParameterBindings::default(),
        &Resolver,
    )
}
fn captured_field(mode: u8) -> (MethodIr<SsaForm>, InsnId, InsnId, ValueId) {
    let mut b = Builder::default();
    let outer = allocation(&mut b, 0);
    let root = *b.order.last().unwrap();
    let holder = b.value(ManagedType::Reference(ClassRef::new(2)), |result| {
        ManagedInsn::FieldGet {
            result,
            object: None,
            field: FieldRef::new(0),
        }
    });
    if mode == 3 {
        // A consumer outside the proven CFG cannot inherit capture availability.
        b.insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(5),
            args: vec![holder],
        });
    }
    let flag = b.value(ManagedType::Boolean, |result| ManagedInsn::Invoke {
        kind: InvokeKind::Virtual,
        result: Some(result),
        method: MethodRef::new(1),
        args: vec![holder],
    });
    if mode == 1 {
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(5),
            args: vec![holder],
        });
    }
    let argument = if mode == 2 {
        b.value(ManagedType::Boolean, |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(2),
            args: vec![flag, flag],
        })
    } else {
        flag
    };
    let initializer = invoke(&mut b, 0, vec![outer, argument]);
    let later = b.value(ManagedType::Boolean, |result| ManagedInsn::Invoke {
        kind: InvokeKind::Virtual,
        result: Some(result),
        method: MethodRef::new(1),
        args: vec![holder],
    });
    b.push(ManagedInsn::Return { value: Some(later) });
    (b.finish(), root, initializer, holder)
}
fn java(label: &str, source: &str, declarations: &str, main: &str) {
    let dir = std::env::temp_dir().join(format!("object-producer-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"),format!("public class Fixture {{static String log=\"\";static int mode;{declarations}{source}public static void main(String[]args){{mode=Integer.parseInt(args[0]);{main}}}}}")).unwrap();
    let home = std::env::var_os("JAVA_HOME").map(std::path::PathBuf::from);
    let executable = |tool: &str| {
        home.as_ref()
            .map(|h| h.join("bin").join(tool))
            .unwrap_or_else(|| tool.into())
    };
    let output = std::process::Command::new(executable("javac"))
        .args(["--release", "17", "Fixture.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    for mode in ["0", "1", "2", "3"] {
        let output = std::process::Command::new(executable("java"))
            .args(["-cp", ".", "Fixture", mode])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "mode={mode}: {}\n{source}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn static_field_virtual_argument_capture_preserves_single_read_and_later_identity() {
    // Exact producer shape in original Li1/l0 constructor 44239: new10,
    // static field12, virtual flag14, init18, and another receiver use55.
    let (ir, _, _, _) = captured_field(0);
    let result = emit(&ir, JavaType::Boolean);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert_eq!(
        source.matches("Fixture.Holder.CURRENT").count(),
        1,
        "{source}"
    );
    assert!(
        source
            .lines()
            .any(|s| s.contains("new Fixture.Outer(") && s.contains(" = Fixture.Holder.CURRENT")),
        "{source}"
    );
    java(
        "capture",
        &source,
        "static class Holder {static {log+=\"H\";}static volatile Holder CURRENT=new Holder(true); final boolean value;Holder(boolean v){value=v;}boolean flag(){log+=\"F\";if(mode==1)throw new IllegalArgumentException();return value;}} static class Outer {static {log+=\"O\";if(mode==3)throw new OutOfMemoryError();}Outer(boolean v){log+=\"C\";if(mode==2)throw new IllegalStateException();if(!v)throw new AssertionError();Holder.CURRENT=new Holder(false);}}",
        "if(mode==0){if(!construct())throw new AssertionError(\"capture lost\");if(!log.equals(\"OHFCF\"))throw new AssertionError(log);}else{try{construct();throw new AssertionError();}catch(IllegalArgumentException|IllegalStateException|OutOfMemoryError expected){}String expected=mode==1?\"OHF\":mode==2?\"OHFC\":\"O\";if(!log.equals(expected))throw new AssertionError(log);}",
    );
}

#[test]
fn capture_reuses_a_reserved_slot_and_publishes_only_at_initializer() {
    let (ir, _, _, holder) = captured_field(0);
    let mut emitter = Emitter {
        ir: &ir,
        resolver: &Resolver,
        values: HashMap::from([(
            holder,
            ("saved".into(), JavaType::Class("Fixture.Holder".into())),
        )]),
        lines: vec!["Fixture.Holder saved;".into()],
        next: 0,
        parameters: HashSet::new(),
        reserved: HashSet::from(["saved".into()]),
        current: None,
        hoisted: HashSet::from([holder]),
        declarations: HashMap::new(),
        allocation_readable: None,
        object_planning: Default::default(),
    };
    let signature = JavaMethod {
        owner: "Fixture".into(),
        name: "construct".into(),
        is_static: true,
        parameters: Vec::new(),
        return_type: JavaType::Boolean,
        throws: Vec::new(),
    };
    let mut initialized = false;
    for (id, instruction) in ir.instructions() {
        let initializer = matches!(
            instruction,
            ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                ..
            }
        );
        if !initialized {
            assert!(emitter.hoisted.contains(&holder));
        }
        emitter.current = Some(id);
        emitter.instruction(instruction, &signature).unwrap();
        if initializer {
            initialized = true;
        }
        if initialized {
            assert!(!emitter.hoisted.contains(&holder));
        }
    }
    assert!(initialized);
    assert_eq!(emitter.get(holder).unwrap().0, "saved");
    let source = emitter.lines.join("\n");
    assert_eq!(
        source.matches("Fixture.Holder.CURRENT").count(),
        1,
        "{source}"
    );
    assert!(
        source.contains("saved = Fixture.Holder.CURRENT"),
        "{source}"
    );
}

#[test]
fn zero_between_original_array_and_initializer_is_captured_for_later_initializer() {
    // Original Li1/l0 ctor44239's separate failing segment: new30,
    // const length32, array34, zero36, init37; new46,array48,init50 reuse zero36.
    let mut b = Builder::default();
    let first = allocation(&mut b, 3);
    let length = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(16),
    });
    let array = b.value(
        ManagedType::Array {
            element: Box::new(ManagedType::Reference(ClassRef::new(4))),
            nullable: true,
        },
        |result| ManagedInsn::NewArray {
            result,
            element: TypeRef::new(0),
            length,
        },
    );
    let zero = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Zero,
    });
    invoke(&mut b, 9, vec![first, array, zero]);
    let second = allocation(&mut b, 3);
    let array = b.value(
        ManagedType::Array {
            element: Box::new(ManagedType::Reference(ClassRef::new(4))),
            nullable: true,
        },
        |result| ManagedInsn::NewArray {
            result,
            element: TypeRef::new(0),
            length,
        },
    );
    invoke(&mut b, 9, vec![second, array, zero]);
    b.push(ManagedInsn::Return { value: Some(zero) });
    let ir = b.finish();
    let result = emit(&ir, JavaType::Int);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert_eq!(
        source.matches("new Fixture.ArrayBox(").count(),
        2,
        "{source}"
    );
    assert!(
        source
            .lines()
            .any(|line| line.contains("new Fixture.ArrayBox(") && line.contains(" = 0")),
        "{source}"
    );
    java(
        "shared-zero-array",
        &source,
        "static class ArrayBox {static {log+=\"O\";if(mode==3)throw new OutOfMemoryError();}ArrayBox(Object[] a,int z){log+=\"C\";if(z!=0||a.length!=16)throw new AssertionError();if(mode==1)throw new IllegalArgumentException();if(mode==2)throw new IllegalStateException();}}",
        "if(mode==0){if(construct()!=0||!log.equals(\"OCC\"))throw new AssertionError(log);}else{try{construct();throw new AssertionError();}catch(IllegalArgumentException|IllegalStateException|OutOfMemoryError expected){}if(!log.equals(mode==3?\"O\":\"OC\"))throw new AssertionError(log);}",
    );
}

#[test]
fn outside_literal_capture_rejects_unavailable_uses_and_resolving_literals() {
    for mode in [0, 1, 2, 3, 4, 5] {
        let mut b = Builder::default();
        let object = allocation(&mut b, 3);
        let length = b.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(16),
        });
        let array = b.value(
            ManagedType::Array {
                element: Box::new(ManagedType::Reference(ClassRef::new(4))),
                nullable: true,
            },
            |result| ManagedInsn::NewArray {
                result,
                element: TypeRef::new(0),
                length,
            },
        );
        let literal = b.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: match mode {
                0 => Constant::Zero,
                1 => Constant::FloatBits(0),
                2 => Constant::DoubleBits(0),
                3 => Constant::String(StringRef::new(0)),
                4 => Constant::ClassLiteral(TypeRef::new(0)),
                _ => Constant::Zero,
            },
        });
        if mode == 0 {
            // Literal identity does not authorize a use with no CFG path.
            b.insns.insert(ManagedInsn::Return {
                value: Some(literal),
            });
        }
        // Mixed raw-zero context would replace its capture assignment with
        // `false` at this first use and leave a later int slot unassigned.
        invoke(
            &mut b,
            if mode == 5 { 10 } else { 9 },
            vec![object, array, literal],
        );
        b.push(ManagedInsn::Return {
            value: Some(literal),
        });
        let ir = b.finish();
        let emitter = Emitter {
            ir: &ir,
            resolver: &Resolver,
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
        let root = *ir.cfg().block(ir.cfg().entry()).unwrap().payload();
        assert!(emitter.argument_object_plan(root).is_err(), "mode={mode}");
    }
}

#[test]
fn sequenced_shared_producer_keeps_unavailable_and_initializer_only_scope_denials() {
    for mode in [1, 2, 3] {
        let (ir, _, _, _) = captured_field(mode);
        let result = emit(&ir, JavaType::Boolean);
        if mode == 3 {
            assert!(
                matches!(result, SourceResult::Unsupported(_)),
                "mode={mode}"
            );
        } else {
            let SourceResult::Complete { source, .. } = result else {
                panic!("mode{mode}:{result:?}")
            };
            crate::object_plan::sequence_tests::compile_fixture(
                &source,
                "static class Outer { Outer(boolean b) {} } static class Holder { static Holder CURRENT; boolean flag() { return true; } } static void consume(Holder h) {} static boolean both(boolean a,boolean b) { return a&&b; }",
            );
        }
    }
    let (mut ir, root, initializer, _) = captured_field(0);
    let mut cfg = Cfg::new(root);
    let mut last = cfg.entry();
    let mut initializer_block = None;
    for (id, _) in ir.instructions().iter().filter(|(id, _)| *id != root) {
        let next = cfg.add_block(id);
        cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
        if id == initializer {
            initializer_block = Some(next);
        }
        last = next;
    }
    cfg.add_edge(
        initializer_block.unwrap(),
        cfg.entry(),
        EdgeKind::Exception("handler".into()),
    )
    .unwrap();
    ir = MethodIr::<SsaForm>::from_parts(cfg, ir.instructions().clone(), ir.values().clone());
    assert!(matches!(
        emit(&ir, JavaType::Boolean),
        SourceResult::Unsupported(_)
    ));
}

#[test]
fn nested_argument_allows_only_nonthrowing_literal_position_changes() {
    // Original Ln5/h constructor 50711: outer44, inner46/init48,
    // first outer int argument51, outer init52.
    let mut b = Builder::default();
    let outer = allocation(&mut b, 0);
    let inner = allocation(&mut b, 1);
    invoke(&mut b, 4, vec![inner]);
    let one = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    invoke(&mut b, 3, vec![outer, one, inner, one]);
    b.push(ManagedInsn::Return { value: Some(outer) });
    let ir = b.finish();
    // The statement route materializes one literal and captures both original uses.
    let SourceResult::Complete { source, .. } = emit(&ir, JavaType::Class("Fixture.Outer".into()))
    else {
        panic!("shared literal sequence must emit");
    };
    java(
        "shared_literal_sequence",
        &source,
        "static class Inner{static {log+=\"I\";}Inner(){log+=\"i\";}}static class Outer{static{log+=\"O\";}Outer(int n,Inner inner,int last){log+=\"C\";if(n!=1||last!=1)throw new AssertionError();}}",
        "construct();if(!log.equals(\"OIiC\"))throw new AssertionError(log);",
    );
    for effectful in [false, true] {
        let mut b = Builder::default();
        let last = b.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(0),
        });
        let outer = allocation(&mut b, 0);
        let inner = allocation(&mut b, 1);
        invoke(&mut b, 4, vec![inner]);
        let first = b.value(ManagedType::Int, |result| {
            if effectful {
                ManagedInsn::Invoke {
                    kind: InvokeKind::Static,
                    result: Some(result),
                    method: MethodRef::new(8),
                    args: vec![],
                }
            } else {
                ManagedInsn::Const {
                    result,
                    value: Constant::Int(1),
                }
            }
        });
        invoke(&mut b, 3, vec![outer, first, inner, last]);
        b.push(ManagedInsn::Return { value: Some(outer) });
        let result = emit(&b.finish(), JavaType::Class("Fixture.Outer".into()));
        if effectful {
            let SourceResult::Complete { source, .. } = result else {
                panic!("effectful literal sequence must emit");
            };
            java(
                "effectful_literal_sequence",
                &source,
                "static class Inner{static {log+=\"I\";}Inner(){log+=\"i\";}}static class Outer{static{log+=\"O\";}Outer(int n,Inner inner,int last){log+=\"C\";if(n!=1||last!=0)throw new AssertionError();}}static int number(){log+=\"T\";return 1;}",
                "construct();if(!log.equals(\"OIiTC\"))throw new AssertionError(log);",
            );
            continue;
        }
        let SourceResult::Complete { source, .. } = result else {
            panic!("{result:?}")
        };
        java(
            "literal",
            &source,
            "static class Inner{static {log+=\"I\";}Inner(){log+=\"i\";}}static class Outer{static{log+=\"O\";}Outer(int n,Inner inner,int last){log+=\"C\";if(n!=1||last!=0)throw new AssertionError();}}",
            "construct();if(!log.equals(\"OIiC\"))throw new AssertionError(log);",
        );
    }
}
