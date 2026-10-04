//! Bounded original producer inventory. No facade export or synthetic bodies.
#![allow(clippy::unwrap_used)]
use dex_decompiler::{
    DexPasses, DexTypes,
    infer::{DexMetadata, TypeContext},
};
use dex_format::DexFile;
use managed_ir::insn::ManagedInsn;

fn descriptor(dex: &DexFile, index: u32) -> &str {
    dex.string(dex.type_id(index).unwrap().descriptor_idx)
        .unwrap()
}
fn method_name(dex: &DexFile, index: u32) -> String {
    let m = dex.method_id(index).unwrap();
    let p = dex.proto_id(u32::from(m.proto_idx)).unwrap();
    let args = p
        .parameters
        .iter()
        .flat_map(|p| &p.types)
        .map(|t| descriptor(dex, u32::from(*t)))
        .collect::<Vec<_>>()
        .join("");
    format!(
        "{}->{}({args}){}",
        descriptor(dex, u32::from(m.class_idx)),
        dex.string(m.name_idx).unwrap(),
        descriptor(dex, p.return_type_idx)
    )
}
#[test]
#[ignore = "requires retained Samsung APK; six selected seeded methods only, no export"]
fn original_wave2_six_ordering_samples() {
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let dex = &inputs[0];
    let metadata = DexMetadata::new(dex);
    let targets = std::env::var("DECOMPHIFY_OBJECT_SAMPLE_INDICES")
        .map(|indices| {
            indices
                .split(',')
                .map(|index| index.parse::<u32>().unwrap())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|_| vec![4, 112, 180, 5892, 5015, 15061]);
    for index in targets {
        let original = dex
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|c| c.direct_methods.iter().chain(&c.virtual_methods))
            .find(|m| m.method_idx == index)
            .unwrap();
        let code = original.code.as_ref().unwrap();
        let raw = dex.method_id(index).unwrap();
        let ctx = DexTypes::with_metadata(&metadata, Some(index));
        let layout = decomp::java_incoming_registers::IncomingLayout::from_proto(
            dex,
            dex.proto_id(u32::from(raw.proto_idx)).unwrap(),
            original.access_flags & 8 != 0,
            code.registers_size,
            code.ins_size,
        )
        .unwrap();
        let mut seeds = std::collections::HashMap::new();
        if let Some(slot) = layout.receiver {
            seeds.insert(
                (slot.register, false),
                ctx.type_at(u32::from(raw.class_idx)).unwrap(),
            );
        }
        let mut types = vec![];
        assert!(ctx.method_params(index, &mut types));
        assert_eq!(types.len(), layout.parameters.len());
        for (slot, ty) in layout.parameters.iter().zip(types) {
            seeds.insert((slot.register, slot.high_register.is_some()), ty);
        }
        let ssa = DexPasses::run_ssa_with(code, &ctx.with_initial_registers(seeds)).unwrap();
        eprintln!(
            "SAMPLE pool=0 index={index} identity={} registers={} incoming={} words={} tries={:?}",
            method_name(dex, index),
            code.registers_size,
            code.ins_size,
            code.insns_size,
            code.tries
        );
        for (id, insn) in ssa.ir().instructions() {
            let symbol = match insn {
                ManagedInsn::Invoke { method, .. } => method_name(dex, method.index()),
                ManagedInsn::NewInstance { class, .. } => descriptor(dex, class.index()).to_owned(),
                ManagedInsn::FieldGet { field, .. } | ManagedInsn::FieldPut { field, .. } => {
                    let f = dex.field_id(field.index()).unwrap();
                    format!(
                        "{}->{}:{}",
                        descriptor(dex, u32::from(f.class_idx)),
                        dex.string(f.name_idx).unwrap(),
                        descriptor(dex, u32::from(f.type_idx))
                    )
                }
                ManagedInsn::Const {
                    value: managed_ir::ops::Constant::String(s),
                    ..
                } => format!("{:?}", dex.string(s.index()).unwrap()),
                _ => String::new(),
            };
            eprintln!("{id:?} off={:?} {insn:?} {symbol}", ssa.offset(id));
        }
        for (id, value) in ssa.ir().values() {
            eprintln!("{id:?} type={:?} def={:?}", value.ty(), value.def());
        }
    }
}

#[test]
#[ignore = "requires wave2 selected identity TSV and Samsung APK; raw opcode incidence only, no export"]
fn wave2_raw_allocation_gap_census() {
    use dex_decompiler::decode::{Opcode, SemanticClass};
    use std::collections::{BTreeMap, HashMap, HashSet};
    let targets =
        std::fs::read_to_string(std::env::var_os("DECOMPHIFY_OBJECT_WAVE2_TARGETS").unwrap())
            .unwrap();
    let targets = targets
        .lines()
        .map(|line| {
            let mut fields = line.split('\t');
            (
                (
                    fields.next().unwrap().parse::<usize>().unwrap(),
                    fields.next().unwrap().parse::<u32>().unwrap(),
                ),
                fields.next().unwrap().to_owned(),
            )
        })
        .collect::<HashMap<_, _>>();
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let mut counts = BTreeMap::<String, usize>::new();
    let mut examples = BTreeMap::<String, Vec<String>>::new();
    let mut matched = 0;
    for (pool, dex) in inputs.iter().enumerate() {
        for method in dex
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|c| c.direct_methods.iter().chain(&c.virtual_methods))
        {
            let Some(kind) = targets.get(&(pool, method.method_idx)) else {
                continue;
            };
            matched += 1;
            let decoded = dex_decompiler::decode_method(method.code.as_ref().unwrap()).unwrap();
            let mut families = HashSet::new();
            for (position, allocation) in decoded.iter().enumerate() {
                let Opcode::Supported(info) = allocation.opcode else {
                    continue;
                };
                if info.opcode != 0x22 {
                    continue;
                };
                let register = allocation.registers[0];
                let class = allocation.pool_index.unwrap();
                let end = decoded
                    .iter()
                    .enumerate()
                    .skip(position + 1)
                    .take(64)
                    .find(|(_, insn)| {
                        let Opcode::Supported(info) = insn.opcode else {
                            return false;
                        };
                        if !matches!(info.opcode, 0x70 | 0x76)
                            || insn.registers.first() != Some(&register)
                        {
                            return false;
                        };
                        let target = dex.method_id(insn.pool_index.unwrap()).unwrap();
                        dex.string(target.name_idx).unwrap() == "<init>"
                            && u32::from(target.class_idx) == class
                    })
                    .map(|(end, _)| end);
                let Some(end) = end else { continue };
                let mut window = HashSet::new();
                for insn in &decoded[position + 1..end] {
                    let Opcode::Supported(info) = insn.opcode else {
                        continue;
                    };
                    let family = match info.opcode {
                        0x21 => Some("array_length"),
                        0x44..=0x4a => Some("array_read"),
                        0x4b..=0x51 => Some("array_store"),
                        0x59..=0x5f | 0x67..=0x6d => Some("field_store"),
                        _ if matches!(
                            info.class,
                            SemanticClass::Branch | SemanticClass::Switch
                        ) =>
                        {
                            Some("branch_or_switch")
                        }
                        _ => None,
                    };
                    if let Some(family) = family {
                        window.insert(family);
                    }
                    if matches!(info.class, SemanticClass::Invoke) {
                        let target = dex.method_id(insn.pool_index.unwrap()).unwrap();
                        let p = dex.proto_id(u32::from(target.proto_idx)).unwrap();
                        if dex.string(target.name_idx).unwrap() != "<init>"
                            && descriptor(dex, p.return_type_idx) == "V"
                        {
                            window.insert("void_call");
                        }
                    }
                }
                if !window.contains("branch_or_switch")
                    && !window.contains("void_call")
                    && !window.contains("array_store")
                    && !window.contains("field_store")
                {
                    if window.contains("array_length") || window.contains("array_read") {
                        window.insert("linear_array_read_or_length_no_store_or_void");
                    }
                    if window.contains("array_length") {
                        window.insert("linear_array_length_no_store_or_void");
                    }
                    if window.contains("array_read") {
                        window.insert("linear_array_read_no_store_or_void");
                    }
                }
                families.extend(window);
            }
            for family in families {
                let key = format!("{kind}:{family}");
                *counts.entry(key.clone()).or_default() += 1;
                let examples = examples.entry(key).or_default();
                if examples.len() < 6 {
                    examples.push(format!(
                        "pool={pool} index={} {}",
                        method.method_idx,
                        method_name(dex, method.method_idx)
                    ));
                }
            }
        }
    }
    assert_eq!(matched, targets.len());
    eprintln!("RAW INCIDENCE ONLY matched={matched} counts={counts:?} examples={examples:#?}");
}

#[test]
#[ignore = "requires Samsung APK and four genuine metadata paths; one bounded original class facade export"]
fn original_array_node_facade() {
    use decomp::{
        dex_java::{JavaOptions, emit_java},
        output::GeneratedSource,
    };
    use std::{
        path::{Path, PathBuf},
        time::Instant,
    };
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let owner =
        std::env::var("DECOMPHIFY_OBJECT_ARRAY_OWNER").unwrap_or_else(|_| "La4/a$c;".into());
    let indices = match owner.as_str() {
        "La4/a$c;" => vec![112, 114, 116, 118],
        "La4/b;" => vec![180],
        _ => panic!("unscoped owner"),
    };
    let dex = &inputs[0];
    for index in &indices {
        let method = dex
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|c| c.direct_methods.iter().chain(&c.virtual_methods))
            .find(|m| m.method_idx == *index)
            .unwrap();
        let raw = dex.method_id(*index).unwrap();
        assert_eq!(descriptor(dex, u32::from(raw.class_idx)), owner);
        let code = method.code.as_ref().unwrap();
        assert!(code.tries.is_empty());
        eprintln!(
            "original pool=0 index={index} identity={} registers={} incoming={} raw_words={:?}",
            method_name(dex, *index),
            code.registers_size,
            code.ins_size,
            code.insns
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>()
        );
    }
    let classpath = [
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod",
        "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/jdk.unsupported.jmod",
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.naming.jmod",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect::<Vec<_>>();
    for path in &classpath {
        assert!(path.is_file());
    }
    let options = JavaOptions {
        source_release: 25,
        workers: 1,
        class_filter: Some(owner.clone()),
        classpath,
        ..JavaOptions::default()
    };
    let start = Instant::now();
    let mut sources = vec![];
    let report = emit_java(&inputs, &options, &mut |unit: &GeneratedSource| {
        sources.push(unit.clone());
        Ok(())
    })
    .unwrap();
    assert_eq!(sources.len(), 1, "{report:?}");
    let unit = sources.remove(0);
    assert_eq!(unit.provenance.descriptor, owner);
    let root = std::env::var_os("DECOMPHIFY_OBJECT_ARRAY_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!(
                "decomphify-array-node-actual-{}",
                std::process::id()
            ))
        });
    let original = root.join("original-units").join(&unit.relative_path);
    std::fs::create_dir_all(original.parent().unwrap()).unwrap();
    std::fs::write(&original, &unit.text).unwrap();
    std::fs::write(root.join("report.txt"), format!("{report:#?}")).unwrap();
    for index in &indices {
        let identity = unit
            .text
            .lines()
            .find(|line| line.contains(&format!("original method pool index {index};")));
        eprintln!(
            "actual array-node owner={owner} original={index} admitted={} identity={identity:?}",
            identity.is_some()
        );
    }
    let required = if owner == "La4/b;" { 180 } else { 112 };
    let marker = format!("original method pool index {required};");
    let identity = unit
        .text
        .lines()
        .find(|line| line.contains(&marker))
        .unwrap_or_else(|| {
            panic!(
                "required original {required} rejected; retained {}",
                original.display()
            )
        });
    assert!(
        identity.contains("complete_body=true")
            && identity.contains("semantic_complete=true")
            && identity.contains("dependency_complete=true")
    );
    let tail = &unit.text[unit.text.find(&marker).unwrap()..];
    let body = tail
        .split("    // original method pool index")
        .next()
        .unwrap();
    assert!(
        body.lines()
            .any(|line| line.contains("new ") && line.contains(".length")),
        "array length must remain within original allocation expression: {body}"
    );
    assert_eq!(std::fs::read_to_string(&original).unwrap(), unit.text);
    std::fs::write(root.join(Path::new("scope.txt")),"Original selected class emitted through seeded production facade with four genuine metadata paths. Entire source retained unchanged; this test does not claim whole-class compilation or real APK runtime.\n").unwrap();
    eprintln!(
        "actual array-node required original={required} complete; elapsed_s={:.3} unchanged unit={}",
        start.elapsed().as_secs_f64(),
        original.display()
    );
}
