//! Persistent scheduling through the public Java export facade.
#![allow(clippy::unwrap_used)]
use decomp::{
    dex_java::{JavaOptions, JavaProfile, emit_java_profiled},
    output::GeneratedSource,
};

#[test]
fn persistent_facade_one_and_thirty_have_identical_sources_metrics_and_limits() {
    let apk = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk");
    let bytes = std::process::Command::new("unzip")
        .arg("-p")
        .arg(apk)
        .arg("classes.dex")
        .output()
        .unwrap();
    assert!(bytes.status.success());
    let mut dex = dex_format::DexFile::from_bytes(&bytes.stdout).unwrap();
    let selected: std::collections::BTreeSet<_> = dex
        .method_ids
        .iter()
        .enumerate()
        .filter(|(_, m)| dex.string(m.name_idx).unwrap() == "asBinder")
        .map(|(i, _)| i as u32)
        .collect();
    dex.classes.retain(|c| {
        c.class_data.as_ref().is_some_and(|d| {
            d.virtual_methods
                .iter()
                .any(|m| selected.contains(&m.method_idx))
        })
    });
    dex.classes.truncate(65);
    assert!(dex.classes.len() > 30);
    for class in &mut dex.classes {
        let data = class.class_data.as_mut().unwrap();
        data.direct_methods.clear();
        data.virtual_methods
            .retain(|m| selected.contains(&m.method_idx));
    }
    for limit in [None, Some(3), Some(0)] {
        let run = |workers| {
            let mut sources = Vec::new();
            let mut profile = JavaProfile::default();
            let report = emit_java_profiled(
                std::slice::from_ref(&dex),
                &JavaOptions {
                    workers,
                    limit,
                    ..Default::default()
                },
                &mut |s: &GeneratedSource| {
                    sources.push((s.relative_path.clone(), s.text.clone()));
                    Ok(())
                },
                &mut profile,
            )
            .unwrap();
            assert!(profile.batches.is_empty());
            assert_eq!(profile.schedulers.len(), 1);
            assert_eq!(profile.schedulers[0].tasks, dex.classes.len());
            assert_eq!(
                profile.classes.iter().map(|c| c.class).collect::<Vec<_>>(),
                dex.classes.iter().map(|c| c.class_idx).collect::<Vec<_>>()
            );
            (format!("{report:?}"), sources)
        };
        assert_eq!(run(1), run(30));
    }
}
