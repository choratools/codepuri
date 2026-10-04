//! Full local APK corpus export using the conservative Java facade.
use decomp::dex_java::{JavaOptions, emit_java};
use decomp::output::{DirectorySink, GeneratedSource, SourceSink};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Default)]
struct ResultRow {
    semantic_bodies_outside_strict_dependency_closure: usize,
    semantic_bodies_with_unresolved_dependencies: usize,
    unavailable_initialization_units: usize,
    category_reasons: BTreeMap<String, usize>,
    name: String,
    counts: [usize; 7],
    extra: [usize; 4],
    reasons: BTreeMap<String, usize>,
    errors: BTreeMap<String, usize>,
    seconds: f64,
    report_available: bool,
}

fn app(path: &Path, root: &Path, options: &JavaOptions) -> ResultRow {
    let start = Instant::now();
    let name = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut row = ResultRow {
        name: name.clone(),
        ..ResultRow::default()
    };
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let dir = root.join(&name);
        fs::create_dir(&dir)?;
        let files = dex_format::apk::parse_apk_path(path)?;
        let mut sink = DirectorySink::new(dir.join("sources"))?;
        let mut output_errors = BTreeMap::new();
        let mut callback = |source: &GeneratedSource| {
            if let Err(e) = sink.emit(source) {
                *output_errors.entry(format!("output: {e}")).or_default() += 1;
            }
            // Continue traversal to retain exact method rejection counts. The
            // separate written count below is authoritative for accepted output.
            Ok(())
        };
        let report = emit_java(&files, options, &mut callback);
        row.errors.extend(output_errors);
        row.counts[6] = sink.counts().total();
        let report = report?;
        row.counts[..6].copy_from_slice(&[
            report.classes_seen,
            report.classes_emitted,
            report.complete_methods,
            report.unsupported_methods,
            report.unavailable_constructors,
            report.skipped_classes,
        ]);
        row.reasons = report.reasons;
        row.unavailable_initialization_units = report.unavailable_initialization_units;
        row.semantic_bodies_with_unresolved_dependencies =
            report.semantic_bodies_with_unresolved_dependencies;
        row.semantic_bodies_outside_strict_dependency_closure =
            report.semantic_bodies_outside_strict_dependency_closure;
        row.category_reasons = report.category_reasons;
        row.extra = [
            report.complete_constructors,
            report.complete_static_initializers,
            report.declaration_only_methods,
            report.fields_emitted,
        ];
        row.report_available = true;
        Ok(())
    })();
    if let Err(e) = result {
        *row.errors.entry(format!("app: {e}")).or_default() += 1;
    }
    row.seconds = start.elapsed().as_secs_f64();
    let detail = format_row(&row);
    if let Err(e) = fs::write(root.join(&name).join("report.txt"), detail) {
        *row.errors.entry(format!("report-write: {e}")).or_default() += 1;
    }
    row
}

fn format_row(r: &ResultRow) -> String {
    let mut text = format!(
        "app={} seconds={:.3} report_available={} classes_seen={} classes_generated={} complete_method_bodies={} unsupported_methods={} unavailable_constructors={} skipped_classes={} classes_written={} complete_constructors={} complete_static_initializers={} declaration_only_methods={} fields_emitted={}\n",
        r.name,
        r.seconds,
        r.report_available,
        r.counts[0],
        r.counts[1],
        r.counts[2],
        r.counts[3],
        r.counts[4],
        r.counts[5],
        r.counts[6],
        r.extra[0],
        r.extra[1],
        r.extra[2],
        r.extra[3]
    );
    for (reason, count) in &r.reasons {
        text.push_str(&format!("reason\t{count}\t{reason}\n"));
    }
    text.push_str(&format!(
        "unavailable_initialization_units={}\n",
        r.unavailable_initialization_units
    ));
    text.push_str(&format!(
        "semantic_bodies_with_unresolved_dependencies={}\n",
        r.semantic_bodies_with_unresolved_dependencies
    ));
    for (reason, count) in &r.category_reasons {
        // Keep legacy closure/rejection counters unchanged.
        text.push_str(&format!("category_reason\t{count}\t{reason}\n"));
    }
    text.push_str(&format!(
        "semantic_bodies_outside_strict_dependency_closure={}\n",
        r.semantic_bodies_outside_strict_dependency_closure
    ));
    for (error, count) in &r.errors {
        text.push_str(&format!("error\t{count}\t{error}\n"));
    }
    text
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut workers = 1usize;
    let mut app_workers = 1usize;
    let mut dirs = Vec::new();
    let mut paths = Vec::new();
    let mut options = JavaOptions::default();
    while let Some(arg) = args.next() {
        if arg == "--workers" {
            workers = args.next().ok_or("missing workers")?.parse()?;
        } else if arg == "--source-release" {
            options.source_release = args.next().ok_or("missing source release")?.parse()?;
        } else if arg == "--app-workers" {
            app_workers = args.next().ok_or("missing app workers")?.parse()?;
        } else if arg == "--app" {
            paths.push(PathBuf::from(args.next().ok_or("missing app path")?));
        } else if arg == "--class" {
            options.class_filter = Some(args.next().ok_or("missing class filter")?);
        } else if arg == "--classpath" {
            options
                .classpath
                .push(args.next().ok_or("missing classpath")?.into());
        } else if arg == "--limit" {
            options.limit = Some(args.next().ok_or("missing limit")?.parse()?);
        } else if arg.starts_with('-') {
            return Err(format!("unknown option {arg}").into());
        } else {
            dirs.push(PathBuf::from(arg));
        }
    }
    if workers == 0 || app_workers == 0 || app_workers > workers {
        return Err("workers must be positive".into());
    }
    if dirs.is_empty() && paths.is_empty() {
        dirs = vec![
            "tests/corpus/apks_device".into(),
            "tests/corpus/apks_bulk".into(),
        ];
    }
    for dir in dirs {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "apk") {
                paths.push(path);
            }
        }
    }
    paths.sort_by_key(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX));
    app_workers = app_workers.min(paths.len().max(1));
    options.workers = workers / app_workers;
    let mut names = std::collections::BTreeSet::new();
    for path in &paths {
        if !names.insert(path.file_stem().ok_or("missing APK stem")?.to_owned()) {
            return Err("duplicate APK stem would alias output directory".into());
        }
    }
    let parent = Path::new("/tmp/opencode/decomphify-java-corpus");
    if !parent.parent().is_some_and(Path::is_dir) {
        return Err("output parent missing".into());
    }
    fs::create_dir_all(parent)?;
    let root = parent.join(format!(
        "run-{}-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
        std::process::id()
    ));
    fs::create_dir(&root)?;
    println!(
        "START apps={} workers={workers} artifacts={}",
        paths.len(),
        root.display()
    );
    println!(
        "PARALLELISM app_workers={app_workers} class_workers={}",
        options.workers
    );
    let start = Instant::now();
    let next = AtomicUsize::new(0);
    let rows = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..app_workers.min(paths.len()) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = paths.get(i) else {
                        break;
                    };
                    let row = std::panic::catch_unwind(|| app(path, &root, &options))
                        .unwrap_or_else(|_| {
                            let mut r = ResultRow {
                                name: path
                                    .file_stem()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned(),
                                ..ResultRow::default()
                            };
                            r.errors.insert("panic: report unavailable".into(), 1);
                            let _ =
                                fs::write(root.join(&r.name).join("report.txt"), format_row(&r));
                            r
                        });
                    println!(
                        "DONE {} seconds={:.3} report={} errors={}",
                        row.name,
                        row.seconds,
                        row.report_available,
                        row.errors.values().sum::<usize>()
                    );
                    rows.lock().unwrap_or_else(|e| e.into_inner()).push(row);
                }
            });
        }
    });
    let mut rows = rows.into_inner().unwrap_or_else(|e| e.into_inner());
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    let mut total = ResultRow {
        name: "TOTAL".into(),
        report_available: rows.iter().all(|r| r.report_available),
        seconds: start.elapsed().as_secs_f64(),
        ..ResultRow::default()
    };
    for r in &rows {
        for (a, b) in total.counts.iter_mut().zip(r.counts) {
            *a += b;
        }
        for (a, b) in total.extra.iter_mut().zip(r.extra) {
            *a += b;
        }
        for (k, v) in &r.reasons {
            *total.reasons.entry(k.clone()).or_default() += v;
        }
        for (k, v) in &r.errors {
            *total.errors.entry(k.clone()).or_default() += v;
        }
    }
    let mut text = format!(
        "Java integrated-subset corpus export\nScope: partial Java subset only. No corpus javac compilation verification.\nworkers={workers} selected={} completed={} reports_available={} artifacts={}\nCounts exclude unavailable reports; complete bodies do not imply complete classes/apps.\n",
        paths.len(),
        rows.len(),
        rows.iter().filter(|r| r.report_available).count(),
        root.display()
    );
    text.push_str(&format_row(&total));
    for row in rows {
        text.push_str(&format_row(&row));
    }
    let mut report = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("summary.txt"))?;
    report.write_all(text.as_bytes())?;
    println!("{}", format_row(&total).lines().next().unwrap_or_default());
    println!("COMPLETE artifacts={}", root.display());
    if !total.report_available || !total.errors.is_empty() {
        return Err(format!(
            "corpus export incomplete: report_available={}, errors={}, artifacts={}",
            total.report_available,
            total.errors.values().sum::<usize>(),
            root.display()
        )
        .into());
    }
    Ok(())
}
