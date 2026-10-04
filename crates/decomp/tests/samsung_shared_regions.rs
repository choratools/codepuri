//! Selected real Samsung methods only: no full-APK decompilation/export.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use decomp_structure::extended::{FlowStructuredCfg, structure_flow};
use std::io::Read;

#[test]
fn three_recorded_samsung_overlaps_have_exact_forward_coverage()
-> Result<(), Box<dyn std::error::Error>> {
    let apk = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
    let mut zip = zip::ZipArchive::new(std::fs::File::open(apk)?)?;
    let mut bytes = Vec::new();
    zip.by_name("classes.dex")?.read_to_end(&mut bytes)?;
    let dex = dex_format::DexFile::from_bytes(&bytes)?;
    let targets = [
        (42, "La2/a;", "cancel"),
        (64, "La2/a;", "v"),
        (104, "La4/a$b;", "readShort"),
    ];
    let mut proven = 0;
    for class in &dex.classes {
        let Some(data) = &class.class_data else {
            continue;
        };
        for method in data.direct_methods.iter().chain(&data.virtual_methods) {
            let Some(&(_, owner, name)) = targets.iter().find(|t| t.0 == method.method_idx) else {
                continue;
            };
            let id = &dex.method_ids[method.method_idx as usize];
            assert_eq!(
                dex.string(dex.type_ids[id.class_idx as usize].descriptor_idx)?,
                owner
            );
            assert_eq!(dex.string(id.name_idx)?, name);
            let code = method.code.as_ref().expect("recorded code-bearing method");
            let context = dex_decompiler::DexTypes::new(&dex, Some(method.method_idx));
            let ssa = dex_decompiler::DexPasses::run_ssa_with(code, &context)?;
            let cfg = ssa.ir().cfg();
            println!(
                "{owner}->{name} #{}: strict={:?}, B={}, E={}",
                method.method_idx,
                decomp_structure::structure_extended(cfg).as_ref().err(),
                cfg.block_count(),
                cfg.edge_count()
            );
            let flow = structure_flow(cfg)?;
            let FlowStructuredCfg::Shared(regions) = flow else {
                panic!("recorded overlap must use shared flow")
            };
            // Show the actual shared entry and its postdominator relation.
            // A common terminal behind only some arms is not a SESE join.
            for region in &regions.blocks {
                let block = cfg.block(region.block)?;
                if block.incoming().len() > 1 {
                    println!(
                        "  shared {:?}, postdom={:?}, instruction={:?}",
                        region.block,
                        region.postdominator,
                        ssa.ir().instructions().get(*block.payload())
                    );
                }
            }
            assert_eq!(regions.blocks.len(), cfg.block_count());
            let distinct: std::collections::HashSet<_> =
                regions.blocks.iter().map(|b| b.block).collect();
            assert_eq!(distinct, cfg.block_ids().collect());
            assert_eq!(regions.edges.len(), cfg.edge_count());
            for edge in &regions.edges {
                let original = cfg.edge(edge.id)?;
                assert_eq!(original.source(), edge.source);
                assert_eq!(original.target(), edge.target);
                assert_eq!(original.kind(), &edge.kind);
                println!("  {:?} -> {:?} {:?}", edge.source, edge.target, edge.kind);
            }
            proven += 1;
        }
    }
    assert_eq!(proven, 3);
    Ok(())
}
