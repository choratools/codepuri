#![allow(clippy::unwrap_used)]
use super::*;

#[test]
fn wave10_jdk_inherited_default_contract_and_runtime_oracle() {
    use std::{fs, process::Command};
    let home = std::env::var_os("JAVA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.join("bin/javac").is_file())
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join("tools/jdk-17.0.13+11")
        });
    let external =
        crate::java_external_metadata::Classpath::from_paths(&[home.join("jmods/java.base.jmod")])
            .unwrap();
    let reference = MethodKey {
        owner: "Ljava/util/zip/CRC32;".into(),
        name: "update".into(),
        parameters: vec!["[B".into()],
        return_type: "V".into(),
    };
    let inherited = MethodKey {
        owner: "Ljava/util/zip/Checksum;".into(),
        ..reference.clone()
    };
    // Missing exact declaration does not attest an empty inherited contract.
    // The genuine default declaration provides that evidence independently.
    assert!(external.method(&reference).is_none());
    let declaration = external.method(&inherited).unwrap();
    assert_eq!(declaration.access_flags, 1);
    assert_eq!(declaration.exceptions, Some(vec![]));
    let interface = external.class_hierarchy(&inherited.owner);
    assert_eq!(interface.len(), 1);
    assert!(interface[0].is_interface);
    let concrete = external.class_hierarchy(&reference.owner);
    assert_eq!(concrete.len(), 1);
    assert!(
        concrete[0]
            .interfaces
            .contains(&"java.util.zip.Checksum".into())
    );

    let dir = std::env::temp_dir().join(format!("decomp-wave10-checksum-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("ChecksumOracle.java"),
        r#"
import java.util.zip.CRC32;
import java.util.zip.Checksum;
public final class ChecksumOracle {
  public static void main(String[] args) throws Exception {
    try {
      CRC32.class.getDeclaredMethod("update", byte[].class);
      throw new AssertionError("unexpected exact declaration");
    } catch (NoSuchMethodException expected) {}
    var selected = Checksum.class.getDeclaredMethod("update", byte[].class);
    if (!selected.isDefault() || selected.getExceptionTypes().length != 0)
      throw new AssertionError("default contract");
    CRC32 crc = new CRC32();
    crc.update("123456789".getBytes(java.nio.charset.StandardCharsets.US_ASCII));
    if (crc.getValue() != 0xcbf43926L) throw new AssertionError("selected execution");
    System.out.println("inherited-default;throws=[];crc32=cbf43926");
  }
}
"#,
    )
    .unwrap();
    let compile = Command::new(home.join("bin/javac"))
        .args(["--release", "17", "ChecksumOracle.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new(home.join("bin/java"))
        .args(["-cp", ".", "ChecksumOracle"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim(),
        "inherited-default;throws=[];crc32=cbf43926"
    );
    fs::remove_dir_all(dir).unwrap();
}

type OriginalFixture = (
    Vec<DexFile>,
    JavaAliases,
    BTreeMap<MethodKey, Option<MethodMetadata>>,
    crate::java_external_metadata::Classpath,
    crate::type_compatibility::Hierarchy,
);
fn original_fixture() -> OriginalFixture {
    let files = dex_format::apk::parse_apk_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
    )
    .unwrap();
    let external = crate::java_external_metadata::Classpath::from_paths(&[
        std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod"),
        std::path::PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
        std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/jdk.unsupported.jmod"),
        std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.naming.jmod"),
    ])
    .unwrap();
    let input_refs = files.iter().collect::<Vec<_>>();
    let super_index =
        crate::java_super_resolution::SuperIndex::collect(&input_refs, &external).unwrap();
    let writes =
        initializers::write_inventory::WriteInventory::scan_with_metadata(&files, &external);
    let (mut aliases, _) =
        export_inventory(&files, &mut None, &writes, Some(&super_index)).unwrap();
    let owners = files
        .iter()
        .flat_map(|d| {
            d.classes
                .iter()
                .filter_map(|c| descriptor(d, c.class_idx).map(str::to_owned))
        })
        .collect();
    aliases.apply_external_member_names(&external.member_source_names(), &owners);
    let hierarchy = crate::java_hierarchy::build_hierarchy(&input_refs, external.hierarchy())
        .unwrap()
        .map_names(|n| {
            aliases
                .class_name(&format!("L{};", n.replace('.', "/")))
                .unwrap_or(n)
                .into()
        })
        .unwrap();
    let catalog = method_catalog(&files);
    (files, aliases, catalog, external, hierarchy)
}

#[test]
#[ignore = "bounded original wave10 source-contract diagnosis with genuine JDK17/Android35; no export"]
fn wave10_original_checked_contract_diagnosis() {
    let (files, aliases, catalog, external, hierarchy) = original_fixture();
    let input_refs = files.iter().collect::<Vec<_>>();
    let super_index =
        crate::java_super_resolution::SuperIndex::collect(&input_refs, &external).unwrap();
    let targets = [
        (0, 158),
        (0, 5339),
        (0, 12274),
        (0, 22483),
        (0, 60324),
        (1, 32438),
    ];
    let roots = targets
        .iter()
        .map(|(p, i)| method_key(&files[*p], *i).unwrap())
        .collect();
    let plan = collect_roots(
        &files,
        &aliases,
        &catalog,
        Some(&external),
        &hierarchy,
        Some(&roots),
    );
    let facts = Facts::new(&files, &aliases, &catalog, Some(&external));
    println!(
        "WAVE10 PLAN available={} unavailable={}",
        plan.methods.len(),
        plan.unavailable.len()
    );
    for (pool, index) in targets {
        let key = method_key(&files[pool], index).unwrap();
        let (_, method) = facts.definitions[&key];
        let metadata = dex_decompiler::infer::DexMetadata::new(&files[pool]);
        let mut resolver =
            Resolver::with_catalog(&files[pool], &aliases, &metadata, Some(&catalog));
        resolver.inputs = &files;
        resolver.dex_index = pool;
        resolver.external = Some(&external);
        resolver.input_owners = facts.owners.keys().cloned().collect();
        resolver.hierarchy = hierarchy.clone();
        resolver.super_index = Some(&super_index);
        let context = Context {
            resolver: &resolver,
            facts: &facts,
        };
        let original = context.invocation_contract(MethodRef::new(index)).unwrap();
        println!(
            "WAVE10 ROOT {pool}:{index} {key:?} original={:?} plan={:?} error={:?}",
            original.declared,
            plan.methods.get(&key),
            plan.unavailable.get(&key)
        );
        let identity = OriginalMethodIdentity {
            dex: pool,
            index,
            key: key.clone(),
        };
        println!(
            "WAVE10 OVERRIDES {pool}:{index} {:?}",
            context.override_contract(&identity)
        );
        let code = method.code.as_ref().unwrap();
        let (ctx, seeds) = initial_context_with_seeds(&resolver, method).unwrap();
        let ssa = cached_frontend(&resolver, method, code, &ctx, seeds, None).unwrap();
        println!(
            "WAVE10 HANDLERS {pool}:{index} {:?} {:?}",
            code.tries, code.handlers
        );
        for (id, insn) in ssa.ir().instructions() {
            if let ManagedInsn::Invoke {
                method: reference,
                kind,
                ..
            } = insn
            {
                let callee = method_key(&files[pool], reference.index()).unwrap();
                println!(
                    "WAVE10 INVOKE {pool}:{index}@{:?} {kind:?} ref{} {callee:?} contract={:?} source={:?} error={:?}",
                    ssa.offset(id),
                    reference.index(),
                    context.invoked_contract(&identity, *kind, *reference),
                    plan.methods.get(&callee).map(|v| &v.source),
                    plan.unavailable.get(&callee)
                );
            }
        }
        let mut current = key;
        let mut seen = BTreeSet::new();
        while seen.insert(current.clone()) {
            let Some(error) = plan.unavailable.get(&current) else {
                break;
            };
            println!("WAVE10 CHAIN {pool}:{index} {current:?} {error:?}");
            let required::ThrowsError::MissingSourceTarget(next) = error else {
                break;
            };
            current = next.clone();
        }
    }
}

#[test]
#[ignore = "original concrete u8/f override domain with genuine JDK17/Android35; no export"]
fn concrete_source_original_u8_cluster_and_immutable_annotation_policy() {
    let (files, aliases, catalog, external, hierarchy) = original_fixture();
    let indices = [
        60312, 60315, 60319, 60324, 60325, 60327, 60329, 60331, 60332, 60333, 60334, 60335, 60337,
        60339, 60341, 60343, 60345, 60347, 60358, 60361, 60362, 60363, 60364, 60365, 60366, 60368,
        60369, 60370,
    ];
    let mut roots = indices
        .iter()
        .map(|index| method_key(&files[0], *index).unwrap())
        .collect::<BTreeSet<_>>();
    roots.extend([62994, 62984, 5339].map(|index| method_key(&files[0], index).unwrap()));
    let before = catalog.clone();
    let plan = collect_roots(
        &files,
        &aliases,
        &catalog,
        Some(&external),
        &hierarchy,
        Some(&roots),
    );
    assert_eq!(catalog, before);
    for index in indices.into_iter().chain([62994, 62984, 5339]) {
        let key = method_key(&files[0], index).unwrap();
        println!(
            "CONCRETE ORIGINAL 0:{index} {key:?} contract={:?} error={:?}",
            plan.methods.get(&key),
            plan.unavailable.get(&key)
        );
    }
    for index in [62994, 62984, 60324] {
        let key = method_key(&files[0], index).unwrap();
        let proof = plan
            .methods
            .get(&key)
            .unwrap_or_else(|| panic!("{key:?}: {:?}", plan.unavailable.get(&key)));
        assert_eq!(proof.identity.index, index);
        assert!(proof.original.is_empty());
        assert_eq!(proof.source, vec!["java.io.IOException"]);
    }
    let denied = method_key(&files[0], 5339).unwrap();
    assert!(!plan.methods.contains_key(&denied));
    assert!(matches!(
        plan.unavailable.get(&denied),
        Some(required::ThrowsError::OverrideConflict(_))
            | Some(required::ThrowsError::MissingSourceTarget(_))
    ));

    check_original_empty_annotation_policy(&files);
}

fn check_original_empty_annotation_policy(files: &[DexFile]) {
    let (class, method) = files[0]
        .classes
        .iter()
        .find_map(|class| {
            class.class_data.as_ref().and_then(|data| {
                data.virtual_methods
                    .iter()
                    .find(|m| m.method_idx == 62994)
                    .map(|m| (class, m))
            })
        })
        .unwrap();
    assert!(concrete_source_reconstructible(&files[0], class, method));
    assert_eq!(method.code.as_ref().unwrap().insns, vec![0x0e, 0x00]);
    let raw = crate::java_members::MemberResolver { dex: &files[0] };
    assert_eq!(raw.declared_throws_for_class(class, 62994), Some(vec![]));
    let throws = files[0]
        .type_ids
        .iter()
        .position(|t| files[0].string(t.descriptor_idx).unwrap() == "Ldalvik/annotation/Throws;")
        .unwrap() as u32;
    let span = dex_format::RawSpan { offset: 0, size: 0 };
    let value_name = files[0]
        .strings
        .iter()
        .position(|s| s.value == "value")
        .unwrap() as u32;
    let mut annotated = class.clone();
    annotated.annotations_off = 1;
    let directory =
        annotated
            .annotations
            .get_or_insert(dex_format::annotations::AnnotationsDirectory {
                class_annotations: None,
                field_annotations: vec![],
                method_annotations: vec![],
                parameter_annotations: vec![],
                span,
            });
    directory
        .method_annotations
        .push(dex_format::annotations::MemberAnnotations {
            member_idx: 62994,
            annotations: dex_format::annotations::AnnotationSet {
                entries: vec![dex_format::annotations::AnnotationItem {
                    visibility: dex_format::annotations::Visibility::System,
                    annotation: dex_format::encoded_value::EncodedAnnotation {
                        type_idx: throws,
                        elements: vec![dex_format::encoded_value::AnnotationElement {
                            name_idx: value_name,
                            value: dex_format::encoded_value::EncodedValue {
                                value: dex_format::encoded_value::EncodedValueKind::Array(vec![]),
                                span,
                            },
                        }],
                        span,
                    },
                    span,
                }],
                span,
            },
        });
    assert_eq!(
        raw.declared_throws_for_class(&annotated, 62994),
        Some(vec![])
    );
    // Presence alone fixes the input contract, including explicit empty or
    // malformed Throws. Missing directories and unknown annotation types deny.
    assert!(!concrete_source_reconstructible(
        &files[0], &annotated, method
    ));
    annotated.annotations = None;
    assert!(!concrete_source_reconstructible(
        &files[0], &annotated, method
    ));
}

#[test]
#[ignore = "original explicit-empty Throws origin policy; DEX-only bounded witness"]
fn concrete_source_original_explicit_empty_annotation_policy() {
    let files = dex_format::apk::parse_apk_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
    )
    .unwrap();
    check_original_empty_annotation_policy(&files);
}

#[test]
#[ignore = "original constructor135 and method137 production source-contract witness; no export"]
fn concrete_source_original_constructor135_contract_and_emission_witness() {
    let (files, aliases, catalog, external, hierarchy) = original_fixture();
    let roots = [135, 137, 158]
        .map(|index| method_key(&files[0], index).unwrap())
        .into_iter()
        .collect();
    let plan = collect_roots(
        &files,
        &aliases,
        &catalog,
        Some(&external),
        &hierarchy,
        Some(&roots),
    );
    let facts = Facts::new(&files, &aliases, &catalog, Some(&external));
    let input_refs = files.iter().collect::<Vec<_>>();
    let super_index =
        crate::java_super_resolution::SuperIndex::collect(&input_refs, &external).unwrap();
    let metadata = dex_decompiler::infer::DexMetadata::new(&files[0]);
    let writes =
        initializers::write_inventory::WriteInventory::scan_with_metadata(&files, &external);
    let mut resolver = Resolver::with_catalog(&files[0], &aliases, &metadata, Some(&catalog));
    resolver.inputs = &files;
    resolver.dex_index = 0;
    resolver.external = Some(&external);
    resolver.hierarchy = hierarchy;
    resolver.input_owners = facts.owners.keys().cloned().collect();
    resolver.super_index = Some(&super_index);
    resolver.source_throws = Some(&plan);
    resolver.source_release = 25;
    resolver.writes = Some(&writes);
    for index in [135, 137, 158] {
        let key = method_key(&files[0], index).unwrap();
        println!(
            "CTOR CONTRACT 0:{index} {key:?} metadata={:?} source={:?} unavailable={:?}",
            catalog[&key],
            plan.methods.get(&key),
            plan.unavailable.get(&key)
        );
        let mut current = key;
        let mut seen = BTreeSet::new();
        while seen.insert(current.clone()) {
            let Some(error) = plan.unavailable.get(&current) else {
                break;
            };
            println!("CTOR CHAIN 0:{index} {current:?} {error:?}");
            let required::ThrowsError::MissingSourceTarget(next) = error else {
                break;
            };
            current = next.clone();
        }
    }
    let key = method_key(&files[0], 135).unwrap();
    assert_eq!(key.name, "<init>");
    let (_, method) = facts.definitions[&key];
    let code = method.code.as_ref().unwrap();
    let (ctx, seeds) = initial_context_with_seeds(&resolver, method).unwrap();
    let ssa = cached_frontend(&resolver, method, code, &ctx, seeds, None).unwrap();
    let signature = resolver.method(MethodRef::new(135)).unwrap();
    let bindings = declaration_bindings(&ssa, &signature, code).unwrap();
    let context = Context {
        resolver: &resolver,
        facts: &facts,
    };
    let identity = OriginalMethodIdentity {
        dex: 0,
        index: 135,
        key: key.clone(),
    };
    for (id, insn) in ssa.ir().instructions() {
        if let ManagedInsn::Invoke { method, kind, .. } = insn {
            let callee = method_key(&files[0], method.index()).unwrap();
            println!(
                "CTOR INVOKE @{:?} ref{} {kind:?} {callee:?} raw={:?} contract={:?} source={:?}",
                ssa.offset(id),
                method.index(),
                resolver.declaration(method.index()),
                context.invoked_contract(&identity, *kind, *method),
                plan.methods.get(&callee)
            );
        }
    }
    let direct = backend_java::constructors::plan_constructor_declaration(
        ssa.ir(),
        MethodRef::new(135),
        "java.lang.Object",
        &bindings,
        &resolver,
    );
    println!(
        "CTOR DIRECT {}",
        match &direct {
            Ok(body) => format!("complete source={} bytes", body.source.len()),
            Err(error) => format!("unavailable {error:?}"),
        }
    );
    if let Ok(body) = &direct {
        assert!(plan.methods.contains_key(&key));
        println!("CTOR SOURCE {}", body.source);
    }
    let class = files[0]
        .classes
        .iter()
        .find(|class| descriptor(&files[0], class.class_idx) == Some("La4/a;"))
        .unwrap();
    let mut diagnostics = BTreeMap::new();
    let family = constructor_subset_source(class, &resolver, &mut diagnostics);
    println!(
        "CTOR FAMILY status={:?} diagnostics={diagnostics:?}",
        family
            .as_ref()
            .map(|members| members.keys().copied().collect::<Vec<_>>())
            .map_err(|error| error.as_str())
    );
    if let Ok(members) = family {
        for (index, body) in members {
            println!("CTOR FAMILY SOURCE {index} {}", body.source);
        }
    }
}

#[path = "java_source_throws_wave11_loss_tests.rs"]
mod wave11_loss_tests;

#[path = "java_source_throws_original_super_tests.rs"]
mod original_super_tests;

#[path = "java_source_throws_wave12_loss_tests.rs"]
mod wave12_loss_tests;
