//! Nested synthetic commits stay uninitialized until their exact constructor.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
};
struct Resolver {
    witness: bool,
}
impl JavaResolver for Resolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r.index() == 0 {
                "Fixture.Outer"
            } else {
                "Fixture.Inner"
            }
            .into(),
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: if r.index() == 0 {
                "Fixture.Outer"
            } else {
                "Fixture.Inner"
            }
            .into(),
            name: "<init>".into(),
            is_static: false,
            return_type: JavaType::Void,
            parameters: if r.index() == 0 {
                vec![JavaType::Class("Fixture.Inner".into())]
            } else {
                vec![]
            },
            throws: vec![],
        })
    }
    fn exception_allocation_chain(&self, _: &[InsnId]) -> bool {
        self.witness
    }
}
fn fixture_observe(escape: bool, early: bool, observe: bool) -> MethodIr<SsaForm> {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let mut order = Vec::new();
    let (outer, _inner, alias) = {
        let mut make = |ty: ManagedType, f: &dyn Fn(ValueId) -> ManagedInsn| {
            let mut value = None;
            let id = insns.insert_with_key(|id| {
                let v = values.insert(Value::new(id, ty));
                value = Some(v);
                f(v)
            });
            order.push(id);
            value.unwrap()
        };
        let outer = make(ManagedType::Reference(ClassRef::new(0)), &|result| {
            ManagedInsn::NewInstance {
                result,
                class: ClassRef::new(0),
            }
        });
        let inner = make(ManagedType::Reference(ClassRef::new(1)), &|result| {
            ManagedInsn::NewInstance {
                result,
                class: ClassRef::new(1),
            }
        });
        let alias = make(ManagedType::Reference(ClassRef::new(1)), &|result| {
            ManagedInsn::Move {
                result,
                value: inner,
            }
        });
        (outer, inner, alias)
    };
    if early {
        order.push(insns.insert(ManagedInsn::Return { value: Some(alias) }));
    }
    order.push(insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![alias],
    }));
    order.push(insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![outer, alias],
    }));
    order.push(insns.insert(ManagedInsn::Return {
        value: Some(if escape { alias } else { outer }),
    }));
    let mut cfg = Cfg::new(order[0]);
    let mut blocks = vec![cfg.entry()];
    let mut last = cfg.entry();
    for i in &order[1..] {
        let n = cfg.add_block(*i);
        cfg.add_edge(last, n, EdgeKind::Unconditional).unwrap();
        blocks.push(n);
        last = n;
    }
    let h = cfg.add_block(insns.insert(ManagedInsn::Return {
        value: if observe { Some(alias) } else { None },
    }));
    for block in blocks.iter().take(blocks.len() - 1) {
        if !matches!(
            insns[*cfg.block(*block).unwrap().payload()],
            ManagedInsn::Move { .. }
        ) {
            cfg.add_edge(*block, h, EdgeKind::Exception("E".into()))
                .unwrap();
        }
    }
    MethodIr::<SsaForm>::from_parts(cfg, insns, values)
}
fn plan(ir: &MethodIr<SsaForm>, witness: bool) -> Result<ObjectPlan, Unsupported> {
    let r = Resolver { witness };
    let e = Emitter {
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
    e.sequence_object_plan(*ir.cfg().block(ir.cfg().entry()).unwrap().payload())
}
#[test]
fn nested_exception_commit_alias_requires_exact_witness() {
    assert!(plan(&fixture(false, false), true).is_ok());
    assert!(plan(&fixture(false, false), false).is_err());
}
#[test]
fn nested_alias_escape_and_uninitialized_read_reject() {
    assert!(plan(&fixture(true, false), true).is_err());
    assert!(plan(&fixture(false, true), true).is_err());
}

fn fixture(escape: bool, early: bool) -> MethodIr<SsaForm> {
    fixture_observe(escape, early, false)
}
struct Metadata {
    scope: decomp_structure::exceptions::RegionScope,
    original: crate::exceptions::OriginalScope,
    hierarchy: crate::type_compatibility::Hierarchy,
}
impl crate::exceptions::RegionMetadata for Metadata {
    fn original_scope(&self) -> Option<&crate::exceptions::OriginalScope> {
        Some(&self.original)
    }
    fn region_scope(&self) -> &decomp_structure::exceptions::RegionScope {
        &self.scope
    }
    fn catch_type(&self, _: &str) -> Option<JavaType> {
        Some(JavaType::Class("java.lang.RuntimeException".into()))
    }
    fn hierarchy(&self) -> &crate::type_compatibility::Hierarchy {
        &self.hierarchy
    }
    fn is_catch_all(&self, _: &str) -> bool {
        false
    }
}
fn proofs(observe: bool) -> std::collections::HashSet<Vec<InsnId>> {
    let ir = fixture_observe(false, false, observe);
    let blocks = ir.cfg().block_ids().collect::<Vec<_>>();
    let handler = *blocks.last().unwrap();
    let offsets = blocks
        .iter()
        .enumerate()
        .filter_map(|(n, b)| {
            let id = *ir.cfg().block(*b).unwrap().payload();
            (!matches!(ir.instruction(id).unwrap(), ManagedInsn::Move { .. }))
                .then_some((id, n as u32))
        })
        .collect();
    let md = Metadata {
        scope: decomp_structure::exceptions::RegionScope {
            protected: blocks[..blocks.len() - 2].to_vec(),
            catches: vec![decomp_structure::exceptions::Catch {
                key: "E".into(),
                entry: handler,
            }],
            continuation: Some(blocks[blocks.len() - 2]),
        },
        original: crate::exceptions::OriginalScope {
            ordinal: 0,
            start: 0,
            end: (blocks.len() - 2) as u32,
            handler_offset: 1,
            catches: vec![("E".into(), (blocks.len() - 1) as u32)],
            offsets,
        },
        hierarchy: Default::default(),
    };
    crate::exceptions::original_allocation_chains(&ir, &[&md], &Resolver { witness: false })
}
#[test]
fn nested_handler_visible_synthetic_alias_denies_genuine_witness() {
    assert!(!proofs(false).is_empty());
    assert!(proofs(true).is_empty());
}
