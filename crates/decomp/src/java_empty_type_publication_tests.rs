#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use dex_format::{RawSpan, ids::TypeId, strings::StringData};

fn fixture() -> DexFile {
    let mut dex = DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let span = RawSpan { offset: 0, size: 0 };
    let mut add = |text: &str| {
        let string = dex.strings.len() as u32;
        dex.strings.push(StringData {
            value: text.into(),
            utf16_size: text.len() as u32,
            span,
        });
        let ty = dex.type_ids.len() as u32;
        dex.type_ids.push(TypeId {
            descriptor_idx: string,
            span,
        });
        ty
    };
    let owner = add("Lp/Marker;");
    let object = add("Ljava/lang/Object;");
    let mut class = dex.classes[0].clone();
    class.class_idx = owner;
    class.superclass_idx = object;
    class.access_flags = 0x601;
    class.class_data_off = 0;
    class.class_data = None;
    class.annotations_off = 0;
    class.annotations = None;
    class.interfaces_off = 0;
    class.interfaces = None;
    class.static_values_off = 0;
    class.static_values = None;
    dex.classes = vec![class];
    dex
}
fn name(dex: &DexFile, index: u32) -> Result<String, DeclarationUnsupported> {
    let d = dex
        .type_id(index)
        .ok()
        .and_then(|t| dex.string(t.descriptor_idx).ok())
        .ok_or(DeclarationUnsupported::InvalidMetadata)?;
    Ok(d.trim_start_matches('L')
        .trim_end_matches(';')
        .replace('/', "."))
}
#[test]
fn genuine_empty_interface_has_no_original_or_java_constructor() {
    let dex = fixture();
    let plan = plan_empty_interface(&dex, &dex.classes[0], |i| name(&dex, i)).unwrap();
    assert_eq!(plan.header, "public interface Marker");
    assert_eq!(plan.access_flags, 0x601);
    assert!(plan.declaration_only());
    assert_eq!(plan.original_defined_methods(), 0);
    for (label, jdk) in [
        ("17", "/home/jtjisgod/tools/jdk-17.0.13+11"),
        ("25", "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1"),
    ] {
        let root = std::env::temp_dir().join(format!(
            "decomphify-empty-interface-{label}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("p")).unwrap();
        std::fs::write(
            root.join("p/Marker.java"),
            format!("package p; {} {{}}", plan.header),
        )
        .unwrap();
        std::fs::write(root.join("Probe.java"),"public class Probe {public static void main(String[] args) { Class<?> c=p.Marker.class; if(c.getModifiers()!=0x601 || c.getDeclaredConstructors().length!=0 || c.getDeclaredMethods().length!=0 || c.getDeclaredFields().length!=0)throw new AssertionError(); }}").unwrap();
        let out = std::process::Command::new(format!("{jdk}/bin/javac"))
            .args(["-d"])
            .arg(&root)
            .arg(root.join("p/Marker.java"))
            .arg(root.join("Probe.java"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            std::process::Command::new(format!("{jdk}/bin/java"))
                .arg("-cp")
                .arg(&root)
                .arg("Probe")
                .status()
                .unwrap()
                .success()
        );
    }
}
#[test]
fn unrepresented_metadata_flags_members_and_hierarchy_stay_closed() {
    let dex = fixture();
    let class = &dex.classes[0];
    for flags in [0x1401, 0x1601, 0x2601, 0x601 | 0x10, 0x200] {
        let mut c = class.clone();
        c.access_flags = flags;
        assert!(plan_empty_interface(&dex, &c, |i| name(&dex, i)).is_err());
    }
    let mut c = class.clone();
    c.annotations_off = 1;
    assert_eq!(
        plan_empty_interface(&dex, &c, |i| name(&dex, i)).unwrap_err(),
        EmptyTypeRequirement::NotPureInterface
    );
    c = class.clone();
    c.interfaces_off = 1;
    assert!(plan_empty_interface(&dex, &c, |i| name(&dex, i)).is_err());
    c = class.clone();
    c.static_values_off = 1;
    assert!(plan_empty_interface(&dex, &c, |i| name(&dex, i)).is_err());
    c = class.clone();
    c.class_data_off = 1;
    assert_eq!(
        plan_empty_interface(&dex, &c, |i| name(&dex, i)).unwrap_err(),
        EmptyTypeRequirement::InvalidMetadata
    );
    c = class.clone();
    c.superclass_idx = u32::MAX;
    assert!(plan_empty_interface(&dex, &c, |i| name(&dex, i)).is_err());
    assert!(
        plan_empty_interface(&dex, class, |_| Err(
            DeclarationUnsupported::InvalidMetadata
        ))
        .is_err()
    );
}
#[test]
#[ignore = "bounded original Samsung APK marker inventory and unchanged compilation-unit probe"]
fn samsung_original_empty_interfaces_and_unchanged_consumer() {
    let apk = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
    let files = dex_format::apk::parse_apk_path(apk).unwrap();
    let mut inventory = crate::java_aliases::AliasInventory::default();
    for dex in &files {
        for ty in &dex.type_ids {
            inventory
                .add_type(dex.string(ty.descriptor_idx).unwrap())
                .unwrap();
        }
    }
    let aliases = inventory.build(crate::java_aliases::AliasMode::Ascii);
    let mut plans = Vec::new();
    for dex in &files {
        for class in &dex.classes {
            if let Ok(plan) = plan_empty_interface(dex, class, |i| {
                let raw = dex.string(dex.type_id(i).unwrap().descriptor_idx).unwrap();
                aliases
                    .class_name(raw)
                    .map(str::to_owned)
                    .ok_or(DeclarationUnsupported::InvalidMetadata)
            }) {
                plans.push(plan);
            }
        }
    }
    assert_eq!(plans.len(), 131);
    let plan = plans.iter().find(|p| p.descriptor == "La1/a;").unwrap();
    assert_eq!(plan.header, "public interface a$type");
    let original = std::path::Path::new(
        "/tmp/opencode/samsung-pass-100-wave46-runtime-metadata/java25/sources/a1/b$type.java",
    );
    let bytes = std::fs::read(original).unwrap();
    let root = std::env::temp_dir().join(format!(
        "decomphify-original-empty-marker-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(root.join("a1")).unwrap();
    std::fs::create_dir_all(root.join("empty-sourcepath")).unwrap();
    std::fs::write(root.join("a1/b$type.java"), &bytes).unwrap();
    let jdk = "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1";
    let compile = |marker: bool| {
        let mut cmd = std::process::Command::new(format!("{jdk}/bin/javac"));
        cmd.args([
            "--release",
            "25",
            "-Xmaxerrs",
            "1000",
            "-cp",
            "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
            "-sourcepath",
        ])
        .arg(root.join("empty-sourcepath"))
        .arg("-d")
        .arg(root.join(if marker { "after" } else { "before" }))
        .arg(root.join("a1/b$type.java"));
        if marker {
            cmd.arg(root.join("a1/a$type.java"));
        }
        cmd.output().unwrap()
    };
    let before = compile(false);
    std::fs::write(
        root.join("a1/a$type.java"),
        format!("package a1; {} {{}}", plan.header),
    )
    .unwrap();
    let after = compile(true);
    std::fs::write(root.join("before.log"), &before.stderr).unwrap();
    std::fs::write(root.join("after.log"), &after.stderr).unwrap();
    let b = String::from_utf8_lossy(&before.stderr);
    let a = String::from_utf8_lossy(&after.stderr);
    assert!(b.contains("class a$type"));
    assert!(!a.contains("class a$type"));
    assert_eq!(std::fs::read(root.join("a1/b$type.java")).unwrap(), bytes);
    println!(
        "actual original marker La1/a; restores unchanged consumer type; remaining dependencies not claimed solved: {}",
        root.display()
    );
}

#[test]
fn production_sink_publishes_declaration_without_original_body_counts() {
    let dex = fixture();
    let mut sources = Vec::new();
    let mut sink = |source: &crate::output::GeneratedSource| {
        sources.push(source.clone());
        Ok(())
    };
    let report =
        crate::dex_java::emit_java(&[dex], &crate::dex_java::JavaOptions::default(), &mut sink)
            .unwrap();
    assert_eq!(report.classes_emitted, 1);
    assert_eq!(report.complete_methods, 0);
    assert_eq!(report.complete_constructors, 0);
    assert_eq!(report.complete_static_initializers, 0);
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].provenance.descriptor, "Lp/Marker;");
    assert!(sources[0].text.contains("public interface Marker"));
    assert!(
        sources[0]
            .diagnostics
            .iter()
            .any(|d| d.message.contains("declaration only")
                && d.message.contains("original defined methods 0"))
    );
}
