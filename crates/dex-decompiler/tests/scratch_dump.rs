//! Temporary debug dump for a single failing method (scratch, to be removed).

use dex_decompiler::{DexPasses, DexTypes, SsaError, StageError};
use dex_format::apk::parse_apk_path;

#[test]
fn dump_method() -> Result<(), Box<dyn std::error::Error>> {
    let apk = std::path::Path::new("../..");
    let apk = if apk
        .join("tests/corpus/apks_device/com.sec.android.app.popupcalculator.apk")
        .exists()
    {
        apk.join("tests/corpus/apks_device/com.sec.android.app.popupcalculator.apk")
    } else {
        std::path::PathBuf::from("tests/corpus/apks_device/com.sec.android.app.popupcalculator.apk")
    };
    let target_idx: u32 = std::env::var("DUMP_METHOD_IDX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8223);
    let dexes = parse_apk_path(&apk)?;
    'outer: for dex in &dexes {
        for class in &dex.classes {
            let Some(class_data) = &class.class_data else {
                continue;
            };
            for method in class_data
                .direct_methods
                .iter()
                .chain(class_data.virtual_methods.iter())
            {
                if method.method_idx != target_idx {
                    continue;
                }
                let Some(code_item) = &method.code else {
                    continue;
                };
                println!("=== method_idx={} ===", method.method_idx);
                let mid = &dex.method_ids[method.method_idx as usize];
                let proto = dex.proto_id(u32::from(mid.proto_idx))?;
                let shorty = dex.string(proto.shorty_idx)?;
                println!(
                    "class={} name={} shorty={}",
                    dex.string(dex.type_ids[mid.class_idx as usize].descriptor_idx)?,
                    dex.string(mid.name_idx)?,
                    shorty,
                );
                if let Some(params) = &proto.parameters {
                    for (i, ty) in params.types.iter().enumerate() {
                        println!(
                            "  param{i}: {}",
                            dex.string(dex.type_ids[*ty as usize].descriptor_idx)?
                        );
                    }
                }
                for (i, unit) in code_item.insns.chunks(2).enumerate().take(80) {
                    let word = u16::from_le_bytes([unit[0], *unit.get(1).unwrap_or(&0)]);
                    println!("  {i:4}: {word:04x}");
                }
                let context = DexTypes::new(dex, Some(method.method_idx));
                match DexPasses::run_ssa_with(code_item, &context) {
                    Ok(_ssa) => println!("OK"),
                    Err(StageError::Ssa(SsaError::Verification(diags))) => {
                        for d in diags {
                            println!("DIAG: {}", d.message);
                        }
                    }
                    Err(other) => println!("OTHER ERR: {other}"),
                }
                let lifted = dex_decompiler::DexPasses::run(code_item)?;
                println!("--- lifted insns (offset: insn) ---");
                let cfg = lifted.ir().cfg();
                let mut lines: Vec<(u32, String)> = Vec::new();
                for block in cfg.block_ids() {
                    let insn = *cfg.block(block)?.payload();
                    if let (Some(off), Some(record)) =
                        (lifted.offset(insn), lifted.ir().instructions().get(insn))
                    {
                        lines.push((off, format!("{insn:?} = {record:?}")));
                    }
                }
                lines.sort_by_key(|(off, _)| *off);
                for (off, line) in lines {
                    println!("{off:6}: {line}");
                }
                break 'outer;
            }
        }
    }
    Ok(())
}
