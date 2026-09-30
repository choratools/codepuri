//! Corpus sweep: parse every APK under `tests/corpus` and report per-app
//! DEX statistics with wall-clock timings.
//!
//! Run: `cargo run --release -p dex-format --example sweep`

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use dex_format::apk::parse_apk_path;

/// One row of sweep output per APK.
struct AppResult {
    name: String,
    bytes: u64,
    dex_count: usize,
    version: u32,
    classes: usize,
    methods: usize,
    code_items: usize,
    elapsed: std::time::Duration,
    error: Option<String>,
}

fn count_app(dexes: &[dex_format::DexFile]) -> (usize, usize, usize, u32) {
    let mut classes = 0usize;
    let mut methods = 0usize;
    let mut code_items = 0usize;
    let mut version = 0u32;
    for dex in dexes {
        version = version.max(dex.header.version);
        classes += dex.classes.len();
        for class in &dex.classes {
            if let Some(data) = &class.class_data {
                for method in data
                    .direct_methods
                    .iter()
                    .chain(data.virtual_methods.iter())
                {
                    methods += 1;
                    if method.code.is_some() {
                        code_items += 1;
                    }
                }
            }
        }
    }
    (classes, methods, code_items, version)
}

fn collect_apks(dir: &Path) -> Vec<PathBuf> {
    let mut apks = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "apk") {
                apks.push(path);
            }
        }
    }
    apks.sort();
    apks
}

fn sweep(dir: &str) -> Vec<AppResult> {
    let mut results = Vec::new();
    for path in collect_apks(Path::new(dir)) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let start = Instant::now();
        match parse_apk_path(&path) {
            Ok(dexes) => {
                let elapsed = start.elapsed();
                let (classes, methods, code_items, version) = count_app(&dexes);
                results.push(AppResult {
                    name,
                    bytes,
                    dex_count: dexes.len(),
                    version,
                    classes,
                    methods,
                    code_items,
                    elapsed,
                    error: None,
                });
            }
            Err(err) => results.push(AppResult {
                name,
                bytes,
                dex_count: 0,
                version: 0,
                classes: 0,
                methods: 0,
                code_items: 0,
                elapsed: start.elapsed(),
                error: Some(err.to_string()),
            }),
        }
    }
    results
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{:<42} {:>9} {:>4} {:>4} {:>7} {:>8} {:>8} {:>9}",
        "app", "size", "dex", "ver", "classes", "methods", "withCode", "ms"
    );
    let dirs: Vec<String> = std::env::args().skip(1).collect();
    let dirs: Vec<&str> = if dirs.is_empty() {
        vec![
            "tests/corpus/apks",
            "tests/corpus/apks_device",
            "tests/corpus/apks_bulk",
        ]
    } else {
        dirs.iter().map(String::as_str).collect()
    };
    let mut total_ms = 0u128;
    let mut total_dex = 0usize;
    let mut total_classes = 0usize;
    let mut failures = 0usize;
    let mut apps = 0usize;
    for dir in dirs {
        for r in sweep(dir) {
            apps += 1;
            total_ms += r.elapsed.as_millis();
            total_dex += r.dex_count;
            total_classes += r.classes;
            match &r.error {
                Some(err) => {
                    failures += 1;
                    println!("{:<42} {:>9} FAIL {}", truncate(&r.name, 42), r.bytes, err);
                }
                None => {
                    println!(
                        "{:<42} {:>9} {:>4} {:>4} {:>7} {:>8} {:>8} {:>9}",
                        truncate(&r.name, 42),
                        r.bytes,
                        r.dex_count,
                        r.version,
                        r.classes,
                        r.methods,
                        r.code_items,
                        r.elapsed.as_millis()
                    );
                }
            }
        }
    }
    println!(
        "\nTOTAL: {apps} apks, {total_dex} dex, {total_classes} classes, {total_ms} ms, {failures} failures"
    );
    Ok(())
}

fn truncate(name: &str, max: usize) -> String {
    if name.len() <= max {
        name.to_owned()
    } else {
        format!("{}...", &name[..max.saturating_sub(3)])
    }
}
