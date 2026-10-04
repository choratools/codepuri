struct ExclusiveResolver;
impl JavaResolver for ExclusiveResolver {
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || matches!((actual, expected), (JavaType::Class(a), JavaType::Class(b)) if a == "Fixture.Failure" && matches!(b.as_str(), "java.lang.Throwable" | "java.lang.RuntimeException"))
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Fixture.Failure".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "Fixture.Failure".into(),
            name: "<init>".into(),
            parameters: vec![],
            return_type: JavaType::Void,
            is_static: false,
            throws: vec![],
        })
    }
}

#[test]
fn exclusive_shared_allocation_is_materialized_per_path_not_globally() {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let this = input(&mut insns, &mut values, ManagedType::Unknown);
    let first = input(&mut insns, &mut values, ManagedType::Boolean);
    let second = input(&mut insns, &mut values, ManagedType::Boolean);
    let (new, allocated) = def(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(0)),
        |result| ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        },
    );
    let init = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![allocated],
    });
    let throw = insns.insert(ManagedInsn::Throw { value: allocated });
    let ret = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(new);
    let init_block = cfg.add_block(init);
    let throw_block = cfg.add_block(throw);
    let incoming = cfg
        .add_edge(cfg.entry(), init_block, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(init_block, throw_block, EdgeKind::Unconditional)
        .unwrap();
    let f = Fixture {
        ir: MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        this,
        start: first,
        limit: second,
        entry: init_block,
        incoming,
    };
    let resolver = ExclusiveResolver;
    let mut e = emitter(&f, &resolver);
    e.values.insert(first, ("p0".into(), JavaType::Boolean));
    e.values.insert(second, ("p1".into(), JavaType::Boolean));
    let instruction =
        |id| crate::loop_control::ControlStep::Ordinary(branching::Step::Instruction(id));
    let tail = vec![instruction(new), instruction(init), instruction(throw)];
    let steps = vec![crate::loop_control::ControlStep::If {
        condition: first,
        then_steps: tail.clone(),
        else_steps: vec![crate::loop_control::ControlStep::If {
            condition: second,
            then_steps: tail,
            else_steps: vec![instruction(ret)],
        }],
    }];
    let mut m = method();
    m.parameters = vec![JavaType::Boolean, JavaType::Boolean];
    render_suffix_control(&mut e, &steps, &m, 0).unwrap();
    assert!(!e.values.contains_key(&allocated));
    let mut repeated = emitter(&f, &resolver);
    assert!(
        render_suffix_control(
            &mut repeated,
            &[instruction(new), instruction(init), instruction(init)],
            &m,
            0
        )
        .is_err()
    );
    let mut early = emitter(&f, &resolver);
    assert!(
        render_suffix_control(&mut early, &[instruction(new), instruction(throw)], &m, 0).is_err()
    );
    let body = e.lines.join("\n");
    assert_eq!(body.matches("new Fixture.Failure(").count(), 2);
    let source = format!(
        r#"public class Fixture {{
        static int effects;
        static final RuntimeException SENTINEL = new RuntimeException();
        static boolean fail;
        static class Failure extends RuntimeException {{ Failure() {{ effects++; if(fail) throw SENTINEL; }} }}
        Fixture(boolean p0, boolean p1) {{ {body} }}
        static void original(boolean p0, boolean p1) {{ if(p0) throw new Failure(); if(p1) throw new Failure(); }}
        static Throwable run(boolean generated, boolean a, boolean b) {{ try {{ if(generated) new Fixture(a,b); else original(a,b); return null; }} catch(Throwable t) {{ return t; }} }}
        public static void main(String[] args) {{ int checks=0; for(boolean f:new boolean[]{{false,true}}) for(boolean a:new boolean[]{{false,true}}) for(boolean b:new boolean[]{{false,true}}) {{ fail=f; effects=0; Throwable expected=run(false,a,b); int count=effects; effects=0; Throwable actual=run(true,a,b); if(effects!=count || (actual==null)!=(expected==null) || (actual!=null && actual.getClass()!=expected.getClass()) || (actual==SENTINEL)!=(expected==SENTINEL)) throw new AssertionError(); checks++; }} System.out.println(checks); }}
    }}"#
    );
    for home in [
        "/home/jtjisgod/tools/jdk-17.0.13+11",
        "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1",
    ] {
        let root = std::env::temp_dir().join(format!(
            "constructor-exclusive-{}-{}",
            std::process::id(),
            home.contains("jdk25")
        ));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("Fixture.java");
        std::fs::write(&file, &source).unwrap();
        let compile = std::process::Command::new(format!("{home}/bin/javac"))
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let run = std::process::Command::new(format!("{home}/bin/java"))
            .args(["-cp", root.to_str().unwrap(), "Fixture"])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "8");
    }
}

#[test]
fn normally_joined_phi_copies_keep_their_later_use() {
    let f = fixture(0);
    let resolver = R;
    let mut e = emitter(&f, &resolver);
    let entry = f.ir.cfg().block(f.entry).unwrap();
    let ManagedInsn::Phi { result, inputs } = f.ir.instruction(*entry.payload()).unwrap() else {
        panic!("fixture Phi")
    };
    e.values.insert(*result, ("joined".into(), JavaType::Int));
    e.hoisted.insert(*result);
    let condition =
        f.ir.values()
            .iter()
            .find_map(|(id, value)| (value.ty() == &ManagedType::Boolean).then_some(id))
            .unwrap();
    e.values
        .insert(condition, ("p0 > 0".into(), JavaType::Boolean));
    let copy = |edge, source| {
        crate::loop_control::ControlStep::Ordinary(branching::Step::Copies {
            edge,
            copies: vec![branching::Copy {
                destination: *result,
                source,
            }],
        })
    };
    let then_steps = vec![copy(inputs[0].0, f.start)];
    let else_steps = vec![copy(inputs[1].0, f.limit)];
    assert!(!suffix_steps_terminate(&f.ir, &then_steps));
    assert!(!suffix_steps_terminate(&f.ir, &else_steps));
    render_suffix_control(
        &mut e,
        &[crate::loop_control::ControlStep::If {
            condition,
            then_steps,
            else_steps,
        }],
        &method(),
        0,
    )
    .unwrap();
    assert_eq!(e.get(*result).unwrap(), ("joined".into(), JavaType::Int));
    assert!(e.hoisted.contains(result)); // Copy emission retains existing bookkeeping.
    let body = e.lines.join("\n");
    let source = format!(
        "public class JoinFixture {{ static int f(int p0,int p1) {{ int joined; {body} return joined; }} public static void main(String[] args) {{ for(int a=-2;a<3;a++) for(int b=-2;b<3;b++) if(f(a,b)!=(a>0?a:b)) throw new AssertionError(); System.out.println(25); }} }}"
    );
    let home = "/home/jtjisgod/tools/jdk-17.0.13+11";
    let root = std::env::temp_dir().join(format!("suffix-joined-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("JoinFixture.java");
    std::fs::write(&file, source).unwrap();
    let compile = std::process::Command::new(format!("{home}/bin/javac"))
        .arg(file)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(format!("{home}/bin/java"))
        .args(["-cp", root.to_str().unwrap(), "JoinFixture"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "25");
}

#[test]
fn terminal_checkpoint_requires_all_prefix_transfers_to_be_internal() {
    use crate::loop_control::ControlStep;
    use crate::loop_control_regions::TransferKind;
    let f = fixture(0);
    let ret = f
        .ir
        .instructions()
        .iter()
        .find_map(|(id, insn)| matches!(insn, ManagedInsn::Return { value: None }).then_some(id))
        .unwrap();
    let termination = ControlStep::Ordinary(branching::Step::Instruction(ret));
    for kind in [
        TransferKind::Break { header: f.entry },
        TransferKind::Continue { header: f.entry },
    ] {
        let transfer = ControlStep::Transfer {
            edge: f.incoming,
            kind,
        };
        let external = vec![
            ControlStep::If {
                condition: f.start,
                then_steps: vec![transfer.clone()],
                else_steps: vec![],
            },
            termination.clone(),
        ];
        assert!(!suffix_steps_terminate(&f.ir, &external));
        let internal = vec![
            ControlStep::Loop {
                header: f.entry,
                body: vec![ControlStep::If {
                    condition: f.start,
                    then_steps: vec![transfer],
                    else_steps: vec![],
                }],
            },
            termination.clone(),
        ];
        assert!(suffix_steps_terminate(&f.ir, &internal));
    }
}
