//! Genuine return bridges, exact body origins and whole-owner shadowing.
#![allow(clippy::unwrap_used)]
use decomp::{
    java_aliases::MethodKey,
    java_external_metadata::{
        ClassMetadata, Classpath, DexMetadata, ExternalMetadata, MetadataChain, MetadataProvider,
    },
};
use std::{path::PathBuf, process::Command, sync::Arc};

fn jdk() -> PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into())
}

fn fixture() -> (Vec<u8>, ClassMetadata) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "covariant-bridge-metadata-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("BridgeEvidence.java"),
        r#"public class BridgeEvidence {
 public static class Base {
   public Number get() { return Integer.valueOf(1); }
   public Number many(int a, long b, Object c, float d, double e, boolean f, byte g, short h, char i) { return Integer.valueOf(a); }
 }
 public static class Mid extends Base {
   @Override public Integer get() { return Integer.valueOf(2); }
   @Override public Integer many(int a, long b, Object c, float d, double e, boolean f, byte g, short h, char i) { return Integer.valueOf(a + (int)b); }
 }
}"#,
    )
    .unwrap();
    let compiled = Command::new(jdk().join("bin/javac"))
        .args(["--release", "17"])
        .arg(root.join("BridgeEvidence.java"))
        .output()
        .unwrap();
    assert!(compiled.status.success(), "{compiled:?}");
    let bytes = std::fs::read(root.join("BridgeEvidence$Mid.class")).unwrap();
    let class = ClassMetadata::parse(&bytes).unwrap();
    (bytes, class)
}

fn pair(class: &ClassMetadata, name: &str) -> (MethodKey, MethodKey) {
    let bridge = class
        .methods
        .iter()
        .find(|m| m.key.name == name && m.metadata.access_flags & 0x1040 == 0x1040)
        .unwrap();
    let target = class
        .methods
        .iter()
        .find(|m| m.key.name == name && m.metadata.access_flags & 0x40 == 0)
        .unwrap();
    (bridge.key.clone(), target.key.clone())
}

#[test]
fn actual_bytecode_attests_typed_incoming_loads_and_rejects_mutation() {
    let (bytes, class) = fixture();
    let mut cp = Classpath::default();
    cp.add_class(&bytes).unwrap();
    for name in ["get", "many"] {
        let (bridge, target) = pair(&class, name);
        assert!(class.attests_covariant_bridge(&bridge, &target));
        assert!(cp.attests_covariant_bridge(&bridge, &target));
        assert!(!class.attests_covariant_bridge(&target, &bridge));

        let mut changed = class.clone();
        changed
            .methods
            .iter_mut()
            .find(|m| m.key == target)
            .unwrap()
            .metadata
            .access_flags |= 8;
        assert!(!changed.attests_covariant_bridge(&bridge, &target));

        let mut changed = class.clone();
        let method = changed
            .methods
            .iter_mut()
            .find(|m| m.key == bridge)
            .unwrap();
        Arc::make_mut(method.code.as_mut().unwrap()).bytes[0] = 0x01;
        assert!(!changed.attests_covariant_bridge(&bridge, &target));

        let mut changed = class.clone();
        changed.methods.push(
            changed
                .methods
                .iter()
                .find(|m| m.key == target)
                .unwrap()
                .clone(),
        );
        assert!(!changed.attests_covariant_bridge(&bridge, &target));

        // Reparse different body bytes with the same genuine bridge flags and
        // descriptor. A non-forwarding body cannot install an origin proof.
        let method = class.methods.iter().find(|m| m.key == bridge).unwrap();
        let code = method.code.as_ref().unwrap();
        let mut changed_bytes = bytes.clone();
        changed_bytes[code.attribute_offset + 8] = 0x01;
        let changed = ClassMetadata::parse(&changed_bytes).unwrap();
        assert!(!changed.attests_covariant_bridge(&bridge, &target));
    }
}

#[test]
fn dex_owner_shadowing_never_borrows_jvm_bridge_proof() {
    let (bytes, class) = fixture();
    let (bridge, target) = pair(&class, "get");
    let mut cp = Classpath::default();
    cp.add_class(&bytes).unwrap();
    let mut dex = dex_format::DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    dex.classes.truncate(1);
    dex.classes[0].class_idx = 0;
    dex.classes[0].access_flags = 1;
    dex.classes[0].superclass_idx = u32::MAX;
    dex.classes[0].class_data = None;
    dex.classes[0].class_data_off = 0;
    dex.classes[0].annotations = None;
    dex.classes[0].annotations_off = 0;
    dex.classes[0].interfaces = None;
    dex.classes[0].interfaces_off = 0;
    dex.classes[0].static_values = None;
    dex.classes[0].static_values_off = 0;
    dex.strings[0].value = class.descriptor.clone();
    dex.type_ids[0].descriptor_idx = 0;
    let runtime = DexMetadata::from_dex_files(&[dex]).unwrap();
    let runtime_first = MetadataChain::new(vec![
        MetadataProvider::Dex(&runtime),
        MetadataProvider::Jvm(&cp),
    ]);
    assert!(!runtime_first.attests_covariant_bridge(&bridge, &target));
    let compiler_first = MetadataChain::new(vec![
        MetadataProvider::Jvm(&cp),
        MetadataProvider::Dex(&runtime),
    ]);
    assert!(compiler_first.attests_covariant_bridge(&bridge, &target));
}

#[test]
#[ignore = "requires genuine local Android35 SDK"]
fn actual_edittext_covariant_return_bridge_has_byte_derived_origin() {
    let cp = Classpath::from_paths([PathBuf::from(
        "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
    )])
    .unwrap();
    let class = cp.class("Landroid/widget/EditText;").unwrap();
    let (bridge, target) = pair(class, "getText");
    assert_eq!(bridge.return_type, "Ljava/lang/CharSequence;");
    assert_eq!(target.return_type, "Landroid/text/Editable;");
    assert!(cp.attests_covariant_bridge(&bridge, &target));
    assert!(!cp.attests_covariant_bridge(&target, &bridge));
}
