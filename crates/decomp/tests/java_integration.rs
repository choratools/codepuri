//! Actual facade output, not independently imported helper modules.
// Fixture construction and successful-result assertions intentionally panic on failure.
#![allow(clippy::unwrap_used)]
use decomp::{
    dex_java::{JavaOptions, emit_java},
    output::{DirectorySink, GeneratedSource, OutputError, SourceSink, SourceStatus},
};

fn pool() -> dex_format::DexFile {
    let apk = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk");
    let output = std::process::Command::new("unzip")
        .args(["-p"])
        .arg(apk)
        .arg("classes.dex")
        .output()
        .unwrap();
    assert!(output.status.success());
    dex_format::DexFile::from_bytes(&output.stdout).unwrap()
}
fn export(
    dex: dex_format::DexFile,
    filter: &str,
) -> (decomp::dex_java::JavaReport, Vec<GeneratedSource>) {
    let mut sources = Vec::new();
    let report = emit_java(
        &[dex],
        &JavaOptions {
            class_filter: Some(filter.into()),
            limit: None,
            workers: 1,
            source_release: 17,
            cache_ssa: true,
            cache_resolutions: true,
            cache_inventory: true,
            classpath: vec![],
        },
        &mut |s: &GeneratedSource| -> Result<(), OutputError> {
            sources.push(s.clone());
            Ok(())
        },
    )
    .unwrap();
    (report, sources)
}
#[test]
fn class_workers_preserve_metrics_order_bytes_and_limits() {
    let mut dex = pool();
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
    for class in &mut dex.classes {
        let data = class.class_data.as_mut().unwrap();
        data.direct_methods.clear();
        data.virtual_methods
            .retain(|m| selected.contains(&m.method_idx));
    }
    for limit in [None, Some(3), Some(0)] {
        let run = |workers, profiled, cache_ssa| {
            let mut sources = Vec::new();
            let options = JavaOptions {
                workers,
                limit,
                cache_ssa,
                cache_resolutions: cache_ssa,
                cache_inventory: cache_ssa,
                ..JavaOptions::default()
            };
            let mut sink = |s: &GeneratedSource| {
                sources.push((s.relative_path.clone(), s.text.clone()));
                Ok(())
            };
            let mut profile = decomp::dex_java::JavaProfile::default();
            let report = if profiled {
                decomp::dex_java::emit_java_profiled(
                    std::slice::from_ref(&dex),
                    &options,
                    &mut sink,
                    &mut profile,
                )
            } else {
                emit_java(std::slice::from_ref(&dex), &options, &mut sink)
            }
            .unwrap();
            if profiled {
                assert_eq!(profile.ssa_cache_enabled, cache_ssa);
                assert_eq!(profile.resolution_cache_enabled, cache_ssa);
                assert_eq!(profile.field_use_cache_enabled, cache_ssa);
                if cache_ssa && limit != Some(0) {
                    assert!(profile.resolution_cache_misses > 0);
                    assert!(profile.resolution_cache_hits > 0);
                } else if !cache_ssa {
                    assert_eq!(profile.resolution_cache_misses, 0);
                    assert_eq!(profile.resolution_cache_hits, 0);
                    assert_eq!(profile.resolution_cache_failures, 0);
                    assert_eq!(profile.field_use_cache_entries, 0);
                    assert_eq!(profile.field_use_cache_hits, 0);
                    assert_eq!(profile.field_use_cache_misses, 0);
                    assert_eq!(profile.field_use_cache_failures, 0);
                }
                if cache_ssa {
                    assert!(profile.ssa_cache_entries > 0);
                    assert!(profile.ssa_cache_hits > 0);
                    assert_eq!(profile.ssa_cache_entries, profile.ssa_cache_misses);
                } else {
                    assert_eq!(profile.ssa_cache_entries, 0);
                    assert_eq!(profile.ssa_cache_hits, 0);
                }
                assert_eq!(profile.classes.len(), dex.classes.len());
                assert_eq!(
                    profile.schedulers.iter().map(|s| s.tasks).sum::<usize>(),
                    dex.classes.len()
                );
                assert!(profile.batches.is_empty());
                assert_eq!(profile.schedulers.len(), 1);
                for s in &profile.schedulers {
                    assert_eq!(s.capacity, workers.min(dex.classes.len()));
                    assert_eq!(s.worker_time, s.task_time + s.queue_wait + s.dispatch_idle);
                    assert!(s.worker_time <= s.wall * s.capacity as u32);
                }
                assert!(profile.serial.contains_key("resolver_prepare"));
            }
            (format!("{report:?}"), sources)
        };
        let baseline = run(1, false, false);
        assert_eq!(baseline, run(30, false, false));
        assert_eq!(baseline, run(1, true, false));
        assert_eq!(baseline, run(30, true, false));
        assert_eq!(baseline, run(1, true, true));
        assert_eq!(baseline, run(30, true, true));
    }
}
#[test]
fn profiling_retains_completed_work_on_sink_error() {
    let dex = pool();
    let options = JavaOptions {
        class_filter: Some("android/support/customtabs/ICustomTabsCallback$Stub$Proxy;".into()),
        workers: 30,
        ..JavaOptions::default()
    };
    let mut calls = 0;
    let mut sink = |_: &GeneratedSource| {
        calls += 1;
        Err(OutputError::InvalidDescriptor(
            "intentional sink failure".into(),
        ))
    };
    let mut profile = decomp::dex_java::JavaProfile::default();
    let result = decomp::dex_java::emit_java_profiled(&[dex], &options, &mut sink, &mut profile);
    assert!(
        matches!(result, Err(OutputError::InvalidDescriptor(message)) if message == "intentional sink failure")
    );
    assert_eq!(calls, 1);
    assert!(!profile.wall.is_zero());
    assert!(profile.ssa_cache_enabled);
    assert!(profile.ssa_cache_entries > 0);
    assert!(profile.serial.contains_key("sink_io"));
    assert!(profile.batches.is_empty());
    assert_eq!(profile.schedulers.len(), 1);
    let scheduler = &profile.schedulers[0];
    // Global source closure audits all completed class publications before sink
    // delivery. A sink error must retain that real completed-work profile.
    assert_eq!(scheduler.tasks, profile.classes.len());
    assert_eq!(
        scheduler.worker_time,
        scheduler.task_time + scheduler.queue_wait + scheduler.dispatch_idle
    );
    let failing = profile
        .classes
        .iter()
        .find(|c| {
            c.descriptor
                .ends_with("android/support/customtabs/ICustomTabsCallback$Stub$Proxy;")
        })
        .unwrap();
    assert!(
        failing
            .descriptor
            .ends_with("android/support/customtabs/ICustomTabsCallback$Stub$Proxy;")
    );
    assert!(
        profile
            .classes
            .iter()
            .any(|c| c.ssa.contains_key("type_context_prepare"))
    );
}

#[test]
fn actual_apk_asbinder_requires_and_emits_its_genuine_field() {
    let (report, sources) = export(
        pool(),
        "android/support/customtabs/ICustomTabsCallback$Stub$Proxy;",
    );
    assert_eq!(sources.len(), 1);
    assert_eq!(report.fields_emitted, 1);
    assert!(report.complete_methods >= 1);
    let source = &sources[0];
    assert_eq!(source.provenance.status, SourceStatus::Fallback);
    assert!(source.text.contains("private android.os.IBinder"));
    assert!(source.text.contains("public android.os.IBinder asBinder()"));
    assert!(source.text.contains(" = this."));
    assert!(source.text.contains("return v0;"));
    assert!(
        source
            .text
            .contains("implements android.support.customtabs.ICustomTabsCallback")
    );
    assert!(report.unsupported_methods > 0);
    assert!(source.text.contains("unavailable original method"));
}
#[test]
fn whole_pool_aliases_are_shared_by_field_declaration_reference_and_output_path() {
    let mut dex = pool();
    let class = dex
        .classes
        .iter()
        .find(|c| {
            dex.string(dex.type_id(c.class_idx).unwrap().descriptor_idx)
                .unwrap()
                == "Landroid/support/customtabs/ICustomTabsCallback$Stub$Proxy;"
        })
        .unwrap();
    let field = class.class_data.as_ref().unwrap().instance_fields[0].field_idx;
    let name_index = dex.field_id(field).unwrap().name_idx;
    let class_descriptor = dex.type_id(class.class_idx).unwrap().descriptor_idx;
    dex.strings[name_index as usize].value = "class".into();
    dex.strings[class_descriptor as usize].value = "Lfixture/class;".into();
    let (_, sources) = export(dex, "Lfixture/class;");
    assert_eq!(sources.len(), 1);
    let source = &sources[0];
    assert_eq!(source.provenance.descriptor, "Lfixture/class;");
    assert!(source.relative_path.to_string_lossy().contains("fixture/"));
    assert!(
        !source
            .relative_path
            .to_string_lossy()
            .ends_with("/class.java")
    );
    let declaration = source
        .text
        .lines()
        .find(|l| l.contains("private android.os.IBinder"))
        .unwrap();
    let alias = declaration
        .split_whitespace()
        .last()
        .unwrap()
        .trim_end_matches(';');
    assert_ne!(alias, "class");
    assert!(source.text.contains(&format!("this.{alias}")));
    let dir = std::env::temp_dir().join(format!(
        "decomphify-alias-integration-{}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).unwrap();
    let mut sink = DirectorySink::new(&dir).unwrap();
    sink.emit(source).unwrap();
    sink.flush_pending().unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join(&source.relative_path)).unwrap(),
        source.text
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn facade_retains_final_field_as_explicit_partial_dependency() {
    let mut dex = pool();
    let class = dex
        .classes
        .iter_mut()
        .find(|c| {
            // This pool's class type index is resolved below to avoid mixing the
            // mutable class borrow with the owning DEX pools.
            c.class_data.as_ref().is_some_and(|d| {
                d.instance_fields.iter().any(|f| {
                    let field = &dex.field_ids[f.field_idx as usize];
                    dex.strings[dex.type_ids[field.class_idx as usize].descriptor_idx as usize]
                        .value
                        == "Landroid/support/customtabs/ICustomTabsCallback$Stub$Proxy;"
                })
            })
        })
        .unwrap();
    class.class_data.as_mut().unwrap().instance_fields[0].access_flags |= 0x10;
    let (report, sources) = export(
        dex,
        "android/support/customtabs/ICustomTabsCallback$Stub$Proxy;",
    );
    assert_eq!(report.fields_emitted, 1);
    assert!(
        sources
            .iter()
            .any(|s| s.text.contains("public android.os.IBinder asBinder()")
                && s.text.contains("final android.os.IBinder"))
    );
    assert!(report.unavailable_initialization_units > 0);
    assert!(
        sources
            .iter()
            .all(|s| s.provenance.status == decomp::output::SourceStatus::Fallback)
    );
}
