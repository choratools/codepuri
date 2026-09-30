//! Codec unit and property tests.

use dex_format::uleb::{
    decode_mutf8, encode_mutf8, encode_uleb128, encode_uleb128_u64, read_sleb128, read_uleb128,
    read_uleb128_u64,
};
use proptest::prelude::*;

#[test]
fn mutf8_roundtrips_encoded_nul_and_surrogate_pair() -> Result<(), Box<dyn std::error::Error>> {
    let text = "a\0😀z";
    let encoded = encode_mutf8(text);

    let decoded = decode_mutf8(&encoded, 0, u32::try_from(text.encode_utf16().count())?)?;

    assert_eq!(decoded.0, text);
    assert_eq!(&encoded[1..3], &[0xc0, 0x80]);
    Ok(())
}

#[test]
fn malformed_uleb_reports_an_error() {
    let result = read_uleb128(&[0x80; 6], 0);

    assert!(result.is_err());
}

#[test]
fn sleb_decodes_signed_boundaries() -> Result<(), dex_format::DexError> {
    assert_eq!(read_sleb128(&[0x00], 0)?, (0, 1));
    assert_eq!(read_sleb128(&[0x7f], 0)?, (-1, 1));
    assert_eq!(
        read_sleb128(&[0x80, 0x80, 0x80, 0x80, 0x78], 0)?,
        (i32::MIN, 5)
    );
    assert_eq!(
        read_sleb128(&[0xff, 0xff, 0xff, 0xff, 0x07], 0)?,
        (i32::MAX, 5)
    );
    Ok(())
}

proptest! {
    #[test]
    fn uleb32_roundtrips(value in any::<u32>()) {
        let encoded = encode_uleb128(value);
        let decoded = read_uleb128(&encoded, 0)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(decoded, (value, encoded.len()));
    }

    #[test]
    fn uleb64_roundtrips(value in any::<u64>()) {
        let encoded = encode_uleb128_u64(value);
        let decoded = read_uleb128_u64(&encoded, 0)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert_eq!(decoded, (value, encoded.len()));
    }
}
