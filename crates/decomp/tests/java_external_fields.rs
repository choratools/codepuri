//! Genuine javac and Android 35 declarations; no invented Samsung platform API.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use decomp::{
    java_aliases::FieldKey,
    java_external_metadata::{
        Classpath, ConstantValue, ExternalMetadata, FieldAccessContext, FieldRequirement,
        resolve_field, retain_field_receiver,
    },
};
use std::{path::PathBuf, process::Command};

fn tool(name: &str) -> PathBuf {
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        let p = PathBuf::from(home).join("bin").join(name);
        if p.is_file() {
            return p;
        }
    }
    if Command::new(name).arg("-version").output().is_ok() {
        return name.into();
    }
    PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/bin").join(name)
}
struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture() -> (Fixture, Classpath) {
    static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let f = Fixture(std::env::temp_dir().join(format!(
        "external-fields-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )));
    std::fs::create_dir(&f.0).unwrap();
    let sources = [
        (
            "Base.java",
            "package p; public class Base { static { System.setProperty(\"external.field.init\",\"yes\"); } public int value=7; protected int protectedValue=8; int packageValue=9; private int privateValue=10; public static final int CONSTANT=31; public static final String TEXT=\"a\\u0000\\ud800\"; public static final float NEGZERO=-0.0f; public static final int RUNTIME=Integer.parseInt(\"32\"); public final int INSTANCE=33; }",
        ),
        (
            "Child.java",
            "package q; public class Child extends p.Base { public String value=\"shadow\"; public int protectedRead(Child x) { return x.protectedValue; } }",
        ),
        (
            "Other.java",
            "package q; public class Other extends p.Base {}",
        ),
        (
            "Left.java",
            "package p; public interface Left { int SAME=1; }",
        ),
        (
            "Right.java",
            "package p; public interface Right { int SAME=2; }",
        ),
        (
            "Both.java",
            "package p; public class Both implements Left,Right {}",
        ),
    ];
    for (name, text) in sources {
        std::fs::write(f.0.join(name), text).unwrap();
    }
    let out = Command::new(tool("javac"))
        .arg("-d")
        .arg(&f.0)
        .args(sources.iter().map(|(n, _)| f.0.join(n)))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut cp = Classpath::from_paths(
        [
            "p/Base.class",
            "q/Child.class",
            "q/Other.class",
            "p/Left.class",
            "p/Right.class",
            "p/Both.class",
        ]
        .map(|s| f.0.join(s)),
    )
    .unwrap();
    let base = std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11"))
        .join("jmods/java.base.jmod");
    cp.add_path(base).unwrap();
    (f, cp)
}
fn key(owner: &str, name: &str, descriptor: &str) -> FieldKey {
    FieldKey {
        owner: owner.into(),
        name: name.into(),
        descriptor: descriptor.into(),
    }
}
fn context<'a>(
    caller: &'a str,
    receiver: Option<&'a str>,
    is_static: bool,
) -> FieldAccessContext<'a> {
    FieldAccessContext {
        caller,
        receiver,
        is_static,
    }
}
#[test]
fn genuine_constantvalue_bits_utf16_and_nonconstant_initialization() {
    let (f, cp) = fixture();
    let get = |name, desc| cp.field(&key("Lp/Base;", name, desc)).unwrap();
    assert_eq!(
        get("CONSTANT", "I").inline_constant(),
        Some(&ConstantValue::Integer(31))
    );
    assert_eq!(
        get("TEXT", "Ljava/lang/String;").inline_constant(),
        Some(&ConstantValue::String(vec![97, 0, 0xd800]))
    );
    assert_eq!(
        get("NEGZERO", "F").inline_constant(),
        Some(&ConstantValue::Float(0x80000000))
    );
    assert_eq!(get("RUNTIME", "I").constant_value, None);
    assert_eq!(
        get("INSTANCE", "I").constant_value,
        Some(ConstantValue::Integer(33))
    );
    assert_eq!(get("INSTANCE", "I").inline_constant(), None);
    assert!(
        get("CONSTANT", "I")
            .attributes
            .iter()
            .any(|(name, data)| name == "ConstantValue" && data.len() == 2)
    );
    let out = Command::new(tool("javap"))
        .args(["-v", "-p", "-classpath"])
        .arg(&f.0)
        .arg("p.Base")
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("ConstantValue: int 31"));
    assert!(text.contains("ConstantValue: float -0.0f"));
    // Owner qualification retains the runtime initializer, not a guessed value.
    std::fs::write(f.0.join("Probe.java"), "public class Probe { public static void main(String[] a) { if(p.Base.CONSTANT!=31 || System.getProperty(\"external.field.init\")!=null) throw new AssertionError(); if(p.Base.RUNTIME!=32 || !\"yes\".equals(System.getProperty(\"external.field.init\"))) throw new AssertionError(); p.Base b=new q.Child(); if(b.value!=7) throw new AssertionError(); } }").unwrap();
    assert!(
        Command::new(tool("javac"))
            .arg("-cp")
            .arg(&f.0)
            .arg("-d")
            .arg(&f.0)
            .arg(f.0.join("Probe.java"))
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new(tool("java"))
            .arg("-cp")
            .arg(&f.0)
            .arg("Probe")
            .status()
            .unwrap()
            .success()
    );
}
#[test]
fn inherited_identity_visibility_receiver_shadow_and_ambiguity() {
    let (f, cp) = fixture();
    let ctx = context("Lq/Child;", Some("Lq/Child;"), false);
    let field = resolve_field(&[], &key("Lq/Child;", "value", "I"), &cp, &ctx).unwrap();
    assert_eq!(field.key.owner, "Lp/Base;");
    assert_eq!(
        field.origin,
        decomp::java_external_metadata::FieldOrigin::Classpath
    );
    assert!(!retain_field_receiver(&[], &field, &cp, &ctx).unwrap());
    let mut inventory = decomp::java_aliases::AliasInventory::default();
    inventory.add_field(field.key.clone()).unwrap();
    inventory
        .add_field(key("Lq/Child;", "value", "Ljava/lang/String;"))
        .unwrap();
    let aliases = inventory.build(decomp::java_aliases::AliasMode::Ascii);
    let read = format!(
        "(({})new {}()).{}",
        aliases.class_name(&field.key.owner).unwrap(),
        aliases.class_name("Lq/Child;").unwrap(),
        aliases.field_name(&field.key).unwrap()
    );
    std::fs::write(f.0.join("AliasProbe.java"), format!("public class AliasProbe {{ public static void main(String[] args) {{ if ({read} != 7) throw new AssertionError(); }} }}")).unwrap();
    assert!(
        Command::new(tool("javac"))
            .arg("-cp")
            .arg(&f.0)
            .arg("-d")
            .arg(&f.0)
            .arg(f.0.join("AliasProbe.java"))
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        Command::new(tool("java"))
            .arg("-cp")
            .arg(&f.0)
            .arg("AliasProbe")
            .status()
            .unwrap()
            .success()
    );
    assert!(resolve_field(&[], &key("Lq/Child;", "protectedValue", "I"), &cp, &ctx).is_ok());
    for receiver in ["Lp/Base;", "Lq/Other;"] {
        assert!(matches!(
            resolve_field(
                &[],
                &key("Lp/Base;", "protectedValue", "I"),
                &cp,
                &context("Lq/Child;", Some(receiver), false)
            ),
            Err(FieldRequirement::Access(_))
        ));
    }
    for name in ["privateValue", "packageValue"] {
        assert!(matches!(
            resolve_field(&[], &key("Lp/Base;", name, "I"), &cp, &ctx),
            Err(FieldRequirement::Access(_))
        ));
    }
    assert!(
        resolve_field(
            &[],
            &key("Lp/Base;", "packageValue", "I"),
            &cp,
            &context("Lp/Peer;", Some("Lp/Base;"), false)
        )
        .is_ok()
    );
    assert!(matches!(
        resolve_field(
            &[],
            &key("Lp/Both;", "SAME", "I"),
            &cp,
            &context("Lp/Peer;", None, true)
        ),
        Err(FieldRequirement::Ambiguous(_))
    ));
    assert!(matches!(
        resolve_field(
            &[],
            &key("Lp/Base;", "value", "I"),
            &cp,
            &context("Lp/Base;", None, true)
        ),
        Err(FieldRequirement::Staticness(_))
    ));
    // javac independently rejects the protected Base-typed receiver.
    std::fs::write(f.0.join("Bad.java"), "package q; public class Bad extends p.Base { int bad(p.Base b) { return b.protectedValue; } }").unwrap();
    assert!(
        !Command::new(tool("javac"))
            .arg("-cp")
            .arg(&f.0)
            .arg(f.0.join("Bad.java"))
            .output()
            .unwrap()
            .status
            .success()
    );
}
#[test]
fn android35_actual_view_build_string_metadata_and_missing_proprietary() {
    let jar = std::env::var_os("ANDROID35_JAR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar")
        });
    assert!(
        jar.is_file(),
        "set ANDROID35_JAR to a genuine Android 35 platform jar"
    );
    let cp = Classpath::from_paths([&jar]).unwrap();
    for (owner, name, desc, constant) in [
        (
            "Landroid/view/View;",
            "VISIBLE",
            "I",
            Some(ConstantValue::Integer(0)),
        ),
        (
            "Landroid/view/View;",
            "GONE",
            "I",
            Some(ConstantValue::Integer(8)),
        ),
        ("Landroid/os/Build$VERSION;", "SDK_INT", "I", None),
        ("Landroid/os/Build;", "MODEL", "Ljava/lang/String;", None),
        ("Landroid/os/Build;", "BRAND", "Ljava/lang/String;", None),
        (
            "Landroid/view/View;",
            "ALPHA",
            "Landroid/util/Property;",
            None,
        ),
        (
            "Ljava/lang/String;",
            "CASE_INSENSITIVE_ORDER",
            "Ljava/util/Comparator;",
            None,
        ),
    ] {
        let resolved = resolve_field(
            &[],
            &key(owner, name, desc),
            &cp,
            &context("Lcom/samsung/android/samsungpass/Caller;", None, true),
        )
        .unwrap();
        assert_eq!(resolved.key.owner, owner);
        assert_eq!(resolved.metadata.constant_value, constant);
        assert_eq!(resolved.metadata.access_flags & 0x19, 0x19);
        let binary = owner
            .trim_start_matches('L')
            .trim_end_matches(';')
            .replace('/', ".");
        let out = Command::new(tool("javap"))
            .args(["-v", "-p", "-classpath"])
            .arg(&jar)
            .arg(binary)
            .output()
            .unwrap();
        assert!(out.status.success());
        let text = String::from_utf8_lossy(&out.stdout);
        let start = text.find(&format!(" {name};")).unwrap();
        let block = &text[start..];
        let block = block.split("\n\n").next().unwrap();
        assert!(block.contains(&format!("descriptor: {desc}")), "{block}");
        assert_eq!(
            block.contains("ConstantValue:"),
            constant.is_some(),
            "{block}"
        );
    }
    assert!(matches!(
        resolve_field(
            &[],
            &key("Lcom/samsung/android/pass/Proprietary;", "FIELD", "I"),
            &cp,
            &context("Lcom/samsung/android/samsungpass/Caller;", None, true)
        ),
        Err(FieldRequirement::Hierarchy(_))
    ));
}
#[test]
fn apk_priority_crossdex_and_duplicate_owner_fail_closed() {
    let dex = dex_format::DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let (class, encoded) = dex
        .classes
        .iter()
        .find_map(|c| {
            c.class_data
                .as_ref()?
                .instance_fields
                .first()
                .map(|f| (c, f))
        })
        .unwrap();
    let id = dex.field_id(encoded.field_idx).unwrap();
    let ty = |index| {
        dex.string(dex.type_id(index).unwrap().descriptor_idx)
            .unwrap()
            .to_owned()
    };
    let k = key(
        &ty(class.class_idx),
        dex.string(id.name_idx).unwrap(),
        &ty(u32::from(id.type_idx)),
    );
    let mut one = dex.clone();
    one.classes.clear();
    let mut two = dex.clone();
    two.classes = vec![class.clone()];
    let ctx = context(&k.owner, Some(&k.owner), false);
    let cp = Classpath::default();
    let field = resolve_field(&[one, two.clone()], &k, &cp, &ctx).unwrap();
    assert_eq!(field.metadata.access_flags, encoded.access_flags);
    assert_eq!(
        field.origin,
        decomp::java_external_metadata::FieldOrigin::Input
    );
    assert!(matches!(
        resolve_field(&[two.clone(), two], &k, &cp, &ctx),
        Err(FieldRequirement::Ambiguous(_))
    ));
}

#[test]
#[ignore = "read-only local SamsungPass field-pool probe; no export or app execution"]
fn samsungpass_actual_external_field_references() {
    use std::io::Read;
    let apk = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
    let apk = std::env::var_os("SAMSUNGPASS_APK")
        .map(PathBuf::from)
        .unwrap_or(apk);
    let jar = std::env::var_os("ANDROID35_JAR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar")
        });
    let cp = Classpath::from_paths([jar]).unwrap();
    let mut archive = zip::ZipArchive::new(std::fs::File::open(apk).unwrap()).unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        if !entry.name().starts_with("classes") || !entry.name().ends_with(".dex") {
            continue;
        }
        let pool = entry.name().to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        let dex = dex_format::DexFile::from_bytes(&bytes).unwrap();
        for (field_index, id) in dex.field_ids.iter().enumerate() {
            let owner = dex
                .string(dex.type_id(u32::from(id.class_idx)).unwrap().descriptor_idx)
                .unwrap();
            if ![
                "Landroid/view/View;",
                "Landroid/os/Build;",
                "Ljava/lang/String;",
            ]
            .contains(&owner)
            {
                continue;
            }
            let name = dex.string(id.name_idx).unwrap();
            let desc = dex
                .string(dex.type_id(u32::from(id.type_idx)).unwrap().descriptor_idx)
                .unwrap();
            let k = key(owner, name, desc);
            let metadata = cp.field(&k).unwrap();
            if metadata.access_flags & 1 == 0 {
                continue;
            }
            let field = resolve_field(
                &[],
                &k,
                &cp,
                &context(
                    "Lcom/samsung/android/samsungpass/Caller;",
                    None,
                    metadata.access_flags & 8 != 0,
                ),
            )
            .unwrap();
            if seen.insert(owner.to_owned()) {
                eprintln!(
                    "SamsungPass {pool} field#{field_index}: {:?}, flags={:#x}, ConstantValue={:?}",
                    field.key, field.metadata.access_flags, field.metadata.constant_value
                );
            }
        }
    }
    assert_eq!(
        seen.len(),
        3,
        "genuine View/Build/String pool references required"
    );
}
