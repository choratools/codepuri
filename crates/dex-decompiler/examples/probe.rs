//! Dumps raw insns bytes of the first failing methods of a given APK to
//! diagnose systematic decode failures.
//!
//! Run: `cargo run --release -p dex-decompiler --example probe -- <apk> [limit]`

use dex_decompiler::decode_method;
use dex_format::apk::parse_apk_path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: probe <apk> [limit]");
        std::process::exit(2);
    };
    let limit: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(5);
    let dexes = match parse_apk_path(path) {
        Ok(dexes) => dexes,
        Err(error) => {
            eprintln!("parse failed: {error}");
            std::process::exit(1);
        }
    };
    eprintln!("dexes: {}", dexes.len());
    let mut scanned = 0u64;
    let mut failures = 0u64;
    for (dex_idx, dex) in dexes.iter().enumerate() {
        let mut dex_methods = 0u64;
        for class in &dex.classes {
            let Some(data) = &class.class_data else {
                continue;
            };
            for method in data
                .direct_methods
                .iter()
                .chain(data.virtual_methods.iter())
            {
                let Some(code) = &method.code else {
                    continue;
                };
                dex_methods += 1;
                scanned += 1;
                if decode_method(code).is_err() {
                    failures += 1;
                    if failures <= u64::try_from(limit).unwrap_or(u64::MAX) {
                        println!(
                            "dex#{dex_idx} method@{} code_off={} regs={} insns_size={} units",
                            method.method_idx,
                            code.span.offset,
                            code.registers_size,
                            code.insns_size
                        );
                        println!("  reason: {}", inspect_reason(code));
                        println!("  bytes: {}", hex(&code.insns[..code.insns.len().min(48)]));
                    }
                }
            }
        }
        eprintln!("dex#{dex_idx}: methods_with_code={dex_methods}");
        let _ = &mut scanned;
    }
    eprintln!("scanned={scanned} failures={failures}");
}

fn inspect_reason(code: &dex_format::code::CodeItem) -> String {
    match decode_method(code) {
        Ok(_) => "decoded".into(),
        Err(error) => error.to_string(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
