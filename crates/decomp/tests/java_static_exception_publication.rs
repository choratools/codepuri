//! Original static exception scopes must reach the production source publisher.
#![allow(clippy::unwrap_used)]
use decomp::{
    dex_java::{JavaOptions, emit_java},
    output::GeneratedSource,
};

#[test]
#[ignore = "local Samsung Pass APK and genuine JDK17/Android35 classpath"]
fn original_rounding_table_exception_initializer_is_published_and_compiles() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let files = dex_format::apk::parse_apk_path(
        root.join("tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
    )
    .unwrap();
    let dex = &files[0];
    let class = dex
        .classes
        .iter()
        .find(|class| {
            class.class_data.as_ref().is_some_and(|data| {
                data.direct_methods
                    .iter()
                    .any(|method| method.method_idx == 34895)
            })
        })
        .unwrap();
    let owner = dex
        .string(dex.type_ids[class.class_idx as usize].descriptor_idx)
        .unwrap();
    assert_eq!(owner, "Lcom/google/common/math/b$a;");
    let method = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|method| method.method_idx == 34895)
        .unwrap();
    assert_eq!(method.code.as_ref().unwrap().tries.len(), 8);
    let options = JavaOptions {
        class_filter: Some(owner.into()),
        classpath: vec![
            "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod".into(),
            "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar".into(),
        ],
        ..JavaOptions::default()
    };
    let mut sources = Vec::new();
    let report = emit_java(&files, &options, &mut |source: &GeneratedSource| {
        sources.push(source.clone());
        Ok(())
    })
    .unwrap();
    assert_eq!(report.complete_static_initializers, 1);
    let source = sources
        .iter()
        .find(|source| source.provenance.descriptor == owner)
        .unwrap();
    assert!(source.text.contains("catch"));
    assert!(source.text.contains("java.lang.NoSuchFieldError"));
    assert!(source.text.contains("static final int[]"));
    let output =
        std::env::temp_dir().join(format!("wave7-static-publication-{}", std::process::id()));
    std::fs::create_dir_all(&output).unwrap();
    let path = output.join(&source.relative_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &source.text).unwrap();
    let compilation = std::process::Command::new("/home/jtjisgod/tools/jdk-17.0.13+11/bin/javac")
        .args(["--release", "17", "-sourcepath", "", "-d"])
        .arg(output.join("compiled"))
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        compilation.status.success(),
        "{}",
        String::from_utf8_lossy(&compilation.stderr)
    );
    println!(
        "ORIGINAL STATIC PUBLISHED pool=0 index=34895 source={} bytes={}",
        path.display(),
        source.text.len()
    );
}
