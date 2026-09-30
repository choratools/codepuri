//! Handcrafted model and malformed-input tests.

mod common;

use dex_format::map::MapEntry;
use dex_format::{DexError, DexFile};

#[test]
fn parses_handcrafted_one_class_dex() -> Result<(), DexError> {
    let bytes = common::minimal_dex();

    let dex = DexFile::from_bytes(&bytes)?;

    assert_eq!(dex.classes.len(), 1);
    assert_eq!(dex.string(0)?, "Lx;");
    assert_eq!(dex.header.version, 35);
    Ok(())
}

#[test]
fn truncations_and_single_byte_mutations_never_panic() {
    let fixture = common::minimal_dex();

    for end in 0..fixture.len() {
        let _ = DexFile::from_bytes(&fixture[..end]);
    }
    for index in 0..fixture.len() {
        let mut mutated = fixture.clone();
        mutated[index] ^= 0xff;
        let _ = DexFile::from_bytes(&mutated);
    }
}

#[test]
fn unsupported_version_is_typed() {
    let mut bytes = common::minimal_dex();
    bytes[4..7].copy_from_slice(b"042");

    assert!(matches!(
        DexFile::from_bytes(&bytes),
        Err(DexError::UnsupportedVersion(42))
    ));
}

#[test]
fn parses_single_file_v41_container() -> Result<(), DexError> {
    let bytes = common::minimal_dex_v41();

    let files = DexFile::from_container(&bytes)?;

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].header.header_size, 120);
    assert_eq!(
        files[0].header.container_size,
        u32::try_from(bytes.len()).unwrap_or(0)
    );
    Ok(())
}

#[test]
fn preserves_unknown_map_entry() -> Result<(), DexError> {
    let mut bytes = common::minimal_dex();
    bytes[216..218].copy_from_slice(&0x7777_u16.to_le_bytes());

    let dex = DexFile::from_bytes(&bytes)?;

    assert!(dex.map.entries.iter().any(|entry| matches!(
        entry,
        MapEntry::Unknown {
            map_type: 0x7777,
            size: 1,
            ..
        }
    )));
    Ok(())
}
