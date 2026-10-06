//! Minimal, unprofiled APK-to-Java export process for paired throughput tests.
use decomp::dex_java::{JavaOptions, emit_java};
use decomp::output::DirectorySink;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let apk = PathBuf::from(args.next().ok_or(
        "usage: java_export_bench APK OUTPUT [--workers N] [--source-release N] [--fast-proofs]",
    )?);
    let output = PathBuf::from(args.next().ok_or("missing output directory")?);
    let mut options = JavaOptions::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--workers" => {
                options.workers = args.next().ok_or("missing worker count")?.parse()?;
            }
            "--source-release" => {
                options.source_release = args.next().ok_or("missing source release")?.parse()?;
            }
            "--fast-proofs" => options.fast_proofs = true,
            _ => return Err(format!("unknown option: {arg}").into()),
        }
    }
    if options.workers == 0 {
        return Err("workers must be positive".into());
    }

    let files = dex_format::apk::parse_apk_path(apk)?;
    std::fs::create_dir(&output)?;
    let mut sink = DirectorySink::new(output.join("sources"))?;
    let _report = emit_java(&files, &options, &mut sink)?;
    Ok(())
}
