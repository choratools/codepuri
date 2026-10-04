#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    types::ManagedType,
    value::Value,
};

fn key(method: MethodRef) -> Option<MethodKey> {
    Some(MethodKey {
        owner: "LOwner;".into(),
        name: "<init>".into(),
        parameters: if method.index() == 8 {
            vec!["I".into()]
        } else {
            vec![]
        },
        return_type: "V".into(),
    })
}
// This fixture exercises identity only. Source validity is independently checked
// by constructor lowering; these guards never authorize prefix effects.
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
    let branch = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(branch);
    let entry = cfg.entry();
    let alias_insn = insns.insert(ManagedInsn::Return { value: None });
    let alias = values.insert(Value::new(
        alias_insn,
        ManagedType::Reference(ClassRef::new(0)),
    ));
    insns[alias_insn] = ManagedInsn::Move {
        result: alias,
        value: receiver,
    };
    let left = cfg.add_block(alias_insn);
    let first = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        method: MethodRef::new(7),
        args: vec![alias],
        result: None,
    });
    let target = cfg.add_block(first);
    cfg.add_edge(left, target, EdgeKind::Unconditional).unwrap();
    let second = insns.insert(match mode {
        1 => ManagedInsn::Throw { value: receiver },
        2 => ManagedInsn::Return { value: None },
        _ => ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            method: MethodRef::new(if mode == 3 { 8 } else { 7 }),
            args: vec![receiver],
            result: None,
        },
    });
    let right = cfg.add_block(second);
    let yes = cfg.add_edge(entry, left, EdgeKind::BranchTrue).unwrap();
    let no = cfg.add_edge(entry, right, EdgeKind::BranchFalse).unwrap();
    insns[branch] = ManagedInsn::Branch {
        condition: Some(receiver),
        then_edge: yes,
        else_edge: Some(no),
    };
    if mode == 4 {
        insns[second] = ManagedInsn::Move {
            result: alias,
            value: receiver,
        };
        cfg.add_edge(right, entry, EdgeKind::Unconditional).unwrap();
    }
    if mode == 5 {
        cfg.add_edge(
            left,
            right,
            EdgeKind::Exception("java.lang.Throwable".into()),
        )
        .unwrap();
    }
    if mode == 6 {
        cfg.add_edge(
            target,
            right,
            EdgeKind::Exception("java.lang.Throwable".into()),
        )
        .unwrap();
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        receiver,
    )
}

#[test]
fn normal_paths_share_exact_receiver_bound_target_and_throw_paths_end() {
    for mode in [0, 1] {
        let (ir, receiver) = graph(mode);
        assert_eq!(
            attest(&ir, receiver, "LOwner;", key),
            Some(MethodRef::new(7))
        );
        assert_eq!(attest(&ir, receiver, "LOther;", key), None);
        assert_eq!(attest(&ir, receiver, "LOwner;", |_| None), None);
    }
}

#[test]
fn incomplete_mixed_target_cyclic_or_exceptional_prefixes_never_attest() {
    for mode in [2, 3, 4, 5, 6] {
        let (ir, receiver) = graph(mode);
        assert_eq!(attest(&ir, receiver, "LOwner;", key), None, "mode {mode}");
    }
}
