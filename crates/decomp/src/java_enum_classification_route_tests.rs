//! Genuine subclass routing must fail at the parent integration boundary.
#![allow(clippy::unwrap_used)]
use super::*;
#[test]
fn original_enum_child_does_not_demand_or_borrow_an_initializer() {
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let aliases = inventory(&inputs, &mut None).unwrap();
    let dex = &inputs[0];
    let child = dex
        .classes
        .iter()
        .find(|c| {
            descriptor(dex, c.class_idx) == Some("Landroidx/datastore/preferences/protobuf/r1$b$a;")
        })
        .unwrap();
    let metadata = dex_decompiler::infer::DexMetadata::new(dex);
    let mut resolver = Resolver::new(dex, &aliases, &metadata);
    resolver.inputs = &inputs;
    let Err(reason) = enum_source(child, &resolver, &mut None) else {
        panic!("parent/child integration has not been proven")
    };
    assert!(reason.starts_with("enum constant body requires original parent integration:"));
    assert!(reason.contains("child=Landroidx/datastore/preferences/protobuf/r1$b$a;"));
    assert!(reason.contains("parent=Landroidx/datastore/preferences/protobuf/r1$b;"));
    assert!(reason.contains("methods=[12861]"));
    assert!(!reason.contains("missing initializer"));
}
