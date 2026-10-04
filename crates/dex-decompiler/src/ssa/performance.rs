//! Opt-in stage measurement with the same descriptor-aware lift as DexPasses.
#![allow(clippy::unwrap_used)]

#[test]
#[ignore = "opt-in caller-controlled stage timings"]
fn calculator_stage_timings() {
    let path = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_device/com.sec.android.app.popupcalculator.apk"
    ));
    let dexes = dex_format::apk::parse_apk_path(path).unwrap();
    let mut count = 0;
    let mut totals = [std::time::Duration::ZERO; 9];
    for dex in &dexes {
        for class in &dex.classes {
            let Some(data) = &class.class_data else {
                continue;
            };
            for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                let Some(code) = &method.code else { continue };
                let ctx = crate::DexTypes::new(dex, Some(method.method_idx));
                let decoded =
                    crate::decode_method_recovering(code, crate::decode::DEFAULT_ERROR_LIMIT);
                let lifted = crate::lift::lift_recovered_with(&decoded, code, &ctx).unwrap();
                let merged = crate::merge_method(lifted).unwrap();
                let mut timing = super::SsaTimings::default();
                let ssa = super::to_ssa_with_timings(merged, &ctx, &mut timing).unwrap();
                for (total, elapsed) in totals.iter_mut().zip([
                    timing.analysis,
                    timing.placement,
                    timing.liveness,
                    timing.phi_placement,
                    timing.rename,
                    timing.phi_cleanup,
                    timing.materialize,
                    timing.inference,
                    timing.verification,
                ]) {
                    *total += elapsed;
                }
                count += 1;
                std::hint::black_box(ssa);
            }
        }
    }
    println!(
        "methods={count} stage_us={:?} stages=analysis,placement,liveness,phi_placement,rename,phi_cleanup,materialize,inference,verification",
        totals.map(|duration| duration.as_micros())
    );
}
