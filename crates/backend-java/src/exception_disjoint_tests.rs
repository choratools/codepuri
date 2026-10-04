fn disjoint_fixture() -> (MethodIr<SsaForm>, Vec<Metadata>, DeclarationBindings) {
    disjoint_fixture_inner(false, false)
}
fn disjoint_fixture_inner(
    handler_call: bool,
    unavailable_result: bool,
) -> (MethodIr<SsaForm>, Vec<Metadata>, DeclarationBindings) {
    let mut b = Builder::new();
    let choose = b.incoming(ManagedType::Boolean);
    let choose_second = b.incoming(ManagedType::Boolean);
    let mode = b.incoming(ManagedType::Int);
    let object = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let entry = b.cfg.entry();
    let h2 = b.block();
    let end = b.block();
    b.set(end, ManagedInsn::Return { value: None });
    let mut scopes = Vec::new();
    for (head, continuation, condition) in [(entry, h2, choose), (h2, end, choose_second)] {
        let work = b.block();
        let result = b.value(work, ManagedType::Int);
        b.set(
            work,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(result),
                method: MethodRef::new(0),
                args: vec![mode, object],
            },
        );
        b.link(work, continuation);
        let caught = b.block();
        let exception = b.value(caught, ManagedType::Unknown);
        b.set(
            caught,
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads: vec![],
                writes: vec![exception],
                effects: EffectFlags::NONE,
            },
        );
        if handler_call {
            let action = b.block();
            let output = b.value(action, ManagedType::Int);
            b.set(
                action,
                ManagedInsn::Invoke {
                    kind: InvokeKind::Static,
                    result: Some(output),
                    method: MethodRef::new(0),
                    args: vec![if unavailable_result { result } else { mode }, object],
                },
            );
            b.link(caught, action);
            b.link(action, continuation);
        } else {
            b.link(caught, continuation);
        }
        let yes = b.edge(head, work, EdgeKind::BranchTrue);
        let no = b.edge(head, continuation, EdgeKind::BranchFalse);
        b.set(
            head,
            ManagedInsn::Branch {
                condition: Some(condition),
                then_edge: yes,
                else_edge: Some(no),
            },
        );
        b.edge(work, caught, EdgeKind::Exception("broad".into()));
        scopes.push(Metadata {
            scope: scopes::RegionScope {
                protected: vec![work],
                catches: vec![scopes::Catch {
                    key: "broad".into(),
                    entry: caught,
                }],
                continuation: Some(continuation),
            },
            hierarchy: hierarchy(),
            all: false,
        });
    }
    (
        b.finish(),
        scopes,
        DeclarationBindings {
            receiver: None,
            parameters: vec![Some(choose), Some(choose_second), Some(mode), Some(object)],
            names: vec![],
        },
    )
}
#[test]
fn disjoint_conditionals_compile_and_run_all_throw_modes() {
    let (ir, metadata, bindings) = disjoint_fixture();
    let refs = metadata
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let method = JavaMethod {
        owner: "RegionRuntime".into(),
        name: "execute".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![
            JavaType::Boolean,
            JavaType::Boolean,
            JavaType::Int,
            JavaType::Class("java.lang.Object".into()),
        ],
        throws: vec![],
    };
    let result = emit_method_disjoint(&ir, &method, &bindings, &Resolver, &refs);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}");
    };
    assert_eq!(source.matches("try {").count(), 2);
    run_java(&format!(
        "public class RegionRuntime {{ static int calls; static int work(int mode,Object o) {{calls++;if(mode>0) throw new IllegalArgumentException();return 3;}} {source} public static void main(String[] args) {{for(int bits=0;bits<4;bits++) {{for(int m=0;m<2;m++) {{calls=0;execute((bits&1)!=0,(bits&2)!=0,m,null);if(calls!=Integer.bitCount(bits)) throw new AssertionError();}}}}System.out.println(\"PASS disjoint 8 paths\");}}}}"
    ));
}
#[test]
fn disjoint_rejects_shared_handlers_and_nested_protected() {
    let (ir, mut metadata, bindings) = disjoint_fixture();
    metadata[1].scope.catches[0].entry = metadata[0].scope.catches[0].entry;
    let refs = metadata
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    assert!(
        plan_disjoint(
            &ir,
            &refs,
            &bindings
                .parameters
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>(),
            &Resolver
        )
        .is_err()
    );
    let shared = metadata[0].scope.protected[0];
    metadata[1].scope.protected.push(shared);
    let refs = metadata
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    assert!(
        plan_disjoint(
            &ir,
            &refs,
            &bindings
                .parameters
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>(),
            &Resolver
        )
        .is_err()
    );
}

#[test]
fn disjoint_handler_throw_stays_outside_other_scope_and_bad_snapshot_rejects() {
    let (ir, metadata, bindings) = disjoint_fixture_inner(true, false);
    let refs = metadata
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let method = JavaMethod {
        owner: "RegionRuntime".into(),
        name: "execute".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![
            JavaType::Boolean,
            JavaType::Boolean,
            JavaType::Int,
            JavaType::Class("java.lang.Object".into()),
        ],
        throws: vec![],
    };
    let result = emit_method_disjoint(&ir, &method, &bindings, &Resolver, &refs);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}");
    };
    run_java(&format!(
        "public class RegionRuntime {{static int calls; static RuntimeException original=new IllegalArgumentException();static int work(int m,Object o) {{calls++;if(m>0) throw original;return 1;}}{source}public static void main(String[] a) {{try {{execute(true,true,1,null);throw new AssertionError();}}catch(RuntimeException e) {{if(e!=original || calls!=2) throw new AssertionError();}}calls=0;execute(true,true,0,null);if(calls!=2) throw new AssertionError();System.out.println(\"PASS handler scope\");}}}}"
    ));
    let (ir, metadata, bindings) = disjoint_fixture_inner(true, true);
    let refs = metadata
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    assert!(matches!(
        plan_disjoint(
            &ir,
            &refs,
            &bindings
                .parameters
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>(),
            &Resolver
        ),
        Err(Error::Unavailable(_))
    ));
}
#[test]
fn disjoint_planner_preserves_prethrow_phi_and_catch_priority() {
    let (ir, metadata, bindings) = branch_fixture(false);
    let mut method = declaration("branch");
    method.parameters.insert(0, JavaType::Boolean);
    let result = emit_method_disjoint(&ir, &method, &bindings, &Resolver, &[&metadata]);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}");
    };
    run_java(&format!(
        "public class RegionRuntime {{static int work(int mode,Object o) {{if(mode==1) throw new ArithmeticException();if(mode==2) throw new IllegalArgumentException();return 42;}}{source}public static void main(String[] a) throws Throwable {{Object o=new Object();if(branch(true,0,o)!=42 || branch(false,0,o)!=42 || branch(true,1,o)!=142 || branch(false,1,o)!=110 || branch(true,2,o)!=242 || branch(false,2,o)!=210)throw new AssertionError();System.out.println(\"PASS prethrow Phi\");}}}}"
    ));
}
