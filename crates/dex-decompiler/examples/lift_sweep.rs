//! Streams every corpus APK through decode -> lift -> CFG and reports
//! stage 2.2/2.3 coverage.
//!
//! Run: `cargo run --release -p dex-decompiler --example lift_sweep`

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use decomp_core::cfg::EdgeKind;
use dex_decompiler::{LiftError, decode_method_recovering, lift_recovered};
use dex_format::apk::parse_apk_path;
use managed_ir::insn::ManagedInsn;

#[derive(Default)]
struct Metrics {
    methods: u64,
    lifted_ok: u64,
    lift_errors: BTreeMap<&'static str, u64>,
    cfg_invalid: u64,
    blocks: u64,
    managed_insns: u64,
    opaque_insns: u64,
    edges: BTreeMap<&'static str, u64>,
}

impl Metrics {
    fn merge(&mut self, other: &Self) {
        self.methods += other.methods;
        self.lifted_ok += other.lifted_ok;
        self.cfg_invalid += other.cfg_invalid;
        self.blocks += other.blocks;
        self.managed_insns += other.managed_insns;
        self.opaque_insns += other.opaque_insns;
        for (reason, count) in &other.lift_errors {
            *self.lift_errors.entry(reason).or_default() += count;
        }
        for (kind, count) in &other.edges {
            *self.edges.entry(kind).or_default() += count;
        }
    }
}

struct AppResult {
    name: String,
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

fn lift_app(path: &Path) -> AppResult {
    let start = Instant::now();
    let name = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    match parse_apk_path(path) {
        Ok(dexes) => {
            let mut metrics = Metrics::default();
            for dex in &dexes {
                for class in &dex.classes {
                    let Some(class_data) = &class.class_data else {
                        continue;
                    };
                    for method in class_data
                        .direct_methods
                        .iter()
                        .chain(class_data.virtual_methods.iter())
                    {
                        let Some(code) = &method.code else {
                            continue;
                        };
                        metrics.methods += 1;
                        let recovered = decode_method_recovering(code, 2);
                        match lift_recovered(&recovered, code) {
                            Ok(lifted) => {
                                metrics.lifted_ok += 1;
                                let cfg = lifted.ir().cfg();
                                if cfg.validate().is_err() {
                                    metrics.cfg_invalid += 1;
                                }
                                metrics.blocks +=
                                    u64::try_from(cfg.block_count()).unwrap_or(u64::MAX);
                                for edge_id in cfg.edge_ids() {
                                    let Ok(edge) = cfg.edge(edge_id) else {
                                        continue;
                                    };
                                    let kind = match edge.kind() {
                                        EdgeKind::Unconditional => "uncond",
                                        EdgeKind::BranchTrue => "br-true",
                                        EdgeKind::BranchFalse => "br-false",
                                        EdgeKind::Switch(_) => "switch",
                                        EdgeKind::Exception(_) => "exception",
                                    };
                                    *metrics.edges.entry(kind).or_default() += 1;
                                }
                                let instructions = lifted.ir().instructions();
                                metrics.managed_insns +=
                                    u64::try_from(instructions.len()).unwrap_or(u64::MAX);
                                metrics.opaque_insns += instructions
                                    .values()
                                    .filter(|insn| matches!(insn, ManagedInsn::Opaque { .. }))
                                    .count()
                                    as u64;
                            }
                            Err(LiftError::DecodeAborted { .. }) => {
                                *metrics.lift_errors.entry("decode-aborted").or_default() += 1;
                            }
                            Err(LiftError::Cfg(_)) => {
                                *metrics.lift_errors.entry("cfg").or_default() += 1;
                            }
                            Err(LiftError::InvalidFilledArray { .. }) => {
                                *metrics
                                    .lift_errors
                                    .entry("invalid-filled-array")
                                    .or_default() += 1;
                            }
                        }
                    }
                }
            }
            AppResult {
                name,
                metrics,
                elapsed: start.elapsed(),
                parse_error: None,
            }
        }
        Err(error) => AppResult {
            name,
            metrics: Metrics::default(),
            elapsed: start.elapsed(),
            parse_error: Some(error.to_string()),
        },
    }
}

fn compact_map<K: std::fmt::Debug + Ord>(values: &BTreeMap<K, u64>) -> String {
    values
        .iter()
        .map(|(key, value)| format!("{key:?}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn truncate(name: &str, max: usize) -> String {
    if name.len() <= max {
        name.to_owned()
    } else {
        format!("{}...", &name[..max.saturating_sub(3)])
    }
}

fn main() {
    println!(
        "{:<42} {:>9} {:>9} {:>9} {:>9} {:>10} {:>9} {:>9}  lift-errors",
        "app", "methods", "lifted", "blocks", "insns", "opaque", "cfg-inv", "ms"
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
    let mut total_elapsed = Duration::ZERO;
    let mut apps = 0_usize;
    let mut parse_failures = 0_usize;
    for dir in dirs {
        for path in collect_apks(Path::new(dir)) {
            let result = lift_app(&path);
            apps += 1;
            total_elapsed += result.elapsed;
            if let Some(error) = &result.parse_error {
                parse_failures += 1;
                println!("{:<42} FAIL {error}", truncate(&result.name, 42));
                continue;
            }
            println!(
                "{:<42} {:>9} {:>9} {:>9} {:>9} {:>10} {:>9} {:>9}  {}",
                truncate(&result.name, 42),
                result.metrics.methods,
                result.metrics.lifted_ok,
                result.metrics.blocks,
                result.metrics.managed_insns,
                result.metrics.opaque_insns,
                result.metrics.cfg_invalid,
                result.elapsed.as_millis(),
                compact_map(&result.metrics.lift_errors),
            );
            total.merge(&result.metrics);
        }
    }
    println!(
        "\nTOTAL: {apps} apks, {} methods, {} lifted ({} lift errors, {} cfg-invalid), {} blocks, {} managed insns ({} opaque), {} edges, {} ms, {parse_failures} parse failures",
        total.methods,
        total.lifted_ok,
        total.lift_errors.values().sum::<u64>(),
        total.cfg_invalid,
        total.blocks,
        total.managed_insns,
        total.opaque_insns,
        total.edges.values().sum::<u64>(),
        total_elapsed.as_millis()
    );
    println!("LIFT-ERRORS: {}", compact_map(&total.lift_errors));
    println!("EDGES: {}", compact_map(&total.edges));
}
