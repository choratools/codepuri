//! Development APK-to-Java subset CLI.
fn main() {
    if let Err(e) = run() {
        eprintln!("decomp: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let apk = args.next().ok_or(
        "usage: decomp <apk> <outdir> [--limit N] [--class FILTER] [--source-release 17|25]",
    )?;
    let out = args.next().ok_or("missing output directory")?;
    let mut options = decomp::dex_java::JavaOptions::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--source-release" => {
                options.source_release = args.next().ok_or("missing source release")?.parse()?;
            }
            "--classpath" => options
                .classpath
                .push(args.next().ok_or("missing classpath")?.into()),
            "--limit" => options.limit = Some(args.next().ok_or("missing limit")?.parse()?),
            "--workers" => {
                options.workers = args.next().ok_or("missing workers")?.parse()?;
                if options.workers == 0 {
                    return Err("workers must be positive".into());
                }
            }
            "--class" => options.class_filter = Some(args.next().ok_or("missing class filter")?),
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    let files = dex_format::apk::parse_apk_path(apk)?;
    let mut sink = decomp::output::DirectorySink::new(out)?;
    let report = decomp::dex_java::emit_java(&files, &options, &mut sink)?;
    println!("Java subset report: {report:#?}");
    Ok(())
}
