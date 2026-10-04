#![allow(clippy::unwrap_used)]
use super::*;

#[test]
fn indexed_membership_preserves_full_metadata_and_exhaustive_fallback() {
    let dex = DexFile::from_bytes(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/dex/clean/classes.dex"
    )))
    .unwrap();
    assert!(!dex.classes.is_empty());
    let mut variants = vec![dex.classes.clone()];
    let mut reversed = dex.classes.clone();
    reversed.reverse();
    variants.push(reversed);
    let mut duplicated = dex.classes.clone();
    let mut changed = duplicated[0].clone();
    changed.access_flags ^= 0x10;
    duplicated.insert(0, changed.clone());
    variants.push(duplicated);
    for classes in variants {
        for class in dex.classes.iter().chain(std::iter::once(&changed)) {
            assert_eq!(
                contains_original_class(&classes, class),
                classes.iter().any(|original| original == class)
            );
        }
    }
    assert!(!contains_original_class(&[], &dex.classes[0]));
    assert!(!contains_original_class(&dex.classes, &changed));
}
