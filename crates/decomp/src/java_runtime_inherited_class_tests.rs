#![allow(clippy::unwrap_used)]
use super::*;
use crate::java_external_metadata::inherited_methods::InheritedMethodKind;

fn key(owner: &str) -> MethodKey {
    MethodKey {
        owner: owner.into(),
        name: "call".into(),
        parameters: vec![],
        return_type: "I".into(),
    }
}
fn owner(name: &str, parent: Option<&str>) -> DexClass {
    DexClass {
        access_flags: 1,
        declaration: Declaration {
            name: name.into(),
            superclass: parent.map(str::to_owned),
            interfaces: vec![],
            is_interface: false,
        },
        methods: vec![],
        fields: vec![],
        member: None,
        signature: None,
        code_presence: BTreeMap::new(),
        covariant_bridges: BTreeMap::new(),
    }
}
fn fixture() -> DexMetadata {
    let mut base = owner("Base", Some("java.lang.Object"));
    let method = ParsedMethod {
        key: key("LBase;"),
        metadata: MethodMetadata {
            access_flags: 1,
            exceptions: Some(vec!["Ljava/io/IOException;".into()]),
        },
        descriptor: "()I".into(),
        signature: None,
        code: None,
    };
    base.code_presence.insert(method.key.clone(), true);
    base.methods.push(method);
    DexMetadata {
        classes: BTreeMap::from([
            ("LLeaf;".into(), owner("Leaf", Some("Base"))),
            ("LBase;".into(), base),
            ("Ljava/lang/Object;".into(), owner("java.lang.Object", None)),
        ]),
    }
}
#[test]
fn runtime_class_declaration_is_distinct_and_declared_apis_stay_exact() {
    let data = fixture();
    assert!(data.method(&key("LLeaf;")).is_none());
    assert!(data.method_access(&key("LLeaf;")).is_none());
    let proof = data.inherited_public_instance(&key("LLeaf;")).unwrap();
    assert_eq!(proof.kind(), InheritedMethodKind::RuntimeClassInstance);
    assert_eq!(proof.reference(), &key("LLeaf;"));
    assert_eq!(proof.declaration().key, key("LBase;"));
    assert_eq!(
        proof.declaration().metadata.exceptions,
        Some(vec!["Ljava/io/IOException;".into()])
    );
    assert!(proof.declaration().code.is_none());
    assert_eq!(proof.owners(), ["LLeaf;", "LBase;", "Ljava/lang/Object;"]);
}

#[test]
fn frozen_chain_caches_positive_and_negative_proofs_without_cross_input_reuse() {
    let data = fixture();
    let good = key("LLeaf;");
    let mut missing = good.clone();
    missing.name = "absent".into();
    let chain = MetadataChain::new(vec![MetadataProvider::Dex(&data)]);
    let expected = data.inherited_public_instance(&good).unwrap();
    let first = chain.inherited_public_instance(&good).unwrap();
    let repeated = chain.inherited_public_instance(&good).unwrap();
    assert!(std::ptr::eq(first.declaration(), repeated.declaration()));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let chain = &chain;
            let good = &good;
            let missing = &missing;
            let expected = &expected;
            scope.spawn(move || {
                for _ in 0..64 {
                    assert_eq!(
                        chain.inherited_public_instance(good).as_ref(),
                        Some(expected)
                    );
                    assert!(chain.inherited_public_instance(missing).is_none());
                    // Declaration APIs never inherit from the memoized proof.
                    assert!(chain.method(good).is_none());
                }
            });
        }
    });
    let cache = chain.inherited.read().unwrap();
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.get(&good), Some(&Some(expected)));
    assert_eq!(cache.get(&missing), Some(&None));
    let mut changed = fixture();
    changed.classes.get_mut("LBase;").unwrap().methods[0]
        .metadata
        .access_flags = 9;
    let other = MetadataChain::new(vec![MetadataProvider::Dex(&changed)]);
    assert!(other.inherited_public_instance(&good).is_none());
    assert_eq!(other.inherited.read().unwrap().get(&good), Some(&None));
}
#[test]
fn nearer_class_masks_never_skip_illegal_or_covariant_declarations() {
    for flags in [2, 4, 9, 0x41, 0x1001] {
        let mut data = fixture();
        data.classes.get_mut("LBase;").unwrap().methods[0]
            .metadata
            .access_flags = flags;
        assert!(
            data.inherited_public_instance(&key("LLeaf;")).is_none(),
            "{flags:#x}"
        );
    }
    let mut data = fixture();
    let class = data.classes.get_mut("LBase;").unwrap();
    class.methods[0].key.return_type = "J".into();
    class.methods[0].descriptor = "()J".into();
    assert!(data.inherited_public_instance(&key("LLeaf;")).is_none());
    let mut data = fixture();
    let original = data.classes["LBase;"].methods[0].clone();
    data.classes
        .get_mut("LLeaf;")
        .unwrap()
        .methods
        .push(ParsedMethod {
            key: key("LLeaf;"),
            ..original
        });
    assert!(data.inherited_public_instance(&key("LLeaf;")).is_none());
}
#[test]
fn missing_cycles_generic_unknown_and_body_contradictions_stay_closed() {
    for change in 0..8 {
        let mut data = fixture();
        match change {
            0 => {
                data.classes.remove("Ljava/lang/Object;");
            }
            1 => {
                data.classes
                    .get_mut("LBase;")
                    .unwrap()
                    .declaration
                    .superclass = Some("Leaf".into());
            }
            2 => {
                data.classes.get_mut("LLeaf;").unwrap().signature =
                    Some("<T:Ljava/lang/Object;>LBase;".into());
            }
            3 => {
                data.classes.get_mut("LBase;").unwrap().methods[0].signature = Some("()TT;".into());
            }
            4 => {
                data.classes.get_mut("LBase;").unwrap().methods[0]
                    .metadata
                    .exceptions = None;
            }
            5 => {
                data.classes
                    .get_mut("LBase;")
                    .unwrap()
                    .code_presence
                    .insert(key("LBase;"), false);
            }
            6 => {
                data.classes.get_mut("LBase;").unwrap().methods[0]
                    .metadata
                    .access_flags = 0x501;
            }
            _ => {
                data.classes
                    .get_mut("LBase;")
                    .unwrap()
                    .declaration
                    .is_interface = true;
            }
        }
        assert!(
            data.inherited_public_instance(&key("LLeaf;")).is_none(),
            "case {change}"
        );
    }
    for flags in [0x101, 0x401] {
        let mut data = fixture();
        let base = data.classes.get_mut("LBase;").unwrap();
        base.methods[0].metadata.access_flags = flags;
        base.code_presence.insert(key("LBase;"), false);
        assert!(data.inherited_public_instance(&key("LLeaf;")).is_some());
    }
}
#[test]
fn runtime_owner_selection_does_not_supply_interface_defaults() {
    let mut data = fixture();
    data.classes.get_mut("LBase;").unwrap().methods.clear();
    data.classes
        .get_mut("LLeaf;")
        .unwrap()
        .declaration
        .interfaces = vec!["MissingDefault".into()];
    let chain = MetadataChain::new(vec![MetadataProvider::Dex(&data)]);
    assert!(chain.jvm_class("LLeaf;").is_none());
    assert!(chain.inherited_public_instance(&key("LLeaf;")).is_none());
}

#[test]
#[ignore = "genuine Samsung Android16 candidate declarations; not APK runtime or javac parity"]
fn runtime_genuine_three_inherited_view_references_and_sdk_shadow_denial() {
    let path = "/tmp/opencode/samsung-platform-metadata-20261003/framework-android16.jar";
    let receipt =
        std::fs::read_to_string("/tmp/opencode/samsung-platform-metadata-20261003/provenance.json")
            .unwrap();
    assert!(receipt.contains("\"original_apk_runtime_compatible\": false"));
    assert!(receipt.contains("\"javac_classpath\": false"));
    let hash = std::process::Command::new("sha256sum")
        .arg(path)
        .output()
        .unwrap();
    assert!(hash.status.success());
    assert!(
        String::from_utf8(hash.stdout)
            .unwrap()
            .starts_with("1360ba6c76b98c2715117ed396fc12d63a162f8c53ba4c8dd896d1a6576cfdcb ")
    );
    let data = DexMetadata::from_path(path).unwrap();
    let jdk = Classpath::from_paths(&[std::path::PathBuf::from(
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod",
    )])
    .unwrap();
    let sdk = Classpath::from_paths(&[std::path::PathBuf::from(
        "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
    )])
    .unwrap();
    let chain = MetadataChain::new(vec![
        MetadataProvider::Jvm(&jdk),
        MetadataProvider::Dex(&data),
        MetadataProvider::Jvm(&sdk),
    ]);
    let targets = [
        MethodKey {
            owner: "Landroid/widget/EditText;".into(),
            name: "setAutoHandwritingEnabled".into(),
            parameters: vec!["Z".into()],
            return_type: "V".into(),
        },
        MethodKey {
            owner: "Landroid/widget/LinearLayout;".into(),
            name: "findOnBackInvokedDispatcher".into(),
            parameters: vec![],
            return_type: "Landroid/window/OnBackInvokedDispatcher;".into(),
        },
        MethodKey {
            owner: "Landroid/view/ViewGroup;".into(),
            name: "setHandwritingBoundsOffsets".into(),
            parameters: vec!["F".into(); 4],
            return_type: "V".into(),
        },
    ];
    for reference in targets {
        assert!(chain.method(&reference).is_none());
        let proof = chain.inherited_public_instance(&reference).unwrap();
        assert_eq!(proof.reference(), &reference);
        assert_eq!(proof.kind(), InheritedMethodKind::RuntimeClassInstance);
        assert_eq!(proof.declaration().key.owner, "Landroid/view/View;");
        assert!(proof.declaration().code.is_none());
        assert!(proof.declaration().metadata.exceptions.is_some());
        assert!(chain.jvm_class(&reference.owner).is_none());
        println!(
            "ACTUAL {reference:?} -> {:?} flags={:#x} throws={:?}",
            proof.declaration().key,
            proof.declaration().metadata.access_flags,
            proof.declaration().metadata.exceptions
        );
    }
    let mut missing = data.clone();
    let method = MethodKey {
        owner: "Landroid/view/View;".into(),
        name: "setAlpha".into(),
        parameters: vec!["F".into()],
        return_type: "V".into(),
    };
    assert!(sdk.method(&method).is_some());
    missing
        .classes
        .get_mut("Landroid/view/View;")
        .unwrap()
        .methods
        .retain(|m| m.key != method);
    let chain = MetadataChain::new(vec![
        MetadataProvider::Jvm(&jdk),
        MetadataProvider::Dex(&missing),
        MetadataProvider::Jvm(&sdk),
    ]);
    assert!(chain.method(&method).is_none());
    let symbolic = MethodKey {
        owner: "Landroid/widget/EditText;".into(),
        ..method
    };
    assert!(chain.inherited_public_instance(&symbolic).is_none());
}
