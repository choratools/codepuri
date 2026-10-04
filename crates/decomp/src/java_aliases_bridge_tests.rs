#![allow(clippy::unwrap_used)]
use super::*;
use crate::java_aliases::{AliasInventory, AliasMode};

#[test]
#[ignore = "explicit JDK17 descriptor-preserving bridge bytecode/runtime fixture"]
fn original_return_only_descriptors_preserve_virtual_dispatch() {
    use std::process::Command;
    let root = std::env::temp_dir().join(format!("bridge-alias-dispatch-{}", std::process::id()));
    let java = std::path::Path::new("/home/jtjisgod/tools/jdk-17.0.13+11/bin");
    for directory in ["source", "aliased", "original", "tool"] {
        std::fs::create_dir_all(root.join(directory)).unwrap();
    }
    std::fs::write(
        root.join("source/DispatchBoundary.java"),
        include_str!("java_aliases_bridge_fixture.java"),
    )
    .unwrap();
    std::fs::write(
        root.join("tool/RenameOriginal.java"),
        include_str!("java_aliases_bridge_transform.java"),
    )
    .unwrap();
    let run = |program: &str, args: Vec<String>| {
        let result = Command::new(java.join(program))
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        result.stdout
    };
    run(
        "javac",
        vec![
            "-d".into(),
            root.join("aliased").display().to_string(),
            root.join("source/DispatchBoundary.java")
                .display()
                .to_string(),
        ],
    );
    let export = "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED";
    run(
        "javac",
        vec![
            "--add-exports".into(),
            export.into(),
            "-d".into(),
            root.join("tool").display().to_string(),
            root.join("tool/RenameOriginal.java").display().to_string(),
        ],
    );
    run(
        "java",
        vec![
            "--add-exports".into(),
            export.into(),
            "-cp".into(),
            root.join("tool").display().to_string(),
            "RenameOriginal".into(),
            root.join("aliased").display().to_string(),
            root.join("original").display().to_string(),
        ],
    );
    let execute = |directory: &str| {
        run(
            "java",
            vec![
                "-cp".into(),
                root.join(directory).display().to_string(),
                "DispatchBoundary".into(),
            ],
        )
    };
    let original = execute("original");
    assert_eq!(original, execute("aliased"));
    assert_eq!(
        String::from_utf8(original).unwrap(),
        "Parent:typed:typed:true:TT\nChild:typed:typed:true:TT\nGrandChild:virtual:typed:false:GT\n"
    );
}
fn fixture() -> (
    Vec<DexFile>,
    JavaAliases,
    crate::java_external_metadata::Classpath,
    Hierarchy,
) {
    let files = dex_format::apk::parse_apk_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
    )
    .unwrap();
    let cp = crate::java_external_metadata::Classpath::from_paths([
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod",
        "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/jdk.unsupported.jmod",
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.naming.jmod",
    ])
    .unwrap();
    let mut inventory = AliasInventory::default();
    for dex in &files {
        for t in &dex.type_ids {
            inventory
                .add_type(dex.string(t.descriptor_idx).unwrap())
                .unwrap();
        }
        for i in 0..dex.method_ids.len() {
            inventory.add_method(key(dex, i as u32).unwrap()).unwrap();
        }
    }
    let h =
        crate::java_hierarchy::build_hierarchy(&files.iter().collect::<Vec<_>>(), cp.hierarchy())
            .unwrap();
    (files, inventory.build(AliasMode::Ascii), cp, h)
}
#[test]
#[ignore = "genuine Samsung DEX bridge family aliases with ordered JDK17/Android35 metadata"]
fn actual_covariant_bridge_family_is_coherent() {
    let (files, mut aliases, cp, h) = fixture();
    let original_aliases = aliases.clone();
    let parent_typed = key(&files[1], 10970).unwrap();
    let parent_bridge = key(&files[1], 10971).unwrap();
    let child = key(&files[1], 10879).unwrap();
    let interface = MethodKey {
        owner: "Lcom/google/protobuf/MessageLite;".into(),
        name: child.name.clone(),
        parameters: vec![],
        return_type: child.return_type.clone(),
    };
    for k in [&parent_typed, &parent_bridge, &child, &interface] {
        println!("BEFORE {k:?} {:?}", aliases.method_name(k));
    }
    let input = Input::collect(&files).unwrap();
    assert_eq!(input.seed(&parent_bridge, &h), Some(parent_typed.clone()));
    let expected = java_class(&interface.owner).unwrap();
    for (owner, (_, class)) in &input.owners {
        if class.access_flags & (0x200 | 0x400) == 0 {
            let relation = h.assignable(&java_class(owner).unwrap(), &expected);
            if matches!(relation, Compatibility::Indeterminate(_)) {
                println!("FIRST_UNAVAILABLE_IMPLEMENTOR_RELATION {owner} {relation:?}");
                break;
            }
        }
    }
    for k in aliases.methods.keys().filter(|k| {
        k.name == parent_bridge.name
            && k.parameters.is_empty()
            && k.return_type != parent_bridge.return_type
    }) {
        if input
            .ancestors(&k.owner, &cp)
            .is_some_and(|a| a.contains(&interface.owner))
        {
            println!("RELATED_TYPED_OPERAND {k:?} {:?}", aliases.method_name(k));
        }
    }
    let direct = family(
        &aliases,
        &input,
        &parent_bridge,
        &parent_typed,
        &cp,
        &h,
        &mut FamilyIndex::new(&aliases),
    );
    println!("DIRECT {direct:?}");
    let alignment_started = std::time::Instant::now();
    let report = aliases.align_final_bridge_targets(&files, &cp, &h);
    println!("ALIGNMENT_WALL {:?}", alignment_started.elapsed());
    println!("REPORT {report:?}");
    for k in [&parent_typed, &parent_bridge, &child, &interface] {
        println!("AFTER {k:?} {:?}", aliases.method_name(k));
    }
    assert_eq!(
        aliases.method_name(&parent_bridge),
        aliases.method_name(&child)
    );
    assert_eq!(
        aliases.method_name(&parent_bridge),
        aliases.method_name(&interface)
    );
    assert_ne!(
        aliases.method_name(&parent_typed),
        aliases.method_name(&interface)
    );
    assert!(
        report
            .families
            .iter()
            .any(|f| f.contains(&parent_typed) && f.contains(&parent_bridge))
    );
    assert_eq!(
        aliases.method_name(&parent_bridge),
        Some(&MethodAlias::Ordinary(parent_bridge.name.clone()))
    );
    assert_eq!(
        aliases.method_name(&parent_typed),
        Some(&MethodAlias::Ordinary("newBuilderForType__2".into()))
    );
    assert!(
        !report
            .changes
            .iter()
            .any(|(k, _, _)| k == &child || k == &interface)
    );
    let super_index =
        crate::java_super_resolution::SuperIndex::collect(&files.iter().collect::<Vec<_>>(), &cp)
            .unwrap();
    let before = crate::java_super_resolution::select_super(
        &child.owner,
        &parent_typed,
        &super_index,
        &original_aliases,
        &h,
    );
    println!("SUPER_BEFORE {before:?}");
    assert!(before.is_err());
    let selected = crate::java_super_resolution::select_super(
        &child.owner,
        &parent_typed,
        &super_index,
        &aliases,
        &h,
    )
    .unwrap();
    println!("SUPER_AFTER {selected:?}");
    assert_eq!(selected.target, parent_typed);
    let original =
        crate::java_super_resolution::SuperMetadata::class(&super_index, &parent_typed.owner)
            .unwrap();
    assert_eq!(
        selected.metadata.exceptions,
        original
            .methods
            .iter()
            .find(|m| m.key == parent_typed)
            .unwrap()
            .metadata
            .exceptions
    );
    struct Unattested<'a>(&'a crate::java_super_resolution::SuperIndex<'a>);
    impl crate::java_super_resolution::SuperMetadata for Unattested<'_> {
        fn class(&self, owner: &str) -> Option<crate::java_super_resolution::SuperClass> {
            crate::java_super_resolution::SuperMetadata::class(self.0, owner)
        }
    }
    assert!(
        crate::java_super_resolution::select_super(
            &child.owner,
            &parent_typed,
            &Unattested(&super_index),
            &aliases,
            &h
        )
        .is_err()
    );
    let raw_super = input.methods[&child].1.code.as_ref().unwrap();
    assert_eq!(
        u16::from_le_bytes([raw_super.insns[0], raw_super.insns[1]]),
        0x106f
    );
    assert_eq!(
        u16::from_le_bytes([raw_super.insns[2], raw_super.insns[3]]),
        10970
    );
    let raw = &input.methods[&child].1;
    assert_eq!(raw.access_flags, 0x1041);
    // The original bodies, descriptor inventory and access flags are evidence,
    // not merely naming hints. Each changed proof must close the whole family.
    for invalid_flags in [0x1049, 0x1042, 0x1141] {
        let mut invalid = (*raw).clone();
        invalid.access_flags = invalid_flags;
        let mut changed = Input::collect(&files).unwrap();
        changed
            .methods
            .insert(child.clone(), (input.methods[&child].0, &invalid));
        assert!(
            family(
                &original_aliases,
                &changed,
                &parent_bridge,
                &parent_typed,
                &cp,
                &h,
                &mut FamilyIndex::new(&original_aliases)
            )
            .is_none()
        );
    }
    let mut invalid_bridge = input.methods[&parent_bridge].1.clone();
    invalid_bridge.code.as_mut().unwrap().insns[0] ^= 1;
    let mut changed = Input::collect(&files).unwrap();
    changed.methods.insert(
        parent_bridge.clone(),
        (input.methods[&parent_bridge].0, &invalid_bridge),
    );
    assert!(changed.seed(&parent_bridge, &h).is_none());
    let mut invalid_receiver = input.methods[&parent_bridge].1.clone();
    let code = invalid_receiver.code.as_mut().unwrap();
    code.registers_size = 17;
    // 35c C is four bits. Word 0x0010 encodes C=0,D=1, not v16.
    code.insns[4..6].copy_from_slice(&16u16.to_le_bytes());
    let mut changed = Input::collect(&files).unwrap();
    changed.methods.insert(
        parent_bridge.clone(),
        (input.methods[&parent_bridge].0, &invalid_receiver),
    );
    assert!(changed.seed(&parent_bridge, &h).is_none());
    let mut missing = Input::collect(&files).unwrap();
    missing.methods.remove(&parent_bridge);
    assert!(
        family(
            &original_aliases,
            &missing,
            &parent_bridge,
            &parent_typed,
            &cp,
            &h,
            &mut FamilyIndex::new(&original_aliases)
        )
        .is_none()
    );
}
