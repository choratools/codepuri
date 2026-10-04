//! Direct outcome proof: a handler edge never attests constructor success.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::method::{InsnArena, ValueArena};
use managed_ir::types::ManagedType;
use managed_ir::value::Value;
struct R;
impl JavaResolver for R {
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
#[test]
fn exact_phi_edge_requires_success_even_after_exception_handler() {
    for mode in 0..=4 {
        let mut insns = InsnArena::default();
        let mut values = ValueArena::default();
        let mut object = None;
        let allocation = insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
            object = Some(v);
            ManagedInsn::NewInstance {
                result: v,
                class: ClassRef::new(0),
            }
        });
        let object = object.unwrap();
        let initializer = insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(0),
            args: vec![object],
        });
        let other = insns.insert(ManagedInsn::Return { value: None });
        let handler = insns.insert(ManagedInsn::Return { value: None });
        let mut out = None;
        let phi = insns.insert_with_key(|id| {
            let value = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
            out = Some(value);
            ManagedInsn::Phi {
                result: value,
                inputs: vec![],
            }
        });
        let start = insns.insert(ManagedInsn::Return { value: None });
        let mut cfg = Cfg::new(start);
        let a = cfg.add_block(allocation);
        let c = cfg.add_block(initializer);
        let o = cfg.add_block(other);
        let h = cfg.add_block(handler);
        let p = (mode != 4).then(|| cfg.add_block(phi));
        let target = p.unwrap_or(o);
        cfg.add_edge(cfg.entry(), a, EdgeKind::BranchTrue).unwrap();
        cfg.add_edge(cfg.entry(), o, EdgeKind::BranchFalse).unwrap();
        cfg.add_edge(a, c, EdgeKind::Unconditional).unwrap();
        let edge = match mode {
            1 => cfg.add_edge(a, target, EdgeKind::Unconditional).unwrap(),
            2 => cfg
                .add_edge(c, target, EdgeKind::Exception("E".into()))
                .unwrap(),
            3 => {
                cfg.add_edge(c, h, EdgeKind::Exception("E".into())).unwrap();
                cfg.add_edge(h, target, EdgeKind::Unconditional).unwrap()
            }
            _ => cfg.add_edge(c, target, EdgeKind::Unconditional).unwrap(),
        };
        insns[phi] = ManagedInsn::Phi {
            result: out.unwrap(),
            inputs: vec![(edge, object)],
        };
        let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
        let e = Emitter {
            ir: &ir,
            resolver: &R,
            values: HashMap::new(),
            lines: vec![],
            next: 0,
            parameters: HashSet::new(),
            reserved: HashSet::new(),
            current: None,
            hoisted: HashSet::new(),
            declarations: HashMap::new(),
            allocation_readable: None,
            object_planning: Default::default(),
        };
        let after = e.after_initializer(initializer).unwrap();
        assert_eq!(
            e.initialized_sequence_use(phi, &[(object, allocation)], &after),
            mode == 0,
            "mode{mode}"
        );
    }
}
