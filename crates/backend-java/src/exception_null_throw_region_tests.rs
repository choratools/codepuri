// A proven null Throw is NPE in both exact-region and dispatch emission.
use super::*;

struct NullMetadata(Metadata);
impl RegionMetadata for NullMetadata {
    fn region_scope(&self) -> &scopes::RegionScope {
        &self.0.scope
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.0.hierarchy
    }
    fn catch_type(&self, _: &str) -> Option<JavaType> {
        Some(JavaType::Class("java.lang.NullPointerException".into()))
    }
    fn is_catch_all(&self, _: &str) -> bool {
        false
    }
}

fn fixture(nonzero: bool) -> (MethodIr<SsaForm>, NullMetadata) {
    let mut b = Builder::new();
    let entry = b.cfg.entry();
    let zero = b.value(entry, ManagedType::Zero);
    b.set(
        entry,
        ManagedInsn::Const {
            result: zero,
            value: if nonzero {
                Constant::Int(1)
            } else {
                Constant::Zero
            },
        },
    );
    let throw = b.block();
    b.link(entry, throw);
    b.set(throw, ManagedInsn::Throw { value: zero });
    let capture = b.block();
    let caught = b.value(capture, ManagedType::Unknown);
    b.set(
        capture,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![caught],
            effects: EffectFlags::NONE,
        },
    );
    b.edge(throw, capture, EdgeKind::Exception("npe".into()));
    let result_block = b.block();
    let result = b.value(result_block, ManagedType::Int);
    b.set(
        result_block,
        ManagedInsn::Const {
            result,
            value: Constant::Int(42),
        },
    );
    b.link(capture, result_block);
    let ret = b.block();
    b.link(result_block, ret);
    b.set(
        ret,
        ManagedInsn::Return {
            value: Some(result),
        },
    );
    let mut h = hierarchy();
    h.insert(Declaration {
        name: "java.lang.NullPointerException".into(),
        superclass: Some("java.lang.RuntimeException".into()),
        interfaces: vec![],
        is_interface: false,
    })
    .unwrap();
    let md = NullMetadata(Metadata {
        scope: scopes::RegionScope {
            protected: vec![throw],
            catches: vec![scopes::Catch {
                key: "npe".into(),
                entry: capture,
            }],
            continuation: None,
        },
        hierarchy: h,
        all: false,
    });
    (b.finish(), md)
}

fn declaration(name: &str) -> JavaMethod {
    JavaMethod {
        owner: "RegionRuntime".into(),
        name: name.into(),
        is_static: true,
        return_type: JavaType::Int,
        parameters: vec![],
        throws: vec![],
    }
}

#[test]
fn implicit_npe_reaches_original_typed_handler_in_both_emitters() {
    let (ir, md) = fixture(false);
    let bindings = DeclarationBindings::default();
    let region = emit_method_region(&ir, &declaration("nullRegion"), &bindings, &Resolver, &md);
    let dispatched = dispatch::emit_method(
        &ir,
        &declaration("nullDispatch"),
        &bindings,
        &Resolver,
        &[&md],
    );
    let mut methods = String::new();
    for output in [region, dispatched] {
        let SourceResult::Complete { source, .. } = output else {
            panic!("{output:?}")
        };
        assert!(source.contains("throw null;"), "{source}");
        assert!(!source.contains("throw (java.lang.Throwable)"));
        methods.push_str(&source);
    }
    run_java(&format!(
        "public class RegionRuntime {{ {methods} public static void main(String[] args) {{ if(nullRegion()!=42 || nullDispatch()!=42) throw new AssertionError(); System.out.print(\"PASS null-region-dispatch-ok\"); }} }}"
    ));
}

#[test]
fn nominal_zero_annotation_cannot_turn_nonzero_into_null_throw() {
    let (ir, md) = fixture(true);
    for output in [
        emit_method_region(
            &ir,
            &declaration("bad"),
            &DeclarationBindings::default(),
            &Resolver,
            &md,
        ),
        dispatch::emit_method(
            &ir,
            &declaration("bad"),
            &DeclarationBindings::default(),
            &Resolver,
            &[&md],
        ),
    ] {
        assert!(
            !matches!(output, SourceResult::Complete { .. }),
            "{output:?}"
        );
    }
}
