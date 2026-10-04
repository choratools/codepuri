//! Incoming declaration seeds must follow physical entry identities through SSA.
#![allow(clippy::unwrap_used)]
use dex_decompiler::{DexPasses, TypeContext};
use dex_format::{RawSpan, code::CodeItem};
use managed_ir::{insn::ManagedInsn, types::ManagedType};
struct Signature;
impl TypeContext for Signature {
    fn initial_register_type(&self, register: u16, wide: bool) -> Option<ManagedType> {
        match (register, wide) {
            (0, false) => Some(ManagedType::Int),
            (0, true) => Some(ManagedType::Long),
            _ => None,
        }
    }
}
fn code(units: &[u16]) -> CodeItem {
    CodeItem {
        registers_size: 2,
        ins_size: 2,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: RawSpan { offset: 0, size: 0 },
    }
}
#[test]
fn incoming_wide_identity_keeps_both_physical_words() {
    let ssa = DexPasses::run_ssa_with(&code(&[0x0010]), &Signature).unwrap();
    let value = ssa
        .ir()
        .values()
        .iter()
        .find(|(_, v)| v.ty() == &ManagedType::Long)
        .unwrap()
        .0;
    let mut words: Vec<_> = ssa
        .ir()
        .register_slots()
        .values()
        .filter(|s| s.values().contains(&value))
        .map(|s| s.index())
        .collect();
    words.sort_unstable();
    assert_eq!(words, vec![0, 1]);
}
#[test]
fn newly_created_ssa_initial_uses_slot_descriptor_not_later_write() {
    // Branch bypasses the file-order v0 write; SSA must create its entry fallback.
    let ssa = DexPasses::run_ssa_with(&code(&[0x0138, 3, 0x7012, 0x000f]), &Signature).unwrap();
    let initials: Vec<_> = ssa
        .ir()
        .values()
        .iter()
        .filter(|(_, v)| {
            matches!(
                ssa.ir().instruction(v.def()),
                Ok(ManagedInsn::Opaque {
                    family_tag: 0x8003,
                    ..
                })
            )
        })
        .collect();
    assert!(initials.iter().any(|(id, v)| {
        v.ty() == &ManagedType::Int
            && ssa
                .ir()
                .register_slots()
                .values()
                .any(|s| s.index() == 0 && s.values().contains(id))
    }));
    assert!(ssa.ir().instructions().values().any(|i| matches!(
        i,
        ManagedInsn::Const {
            value: managed_ir::ops::Constant::Int(7),
            ..
        }
    )));
}
