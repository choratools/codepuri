#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
};

struct Resolver;
struct WideResolver;
impl JavaResolver for WideResolver {
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Fixture.Box".into())
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "Fixture.Box".into(),
            name: "<init>".into(),
            is_static: false,
            return_type: JavaType::Void,
            parameters: vec![JavaType::Long, JavaType::Int],
            throws: vec![],
        })
    }
}

#[test]
fn constructor_words_preserve_wide_identity_and_following_argument() {
    for invalid in [false, true] {
        let mut b = Builder::default();
        let long = b.parameter(ManagedType::Long);
        let high = if invalid {
            b.parameter(ManagedType::Long)
        } else {
            long
        };
        let int = b.parameter(ManagedType::Int);
        let object = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
            ManagedInsn::NewInstance {
                result,
                class: ClassRef::new(0),
            }
        });
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(0),
            args: vec![object, long, high, int],
        });
        b.push(ManagedInsn::Return {
            value: Some(object),
        });
        let ir = b.finish();
        let result = emit_method(
            &ir,
            &signature(
                "make",
                JavaType::Class("Fixture.Box".into()),
                if invalid {
                    vec![JavaType::Long, JavaType::Long, JavaType::Int]
                } else {
                    vec![JavaType::Long, JavaType::Int]
                },
            ),
            &ParameterBindings {
                parameters: if invalid {
                    vec![long, high, int]
                } else {
                    vec![long, int]
                },
                ..Default::default()
            },
            &WideResolver,
        );
        if invalid {
            assert!(matches!(result, SourceResult::Unsupported(_)));
        } else {
            let SourceResult::Complete { source, .. } = result else {
                panic!("{result:?}")
            };
            assert!(source.contains("(long) p0, (int) p1"), "{source}");
            java(
                &format!(
                    "static class Box {{ final long x; final int y; Box(long x,int y) {{ this.x=x;this.y=y; }} }} {source}"
                ),
                "Box b=make(4294967297L,17); if(b.x!=4294967297L || b.y!=17) throw new AssertionError();",
            );
        }
    }
}
impl JavaResolver for Resolver {
    fn discarded_invoke_source_target(&self, r: MethodRef) -> bool {
        // The executable fixture declares exactly static int tick(int).
        r.index() == 0
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("java.lang.String".into())
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        if r.index() == 3 {
            return Some(JavaMethod {
                owner: "java.lang.StringBuilder".into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: vec![],
                throws: vec![],
            });
        }
        Some(if r.index() == 1 || r.index() == 2 {
            JavaMethod {
                owner: "java.lang.String".into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: if r.index() == 2 {
                    vec![
                        JavaType::Array(Box::new(JavaType::Char)),
                        JavaType::Int,
                        JavaType::Int,
                    ]
                } else {
                    vec![JavaType::Array(Box::new(JavaType::Char))]
                },
                throws: vec![],
            }
        } else {
            signature("tick", JavaType::Int, vec![JavaType::Int])
        })
    }
}

#[test]
fn reserved_object_slot_cannot_be_read_before_its_unique_initializer() {
    let mut builder = Builder::default();
    let chars = builder.parameter(ManagedType::Array {
        element: Box::new(ManagedType::Char),
        nullable: true,
    });
    let object = builder.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    builder.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![object, chars],
    });
    let alias = builder.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::Move {
            result,
            value: object,
        }
    });
    let ir = builder.finish();
    let allocation = ir.value(object).unwrap().def();
    let initializer = ir
        .instructions()
        .iter()
        .find_map(|(id, instruction)| {
            matches!(instruction, ManagedInsn::Invoke { .. }).then_some(id)
        })
        .unwrap();
    let mut emitter = Emitter {
        allocation_readable: None,
        object_planning: Default::default(),
        ir: &ir,
        resolver: &Resolver,
        values: HashMap::from([
            (
                chars,
                ("p0".into(), JavaType::Array(Box::new(JavaType::Char))),
            ),
            (
                object,
                (
                    "reserved".into(),
                    JavaType::Class("java.lang.String".into()),
                ),
            ),
        ]),
        lines: vec![],
        next: 0,
        parameters: HashSet::from([chars]),
        reserved: HashSet::new(),
        current: Some(allocation),
        hoisted: HashSet::from([object]),
        declarations: HashMap::new(),
    };
    assert!(emitter.get(object).is_err());
    let method = signature("make", JavaType::Class("java.lang.String".into()), vec![]);
    emitter
        .instruction(ir.instruction(allocation).unwrap(), &method)
        .unwrap();
    assert!(emitter.get(object).is_err());
    emitter.current = Some(initializer);
    emitter
        .instruction(ir.instruction(initializer).unwrap(), &method)
        .unwrap();
    assert_eq!(emitter.get(object).unwrap().0, "reserved");
    assert_eq!(emitter.lines.len(), 1);
    assert!(emitter.lines[0].starts_with("reserved = new java.lang.String("));
    emitter.values.insert(
        alias,
        ("copy".into(), JavaType::Class("java.lang.String".into())),
    );
    emitter.hoisted.insert(alias);
    assert!(emitter.get(alias).is_err());
    let copy = ir.value(alias).unwrap().def();
    emitter.current = Some(copy);
    emitter
        .instruction(ir.instruction(copy).unwrap(), &method)
        .unwrap();
    assert_eq!(emitter.get(alias).unwrap().0, "copy");
    emitter.current = Some(initializer);
    assert!(
        emitter
            .instruction(ir.instruction(initializer).unwrap(), &method)
            .is_err()
    );
}
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
            let v = self.values.insert(Value::new(id, ty));
            value = Some(v);
            f(v)
        });
        self.order.push(id);
        value.unwrap()
    }
    fn parameter(&mut self, ty: ManagedType) -> ValueId {
        let v = self.value(ty, |v| ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![v],
            effects: managed_ir::effects::EffectFlags::NONE,
        });
        self.order.pop();
        v
    }
    fn push(&mut self, insn: ManagedInsn) {
        self.order.push(self.insns.insert(insn));
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
fn signature(name: &str, return_type: JavaType, parameters: Vec<JavaType>) -> JavaMethod {
    JavaMethod {
        owner: "Fixture".into(),
        name: name.into(),
        is_static: true,
        return_type,
        parameters,
        throws: vec![],
    }
}
fn java(method: &str, assertions: &str) {
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let dir = std::env::temp_dir().join(format!(
        "object-plan-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), format!("public class Fixture {{ static int effects; static int tick(int n) {{ effects++; return n; }} {method} public static void main(String[] args) {{ {assertions} }} }}")).unwrap();
    for (tool, args) in [
        ("javac", vec!["--release", "17", "Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let output = Command::new(format!("{home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

fn fixture(barrier: u8) -> (MethodIr<SsaForm>, ValueId) {
    let mut b = Builder::default();
    let chars = b.parameter(ManagedType::Array {
        element: Box::new(ManagedType::Char),
        nullable: true,
    });
    let before = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(0),
        args: vec![before],
    });
    let obj = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    let alias = b.value(
        if barrier == 7 {
            ManagedType::Int
        } else {
            ManagedType::Reference(ClassRef::new(0))
        },
        |result| ManagedInsn::Move { result, value: obj },
    );
    let one = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    let alias = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::Move {
            result,
            value: alias,
        }
    });
    match barrier {
        1 => b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(0),
            args: vec![one],
        }),
        2 => {
            b.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
                op: ArithOp::Div,
                result,
                lhs: one,
                rhs: one,
            });
        }
        3 => {
            b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
                ManagedInsn::NewInstance {
                    result,
                    class: ClassRef::new(0),
                }
            });
        }
        4 => {
            b.value(ManagedType::Boolean, |result| ManagedInsn::Compare {
                op: CompareOp::Equal,
                result,
                lhs: obj,
                rhs: alias,
            });
        }
        5 => {
            b.value(ManagedType::Int, |result| ManagedInsn::ArrayLength {
                result,
                array: chars,
            });
        }
        9 => {
            b.value(
                ManagedType::Array {
                    element: Box::new(ManagedType::Char),
                    nullable: true,
                },
                |result| ManagedInsn::NewArray {
                    result,
                    element: TypeRef::new(0),
                    length: one,
                },
            );
        }
        10 => {
            b.value(ManagedType::Int, |result| ManagedInsn::FieldGet {
                result,
                object: None,
                field: FieldRef::new(0),
            });
        }
        11 => {
            b.value(
                ManagedType::Array {
                    element: Box::new(ManagedType::Char),
                    nullable: true,
                },
                |result| ManagedInsn::Cast {
                    result,
                    value: chars,
                    target: TypeRef::new(0),
                },
            );
        }
        12 => {
            b.push(ManagedInsn::Opaque {
                family_tag: 17,
                reads: vec![],
                writes: vec![],
                effects: managed_ir::effects::EffectFlags::NONE,
            });
        }
        _ => {
            b.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
                op: ArithOp::Add,
                result,
                lhs: one,
                rhs: one,
            });
        }
    }
    let count = b.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: one,
        rhs: one,
    });
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(if barrier == 8 { 3 } else { 2 }),
        args: vec![alias, chars, one, count],
    });
    if barrier == 6 {
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(1),
            args: vec![obj, chars],
        });
    }
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(0),
        args: vec![before],
    });
    b.push(ManagedInsn::Return { value: Some(alias) });
    (b.finish(), chars)
}

#[test]
fn alias_and_pure_gap_compile_and_preserve_constructor_exception() {
    let (ir, chars) = fixture(0);
    let result = emit_method(
        &ir,
        &signature(
            "construct",
            JavaType::Class("java.lang.String".into()),
            vec![JavaType::Array(Box::new(JavaType::Char))],
        ),
        &ParameterBindings {
            parameters: vec![chars],
            ..Default::default()
        },
        &Resolver,
    );
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    java(
        &source,
        "if (!construct(new char[]{'x','y','z'}).equals(\"yz\") || effects != 2) throw new AssertionError(); effects = 0; try { construct(null); throw new AssertionError(); } catch (NullPointerException expected) {} if (effects != 1) throw new AssertionError(); effects = 0; try { construct(new char[]{'x'}); throw new AssertionError(); } catch (IndexOutOfBoundsException expected) {} if (effects != 1) throw new AssertionError(); ",
    );
}

#[test]
fn effects_throwing_allocation_preinit_use_and_repeated_init_reject() {
    for barrier in 1..=12 {
        let (ir, chars) = fixture(barrier);
        let result = emit_method(
            &ir,
            &signature(
                "construct",
                JavaType::Class("java.lang.String".into()),
                vec![JavaType::Array(Box::new(JavaType::Char))],
            ),
            &ParameterBindings {
                parameters: vec![chars],
                ..Default::default()
            },
            &Resolver,
        );
        if matches!(barrier, 1 | 2 | 5) {
            let SourceResult::Complete { source, .. } = result else {
                panic!("barrier{barrier}:{result:?}")
            };
            crate::object_plan::sequence_tests::compile_fixture(
                &source,
                "static int tick(int n) { return n; }",
            );
            continue;
        }
        let rejected = match barrier {
            7 => matches!(result, SourceResult::Unsupported(Unsupported::Type(_))),
            8 | 9 | 11 => matches!(result, SourceResult::Unsupported(Unsupported::Metadata(_))),
            _ => matches!(
                result,
                SourceResult::Unsupported(Unsupported::Instruction(_))
            ),
        };
        assert!(rejected, "barrier {barrier}: {result:?}");
    }
}

#[test]
fn explicit_branch_between_allocation_and_constructor_is_a_barrier() {
    let mut b = Builder::default();
    let chars = b.parameter(ManagedType::Array {
        element: Box::new(ManagedType::Char),
        nullable: true,
    });
    let obj = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    b.push(ManagedInsn::Return { value: None }); // Replaced with exact CFG branch below.
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![obj, chars],
    });
    b.push(ManagedInsn::Return { value: Some(obj) });
    let mut cfg = Cfg::new(b.order[0]);
    let branch = cfg.add_block(b.order[1]);
    let init = cfg.add_block(b.order[2]);
    let ret = cfg.add_block(b.order[3]);
    cfg.add_edge(cfg.entry(), branch, EdgeKind::Unconditional)
        .unwrap();
    let edge = cfg.add_edge(branch, init, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(init, ret, EdgeKind::Unconditional).unwrap();
    b.insns[b.order[1]] = ManagedInsn::Branch {
        condition: None,
        then_edge: edge,
        else_edge: None,
    };
    let ir = MethodIr::<SsaForm>::from_parts(cfg, b.insns, b.values);
    let result = emit_method(
        &ir,
        &signature(
            "construct",
            JavaType::Class("java.lang.String".into()),
            vec![JavaType::Array(Box::new(JavaType::Char))],
        ),
        &ParameterBindings {
            parameters: vec![chars],
            ..Default::default()
        },
        &Resolver,
    );
    assert!(matches!(
        result,
        SourceResult::Unsupported(Unsupported::Instruction(_))
    ));
}

#[test]
fn hoisted_reference_copy_retains_reserved_static_type() {
    struct Widening;
    impl JavaResolver for Widening {
        fn class(&self, r: ClassRef) -> Option<String> {
            Some(
                if r.index() == 0 {
                    "java.lang.Integer"
                } else {
                    "java.lang.Object"
                }
                .into(),
            )
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
        fn assignable(&self, a: &JavaType, e: &JavaType) -> bool {
            a == e
                || (a == &JavaType::Class("java.lang.Integer".into())
                    && e == &JavaType::Class("java.lang.Object".into()))
        }
    }
    let mut b = Builder::default();
    let input = b.parameter(ManagedType::Reference(ClassRef::new(0)));
    let result = b.value(ManagedType::Reference(ClassRef::new(1)), |result| {
        ManagedInsn::Move {
            result,
            value: input,
        }
    });
    let ir = b.finish();
    let object = JavaType::Class("java.lang.Object".into());
    let mut e = Emitter {
        allocation_readable: None,
        object_planning: Default::default(),
        ir: &ir,
        resolver: &Widening,
        values: HashMap::from([(result, ("slot".into(), object.clone()))]),
        lines: vec![],
        next: 0,
        parameters: HashSet::from([input]),
        reserved: HashSet::new(),
        current: Some(ir.value(result).unwrap().def()),
        hoisted: HashSet::from([result]),
        declarations: HashMap::new(),
    };
    e.define_local(
        result,
        "p0".into(),
        JavaType::Class("java.lang.Integer".into()),
        false,
    )
    .unwrap();
    assert_eq!(e.lines, vec!["slot = p0;"]);
    assert_eq!(e.get(result).unwrap(), ("slot".into(), object));
}
