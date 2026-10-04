#![allow(clippy::unwrap_used)]
use super::*;
struct Fixture {
    classes: BTreeMap<String, SuperClass>,
    interfaces: BTreeMap<String, Vec<String>>,
}
impl SuperMetadata for Fixture {
    fn class(&self, owner: &str) -> Option<SuperClass> {
        self.classes.get(owner).cloned()
    }
    fn interfaces(&self, owner: &str) -> Option<Vec<String>> {
        self.interfaces.get(owner).cloned()
    }
}
fn fixture() -> (Fixture, MethodKey, JavaAliases) {
    let key = MethodKey {
        owner: "Lp/I;".into(),
        name: "m".into(),
        parameters: vec![],
        return_type: "I".into(),
    };
    let method = SuperMethod {
        key: key.clone(),
        metadata: MethodMetadata {
            access_flags: 1,
            exceptions: Some(vec!["Ljava/io/IOException;".into()]),
        },
        generic: false,
    };
    let classes: BTreeMap<String, SuperClass> = BTreeMap::from([
        (
            "Lp/C;".into(),
            SuperClass {
                superclass: Some("Ljava/lang/Object;".into()),
                access_flags: 1,
                methods: vec![],
            },
        ),
        (
            "Lp/I;".into(),
            SuperClass {
                superclass: Some("Ljava/lang/Object;".into()),
                access_flags: 0x601,
                methods: vec![method],
            },
        ),
        (
            "Ljava/lang/Object;".into(),
            SuperClass {
                superclass: None,
                access_flags: 1,
                methods: vec![],
            },
        ),
    ]);
    let interfaces = BTreeMap::from([
        ("Lp/C;".into(), vec!["Lp/I;".into()]),
        ("Lp/I;".into(), vec![]),
        ("Ljava/lang/Object;".into(), vec![]),
    ]);
    let mut a = AliasInventory::default();
    for owner in classes.keys() {
        a.add_type(owner).unwrap();
    }
    a.add_method(key.clone()).unwrap();
    (
        Fixture {
            classes,
            interfaces,
        },
        key,
        a.build(crate::java_aliases::AliasMode::Ascii),
    )
}
#[test]
fn exact_interface_super_requires_complete_unique_direct_default() {
    let (mut f, key, aliases) = fixture();
    let selected = select_interface_super("Lp/C;", &key, &f, &aliases).unwrap();
    assert_eq!(selected.target, key);
    assert_eq!(selected.interface_qualifier.as_deref(), Some("p.I"));
    assert_eq!(
        selected.metadata.exceptions,
        Some(vec!["Ljava/io/IOException;".into()])
    );
    assert!(select_interface_super("Lp/I;", &key, &f, &aliases).is_err()); // static helpers
    f.interfaces.get_mut("Lp/C;").unwrap().clear();
    assert!(select_interface_super("Lp/C;", &key, &f, &aliases).is_err());
    let (mut f, key, aliases) = fixture();
    f.interfaces.remove("Ljava/lang/Object;");
    assert!(select_interface_super("Lp/C;", &key, &f, &aliases).is_err());
    for flag in [0x401, 9, 3, 0x41, 0x81] {
        let (mut f, key, aliases) = fixture();
        f.classes.get_mut("Lp/I;").unwrap().methods[0]
            .metadata
            .access_flags = flag;
        assert!(select_interface_super("Lp/C;", &key, &f, &aliases).is_err());
    }
    let (mut f, key, aliases) = fixture();
    f.classes.get_mut("Lp/I;").unwrap().methods[0]
        .metadata
        .exceptions = None;
    assert!(select_interface_super("Lp/C;", &key, &f, &aliases).is_err());
}
#[test]
fn redundant_competing_abstract_cyclic_and_superclass_overrides_reject() {
    for redundant in [false, true] {
        let (mut f, key, aliases) = fixture();
        f.classes.insert("Lp/J;".into(), f.classes["Lp/I;"].clone());
        f.interfaces.insert(
            "Lp/J;".into(),
            if redundant {
                vec!["Lp/I;".into()]
            } else {
                vec![]
            },
        );
        f.interfaces.get_mut("Lp/C;").unwrap().push("Lp/J;".into());
        assert!(select_interface_super("Lp/C;", &key, &f, &aliases).is_err());
    }
    let (mut f, key, aliases) = fixture();
    f.interfaces.get_mut("Lp/I;").unwrap().push("Lp/I;".into());
    assert!(select_interface_super("Lp/C;", &key, &f, &aliases).is_err());
    let (mut f, key, aliases) = fixture();
    f.classes.get_mut("Ljava/lang/Object;").unwrap().methods = f.classes["Lp/I;"].methods.clone();
    assert!(select_interface_super("Lp/C;", &key, &f, &aliases).is_err());
    let (mut f, key, aliases) = fixture();
    f.classes.get_mut("Lp/I;").unwrap().methods[0].generic = true;
    assert!(select_interface_super("Lp/C;", &key, &f, &aliases).is_err());
}
#[test]
#[ignore = "requires local Samsung Pass APK, Android35 and JDK metadata"]
fn original_certificate_repository_exact_default_target() {
    use crate::java_external_metadata::Classpath;
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let refs: Vec<_> = inputs.iter().collect();
    let cp = Classpath::from_paths([
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod",
        "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
    ])
    .unwrap();
    let provider = SuperIndex::collect(&refs, &cp).unwrap();
    let key = crate::java_members::source_closure::method_key(&inputs[1], 24243).unwrap();
    let mut inventory = AliasInventory::default();
    for owner in provider.inputs.keys() {
        inventory.add_type(owner).unwrap();
    }
    inventory.add_method(key.clone()).unwrap();
    let aliases = inventory.build(crate::java_aliases::AliasMode::Ascii);
    let caller =
        "Lcom/samsung/android/samsungpass/certificate/repository/CertificateRepositoryImpl;";
    let selected = select_interface_super(caller, &key, &provider, &aliases).unwrap();
    assert_eq!(selected.target, key);
    assert_eq!(selected.metadata.access_flags, 1);
    assert!(select_interface_super(&key.owner, &key, &provider, &aliases).is_err());
}
