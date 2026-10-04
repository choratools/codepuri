use super::*;

#[test]
fn contextual_phi_preserves_selected_edge_and_availability() {
    let mut b = Builder::new();
    let object = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let choose = b.incoming(ManagedType::Boolean);
    let head = b.cfg.entry();
    let null = b.block();
    let zero = b.value(null, ManagedType::Zero);
    b.set(
        null,
        ManagedInsn::Const {
            result: zero,
            value: managed_ir::insn::Constant::Zero,
        },
    );
    let reference = b.block();
    let copied = b.value(reference, ManagedType::Reference(ClassRef::new(0)));
    b.set(
        reference,
        ManagedInsn::Move {
            result: copied,
            value: object,
        },
    );
    let yes = b.edge(head, null, EdgeKind::BranchTrue);
    let no = b.edge(head, reference, EdgeKind::BranchFalse);
    b.set(
        head,
        ManagedInsn::Branch {
            condition: Some(choose),
            then_edge: yes,
            else_edge: Some(no),
        },
    );
    let join = b.block();
    let result = b.value(join, ManagedType::Reference(ClassRef::new(0)));
    let null_edge = b.link(null, join);
    let reference_edge = b.link(reference, join);
    b.set(
        join,
        ManagedInsn::Phi {
            result,
            inputs: vec![(null_edge, zero), (reference_edge, copied)],
        },
    );
    let end = b.block();
    b.link(join, end);
    b.set(
        end,
        ManagedInsn::Return {
            value: Some(result),
        },
    );
    let ir = b.finish();
    assert!(matches!(
        prepare_region(&ir, &[object, choose], &HashMap::new()),
        Err(Error::Invalid(_))
    ));
    let declarations = crate::contextual_locals::declaration_plan(
        &ir,
        &JavaType::Class("java.lang.Object".into()),
        &Resolver,
    )
    .unwrap();
    assert!(declarations.contains_key(&result));
    let (p, _, _) = prepare_region(&ir, &[object, choose], &declarations).unwrap();
    let mut null_state = HashSet::from([zero]);
    p.transfer(null_edge, &mut null_state).unwrap();
    assert!(null_state.contains(&result));
    let mut ref_state = HashSet::from([copied]);
    p.transfer(reference_edge, &mut ref_state).unwrap();
    assert!(ref_state.contains(&result));
    assert!(
        matches!(p.transfer(null_edge, &mut HashSet::new()), Err(Error::Unavailable(v)) if v == zero)
    );
    let incompatible =
        crate::contextual_locals::declaration_plan(&ir, &JavaType::Int, &Resolver).unwrap();
    assert!(!incompatible.contains_key(&result));
}

#[test]
fn phi_family_equality_to_literal_zero_is_contextual_not_unknown() {
    use managed_ir::ops::CompareOp;
    for (op, unknown_peer, accepted) in [
        (CompareOp::Equal, false, true),
        (CompareOp::NotEqual, false, true),
        (CompareOp::LessThan, false, false),
        (CompareOp::Equal, true, false),
    ] {
        let mut b = Builder::new();
        let object = b.incoming(ManagedType::Reference(ClassRef::new(0)));
        let copy_block = b.cfg.entry();
        let copied = b.value(copy_block, ManagedType::Reference(ClassRef::new(0)));
        b.set(
            copy_block,
            ManagedInsn::Move {
                result: copied,
                value: object,
            },
        );
        let literal_block = b.block();
        let peer = if unknown_peer {
            b.incoming(ManagedType::Unknown)
        } else {
            let v = b.value(literal_block, ManagedType::Zero);
            b.set(
                literal_block,
                ManagedInsn::Const {
                    result: v,
                    value: Constant::Zero,
                },
            );
            v
        };
        b.link(copy_block, literal_block);
        let compare = b.block();
        let condition = b.value(compare, ManagedType::Boolean);
        b.set(
            compare,
            ManagedInsn::Compare {
                op,
                result: condition,
                lhs: copied,
                rhs: peer,
            },
        );
        b.link(literal_block, compare);
        let join = b.block();
        let result = b.value(join, ManagedType::Reference(ClassRef::new(0)));
        let edge = b.link(compare, join);
        b.set(
            join,
            ManagedInsn::Phi {
                result,
                inputs: vec![(edge, copied)],
            },
        );
        let ret = b.block();
        b.link(join, ret);
        b.set(
            ret,
            ManagedInsn::Return {
                value: Some(result),
            },
        );
        let ir = b.finish();
        let proof = crate::contextual_locals::checked_phi_plan(
            &ir,
            result,
            &JavaType::Class("java.lang.Object".into()),
            &JavaType::Class("java.lang.Object".into()),
            &hierarchy(),
            &Resolver,
        );
        assert_eq!(
            proof.is_ok(),
            accepted,
            "{op:?} unknown={unknown_peer}: {proof:?}"
        );
    }
}

fn shared_void_fixture(kind: u8) -> (MethodIr<SsaForm>, Metadata, DeclarationBindings) {
    let mut b = Builder::new();
    let choose = b.incoming(ManagedType::Boolean);
    let mode = b.incoming(ManagedType::Int);
    let object = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let head = b.cfg.entry();
    let work = b.block();
    let value = b.value(work, ManagedType::Int);
    b.set(
        work,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(value),
            method: MethodRef::new(0),
            args: vec![mode, object],
        },
    );
    let terminal = b.block();
    b.set(
        terminal,
        if kind == 1 {
            ManagedInsn::Return { value: Some(value) }
        } else if kind == 2 {
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(1),
                args: vec![],
            }
        } else {
            ManagedInsn::Return { value: None }
        },
    );
    if kind == 4 {
        let selected = b.value(terminal, ManagedType::Int);
        b.set(
            terminal,
            ManagedInsn::Phi {
                result: selected,
                inputs: vec![],
            },
        );
    } else if kind == 5 {
        let caught = b.value(terminal, ManagedType::Reference(ClassRef::new(1)));
        b.set(
            terminal,
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads: vec![],
                writes: vec![caught],
                effects: EffectFlags::NONE,
            },
        );
    }
    let yes = b.edge(head, work, EdgeKind::BranchTrue);
    let no = b.edge(head, terminal, EdgeKind::BranchFalse);
    b.set(
        head,
        ManagedInsn::Branch {
            condition: Some(choose),
            then_edge: yes,
            else_edge: Some(no),
        },
    );
    b.link(work, terminal);
    b.edge(work, terminal, EdgeKind::Exception("broad".into()));
    if kind == 3 {
        b.link(terminal, head);
    }
    (
        b.finish(),
        Metadata {
            scope: scopes::RegionScope {
                protected: vec![work],
                catches: vec![scopes::Catch {
                    key: "broad".into(),
                    entry: terminal,
                }],
                continuation: Some(terminal),
            },
            hierarchy: hierarchy(),
            all: false,
        },
        DeclarationBindings {
            parameters: vec![Some(choose), Some(mode), Some(object)],
            ..Default::default()
        },
    )
}

#[test]
fn shared_void_terminal_preserves_bypass_normal_and_exception_effects() {
    let (ir, metadata, bindings) = shared_void_fixture(0);
    assert!(shared_void_terminal(&ir, &metadata.scope));
    assert!(scopes::structure_region(ir.cfg(), &metadata.scope).is_err());
    let method = JavaMethod {
        owner: "RegionRuntime".into(),
        name: "send".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![
            JavaType::Boolean,
            JavaType::Int,
            JavaType::Class("java.lang.Object".into()),
        ],
        throws: vec![],
    };
    let source = match emit_method_region(&ir, &method, &bindings, &Resolver, &metadata) {
        SourceResult::Complete { source, .. } => source,
        other => panic!("{other:?}"),
    };
    run_java(&format!(
        r#"public class RegionRuntime {{
      static int effects; static RuntimeException original=new ArithmeticException("original");
      static int work(int mode,Object obj) {{effects++;if(mode!=0)throw original;return 42;}}
      static void cleanup() {{throw new AssertionError("unexpected handler effect");}}
      {source}
      public static void main(String[] args) {{
        send(false,1,null);if(effects!=0)throw new AssertionError("bypass");
        send(true,0,new Object());if(effects!=1)throw new AssertionError("normal");
        send(true,1,null);if(effects!=2)throw new AssertionError("caught effect");
        System.out.println("PASS shared terminal bypass normal caught");
      }}
    }}"#
    ));
    let mut invalid_scope = metadata.scope.clone();
    invalid_scope.protected.push(invalid_scope.protected[0]);
    assert!(scopes::structure_disjoint(ir.cfg(), &[invalid_scope]).is_err());
    for kind in [1, 2, 3, 4, 5] {
        let (ir, metadata, _) = shared_void_fixture(kind);
        assert!(!shared_void_terminal(&ir, &metadata.scope));
    }
}

struct SharedResolver;
impl JavaResolver for SharedResolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Resolver.class(r)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        if r.index() == 3 {
            return Some(JavaMethod {
                owner: "RegionRuntime".into(),
                name: "log".into(),
                is_static: true,
                return_type: JavaType::Void,
                parameters: vec![JavaType::Class("java.lang.Throwable".into())],
                throws: vec![],
            });
        }
        Resolver.method(r)
    }
    fn field(&self, r: FieldRef) -> Option<crate::JavaField> {
        Resolver.field(r)
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        Resolver.string(r)
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        Resolver.assignable(a, b)
    }
}

fn shared_handler_fixture(variant: u8) -> (MethodIr<SsaForm>, [Metadata; 2], DeclarationBindings) {
    let mut b = Builder::new();
    let mode = b.incoming(ManagedType::Int);
    let object = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let work1 = b.cfg.entry();
    let first = b.value(work1, ManagedType::Int);
    b.set(
        work1,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(first),
            method: MethodRef::new(0),
            args: vec![mode, object],
        },
    );
    let gap = b.block();
    b.set(
        gap,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(2),
            args: vec![],
        },
    );
    b.link(work1, gap);
    let work2 = b.block();
    let second = b.value(work2, ManagedType::Int);
    b.set(
        work2,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(second),
            method: MethodRef::new(0),
            args: vec![mode, object],
        },
    );
    b.link(gap, work2);
    let ret = b.block();
    b.link(work2, ret);
    b.set(ret, ManagedInsn::Return { value: None });
    let tag = b.block();
    let caught = b.value(tag, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        tag,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![caught],
            effects: EffectFlags::MAY_THROW,
        },
    );
    if variant == 1 {
        b.set(
            tag,
            ManagedInsn::Const {
                result: caught,
                value: Constant::Null,
            },
        );
    }
    let catch_entry = if variant == 3 {
        let phi = b.block();
        let saved = b.value(phi, ManagedType::Int);
        let left = b.edge(work1, phi, EdgeKind::Exception("shared".into()));
        let right = b.edge(work2, phi, EdgeKind::Exception("shared".into()));
        b.set(
            phi,
            ManagedInsn::Phi {
                result: saved,
                inputs: vec![(left, first), (right, second)],
            },
        );
        b.link(phi, tag);
        phi
    } else {
        b.edge(work1, tag, EdgeKind::Exception("shared".into()));
        b.edge(work2, tag, EdgeKind::Exception("shared".into()));
        tag
    };
    if variant == 2 {
        let edge = b.link(ret, catch_entry);
        b.set(
            ret,
            ManagedInsn::Branch {
                condition: None,
                then_edge: edge,
                else_edge: None,
            },
        );
    }
    let alias = b.block();
    b.link(tag, alias);
    let copied = b.value(alias, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        alias,
        ManagedInsn::Move {
            result: copied,
            value: caught,
        },
    );
    let log = b.block();
    b.link(alias, log);
    b.set(
        log,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(3),
            args: vec![copied],
        },
    );
    let end = b.block();
    b.link(log, end);
    b.set(end, ManagedInsn::Return { value: None });
    let scope = |protected, continuation| Metadata {
        scope: scopes::RegionScope {
            protected: vec![protected],
            catches: vec![scopes::Catch {
                key: "shared".into(),
                entry: catch_entry,
            }],
            continuation: Some(continuation),
        },
        hierarchy: hierarchy(),
        all: false,
    };
    (
        b.finish(),
        [scope(work1, gap), scope(work2, ret)],
        DeclarationBindings {
            parameters: vec![Some(mode), Some(object)],
            ..Default::default()
        },
    )
}

#[test]
fn shared_effectful_handler_runs_once_and_gap_throw_is_uncaught() {
    let (ir, metadata, bindings) = shared_handler_fixture(0);
    let original = metadata.iter().map(|m| m.scope.clone()).collect::<Vec<_>>();
    assert!(scopes::structure_disjoint(ir.cfg(), &original).is_err());
    assert!(scopes::structure_disjoint_shared_handlers(ir.cfg(), &original).is_ok());
    let method = JavaMethod {
        owner: "RegionRuntime".into(),
        name: "send".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![JavaType::Int, JavaType::Class("java.lang.Object".into())],
        throws: vec!["java.lang.Throwable".into()],
    };
    let refs: Vec<&dyn RegionMetadata> =
        metadata.iter().map(|m| m as &dyn RegionMetadata).collect();
    let source = match emit_method_disjoint(&ir, &method, &bindings, &SharedResolver, &refs) {
        SourceResult::Complete { source, .. } => source,
        other => panic!("{other:?}"),
    };
    assert_eq!(source.matches("RegionRuntime.log(").count(), 1, "{source}");
    assert!(source.contains("break handler"), "{source}");
    run_java(&format!(
        r#"public class RegionRuntime {{
      static int effects,logs,gaps; static boolean gapThrow;
      static RuntimeException original=new ArithmeticException("original"),gapFailure=new IllegalArgumentException("gap");
      static int work(int at,Object object) {{effects++;if(at==effects)throw original;return 42;}}
      static void cleanup() {{gaps++;if(gapThrow)throw gapFailure;}}
      static void log(Throwable e) {{if(e!=original)throw new AssertionError("identity");logs++;}}
      {source}
      public static void main(String[] args) throws Throwable {{
        send(0,null);if(effects!=2||logs!=0||gaps!=1)throw new AssertionError("normal");
        effects=logs=gaps=0;send(1,null);if(effects!=1||logs!=1||gaps!=0)throw new AssertionError("first");
        effects=logs=gaps=0;send(2,null);if(effects!=2||logs!=1||gaps!=1)throw new AssertionError("second");
        effects=logs=gaps=0;gapThrow=true;try{{send(0,null);throw new AssertionError("gap returned");}}catch(IllegalArgumentException e){{if(e!=gapFailure||logs!=0||effects!=1)throw new AssertionError("gap captured");}}
        System.out.println("PASS shared handler identity single log original gap scope");
      }}
    }}"#
    ));
    let mut mismatch = original.clone();
    mismatch[1].catches[0].key = "other".into();
    assert!(scopes::structure_disjoint_shared_handlers(ir.cfg(), &mismatch).is_err());
    let mut order = original.clone();
    let duplicate = order[1].catches[0].clone();
    order[1].catches.push(duplicate);
    assert!(scopes::structure_disjoint_shared_handlers(ir.cfg(), &order).is_err());
}

#[test]
fn shared_handler_rejects_capture_loss_normal_fallthrough_and_prethrow_unavailable_phi() {
    for variant in [1, 2, 3] {
        let (ir, metadata, bindings) = shared_handler_fixture(variant);
        let refs: Vec<&dyn RegionMetadata> =
            metadata.iter().map(|m| m as &dyn RegionMetadata).collect();
        let result = plan_disjoint(
            &ir,
            &refs,
            &bindings
                .parameters
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>(),
            &SharedResolver,
        );
        assert!(result.is_err(), "variant {variant}: {result:?}");
        if variant == 3 {
            assert!(matches!(result, Err(Error::Unavailable(_))), "{result:?}");
        }
    }
}
