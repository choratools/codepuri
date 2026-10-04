//! Read-only candidate recognition and the pre-super outer-capture semantic boundary.
#![allow(clippy::unwrap_used)]
#[path = "../src/java_inner_classes.rs"]
mod inner;
use dex_format::{
    DexFile, annotations::Visibility, class::ClassDef, encoded_value::EncodedValueKind,
};
use inner::*;
use std::sync::OnceLock;
fn corpus() -> &'static (Vec<DexFile>, OuterCaptureInventory) {
    static DATA: OnceLock<(Vec<DexFile>, OuterCaptureInventory)> = OnceLock::new();
    DATA.get_or_init(|| {
        let files = dex_format::apk::parse_apk_path(std::path::Path::new(
            "../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
        ))
        .unwrap();
        let inventory = OuterCaptureInventory::scan(&files);
        (files, inventory)
    })
}
fn class_for(files: &[DexFile], idx: u32) -> &ClassDef {
    files[1]
        .classes
        .iter()
        .find(|c| {
            c.class_data
                .as_ref()
                .is_some_and(|d| d.direct_methods.iter().any(|m| m.method_idx == idx))
        })
        .unwrap()
}
#[test]
#[ignore = "explicit local Samsung APK corpus"]
fn samsung_system_annotations_and_original_first_stores_are_recognized() {
    let (files, inv) = corpus();
    {
        let idx = 19980;
        let class = class_for(files, idx);
        let candidate = recognize_outer_capture(files, 1, class, idx, inv).unwrap();
        assert!(matches!(candidate.kind, InnerClassKind::Anonymous { .. }));
        assert_eq!(candidate.capture_offset, 0);
        assert_eq!(candidate.super_offset, 2);
        assert!(
            candidate
                .descriptor
                .starts_with("Lcom/samsung/android/samsungpass/application/")
        );
        assert!(candidate.unknown_write_bodies > 0);
        println!("{candidate:?}");
    }
    // Kotlin's static anonymous closure has an explicit captured object, not
    // evidence of a Java implicit enclosing-instance class.
    assert_eq!(
        recognize_outer_capture(files, 1, class_for(files, 20027), 20027, inv),
        Err(InnerClassError::Static)
    );
}
#[test]
#[ignore = "explicit local Samsung APK corpus"]
fn malformed_static_visibility_duplicate_metadata_and_mutated_body_reject() {
    let (files, inv) = corpus();
    let original = class_for(files, 19980);
    let mut class = original.clone();
    let entries = &mut class
        .annotations
        .as_mut()
        .unwrap()
        .class_annotations
        .as_mut()
        .unwrap()
        .entries;
    let item = entries
        .iter_mut()
        .find(|a| {
            a.annotation
                .elements
                .iter()
                .any(|e| files[1].string(e.name_idx).unwrap() == "accessFlags")
        })
        .unwrap();
    for e in &mut item.annotation.elements {
        if files[1].string(e.name_idx).unwrap() == "accessFlags" {
            e.value.value = EncodedValueKind::Int(9);
        }
    }
    assert_eq!(
        recognize_outer_capture(files, 1, &class, 19980, inv),
        Err(InnerClassError::Static)
    );
    let mut class = original.clone();
    let entries = &mut class
        .annotations
        .as_mut()
        .unwrap()
        .class_annotations
        .as_mut()
        .unwrap()
        .entries;
    entries[0].visibility = Visibility::Runtime;
    assert!(recognize_outer_capture(files, 1, &class, 19980, inv).is_err());
    let mut class = original.clone();
    let entries = &mut class
        .annotations
        .as_mut()
        .unwrap()
        .class_annotations
        .as_mut()
        .unwrap()
        .entries;
    entries.push(entries[0].clone());
    assert!(recognize_outer_capture(files, 1, &class, 19980, inv).is_err());
    let mut class = original.clone();
    class
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .iter_mut()
        .find(|m| m.method_idx == 19980)
        .unwrap()
        .code
        .as_mut()
        .unwrap()
        .insns[0] = 0;
    assert!(recognize_outer_capture(files, 1, &class, 19980, inv).is_err());
    let mut class = original.clone();
    class.class_data.as_mut().unwrap().instance_fields[0].access_flags &= !0x1000;
    assert!(recognize_outer_capture(files, 1, &class, 19980, inv).is_err());
}
#[test]
fn java_nested_capture_exists_during_super_callback() {
    let root = std::env::temp_dir().join(format!("inner-capture-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = r#"public class Outer {final int number;Outer(int n){number=n;}abstract static class Base {Base(){if(read()!=37)throw new AssertionError();}abstract int read();}class Member extends Base {int read(){return Outer.this.number;}}Object local(){class Local extends Base {int read(){return Outer.this.number;}}return new Local();}Object anonymous(){return new Base(){int read(){return number;}};}static class Flattened extends Base {final Outer outer;Flattened(Outer o){super();outer=o;}int read(){return outer.number;}}public static void main(String[]x){Outer o=new Outer(37);o.new Member();o.local();o.anonymous();try{new Flattened(o);throw new AssertionError("late capture accepted");}catch(NullPointerException expected){}}}"#;
    std::fs::write(root.join("Outer.java"), source).unwrap();
    for (tool, args) in [
        ("javac", vec!["Outer.java"]),
        ("java", vec!["-cp", ".", "Outer"]),
    ] {
        let out =
            std::process::Command::new(format!("/home/jtjisgod/tools/jdk-17.0.13+11/bin/{tool}"))
                .args(args)
                .current_dir(&root)
                .output()
                .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
