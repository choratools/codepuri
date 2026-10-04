//! Complete payload traces forbid expression duplication and reordering.
#![allow(clippy::unwrap_used)]
use super::*;
#[test]
fn payload_trace_requires_exact_once_original_order() {
    let mut arena = managed_ir::method::InsnArena::default();
    let a = arena.insert(ManagedInsn::Return { value: None });
    let b = arena.insert(ManagedInsn::Return { value: None });
    let expression = |effects: Vec<InsnId>| Value::ReferenceExpression {
        source: "opaque_for_trace_test".into(),
        ty: JavaType::Class("java.lang.Object".into()),
        effects,
    };
    let first = expression(vec![a]);
    let second = expression(vec![b]);
    let nested = expression(vec![a, b]);
    assert!(payload_effects_match([&first, &second], &[a, b]));
    assert!(payload_effects_match([&nested], &[a, b]));
    assert!(!payload_effects_match([&second, &first], &[a, b]));
    assert!(!payload_effects_match([&first, &first], &[a]));
    assert!(!payload_effects_match([&first], &[a, b]));
    assert!(!payload_effects_match([&first], &[]));
    assert!(!payload_effects_match([&Value::Int(0)], &[a]));
}
#[test]
fn payload_bit_constants_preserve_negative_zero_and_finite_bits() {
    assert_eq!(float_source(0).unwrap(), "0e0f");
    assert_eq!(float_source(0x8000_0000).unwrap(), "-0e0f");
    assert_eq!(double_source(0x8000_0000_0000_0000).unwrap(), "-0e0d");
    assert!(float_source(0x7fc0_0011).is_err());
    assert!(double_source(0x7ff8_0000_0000_0011).is_err());
    for bits in [
        1,
        0x0080_0000,
        0x3f80_0000,
        0x3eaa_aaab,
        0x7f7f_ffff,
        0x8080_0000,
    ] {
        let source = float_source(bits).unwrap();
        assert_eq!(
            source
                .strip_suffix('f')
                .unwrap()
                .parse::<f32>()
                .unwrap()
                .to_bits(),
            bits
        );
    }
}
