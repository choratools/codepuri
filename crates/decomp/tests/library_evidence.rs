//! Real D8 fixtures exercise threshold controls and mandatory implementation proof.
#![allow(clippy::unwrap_used)]
use decomp::library_evidence::{self, DEFAULT_MIN_SCORE};
use std::io::Write;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};
use zip::{ZipWriter, write::SimpleFileOptions};

const JAVA: &str = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
const D8: &str = "/home/jtjisgod/Android/Sdk/build-tools/35.0.0/d8";
const ANDROID: &str = "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar";
const LIBRARY: &str = "package fixture; public final class Library { public static int transform(int value) { int sum=0; for(int i=0;i<3;i++) sum=(sum*31)^(value+i); return sum; } }";

struct Fixture {
    root: PathBuf,
    artifact: PathBuf,
    reference: PathBuf,
    apk: PathBuf,
    changed: PathBuf,
    ambiguous: PathBuf,
}

fn command(program: &str, args: &[&str], root: &Path) {
    let result = Command::new(program)
        .args(args)
        .env("JAVA_HOME", "/home/jtjisgod/tools/jdk-17.0.13+11")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn jar(path: &Path, classes: &[(&str, &Path)]) {
    let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
    for (name, input) in classes {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(&fs::read(input).unwrap()).unwrap();
    }
    zip.finish().unwrap();
}

fn dex_zip(root: &Path, name: &str, inputs: &[&Path]) -> PathBuf {
    let folder = root.join(name);
    fs::create_dir_all(&folder).unwrap();
    let mut args = vec![
        "--min-api",
        "24",
        "--lib",
        ANDROID,
        "--output",
        folder.to_str().unwrap(),
    ];
    args.extend(inputs.iter().map(|path| path.to_str().unwrap()));
    command(D8, &args, root);
    let output = root.join(format!("{name}.apk"));
    jar(
        &output,
        &[
            ("classes.dex", &folder.join("classes.dex")),
            ("AndroidManifest.xml", &root.join("AndroidManifest.xml")),
        ],
    );
    output
}

fn fixture() -> &'static Fixture {
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("codepuri-library-evidence-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("AndroidManifest.xml"), "<manifest package=\"fixture\"/>").unwrap();
        fs::write(root.join("Library.java"), LIBRARY).unwrap();
        fs::write(root.join("Own.java"), "package fixture; public final class Own { public static int application(int value) { return Library.transform(value)+7; } }").unwrap();
        fs::write(root.join("Weak.java"), "package fixture; public final class Weak { public static boolean flag(){return true;} }").unwrap();
        command(&format!("{JAVA}/javac"), &["--release", "8", "-d", ".", "Library.java", "Own.java", "Weak.java"], &root);
        let artifact = root.join("library.jar");
        jar(&artifact, &[("fixture/Library.class", &root.join("fixture/Library.class")), ("fixture/Weak.class", &root.join("fixture/Weak.class"))]);
        let own = root.join("own.jar");
        jar(&own, &[("fixture/Own.class", &root.join("fixture/Own.class"))]);
        let reference = dex_zip(&root, "reference", &[&artifact]);
        let apk = dex_zip(&root, "input", &[&artifact, &own]);
        fs::write(root.join("Library.java"), LIBRARY.replace("i<3", "i<4")).unwrap();
        command(&format!("{JAVA}/javac"), &["--release", "8", "-d", ".", "Library.java"], &root);
        let modified = root.join("modified.jar");
        jar(&modified, &[("fixture/Library.class", &root.join("fixture/Library.class"))]);
        let changed = dex_zip(&root, "changed", &[&modified, &own]);
        fs::write(root.join("Clone.java"), LIBRARY.replace("class Library", "class Clone")).unwrap();
        command(&format!("{JAVA}/javac"), &["--release", "8", "-d", ".", "Clone.java"], &root);
        let clone = root.join("clone.jar");
        jar(&clone, &[("fixture/Clone.class", &root.join("fixture/Clone.class"))]);
        let ambiguous = dex_zip(&root, "ambiguous", &[&artifact, &clone]);
        Fixture { root, artifact, reference, apk, changed, ambiguous }
    })
}

fn evidence(f: &Fixture, apk: &Path, score: f64, name: &str) -> PathBuf {
    let path = f.root.join(format!("{name}.json"));
    fs::write(&path, serde_json::to_vec(&serde_json::json!({
        "format":"codepuri-library-evidence-v1", "apk_sha256":library_evidence::file_sha256(apk).unwrap(),
        "references":[{"id":"fixture:library:1", "artifact_file":f.artifact, "artifact_sha256":library_evidence::file_sha256(&f.artifact).unwrap(),
            "dex_file":f.reference, "dex_sha256":library_evidence::file_sha256(&f.reference).unwrap(), "score":score,
            "score_basis":"known fixture reference candidate"}]
    })).unwrap()).unwrap();
    path
}

#[test]
fn score_is_configurable_but_weak_changed_and_ambiguous_implementations_stay() {
    let f = fixture();
    let manifest = evidence(f, &f.apk, 0.93, "threshold");
    let files = dex_format::apk::parse_apk_path(&f.apk).unwrap();
    assert!(
        library_evidence::select(&manifest, &f.apk, &files, DEFAULT_MIN_SCORE)
            .unwrap()
            .owners
            .is_empty()
    );
    let relaxed = library_evidence::select(&manifest, &f.apk, &files, 0.90).unwrap();
    assert_eq!(
        relaxed.owners,
        std::collections::BTreeSet::from(["Lfixture/Library;".into()])
    );
    assert!(
        relaxed
            .decisions
            .iter()
            .any(|row| row.owner == "Lfixture/Weak;" && !row.selected && row.unique_anchors == 0)
    );
    let changed = evidence(f, &f.changed, 1.0, "changed-evidence");
    let files = dex_format::apk::parse_apk_path(&f.changed).unwrap();
    assert!(
        library_evidence::select(&changed, &f.changed, &files, 0.0)
            .unwrap()
            .owners
            .is_empty()
    );
    let ambiguous = evidence(f, &f.ambiguous, 1.0, "ambiguous-evidence");
    let files = dex_format::apk::parse_apk_path(&f.ambiguous).unwrap();
    let selection = library_evidence::select(&ambiguous, &f.ambiguous, &files, 0.0).unwrap();
    assert!(selection.owners.is_empty());
    assert!(
        selection
            .decisions
            .iter()
            .any(|row| row.owner == "Lfixture/Library;" && row.reason.contains("distinctive"))
    );
}

#[test]
fn hashes_and_formats_fail_closed_before_selecting_source() {
    let f = fixture();
    let manifest = evidence(f, &f.apk, 1.0, "hash-evidence");
    let files = dex_format::apk::parse_apk_path(&f.apk).unwrap();
    assert!(
        library_evidence::select(&manifest, &f.changed, &files, 0.95)
            .unwrap_err()
            .contains("SHA-256 mismatch")
    );
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["references"][0]["dex_sha256"] = serde_json::json!("0".repeat(64));
    let bad = f.root.join("wrong-reference-hash.json");
    fs::write(&bad, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        library_evidence::select(&bad, &f.apk, &files, 0.95)
            .unwrap_err()
            .contains("SHA-256 mismatch")
    );
    for invalid in [f64::NAN, f64::INFINITY, -0.1, 1.01] {
        assert!(library_evidence::validate_score(invalid).is_err());
    }
}

#[test]
fn cli_accepts_threshold_and_routes_by_original_owner() {
    let f = fixture();
    let manifest = evidence(f, &f.apk, 0.93, "cli-evidence");
    let app = f.root.join("app-source");
    let library = f.root.join("library-source");
    let result = Command::new(env!("CARGO_BIN_EXE_decomp"))
        .arg(&f.apk)
        .arg(&app)
        .args([
            "--library-evidence",
            manifest.to_str().unwrap(),
            "--library-min-score",
            "0.90",
            "--library-out",
            library.to_str().unwrap(),
            "--classpath",
            ANDROID,
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(library.join("fixture/Library.java").is_file());
    assert!(!app.join("fixture/Library.java").exists());
    assert!(app.join("fixture/Own.java").is_file());
    assert!(app.join("fixture/Weak.java").is_file());
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(app.join("library-selection.json")).unwrap()).unwrap();
    assert_eq!(receipt["min_score"], 0.90);
    assert_eq!(receipt["owners"], serde_json::json!(["Lfixture/Library;"]));
    let retained = f.root.join("default-threshold-source");
    let result = Command::new(env!("CARGO_BIN_EXE_decomp"))
        .arg(&f.apk)
        .arg(&retained)
        .args([
            "--library-evidence",
            manifest.to_str().unwrap(),
            "--classpath",
            ANDROID,
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(retained.join("fixture/Library.java").is_file());
    for (relative, separated) in [
        ("fixture/Library.java", &library),
        ("fixture/Own.java", &app),
        ("fixture/Weak.java", &app),
    ] {
        assert_eq!(
            fs::read(retained.join(relative)).unwrap(),
            fs::read(separated.join(relative)).unwrap(),
            "routing changed source {relative}"
        );
    }
    let excluded = f.root.join("exclude-without-library-out");
    let result = Command::new(env!("CARGO_BIN_EXE_decomp"))
        .arg(&f.apk)
        .arg(&excluded)
        .args([
            "--library-evidence",
            manifest.to_str().unwrap(),
            "--library-min-score",
            "0.90",
            "--classpath",
            ANDROID,
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!excluded.join("fixture/Library.java").exists());
    assert!(excluded.join("fixture/Own.java").is_file());
    assert!(excluded.join("fixture/Weak.java").is_file());
    let overlap = f.root.join("overlapping-output");
    let result = Command::new(env!("CARGO_BIN_EXE_decomp"))
        .arg(&f.apk)
        .arg(&overlap)
        .args([
            "--library-evidence",
            manifest.to_str().unwrap(),
            "--library-out",
            overlap.join("libs").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("must not overlap"));
    for invalid in ["NaN", "inf", "-1", "1.1"] {
        let result = Command::new(env!("CARGO_BIN_EXE_decomp"))
            .args(["missing.apk", "unused", "--library-min-score", invalid])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("finite"));
    }
    let result = Command::new(env!("CARGO_BIN_EXE_decomp"))
        .args(["missing.apk", "unused", "--library-min-score", "0.95"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("require --library-evidence"));
}
