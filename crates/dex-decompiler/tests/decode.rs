//! Decoder behavior across operand families and validation boundaries.

use dex_decompiler::{DecodeReason, Format, Opcode, decode_method, opcode};
use dex_format::{RawSpan, code::CodeItem};
use proptest::prelude::*;

fn must<T, E: std::fmt::Debug>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("unexpected error: {error:?}"),
    }
}

fn must_err<T: std::fmt::Debug, E>(result: Result<T, E>) -> E {
    match result {
        Ok(value) => panic!("unexpected success: {value:?}"),
        Err(error) => error,
    }
}

fn code(registers_size: u16, units: &[u16]) -> CodeItem {
    let insns = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    CodeItem {
        registers_size,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: must(u32::try_from(units.len())),
        insns,
        tries: Vec::new(),
        handlers: Vec::new(),
        span: RawSpan { offset: 0, size: 0 },
    }
}

fn must_decode(item: &CodeItem) -> Vec<dex_decompiler::DecodedInsn> {
    must(decode_method(item))
}

#[test]
fn decodes_branch_22t_when_target_is_in_bounds() {
    let decoded = must_decode(&code(2, &[0x1032, 2, 0x000e]));

    assert_eq!(decoded[0].format(), Format::F22t);
    assert_eq!(decoded[0].registers(), &[0, 1]);
    assert_eq!(decoded[0].branch_offset(), Some(2));
}

#[test]
fn decodes_invoke_35c_registers_in_operand_order() {
    let decoded = must_decode(&code(4, &[0x3071, 0x0042, 0x0321]));

    assert_eq!(decoded[0].format(), Format::F35c);
    assert_eq!(decoded[0].registers(), &[1, 2, 3]);
    assert_eq!(decoded[0].pool_index(), Some(0x42));
}

#[test]
fn decodes_const_31i_signed_literal() {
    let decoded = must_decode(&code(2, &[0x0114, 0x5678, 0x9234]));

    assert_eq!(decoded[0].format(), Format::F31i);
    assert_eq!(decoded[0].registers(), &[1]);
    assert_eq!(decoded[0].literal(), Some(i64::from(-1_842_063_752_i32)));
}

#[test]
fn every_fixed_format_has_the_specified_code_unit_width() {
    let cases = [
        (Format::F10x, 1),
        (Format::F12x, 1),
        (Format::F11n, 1),
        (Format::F11x, 1),
        (Format::F10t, 1),
        (Format::F20t, 2),
        (Format::F22x, 2),
        (Format::F21t, 2),
        (Format::F21s, 2),
        (Format::F21h, 2),
        (Format::F21c, 2),
        (Format::F23x, 2),
        (Format::F22b, 2),
        (Format::F22t, 2),
        (Format::F22s, 2),
        (Format::F22c, 2),
        (Format::F30t, 3),
        (Format::F31i, 3),
        (Format::F31t, 3),
        (Format::F31c, 3),
        (Format::F32x, 3),
        (Format::F35c, 3),
        (Format::F3rc, 3),
        (Format::F45cc, 4),
        (Format::F4rcc, 4),
        (Format::F51l, 5),
    ];

    for (format, width) in cases {
        assert_eq!(format.width(), Some(width));
    }
}

#[test]
fn payload_formats_are_variable_width() {
    for format in [
        Format::PackedSwitchPayload,
        Format::SparseSwitchPayload,
        Format::ArrayDataPayload,
    ] {
        assert_eq!(format.width(), None);
    }
}

#[test]
fn opcode_table_maps_all_reserved_values_explicitly() {
    let unsupported: Vec<u8> = (u8::MIN..=u8::MAX)
        .filter(|&value| matches!(opcode(value), Opcode::Unsupported { opcode: _ }))
        .collect();
    let expected: Vec<u8> = (0x3e..=0x43)
        .chain([0x73])
        .chain(0x79..=0x7a)
        .chain(0xe3..=0xf9)
        .collect();

    assert_eq!(unsupported, expected);
}

#[test]
fn nonzero_unused_nop_bits_do_not_imply_a_payload() {
    let decoded = must_decode(&code(0, &[0x0400]));

    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].format(), Format::F10x);
}

#[test]
fn decodes_signed_nibbles_high_literals_and_byte_literals() {
    let const_four = must_decode(&code(1, &[0xf012]));
    let high_short = must_decode(&code(1, &[0x0015, 0xffff]));
    let literal_byte = must_decode(&code(2, &[0x01d8, 0xff01]));

    assert_eq!(const_four[0].literal(), Some(-1));
    assert_eq!(high_short[0].literal(), Some(-65_536));
    assert_eq!(literal_byte[0].registers(), &[1, 1]);
    assert_eq!(literal_byte[0].literal(), Some(-1));
}

#[test]
fn decodes_range_and_wide_literal_formats() {
    let range = must_decode(&code(8, &[0x0374, 9, 4]));
    let wide = must_decode(&code(1, &[0x0018, 0x7788, 0x5566, 0x3344, 0x1122]));

    assert_eq!(range[0].registers(), &[4, 5, 6]);
    assert_eq!(range[0].pool_index(), Some(9));
    assert_eq!(wide[0].literal(), Some(0x1122_3344_5566_7788));
}

#[test]
fn parses_sparse_switch_and_array_data_payloads() {
    let sparse = [0x002c, 4, 0, 0, 0x0200, 1, 9, 0, 3, 0];
    let array = [0x0026, 4, 0, 0, 0x0300, 1, 3, 0, 0x0201, 0x0003];

    assert_eq!(must_decode(&code(1, &sparse))[0].payload_offset(), Some(4));
    assert_eq!(must_decode(&code(1, &array))[0].payload_offset(), Some(4));
}

#[test]
fn rejects_unaligned_and_invalid_width_payloads() {
    let unaligned = code(0, &[0x0000, 0x0100, 0, 0, 0]);
    let invalid_width = code(1, &[0x0026, 4, 0, 0, 0x0300, 3, 1, 0, 0]);

    let unaligned_error = must_err(decode_method(&unaligned));
    let width_error = must_err(decode_method(&invalid_width));
    assert!(matches!(
        unaligned_error.reason,
        DecodeReason::MalformedPayload { .. }
    ));
    assert!(matches!(
        width_error.reason,
        DecodeReason::MalformedPayload { .. }
    ));
}

#[test]
fn decodes_invoke_polymorphic_45cc_operands() {
    let decoded = must_decode(&code(3, &[0x20fa, 3, 0x0021, 4]));

    assert_eq!(decoded[0].format(), Format::F45cc);
    assert_eq!(decoded[0].registers(), &[1, 2]);
    assert_eq!(decoded[0].pool_index(), Some(3));
    assert_eq!(decoded[0].secondary_pool_index(), Some(4));
}

#[test]
fn accepts_branch_targets_at_current_and_last_code_unit() {
    let current = must_decode(&code(0, &[0x0028]));
    let last = must_decode(&code(0, &[0x0128, 0x000e]));

    assert_eq!(current[0].branch_offset(), Some(0));
    assert_eq!(last[0].branch_offset(), Some(1));
}

#[test]
fn rejects_register_equal_to_registers_size() {
    let error = must_err(decode_method(&code(1, &[0x1112])));

    assert!(matches!(
        error.reason,
        DecodeReason::RegisterOutOfRange {
            register: 1,
            registers_size: 1
        }
    ));
}

#[test]
fn parses_and_validates_packed_switch_payload() {
    let units = [0x002b, 4, 0, 0, 0x0100, 1, 7, 0, 3, 0];
    let decoded = must_decode(&code(1, &units));

    assert_eq!(decoded[0].payload_offset(), Some(4));
    assert!(decoded.iter().any(|insn| insn.offset() == 4));
}

proptest! {
    #[test]
    fn arbitrary_truncation_never_panics(cut in 0usize..11) {
        let bytes = [0x18, 0x00, 1, 2, 3, 4, 5, 6, 7, 8];
        let item = CodeItem {
            registers_size: 1,
            ins_size: 0,
            outs_size: 0,
            debug_info_off: 0,
            insns_size: must(u32::try_from(cut / 2)),
            insns: bytes[..cut.min(bytes.len())].to_vec(),
            tries: Vec::new(),
            handlers: Vec::new(),
            span: RawSpan { offset: 0, size: 0 },
        };

        let _ = decode_method(&item);
    }
}
