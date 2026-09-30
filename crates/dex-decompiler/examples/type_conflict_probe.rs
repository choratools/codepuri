//! Targeted APK conflict inventory. Optional method-name substring dumps raw/SSA context.
#![allow(clippy::unwrap_used)]
use dex_decompiler::{DexPasses, DexTypes, decode_method};
use dex_format::{DexFile, apk::parse_apk_path};

fn ty(dex: &DexFile, index: u16) -> &str {
    dex.string(dex.type_ids[index as usize].descriptor_idx)
        .unwrap()
}

fn method_name(dex: &DexFile, index: u32) -> String {
    let id = &dex.method_ids[index as usize];
    let proto = &dex.proto_ids[id.proto_idx as usize];
    let params = proto
        .parameters
        .as_ref()
        .map(|p| p.types.iter().map(|i| ty(dex, *i)).collect::<String>())
        .unwrap_or_default();
    format!(
        "{}->{}({}){}",
        ty(dex, id.class_idx),
        dex.string(id.name_idx).unwrap(),
        params,
        ty(dex, proto.return_type_idx as u16)
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("expected APK path")?;
    let filter = args.next();
    let mut methods = 0;
    let mut affected = 0;
    let mut conflicts = 0;
    let mut errors = 0;
    for (di, dex) in parse_apk_path(std::path::Path::new(&path))?
        .iter()
        .enumerate()
    {
        let results = std::thread::scope(|scope| {
            let handles: Vec<_> = dex
                .classes
                .chunks(dex.classes.len().div_ceil(32).max(1))
                .map(|classes| {
                    let filter = &filter;
                    scope.spawn(move || {
                        let (mut methods, mut affected, mut conflicts, mut errors) = (0, 0, 0, 0);
                        for class in classes {
                            let Some(data) = &class.class_data else {
                                continue;
                            };
                            for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                                let Some(code) = &method.code else { continue };
                                let name = method_name(dex, method.method_idx);
                                if filter.as_ref().is_some_and(|f| !name.contains(f)) {
                                    continue;
                                }
                                methods += 1;
                                let ssa = match DexPasses::run_ssa_with(
                                    code,
                                    &DexTypes::new(dex, Some(method.method_idx)),
                                ) {
                                    Ok(ssa) => ssa,
                                    Err(e) => {
                                        errors += 1;
                                        println!("ERROR {name}: {e}");
                                        continue;
                                    }
                                };
                                let ds: Vec<_> = ssa
                                    .diagnostics()
                                    .iter()
                                    .filter(|d| d.code() == "DEX_TYPE_CONFLICT")
                                    .collect();
                                if !ds.is_empty() {
                                    affected += 1;
                                    conflicts += ds.len();
                                }
                                if ds.is_empty() && filter.is_none() {
                                    continue;
                                }
                                println!(
                                    "METHOD dex={di} idx={} {name} registers={} ins={} units={}",
                                    method.method_idx,
                                    code.registers_size,
                                    code.ins_size,
                                    code.insns_size
                                );
                                for d in ds {
                                    println!("  {name}: {}", d.message);
                                }
                                if filter.is_some() {
                                    println!("TRIES {:?} HANDLERS {:?}", code.tries, code.handlers);
                                    match decode_method(code) {
                                        Ok(decoded) => {
                                            for raw in decoded {
                                                if let (
                                                    dex_decompiler::decode::Opcode::Supported(op),
                                                    Some(pool),
                                                ) = (raw.opcode, raw.pool_index)
                                                {
                                                    let resolved = match op.opcode {
                                                        0x6e..=0x72 | 0x74..=0x78 => {
                                                            Some(method_name(dex, pool))
                                                        }
                                                        0x1c | 0x1f | 0x20 | 0x22..=0x25 => {
                                                            Some(ty(dex, pool as u16).to_owned())
                                                        }
                                                        0x52..=0x6d => {
                                                            let f = &dex.field_ids[pool as usize];
                                                            Some(format!(
                                                                "{}->{}:{}",
                                                                ty(dex, f.class_idx),
                                                                dex.string(f.name_idx).unwrap(),
                                                                ty(dex, f.type_idx)
                                                            ))
                                                        }
                                                        _ => None,
                                                    };
                                                    if let Some(resolved) = resolved {
                                                        println!(
                                                            "POOL @0x{:x} {resolved}",
                                                            raw.offset
                                                        );
                                                    }
                                                }
                                                println!("RAW {raw:?}");
                                            }
                                        }
                                        Err(error) => println!("RAW_DECODE_ERROR {error}"),
                                    }
                                    for (iid, record) in ssa.ir().instructions().iter() {
                                        println!("SSA {iid:?} {:?} {record:?}", ssa.offset(iid));
                                    }
                                    for (vid, value) in ssa.ir().values().iter() {
                                        println!(
                                            "VALUE {vid:?} def={:?} type={:?}",
                                            value.def(),
                                            value.ty()
                                        );
                                    }
                                }
                            }
                        }
                        (methods, affected, conflicts, errors)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect::<Vec<_>>()
        });
        for (m, a, c, e) in results {
            methods += m;
            affected += a;
            conflicts += c;
            errors += e;
        }
    }
    println!("TOTAL methods={methods} affected={affected} conflicts={conflicts} errors={errors}");
    Ok(())
}
