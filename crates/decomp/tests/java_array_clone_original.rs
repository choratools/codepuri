//! Original Samsung Pass array-owner clone regression probes.
#![allow(clippy::unwrap_used)]

use decomp::{
    dex_java::{JavaOptions, emit_java},
    output::GeneratedSource,
};
use std::path::PathBuf;

const APK: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
);

#[test]
#[ignore = "requires retained Samsung Pass APK and local JDK metadata"]
fn bouncycastle_array_owner_clone_methods_publish() {
    let inputs = dex_format::apk::parse_apk_path(APK).unwrap();
    let mut units = Vec::<GeneratedSource>::new();
    let options = JavaOptions {
        class_filter: Some("Lorg/bouncycastle/util/Arrays;".into()),
        workers: 1,
        classpath: vec![PathBuf::from(
            "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod",
        )],
        ..JavaOptions::default()
    };
    let report = emit_java(&inputs, &options, &mut |unit: &GeneratedSource| {
        units.push(unit.clone());
        Ok(())
    })
    .unwrap();
    assert_eq!(units.len(), 1, "report={report:?}");
    let source = &units[0].text;
    for index in [29375, 29377, 29378, 29379, 29381, 29382, 29383] {
        let marker = format!("original method pool index {index};");
        assert!(source.contains(&marker), "missing {marker}\n{source}");
        let unavailable = format!("unavailable original method index {index}:");
        assert!(!source.contains(&unavailable), "still rejected {index}");
    }
    assert!(source.contains("p0.clone()"), "{source}");
    assert!(!source.contains("CloneNotSupportedException"), "{source}");
}
