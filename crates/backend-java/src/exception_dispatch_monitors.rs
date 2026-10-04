//! Same-object monitor ownership across original normal and exceptional edges.
//!
//! The renderer may use lexical synchronized only after this proof: acquisition
//! failures own no lock; every reachable release owns the acquired identity;
//! every potentially throwing held operation has a final Throwable handler;
//! normal/exceptional continuations agree on ownership. Proven owned releases
//! cannot throw NPE/IllegalMonitorStateException, so their encoded exceptional
//! edges are retained in the graph but not taken by the lexical lowering.
use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Proof {
    pub owners: HashMap<BlockId, Option<BlockId>>,
    pub acquisitions: HashMap<BlockId, ValueId>,
    pub releases: HashMap<BlockId, EdgeId>,
    pub impossible: HashSet<EdgeId>,
}

pub(super) fn prove(
    ir: &MethodIr<SsaForm>,
    regions: &decomp_structure::exception_dispatch::ScopedPlan,
    metadata: &[&dyn RegionMetadata],
) -> Result<Proof, Error> {
    if !ir.instructions().values().any(|insn| {
        matches!(
            insn,
            ManagedInsn::MonitorEnter { .. } | ManagedInsn::MonitorExit { .. }
        )
    }) {
        return Ok(Proof {
            owners: regions
                .graph
                .states
                .iter()
                .map(|state| (state.block, None))
                .collect(),
            ..Default::default()
        });
    }
    let states = regions
        .graph
        .states
        .iter()
        .map(|state| (state.block, state))
        .collect::<HashMap<_, _>>();
    let instruction = |block| {
        let id = *ir.cfg().block(block).map_err(invalid)?.payload();
        ir.instruction(id).map_err(invalid)
    };
    let mut proof = Proof::default();
    let mut queue = VecDeque::from([(ir.cfg().entry(), None)]);
    while let Some((block, owner)) = queue.pop_front() {
        if let Some(previous) = proof.owners.get(&block) {
            if *previous != owner {
                return Err(Error::Unsupported(
                    "monitor ownership differs across original paths",
                ));
            }
            continue;
        }
        proof.owners.insert(block, owner);
        let state = states
            .get(&block)
            .ok_or(Error::Unsupported("monitor state outside dispatch graph"))?;
        let insn = instruction(block)?;
        let normal_owner = match insn {
            ManagedInsn::MonitorEnter { object } => {
                if owner.is_some() {
                    return Err(Error::Unsupported(
                        "nested monitor requires nested ownership proof",
                    ));
                }
                proof.acquisitions.insert(block, *object);
                Some(block)
            }
            ManagedInsn::MonitorExit { object } => {
                let acquired = owner.ok_or(Error::Unsupported(
                    "monitor exit without successful acquisition",
                ))?;
                let acquired_object = proof.acquisitions[&acquired];
                if !same_identity(ir, acquired_object, *object) {
                    return Err(Error::Unsupported(
                        "monitor release identity differs from acquisition",
                    ));
                }
                let [edge] = state.normal.as_slice() else {
                    return Err(Error::Unsupported(
                        "monitor release lacks exact normal continuation",
                    ));
                };
                proof.releases.insert(block, *edge);
                proof.impossible.extend(state.exceptional.iter().copied());
                None
            }
            ManagedInsn::Return { .. } if owner.is_some() => {
                return Err(Error::Unsupported(
                    "original return still owns explicit monitor",
                ));
            }
            _ => owner,
        };
        if owner.is_some()
            && !matches!(insn, ManagedInsn::MonitorExit { .. })
            && !matches!(
                insn,
                ManagedInsn::Opaque {
                    family_tag: 0x8002,
                    ..
                }
            )
            && may_throw(ir, insn)
        {
            let scope = regions
                .coverage
                .get(&block)
                .and_then(|&index| metadata.get(index))
                .ok_or(Error::Unsupported(
                    "held throwing operation lacks original cleanup table",
                ))?;
            let catches = &scope.region_scope().catches;
            if !catches
                .last()
                .is_some_and(|catch| scope.is_catch_all(&catch.key))
            {
                return Err(Error::Unsupported(
                    "held throwing operation can escape without original release",
                ));
            }
        }
        for &edge in &state.normal {
            queue.push_back((ir.cfg().edge(edge).map_err(invalid)?.target(), normal_owner));
        }
        if !matches!(insn, ManagedInsn::MonitorExit { .. }) {
            for &edge in &state.exceptional {
                queue.push_back((ir.cfg().edge(edge).map_err(invalid)?.target(), owner));
            }
        }
    }
    if proof.owners.len() != regions.graph.states.len() {
        return Err(Error::Unsupported(
            "monitor proof has unreachable original states",
        ));
    }
    Ok(proof)
}

/// Resolve the single non-alias origin of a reachable Move/Phi graph.
/// Mutually recursive Phis are legitimate loop-carried aliases only if every
/// external input reaches the same origin. Unanchored cycles and distinct
/// origins remain unproved; register numbers never establish object identity.
fn same_identity(ir: &MethodIr<SsaForm>, acquired: ValueId, released: ValueId) -> bool {
    // A selected Phi is itself a stable SSA identity even if its incoming
    // objects differ. Strip only moves, then use that selected value as an
    // anchor; do not conflate other Phis with the same set of input objects.
    let mut anchor = acquired;
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(anchor) {
            return false;
        }
        let Ok(value) = ir.value(anchor) else {
            return false;
        };
        match ir.instruction(value.def()) {
            Ok(ManagedInsn::Move { result, value }) if *result == anchor => anchor = *value,
            Ok(_) => break,
            Err(_) => return false,
        }
    }
    if alias_identity_anchored(ir, released, Some(anchor)) == Some(anchor) {
        return true;
    }
    alias_identity(ir, acquired).is_some_and(|origin| alias_identity(ir, released) == Some(origin))
}

fn alias_identity(ir: &MethodIr<SsaForm>, value: ValueId) -> Option<ValueId> {
    alias_identity_anchored(ir, value, None)
}

fn alias_identity_anchored(
    ir: &MethodIr<SsaForm>,
    value: ValueId,
    anchor: Option<ValueId>,
) -> Option<ValueId> {
    let mut pending = vec![value];
    let mut visited = HashSet::new();
    let mut origin = None;
    while let Some(value) = pending.pop() {
        if !visited.insert(value) {
            continue;
        }
        let definition = ir.value(value).ok()?.def();
        if Some(value) == anchor {
            if origin.is_some_and(|previous| previous != value) {
                return None;
            }
            origin = Some(value);
            continue;
        }
        match ir.instruction(definition).ok()? {
            ManagedInsn::Move {
                result,
                value: input,
            } if *result == value => {
                pending.push(*input);
            }
            ManagedInsn::Phi { result, inputs } if *result == value => {
                if inputs.is_empty() {
                    return None;
                }
                pending.extend(inputs.iter().map(|(_, input)| *input));
            }
            _ => {
                if origin.is_some_and(|previous| previous != value) {
                    return None;
                }
                origin = Some(value);
            }
        }
    }
    origin
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod identity_tests {
    use super::*;
    use decomp_core::cfg::Cfg;
    use managed_ir::{
        method::{InsnArena, ValueArena},
        refs::ClassRef,
        types::ManagedType,
        value::Value,
    };

    fn graph(distinct: bool, anchored: bool) -> (MethodIr<SsaForm>, ValueId, ValueId) {
        let mut insns = InsnArena::with_key();
        let root_def = insns.insert(ManagedInsn::Return { value: None });
        let other_def = insns.insert(ManagedInsn::Return { value: None });
        let a_def = insns.insert(ManagedInsn::Return { value: None });
        let b_def = insns.insert(ManagedInsn::Return { value: None });
        let mut values = ValueArena::with_key();
        let ty = ManagedType::Reference(ClassRef::new(0));
        let origin = values.insert(Value::new(root_def, ty.clone()));
        let other = values.insert(Value::new(other_def, ty.clone()));
        let a = values.insert(Value::new(a_def, ty.clone()));
        let b = values.insert(Value::new(b_def, ty));
        let mut cfg = Cfg::new(root_def);
        let ab = cfg.add_block(a_def);
        let bb = cfg.add_block(b_def);
        let start = cfg
            .add_edge(cfg.entry(), ab, EdgeKind::Unconditional)
            .unwrap();
        let back = cfg.add_edge(bb, ab, EdgeKind::Unconditional).unwrap();
        let forward = cfg.add_edge(ab, bb, EdgeKind::Unconditional).unwrap();
        let mut inputs = vec![(back, b)];
        if anchored {
            inputs.push((start, origin));
        }
        insns[a_def] = ManagedInsn::Phi { result: a, inputs };
        insns[b_def] = ManagedInsn::Phi {
            result: b,
            inputs: if distinct {
                vec![(forward, a), (start, other)]
            } else {
                vec![(forward, a)]
            },
        };
        (
            MethodIr::<SsaForm>::from_parts(cfg, insns, values),
            origin,
            b,
        )
    }

    #[test]
    fn mutually_recursive_phi_aliases_have_one_original_identity() {
        let (ir, origin, alias) = graph(false, true);
        assert_eq!(alias_identity(&ir, alias), Some(origin));
    }

    #[test]
    fn selected_multi_origin_phi_retains_its_own_monitor_identity() {
        let (ir, other, selected) = graph(true, true);
        assert_eq!(alias_identity(&ir, selected), None);
        assert!(same_identity(&ir, selected, selected));
        assert!(!same_identity(&ir, selected, other));
        assert!(!same_identity(&ir, other, selected));
    }

    #[test]
    fn cyclic_aliases_cannot_hide_another_object_or_invent_an_origin() {
        for (distinct, anchored) in [(true, true), (false, false)] {
            let (ir, _, alias) = graph(distinct, anchored);
            assert_eq!(alias_identity(&ir, alias), None);
        }
    }
}
