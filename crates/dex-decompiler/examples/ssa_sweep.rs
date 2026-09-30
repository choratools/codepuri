//! Streams every corpus APK through decode -> lift -> merge -> SSA ->
//! verify -> type inference and reports stage 2.4 coverage.
//!
//! Run: `cargo run --release -p dex-decompiler --example ssa_sweep`
//! Optional directory arguments narrow the corpus (defaults: both corpus
//! APK directories).
//! Use `--workers N` for bounded parallel method processing (default: 1).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use dex_decompiler::{DexPasses, DexTypes, StageError};
use dex_format::apk::parse_apk_path;

#[derive(Default)]
struct Metrics {
    methods: u64,
    ssa_ok: u64,
    type_conflicts: u64,
    type_warnings: u64,
    lifted_blocks: u64,
    merged_blocks: u64,
    ssa_blocks: u64,
    phis: u64,
    dropped_gotos: u64,
    dead_blocks: u64,
    dead_insns: u64,
    hoisted: u64,
    entry_initials: u64,
    errors: BTreeMap<&'static str, u64>,
}

impl Metrics {
    fn merge(&mut self, other: &Self) {
        self.methods += other.methods;
        self.ssa_ok += other.ssa_ok;
        self.type_conflicts += other.type_conflicts;
        self.type_warnings += other.type_warnings;
        self.lifted_blocks += other.lifted_blocks;
        self.merged_blocks += other.merged_blocks;
        self.ssa_blocks += other.ssa_blocks;
        self.phis += other.phis;
        self.dropped_gotos += other.dropped_gotos;
        self.dead_blocks += other.dead_blocks;
        self.dead_insns += other.dead_insns;
        self.hoisted += other.hoisted;
        self.entry_initials += other.entry_initials;
        for (reason, count) in &other.errors {
            *self.errors.entry(reason).or_default() += count;
        }
    }
}

struct AppResult {
    name: String,
    metrics: Metrics,
    elapsed: Duration,
    parse_error: Option<String>,
    workers: usize,
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

fn classify(error: &StageError) -> &'static str {
    match error {
        StageError::Lift(_) => "lift",
        StageError::Merge(_) => "merge",
        StageError::Ssa(dex_decompiler::SsaError::Verification(_)) => "ssa-invalid",
        StageError::Ssa(dex_decompiler::SsaError::Structure(_)) => "ssa-structure",
        StageError::Ssa(dex_decompiler::SsaError::Cfg(_)) => "ssa-cfg",
        StageError::Ssa(dex_decompiler::SsaError::Merge(_)) => "merge",
    }
}

// Each relaxed fetch claims a unique descriptor; scoped thread creation publishes
// the immutable queue. Claim one method at a time so expensive methods cannot be
// stranded in a worker's private chunk. SSA work dwarfs this atomic operation.
fn claim_method(next: &AtomicUsize, len: usize) -> Option<usize> {
    let index = next.fetch_add(1, Ordering::Relaxed);
    (index < len).then_some(index)
}

fn largest_first<T>(methods: &mut [T], estimate: impl Fn(&T) -> usize) {
    // Stable ties preserve DEX/class/method traversal order. Bytecode length is
    // a cheap estimate: no decoding or method IR is needed to build the queue.
    methods.sort_by_key(|method| std::cmp::Reverse(estimate(method)));
}

fn sweep_app(path: &Path, workers: usize) -> AppResult {
    let start = Instant::now();
    let name = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    match parse_apk_path(path) {
        Ok(dexes) => {
            // Retain only borrowed descriptors, never whole-app method IR.
            // Parsing retains one APK; each worker retains its current method IR.
            let mut methods: Vec<_> = dexes
                .iter()
                .flat_map(|dex| {
                    dex.classes
                        .iter()
                        .filter_map(|class| class.class_data.as_ref())
                        .flat_map(move |data| {
                            data.direct_methods
                                .iter()
                                .chain(&data.virtual_methods)
                                .filter_map(move |method| {
                                    method
                                        .code
                                        .as_ref()
                                        .map(|code| (dex, method.method_idx, code))
                                })
                        })
                })
                .collect();
            largest_first(&mut methods, |(_, _, code)| code.insns.len());
            let active_workers = workers.min(methods.len());
            let next = AtomicUsize::new(0);
            let metrics = thread::scope(|scope| {
                let handles: Vec<_> = (0..active_workers)
                    .map(|_| {
                        let methods = &methods;
                        let next = &next;
                        scope.spawn(move || {
                            let mut metrics = Metrics::default();
                            while let Some(index) = claim_method(next, methods.len()) {
                                let (dex, method_idx, code) = methods[index];
                                metrics.methods += 1;
                                let context = DexTypes::new(dex, Some(method_idx));
                                match DexPasses::run_ssa_with(code, &context) {
                                    Ok(ssa) => {
                                        metrics.ssa_ok += 1;
                                        let stats = ssa.stats();
                                        metrics.lifted_blocks += stats.lifted_blocks as u64;
                                        metrics.merged_blocks += stats.merged_blocks as u64;
                                        metrics.ssa_blocks += stats.ssa_blocks as u64;
                                        metrics.phis +=
                                            (stats.phis_placed - stats.phis_eliminated) as u64;
                                        metrics.dropped_gotos += stats.dropped_gotos as u64;
                                        metrics.dead_blocks += stats.dead_blocks as u64;
                                        metrics.dead_insns += stats.dead_insns as u64;
                                        metrics.hoisted += stats.hoisted_defs as u64;
                                        metrics.entry_initials += stats.entry_initials as u64;
                                        for diagnostic in ssa.diagnostics() {
                                            if diagnostic.code() == "DEX_TYPE_CONFLICT" {
                                                metrics.type_conflicts += 1;
                                            } else {
                                                metrics.type_warnings += 1;
                                            }
                                        }
                                    }
                                    Err(error) => {
                                        *metrics.errors.entry(classify(&error)).or_default() += 1;
                                    }
                                }
                            }
                            metrics
                        })
                    })
                    .collect();
                let mut metrics = Metrics::default();
                // Join and reduce in spawn order; integer counters and sorted errors
                // are independent of scheduling.
                for handle in handles {
                    let worker_metrics = handle
                        .join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
                    metrics.merge(&worker_metrics);
                }
                metrics
            });
            AppResult {
                name,
                metrics,
                elapsed: start.elapsed(),
                parse_error: None,
                workers: active_workers,
            }
        }
        Err(error) => AppResult {
            name,
            metrics: Metrics::default(),
            elapsed: start.elapsed(),
            parse_error: Some(error.to_string()),
            workers: 0,
        },
    }
}

fn compact_map(values: &BTreeMap<&'static str, u64>) -> String {
    values
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
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

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<(usize, Vec<String>), String> {
    let mut workers = 1;
    let mut dirs = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            dirs.extend(args);
            break;
        } else if arg == "--workers" {
            workers = args
                .next()
                .ok_or("--workers requires a positive integer")?
                .parse::<usize>()
                .map_err(|_| "--workers requires a positive integer")?;
            if workers == 0 {
                return Err("--workers requires a positive integer".to_owned());
            }
        } else if arg.starts_with("--") {
            return Err(format!("unknown option: {arg}"));
        } else {
            dirs.push(arg);
        }
    }
    if dirs.is_empty() {
        dirs = vec![
            "tests/corpus/apks_device".to_owned(),
            "tests/corpus/apks_bulk".to_owned(),
        ];
    }
    Ok((workers, dirs))
}

fn main() {
    let (workers, dirs) = parse_args(std::env::args().skip(1)).unwrap_or_else(|error| {
        eprintln!("{error}\nUsage: ssa_sweep [--workers N] [directory ...]");
        std::process::exit(2);
    });
    println!(
        "{:<40} {:>9} {:>9} {:>10} {:>9} {:>8} {:>8} {:>7} {:>8} {:>7}  errors",
        "app",
        "methods",
        "ssa-ok",
        "merged-blk",
        "ssa-blk",
        "phis",
        "conflicts",
        "warn",
        "ms",
        "workers"
    );
    let mut total = Metrics::default();
    let mut total_elapsed = Duration::ZERO;
    let mut apps = 0_usize;
    let mut parse_failures = 0_usize;
    for dir in &dirs {
        for path in collect_apks(Path::new(dir)) {
            let result = sweep_app(&path, workers);
            apps += 1;
            total_elapsed += result.elapsed;
            if let Some(error) = &result.parse_error {
                parse_failures += 1;
                println!(
                    "{:<40} FAIL {error} ({} ms, {} workers)",
                    truncate(&result.name, 40),
                    result.elapsed.as_millis(),
                    result.workers
                );
                continue;
            }
            let m = &result.metrics;
            println!(
                "{:<40} {:>9} {:>9} {:>10} {:>9} {:>8} {:>8} {:>7} {:>8} {:>7}  {}",
                truncate(&result.name, 40),
                m.methods,
                m.ssa_ok,
                m.merged_blocks,
                m.ssa_blocks,
                m.phis,
                m.type_conflicts,
                m.type_warnings,
                result.elapsed.as_millis(),
                result.workers,
                compact_map(&m.errors),
            );
            total.merge(m);
        }
    }
    let ratio = if total.lifted_blocks > 0 {
        total.lifted_blocks as f64 / total.merged_blocks.max(1) as f64
    } else {
        0.0
    };
    println!(
        "\nTOTAL: {apps} apks, {} methods, {} ssa-ok ({} errors, {parse_failures} parse failures)",
        total.methods,
        total.ssa_ok,
        total.errors.values().sum::<u64>(),
    );
    println!(
        "BLOCKS: {} lifted -> {} merged ({ratio:.2}x) -> {} published ssa blocks",
        total.lifted_blocks, total.merged_blocks, total.ssa_blocks,
    );
    println!(
        "SSA: {} live phis, {} dropped gotos, {} dead chains ({} insns), {} hoisted defs, {} entry initials",
        total.phis,
        total.dropped_gotos,
        total.dead_blocks,
        total.dead_insns,
        total.hoisted,
        total.entry_initials,
    );
    println!(
        "TYPES: {} conflicts, {} warnings; {} ms",
        total.type_conflicts,
        total.type_warnings,
        total_elapsed.as_millis()
    );
    println!("ERRORS: {}", compact_map(&total.errors));
}

#[cfg(test)]
mod tests {
    use super::{claim_method, largest_first, parse_args};
    use std::sync::atomic::AtomicUsize;
    use std::thread;

    #[test]
    fn estimates_descend_with_stable_ties() {
        let mut methods = [(0, 3), (1, 9), (2, 3), (3, 1), (4, 9)];
        largest_first(&mut methods, |method| method.1);
        assert_eq!(methods, [(1, 9), (4, 9), (0, 3), (2, 3), (3, 1)]);
    }

    #[test]
    fn concurrent_claims_cover_each_method_once() {
        for len in [0, 1, 7, 8, 9, 10_003] {
            for workers in [1, 8] {
                let next = AtomicUsize::new(0);
                let mut claimed = thread::scope(|scope| {
                    let handles: Vec<_> = (0..workers)
                        .map(|_| {
                            let next = &next;
                            scope.spawn(move || {
                                let mut claimed = Vec::new();
                                while let Some(index) = claim_method(next, len) {
                                    claimed.push(index);
                                }
                                claimed
                            })
                        })
                        .collect();
                    handles
                        .into_iter()
                        .flat_map(|handle| {
                            handle
                                .join()
                                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                        })
                        .collect::<Vec<_>>()
                });
                claimed.sort_unstable();
                assert_eq!(claimed, (0..len).collect::<Vec<_>>());
                assert_eq!(claim_method(&next, len), None);
            }
        }
    }

    #[test]
    fn workers_and_directories_are_independent() {
        let parsed = parse_args(["first", "--workers", "2", "second"].map(str::to_owned));
        assert_eq!(
            parsed,
            Ok((2, vec!["first".to_owned(), "second".to_owned()]))
        );
        assert_eq!(
            parse_args(["first".to_owned()]),
            Ok((1, vec!["first".to_owned()]))
        );
        let defaults = parse_args(["--workers", "2"].map(str::to_owned));
        assert_eq!(
            defaults
                .as_ref()
                .map(|(workers, dirs)| (*workers, dirs.len())),
            Ok((2, 2))
        );
    }

    #[test]
    fn invalid_worker_counts_are_rejected() {
        for args in [
            vec!["--workers"],
            vec!["--workers", "0"],
            vec!["--workers", "-1"],
            vec!["--workers", "no"],
        ] {
            assert!(parse_args(args.into_iter().map(str::to_owned)).is_err());
        }
    }
}
