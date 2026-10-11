//! Development APK-to-Java subset CLI.
use decomp::java_external_metadata::{
    Classpath, DexMetadata, MetadataChain, MetadataProvider, emit_with_metadata,
};
use decomp::output::SourceSink;
use std::path::PathBuf;

const USAGE: &str = "usage: decomp <apk> <outdir> [--limit N] [--class FILTER] [--source-prefix PREFIX]... [--library-prefix PREFIX]... [--library-evidence PATH] [--library-min-score SCORE] [--library-out PATH] [--source-release 17|25] [--workers N] [--compact-source] [--classpath PATH]... [--metadata-dex PATH]...
  --source-prefix    repeatable original DEX owner prefix, e.g. Lcom/example/app/
  --library-prefix   retain matching owners as input metadata without Java output
  --library-evidence  verified-reference manifest; raw LibScout JSON is not accepted
  --library-min-score finite candidate score 0..1, default 0.95; exact implementation proof remains mandatory
  --library-out PATH  route qualified library sources in the selected scope to a separate directory
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
    let mut library_evidence = None::<PathBuf>;
    let mut library_out = None::<PathBuf>;
    let mut library_min_score = decomp::library_evidence::DEFAULT_MIN_SCORE;
    let mut library_score_supplied = false;
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
            "--library-evidence" => {
                library_evidence = Some(args.next().ok_or("missing library evidence path")?.into())
            }
            "--library-out" => {
                library_out = Some(
                    args.next()
                        .ok_or("missing library output directory")?
                        .into(),
                )
            }
            "--library-min-score" => {
                library_min_score = args.next().ok_or("missing library score")?.parse()?;
                decomp::library_evidence::validate_score(library_min_score)?;
                library_score_supplied = true;
            }
            "--class" => options.class_filter = Some(args.next().ok_or("missing class filter")?),
            "--source-prefix" | "--library-prefix" => {
                let prefix = args.next().ok_or("missing owner prefix")?;
                if prefix.is_empty() || !prefix.starts_with('L') {
                    return Err(
                        "owner prefix must be a nonempty original DEX prefix starting with L"
                            .into(),
                    );
                }
                if arg == "--source-prefix" {
                    options.source_prefixes.push(prefix);
                } else {
                    options.library_prefixes.push(prefix);
                }
            }
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    if library_evidence.is_none() && (library_score_supplied || library_out.is_some()) {
        return Err("--library-min-score and --library-out require --library-evidence".into());
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
    let files = dex_format::apk::parse_apk_path(&apk)?;
    let selection = library_evidence
        .as_ref()
        .map(|path| {
            decomp::library_evidence::select(
                path,
                std::path::Path::new(&apk),
                &files,
                library_min_score,
            )
        })
        .transpose()?;
    if library_out.is_none()
        && let Some(selection) = &selection
    {
        options.library_owners = selection.owners.clone();
    }
    let receipt_path = PathBuf::from(&out).join("library-selection.json");
    if selection.is_some() && receipt_path.exists() {
        return Err("library selection receipt already exists".into());
    }
    if let Some(library_out) = &library_out
        && library_out == std::path::Path::new(&out)
    {
        return Err("library output must be separate from application output".into());
    }
    let mut sink = if options.compact_source {
        decomp::output::DirectorySink::with_diagnostic_sidecar(&out)?
    } else {
        decomp::output::DirectorySink::new(&out)?
    };
    let mut library_sink = library_out
        .as_ref()
        .map(|path| {
            if options.compact_source {
                decomp::output::DirectorySink::with_diagnostic_sidecar(path)
            } else {
                decomp::output::DirectorySink::new(path)
            }
        })
        .transpose()?;
    if let Some(library_out) = &library_out {
        let app = std::fs::canonicalize(&out)?;
        let library = std::fs::canonicalize(library_out)?;
        if app.starts_with(&library) || library.starts_with(&app) {
            return Err("application and library output directories must not overlap".into());
        }
    }
    let mut app_files = 0usize;
    let mut library_files = 0usize;
    let mut routed = |source: &decomp::output::GeneratedSource| {
        if selection
            .as_ref()
            .is_some_and(|selection| selection.owners.contains(&source.provenance.descriptor))
            && let Some(library_sink) = &mut library_sink
        {
            library_sink.emit(source)?;
            library_files += 1;
            Ok(())
        } else {
            sink.emit(source)?;
            app_files += 1;
            Ok(())
        }
    };
    let report = if runtime_metadata.is_empty() && classpath.is_none() {
        decomp::dex_java::emit_java(&files, &options, &mut routed)?
    } else {
        let mut providers: Vec<_> = runtime_metadata.iter().map(MetadataProvider::Dex).collect();
        if let Some(classpath) = &classpath {
            providers.push(MetadataProvider::Jvm(classpath));
        }
        let metadata = MetadataChain::new(providers);
        emit_with_metadata(&files, &options, &mut routed, None, &metadata)?
    };
    if let Some(selection) = &selection {
        decomp::library_evidence::write_receipt(&receipt_path, selection)?;
        println!(
            "Library selection: min_score={library_min_score}; qualified_owners={}; emitted_app_files={app_files}; emitted_library_files={library_files}; receipt={}",
            selection.owners.len(),
            receipt_path.display()
        );
    }
    println!("Java subset report: {report:#?}");
    Ok(())
}
