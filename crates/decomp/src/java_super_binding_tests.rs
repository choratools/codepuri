//! Selected superclass declarations stay separate from ordinary pool references.
#![allow(clippy::unwrap_used)]
use super::*;
use dex_format::{
    RawSpan,
    class::ClassData,
    code::CodeItem,
    ids::{MethodId, ProtoId, TypeId},
    strings::StringData,
};
use std::{path::PathBuf, process::Command};

fn code(words: &[u16], registers: u16) -> CodeItem {
    CodeItem {
        registers_size: registers,
        ins_size: 1,
        outs_size: 1,
        debug_info_off: 0,
        insns_size: words.len() as u32,
        insns: words.iter().flat_map(|word| word.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: RawSpan { offset: 0, size: 0 },
    }
}
fn fixture() -> DexFile {
    let mut dex = DexFile::from_bytes(&super::index_tests::fixture::minimal_dex()).unwrap();
    let span = RawSpan { offset: 0, size: 0 };
    dex.strings[0].value = "LChild;".into();
    for text in [
        "LBase;",
        "LMid;",
        "Ljava/lang/Object;",
        "I",
        "V",
        "probe",
        "choose",
        "normal",
        "<init>",
    ] {
        dex.strings.push(StringData {
            value: text.into(),
            utf16_size: text.len() as u32,
            span,
        });
    }
    for descriptor_idx in 1..=5 {
        dex.type_ids.push(TypeId {
            descriptor_idx,
            span,
        });
    }
    for return_type_idx in [4, 5] {
        dex.proto_ids.push(ProtoId {
            shorty_idx: return_type_idx,
            return_type_idx,
            parameters_off: 0,
            parameters: None,
            span,
        });
    }
    for (class_idx, proto_idx, name_idx) in [
        (0, 0, 6),
        (1, 0, 7),
        (0, 0, 7),
        (0, 0, 8),
        (2, 1, 9),
        (0, 1, 9),
    ] {
        dex.method_ids.push(MethodId {
            class_idx,
            proto_idx,
            name_idx,
            span,
        });
    }
    dex.classes[0].superclass_idx = 2;
    let method = |index, words: &[u16], registers, flags| EncodedMethod {
        method_idx: index,
        access_flags: flags,
        code_off: index + 1,
        code: Some(code(words, registers)),
    };
    dex.classes[0].class_data = Some(ClassData {
        static_fields: vec![],
        instance_fields: vec![],
        direct_methods: vec![method(5, &[0x1070, 4, 0, 0x000e], 1, 0x10001)],
        virtual_methods: vec![
            method(0, &[0x106f, 1, 0, 0x000a, 0x000f], 1, 1),
            method(2, &[0x0013, 9, 0x000f], 2, 1),
            method(3, &[0x106e, 1, 0, 0x000a, 0x000f], 1, 1),
        ],
        span,
    });
    dex
}
fn jdk() -> PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into())
}

#[test]
fn actual_facade_inventories_missing_selected_owner_and_preserves_both_dispatches() {
    let root =
        std::env::temp_dir().join(format!("decomphify-super-binding-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("Base.java"),
        "public class Base { public int choose(){return 1;} }",
    )
    .unwrap();
    std::fs::write(
        root.join("Mid.java"),
        "public class Mid extends Base { @Override public int choose(){return 3;} }",
    )
    .unwrap();
    assert!(
        Command::new(jdk().join("bin/javac"))
            .arg("--release")
            .arg("17")
            .arg(root.join("Base.java"))
            .arg(root.join("Mid.java"))
            .status()
            .unwrap()
            .success()
    );
    let classpath = vec![
        root.join("Base.class"),
        root.join("Mid.class"),
        jdk().join("jmods/java.base.jmod"),
    ];
    let external = crate::java_external_metadata::Classpath::from_paths(&classpath).unwrap();
    let files = [fixture()];
    let views = files.iter().collect::<Vec<_>>();
    let index = crate::java_super_resolution::SuperIndex::collect(&views, &external).unwrap();
    let writes = initializers::write_inventory::WriteInventory::scan(&files);
    let (aliases, _) = export_inventory(&files, &mut None, &writes, Some(&index)).unwrap();
    let selected_key = MethodKey {
        owner: "LMid;".into(),
        name: "choose".into(),
        parameters: vec![],
        return_type: "I".into(),
    };
    assert!(
        !files[0]
            .method_ids
            .iter()
            .enumerate()
            .any(|(i, _)| method_key(&files[0], i as u32) == Some(selected_key.clone()))
    );
    assert_eq!(
        aliases.method_name(&selected_key),
        Some(&MethodAlias::Ordinary("choose".into()))
    );
    let global = GlobalDeclarations::new(&files).unwrap();
    let metadata = dex_decompiler::infer::DexMetadata::new(&files[0]);
    let mut resolver = Resolver::with_catalog(
        &files[0],
        &aliases,
        &metadata,
        Some(&global.declaration_metadata),
    );
    resolver.external = Some(&external);
    resolver.inputs = &files;
    resolver.input_owners.insert("LChild;".into());
    resolver.super_index = Some(&index);
    resolver.hierarchy = crate::java_hierarchy::build_with_metadata(&views, &external).unwrap();
    resolver.method_cache = Some(resolution_cache::LookupCache::new(
        true,
        files[0].method_ids.len(),
    ));
    resolver.cache_super_bindings = true;
    let binding = resolver.bound_super("Child", MethodRef::new(1)).unwrap();
    assert_eq!(binding.key, selected_key);
    assert_eq!(binding.method.owner, "Mid");
    assert_eq!(resolver.method(MethodRef::new(1)).unwrap().owner, "Base");
    assert!(resolver.bound_super("Base", MethodRef::new(1)).is_none());
    assert_eq!(
        resolver
            .bound_super("Child", MethodRef::new(1))
            .unwrap()
            .method
            .owner,
        "Mid"
    );
    let mut units = Vec::new();
    let report = crate::dex_java::emit_java(
        &files,
        &JavaOptions {
            classpath,
            ..JavaOptions::default()
        },
        &mut |source: &GeneratedSource| {
            units.push(source.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(report.complete_methods, 3);
    assert_eq!(report.complete_constructors, 1);
    assert_eq!(report.unsupported_methods, 0);
    assert!(units[0].text.contains("super.choose()"));
    std::fs::write(root.join("Child.java"), &units[0].text).unwrap();
    std::fs::write(root.join("Run.java"), "public class Run {public static void main(String[]a){Child c=new Child();if(c.probe()!=3||c.normal()!=9||c.choose()!=9)throw new AssertionError();System.out.print(\"3:9\");}}").unwrap();
    assert!(
        Command::new(jdk().join("bin/javac"))
            .args(["--release", "17", "-cp"])
            .arg(&root)
            .arg(root.join("Child.java"))
            .arg(root.join("Run.java"))
            .status()
            .unwrap()
            .success()
    );
    let result = Command::new(jdk().join("bin/java"))
        .arg("-cp")
        .arg(&root)
        .arg("Run")
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(result.stdout, b"3:9");
}
