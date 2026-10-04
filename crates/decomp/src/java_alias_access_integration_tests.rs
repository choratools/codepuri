#![allow(clippy::unwrap_used)]
use super::*;

#[test]
fn aliased_cross_package_protected_super_uses_original_hierarchy() {
    use dex_format::{RawSpan, ids::TypeId, strings::StringData};
    let span = RawSpan { offset: 0, size: 0 };
    let mut parent = small_dex();
    parent.strings[0].value = "Lp/a;".into();
    parent.classes[0].access_flags = 1;
    parent.proto_ids[0].return_type_idx = 4;
    let data = parent.classes[0].class_data.as_mut().unwrap();
    data.direct_methods.truncate(1);
    data.direct_methods[0].access_flags = 4;
    let code = data.direct_methods[0].code.as_mut().unwrap();
    code.registers_size = 2;
    code.ins_size = 1;
    code.insns = [0x7012u16, 0x000f]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    code.insns_size = 2;
    data.virtual_methods = std::mem::take(&mut data.direct_methods);

    let mut child = small_dex();
    child.strings[0].value = "Lq/a;".into();
    child.classes[0].access_flags = 1;
    child.proto_ids[0].return_type_idx = 4;
    // A root package named `a` forces both input classes' simple `a` names
    // to change. A nested package p/a alone does not trigger that rule.
    for text in ["Lp/a;", "La/Unused;", "Lq/a/Unused;"] {
        let descriptor_idx = child.strings.len() as u32;
        child.strings.push(StringData {
            value: text.into(),
            utf16_size: text.len() as u32,
            span,
        });
        child.type_ids.push(TypeId {
            descriptor_idx,
            span,
        });
    }
    child.classes[0].superclass_idx = 5;
    child.method_ids[0].name_idx = 3;
    child.method_ids[1].class_idx = 5;
    child.method_ids[1].name_idx = 2;
    let data = child.classes[0].class_data.as_mut().unwrap();
    data.direct_methods.truncate(1);
    data.direct_methods[0].access_flags = 1;
    let code = data.direct_methods[0].code.as_mut().unwrap();
    code.registers_size = 2;
    code.ins_size = 1;
    code.outs_size = 1;
    code.insns = [0x106fu16, 1, 1, 0x000a, 0x000f]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    code.insns_size = 5;
    data.virtual_methods = std::mem::take(&mut data.direct_methods);
    let mut sources = Vec::new();
    let report = crate::dex_java::emit_java(
        &[parent, child],
        &JavaOptions::default(),
        &mut |source: &GeneratedSource| {
            sources.push(source.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(report.complete_methods, 2, "{report:?}");
    let child = sources
        .iter()
        .find(|source| source.relative_path.starts_with("q"))
        .unwrap();
    assert!(child.text.contains("a$type"), "{}", child.text);
    assert!(child.text.contains("super.first("), "{}", child.text);
    let dir = std::env::temp_dir().join(format!("aliased-protected-super-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut paths = Vec::new();
    for source in &sources {
        let path = dir.join(&source.relative_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &source.text).unwrap();
        paths.push(path);
    }
    let javac = std::env::var_os("JAVA_HOME")
        .map(|p| std::path::PathBuf::from(p).join("bin/javac"))
        .unwrap_or_else(|| "javac".into());
    let output = std::process::Command::new(javac)
        .arg("-d")
        .arg(&dir)
        .args(paths)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
