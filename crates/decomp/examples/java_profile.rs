//! Single-input opt-in profiler; see bench/JAVA_PROFILING.md.
use decomp::dex_java::{JavaOptions, JavaProfile, emit_java, emit_java_profiled};
use decomp::java_external_metadata::{
    Classpath, DexMetadata, MetadataChain, MetadataProvider, emit_with_metadata,
};
use decomp::output::{DirectorySink, GeneratedSource, SourceSink};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

fn seconds(d: Duration) -> f64 {
    d.as_secs_f64()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let apk = PathBuf::from(args.next().ok_or("usage: java_profile APK OUTPUT [--workers N] [--class DESCRIPTOR] [--classpath PATH] [--metadata-dex PATH] [--source-release N] [--no-io] [--unprofiled] [--no-ssa-cache] [--no-resolution-cache] [--no-inventory-cache]")?);
    let root = PathBuf::from(
        args.next()
            .ok_or("missing output directory (must not exist)")?,
    );
    let mut options = JavaOptions::default();
    let mut no_io = false;
    let mut unprofiled = false;
    let mut metadata_dex = Vec::<PathBuf>::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--source-release" => {
                options.source_release = args.next().ok_or("missing source release")?.parse()?;
            }
            "--classpath" => options
                .classpath
                .push(args.next().ok_or("missing classpath")?.into()),
            "--metadata-dex" => {
                metadata_dex.push(
                    args.next()
                        .ok_or("missing external DEX metadata path")?
                        .into(),
                );
            }
            "--workers" => options.workers = args.next().ok_or("missing workers")?.parse()?,
            "--class" => options.class_filter = Some(args.next().ok_or("missing class")?),
            "--no-io" => no_io = true,
            "--unprofiled" => unprofiled = true,
            "--no-ssa-cache" => options.cache_ssa = false,
            "--no-resolution-cache" => options.cache_resolutions = false,
            "--no-inventory-cache" => options.cache_inventory = false,
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    if options.workers == 0 {
        return Err("workers must be positive".into());
    }
    fs::create_dir(&root)?;
    let parse_start = Instant::now();
    let files = dex_format::apk::parse_apk_path(&apk)?;
    let parse = parse_start.elapsed();
    // Explicit runtime declarations are a separate experiment. They do not
    // make a DEX-only archive consumable by javac or merge shadowed SDK members.
    let metadata_start = Instant::now();
    let runtime_metadata = metadata_dex
        .iter()
        .map(DexMetadata::from_path)
        .collect::<Result<Vec<_>, _>>()?;
    let compiler_metadata = if runtime_metadata.is_empty() {
        None
    } else {
        Some(Classpath::from_paths(&options.classpath)?)
    };
    let external = compiler_metadata.as_ref().map(|classpath| {
        let mut providers: Vec<_> = runtime_metadata.iter().map(MetadataProvider::Dex).collect();
        providers.push(MetadataProvider::Jvm(classpath));
        MetadataChain::new(providers)
    });
    let metadata_setup = metadata_start.elapsed();
    let mut directory = if no_io {
        None
    } else {
        Some(DirectorySink::new(root.join("sources"))?)
    };
    let mut hash = 0xcbf29ce484222325u64;
    let mut count = 0usize;
    let mut bytes = 0usize;
    let mut sink = |source: &GeneratedSource| {
        // Diagnostic checksum outside DirectorySink, included in sink_io.
        for b in source
            .relative_path
            .to_string_lossy()
            .bytes()
            .chain([0])
            .chain(source.text.bytes())
            .chain([0])
        {
            hash = (hash ^ u64::from(b)).wrapping_mul(0x100000001b3);
        }
        count += 1;
        bytes += source.text.len();
        if let Some(sink) = &mut directory {
            sink.emit(source)?;
        }
        Ok(())
    };
    let mut profile = JavaProfile::default();
    let start = Instant::now();
    let report = if let Some(metadata) = &external {
        emit_with_metadata(
            &files,
            &options,
            &mut sink,
            (!unprofiled).then_some(&mut profile),
            metadata,
        )
    } else if unprofiled {
        emit_java(&files, &options, &mut sink)
    } else {
        emit_java_profiled(&files, &options, &mut sink, &mut profile)
    }?;
    let export_wall = start.elapsed();
    // Diagnostic receipt only: absent units remain absent. Hex preserves exact
    // UTF-8 identities/reasons without inventing source or JSON escape rules.
    let mut unpublished = String::from("identity_hex\treason_hex\n");
    for (identity, reason) in &report.unpublished_method_reasons {
        let hex = |text: &str| {
            text.as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        unpublished.push_str(&format!("{}\t{}\n", hex(identity), hex(reason)));
    }
    fs::write(root.join("unpublished-methods.tsv"), unpublished)?;

    let mut text = format!(
        "apk={} workers={} source_release={} no_io={} unprofiled={} parse_s={:.6} export_wall_s={:.6} sources={} bytes={} ordered_checksum={hash:016x}\nreport={report:?}\n",
        apk.display(),
        options.workers,
        options.source_release,
        no_io,
        unprofiled,
        seconds(parse),
        seconds(export_wall),
        count,
        bytes
    );
    text.push_str(&format!(
        "runtime_metadata_paths={metadata_dex:?} metadata_setup_s={:.6} javac_availability_attested=false\n",
        seconds(metadata_setup)
    ));
    let mut stages = BTreeMap::<&str, Duration>::new();
    if unprofiled {
        text.push_str(&format!(
            "ssa_cache enabled={} statistics=unprofiled\n",
            options.cache_ssa,
        ));
    } else {
        text.push_str(&format!(
            "ssa_cache enabled={} hits={} misses={} entries={} failures={}\n",
            profile.ssa_cache_enabled,
            profile.ssa_cache_hits,
            profile.ssa_cache_misses,
            profile.ssa_cache_entries,
            profile.ssa_cache_failures,
        ));
    }
    if unprofiled {
        text.push_str(&format!(
            "resolution_cache enabled={} statistics=unprofiled\n",
            options.cache_resolutions
        ));
    } else {
        text.push_str(&format!(
            "resolution_cache enabled={} hits={} misses={} failures={}\n",
            profile.resolution_cache_enabled,
            profile.resolution_cache_hits,
            profile.resolution_cache_misses,
            profile.resolution_cache_failures
        ));
    }
    text.push_str(&format!(
        "inventory_cache enabled={}\n",
        options.cache_inventory
    ));
    if unprofiled {
        text.push_str(&format!(
            "field_use_cache enabled={} statistics=unprofiled\n",
            options.cache_resolutions
        ));
    } else {
        text.push_str(&format!(
            "field_use_cache enabled={} hits={} misses={} entries={} failures={}\n",
            profile.field_use_cache_enabled,
            profile.field_use_cache_hits,
            profile.field_use_cache_misses,
            profile.field_use_cache_entries,
            profile.field_use_cache_failures
        ));
    }
    let mut ssa = BTreeMap::<&str, Duration>::new();
    let task_sum: Duration = profile.classes.iter().map(|c| c.elapsed).sum();
    let scheduler_wall: Duration = profile.schedulers.iter().map(|s| s.wall).sum();
    let worker_time: Duration = profile.schedulers.iter().map(|s| s.worker_time).sum();
    let active: Duration = profile.schedulers.iter().map(|s| s.task_time).sum();
    let queue_wait: Duration = profile.schedulers.iter().map(|s| s.queue_wait).sum();
    let dispatch_idle: Duration = profile.schedulers.iter().map(|s| s.dispatch_idle).sum();
    let capacity_s: f64 = profile
        .schedulers
        .iter()
        .map(|s| seconds(s.wall) * s.capacity as f64)
        .sum();
    let completed: usize = profile.schedulers.iter().map(|s| s.tasks).sum();
    text.push_str(&format!("profile_wall_s={:.6} delivered_class_profiles={} completed_tasks={completed} task_elapsed_sum_s={:.6} scheduler_wall_sum_s={:.6} capacity_slot_s={capacity_s:.6} worker_lifetime_s={:.6} active_task_s={:.6} queue_wait_s={:.6} dispatch_idle_s={:.6} effective_active_workers={:.3} capacity_occupancy_pct={:.3} lifetime_active_pct={:.3}\n", seconds(profile.wall), profile.classes.len(), seconds(task_sum), seconds(scheduler_wall), seconds(worker_time), seconds(active), seconds(queue_wait), seconds(dispatch_idle), seconds(active) / seconds(scheduler_wall).max(f64::MIN_POSITIVE), 100.0 * seconds(active) / capacity_s.max(f64::MIN_POSITIVE), 100.0 * seconds(active) / seconds(worker_time).max(f64::MIN_POSITIVE)));
    for c in &profile.classes {
        for (&k, &v) in &c.stages {
            *stages.entry(k).or_default() += v;
        }
        for (&k, &v) in &c.ssa {
            *ssa.entry(k).or_default() += v;
        }
    }
    for (k, v) in &profile.serial {
        text.push_str(&format!(
            "serial\t{k}\t{:.6}\twall_pct={:.3}\n",
            seconds(*v),
            100.0 * seconds(*v) / seconds(export_wall)
        ));
    }
    for (k, v) in &stages {
        text.push_str(&format!(
            "task_stage\t{k}\t{:.6}\ttask_sum_pct={:.3}\n",
            seconds(*v),
            100.0 * seconds(*v) / seconds(task_sum).max(f64::MIN_POSITIVE)
        ));
    }
    for (k, v) in &ssa {
        text.push_str(&format!(
            "nested_ssa\t{k}\t{:.6}\tfrontend_pct={:.3}\n",
            seconds(*v),
            100.0 * seconds(*v)
                / seconds(*stages.get("frontend").unwrap_or(&Duration::ZERO))
                    .max(f64::MIN_POSITIVE)
        ));
    }
    let measured: Duration = stages.values().copied().sum();
    text.push_str(&format!(
        "task_unattributed_s={:.6}\n",
        seconds(task_sum.saturating_sub(measured))
    ));
    let mut classes: Vec<_> = profile.classes.iter().collect();
    classes.sort_by_key(|c| std::cmp::Reverse(c.elapsed));
    for c in classes.iter().take(20) {
        text.push_str(&format!(
            "top_class\tdex={}\t{}\telapsed_s={:.6}\tmethods={}\tstages={:?}\tssa={:?}\n",
            c.dex,
            c.descriptor,
            seconds(c.elapsed),
            c.methods,
            c.stages,
            c.ssa
        ));
    }
    for s in &profile.schedulers {
        text.push_str(&format!(
            "scheduler\tdex={}\ttasks={}\tcapacity={}\treorder_capacity={}\tmax_pending={}\twall_s={:.6}\tactive_s={:.6}\tqueue_wait_s={:.6}\tdispatch_idle_s={:.6}\tworker_lifetime_s={:.6}\n",
            s.dex,
            s.tasks,
            s.capacity,
            s.reorder_capacity,
            s.max_pending,
            seconds(s.wall),
            seconds(s.task_time),
            seconds(s.queue_wait),
            seconds(s.dispatch_idle),
            seconds(s.worker_time)
        ));
    }
    fs::write(root.join("summary.txt"), &text)?;
    let mut rows =
        String::from("dex\tclass\tdescriptor\tstart_s\telapsed_s\tmethods\tstages\tssa\n");
    for c in &profile.classes {
        rows.push_str(&format!(
            "{}\t{}\t{}\t{:.9}\t{:.9}\t{}\t{:?}\t{:?}\n",
            c.dex,
            c.class,
            c.descriptor,
            seconds(c.start),
            seconds(c.elapsed),
            c.methods,
            c.stages,
            c.ssa
        ));
    }
    fs::write(root.join("classes.tsv"), rows)?;
    let mut rows = String::from(
        "dex\ttasks\tcapacity\treorder_capacity\tmax_pending\twall_s\tactive_s\tqueue_wait_s\tdispatch_idle_s\tworker_lifetime_s\n",
    );
    for s in &profile.schedulers {
        rows.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{:.9}\t{:.9}\t{:.9}\t{:.9}\t{:.9}\n",
            s.dex,
            s.tasks,
            s.capacity,
            s.reorder_capacity,
            s.max_pending,
            seconds(s.wall),
            seconds(s.task_time),
            seconds(s.queue_wait),
            seconds(s.dispatch_idle),
            seconds(s.worker_time)
        ));
    }
    fs::write(root.join("schedulers.tsv"), rows)?;
    println!("{text}artifacts={}", root.display());
    Ok(())
}
