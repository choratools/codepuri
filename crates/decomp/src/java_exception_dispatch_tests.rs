/// Exact DEX-to-dispatch bridge proof, independent of structured source shapes.
mod dispatch_bridge_tests {
    use super::*;
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
    fn raw(units: &[u16], registers: u16, start: u32, count: u16, handler: u32) -> CodeItem {
        let span = RawSpan { offset: 0, size: 0 };
        CodeItem {
            registers_size: registers,
            ins_size: 1,
            outs_size: 0,
            debug_info_off: 0,
            insns_size: units.len() as u32,
            insns: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
            tries: vec![TryItem {
                start_addr: start,
                insn_count: count,
                handler_off: 1,
                span,
            }],
            handlers: vec![EncodedCatchHandler {
                relative_offset: 1,
                handlers: vec![TypeAddrPair {
                    type_idx: 1,
                    address: handler,
                }],
                catch_all_addr: None,
                span,
            }],
            span,
        }
    }
    fn code_loop() -> CodeItem {
        // v3 is the incoming divisor/counter. The protected division repeats
        // over a genuine SSA backedge; the original handler returns -1.
        raw(
            &[
                0x0012, 0x1112, 0x0293, 0x0301, 0x13b1, 0x033c, 0xfffd, 0x020f, 0x000d, 0xf212,
                0x020f,
            ],
            4,
            2,
            5,
            8,
        )
    }
    fn code_multi_exit() -> CodeItem {
        // Two normal exits lead to distinct effectful original instructions,
        // so the strict adapter cannot absorb their outside control bridges.
        raw(
            &[
                0x0013, 10, 0x1112, 0x0093, 0x0200, 0x0238, 5, 0x0093, 0x0100, 0x000f, 0x0093,
                0x0100, 0x000f, 0x010d, 0xf012, 0x000f,
            ],
            3,
            3,
            4,
            13,
        )
    }

    #[test]
    fn exact_raw_loop_and_multiple_exit_adapters_compile_and_run() {
        let mut sources = Vec::new();
        for (name, code) in [("loop", code_loop()), ("multi", code_multi_exit())] {
            let ssa = dex_decompiler::DexPasses::run_ssa_with(&code, &Types).unwrap();
            assert!(
                from_dex(&ssa, &code, &Resolver, &jdk_hierarchy()).is_err(),
                "strict legacy {name} shape must remain unsupported"
            );
            let md = from_dex_dispatch(&ssa, &code, &Resolver, &jdk_hierarchy()).unwrap();
            assert_eq!(md.scopes.len(), 1);
            assert_eq!(md.scopes[0].original.start, code.tries[0].start_addr);
            assert_eq!(
                md.scopes[0].original.catches,
                vec![("1".into(), code.handlers[0].handlers[0].address)]
            );
            if name == "multi" {
                assert!(md.scopes[0].scope.continuation.is_none());
            }
            let method = JavaMethod {
                owner: "ExactDispatch".into(),
                name: name.into(),
                is_static: true,
                return_type: JavaType::Int,
                parameters: vec![JavaType::Int],
                throws: vec![],
            };
            let bound = crate::dex_java::parameter_bindings(
                ssa.ir(),
                &method,
                code.registers_size,
                code.ins_size,
            )
            .unwrap();
            let bindings = backend_java::DeclarationBindings {
                receiver: None,
                parameters: bound.parameters.into_iter().map(Some).collect(),
                names: vec![],
            };
            let tables = md
                .scopes
                .iter()
                .map(|m| m as &dyn RegionMetadata)
                .collect::<Vec<_>>();
            let source = match backend_java::exceptions::dispatch::emit_method(
                ssa.ir(),
                &method,
                &bindings,
                &Resolver,
                &tables,
            ) {
                backend_java::SourceResult::Complete { source, .. } => source,
                error => panic!("{name}: {error:?}"),
            };
            sources.push(source);
            let mut bad = code.clone();
            bad.handlers[0].handlers[0].address += 1;
            assert!(
                from_dex_dispatch(&ssa, &bad, &Resolver, &jdk_hierarchy())
                    .unwrap_err()
                    .contains("handler address disagrees")
            );
            let mut overlap = code.clone();
            overlap.tries.push(overlap.tries[0].clone());
            assert!(
                from_dex_dispatch(&ssa, &overlap, &Resolver, &jdk_hierarchy())
                    .unwrap_err()
                    .contains("overlapping source ranges")
            );
            let mut bounds = code.clone();
            bounds.tries[0].insn_count = 65535;
            assert!(
                from_dex_dispatch(&ssa, &bounds, &Resolver, &jdk_hierarchy())
                    .unwrap_err()
                    .contains("outside encoded instructions")
            );
        }
        let source = format!(
            r#"public class ExactDispatch {{ {}
            public static void main(String[] args) {{
                if(loop(3)!=1 || loop(0)!=-1 || loop(-1)!=-1) throw new AssertionError("loop");
                if(multi(2)!=5 || multi(0)!=-1 || multi(-2)!=-5) throw new AssertionError("multi");
                System.out.print("exact-dispatch-bridge-ok");
            }}
        }}"#,
            sources.join("\n")
        );
        let dir = std::env::temp_dir().join(format!(
            "decomphify-exact-dispatch-bridge-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ExactDispatch.java"), &source).unwrap();
        let compile = std::process::Command::new("javac")
            .arg("--release")
            .arg("17")
            .arg("ExactDispatch.java")
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}\n{source}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let run = std::process::Command::new("java")
            .arg("-cp")
            .arg(&dir)
            .arg("ExactDispatch")
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            "exact-dispatch-bridge-ok"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "requires local Samsung Pass APK; selected original method graph proofs"]
    fn samsung_selected_original_exception_dispatch_graphs() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
        let files = dex_format::apk::parse_apk_path(path).unwrap();
        let mut gained = 0;
        for (pool, index) in [
            (0, 29),
            (0, 30),
            (0, 41),
            (0, 62),
            (0, 122),
            (0, 137),
            (0, 158),
            (0, 167),
            (0, 9301),
            (0, 55648),
        ] {
            let dex = &files[pool];
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
            let md = from_dex_dispatch(&ssa, code, &Names(dex), &jdk_hierarchy()).unwrap();
            assert_eq!(md.scopes.len(), code.tries.len());
            let legacy = from_dex_disjoint(&ssa, code, &Names(dex), &jdk_hierarchy());
            if legacy.is_err() {
                gained += 1;
            }
            println!(
                "Samsung pool={pool} method={index} scopes={} blocks={} legacy={:?}",
                md.scopes.len(),
                ssa.ir().cfg().block_count(),
                legacy.as_ref().map(|_| ()).map_err(String::as_str)
            );
        }
        assert!(
            gained > 0,
            "selected originals must include a previously unsupported region grammar"
        );
    }

    #[test]
    #[ignore = "requires local Samsung Pass APK and JDK17; one exact original parsing method"]
    fn samsung_original_jdk_parsing_method_dispatch_compiles_and_runs() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
        let files = dex_format::apk::parse_apk_path(path).unwrap();
        let mut selected = None;
        let mut candidates = 0;
        'pools: for (pool, dex) in files.iter().enumerate() {
            let resolver = Names(dex);
            for encoded in dex
                .classes
                .iter()
                .filter_map(|c| c.class_data.as_ref())
                .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
            {
                if encoded.access_flags & 8 == 0 {
                    continue;
                }
                let Some(code) = encoded.code.as_ref() else {
                    continue;
                };
                if code.tries.is_empty() {
                    continue;
                }
                let Ok(method) =
                    crate::java_descriptors::method_from_pool(dex, encoded.method_idx, true)
                else {
                    continue;
                };
                if method.return_type != JavaType::Int
                    || method.parameters
                        != [JavaType::Class("java.lang.String".into()), JavaType::Int]
                {
                    continue;
                }
                if !code.handlers.iter().flat_map(|h| &h.handlers).any(|h| {
                    resolver.class(ClassRef::new(h.type_idx)).as_deref()
                        == Some("java.lang.NumberFormatException")
                }) {
                    continue;
                }
                candidates += 1;
                assert!(candidates <= 16, "bounded parsing candidates exhausted");
                let ssa = dex_decompiler::DexPasses::run_ssa_with(
                    code,
                    &dex_decompiler::DexTypes::new(dex, Some(encoded.method_idx)),
                )
                .unwrap();
                let metadata = from_dex_dispatch(&ssa, code, &resolver, &jdk_hierarchy()).unwrap();
                let bound = crate::dex_java::parameter_bindings(
                    ssa.ir(),
                    &method,
                    code.registers_size,
                    code.ins_size,
                )
                .unwrap();
                let bindings = backend_java::DeclarationBindings {
                    receiver: None,
                    parameters: bound.parameters.into_iter().map(Some).collect(),
                    names: vec![],
                };
                let tables = metadata
                    .scopes
                    .iter()
                    .map(|m| m as &dyn RegionMetadata)
                    .collect::<Vec<_>>();
                match backend_java::exceptions::dispatch::emit_method(
                    ssa.ir(),
                    &method,
                    &bindings,
                    &resolver,
                    &tables,
                ) {
                    backend_java::SourceResult::Complete {
                        source,
                        method_name,
                        ..
                    } => {
                        println!(
                            "Samsung original parsing method pool={pool} index={} owner={} name={} exact_ranges={}",
                            encoded.method_idx,
                            method.owner,
                            method.name,
                            metadata.scopes.len()
                        );
                        selected = Some((source, method_name));
                        break 'pools;
                    }
                    error => println!(
                        "bounded Samsung parsing candidate pool={pool} index={} unsupported={error:?}",
                        encoded.method_idx
                    ),
                }
            }
        }
        let (declaration, name) = selected.unwrap();
        let source = format!(
            r#"public class SamsungDispatchSubset {{ {declaration}
            public static void main(String[] args) {{
                if({name}("123",7)!=123 || {name}(null,7)!=7 || {name}("bad",7)!=7 || {name}("2147483648",7)!=7) throw new AssertionError("original parsing behavior");
                System.out.print("Samsung-original-dispatch-ok");
            }}
        }}"#
        );
        let dir = std::env::temp_dir().join(format!(
            "decomphify-samsung-dispatch-subset-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SamsungDispatchSubset.java"), &source).unwrap();
        let compile = std::process::Command::new("javac")
            .arg("--release")
            .arg("17")
            .arg("SamsungDispatchSubset.java")
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}\n{source}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let run = std::process::Command::new("java")
            .arg("-cp")
            .arg(&dir)
            .arg("SamsungDispatchSubset")
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            "Samsung-original-dispatch-ok"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
