//! Indexed declaration closure retains the legacy evidence and rejection rules.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use decomp::java_members::{MemberIndex, MemberRequirements, MemberResolver, MethodBodyStatus};
use dex_format::DexFile;
use std::collections::BTreeSet;

fn fixture() -> DexFile {
    DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap()
}

fn selected_method(dex: &DexFile) -> (usize, usize, u32) {
    dex.classes
        .iter()
        .enumerate()
        .find_map(|(ci, class)| {
            class
                .class_data
                .as_ref()?
                .direct_methods
                .iter()
                .enumerate()
                .find_map(|(mi, method)| {
                    let name = dex
                        .string(dex.method_id(method.method_idx).ok()?.name_idx)
                        .ok()?;
                    (!name.starts_with('<') && method.code.is_some()).then_some((
                        ci,
                        mi,
                        method.method_idx,
                    ))
                })
        })
        .expect("fixture has a non-constructor source method")
}

fn assert_parity(dex: &DexFile, method: u32) {
    let legacy = MemberResolver { dex };
    let index = MemberIndex::new(dex);
    assert_eq!(
        index.method_body_status(method),
        legacy.method_body_status(method)
    );
    assert_eq!(
        index.bodyless_method_declaration(method),
        legacy.bodyless_method_declaration(method)
    );
    let requirements = MemberRequirements {
        methods: BTreeSet::from([method]),
        ..Default::default()
    };
    for emitted in [BTreeSet::new(), BTreeSet::from([method])] {
        for bodyless in [BTreeSet::new(), BTreeSet::from([method])] {
            assert_eq!(
                requirements.method_closure_indexed(&index, &emitted, &bodyless),
                requirements.method_closure(&legacy, &emitted, &bodyless)
            );
        }
    }
}

#[test]
fn indexed_pool_and_emitted_closure_match_legacy() {
    let dex = fixture();
    for method in 0..dex.method_ids.len() as u32 {
        assert_parity(&dex, method);
    }
    assert_parity(&dex, u32::MAX);

    // The immutable per-DEX index can be borrowed by concurrent class workers.
    fn assert_sync<T: Sync>() {}
    assert_sync::<MemberIndex<'_>>();
}

#[test]
fn indexed_flags_and_body_metadata_retain_rejections() {
    let original = fixture();
    let (ci, mi, method) = selected_method(&original);
    for class_flags in [1, 0x401, 0x411, 0x201] {
        for flags in [
            1, 9, 0x101, 0x109, 0x103, 0x141, 0x121, 0x20101, 0x100101, 0x401, 0x402, 0x404, 0x409,
            0x411, 0x421, 0x20401, 0x501,
        ] {
            for retain_code in [true, false] {
                let mut dex = original.clone();
                dex.classes[ci].access_flags = class_flags;
                let encoded = &mut dex.classes[ci].class_data.as_mut().unwrap().direct_methods[mi];
                encoded.access_flags = flags;
                if !retain_code {
                    encoded.code = None;
                }
                assert_parity(&dex, method);
            }
        }
    }

    let mut dex = original.clone();
    let encoded = &mut dex.classes[ci].class_data.as_mut().unwrap().direct_methods[mi];
    encoded.access_flags = 0x109;
    encoded.code = None;
    let index = MemberIndex::new(&dex);
    let requirements = MemberRequirements {
        methods: BTreeSet::from([method]),
        ..Default::default()
    };
    assert_eq!(
        index.method_body_status(method),
        Ok(MethodBodyStatus::Native)
    );
    assert!(
        index
            .bodyless_method_declaration(method)
            .unwrap()
            .contains("native ")
    );
    assert!(
        requirements
            .method_closure_indexed(&index, &BTreeSet::from([method]), &BTreeSet::new())
            .unmet
            .contains_key(&method),
        "a source-body set does not prove native declaration publication"
    );
}

#[test]
fn indexed_definitions_use_first_owner_and_reject_member_duplicates() {
    let original = fixture();
    let (ci, mi, method) = selected_method(&original);
    let mut dex = original.clone();
    let duplicate = dex.classes[ci].class_data.as_ref().unwrap().direct_methods[mi].clone();
    dex.classes[ci]
        .class_data
        .as_mut()
        .unwrap()
        .virtual_methods
        .push(duplicate.clone());
    assert_parity(&dex, method);
    assert_eq!(
        MemberIndex::new(&dex)
            .method_body_status(method)
            .unwrap_err(),
        "missing target definition"
    );

    // A later class for the same owner cannot fill holes in the first class.
    let mut dex = original.clone();
    let valid_class = dex.classes[ci].clone();
    dex.classes[ci].class_data = None;
    dex.classes.push(valid_class);
    assert_parity(&dex, method);
    assert!(MemberIndex::new(&dex).method_body_status(method).is_err());

    // Conversely, duplicates or inconsistent bodies in a later owner are ignored.
    let mut dex = original.clone();
    let mut later = dex.classes[ci].clone();
    later
        .class_data
        .as_mut()
        .unwrap()
        .virtual_methods
        .push(duplicate.clone());
    later.class_data.as_mut().unwrap().direct_methods[mi].access_flags = 0x501;
    dex.classes.push(later);
    assert_parity(&dex, method);
    assert_eq!(
        MemberIndex::new(&dex).method_body_status(method),
        Ok(MethodBodyStatus::SourceBody)
    );

    // A definition in another class cannot satisfy the method-id's actual owner.
    let mut dex = original.clone();
    let mut misplaced = dex.classes[ci].clone();
    misplaced.class_idx = u32::MAX;
    dex.classes[ci]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .remove(mi);
    dex.classes.insert(0, misplaced);
    assert_parity(&dex, method);
    assert!(MemberIndex::new(&dex).method_body_status(method).is_err());
}

#[test]
fn indexed_bodyless_signatures_validate_pool_metadata() {
    let mut original = fixture();
    let (ci, mi, method) = selected_method(&original);
    original.classes[ci]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods[mi]
        .access_flags = 0x109;
    original.classes[ci]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods[mi]
        .code = None;
    assert_parity(&original, method);

    let constructor_name = original
        .method_ids
        .iter()
        .find(|id| original.string(id.name_idx).unwrap() == "<init>")
        .unwrap()
        .name_idx;
    for name in [constructor_name, u32::MAX] {
        let mut dex = original.clone();
        dex.method_ids[method as usize].name_idx = name;
        assert_parity(&dex, method);
        assert!(
            MemberIndex::new(&dex)
                .bodyless_method_declaration(method)
                .is_err()
        );
    }
    let mut dex = original.clone();
    dex.method_ids[method as usize].proto_idx = u16::MAX;
    assert_parity(&dex, method);
    assert!(
        MemberIndex::new(&dex)
            .bodyless_method_declaration(method)
            .is_err()
    );

    // An unrelated earlier malformed definition must not alter rendering from
    // the real owner, even though the legacy raw resolver sees its static flag.
    let mut dex = original.clone();
    let mut misplaced = dex.classes[ci].clone();
    misplaced.class_idx = u32::MAX;
    misplaced.class_data.as_mut().unwrap().direct_methods[mi].access_flags = 0x101;
    dex.classes.insert(0, misplaced);
    assert_parity(&dex, method);
}
