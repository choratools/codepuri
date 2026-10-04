//! This is exact receiver identity only, never prefix effect permission.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    types::ManagedType,
    value::Value,
};
fn key(_: MethodRef) -> Option<MethodKey> {
    Some(MethodKey {
        owner: "LOwner;".into(),
        name: "<init>".into(),
        parameters: vec![],
        return_type: "V".into(),
    })
}
fn graph(mode: u8) -> (MethodIr<SsaForm>, ValueId) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let initial = insns.insert(ManagedInsn::Return { value: None });
    let receiver = values.insert(Value::new(
        initial,
        ManagedType::Reference(ClassRef::new(0)),
    ));
    insns[initial] = ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![receiver],
        effects: EffectFlags::NONE,
    };
    let other_insn = insns.insert(ManagedInsn::Return { value: None });
    let other = values.insert(Value::new(
        other_insn,
        ManagedType::Reference(ClassRef::new(0)),
    ));
    insns[other_insn] = ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![other],
        effects: EffectFlags::NONE,
    };
    let branch = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(branch);
    let entry = cfg.entry();
    let mut arms = Vec::new();
    for source in [receiver, if mode == 1 { other } else { receiver }] {
        let id = insns.insert(ManagedInsn::Return { value: None });
        let alias = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
        insns[id] = ManagedInsn::Move {
            result: alias,
            value: source,
        };
        arms.push((cfg.add_block(id), alias));
    }
    let id = insns.insert(ManagedInsn::Return { value: None });
    let value = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
    let head = cfg.add_block(id);
    let first = cfg
        .add_edge(arms[0].0, head, EdgeKind::Unconditional)
        .unwrap();
    let second = cfg
        .add_edge(arms[1].0, head, EdgeKind::Unconditional)
        .unwrap();
    let mut inputs = vec![(first, arms[0].1), (second, arms[1].1)];
    if mode == 2 {
        inputs[1].1 = value;
    }
    if mode == 3 {
        inputs[1].0 = first;
    }
    if mode == 4 {
        inputs.swap(0, 1);
        inputs[0].0 = first;
        inputs[1].0 = second;
    }
    if mode == 5 {
        inputs.clear();
    }
    insns[id] = ManagedInsn::Phi {
        result: value,
        inputs,
    };
    let id2 = insns.insert(ManagedInsn::Return { value: None });
    let value2 = values.insert(Value::new(id2, ManagedType::Reference(ClassRef::new(0))));
    insns[id2] = ManagedInsn::Phi {
        result: value2,
        inputs: vec![(first, arms[0].1), (second, arms[1].1)],
    };
    let next = cfg.add_block(id2);
    cfg.add_edge(head, next, EdgeKind::Unconditional).unwrap();
    let target = cfg.add_block(insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        method: MethodRef::new(7),
        args: vec![if mode == 0 { value2 } else { value }],
        result: None,
    }));
    cfg.add_edge(next, target, EdgeKind::Unconditional).unwrap();
    let yes = cfg
        .add_edge(entry, arms[0].0, EdgeKind::BranchTrue)
        .unwrap();
    let no = cfg
        .add_edge(entry, arms[1].0, EdgeKind::BranchFalse)
        .unwrap();
    insns[branch] = ManagedInsn::Branch {
        condition: Some(receiver),
        then_edge: yes,
        else_edge: Some(no),
    };
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        receiver,
    )
}
#[test]
fn genuine_same_receiver_phi_cluster_attests_exact_original_target() {
    let (ir, receiver) = graph(0);
    assert_eq!(
        attest(&ir, receiver, "LOwner;", key),
        Some(MethodRef::new(7))
    );
}
#[test]
fn foreign_cycle_duplicate_missing_and_wrong_predecessor_inputs_reject() {
    for mode in 1..=5 {
        let (ir, receiver) = graph(mode);
        assert_eq!(attest(&ir, receiver, "LOwner;", key), None, "mode {mode}");
    }
}
