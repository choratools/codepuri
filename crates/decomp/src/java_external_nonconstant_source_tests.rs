//! Genuine external classfiles distinguish absent ConstantValue from unavailable metadata.
#![allow(clippy::unwrap_used)]
use super::*;
use dex_format::{
    RawSpan,
    ids::{FieldId, TypeId},
    strings::StringData,
};
fn references() -> DexFile {
    let mut dex = DexFile::from_bytes(&external_fixture::minimal_dex()).unwrap();
    let span = RawSpan { offset: 0, size: 0 };
    for (owner, name) in [
        ("Landroid/os/Build$VERSION;", "SDK_INT"),
        ("Ljava/lang/Integer;", "MAX_VALUE"),
        ("Lmissing/Unavailable;", "SDK_INT"),
    ] {
        let owner_string = dex.strings.len() as u32;
        dex.strings.push(StringData {
            value: owner.into(),
            utf16_size: owner.len() as u32,
            span,
        });
        let owner_type = dex.type_ids.len() as u16;
        dex.type_ids.push(TypeId {
            descriptor_idx: owner_string,
            span,
        });
        let type_string = dex.strings.len() as u32;
        dex.strings.push(StringData {
            value: "I".into(),
            utf16_size: 1,
            span,
        });
        let field_type = dex.type_ids.len() as u16;
        dex.type_ids.push(TypeId {
            descriptor_idx: type_string,
            span,
        });
        let name_string = dex.strings.len() as u32;
        dex.strings.push(StringData {
            value: name.into(),
            utf16_size: name.len() as u32,
            span,
        });
        dex.field_ids.push(FieldId {
            class_idx: owner_type,
            type_idx: field_type,
            name_idx: name_string,
            span,
        });
    }
    dex
}
#[test]
#[ignore = "requires genuine local Android35 and JDK17 metadata"]
fn genuine_android_sdk_int_nonconstant_hook_preserves_source_getstatic() {
    use crate::java_external_metadata::Classpath;
    use decomp_core::cfg::{Cfg, EdgeKind};
    use managed_ir::{
        method::{InsnArena, ValueArena},
        types::ManagedType,
        value::Value,
    };
    use std::process::Command;
    let dex = references();
    let mut names = crate::java_aliases::AliasInventory::default();
    for index in 0..dex.type_ids.len() {
        names
            .add_type(descriptor(&dex, index as u32).unwrap())
            .unwrap();
    }
    for index in 0..dex.field_ids.len() {
        names
            .add_field(field_key(&dex, index as u32).unwrap())
            .unwrap();
    }
    let mut aliases = names.build(AliasMode::Ascii);
    let metadata = dex_decompiler::infer::DexMetadata::new(&dex);
    let cp = Classpath::from_paths([
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod",
        "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
    ])
    .unwrap();
    let owners = dex
        .classes
        .iter()
        .filter_map(|class| descriptor(&dex, class.class_idx).map(str::to_owned))
        .collect();
    aliases.apply_external_member_names(&cp.member_source_names(), &owners);
    let sdk = cp
        .class("Landroid/os/Build$VERSION;")
        .unwrap()
        .fields
        .iter()
        .find(|f| f.name == "SDK_INT")
        .unwrap();
    assert_eq!(sdk.access_flags, 25);
    assert!(sdk.constant_value.is_none());
    let constant = cp
        .class("Ljava/lang/Integer;")
        .unwrap()
        .fields
        .iter()
        .find(|f| f.name == "MAX_VALUE")
        .unwrap();
    assert!(constant.constant_value.is_some());
    let mut resolver = Resolver::new(&dex, &aliases, &metadata);
    resolver.external = Some(&cp);
    assert!(resolver.field_source_nonconstant(FieldRef::new(0)));
    assert!(!resolver.field_source_nonconstant(FieldRef::new(1)));
    assert!(!resolver.field_source_nonconstant(FieldRef::new(2)));
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let load = insns.insert(ManagedInsn::Return { value: None });
    let value = values.insert(Value::new(load, ManagedType::Int));
    insns[load] = ManagedInsn::FieldGet {
        result: value,
        object: None,
        field: FieldRef::new(0),
    };
    let ret = insns.insert(ManagedInsn::Return { value: Some(value) });
    let mut cfg = Cfg::new(load);
    let exit = cfg.add_block(ret);
    cfg.add_edge(cfg.entry(), exit, EdgeKind::Unconditional)
        .unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    let method = JavaMethod {
        owner: "SdkGetter".into(),
        name: "read".into(),
        parameters: vec![],
        return_type: JavaType::Int,
        is_static: true,
        throws: vec![],
    };
    let source = backend_java::emit_method(
        &ir,
        &method,
        &backend_java::ParameterBindings::default(),
        &resolver,
    );
    let SourceResult::Complete { source, .. } = source else {
        panic!("{source:?}")
    };
    assert!(source.contains("SDK_INT"));
    let dir = std::env::temp_dir().join(format!("decomphify-sdk-getstatic-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SdkGetter.java"),
        format!("class SdkGetter {{ {source} }}"),
    )
    .unwrap();
    let compile = Command::new("javac")
        .args([
            "-classpath",
            "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
            "SdkGetter.java",
        ])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let inspect = Command::new("javap")
        .args(["-c", "SdkGetter"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let bytecode = String::from_utf8(inspect.stdout).unwrap();
    assert!(
        bytecode.contains("getstatic") && bytecode.contains("Build$VERSION.SDK_INT:I"),
        "{bytecode}"
    );
    std::fs::remove_dir_all(dir).unwrap();
    resolver.external = None;
    assert!(!resolver.field_source_nonconstant(FieldRef::new(0)));
}

#[test]
fn unresolved_external_constant_metadata_stays_closed() {
    let dex = references();
    let mut names = crate::java_aliases::AliasInventory::default();
    for index in 0..dex.type_ids.len() {
        names
            .add_type(descriptor(&dex, index as u32).unwrap())
            .unwrap();
    }
    for index in 0..dex.field_ids.len() {
        names
            .add_field(field_key(&dex, index as u32).unwrap())
            .unwrap();
    }
    let aliases = names.build(AliasMode::Ascii);
    let metadata = dex_decompiler::infer::DexMetadata::new(&dex);
    let resolver = Resolver::new(&dex, &aliases, &metadata);
    for index in 0..dex.field_ids.len() {
        assert!(!resolver.field_source_nonconstant(FieldRef::new(index as u32)));
    }
}
