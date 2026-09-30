//! Streams every corpus APK through the Dalvik decoder and reports coverage.
//!
//! Run: `cargo run --release -p dex-decompiler --example decode_sweep`

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dex_decompiler::{DecodeReason, Opcode, SemanticClass, decode_method_recovering};
use dex_format::apk::parse_apk_path;

#[derive(Default)]
struct Metrics {
    methods: u64,
    clean: u64,
    partial: u64,
    aborted: u64,
    errors: BTreeMap<&'static str, u64>,
    classes: BTreeMap<SemanticClass, u64>,
    unsupported: BTreeSet<u8>,
    insns: u64,
}

impl Metrics {
    fn merge(&mut self, other: &Self) {
        self.methods += other.methods;
        self.clean += other.clean;
        self.partial += other.partial;
        self.aborted += other.aborted;
        self.insns += other.insns;
        for (&reason, &count) in &other.errors {
            *self.errors.entry(reason).or_default() += count;
        }
        for (&class, &count) in &other.classes {
            *self.classes.entry(class).or_default() += count;
        }
        self.unsupported.extend(&other.unsupported);
    }
}

struct AppResult {
    name: String,
    dex_count: usize,
    metrics: Metrics,
    elapsed: Duration,
    parse_error: Option<String>,
}

fn collect_apks(dir: &Path) -> Vec<PathBuf> {
    let mut apks = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "apk") {
                apks.push(path);
            }
        }
    }
    apks.sort();
    apks
}

fn decode_app(path: &Path) -> AppResult {
    let start = Instant::now();
    let name = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    match parse_apk_path(path) {
        Ok(dexes) => {
            let mut metrics = Metrics::default();
            for dex in &dexes {
                for class in &dex.classes {
                    if let Some(data) = &class.class_data {
                        for method in data
                            .direct_methods
                            .iter()
                            .chain(data.virtual_methods.iter())
                        {
                            if let Some(code) = &method.code {
                                let recovered = decode_method_recovering(code, 2);
                                metrics.methods += 1;
                                // Classification is budget-driven: noise
                                // diagnostics (padding-ident lookalikes,
                                // register-range oddities) never demote a
                                // method, matching jadx tolerance.
                                if recovered.aborted {
                                    metrics.aborted += 1;
                                } else if recovered.budget_used == 0 {
                                    metrics.clean += 1;
                                } else {
                                    metrics.partial += 1;
                                }
                                metrics.insns +=
                                    u64::try_from(recovered.insns.len()).unwrap_or(u64::MAX);
                                for insn in &recovered.insns {
                                    let class = match insn.opcode {
                                        Opcode::Supported(info) => info.class,
                                        Opcode::Payload { ident: _ } => SemanticClass::Payload,
                                        Opcode::Unsupported { opcode } => {
                                            metrics.unsupported.insert(opcode);
                                            continue;
                                        }
                                    };
                                    *metrics.classes.entry(class).or_default() += 1;
                                }
                                for error in &recovered.errors {
                                    *metrics
                                        .errors
                                        .entry(reason_name(&error.reason))
                                        .or_default() += 1;
                                    if let DecodeReason::UnsupportedOpcode { opcode } = error.reason
                                    {
                                        metrics.unsupported.insert(opcode);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            AppResult {
                name,
                dex_count: dexes.len(),
                metrics,
                elapsed: start.elapsed(),
                parse_error: None,
            }
        }
        Err(error) => AppResult {
            name,
            dex_count: 0,
            metrics: Metrics::default(),
            elapsed: start.elapsed(),
            parse_error: Some(error.to_string()),
        },
    }
}

fn reason_name(reason: &DecodeReason) -> &'static str {
    match reason {
        DecodeReason::Truncated { .. } => "truncated",
        DecodeReason::UnsupportedOpcode { .. } => "unsupported",
        DecodeReason::RegisterOutOfRange { .. } => "register-oob",
        DecodeReason::BranchTargetOutOfRange { .. } => "branch-oob",
        DecodeReason::BadPayloadIdent { .. } => "bad-payload-ident",
        DecodeReason::MalformedPayload { .. } => "malformed-payload",
        DecodeReason::InvalidArgumentCount { .. } => "bad-arg-count",
    }
}

fn compact_map<K: std::fmt::Debug + Ord>(values: &BTreeMap<K, u64>) -> String {
    values
        .iter()
        .map(|(key, value)| format!("{key:?}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn main() {
    println!(
        "{:<42} {:>4} {:>9} {:>7} {:>8} {:>8} {:>12} {:>9}  errors",
        "app", "dex", "methods", "clean", "partial", "aborted", "insns", "ms"
    );
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dirs: Vec<&str> = if args.is_empty() {
        vec![
            "tests/corpus/apks",
            "tests/corpus/apks_device",
            "tests/corpus/apks_bulk",
        ]
    } else {
        args.iter().map(String::as_str).collect()
    };
    let mut total = Metrics::default();
    let mut total_dex = 0_usize;
    let mut total_elapsed = Duration::ZERO;
    let mut apps = 0_usize;
    let mut parse_failures = 0_usize;
    for dir in dirs {
        for path in collect_apks(Path::new(dir)) {
            let result = decode_app(&path);
            apps += 1;
            total_dex += result.dex_count;
            total_elapsed += result.elapsed;
            if let Some(error) = &result.parse_error {
                parse_failures += 1;
                println!("{:<42} FAIL {error}", truncate(&result.name, 42));
                continue;
            }
            println!(
                "{:<42} {:>4} {:>9} {:>7} {:>8} {:>8} {:>12} {:>9}  {}",
                truncate(&result.name, 42),
                result.dex_count,
                result.metrics.methods,
                result.metrics.clean,
                result.metrics.partial,
                result.metrics.aborted,
                result.metrics.insns,
                result.elapsed.as_millis(),
                compact_map(&result.metrics.errors),
            );
            total.merge(&result.metrics);
        }
    }
    println!(
        "\nTOTAL: {apps} apks, {total_dex} dex, {} methods ({} clean, {} partial, {} aborted), {} diagnostics, {} insns, {} ms, {parse_failures} parse failures",
        total.methods,
        total.clean,
        total.partial,
        total.aborted,
        total.errors.values().sum::<u64>(),
        total.insns,
        total_elapsed.as_millis()
    );
    println!("ERRORS: {}", compact_map(&total.errors));
    println!("SEMANTIC: {}", compact_map(&total.classes));
    println!("UNSUPPORTED: {:?}", total.unsupported);
}

fn truncate(name: &str, max: usize) -> String {
    if name.len() <= max {
        name.to_owned()
    } else {
        format!("{}...", &name[..max.saturating_sub(3)])
    }
}
