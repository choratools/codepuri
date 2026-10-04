//! Opt-in bounded stress measurement of the three largest calculator methods.
//! Run release with --ignored --nocapture --test-threads=1.
#![allow(clippy::unwrap_used)]
use dex_decompiler::{DexPasses, DexTypes};
use dex_format::apk::parse_apk_path;
use std::time::Instant;

#[test]
#[ignore = "bounded release performance measurement"]
fn calculator_large_methods() {
    let path = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_device/com.sec.android.app.popupcalculator.apk"
    ));
    let dexes = parse_apk_path(path).unwrap();
    let mut methods: Vec<_> = dexes
        .iter()
        .flat_map(|dex| {
            dex.classes
                .iter()
                .filter_map(|class| class.class_data.as_ref())
                .flat_map(move |data| {
                    data.direct_methods
                        .iter()
                        .chain(&data.virtual_methods)
                        .filter_map(move |method| {
                            method
                                .code
                                .as_ref()
                                .map(|code| (dex, method.method_idx, code))
                        })
                })
        })
        .collect();
    methods.sort_by_key(|(_, _, code)| std::cmp::Reverse(code.insns.len()));
    for (dex, id, code) in methods.into_iter().take(3) {
        let ctx = DexTypes::new(dex, Some(id));
        let start = Instant::now();
        let mut stats = None;
        for _ in 0..10 {
            let ssa = DexPasses::run_ssa_with(code, &ctx).unwrap();
            stats = Some(*ssa.stats());
            std::hint::black_box(ssa);
        }
        println!(
            "method={id} code_units={} iterations=10 elapsed_us={} stats={:?}",
            code.insns_size,
            start.elapsed().as_micros(),
            stats.unwrap()
        );
    }
}
