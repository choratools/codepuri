//! Genuine JDK AbstractList protected field access and receiver selection proof.
#![allow(clippy::unwrap_used)]
use decomp::java_external_metadata::fields::{
    resolve_field_declaration, retain_field_receiver_with_aliases,
};
use decomp::{
    java_aliases::{AliasInventory, AliasMode, FieldKey},
    java_external_metadata::{
        Classpath, FieldAccessContext, FieldRequirement, resolve_field, retain_field_receiver,
    },
};
use std::{path::PathBuf, process::Command};
fn tool(name: &str) -> PathBuf {
    PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/bin").join(name)
}
fn command(dir: &std::path::Path, toolname: &str, args: &[&str]) -> std::process::Output {
    Command::new(tool(toolname))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}
fn key(owner: &str) -> FieldKey {
    FieldKey {
        owner: owner.into(),
        name: "modCount".into(),
        descriptor: "I".into(),
    }
}
#[test]
fn actual_abstract_list_modcount_retains_receiver_and_rejects_illegal_parent_cast() {
    let dir = std::env::temp_dir().join(format!("abstract-list-fields-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sources = [
        (
            "Child.java",
            "package q;public class Child extends java.util.AbstractList<Object>{public Object get(int i){return null;}public int size(){return 0;}public int read(Child x){return x.modCount;}public void write(Child x,int v){x.modCount=v;}public static void main(String[]a){Child c=new Child();c.write(c,31);if(c.read(c)!=31)throw new AssertionError();}}",
        ),
        (
            "Shadow.java",
            "package q;public class Shadow extends Child {public String modCount=\"shadow\";}",
        ),
        (
            "Other.java",
            "package q;public class Other extends java.util.AbstractList<Object>{public Object get(int i){return null;}public int size(){return 0;}}",
        ),
    ];
    for (name, s) in sources {
        std::fs::write(dir.join(name), s).unwrap();
    }
    let o = command(
        &dir,
        "javac",
        &["-d", ".", "Child.java", "Shadow.java", "Other.java"],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let cp = Classpath::from_paths([
        dir.join("q/Child.class"),
        dir.join("q/Shadow.class"),
        dir.join("q/Other.class"),
        PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod"),
    ])
    .unwrap();
    let ctx = FieldAccessContext {
        caller: "Lq/Child;",
        receiver: Some("Lq/Child;"),
        is_static: false,
    };
    let f = resolve_field(&[], &key("Lq/Child;"), &cp, &ctx).unwrap();
    assert_eq!(f.key.owner, "Ljava/util/AbstractList;");
    assert_eq!(f.metadata.access_flags, 0x84);
    assert!(retain_field_receiver(&[], &f, &cp, &ctx).unwrap());
    let mut inv = AliasInventory::default();
    inv.add_field(f.key.clone()).unwrap();
    let aliases = inv.build(AliasMode::Ascii);
    assert!(retain_field_receiver_with_aliases(&[], &f, &cp, &ctx, &aliases).unwrap());
    for receiver in ["Ljava/util/AbstractList;", "Lq/Other;"] {
        let invalid = FieldAccessContext {
            receiver: Some(receiver),
            ..ctx
        };
        assert!(matches!(
            resolve_field(&[], &f.key, &cp, &invalid),
            Err(FieldRequirement::Access(_))
        ));
    }
    let shadow = FieldAccessContext {
        receiver: Some("Lq/Shadow;"),
        ..ctx
    };
    assert!(!retain_field_receiver(&[], &f, &cp, &shadow).unwrap());
    let mut shadow_inventory = AliasInventory::default();
    shadow_inventory.add_field(f.key.clone()).unwrap();
    shadow_inventory
        .add_field(FieldKey {
            owner: "Lq/Shadow;".into(),
            name: "modCount".into(),
            descriptor: "Ljava/lang/String;".into(),
        })
        .unwrap();
    let shadow_aliases = shadow_inventory.build(AliasMode::Ascii);
    assert!(!retain_field_receiver_with_aliases(&[], &f, &cp, &shadow, &shadow_aliases).unwrap());

    // Inspecting a genuine protected declaration grants no permission.
    assert_eq!(
        resolve_field_declaration(&[], &f.key, &cp, false).unwrap(),
        f
    );
    assert!(matches!(
        resolve_field_declaration(&[], &f.key, &cp, true),
        Err(FieldRequirement::Staticness(_))
    ));
    let outsider = FieldAccessContext {
        caller: "Lq/Other;",
        receiver: Some("Lq/Child;"),
        is_static: false,
    };
    assert!(matches!(
        resolve_field(&[], &f.key, &cp, &outsider),
        Err(FieldRequirement::Access(_))
    ));
    let staticuse = FieldAccessContext {
        is_static: true,
        ..ctx
    };
    assert!(matches!(
        resolve_field(&[], &f.key, &cp, &staticuse),
        Err(FieldRequirement::Staticness(_))
    ));
    let o = command(&dir, "java", &["-cp", ".", "q.Child"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    std::fs::write(dir.join("Bad.java"),"package q;public class Bad extends Child{int bad(){return ((java.util.AbstractList<Object>)this).modCount;}}").unwrap();
    let o = command(&dir, "javac", &["-cp", ".", "-d", ".", "Bad.java"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("protected access"));
}
#[test]
#[ignore = "read-only local Samsung Pass original hierarchy probe"]
fn samsung_original_modcount_callers_have_protected_receiver_proof() {
    use std::io::Read;
    let apk = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
    let mut zip = zip::ZipArchive::new(std::fs::File::open(apk).unwrap()).unwrap();
    let mut files = vec![];
    for n in 1..=4 {
        let name = if n == 1 {
            "classes.dex".to_owned()
        } else {
            format!("classes{n}.dex")
        };
        let mut bytes = vec![];
        zip.by_name(&name).unwrap().read_to_end(&mut bytes).unwrap();
        files.push(dex_format::DexFile::from_bytes(&bytes).unwrap());
    }
    let cp = Classpath::from_paths([
        PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod"),
        PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
    ])
    .unwrap();
    for owner in ["Lfq/b$a;", "Leq/i;"] {
        assert!(files.iter().any(|dex| dex.classes.iter().any(|c| {
            dex.string(dex.type_id(c.class_idx).unwrap().descriptor_idx)
                .unwrap()
                == owner
        })));
        let ctx = FieldAccessContext {
            caller: owner,
            receiver: Some(owner),
            is_static: false,
        };
        let resolved = resolve_field(&files, &key("Ljava/util/AbstractList;"), &cp, &ctx).unwrap();
        assert_eq!(resolved.metadata.access_flags, 0x84);
        assert!(retain_field_receiver(&files, &resolved, &cp, &ctx).unwrap());
    }
}

struct InvalidProvider(u8);
impl decomp::java_external_metadata::ExternalMetadata for InvalidProvider {
    fn method(
        &self,
        _: &decomp::java_aliases::MethodKey,
    ) -> Option<decomp::java_external_metadata::MethodMetadata> {
        None
    }
    fn class_access(&self, _: &str) -> Option<u32> {
        Some(1)
    }
    fn class_hierarchy(&self, _: &str) -> Vec<decomp::type_compatibility::Declaration> {
        let d = decomp::type_compatibility::Declaration {
            name: if self.0 == 0 {
                "wrong.Owner"
            } else {
                "p.Owner"
            }
            .into(),
            superclass: if self.0 == 2 {
                Some("p.Owner".into())
            } else {
                None
            },
            interfaces: vec![],
            is_interface: false,
        };
        if self.0 == 1 {
            vec![d.clone(), d]
        } else {
            vec![d]
        }
    }
}
#[test]
fn declaration_lookup_retains_invalid_ambiguous_and_cycle_rejection() {
    let k = FieldKey {
        owner: "Lp/Owner;".into(),
        name: "value".into(),
        descriptor: "I".into(),
    };
    assert!(matches!(
        resolve_field_declaration(&[], &k, &InvalidProvider(0), false),
        Err(FieldRequirement::Invalid(_))
    ));
    assert!(matches!(
        resolve_field_declaration(&[], &k, &InvalidProvider(1), false),
        Err(FieldRequirement::Ambiguous(_))
    ));
    assert!(matches!(
        resolve_field_declaration(&[], &k, &InvalidProvider(2), false),
        Err(FieldRequirement::Invalid(_))
    ));
}
