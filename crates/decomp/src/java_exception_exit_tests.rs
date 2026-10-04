#[test]
#[ignore = "requires local Samsung Pass APK; bounded exact exception adapters"]
fn samsung_control_only_exit_bridges() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
    let files = dex_format::apk::parse_apk_path(path).unwrap();
    let dex = &files[0];
    for index in [
        81, 122, 137, 154, 167, 441, 591, 594, 595, 596, 598, 732, 786, 793, 5174, 5176, 5792,
        5794, 5848,
    ] {
        let method = dex
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
            .find(|m| m.method_idx == index)
            .unwrap();
        let code = method.code.as_ref().unwrap();
        let ssa = dex_decompiler::DexPasses::run_ssa_with(
            code,
            &dex_decompiler::DexTypes::new(dex, Some(index)),
        )
        .unwrap();
        let result = from_dex(&ssa, code, &Names(dex), &jdk_hierarchy());
        if [81, 732, 786, 793, 5848].contains(&index) {
            assert!(
                result.is_ok(),
                "original Samsung method {index}: {result:?}"
            );
        }
        match result {
            Ok(md) => println!("method={index} accepted protected={:?}", md.scope.protected),
            Err(e) => println!("method={index} rejected={e}"),
        }
    }
}

#[test]
fn terminal_return_bridges_compile_run_and_exclude_handler_shared_return() {
    use dex_format::{
        RawSpan,
        code::{EncodedCatchHandler, TryItem, TypeAddrPair},
    };
    struct Types;
    impl dex_decompiler::TypeContext for Types {
        fn enclosing_return(&self) -> Option<managed_ir::types::ManagedType> {
            Some(managed_ir::types::ManagedType::Int)
        }
    }
    struct Resolver;
    impl JavaResolver for Resolver {
        fn class(&self, _: ClassRef) -> Option<String> {
            Some("java.lang.ArithmeticException".into())
        }
        fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
            jdk_hierarchy().assignable(a, b)
                == backend_java::type_compatibility::Compatibility::Assignable
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
    }
    // The try range ends before both return instructions. They evaluate
    // already-materialized registers and introduce no catchable effects.
    let units = [
        0x0013u16, 10, 0xf112, 0x0238, 5, 0x0093, 0x0200, 0x000f, 0x010f, 0x010d, 0xe012, 0x000f,
    ];
    let span = RawSpan { offset: 0, size: 0 };
    let code = CodeItem {
        registers_size: 3,
        ins_size: 1,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
        tries: vec![TryItem {
            start_addr: 3,
            insn_count: 4,
            handler_off: 1,
            span,
        }],
        handlers: vec![EncodedCatchHandler {
            relative_offset: 1,
            handlers: vec![TypeAddrPair {
                type_idx: 1,
                address: 9,
            }],
            catch_all_addr: None,
            span,
        }],
        span,
    };
    let ssa = dex_decompiler::DexPasses::run_ssa_with(&code, &Types).unwrap();
    let metadata = from_dex(&ssa, &code, &Resolver, &jdk_hierarchy()).unwrap();
    assert_eq!(
        metadata.catch_names().collect::<Vec<_>>(),
        ["java.lang.ArithmeticException"]
    );
    let method = JavaMethod {
        owner: "RawTerminal".into(),
        name: "divide".into(),
        is_static: true,
        return_type: JavaType::Int,
        parameters: vec![JavaType::Int],
        throws: vec![],
    };
    let bindings = crate::dex_java::parameter_bindings(ssa.ir(), &method, 3, 1).unwrap();
    let result = backend_java::exceptions::emit_method_region(
        ssa.ir(),
        &method,
        &backend_java::DeclarationBindings {
            receiver: None,
            parameters: bindings.parameters.into_iter().map(Some).collect(),
            names: vec![],
        },
        &Resolver,
        &metadata,
    );
    let backend_java::SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}");
    };
    let source = format!(
        "public class RawTerminal {{ {source} public static void main(String[] a) {{if(divide(1)!=10 || divide(0)!=-1 || divide(2)!=5 || divide(-2)!=-5) throw new AssertionError();System.out.println(\"PASS raw branching DEX scope\");}} }}"
    );
    let root = std::env::temp_dir().join(format!(
        "decomphify-terminal-exception-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("RawTerminal.java"), &source).unwrap();
    let compiled = std::process::Command::new("javac")
        .arg("RawTerminal.java")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let run = std::process::Command::new("java")
        .args(["-cp", ".", "RawTerminal"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let mut bad = code.clone();
    bad.handlers[0].handlers[0].address = 10;
    assert!(
        from_dex(&ssa, &bad, &Resolver, &jdk_hierarchy())
            .unwrap_err()
            .contains("encoded handler address")
    );
    // An outside effectful operation cannot be used as a source bridge.
    let mut effectful = code.clone();
    effectful.insns[14..16].copy_from_slice(&0x001eu16.to_le_bytes());
    let effectful_ssa = dex_decompiler::DexPasses::run_ssa_with(&effectful, &Types).unwrap();
    assert!(
        from_dex(&effectful_ssa, &effectful, &Resolver, &jdk_hierarchy())
            .unwrap_err()
            .contains("multiple normal exits")
    );
    // A handler joining a normal success Return must not make that Return
    // part of the try. In particular it cannot read the post-divide value.
    let mut shared = code.clone();
    shared.insns[22..24].copy_from_slice(&0xfc28u16.to_le_bytes());
    let shared_ssa = dex_decompiler::DexPasses::run_ssa_with(&shared, &Types).unwrap();
    let shared_metadata = from_dex(&shared_ssa, &shared, &Resolver, &jdk_hierarchy()).unwrap();
    let shared_return = shared_ssa
        .ir()
        .cfg()
        .block_ids()
        .find(|b| {
            let id = *shared_ssa.ir().cfg().block(*b).unwrap().payload();
            shared_ssa.offset(id) == Some(7)
        })
        .unwrap();
    assert!(
        !shared_metadata.scope.protected.contains(&shared_return),
        "handler-shared return must stay outside the protected scope"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "local Samsung Pass exact high-fanout exception witnesses"]
fn samsung_high_fanout_exit_inventory() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
    let files = dex_format::apk::parse_apk_path(path).unwrap();
    for (pool, index) in [(1, 39652), (0, 60314)] {
        let dex = &files[pool];
        let method = dex
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
            .find(|m| m.method_idx == index)
            .unwrap();
        let code = method.code.as_ref().unwrap();
        if pool == 0 {
            println!(
                "B1 callee {:?}",
                crate::java_descriptors::method_from_pool(dex, 46430, false)
            );
        }
        println!(
            "POOL {pool} METHOD {index} TRIES {:?} HANDLERS {:?}",
            code.tries, code.handlers
        );
        let ssa = dex_decompiler::DexPasses::run_ssa_with(
            code,
            &dex_decompiler::DexTypes::new(dex, Some(index)),
        )
        .unwrap();
        let adapter = from_dex(&ssa, code, &Names(dex), &jdk_hierarchy());
        if pool == 1 {
            assert!(adapter.is_ok(), "launchActivity: {adapter:?}");
        }
        if pool == 0 {
            assert!(code.tries.is_empty(), "B1 is not an exception-scope case");
        }
        println!("ADAPTER {adapter:?}");
        for b in ssa.ir().cfg().block_ids() {
            let id = *ssa.ir().cfg().block(b).unwrap().payload();
            println!(
                "BLOCK {b:?} OFF {:?} {:?}",
                ssa.offset(id),
                ssa.ir().instruction(id).unwrap()
            );
            for e in ssa.ir().cfg().block(b).unwrap().outgoing() {
                println!("EDGE {:?}", ssa.ir().cfg().edge(*e).unwrap());
            }
        }
    }
}

#[test]
fn outside_conditional_entry_and_in_range_handler_scaffold_preserve_catches() {
    use dex_format::{
        RawSpan,
        code::{EncodedCatchHandler, TryItem, TypeAddrPair},
    };
    struct Types;
    impl dex_decompiler::TypeContext for Types {
        fn enclosing_return(&self) -> Option<managed_ir::types::ManagedType> {
            Some(managed_ir::types::ManagedType::Int)
        }
    }
    struct Resolver;
    impl JavaResolver for Resolver {
        fn class(&self, _: ClassRef) -> Option<String> {
            Some("java.lang.ArithmeticException".into())
        }
        fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
            jdk_hierarchy().assignable(a, b)
                == backend_java::type_compatibility::Compatibility::Assignable
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
    }
    // The try range ends before both return instructions. They evaluate
    // already-materialized registers and introduce no catchable effects.
    let units = [
        0x0013u16, 10, 0xf112, 0x0238, 7, 0x0093, 0x0200, 0x000f, 0x010d, 0x0428, 0x0093, 0x0200,
        0x000f, 0xe012, 0x000f,
    ];
    let span = RawSpan { offset: 0, size: 0 };
    let code = CodeItem {
        registers_size: 3,
        ins_size: 1,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
        tries: vec![TryItem {
            start_addr: 5,
            insn_count: 8,
            handler_off: 1,
            span,
        }],
        handlers: vec![EncodedCatchHandler {
            relative_offset: 1,
            handlers: vec![TypeAddrPair {
                type_idx: 1,
                address: 8,
            }],
            catch_all_addr: None,
            span,
        }],
        span,
    };
    let ssa = dex_decompiler::DexPasses::run_ssa_with(&code, &Types).unwrap();
    let metadata = from_dex(&ssa, &code, &Resolver, &jdk_hierarchy()).unwrap();
    assert_eq!(
        metadata.catch_names().collect::<Vec<_>>(),
        ["java.lang.ArithmeticException"]
    );
    let method = JavaMethod {
        owner: "RawCoveredHandler".into(),
        name: "divide".into(),
        is_static: true,
        return_type: JavaType::Int,
        parameters: vec![JavaType::Int],
        throws: vec![],
    };
    let bindings = crate::dex_java::parameter_bindings(ssa.ir(), &method, 3, 1).unwrap();
    let result = backend_java::exceptions::emit_method_region(
        ssa.ir(),
        &method,
        &backend_java::DeclarationBindings {
            receiver: None,
            parameters: bindings.parameters.into_iter().map(Some).collect(),
            names: vec![],
        },
        &Resolver,
        &metadata,
    );
    let backend_java::SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}");
    };
    let source = format!(
        "public class RawCoveredHandler {{ {source} public static void main(String[] a) {{if(divide(1)!=10 || divide(0)!=-2 || divide(2)!=5 || divide(-2)!=-5) throw new AssertionError();System.out.println(\"PASS raw branching DEX scope\");}} }}"
    );
    let root = std::env::temp_dir().join(format!(
        "decomphify-covered-handler-exception-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("RawCoveredHandler.java"), &source).unwrap();
    let compiled = std::process::Command::new("javac")
        .arg("RawCoveredHandler.java")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let run = std::process::Command::new("java")
        .args(["-cp", ".", "RawCoveredHandler"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let md = from_dex(&ssa, &code, &Resolver, &jdk_hierarchy()).unwrap();
    for b in ssa.ir().cfg().block_ids() {
        let id = *ssa.ir().cfg().block(b).unwrap().payload();
        match ssa.offset(id) {
            Some(3) => assert!(md.scope.protected.contains(&b)),
            Some(8 | 9) => assert!(!md.scope.protected.contains(&b)),
            _ => {}
        }
    }
    let mut effectful_handler = code.clone();
    effectful_handler.insns[18..20].copy_from_slice(&0x011eu16.to_le_bytes());
    let bad_ssa = dex_decompiler::DexPasses::run_ssa_with(&effectful_handler, &Types).unwrap();
    assert!(
        from_dex(&bad_ssa, &effectful_handler, &Resolver, &jdk_hierarchy()).is_err(),
        "an effectful in-range handler instruction must not be carved out"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "local Samsung Pass shared void terminal witness"]
fn samsung_result_receiver_shared_void_terminal() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
    let files = dex_format::apk::parse_apk_path(path).unwrap();
    let dex = &files[0];
    let method = dex
        .classes
        .iter()
        .filter_map(|c| c.class_data.as_ref())
        .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
        .find(|m| m.method_idx == 3032)
        .unwrap();
    let code = method.code.as_ref().unwrap();
    assert_eq!(code.tries[0].start_addr, 25);
    assert_eq!(code.tries[0].insn_count, 3);
    let ssa = dex_decompiler::DexPasses::run_ssa_with(
        code,
        &dex_decompiler::DexTypes::new(dex, Some(3032)),
    )
    .unwrap();
    let actual = from_dex_disjoint(&ssa, code, &Names(dex), &jdk_hierarchy());
    assert!(actual.is_ok(), "existing single-scope disjoint: {actual:?}");
    let routed = from_dex(&ssa, code, &Names(dex), &jdk_hierarchy()).unwrap();
    assert!(backend_java::exceptions::shared_void_terminal(
        ssa.ir(),
        &routed.scope
    ));
}

#[test]
#[ignore = "local Samsung Pass exact shared effectful typed handler"]
fn samsung_passoperation_shared_remote_exception_handler() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
    let files = dex_format::apk::parse_apk_path(path).unwrap();
    let dex = &files[1];
    let method = dex
        .classes
        .iter()
        .filter_map(|c| c.class_data.as_ref())
        .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
        .find(|m| m.method_idx == 16814)
        .unwrap();
    let code = method.code.as_ref().unwrap();
    assert_eq!(
        (code.tries[0].start_addr, code.tries[0].insn_count),
        (43, 26)
    );
    assert_eq!(
        (code.tries[1].start_addr, code.tries[1].insn_count),
        (73, 86)
    );
    let ssa = dex_decompiler::DexPasses::run_ssa_with(
        code,
        &dex_decompiler::DexTypes::new(dex, Some(16814)),
    )
    .unwrap();
    let metadata = from_dex_disjoint(&ssa, code, &Names(dex), &jdk_hierarchy()).unwrap();
    let scopes = metadata
        .scopes
        .iter()
        .map(|m| m.scope.clone())
        .collect::<Vec<_>>();
    assert!(backend_java::exceptions::scopes::structure_disjoint(ssa.ir().cfg(), &scopes).is_err());
    let plan = backend_java::exceptions::scopes::structure_disjoint_shared_handlers(
        ssa.ir().cfg(),
        &scopes,
    )
    .unwrap();
    assert_eq!(plan.scopes[0].handlers, plan.scopes[1].handlers);
    assert_eq!(scopes[0].catches[0].entry, scopes[1].catches[0].entry);
}
