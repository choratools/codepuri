//! Hexdumps raw DEX bytes around code_item offsets to diagnose header
//! alignment.
//!
//! Run: `cargo run --release -p dex-decompiler --example hexdump -- <apk>`

use dex_format::apk::parse_apk_path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: hexdump <apk>");
        std::process::exit(2);
    };
    let dexes = match parse_apk_path(path) {
        Ok(dexes) => dexes,
        Err(error) => {
            eprintln!("parse failed: {error}");
            std::process::exit(1);
        }
    };
    let dex = &dexes[0];
    let raw = dex.raw_bytes();
    let offsets = [1_724_160usize, 1_724_200, 1_724_228];
    for code_off in offsets {
        let start = code_off.saturating_sub(8);
        let end = (code_off + 48).min(raw.len());
        println!("code_off={code_off}");
        for addr in (start..end).step_by(16) {
            let stop = (addr + 16).min(end);
            let mut line = format!("{addr:08x}: ");
            for byte in &raw[addr..stop] {
                line.push_str(&format!("{byte:02x} "));
            }
            println!("{line}");
        }
        println!();
    }
}
