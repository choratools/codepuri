//! Real APK corpus integration test.

use dex_format::apk::parse_apk_path;
use std::fs;
use std::path::PathBuf;

#[test]
fn parses_every_dex_from_real_apk_corpus() -> Result<(), Box<dyn std::error::Error>> {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/apks");
    let entries = fs::read_dir(&corpus)?;
    let mut apk_count = 0_usize;

    for entry in entries {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "apk") {
            apk_count += 1;
            let dex_files = parse_apk_path(&path)?;
            assert!(!dex_files.is_empty());
            assert!(dex_files.iter().all(|dex| !dex.classes.is_empty()));
        }
    }

    assert!(apk_count > 0);
    Ok(())
}
