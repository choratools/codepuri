//! jadx-style per-instruction recovery: budgeted failures abort the walk,
//! while register-range and payload noise are recorded without aborting.

use dex_decompiler::{DecodeReason, decode_method_recovering};
use dex_format::RawSpan;
use dex_format::code::CodeItem;

fn code_item(registers_size: u16, insns: &[u8]) -> CodeItem {
    CodeItem {
        registers_size,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: u32::try_from(insns.len() / 2).unwrap_or(0),
        insns: insns.to_vec(),
        tries: Vec::new(),
        handlers: Vec::new(),
        span: RawSpan { offset: 0, size: 0 },
    }
}

/// Test_Decoding_clean_stream_when_no_failures_exist
#[test]
fn clean_stream_has_no_diagnostics() {
    // const/4 v0, 1 ; return-void
    let item = code_item(1, &[0x12, 0x10, 0x0e, 0x00]);
    let recovered = decode_method_recovering(&item, 2);
    assert!(recovered.is_clean());
    assert!(!recovered.aborted);
    assert_eq!(recovered.insns.len(), 2);
    assert!(recovered.errors.is_empty());
}

/// Test_Recording_noise_without_aborting_when_fake_payload_ident_appears
#[test]
fn garbage_payload_yields_gap_and_continues() {
    // const/4 v0, 1 ; fake array-data ident (noise) ; 0xff junk ; return-void
    let item = code_item(4, &[0x12, 0x10, 0x00, 0x01, 0xff, 0xff, 0x0e, 0x00]);
    let recovered = decode_method_recovering(&item, 2);
    assert!(!recovered.aborted);
    // payload-ident noise and register-range noise are recorded but never
    // budgeted, and 0xff (const-method-type) is a supported opcode whose
    // junk operand merely violates the register bound.
    assert!(!recovered.errors.is_empty());
    assert!(recovered.insns.len() >= 2);
}

/// Test_Aborting_when_budgeted_errors_exhaust_the_limit
#[test]
fn error_limit_aborts_walk() {
    // three consecutive reserved opcodes with budget 2
    let item = code_item(1, &[0x73, 0x00, 0x79, 0x00, 0x7a, 0x00]);
    let recovered = decode_method_recovering(&item, 2);
    assert!(recovered.aborted);
    assert_eq!(recovered.budget_used, 2);
    assert_eq!(recovered.errors.len(), 2);
    assert!(recovered.insns.is_empty());
}

/// Test_Continuing_past_one_budgeted_error_within_limit
#[test]
fn unsupported_opcode_is_diagnostic_not_abort() {
    // const/4 v0, 1 ; 0x73 (reserved) ; return-void
    let item = code_item(1, &[0x12, 0x10, 0x73, 0x00, 0x0e, 0x00]);
    let recovered = decode_method_recovering(&item, 2);
    assert!(!recovered.aborted);
    assert_eq!(recovered.budget_used, 1);
    assert_eq!(
        recovered.errors[0].reason,
        DecodeReason::UnsupportedOpcode { opcode: 0x73 }
    );
    assert_eq!(recovered.insns.len(), 2);
}

/// Test_Keeping_insn_when_register_index_exceeds_registers_size
#[test]
fn register_oob_keeps_insn_jadx_parity() {
    // const/16 v255, 0 ; return-void  (registers_size = 4)
    let item = code_item(4, &[0x13, 0xff, 0x00, 0x00, 0x0e, 0x00]);
    let recovered = decode_method_recovering(&item, 2);
    assert!(!recovered.aborted);
    assert_eq!(recovered.budget_used, 0);
    assert_eq!(
        recovered.errors[0].reason,
        DecodeReason::RegisterOutOfRange {
            register: 255,
            registers_size: 4,
        }
    );
    // both instructions survive — register bounds do not affect Java semantics
    assert_eq!(recovered.insns.len(), 2);
}
