//! Immutable generated-source reuse and output failure contracts.
#![allow(clippy::unwrap_used)]
use decomp::{
    dex_java::{JavaOptions, emit_java},
    java_session::JavaSession,
    output::{GeneratedSource, OutputError},
};
use dex_format::DexFile;

fn inputs() -> Vec<DexFile> {
    vec![
        DexFile::from_bytes(include_bytes!(
            "../../../tests/corpus/dex/clean/classes_named.dex"
        ))
        .unwrap(),
    ]
}

#[test]
fn warm_replay_preserves_all_sources_reports_order_and_clear_lifetime() {
    let files = inputs();
    let options = JavaOptions::default();
    let mut direct = Vec::new();
    let direct_report = emit_java(&files, &options, &mut |s: &GeneratedSource| {
        direct.push(s.clone());
        Ok(())
    })
    .unwrap();
    assert!(!direct.is_empty());
    let mut session = JavaSession::new(files, options).unwrap();
    assert!(!session.stats().ready);
    for warm in [false, true] {
        let mut sources = Vec::new();
        let report = session
            .emit(&mut |s: &GeneratedSource| {
                sources.push(s.clone());
                Ok(())
            })
            .unwrap();
        assert_eq!(report, direct_report);
        assert_eq!(sources, direct);
        assert_eq!(session.sources(), direct);
        assert_eq!(session.stats().renders, 1);
        assert_eq!(session.stats().replays, usize::from(warm));
        assert_eq!(
            session.stats().source_text_bytes,
            direct.iter().map(|s| s.text.len()).sum()
        );
    }
    session.clear_cache();
    assert!(!session.stats().ready);
    assert_eq!(session.stats().sources, 0);
    assert_eq!(session.prepare().unwrap(), &direct_report);
    assert_eq!(session.stats().renders, 2);
}

#[test]
fn failed_sink_retains_snapshot_and_retry_starts_at_first_original_source() {
    let mut session = JavaSession::new(inputs(), JavaOptions::default()).unwrap();
    let failed = session.emit(&mut |_: &GeneratedSource| {
        Err(OutputError::Io(std::io::Error::other("sink refused")))
    });
    assert!(matches!(failed, Err(OutputError::Io(_))));
    assert!(session.stats().ready);
    assert_eq!(session.stats().renders, 1);
    assert_eq!(session.stats().replays, 0);
    let expected = session.sources().to_vec();
    let mut retry = Vec::new();
    session
        .emit(&mut |s: &GeneratedSource| {
            retry.push(s.clone());
            Ok(())
        })
        .unwrap();
    assert_eq!(retry, expected);
    assert_eq!(session.stats().renders, 1);
    assert_eq!(session.stats().replays, 1);
}

#[test]
fn empty_selection_is_a_valid_cached_result_and_other_sessions_are_isolated() {
    let mut empty = JavaSession::new(
        inputs(),
        JavaOptions {
            class_filter: Some("no-such-original-class".into()),
            ..JavaOptions::default()
        },
    )
    .unwrap();
    let first = empty.prepare().unwrap().clone();
    assert_eq!(empty.prepare().unwrap(), &first);
    assert_eq!(empty.stats().renders, 1);
    assert!(empty.stats().ready);
    assert!(empty.sources().is_empty());
    let mut other = JavaSession::new(inputs(), JavaOptions::default()).unwrap();
    other.prepare().unwrap();
    assert!(!other.sources().is_empty());
    assert!(empty.sources().is_empty());
}

#[test]
fn failed_render_never_installs_a_partial_snapshot() {
    let mut files = inputs();
    files[0].classes[0].class_idx = u32::MAX;
    let mut session = JavaSession::new(files, JavaOptions::default()).unwrap();
    for _ in 0..2 {
        assert!(session.prepare().is_err());
        assert!(!session.stats().ready);
        assert_eq!(session.stats().renders, 0);
        assert!(session.sources().is_empty());
    }
}

#[test]
fn classpath_is_snapshotted_before_its_file_is_removed() {
    // Genuine classfile generated once by the installed JDK, never a fake API.
    let root = std::env::temp_dir().join(format!("java-session-cp-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("External.java"),
        "public class External { public static void touch() {} }",
    )
    .unwrap();
    let javac = std::env::var_os("JAVA_HOME")
        .map(|home| std::path::PathBuf::from(home).join("bin/javac"))
        .unwrap_or_else(|| "javac".into());
    assert!(
        std::process::Command::new(javac)
            .arg(root.join("External.java"))
            .status()
            .unwrap()
            .success()
    );
    let path = root.join("External.class");
    let options = JavaOptions {
        classpath: vec![path.clone()],
        ..JavaOptions::default()
    };
    // A fixture body calls a genuine classpath method; cached metadata must
    // continue resolving that exact declaration after its file disappears.
    let mut files = inputs();
    let dex = &mut files[0];
    let span = dex_format::RawSpan { offset: 0, size: 0 };
    let descriptor = dex.strings.len() as u32;
    for text in ["LExternal;", "touch"] {
        dex.strings.push(dex_format::strings::StringData {
            value: text.into(),
            utf16_size: text.len() as u32,
            span,
        });
    }
    let class_idx = dex.type_ids.len() as u16;
    dex.type_ids.push(dex_format::ids::TypeId {
        descriptor_idx: descriptor,
        span,
    });
    let proto_idx = dex
        .method_ids
        .iter()
        .find(|m| dex.string(m.name_idx).unwrap() == "<init>")
        .unwrap()
        .proto_idx;
    let target = dex.method_ids.len() as u16;
    dex.method_ids.push(dex_format::ids::MethodId {
        class_idx,
        proto_idx,
        name_idx: descriptor + 1,
        span,
    });
    let caller = dex.classes[0]
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|m| {
            dex.string(dex.method_id(m.method_idx).unwrap().name_idx)
                .unwrap()
                == "accumulate"
        })
        .unwrap()
        .method_idx;
    let code = dex.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .iter_mut()
        .find(|m| m.method_idx == caller)
        .unwrap()
        .code
        .as_mut()
        .unwrap();
    let mut body: Vec<u8> = [0x0071_u16, target, 0]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    body.extend_from_slice(&code.insns);
    code.insns = body;
    code.insns_size += 3;
    let mut session = JavaSession::new(files, options.clone()).unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(JavaSession::new(inputs(), options).is_err());
    let report = session.prepare().unwrap();
    assert!(report.complete_methods > 0, "{report:?}");
    assert!(
        session
            .sources()
            .iter()
            .any(|s| s.text.contains("External.touch()"))
    );
    session.clear_cache();
    assert!(session.prepare().is_ok());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires retained Samsung Pass APK and explicit JDK17/Android35 metadata"]
fn samsung_fragment_snapshot_replays_attested_static_prefix() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let files = dex_format::apk::parse_apk_path(
        root.join("tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"),
    )
    .unwrap();
    let options = JavaOptions {
        class_filter: Some("Landroidx/fragment/app/Fragment;".into()),
        workers: 4,
        classpath: vec![
            "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod".into(),
            "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar".into(),
        ],
        ..JavaOptions::default()
    };
    let mut session = JavaSession::new(files, options).unwrap();
    let report = session.prepare().unwrap().clone();
    let source = session
        .sources()
        .iter()
        .find(|s| s.provenance.descriptor == "Landroidx/fragment/app/Fragment;")
        .unwrap();
    assert!(!source.text.contains("USE_DEFAULT_TRANSITION = null;"));
    assert_eq!(source.text.matches("USE_DEFAULT_TRANSITION =").count(), 1);
    let exact = session.sources().to_vec();
    let mut replay = Vec::new();
    assert_eq!(
        session
            .emit(&mut |s: &GeneratedSource| {
                replay.push(s.clone());
                Ok(())
            })
            .unwrap(),
        report
    );
    assert_eq!(replay, exact);
    assert_eq!(session.stats().renders, 1);
}
