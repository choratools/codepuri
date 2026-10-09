//! Differential success-cut checks over loops, joins, exceptional and dead paths.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::method::{InsnArena, ValueArena};

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
fn emitter(ir: &MethodIr<SsaForm>) -> Emitter<'_, R> {
    Emitter {
        ir,
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
    }
}
fn reference(ir: &MethodIr<SsaForm>, cut: InsnId) -> HashSet<InsnId> {
    let walk = |cut: Option<InsnId>| {
        let mut seen = HashSet::new();
        let mut instructions = HashSet::new();
        let mut pending = vec![ir.cfg().entry()];
        while let Some(block) = pending.pop() {
            if !seen.insert(block) {
                continue;
            }
            let node = ir.cfg().block(block).unwrap();
            if Some(*node.payload()) != cut {
                instructions.insert(*node.payload());
            }
            for edge_id in node.outgoing() {
                let edge = ir.cfg().edge(*edge_id).unwrap();
                if Some(*node.payload()) != cut || matches!(edge.kind(), EdgeKind::Exception(_)) {
                    pending.push(edge.target());
                }
            }
        }
        instructions
    };
    walk(None).difference(&walk(Some(cut))).copied().collect()
}

#[test]
fn success_dominance_matches_every_three_node_normal_exception_graph() {
    for mut mask in 0..3usize.pow(9) {
        let mut instructions = InsnArena::with_key();
        let ids: Vec<_> = (0..4)
            .map(|_| instructions.insert(ManagedInsn::Return { value: None }))
            .collect();
        let mut cfg = Cfg::new(ids[0]);
        let nodes = [cfg.entry(), cfg.add_block(ids[1]), cfg.add_block(ids[2])];
        for source in nodes {
            for target in nodes {
                match mask % 3 {
                    1 => {
                        cfg.add_edge(source, target, EdgeKind::Unconditional)
                            .unwrap();
                    }
                    2 => {
                        cfg.add_edge(source, target, EdgeKind::Exception("E".into()))
                            .unwrap();
                    }
                    _ => {}
                }
                mask /= 3;
            }
        }
        let ir = MethodIr::<SsaForm>::from_parts(cfg, instructions, ValueArena::with_key());
        let e = emitter(&ir);
        assert!(e.object_success_dominance().is_some());
        for &cut in &ids {
            let expected = reference(&ir, cut);
            let actual = e.after_initializer(cut).unwrap();
            for target in &ids {
                assert_eq!(actual.contains(target), expected.contains(target));
            }
            assert_eq!(e.after_initializer(cut).unwrap().into_set(), expected);
            for &other in &ids {
                let other_expected = reference(&ir, other);
                let intersection = actual.intersect(&e.after_initializer(other).unwrap());
                let expected: HashSet<_> =
                    expected.intersection(&other_expected).copied().collect();
                assert_eq!(intersection.into_set(), expected);
            }
        }
    }
}

#[test]
fn a_small_cut_cache_promotes_to_one_shared_success_index() {
    let mut instructions = InsnArena::with_key();
    let ids: Vec<_> = (0..6)
        .map(|_| instructions.insert(ManagedInsn::Return { value: None }))
        .collect();
    let mut cfg = Cfg::new(ids[0]);
    let mut nodes = vec![cfg.entry()];
    nodes.extend(ids[1..].iter().map(|id| cfg.add_block(*id)));
    cfg.add_edge(nodes[0], nodes[1], EdgeKind::BranchTrue)
        .unwrap();
    cfg.add_edge(nodes[0], nodes[2], EdgeKind::BranchFalse)
        .unwrap();
    cfg.add_edge(nodes[1], nodes[3], EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(nodes[2], nodes[3], EdgeKind::Exception("E".into()))
        .unwrap();
    cfg.add_edge(nodes[3], nodes[4], EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(nodes[4], nodes[5], EdgeKind::Unconditional)
        .unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, instructions, ValueArena::with_key());
    let e = emitter(&ir);
    let first = e.after_initializer(ids[0]).unwrap();
    let second = e.after_initializer(ids[1]).unwrap();
    let intersection = first.intersect(&second);
    assert!(matches!(
        intersection,
        InstructionAvailability::SharedPair(_)
    ));
    assert_eq!(
        intersection.into_set(),
        reference(&ir, ids[0])
            .intersection(&reference(&ir, ids[1]))
            .copied()
            .collect()
    );
    for &cut in &ids[..4] {
        assert_eq!(
            e.after_initializer(cut).unwrap().into_set(),
            reference(&ir, cut)
        );
    }
    assert!(e.object_planning.success_dominance.get().is_none());
    assert_eq!(e.object_planning.success_cuts.borrow().len(), 4);
    assert_eq!(
        e.after_initializer(ids[4]).unwrap().into_set(),
        reference(&ir, ids[4])
    );
    assert!(e.object_planning.success_dominance.get().is_some());
    assert!(e.object_planning.success_cuts.borrow().is_empty());
    for &cut in &ids {
        assert_eq!(
            e.after_initializer(cut).unwrap().into_set(),
            reference(&ir, cut)
        );
    }
}

#[test]
fn duplicate_payloads_keep_original_cut_traversal_and_cache_rejection() {
    let mut instructions = InsnArena::with_key();
    let cut = instructions.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(cut);
    let duplicate = cfg.add_block(cut);
    cfg.add_edge(cfg.entry(), duplicate, EdgeKind::Exception("E".into()))
        .unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, instructions, ValueArena::with_key());
    let e = emitter(&ir);
    assert!(e.object_success_dominance().is_none());
    assert!(e.object_success_dominance().is_none());
    assert_eq!(
        e.after_initializer(cut).unwrap().into_set(),
        reference(&ir, cut)
    );
}
