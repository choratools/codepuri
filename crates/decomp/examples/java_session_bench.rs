//! Cold-render versus warm-source-replay benchmark; both write real files.
use decomp::{
    dex_java::JavaOptions,
    java_session::JavaSession,
    output::{DirectorySink, GeneratedSource, SourceSink},
};
use std::{fs, path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let apk = PathBuf::from(args.next().ok_or("usage: java_session_bench APK OUTPUT [--workers N] [--classpath PATH] [--class DESCRIPTOR]")?);
    let root = PathBuf::from(args.next().ok_or("missing output root")?);
    let mut options = JavaOptions::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--source-release" => {
                options.source_release = args.next().ok_or("missing source release")?.parse()?;
            }
            "--workers" => options.workers = args.next().ok_or("missing workers")?.parse()?,
            "--classpath" => options
                .classpath
                .push(args.next().ok_or("missing classpath")?.into()),
            "--class" => options.class_filter = Some(args.next().ok_or("missing descriptor")?),
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    if options.workers == 0 {
        return Err("workers must be positive".into());
    }
    fs::create_dir(&root)?;
    let setup = Instant::now();
    let files = dex_format::apk::parse_apk_path(apk)?;
    let mut session = JavaSession::new(files, options)?;
    let setup_s = setup.elapsed().as_secs_f64();
    let mut rows = Vec::new();
    let mut first_report = None;
    let mut first_checksum = None;
    for phase in ["cold", "warm"] {
        fs::create_dir(root.join(phase))?;
        let mut sink = DirectorySink::new(root.join(phase).join("sources"))?;
        let mut checksum = 0xcbf29ce484222325_u64;
        let mut sources = 0_usize;
        let mut bytes = 0_usize;
        let start = Instant::now();
        let report = session.emit(&mut |source: &GeneratedSource| {
            sink.emit(source)?;
            for byte in source
                .relative_path
                .to_string_lossy()
                .bytes()
                .chain([0])
                .chain(source.text.bytes())
                .chain([0])
            {
                checksum = (checksum ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
            sources += 1;
            bytes += source.text.len();
            Ok(())
        })?;
        let elapsed = start.elapsed().as_secs_f64();
        if phase == "cold" {
            first_report = Some(report.clone());
            first_checksum = Some(checksum);
        } else if first_report.as_ref() != Some(&report) || first_checksum != Some(checksum) {
            return Err("warm replay changed report or ordered source checksum".into());
        }
        let row = format!(
            "phase={phase} export_wall_s={elapsed:.6} sources={sources} bytes={bytes} checksum={checksum:016x} complete_methods={} constructors={} static_initializers={} unsupported_methods={} unavailable_constructors={} declaration_only={} stats={:?}",
            report.complete_methods,
            report.complete_constructors,
            report.complete_static_initializers,
            report.unsupported_methods,
            report.unavailable_constructors,
            report.declaration_only_methods,
            session.stats()
        );
        println!("{row}");
        fs::write(
            root.join(phase).join("report-debug.txt"),
            format!("{report:?}\n"),
        )?;
        rows.push(row);
    }
    fs::write(
        root.join("summary.txt"),
        format!("setup_s={setup_s:.6}\n{}\n", rows.join("\n")),
    )?;
    println!("setup_s={setup_s:.6} artifacts={}", root.display());
    Ok(())
}
