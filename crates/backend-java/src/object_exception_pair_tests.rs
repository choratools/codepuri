//! Exact exception-chain capability seam; provenance proofs belong to renderer.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
};
struct Resolver {
    chain: Option<Vec<InsnId>>,
}
impl JavaResolver for Resolver {
    fn exception_allocation_chain(&self, chain: &[InsnId]) -> bool {
        self.chain.as_deref() == Some(chain)
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Fixture.Outer".into())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "Fixture.Outer".into(),
            name: "<init>".into(),
            is_static: false,
            return_type: JavaType::Void,
            parameters: vec![],
            throws: vec![],
        })
    }
}
fn fixture(extra: bool, allocation_protected: bool) -> (MethodIr<SsaForm>, Vec<InsnId>) {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let mut v = None;
    let allocation = insns.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
        v = Some(result);
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    let mut alias = None;
    let commit = insns.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
        alias = Some(result);
        ManagedInsn::Move {
            result,
            value: v.unwrap(),
        }
    });
    let mut chain = vec![allocation, commit];
    if extra {
        chain.push(insns.insert_with_key(|id| {
            let result = values.insert(Value::new(id, ManagedType::Int));
            ManagedInsn::Const {
                result,
                value: Constant::Int(3),
            }
        }));
    }
    let initializer = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![alias.unwrap()],
    });
    chain.push(initializer);
    let exit = insns.insert(ManagedInsn::Return {
        value: Some(alias.unwrap()),
    });
    let handler = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(allocation);
    let entry = cfg.entry();
    let mut current = entry;
    for id in &chain[1..] {
        let next = cfg.add_block(*id);
        cfg.add_edge(current, next, EdgeKind::Unconditional)
            .unwrap();
        current = next;
    }
    let ctor = current;
    let normal_exit = cfg.add_block(exit);
    cfg.add_edge(ctor, normal_exit, EdgeKind::Unconditional)
        .unwrap();
    let catch = cfg.add_block(handler);
    if allocation_protected {
        cfg.add_edge(entry, catch, EdgeKind::Exception("E".into()))
            .unwrap();
    }
    cfg.add_edge(ctor, catch, EdgeKind::Exception("E".into()))
        .unwrap();
    (MethodIr::<SsaForm>::from_parts(cfg, insns, values), chain)
}
fn plan(
    ir: &MethodIr<SsaForm>,
    chain: &[InsnId],
    attested: Option<Vec<InsnId>>,
) -> Result<ObjectPlan, Unsupported> {
    let r = Resolver { chain: attested };
    let emitter = Emitter {
        allocation_readable: None,
        object_planning: Default::default(),
        ir,
        resolver: &r,
        values: HashMap::new(),
        lines: vec![],
        next: 0,
        parameters: HashSet::new(),
        reserved: HashSet::new(),
        current: None,
        hoisted: HashSet::new(),
        declarations: HashMap::new(),
    };
    emitter.object_plan(chain[0])
}
#[test]
fn exception_pair_requires_exact_scoped_alias_chain_capability() {
    let (ir, chain) = fixture(false, true);
    assert!(plan(&ir, &chain, None).is_err());
    assert!(plan(&ir, &chain, Some(vec![chain[0], chain[2]])).is_err());
    let p = plan(&ir, &chain, Some(chain.clone())).unwrap();
    assert_eq!(p.initializer, chain[2]);
    assert_eq!(p.aliases.len(), 2);
    assert!(p.argument_gap.is_empty());
    // Ordinary metadata cannot move a formerly unprotected allocation into try.
    let (ir, chain) = fixture(false, false);
    assert!(plan(&ir, &chain, None).is_err());
    // Even an affirmative external capability cannot widen the alias-only grammar.
    let (ir, chain) = fixture(true, true);
    assert!(plan(&ir, &chain, Some(chain.clone())).is_err());
}
