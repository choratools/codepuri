#![allow(clippy::unwrap_used)]
use super::*;
fn key(owner: &str) -> MethodKey {
    MethodKey {
        owner: owner.into(),
        name: "write".into(),
        parameters: vec!["I".into()],
        return_type: "V".into(),
    }
}
fn declaration(owner: &str, flags: u32) -> ParsedMethod {
    ParsedMethod {
        key: key(owner),
        metadata: MethodMetadata {
            access_flags: flags,
            exceptions: Some(vec![]),
        },
        descriptor: "(I)V".into(),
        signature: None,
        code: None,
    }
}
fn fixture(owner: &str, flags: u32, shadow: bool) -> Option<Class> {
    match owner {
        "Lfixture/Child;" => Some(Class {
            parent: Some("Lfixture/Parent;".into()),
            methods: if shadow { vec![key(owner)] } else { vec![] },
            declarations: None,
        }),
        "Lfixture/Parent;" => Some(Class {
            parent: Some("Lfixture/Base;".into()),
            methods: vec![key(owner)],
            declarations: Some(vec![declaration(owner, flags)]),
        }),
        "Lfixture/Base;" => Some(Class {
            parent: None,
            methods: vec![key(owner)],
            declarations: Some(vec![declaration(owner, 1)]),
        }),
        _ => None,
    }
}
#[test]
fn closest_public_class_static_contract_preserves_symbolic_identity() {
    let symbolic = key("Lfixture/Base;");
    let selected = select(&symbolic, "Lfixture/Child;", |o| fixture(o, 1, false)).unwrap();
    assert_eq!(selected.key.owner, "Lfixture/Parent;");
    assert_eq!(symbolic.owner, "Lfixture/Base;");
    for flags in [2, 4, 9, 0x401] {
        assert!(select(&symbolic, "Lfixture/Child;", |o| fixture(o, flags, false)).is_none());
    }
    assert!(select(&symbolic, "Lfixture/Child;", |o| fixture(o, 1, true)).is_none());
    assert!(
        select(&symbolic, "Lfixture/Child;", |o| {
            let mut c = fixture(o, 1, false)?;
            if o == "Lfixture/Parent;" {
                c.declarations.as_mut()?[0].signature = Some("(TT;)V".into());
            }
            Some(c)
        })
        .is_none()
    );
    assert!(
        select(&symbolic, "Lfixture/Child;", |o| {
            let mut c = fixture(o, 1, false)?;
            if o == "Lfixture/Parent;" {
                c.declarations.as_mut()?[0].key.return_type = "I".into();
            }
            Some(c)
        })
        .is_none()
    );
    assert!(
        select(&symbolic, "Lfixture/Child;", |o| {
            let mut c = fixture(o, 1, false)?;
            if o == "Lfixture/Parent;" {
                c.parent = Some("Lfixture/Child;".into());
            }
            Some(c)
        })
        .is_none()
    );
    assert!(
        select(&symbolic, "Lfixture/Child;", |o| if o == "Lfixture/Base;" {
            None
        } else {
            fixture(o, 1, false)
        })
        .is_none()
    );
}

#[test]
#[ignore = "requires genuine Samsung Pass APK and explicit JDK17 metadata"]
fn actual_null_digest_selected_receivers_and_original_emission() {
    use crate::java_external_metadata::{Classpath, fields::InputIndex};
    use std::process::Command;
    let files = dex_format::apk::parse_apk_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
    )
    .unwrap();
    let cp = Classpath::from_paths(["/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod"])
        .unwrap();
    let mut inventory = AliasInventory::default();
    for dex in &files {
        for ty in &dex.type_ids {
            inventory
                .add_type(dex.string(ty.descriptor_idx).unwrap())
                .unwrap();
        }
        for i in 0..dex.method_ids.len() {
            inventory
                .add_method(method_key(dex, i as u32).unwrap())
                .unwrap();
        }
        for i in 0..dex.field_ids.len() {
            inventory
                .add_field(field_key(dex, i as u32).unwrap())
                .unwrap();
        }
    }
    let aliases = inventory.build(AliasMode::Ascii);
    let catalog = method_catalog(&files);
    let dex = &files[3];
    let metadata = dex_decompiler::infer::DexMetadata::new(dex);
    let field_index = InputIndex::new(&files);
    let mut r = Resolver::with_catalog(dex, &aliases, &metadata, Some(&catalog));
    r.inputs = &files;
    r.external = Some(&cp);
    r.field_index = Some(&field_index);
    r.input_owners = files
        .iter()
        .flat_map(|d| {
            d.classes
                .iter()
                .filter_map(|c| descriptor(d, c.class_idx).map(str::to_owned))
        })
        .collect();
    r.hierarchy =
        crate::java_hierarchy::build_hierarchy(&files.iter().collect::<Vec<_>>(), cp.hierarchy())
            .unwrap()
            .map_names(|n| {
                aliases
                    .class_name(&format!("L{};", n.replace('.', "/")))
                    .unwrap_or(n)
                    .into()
            })
            .unwrap();
    let owner = "Lorg/bouncycastle/crypto/digests/NullDigest;";
    let receiver = JavaType::Class(
        aliases
            .class_name("Lorg/bouncycastle/crypto/digests/NullDigest$OpenByteArrayOutputStream;")
            .unwrap()
            .into(),
    );
    let caller = aliases.class_name(owner).unwrap();
    let mut bodies = vec![];
    for index in [9842, 9843] {
        let class = dex
            .classes
            .iter()
            .find(|c| descriptor(dex, c.class_idx) == Some(owner))
            .unwrap();
        let original = class
            .class_data
            .as_ref()
            .unwrap()
            .virtual_methods
            .iter()
            .find(|m| m.method_idx == index)
            .unwrap();
        let code = original.code.as_ref().unwrap();
        let (ctx, seeds) = initial_context_with_seeds(&r, original).unwrap();
        let ssa = cached_frontend(&r, original, code, &ctx, seeds, None).unwrap();
        let call = ssa
            .ir()
            .instructions()
            .iter()
            .find_map(|(_, i)| {
                if let ManagedInsn::Invoke {
                    kind: InvokeKind::Virtual,
                    method,
                    ..
                } = i
                {
                    Some(*method)
                } else {
                    None
                }
            })
            .unwrap();
        let symbolic = r.method(call).unwrap();
        assert_eq!(symbolic.owner, "java.io.OutputStream");
        assert_eq!(symbolic.throws, vec!["java.io.IOException"]);
        let selected = r.virtual_receiver_method(call, caller, &receiver).unwrap();
        assert_eq!(selected.owner, "java.io.ByteArrayOutputStream");
        assert!(selected.throws.is_empty());
        assert!(
            r.virtual_receiver_method(
                call,
                caller,
                &JavaType::Class("java.io.OutputStream".into())
            )
            .is_none()
        );
        let sig = r.method(MethodRef::new(index)).unwrap();
        assert!(sig.throws.is_empty());
        let bindings = declaration_bindings(&ssa, &sig, code).unwrap();
        let result = backend_java::emit_method_declaration(ssa.ir(), &sig, &bindings, &r);
        let SourceResult::Complete { source, .. } = result else {
            panic!("{index} {result:?}");
        };
        assert!(!source.contains("((java.io.OutputStream)"), "{source}");
        assert!(!source.contains("throws java.io.IOException"), "{source}");
        println!("ORIGINAL {index} target={selected:?}\n{source}");
        bodies.push(source);
    }
    // Explicit boundary fixture supplies only the receiver's original inherited
    // static type; the two method declarations/bodies above remain unchanged.
    let dir = std::env::temp_dir().join(format!(
        "decomphify-null-digest-contract-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let text = format!(
        "package org.bouncycastle.crypto.digests; class NullDigest$OpenByteArrayOutputStream extends java.io.ByteArrayOutputStream {{ }} public class NullDigest {{ NullDigest$OpenByteArrayOutputStream bOut=new NullDigest$OpenByteArrayOutputStream(); {} public static void main(String[] args) {{ NullDigest d=new NullDigest(); d.update((byte)65); d.update(new byte[]{{66,67}},0,2); if(!java.util.Arrays.equals(d.bOut.toByteArray(),new byte[]{{65,66,67}}))throw new AssertionError(); }} }}",
        bodies.join("\n")
    );
    std::fs::write(dir.join("NullDigest.java"), text).unwrap();
    let out = Command::new("/home/jtjisgod/tools/jdk-17.0.13+11/bin/javac")
        .args(["-d", ".", "NullDigest.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = Command::new("/home/jtjisgod/tools/jdk-17.0.13+11/bin/java")
        .args(["-cp", ".", "org.bouncycastle.crypto.digests.NullDigest"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
