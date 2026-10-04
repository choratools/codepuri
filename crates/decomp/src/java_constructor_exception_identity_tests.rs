//! Exception provenance belongs to a particular owned constructor SSA arena.
#![allow(clippy::unwrap_used)]
use super::*;
use backend_java::constructors::ConstructorResolver;

#[test]
fn constructor_scopes_do_not_cross_equal_arena_ids_or_uncached_runs() {
    let dex = DexFile::from_bytes(&index_tests::fixture::minimal_dex()).unwrap();
    let aliases = AliasInventory::default().build(AliasMode::Unicode);
    let metadata = dex_decompiler::infer::DexMetadata::new(&dex);
    let resolver = Resolver::new(&dex, &aliases, &metadata);
    let code = dex_format::code::CodeItem {
        registers_size: 0,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: 1,
        insns: vec![0x0e, 0],
        tries: vec![],
        handlers: vec![],
        span: dex_format::RawSpan { offset: 0, size: 0 },
    };
    let context = dex_decompiler::DexTypes::with_metadata(&metadata, None);
    let first = Arc::new(dex_decompiler::DexPasses::run_ssa_with(&code, &context).unwrap());
    let second = Arc::new(dex_decompiler::DexPasses::run_ssa_with(&code, &context).unwrap());
    assert_eq!(first.ir().cfg().entry(), second.ir().cfg().entry());
    let store = |ssa: Arc<dex_decompiler::ssa::SsaMethod>| {
        resolver
            .constructor_exception_scopes
            .lock()
            .unwrap()
            .entry(5)
            .or_default()
            .push(Arc::new(StoredConstructorException {
                ssa,
                // This fixture tests provenance association only. Empty scopes
                // do not authorize exception emission; the backend rejects them.
                metadata: crate::java_exception_regions::DisjointMetadata { scopes: vec![] },
            }));
    };
    store(first.clone());
    assert!(
        resolver
            .constructor_exception_metadata(MethodRef::new(5), first.ir())
            .is_some()
    );
    assert!(
        resolver
            .constructor_exception_metadata(MethodRef::new(5), second.ir())
            .is_none()
    );
    assert!(
        resolver
            .constructor_exception_metadata(MethodRef::new(6), first.ir())
            .is_none()
    );
    store(second.clone());
    assert!(
        resolver
            .constructor_exception_metadata(MethodRef::new(5), first.ir())
            .is_some()
    );
    assert!(
        resolver
            .constructor_exception_metadata(MethodRef::new(5), second.ir())
            .is_some()
    );
}
