//! Histograms SSA verification failure diagnostic codes for a given APK and
//! prints sample failing methods per code.
//!
//! Run: `cargo run --release -p dex-decompiler --example ssa_probe -- <apk> [limit]`

use std::collections::BTreeMap;

use dex_decompiler::passes::DexPasses;
use dex_format::apk::parse_apk_path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: ssa_probe <apk> [limit]");
        std::process::exit(2);
    };
    let limit: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(3);

    let dexes = match parse_apk_path(std::path::Path::new(path)) {
        Ok(dexes) => dexes,
        Err(error) => {
            eprintln!("parse failed: {error}");
            std::process::exit(1);
        }
    };

    let mut codes: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut samples: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    let mut failed = 0u64;
    let mut total = 0u64;
    let mut inference_codes = BTreeMap::new();
    let mut inference_samples = Vec::new();

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
                let Some(code_item) = &method.code else {
                    continue;
                };
                if let Ok(filter) = std::env::var("SSA_METHOD")
                    && filter.parse::<u32>().ok() != Some(method.method_idx)
                {
                    continue;
                }
                total += 1;
                let context = DexTypes::new(dex, Some(method.method_idx));
                let error = match DexPasses::run_ssa_with(code_item, &context) {
                    Ok(ssa) => {
                        if std::env::var_os("SSA_DUMP").is_some() {
                            for (id, value) in ssa.ir().values() {
                                println!("VALUE {id:?} {:?} def={:?}", value.ty(), value.def());
                            }
                            for (id, insn) in ssa.ir().instructions() {
                                println!("INSN {id:?} off={:?} {insn:?}", ssa.offset(id));
                            }
                        }
                        for diagnostic in ssa.diagnostics() {
                            *inference_codes.entry(diagnostic.code).or_insert(0u64) += 1;
                            if inference_samples.len() < limit {
                                inference_samples.push(format!(
                                    "method_idx={}: {}",
                                    method.method_idx, diagnostic.message
                                ));
                            }
                        }
                        continue;
                    }
                    Err(error) => error,
                };
                failed += 1;
                let diagnostics: &[decomp_core::diagnostics::Diagnostic] = match &error {
                    dex_decompiler::StageError::Ssa(dex_decompiler::SsaError::Verification(
                        list,
                    )) => list,
                    other => {
                        *codes.entry("STRUCTURE").or_default() += 1;
                        if samples.entry("STRUCTURE").or_default().len() < limit {
                            samples
                                .entry("STRUCTURE")
                                .or_default()
                                .push(format!("method_idx={} {other}", method.method_idx));
                        }
                        continue;
                    }
                };
                for diagnostic in diagnostics {
                    *codes.entry(diagnostic.code).or_default() += 1;
                    let bucket = samples.entry(diagnostic.code).or_default();
                    if bucket.len() < limit {
                        bucket.push(format!(
                            "method_idx={} off={}: {}",
                            method.method_idx, code_item.span.offset, diagnostic.message
                        ));
                    }
                }
            }
        }
    }

    println!("total={total} failed={failed}");
    if std::env::var_os("SSA_TYPE_DIAGNOSTICS").is_some() {
        println!("inference diagnostics: {inference_codes:?}");
        for sample in inference_samples {
            println!("  {sample}");
        }
    }
    let mut ranked: Vec<_> = codes.into_iter().collect();
    ranked.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for (code, count) in &ranked {
        println!("  {code:45} {count}");
    }
    println!();
    for (code, _) in &ranked {
        println!("[{code}]");
        for sample in &samples[code] {
            println!("  {sample}");
        }
    }
}

// Re-export shim so the probe compiles standalone: `DexTypes` lives in the
// same public path the sweep example uses.
use dex_decompiler::DexTypes;
