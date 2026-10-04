//! Original handler paths observe only their exact predecessor-selected Phi.
#![allow(clippy::unwrap_used)]
use super::*;
use managed_ir::{
    method::{InsnArena, ValueArena},
    ops::Constant,
    types::ManagedType,
    value::Value,
};
fn fixture(mode: u8) -> (MethodIr<SsaForm>, BlockId, EdgeId, HashSet<ValueId>) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let root = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = decomp_core::cfg::Cfg::new(root);
    let foreign = insns.insert(ManagedInsn::Return { value: None });
    let foreign = cfg.add_block(foreign);
    let n = insns.insert(ManagedInsn::Return { value: None });
    let zero = values.insert(Value::new(n, ManagedType::Null));
    insns[n] = ManagedInsn::Const {
        result: zero,
        value: Constant::Null,
    };
    let a = insns.insert(ManagedInsn::Return { value: None });
    let forbidden = values.insert(Value::new(a, ManagedType::Unknown));
    let handler = insns.insert(ManagedInsn::Return { value: None });
    let entry = cfg.add_block(handler);
    let incoming = cfg
        .add_edge(cfg.entry(), entry, EdgeKind::Exception("any".into()))
        .unwrap();
    let p = insns.insert(ManagedInsn::Return { value: None });
    let join = cfg.add_block(p);
    let selected = values.insert(Value::new(p, ManagedType::Unknown));
    let first = cfg.add_edge(entry, join, EdgeKind::Unconditional).unwrap();
    let outside = cfg
        .add_edge(foreign, join, EdgeKind::Unconditional)
        .unwrap();
    insns[p] = ManagedInsn::Phi {
        result: selected,
        inputs: vec![
            (first, if mode == 1 { forbidden } else { zero }),
            (outside, forbidden),
        ],
    };
    let mut end = join;
    if mode == 2 {
        cfg.add_edge(entry, foreign, EdgeKind::BranchFalse).unwrap();
    }
    if mode == 3 {
        insns[p] = ManagedInsn::Phi {
            result: selected,
            inputs: vec![(outside, zero)],
        };
    }
    if mode == 4 {
        insns[p] = ManagedInsn::Phi {
            result: selected,
            inputs: vec![(first, zero), (first, zero), (outside, forbidden)],
        };
    }
    if mode == 5 || mode == 6 {
        let pp = insns.insert(ManagedInsn::Return { value: None });
        let pb = cfg.add_block(pp);
        let result = values.insert(Value::new(pp, ManagedType::Unknown));
        cfg.add_edge(join, pb, EdgeKind::Unconditional).unwrap();
        insns[pp] = ManagedInsn::Phi {
            result,
            inputs: vec![
                (first, if mode == 6 { forbidden } else { zero }),
                (outside, forbidden),
            ],
        };
        end = pb;
    }
    let ret = insns.insert(ManagedInsn::Return {
        value: Some(selected),
    });
    let ret = cfg.add_block(ret);
    cfg.add_edge(end, ret, EdgeKind::Unconditional).unwrap();
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        entry,
        incoming,
        HashSet::from([forbidden]),
    )
}
#[test]
fn selected_handler_edge_excludes_foreign_phi_but_never_reachable_alias() {
    for mode in [0, 5] {
        let (ir, entry, edge, aliases) = fixture(mode);
        assert!(
            !handler_observes(&ir, entry, edge, &aliases, &ClosedResolver),
            "mode {mode}"
        );
    }
    for mode in [1, 2, 3, 4, 6] {
        let (ir, entry, edge, aliases) = fixture(mode);
        assert!(
            handler_observes(&ir, entry, edge, &aliases, &ClosedResolver),
            "mode {mode}"
        );
    }
}

struct ClosedResolver;
impl JavaResolver for ClosedResolver {
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
    }
}
