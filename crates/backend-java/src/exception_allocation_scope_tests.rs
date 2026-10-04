use super::*;
struct OriginalMetadata {
    base: Metadata,
    original: OriginalScope,
}
impl RegionMetadata for OriginalMetadata {
    fn original_scope(&self) -> Option<&OriginalScope> {
        Some(&self.original)
    }
    fn region_scope(&self) -> &scopes::RegionScope {
        self.base.region_scope()
    }
    fn catch_type(&self, key: &str) -> Option<JavaType> {
        self.base.catch_type(key)
    }
    fn hierarchy(&self) -> &Hierarchy {
        self.base.hierarchy()
    }
    fn is_catch_all(&self, key: &str) -> bool {
        self.base.is_catch_all(key)
    }
}
struct AllocationResolver;
impl JavaResolver for AllocationResolver {
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "java.lang.StringBuilder".into(),
            name: "<init>".into(),
            is_static: false,
            return_type: JavaType::Void,
            parameters: vec![],
            throws: vec![],
        })
    }
    fn class(&self, reference: ClassRef) -> Option<String> {
        Some(
            if reference == ClassRef::new(0) {
                "java.lang.StringBuilder"
            } else {
                "java.lang.RuntimeException"
            }
            .into(),
        )
    }
    fn field(&self, _: FieldRef) -> Option<crate::JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        Resolver.assignable(a, b)
    }
}
fn fixture(observe: bool) -> (MethodIr<SsaForm>, OriginalMetadata, Vec<InsnId>) {
    fixture_phi(observe, 0)
}
fn fixture_phi(observe: bool, phi_mode: u8) -> (MethodIr<SsaForm>, OriginalMetadata, Vec<InsnId>) {
    let mut b = Builder::new();
    let new = b.cfg.entry();
    let allocated = b.value(new, ManagedType::Reference(ClassRef::new(0)));
    b.set(
        new,
        ManagedInsn::NewInstance {
            result: allocated,
            class: ClassRef::new(0),
        },
    );
    let alias = b.block();
    let copied = b.value(alias, ManagedType::Reference(ClassRef::new(0)));
    b.set(
        alias,
        ManagedInsn::Move {
            result: copied,
            value: allocated,
        },
    );
    b.link(new, alias);
    let ctor = b.block();
    b.set(
        ctor,
        ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(0),
            args: vec![copied],
        },
    );
    b.link(alias, ctor);
    let normal = b.block();
    b.link(ctor, normal);
    let handler = b.block();
    let caught = b.value(handler, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        handler,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![caught],
            effects: EffectFlags::NONE,
        },
    );
    let end = b.block();
    b.link(handler, end);
    if observe {
        b.set(
            end,
            ManagedInsn::Return {
                value: Some(copied),
            },
        );
    }
    let entry = if phi_mode == 0 { handler } else { b.block() };
    let e1 = b.edge(new, entry, EdgeKind::Exception("broad".into()));
    let e2 = b.edge(ctor, entry, EdgeKind::Exception("broad".into()));
    if phi_mode != 0 {
        let left = b.incoming(ManagedType::Int);
        let right = if phi_mode == 1 {
            b.incoming(ManagedType::Int)
        } else {
            copied
        };
        let selected = b.value(entry, ManagedType::Int);
        b.set(
            entry,
            ManagedInsn::Phi {
                result: selected,
                inputs: vec![(e1, left), (e2, right)],
            },
        );
        b.link(entry, handler);
    }
    let chain = [new, alias, ctor]
        .iter()
        .map(|id| *b.cfg.block(*id).unwrap().payload())
        .collect::<Vec<_>>();
    let offsets = HashMap::from([
        (chain[0], 43),
        (chain[2], 45),
        (*b.cfg.block(end).unwrap().payload(), 59),
    ]);
    let metadata = OriginalMetadata {
        base: Metadata {
            scope: scopes::RegionScope {
                protected: vec![new, alias, ctor],
                catches: vec![scopes::Catch {
                    key: "broad".into(),
                    entry,
                }],
                continuation: Some(normal),
            },
            hierarchy: hierarchy(),
            all: false,
        },
        original: OriginalScope {
            ordinal: 0,
            start: 43,
            end: 69,
            handler_offset: 1,
            catches: vec![("broad".into(), 59)],
            offsets,
        },
    };
    (b.finish(), metadata, chain)
}
#[test]
fn same_original_range_exact_chain_and_handler_observation() {
    let (ir, mut md, chain) = fixture(false);
    assert!(allocation_proofs::prove(&ir, &[&md], &AllocationResolver).contains(&chain));
    let scoped = allocation_proofs::ScopedResolver::new(&AllocationResolver);
    assert!(!scoped.exception_allocation_chain(&chain));
    scoped.install(allocation_proofs::prove(&ir, &[&md], &AllocationResolver));
    assert!(scoped.exception_allocation_chain(&chain));
    assert!(!scoped.exception_allocation_chain(&[chain[0], chain[2]]));
    md.original.end = 45; // Same dispatch edges, different original range membership.
    assert!(allocation_proofs::prove(&ir, &[&md], &AllocationResolver).is_empty());
    let (_, mut adjacent, _) = fixture(false);
    adjacent.original.ordinal = 1;
    adjacent.original.start = 45;
    assert!(allocation_proofs::prove(&ir, &[&md, &adjacent], &AllocationResolver).is_empty());
    md.original.end = 69;
    md.original.catches[0].0 = "narrow".into();
    assert!(allocation_proofs::prove(&ir, &[&md], &AllocationResolver).is_empty());
    md.original.catches[0].0 = "broad".into();
    md.original.catches[0].1 = 60;
    assert!(allocation_proofs::prove(&ir, &[&md], &AllocationResolver).is_empty());
    md.original.catches[0].1 = 59;
    md.base.scope.protected.pop();
    assert!(allocation_proofs::prove(&ir, &[&md], &AllocationResolver).is_empty());
    let (ir, md, _) = fixture(true);
    assert!(allocation_proofs::prove(&ir, &[&md], &AllocationResolver).is_empty());
}
#[test]
fn canonical_exception_plan_installs_proof_only_at_emission() {
    let (ir, md, _) = fixture(false);
    let method = JavaMethod {
        owner: "Example".into(),
        name: "run".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![],
        throws: vec![],
    };
    let result = emit_method_region(
        &ir,
        &method,
        &DeclarationBindings::default(),
        &AllocationResolver,
        &md,
    );
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert!(source.contains("new java.lang.StringBuilder()"));
    assert!(source.contains("catch (java.lang.RuntimeException"));
    let bare = emit_method_region(
        &ir,
        &method,
        &DeclarationBindings::default(),
        &AllocationResolver,
        &md.base,
    );
    assert!(matches!(bare, SourceResult::Unsupported(_)));
}

#[test]
fn unequal_original_prethrow_phi_transfers_and_alias_sources_reject() {
    for mode in [1, 2] {
        let (ir, md, _) = fixture_phi(false, mode);
        assert!(allocation_proofs::prove(&ir, &[&md], &AllocationResolver).is_empty());
    }
}

struct RuntimeResolver;
impl JavaResolver for RuntimeResolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: if r == MethodRef::new(0) {
                "RegionRuntime.Item"
            } else {
                "RegionRuntime"
            }
            .into(),
            name: if r == MethodRef::new(0) {
                "<init>"
            } else {
                "captureLog"
            }
            .into(),
            is_static: r != MethodRef::new(0),
            return_type: JavaType::Void,
            parameters: if r == MethodRef::new(0) {
                vec![]
            } else {
                vec![JavaType::Class("java.lang.RuntimeException".into())]
            },
            throws: vec![],
        })
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r == ClassRef::new(0) {
                "RegionRuntime.Item"
            } else {
                "java.lang.RuntimeException"
            }
            .into(),
        )
    }
    fn field(&self, _: FieldRef) -> Option<crate::JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        Resolver.assignable(a, b)
    }
}
#[test]
fn original_scope_pair_preserves_constructor_effects_and_throw_identity_jdk() {
    let mut b = Builder::new();
    let new = b.cfg.entry();
    let allocated = b.value(new, ManagedType::Reference(ClassRef::new(0)));
    b.set(
        new,
        ManagedInsn::NewInstance {
            result: allocated,
            class: ClassRef::new(0),
        },
    );
    let alias = b.block();
    let copied = b.value(alias, ManagedType::Reference(ClassRef::new(0)));
    b.set(
        alias,
        ManagedInsn::Move {
            result: copied,
            value: allocated,
        },
    );
    b.link(new, alias);
    let ctor = b.block();
    b.set(
        ctor,
        ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(0),
            args: vec![copied],
        },
    );
    b.link(alias, ctor);
    let normal = b.block();
    b.link(ctor, normal);
    let capture = b.block();
    let caught = b.value(capture, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        capture,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![caught],
            effects: EffectFlags::NONE,
        },
    );
    let log = b.block();
    b.set(
        log,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(1),
            args: vec![caught],
        },
    );
    b.link(capture, log);
    let end = b.block();
    b.link(log, end);
    b.edge(new, capture, EdgeKind::Exception("broad".into()));
    b.edge(ctor, capture, EdgeKind::Exception("broad".into()));
    let ids = [new, ctor, log].map(|id| *b.cfg.block(id).unwrap().payload());
    let md = OriginalMetadata {
        base: Metadata {
            scope: scopes::RegionScope {
                protected: vec![new, alias, ctor],
                catches: vec![scopes::Catch {
                    key: "broad".into(),
                    entry: capture,
                }],
                continuation: Some(normal),
            },
            hierarchy: hierarchy(),
            all: false,
        },
        original: OriginalScope {
            ordinal: 0,
            start: 43,
            end: 69,
            handler_offset: 1,
            catches: vec![("broad".into(), 59)],
            offsets: HashMap::from([(ids[0], 43), (ids[1], 45), (ids[2], 59)]),
        },
    };
    let ir = b.finish();
    let method = JavaMethod {
        owner: "RegionRuntime".into(),
        name: "generated".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![],
        throws: vec![],
    };
    let result = emit_method_region(
        &ir,
        &method,
        &DeclarationBindings::default(),
        &RuntimeResolver,
        &md,
    );
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    run_java(&format!(
        r#"public class RegionRuntime {{
      static boolean fail; static int effects, logs; static Throwable seen;
      static final RuntimeException original = new RuntimeException("original");
      static class Item {{ Item() {{ effects++; if(fail)throw original; }} }}
      static void captureLog(RuntimeException caught) {{ logs++; seen=caught; }}
      static void reference() {{ try {{ new Item(); }} catch(RuntimeException caught) {{captureLog(caught);}} }}
      {source}
      public static void main(String[] args) {{for(boolean mode:new boolean[]{{false,true}}) {{
        fail=mode;effects=logs=0;seen=null;reference();int expectedEffects=effects,expectedLogs=logs;Throwable expected=seen;
        effects=logs=0;seen=null;generated();if(effects!=expectedEffects||logs!=expectedLogs||seen!=expected)throw new AssertionError();
      }} System.out.println("PASS original constructor effects and thrown identity"); }}
    }}"#
    ));
}
