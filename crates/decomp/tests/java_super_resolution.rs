//! Exact inherited super dispatch and rejected incomplete evidence.
#![allow(clippy::unwrap_used)]
pub use decomp::{java_aliases, java_external_metadata, java_members};
#[allow(missing_docs)]
#[path = "../src/java_access.rs"]
pub mod java_access;
#[path = "../src/java_super_resolution.rs"]
mod java_super_resolution;
use decomp::java_aliases::{AliasInventory, AliasMode, MethodKey};
use decomp::java_external_metadata::Classpath;
use java_super_resolution::*;
fn key(owner: &str, name: &str, ret: &str) -> MethodKey {
    MethodKey {
        owner: owner.into(),
        name: name.into(),
        parameters: vec![],
        return_type: ret.into(),
    }
}
#[test]
fn actual_classfile_intermediate_override_dispatch_and_negative_proofs() {
    let root = std::env::temp_dir().join(format!(
        "decomphify-super-resolution-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let source = r#"public class SuperDispatch {
 public static class Base { protected int value(){return 1;} }
 public static class Mid extends Base {}
 public static class Child extends Mid { protected int value(){return 9;} int original(){return super.value();} }
 public static class OverrideMid extends Base { protected int value(){return 3;} }
 public static class OverrideChild extends OverrideMid { protected int value(){return 8;} int original(){return super.value();} }
 public static void main(String[] args){Child a=new Child();OverrideChild b=new OverrideChild();if(a.original()!=1||a.value()!=9||b.original()!=3||b.value()!=8)throw new AssertionError();}
}"#;
    std::fs::write(root.join("SuperDispatch.java"), source).unwrap();
    let jdk = "/home/jtjisgod/tools/jdk-17.0.13+11";
    assert!(
        std::process::Command::new(format!("{jdk}/bin/javac"))
            .arg(root.join("SuperDispatch.java"))
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new(format!("{jdk}/bin/java"))
            .args(["-cp", root.to_str().unwrap(), "SuperDispatch"])
            .status()
            .unwrap()
            .success()
    );
    let mut cp = Classpath::from_paths(
        std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "class")),
    )
    .unwrap();
    cp.add_path(format!("{jdk}/jmods/java.base.jmod")).unwrap();
    let index = SuperIndex::collect(&[], &cp).unwrap();
    let h = decomp::java_hierarchy::build_with_metadata(&[], &cp).unwrap();
    // Discover real ancestor members even when their exact owner is absent
    // from the input pool. This controlled DEX owner uses genuine classfiles.
    let mut descendant = dex_format::DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes_named.dex"
    ))
    .unwrap();
    descendant.classes.truncate(1);
    let name_idx = descendant
        .type_id(descendant.classes[0].class_idx)
        .unwrap()
        .descriptor_idx;
    descendant.strings[name_idx as usize].value = "LSuperDispatch$Child;".into();
    let mut ancestor_names = AliasInventory::default();
    inventory_ancestors(&[descendant], &index, &mut ancestor_names).unwrap();
    assert!(
        ancestor_names
            .build(AliasMode::Unicode)
            .method_name(&key("LSuperDispatch$Base;", "value", "I"))
            .is_some()
    );
    let mut names = AliasInventory::default();
    for c in cp.classes() {
        names.add_type(&c.descriptor).unwrap();
        for m in &c.methods {
            names.add_method(m.key.clone()).unwrap();
        }
    }
    let aliases = names.build(AliasMode::Unicode);
    let reference = key("LSuperDispatch$Base;", "value", "I");
    let selected = select_super("LSuperDispatch$Child;", &reference, &index, &aliases, &h).unwrap();
    assert_eq!(selected.target, reference);
    assert_eq!(selected.metadata.access_flags, 4);
    assert_eq!(
        selected.ancestry,
        vec!["LSuperDispatch$Mid;", "LSuperDispatch$Base;"]
    );
    let selected = select_super(
        "LSuperDispatch$OverrideChild;",
        &reference,
        &index,
        &aliases,
        &h,
    )
    .unwrap();
    assert_eq!(selected.target.owner, "LSuperDispatch$OverrideMid;");
    struct Mutated<'a> {
        base: &'a SuperIndex<'a>,
        flags: u32,
        generic: bool,
        duplicate: bool,
    }
    impl SuperMetadata for Mutated<'_> {
        fn class(&self, owner: &str) -> Option<SuperClass> {
            let mut c = self.base.class(owner)?;
            if owner == "LSuperDispatch$Base;" {
                let m = c.methods.iter_mut().find(|m| m.key.name == "value")?;
                m.metadata.access_flags = self.flags;
                m.generic = self.generic;
                let m = m.clone();
                if self.duplicate {
                    c.methods.push(m);
                }
            }
            Some(c)
        }
    }
    for flags in [2, 8, 0x400, 0x40, 0x80] {
        assert!(
            select_super(
                "LSuperDispatch$Child;",
                &reference,
                &Mutated {
                    base: &index,
                    flags,
                    generic: false,
                    duplicate: false
                },
                &aliases,
                &h
            )
            .is_err()
        );
    }
    assert_eq!(
        select_super(
            "LSuperDispatch$Child;",
            &reference,
            &Mutated {
                base: &index,
                flags: 4,
                generic: true,
                duplicate: false
            },
            &aliases,
            &h
        )
        .unwrap_err(),
        SuperError::Overload
    );
    struct Erased<'a> {
        original: Mutated<'a>,
        target: MethodKey,
    }
    impl SuperMetadata for Erased<'_> {
        fn class(&self, owner: &str) -> Option<SuperClass> {
            self.original.class(owner)
        }
        fn source_signature_is_erased(&self, method: &MethodKey) -> bool {
            method == &self.target
        }
    }
    // The genuine compiled fixture supplies a nongeneric source declaration;
    // this models input Signature retention separate from erased source syntax.
    let erased = Erased {
        original: Mutated {
            base: &index,
            flags: 4,
            generic: true,
            duplicate: false,
        },
        target: reference.clone(),
    };
    assert_eq!(
        select_super("LSuperDispatch$Child;", &reference, &erased, &aliases, &h)
            .unwrap()
            .target,
        reference
    );
    for flags in [0x40, 0x80] {
        let blocked = Erased {
            original: Mutated {
                base: &index,
                flags,
                generic: true,
                duplicate: false,
            },
            target: reference.clone(),
        };
        assert!(select_super("LSuperDispatch$Child;", &reference, &blocked, &aliases, &h).is_err());
    }
    assert_eq!(
        select_super(
            "LSuperDispatch$Child;",
            &reference,
            &Mutated {
                base: &index,
                flags: 4,
                generic: false,
                duplicate: true
            },
            &aliases,
            &h
        )
        .unwrap_err(),
        SuperError::Selection
    );
    let bad = key("Ljava/lang/String;", "value", "I");
    assert!(select_super("LSuperDispatch$Child;", &bad, &index, &aliases, &h).is_err());
    let wrong = key("LSuperDispatch$Base;", "value", "J");
    assert!(select_super("LSuperDispatch$Child;", &wrong, &index, &aliases, &h).is_err());
    struct Missing<'a>(&'a SuperIndex<'a>);
    impl SuperMetadata for Missing<'_> {
        fn class(&self, d: &str) -> Option<SuperClass> {
            if d == "LSuperDispatch$Mid;" {
                None
            } else {
                self.0.class(d)
            }
        }
    }
    assert_eq!(
        select_super(
            "LSuperDispatch$Child;",
            &reference,
            &Missing(&index),
            &aliases,
            &h
        )
        .unwrap_err(),
        SuperError::Metadata
    );
}

#[test]
#[ignore = "Requires preserved original Samsung Pass APK and local JDK/Android SDK"]
fn original_samsung_super_metadata_witnesses() {
    let dexes = dex_format::apk::parse_apk_path(
        "../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    let cp = Classpath::from_paths([
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod",
        "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
    ])
    .unwrap();
    let refs: Vec<_> = dexes.iter().collect();
    let index = SuperIndex::collect(&refs, &cp).unwrap();
    let hierarchy = decomp::java_hierarchy::build_with_metadata(&refs, &cp).unwrap();
    let mut names = AliasInventory::default();
    // Bounded witness naming catalog: exact callers and complete named-method
    // declarations on their real ancestor chains. Production supplies its own
    // complete frozen alias inventory; this probe proves metadata equivalence.
    for (pool, idx) in [(0, 228), (0, 262), (0, 396), (3, 31193)] {
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
    for (pool, idx) in [(0, 228), (0, 262), (0, 396), (3, 31193)] {
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
    assert_eq!(successes, 4);
}
