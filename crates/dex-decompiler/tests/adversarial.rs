//! Regression coverage for hostile raw Dalvik instruction fixtures.

use std::fs;
use std::path::Path;

use dex_decompiler::{DecodeReason, decode_method};
use dex_format::{RawSpan, code::CodeItem};

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

fn fixture(name: &str) -> CodeItem {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/dex/adversarial/opcode_invariant")
        .join(name);
    let insns = must(fs::read(path));
    CodeItem {
        registers_size: u16::MAX,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: must(u32::try_from(insns.len() / 2)),
        insns,
        tries: Vec::new(),
        handlers: Vec::new(),
        span: RawSpan { offset: 0, size: 0 },
    }
}

#[test]
fn branch_out_of_bounds_fixture_returns_typed_error() {
    let error = must_err(decode_method(&fixture("m1_goto32_target_oob.bin")));

    assert!(matches!(
        error.reason,
        DecodeReason::BranchTargetOutOfRange { .. }
    ));
}

#[test]
fn unknown_opcode_fixture_returns_typed_error() {
    let error = must_err(decode_method(&fixture(
        "unchecked1_unknown_opcode_0x73.bin",
    )));

    assert!(matches!(
        error.reason,
        DecodeReason::UnsupportedOpcode { opcode: 0x73 }
    ));
}

#[test]
fn packed_switch_case_target_out_of_bounds_returns_typed_error() {
    let error = must_err(decode_method(&fixture(
        "m1_packed_switch_case_target_oob.bin",
    )));

    assert!(matches!(
        error.reason,
        DecodeReason::BranchTargetOutOfRange { .. }
    ));
}

#[test]
fn bad_payload_ident_returns_typed_error() {
    let error = must_err(decode_method(&fixture(
        "m2_packed_switch_ident_mismatch.bin",
    )));

    assert!(matches!(error.reason, DecodeReason::BadPayloadIdent { .. }));
}
