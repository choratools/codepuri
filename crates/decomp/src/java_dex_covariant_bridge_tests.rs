#![allow(clippy::unwrap_used)]
use super::*;
fn key() -> MethodKey {
    MethodKey {
        owner: "Landroid/widget/EditText;".into(),
        name: "getText".into(),
        parameters: vec![],
        return_type: "Ljava/lang/CharSequence;".into(),
    }
}
fn code() -> dex_format::code::CodeItem {
    dex_format::code::CodeItem {
        registers_size: 1,
        ins_size: 1,
        outs_size: 1,
        debug_info_off: 0,
        insns_size: 5,
        insns: [0x106eu16, 8768, 0, 0x000c, 0x0011]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect(),
        tries: vec![],
        handlers: vec![],
        span: dex_format::RawSpan { offset: 0, size: 0 },
    }
}
#[test]
fn exact_forwarding_and_negative_mutations() {
    assert_eq!(dex_bridge_target(&code(), &key(), 0x1041), Some(8768));
    for flags in [1, 0x1049, 0x1043, 0x1141, 0x1441, 0x1061] {
        assert_eq!(dex_bridge_target(&code(), &key(), flags), None);
    }
    for (offset, value) in [(4, 1u8), (6, 0), (8, 0), (9, 1), (0, 0x6f)] {
        let mut c = code();
        c.insns[offset] = value;
        assert_eq!(dex_bridge_target(&c, &key(), 0x1041), None);
    }
    let mut c = code();
    c.insns.extend([0, 0]);
    c.insns_size += 1;
    assert_eq!(dex_bridge_target(&c, &key(), 0x1041), None);
    let mut c = code();
    c.ins_size = 0;
    assert_eq!(dex_bridge_target(&c, &key(), 0x1041), None);
    let mut c = code();
    c.tries.push(dex_format::code::TryItem {
        start_addr: 0,
        insn_count: 3,
        handler_off: 0,
        span: dex_format::RawSpan { offset: 0, size: 0 },
    });
    assert_eq!(dex_bridge_target(&c, &key(), 0x1041), None);
}
#[test]
#[ignore = "requires genuine matching Samsung framework metadata archive"]
fn actual_matching_edittext_bridge() {
    let metadata = DexMetadata::from_path(std::path::Path::new(
        "/tmp/opencode/samsung-framework-metadata/framework.jar",
    ))
    .unwrap();
    let bridge = key();
    let mut target = bridge.clone();
    target.return_type = "Landroid/text/Editable;".into();
    assert!(metadata.attests_covariant_bridge(&bridge, &target));
    let mut foreign = target.clone();
    foreign.owner = "Landroid/widget/TextView;".into();
    assert!(!metadata.attests_covariant_bridge(&bridge, &foreign));
    assert!(!metadata.attests_covariant_bridge(&target, &bridge));
}

#[test]
#[ignore = "requires genuine matching Samsung framework metadata archive"]
fn genuine_definition_mutations_never_attest() {
    let bytes = std::fs::read("/tmp/opencode/samsung-framework-metadata/framework.jar").unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name("classes5.dex")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    let mut dex = DexFile::from_container(&bytes).unwrap().remove(0);
    let classes = dex
        .classes
        .iter()
        .filter(|c| descriptor(&dex, c.class_idx).ok() == Some("Landroid/widget/EditText;"))
        .cloned()
        .collect();
    dex.classes = classes;
    let bridge = key();
    let mut target = bridge.clone();
    target.return_type = "Landroid/text/Editable;".into();
    assert!(
        DexMetadata::from_dex_files(&[dex.clone()])
            .unwrap()
            .attests_covariant_bridge(&bridge, &target)
    );
    for mutation in 0..7 {
        let mut changed = dex.clone();
        let data = changed.classes[0].class_data.as_mut().unwrap();
        let encoded = data
            .virtual_methods
            .iter_mut()
            .find(|m| m.method_idx == 8769)
            .unwrap();
        let code = encoded.code.as_mut().unwrap();
        match mutation {
            0 => code.insns[4] = 1, // foreign receiver
            1 => code.insns[2..4].copy_from_slice(&8769u16.to_le_bytes()), // self bridge
            2 => code.insns[8] = 0x0e, // does not return the exact result
            3 => encoded.access_flags = 1, // flags cannot manufacture proof
            4 => code.tries.push(dex_format::code::TryItem {
                start_addr: 0,
                insn_count: 3,
                handler_off: 0,
                span: dex_format::RawSpan { offset: 0, size: 0 },
            }),
            5 => {
                code.insns.extend([0, 0]);
                code.insns_size += 1;
            }
            6 => data.virtual_methods.retain(|m| m.method_idx != 8768), // pool ref is not definition
            _ => unreachable!(),
        }
        assert!(
            !DexMetadata::from_dex_files(&[changed])
                .is_ok_and(|m| m.attests_covariant_bridge(&bridge, &target)),
            "mutation {mutation}"
        );
    }
}

#[test]
fn incoming_wide_words_and_range_are_exact() {
    let mut signature = key();
    signature.parameters = vec!["J".into(), "I".into()];
    let mut c = code();
    c.registers_size = 4;
    c.ins_size = 4;
    c.outs_size = 4;
    c.insns = [0x406eu16, 8768, 0x3210, 0x000c, 0x0011]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert_eq!(dex_bridge_target(&c, &signature, 0x1041), Some(8768));
    c.insns[4..6].copy_from_slice(&0x3120u16.to_le_bytes());
    assert_eq!(dex_bridge_target(&c, &signature, 0x1041), None);
    c.insns = [0x0474u16, 8768, 0, 0x000c, 0x0011]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert_eq!(dex_bridge_target(&c, &signature, 0x1041), Some(8768));
    c.insns[4] = 1;
    assert_eq!(dex_bridge_target(&c, &signature, 0x1041), None);
}
