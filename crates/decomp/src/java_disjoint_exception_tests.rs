#[test]
#[ignore = "requires local Samsung Pass APK; bounded exact methods only"]
fn samsung_disjoint_scopes_actual_graphs() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
    let files = dex_format::apk::parse_apk_path(path).unwrap();
    let dex = &files[0];
    for index in [9301, 55648] {
        let original = dex
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
            .find(|m| m.method_idx == index)
            .unwrap();
        let code = original.code.as_ref().unwrap();
        let ssa = dex_decompiler::DexPasses::run_ssa_with(
            code,
            &dex_decompiler::DexTypes::new(dex, Some(index)),
        )
        .unwrap();
        let md = from_dex_disjoint(&ssa, code, &Names(dex), &jdk_hierarchy()).unwrap();
        assert_eq!(md.scopes.len(), 2);
        let tree = backend_java::exceptions::scopes::structure_disjoint(
            ssa.ir().cfg(),
            &md.scopes
                .iter()
                .map(|m| m.scope.clone())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        fn count(regions: &[backend_java::exceptions::scopes::DisjointRegion]) -> usize {
            regions
                .iter()
                .map(|r| match r {
                    backend_java::exceptions::scopes::DisjointRegion::Try(_) => 1,
                    backend_java::exceptions::scopes::DisjointRegion::If {
                        then_body,
                        else_body,
                        ..
                    } => count(then_body) + count(else_body),
                    _ => 0,
                })
                .sum()
        }
        assert_eq!(count(&tree.body), 2);
        println!(
            "Samsung original method {index}: two disjoint scoped tries; full single-placement CFG coverage"
        );
    }
}
