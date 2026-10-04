#![allow(clippy::unwrap_used)]
use super::*;
use crate::{JavaField, JavaMethod};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    ops::{ArithOp, InvokeKind},
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
    types::ManagedType,
    value::Value,
};

struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("java.lang.Object".into())
    }
    fn method(&self, method: MethodRef) -> Option<JavaMethod> {
        (method.index() == 0).then(|| JavaMethod {
            owner: "Consumer".into(),
            name: "consume".into(),
            is_static: true,
            parameters: vec![JavaType::Int, JavaType::Int],
            return_type: JavaType::Int,
            throws: vec![],
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn instruction_type(&self, _: TypeRef) -> Option<JavaType> {
        None
    }
}

struct Fixture {
    insns: InsnArena,
    values: ValueArena,
    cfg: Cfg<InsnId>,
}
impl Fixture {
    fn new() -> Self {
        let mut insns = InsnArena::with_key();
        let entry = insns.insert(ManagedInsn::Return { value: None });
        Self {
            insns,
            values: ValueArena::with_key(),
            cfg: Cfg::new(entry),
        }
    }
    fn value(
        &mut self,
        ty: ManagedType,
        instruction: impl FnOnce(ValueId) -> ManagedInsn,
    ) -> ValueId {
        let id = self.insns.insert(ManagedInsn::Return { value: None });
        let value = self.values.insert(Value::new(id, ty));
        self.insns[id] = instruction(value);
        value
    }
    fn finish(self) -> MethodIr<SsaForm> {
        MethodIr::<SsaForm>::from_parts(self.cfg, self.insns, self.values)
    }
}

// The pre-index collector grew the Move closure by repeated complete scans.
fn scan_closure(ir: &MethodIr<SsaForm>, locals: &[ValueId]) -> HashSet<ValueId> {
    let mut locals = locals.to_vec();
    loop {
        let before = locals.len();
        for (_, instruction) in ir.instructions() {
            if let ManagedInsn::Move { result, value } = instruction
                && locals.contains(value)
                && !locals.contains(result)
            {
                locals.push(*result);
            }
        }
        if before == locals.len() {
            return locals.into_iter().collect();
        }
    }
}

#[test]
fn indexed_consumers_equal_full_arena_scan_with_order_and_duplicates() {
    let mut f = Fixture::new();
    let seed = f.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(2),
    });
    let unrelated = f.value(ManagedType::Unknown, |result| ManagedInsn::Opaque {
        family_tag: 1,
        reads: vec![],
        writes: vec![result],
        effects: EffectFlags::NONE,
    });
    // Deliberately reverse arena order of a three-hop copy chain.
    let a = f.value(ManagedType::Int, |_| ManagedInsn::Return { value: None });
    let b = f.value(ManagedType::Int, |_| ManagedInsn::Return { value: None });
    let c = f.value(ManagedType::Int, |_| ManagedInsn::Return { value: None });
    for (result, value) in [(a, b), (b, c), (c, seed)] {
        let id = f.values[result].def();
        f.insns[id] = ManagedInsn::Move { result, value };
    }
    f.insns.insert(ManagedInsn::FieldGet {
        result: unrelated,
        object: Some(unrelated),
        field: FieldRef::new(99),
    });
    f.insns.insert(ManagedInsn::FieldPut {
        object: Some(unrelated),
        field: FieldRef::new(99),
        value: unrelated,
    });
    f.insns.insert(ManagedInsn::Cast {
        result: unrelated,
        value: unrelated,
        target: TypeRef::new(99),
    });
    f.insns.insert(ManagedInsn::NewArray {
        result: unrelated,
        length: unrelated,
        element: TypeRef::new(99),
    });
    f.insns.insert(ManagedInsn::Opaque {
        family_tag: 2,
        reads: vec![unrelated],
        writes: vec![],
        effects: EffectFlags::NONE,
    });
    f.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
        op: ArithOp::Add,
        result,
        lhs: a,
        rhs: a,
    });
    f.insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(0),
        args: vec![b, b],
    });
    f.insns.insert(ManagedInsn::Return { value: Some(c) });
    let ir = f.finish();
    let index = MethodIndex::new(&ir);
    let locals = scan_closure(&ir, &[seed, seed]);
    assert_eq!(index.copy_closure(&[seed, seed]), locals);
    let all: Vec<_> = (0..index.instructions.len()).collect();
    let scanned = consumer_constraints_at_positions(
        &ir,
        &index,
        &locals,
        &all,
        &JavaType::Long,
        &Resolver,
        &HashMap::new(),
    );
    let indexed = consumer_constraints_indexed(
        &ir,
        &index,
        &[seed, seed],
        &JavaType::Long,
        &Resolver,
        &HashMap::new(),
    );
    assert_eq!(indexed, scanned);
    assert_eq!(
        indexed.unwrap(),
        vec![
            JavaType::Int,
            JavaType::Int,
            JavaType::Int,
            JavaType::Int,
            JavaType::Long
        ]
    );
    assert!(index.consumer_positions(&locals).len() < all.len());
}

#[test]
fn outside_cfg_opaque_read_and_missing_metadata_still_reject() {
    for kind in 0..3 {
        let mut f = Fixture::new();
        let seed = f.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(2),
        });
        let copied = f.value(ManagedType::Int, |result| ManagedInsn::Move {
            result,
            value: seed,
        });
        let outside = match kind {
            0 => ManagedInsn::Opaque {
                family_tag: 9,
                reads: vec![copied, copied],
                writes: vec![],
                effects: EffectFlags::NONE,
            },
            1 => ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(99),
                args: vec![copied],
            },
            _ => ManagedInsn::FieldPut {
                object: None,
                field: FieldRef::new(99),
                value: copied,
            },
        };
        f.insns.insert(outside);
        let ir = f.finish();
        let index = MethodIndex::new(&ir);
        let locals = scan_closure(&ir, &[seed]);
        let all: Vec<_> = (0..index.instructions.len()).collect();
        let scanned = consumer_constraints_at_positions(
            &ir,
            &index,
            &locals,
            &all,
            &JavaType::Int,
            &Resolver,
            &HashMap::new(),
        );
        let indexed = consumer_constraints_indexed(
            &ir,
            &index,
            &[seed],
            &JavaType::Int,
            &Resolver,
            &HashMap::new(),
        );
        assert_eq!(indexed, scanned);
        assert!(indexed.is_err(), "consumer kind {kind}");
    }
}

#[test]
fn phi_adjacency_preserves_scan_order_duplicate_inputs_and_literal_exclusion() {
    let mut f = Fixture::new();
    let zero = f.value(ManagedType::Zero, |result| ManagedInsn::Const {
        result,
        value: Constant::Zero,
    });
    let leaf = f.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(2),
    });
    let extra_block = f
        .cfg
        .add_block(f.cfg.block(f.cfg.entry()).unwrap().payload().to_owned());
    let edge = f
        .cfg
        .add_edge(f.cfg.entry(), extra_block, EdgeKind::Unconditional)
        .unwrap();
    let phi = f.value(ManagedType::Int, |result| ManagedInsn::Phi {
        result,
        inputs: vec![(edge, zero), (edge, leaf), (edge, leaf)],
    });
    let copied = f.value(ManagedType::Int, |result| ManagedInsn::Move {
        result,
        value: phi,
    });
    let next = f.value(ManagedType::Int, |result| ManagedInsn::Phi {
        result,
        inputs: vec![(edge, phi), (edge, phi), (edge, copied)],
    });
    let literal_only = f.value(ManagedType::Unknown, |result| ManagedInsn::Phi {
        result,
        inputs: vec![(edge, zero)],
    });
    f.insns.insert(ManagedInsn::Return { value: Some(next) });
    let ir = f.finish();
    let index = MethodIndex::new(&ir);
    let mut scan_index = MethodIndex::new(&ir);
    scan_index.downstream.clear();
    for (value, _) in ir.values() {
        let mut scanned = Vec::new();
        for (_, instruction) in ir.instructions() {
            match instruction {
                ManagedInsn::Move {
                    result,
                    value: source,
                } if *source == value => scanned.push(*result),
                ManagedInsn::Phi { result, inputs }
                    if inputs.iter().any(|(_, source)| *source == value) =>
                {
                    scanned.push(*result)
                }
                _ => {}
            }
        }
        assert_eq!(
            index.downstream.get(&value).cloned().unwrap_or_default(),
            scanned
        );
        scan_index.downstream.insert(value, scanned);
    }
    let hierarchy = Hierarchy::default();
    let indexed =
        phi_proof_indexed(&ir, &index, phi, &JavaType::Int, &[], &hierarchy, &Resolver).unwrap();
    let scanned = phi_proof_indexed(
        &ir,
        &scan_index,
        phi,
        &JavaType::Int,
        &[],
        &hierarchy,
        &Resolver,
    )
    .unwrap();
    assert_eq!(indexed.plan, scanned.plan);
    assert_eq!(indexed.leaves, scanned.leaves);
    assert_eq!(
        indexed
            .plan
            .inputs
            .iter()
            .map(|(value, _)| *value)
            .collect::<Vec<_>>(),
        vec![zero, leaf, leaf]
    );
    assert!(!indexed.plan.locals.contains(&zero));
    assert!(!indexed.plan.locals.contains(&literal_only));
    let declarations = indexed
        .plan
        .locals
        .iter()
        .map(|v| (*v, JavaType::Int))
        .collect();
    let consumers = consumer_constraints_indexed(
        &ir,
        &index,
        &indexed.plan.locals,
        &JavaType::Int,
        &Resolver,
        &declarations,
    )
    .unwrap();
    let two_pass = phi_plan(&ir, phi, &JavaType::Int, &consumers, &hierarchy, &Resolver).unwrap();
    assert_eq!(
        checked_phi_plan(
            &ir,
            phi,
            &JavaType::Int,
            &JavaType::Int,
            &hierarchy,
            &Resolver
        )
        .unwrap(),
        two_pass
    );
}
