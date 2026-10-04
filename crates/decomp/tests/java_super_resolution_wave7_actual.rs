//! Exact original Samsung classfile/DEX binding witnesses for wave7.
#![allow(clippy::unwrap_used)]
use decomp::java_aliases::{AliasInventory, AliasMode};
use decomp::java_external_metadata::Classpath;
use decomp::java_super_resolution::*;
#[test]
#[ignore = "Requires preserved original Samsung Pass APK and local JDK/Android SDK"]
fn original_samsung_wave7_super_selection_diagnostics() {
    let dexes = dex_format::apk::parse_apk_path(
        "../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    let cp = Classpath::from_paths([
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod",
        "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/jdk.unsupported.jmod",
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.naming.jmod",
    ])
    .unwrap();
    let refs: Vec<_> = dexes.iter().collect();
    let index = SuperIndex::collect(&refs, &cp).unwrap();
    let hierarchy = decomp::java_hierarchy::build_with_metadata(&refs, &cp).unwrap();
    let mut names = AliasInventory::default();
    // Bounded witness naming catalog: exact callers and complete named-method
    // declarations on their real ancestor chains. Production supplies its own
    // complete frozen alias inventory; this probe proves metadata equivalence.
    for (pool, idx) in [
        (0, 6855),
        (0, 8843),
        (0, 8844),
        (0, 8850),
        (0, 8851),
        (0, 11810),
        (0, 26343),
        (0, 26846),
        (1, 4019),
        (3, 30573),
    ] {
        let caller = decomp::java_members::source_closure::method_key(&dexes[pool], idx).unwrap();
        names.add_type(&caller.owner).unwrap();
        let mut next = index.class(&caller.owner).unwrap().superclass;
        while let Some(owner) = next {
            names.add_type(&owner).unwrap();
            let c = index.class(&owner).unwrap();
            for m in &c.methods {
                if m.key.name == caller.name {
                    names.add_method(m.key.clone()).unwrap();
                }
            }
            next = c.superclass;
        }
    }
    let aliases = names.build(AliasMode::Unicode);
    let mut successes = 0;
    let mut checked = 0;
    for (pool, idx) in [
        (0, 6855),
        (0, 8843),
        (0, 8844),
        (0, 8850),
        (0, 8851),
        (0, 11810),
        (0, 26343),
        (0, 26846),
        (1, 4019),
        (3, 30573),
    ] {
        let d = &dexes[pool];
        let caller = decomp::java_members::source_closure::method_key(d, idx).unwrap();
        let m = d
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|c| c.direct_methods.iter().chain(&c.virtual_methods))
            .find(|m| m.method_idx == idx)
            .unwrap();
        let code = m.code.as_ref().unwrap();
        for i in dex_decompiler::decode_method_recovering(code, 2).insns {
            let dex_decompiler::Opcode::Supported(op) = i.opcode else {
                continue;
            };
            if op.opcode != 0x6f && op.opcode != 0x75 {
                continue;
            }
            let reference =
                decomp::java_members::source_closure::method_key(d, i.pool_index.unwrap()).unwrap();
            checked += 1;
            let mut next = index.class(&caller.owner).unwrap().superclass;
            while let Some(owner) = next {
                let c = index.class(&owner).unwrap();
                for m in &c.methods {
                    if m.key.name == reference.name {
                        println!(
                            "DECL {owner} {:?} flags={:x} generic={} erased={} classpath_signature={:?}",
                            m.key,
                            m.metadata.access_flags,
                            m.generic,
                            index.source_signature_is_erased(&m.key),
                            cp.class(&owner)
                                .and_then(|c| c.methods.iter().find(|x| x.key == m.key))
                                .and_then(|x| x.signature.as_ref())
                        );
                    }
                }
                next = c.superclass;
            }
            let selection = select_super(&caller.owner, &reference, &index, &aliases, &hierarchy);
            println!(
                "method={idx} caller={caller:?} code={} invoke={} reference={reference:?} selected={selection:?}",
                code.span.offset, i.offset
            );
            if selection.is_ok() {
                successes += 1;
            }
        }
    }
    println!("checked={checked} successes={successes}");
    assert_eq!(checked, 10);
    assert_eq!(successes, 10);
}
