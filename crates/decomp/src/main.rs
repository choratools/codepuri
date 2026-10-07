//! Development APK-to-Java subset CLI.
use decomp::java_external_metadata::{
    Classpath, DexMetadata, MetadataChain, MetadataProvider, emit_with_metadata,
};
use std::path::PathBuf;

const USAGE: &str = "usage: decomp <apk> <outdir> [--limit N] [--class FILTER] [--source-release 17|25] [--workers N] [--compact-source] [--classpath PATH]... [--metadata-dex PATH]...
  --compact-source    remove standalone diagnostic comments; write diagnostics.tsv
  --classpath PATH    repeatable JVM .class/.jar/.zip/.jmod declarations
  --metadata-dex PATH repeatable raw DEX or ZIP/APK with root classes*.dex; runtime declarations only, not a javac classpath
  Runtime DEX metadata takes precedence over JVM classpath metadata; whole class declarations are not merged.";

fn main() {
    if let Err(e) = run() {
        eprintln!("decomp: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(apk) = args.next() else {
        return Err(USAGE.into());
    };
    if matches!(apk.as_str(), "--help" | "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let out = args.next().ok_or("missing output directory")?;
    let mut options = decomp::dex_java::JavaOptions::default();
    let mut metadata_dex_paths = Vec::<PathBuf>::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--source-release" => {
                options.source_release = args.next().ok_or("missing source release")?.parse()?;
            }
            "--classpath" => options
                .classpath
                .push(args.next().ok_or("missing classpath")?.into()),
            "--metadata-dex" => metadata_dex_paths.push(
                args.next()
                    .ok_or("missing external DEX metadata path")?
                    .into(),
            ),
            "--limit" => options.limit = Some(args.next().ok_or("missing limit")?.parse()?),
            "--workers" => {
                options.workers = args.next().ok_or("missing workers")?.parse()?;
                if options.workers == 0 {
                    return Err("workers must be positive".into());
                }
            }
            "--compact-source" => options.compact_source = true,
            "--class" => options.class_filter = Some(args.next().ok_or("missing class filter")?),
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    let runtime_metadata = metadata_dex_paths
        .iter()
        .map(DexMetadata::from_path)
        .collect::<Result<Vec<_>, _>>()?;
    let classpath = if options.classpath.is_empty() {
        None
    } else {
        Some(Classpath::from_paths(&options.classpath)?)
    };
    let files = dex_format::apk::parse_apk_path(apk)?;
    let mut sink = if options.compact_source {
        decomp::output::DirectorySink::with_diagnostic_sidecar(out)?
    } else {
        decomp::output::DirectorySink::new(out)?
    };
    let report = if runtime_metadata.is_empty() && classpath.is_none() {
        decomp::dex_java::emit_java(&files, &options, &mut sink)?
    } else {
        let mut providers: Vec<_> = runtime_metadata.iter().map(MetadataProvider::Dex).collect();
        if let Some(classpath) = &classpath {
            providers.push(MetadataProvider::Jvm(classpath));
        }
        let metadata = MetadataChain::new(providers);
        emit_with_metadata(&files, &options, &mut sink, None, &metadata)?
    };
    println!("Java subset report: {report:#?}");
    Ok(())
}
