//! Genuine Java17 Object Code preservation and attestation boundaries.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use decomp::java_external_metadata::{ClassMetadata, Classpath};
use std::{io::Read, path::PathBuf};
fn jmod() -> PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11"))
        .join("jmods/java.base.jmod")
}
fn object_bytes() -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(jmod()).unwrap()).unwrap();
    let mut bytes = Vec::new();
    archive
        .by_name("classes/java/lang/Object.class")
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}
fn constructor_span(bytes: &[u8]) -> (usize, usize) {
    let class = ClassMetadata::parse(bytes).unwrap();
    let proof = class
        .object_return_only_evidence()
        .expect("genuine local Java17 root");
    let code = proof.constructor().code.as_ref().unwrap();
    (code.attribute_offset, code.attribute_length)
}
fn put_u4(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}
#[test]
fn explicit_jdk17_jmod_preserves_actual_root_code() {
    let bytes = object_bytes();
    let parsed = ClassMetadata::parse(&bytes).unwrap();
    let proof = parsed.object_return_only_evidence().unwrap();
    assert_eq!(proof.class().descriptor, "Ljava/lang/Object;");
    for method in [proof.constructor(), proof.finalizer()] {
        let code = method.code.as_ref().unwrap();
        assert_eq!(code.bytes, [0xb1]);
        assert_eq!((code.max_stack, code.max_locals), (0, 1));
        assert!(code.exceptions.is_empty());
        assert_eq!(
            &bytes[code.attribute_offset + 8..code.attribute_offset + 9],
            [0xb1]
        );
    }
    let cp = Classpath::from_paths([jmod()]).unwrap();
    assert!(
        cp.class("Ljava/lang/Object;")
            .unwrap()
            .object_return_only_evidence()
            .is_some()
    );
}
#[test]
fn changed_code_stack_locals_owner_and_version_do_not_attest() {
    let original = object_bytes();
    let (base, _) = constructor_span(&original);
    for (offset, value) in [(base + 8, 0xbf), (base + 1, 1), (base + 3, 2), (7, 62)] {
        let mut changed = original.clone();
        changed[offset] = value;
        assert!(
            ClassMetadata::parse(&changed)
                .unwrap()
                .object_return_only_evidence()
                .is_none()
        );
    }
    let mut changed = original.clone();
    changed[base + 8] = 0xbf;
    let mut forged = ClassMetadata::parse(&changed).unwrap();
    let forged_code = forged
        .methods
        .iter_mut()
        .find(|m| m.key.name == "<init>")
        .unwrap()
        .code
        .as_mut()
        .unwrap();
    std::sync::Arc::make_mut(forged_code).bytes = vec![0xb1];
    assert!(
        forged.object_return_only_evidence().is_none(),
        "public metadata changes cannot create byte-derived origin"
    );
    let mut class = ClassMetadata::parse(&original).unwrap();
    class.descriptor = "Lexample/Object;".into();
    assert!(class.object_return_only_evidence().is_none());
}
#[test]
fn malformed_duplicate_code_and_native_abstract_access_reject() {
    let original = object_bytes();
    let (base, len) = constructor_span(&original);
    for size in [0, 65_536, u32::MAX] {
        let mut changed = original.clone();
        put_u4(&mut changed, base + 4, size);
        assert!(ClassMetadata::parse(&changed).is_err());
    }
    // Actual Object constructor has Code first and a following annotation attribute.
    let count = u16::from_be_bytes(original[base - 8..base - 6].try_into().unwrap());
    assert!(count >= 1);
    assert_eq!(&original[base - 14..base - 12], [0, 1]);
    let attribute = original[base - 6..base + len].to_vec();
    let mut duplicate = original.clone();
    duplicate[base - 8..base - 6].copy_from_slice(&(count + 1).to_be_bytes());
    duplicate.splice(base + len..base + len, attribute);
    assert!(ClassMetadata::parse(&duplicate).is_err());
    for flags in [0x101u16, 0x401] {
        let mut changed = original.clone();
        changed[base - 14..base - 12].copy_from_slice(&flags.to_be_bytes());
        assert!(ClassMetadata::parse(&changed).is_err());
    }
    let mut truncated = original.clone();
    truncated.truncate(base + 8);
    assert!(ClassMetadata::parse(&truncated).is_err());
}
#[test]
fn exact_exception_table_is_preserved_and_prevents_root_attestation() {
    let original = object_bytes();
    let (base, len) = constructor_span(&original);
    // One-byte original Code, then u2 exception_count, then nested attrs.
    assert_eq!(&original[base + 9..base + 11], [0, 0]);
    let mut changed = original.clone();
    changed[base + 10] = 1;
    changed.splice(base + 11..base + 11, [0, 0, 0, 1, 0, 0, 0, 0]);
    put_u4(&mut changed, base - 4, (len + 8) as u32);
    let parsed = ClassMetadata::parse(&changed).unwrap();
    assert!(parsed.object_return_only_evidence().is_none());
    let method = parsed
        .methods
        .iter()
        .find(|m| m.key.name == "<init>")
        .unwrap();
    let entry = &method.code.as_ref().unwrap().exceptions[0];
    assert_eq!((entry.start_pc, entry.end_pc, entry.handler_pc), (0, 1, 0));
    assert!(entry.catch_type.is_none());
    changed[base + 14] = 2; // end_pc outside code length
    assert!(ClassMetadata::parse(&changed).is_err());
}
