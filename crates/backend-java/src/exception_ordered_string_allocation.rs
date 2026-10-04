//! Original-order resolving-string allocation/initializer transfer proof.
//! This capability never attests pure gaps or permits hoisting. Each
//! selected handler must reach the same original effect tail with the same
//! observed caught identity and selected SSA values. Resolving strings stay
//! after NEW and require positive exclusion from newly enclosing catches.
use super::*;
use crate::type_compatibility::Compatibility;
use decomp_core::ids::EdgeId;
use managed_ir::{effects::EffectFlags, ops::Constant};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Canonical {
    Caught,
    Constant(Constant),
    Available(ValueId),
}
#[derive(Clone, Debug)]
pub(super) struct Witness {
    // Retain both original tables and exact edge selections; this is never a
    // union scope or a name-only equivalence decision.
    scopes: (usize, usize),
    transfers: Vec<(EdgeId, EdgeId, BlockId, HashMap<ValueId, Canonical>)>,
}
impl Witness {
    pub(super) fn attested(&self) -> bool {
        self.scopes.0 != self.scopes.1 && !self.transfers.is_empty()
    }
}
fn primitive(c: Constant) -> bool {
    !matches!(c, Constant::String(_) | Constant::ClassLiteral(_))
}
fn block(ir: &MethodIr<SsaForm>, id: InsnId) -> Option<BlockId> {
    let mut found = ir
        .cfg()
        .block_ids()
        .filter(|b| ir.cfg().block(*b).is_ok_and(|n| *n.payload() == id));
    let first = found.next()?;
    found.next().is_none().then_some(first)
}
fn offset(metadata: &[&dyn RegionMetadata], id: InsnId) -> Option<u32> {
    let mut offsets = metadata
        .iter()
        .filter_map(|m| m.original_scope()?.offsets.get(&id).copied());
    let first = offsets.next()?;
    offsets.all(|o| o == first).then_some(first)
}
fn scope<'a>(
    ir: &MethodIr<SsaForm>,
    metadata: &'a [&dyn RegionMetadata],
    id: InsnId,
) -> Option<&'a dyn RegionMetadata> {
    let mut found = metadata
        .iter()
        .copied()
        .filter(|m| m.original_scope().is_some_and(|s| s.contains(id)));
    let md = found.next()?;
    if found.next().is_some() {
        return None;
    }
    let original = md.original_scope()?;
    if !md.region_scope().protected.contains(&block(ir, id)?)
        || original.catches.len() != md.region_scope().catches.len()
        || original
            .catches
            .iter()
            .zip(&md.region_scope().catches)
            .any(|((key, address), c)| {
                key != &c.key || !handler_address(ir, c.entry, original, *address)
            })
    {
        return None;
    }
    Some(md)
}
fn next(ir: &MethodIr<SsaForm>, b: BlockId) -> Option<(EdgeId, BlockId)> {
    let out = ir.cfg().block(b).ok()?.outgoing();
    let [id] = out else {
        return None;
    };
    let e = ir.cfg().edge(*id).ok()?;
    (e.kind() == &EdgeKind::Unconditional).then_some((*id, e.target()))
}
/// Cut every successful definition edge. Any remaining route to NEW proves
/// that the value may be absent there, including a throwing producer's handler.
fn available(ir: &MethodIr<SsaForm>, at: BlockId, value: ValueId) -> bool {
    let Ok(v) = ir.value(value) else {
        return false;
    };
    let Ok(insn) = ir.instruction(v.def()) else {
        return false;
    };
    if matches!(insn, ManagedInsn::Opaque { family_tag: 0x8001 | 0x8003, reads, writes, effects } if reads.is_empty() && writes.as_slice() == [value] && *effects == EffectFlags::NONE)
    {
        // Declaration binding and concrete source typing remain independently
        // mandatory in the complete exception renderer.
        return true;
    }
    let Some(def) = block(ir, v.def()) else {
        return false;
    };
    if !super::super::defined(insn).contains(&value) {
        return false;
    }
    let reachable = |cut: bool| {
        let mut todo = vec![ir.cfg().entry()];
        let mut seen = HashSet::new();
        while let Some(b) = todo.pop() {
            if !seen.insert(b) {
                continue;
            }
            if b == at {
                return true;
            }
            let Ok(n) = ir.cfg().block(b) else {
                return true;
            };
            for id in n.outgoing() {
                let Ok(e) = ir.cfg().edge(*id) else {
                    return true;
                };
                if !(cut && b == def && !matches!(e.kind(), EdgeKind::Exception(_))) {
                    todo.push(e.target());
                }
            }
        }
        false
    };
    reachable(false) && !reachable(true)
}
fn canonical(
    ir: &MethodIr<SsaForm>,
    at: BlockId,
    state: &HashMap<ValueId, Canonical>,
    mut value: ValueId,
) -> Option<Canonical> {
    let mut seen = HashSet::new();
    while seen.insert(value) {
        if let Some(c) = state.get(&value) {
            return Some(c.clone());
        }
        if !available(ir, at, value) {
            return None;
        }
        let v = ir.value(value).ok()?;
        match ir.instruction(v.def()).ok()? {
            ManagedInsn::Move { value: source, .. } => value = *source,
            ManagedInsn::Const { value: c, .. } if primitive(*c) => {
                return Some(Canonical::Constant(*c));
            }
            _ => return Some(Canonical::Available(value)),
        }
    }
    None
}
struct Adapter {
    entry_edge: EdgeId,
    tail: BlockId,
    path: HashSet<BlockId>,
    state: HashMap<ValueId, Canonical>,
    joins: HashSet<ValueId>,
}
fn adapter(
    ir: &MethodIr<SsaForm>,
    metadata: &[&dyn RegionMetadata],
    at: BlockId,
    edge: EdgeId,
    mut state: HashMap<ValueId, Canonical>,
) -> Option<Adapter> {
    let mut b = ir.cfg().edge(edge).ok()?.target();
    if ir.cfg().block(b).ok()?.incoming().iter().any(|e| {
        ir.cfg()
            .edge(*e)
            .is_ok_and(|e| !matches!(e.kind(), EdgeKind::Exception(_)))
    }) {
        return None;
    }
    let mut incoming = edge;
    let mut phi_edge = None;
    let mut phi_inputs = HashSet::new();
    let mut phi_snapshot = HashMap::new();
    let mut path = HashSet::new();
    let mut joins = HashSet::new();
    let mut captured = false;
    loop {
        if path.len() >= 64 || !path.insert(b) {
            return None;
        }
        let n = ir.cfg().block(b).ok()?;
        let id = *n.payload();
        let insn = ir.instruction(id).ok()?;
        if super::super::defined(insn)
            .iter()
            .any(|v| ir.value(*v).ok().is_none_or(|v| v.def() != id))
        {
            return None;
        }
        match insn {
            ManagedInsn::Phi { result, inputs } => {
                if phi_edge.is_none() || n.incoming().len() != 1 {
                    phi_edge = Some(incoming);
                    phi_inputs = n.incoming().iter().copied().collect();
                    phi_snapshot = state.clone();
                }
                let selection = phi_edge?;
                let source = inputs
                    .iter()
                    .filter(|(e, _)| *e == selection)
                    .collect::<Vec<_>>();
                let [(_, value)] = source.as_slice() else {
                    return None;
                };
                if inputs.len() != phi_inputs.len()
                    || inputs.iter().map(|(e, _)| *e).collect::<HashSet<_>>() != phi_inputs
                {
                    return None;
                }
                let value = canonical(ir, at, &phi_snapshot, *value)?;
                state.insert(*result, value);
                joins.insert(*result);
            }
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads,
                writes,
                effects,
            } if !captured
                && reads.is_empty()
                && writes.len() == 1
                && matches!(*effects, EffectFlags::NONE | EffectFlags::MAY_THROW) =>
            {
                captured = true;
                phi_edge = None;
                state.insert(writes[0], Canonical::Caught);
            }
            ManagedInsn::Move { result, value } if offset(metadata, id).is_some() => {
                phi_edge = None;
                let value = canonical(ir, at, &state, *value)?;
                state.insert(*result, value);
            }
            ManagedInsn::Branch {
                condition: None,
                then_edge,
                else_edge: None,
            } if offset(metadata, id).is_some() && next(ir, b)?.0 == *then_edge => {
                phi_edge = None;
            }
            _ if captured || offset(metadata, id).is_some() => {
                return Some(Adapter {
                    entry_edge: edge,
                    tail: b,
                    path,
                    state,
                    joins,
                });
            }
            _ => return None,
        }
        let (edge, target) = next(ir, b)?;
        incoming = edge;
        b = target;
    }
}
fn exceptional(ir: &MethodIr<SsaForm>, b: BlockId, md: &dyn RegionMetadata) -> Option<Vec<EdgeId>> {
    let edges = ir
        .cfg()
        .block(b)
        .ok()?
        .outgoing()
        .iter()
        .filter_map(|id| {
            ir.cfg()
                .edge(*id)
                .ok()
                .filter(|e| matches!(e.kind(), EdgeKind::Exception(_)))
                .map(|_| *id)
        })
        .collect::<Vec<_>>();
    if edges.len() != md.region_scope().catches.len() || edges.is_empty() {
        return None;
    }
    md.region_scope()
        .catches
        .iter()
        .map(|catch| {
            let selected = edges
                .iter()
                .copied()
                .filter(|id| {
                    ir.cfg().edge(*id).is_ok_and(|e| {
                        e.target() == catch.entry
                            && e.kind() == &EdgeKind::Exception(catch.key.clone())
                    })
                })
                .collect::<Vec<_>>();
            let [e] = selected.as_slice() else {
                return None;
            };
            Some(*e)
        })
        .collect()
}
fn equivalent(
    ir: &MethodIr<SsaForm>,
    at: BlockId,
    a: &Adapter,
    b: &Adapter,
) -> Option<HashMap<ValueId, Canonical>> {
    if a.tail != b.tail {
        return None;
    }
    let common = a
        .path
        .intersection(&b.path)
        .copied()
        .collect::<HashSet<_>>();
    let mut live = a
        .joins
        .intersection(&b.joins)
        .copied()
        .collect::<HashSet<_>>();
    // Require each shared join transfer, including unused destinations. Missing
    // selections and a changed but apparently dead snapshot remain refusals.
    for node in &common {
        if let ManagedInsn::Phi { result, .. } = ir
            .instruction(*ir.cfg().block(*node).ok()?.payload())
            .ok()?
        {
            live.insert(*result);
        }
    }
    let mut todo = vec![a.tail];
    let mut tail = HashSet::new();
    while let Some(node) = todo.pop() {
        if !tail.insert(node) {
            continue;
        }
        if tail.len() > 4096 {
            return None;
        }
        for edge in ir.cfg().block(node).ok()?.outgoing() {
            todo.push(ir.cfg().edge(*edge).ok()?.target());
        }
    }
    let defs = tail
        .iter()
        .map(|node| ir.cfg().block(*node).map(|n| *n.payload()))
        .collect::<Result<HashSet<_>, _>>()
        .ok()?;
    for node in &tail {
        let insn = ir
            .instruction(*ir.cfg().block(*node).ok()?.payload())
            .ok()?;
        // Only original edges reachable from the common handler tail select
        // these Phi inputs; unrelated successful formatting paths do not.
        let used = match insn {
            ManagedInsn::Phi { inputs, .. } => {
                let mut selected = Vec::new();
                for (edge, value) in inputs {
                    let edge = ir.cfg().edge(*edge).ok()?;
                    if tail.contains(&edge.source()) {
                        selected.push(*value);
                    }
                }
                selected
            }
            _ => reads(insn),
        };
        for v in used {
            if !defs.contains(&ir.value(v).ok()?.def()) {
                live.insert(v);
            }
        }
    }
    let mut values = HashMap::new();
    for value in live {
        let left = canonical(ir, at, &a.state, value)?;
        let right = canonical(ir, at, &b.state, value)?;
        if left != right {
            return None;
        }
        values.insert(value, left);
    }
    Some(values)
}
fn excludes_string_errors(md: &dyn RegionMetadata) -> bool {
    let error = JavaType::Class("java.lang.Error".into());
    md.region_scope().catches.iter().all(|c| {
        !md.is_catch_all(&c.key)
            && md.catch_type(&c.key).is_some_and(|t| {
                md.hierarchy().assignable(&error, &t) == Compatibility::Incompatible
                    && md.hierarchy().assignable(&t, &error) == Compatibility::Incompatible
                    && md
                        .hierarchy()
                        .assignable(&t, &JavaType::Class("java.lang.Throwable".into()))
                        == Compatibility::Assignable
            })
    })
}
pub(super) fn prove(
    ir: &MethodIr<SsaForm>,
    metadata: &[&dyn RegionMetadata],
    resolver: &impl JavaResolver,
) -> HashMap<Vec<InsnId>, Witness> {
    let mut output = HashMap::new();
    for start in ir.cfg().block_ids() {
        let Ok(n) = ir.cfg().block(start) else {
            continue;
        };
        let allocation = *n.payload();
        let Ok(ManagedInsn::NewInstance { result, class }) = ir.instruction(allocation) else {
            continue;
        };
        let Some(left) = scope(ir, metadata, allocation) else {
            continue;
        };
        let mut chain = vec![allocation];
        let mut aliases = HashSet::from([*result]);
        let mut gap_state = HashMap::new();
        let mut resolving_string = false;
        let mut current = start;
        let mut seen = HashSet::new();
        while seen.insert(current) && seen.len() <= 64 {
            let normal = ir
                .cfg()
                .block(current)
                .ok()
                .into_iter()
                .flat_map(|n| n.outgoing())
                .filter_map(|e| ir.cfg().edge(*e).ok())
                .filter(|e| !matches!(e.kind(), EdgeKind::Exception(_)))
                .collect::<Vec<_>>();
            let [edge] = normal.as_slice() else {
                break;
            };
            if edge.kind() != &EdgeKind::Unconditional {
                break;
            }
            current = edge.target();
            let Ok(n) = ir.cfg().block(current) else {
                break;
            };
            if n.incoming().len() != 1 {
                break;
            }
            let id = *n.payload();
            chain.push(id);
            if ir.instruction(id).ok().is_none_or(|insn| {
                super::super::defined(insn)
                    .iter()
                    .any(|v| ir.value(*v).ok().is_none_or(|v| v.def() != id))
            }) {
                break;
            }
            match ir.instruction(id) {
                Ok(ManagedInsn::Move { result, value }) if aliases.contains(value) => {
                    if !n.outgoing().iter().all(|e| {
                        ir.cfg()
                            .edge(*e)
                            .is_ok_and(|e| !matches!(e.kind(), EdgeKind::Exception(_)))
                    }) {
                        break;
                    }
                    aliases.insert(*result);
                }
                Ok(ManagedInsn::Const { result, value })
                    if primitive(*value)
                        // Raw floating bits may lower through a library call;
                        // that requires a separate initialization/domain proof.
                        && !matches!(value, Constant::FloatBits(_) | Constant::DoubleBits(_))
                        && offset(metadata, id).is_some()
                        && next(ir, current).is_some() =>
                {
                    gap_state.insert(*result, Canonical::Constant(*value));
                }
                Ok(ManagedInsn::Const {
                    result,
                    value: Constant::String(reference),
                }) if resolver.string(*reference).is_some()
                    && offset(metadata, id).is_some()
                    && next(ir, current).is_some()
                    && excludes_string_errors(left)
                    && metadata
                        .iter()
                        .filter(|m| m.original_scope().is_some_and(|s| s.contains(id)))
                        .all(|m| excludes_string_errors(*m)) =>
                {
                    resolving_string = true;
                    gap_state.insert(*result, Canonical::Constant(Constant::String(*reference)));
                }
                Ok(ManagedInsn::Move { result, value })
                    if offset(metadata, id).is_some()
                        && available(ir, start, *value)
                        && next(ir, current).is_some() =>
                {
                    let Some(c) = canonical(ir, start, &HashMap::new(), *value) else {
                        break;
                    };
                    gap_state.insert(*result, c);
                }
                Ok(ManagedInsn::Invoke {
                    kind: InvokeKind::Special,
                    result: None,
                    method,
                    args,
                }) if args.first().is_some_and(|v| aliases.contains(v))
                    && !args.iter().skip(1).any(|v| aliases.contains(v)) =>
                {
                    let Some(target) = resolver.method(*method) else {
                        break;
                    };
                    if target.name != "<init>"
                        || target.is_static
                        || target.return_type != JavaType::Void
                        || resolver.class(*class).as_deref() != Some(&target.owner)
                    {
                        break;
                    }
                    let Some(right) = scope(ir, metadata, id) else {
                        break;
                    };
                    let (Some(ls), Some(rs)) = (left.original_scope(), right.original_scope())
                    else {
                        break;
                    };
                    if ls.ordinal == rs.ordinal
                        || !resolving_string
                        || !excludes_string_errors(right)
                    {
                        break;
                    }
                    let (Some(le), Some(re)) = (
                        exceptional(ir, start, left),
                        exceptional(ir, current, right),
                    ) else {
                        break;
                    };
                    if le.len() != re.len() {
                        break;
                    }
                    let mut transfers = vec![];
                    for ((l, r), (lc, rc)) in le.iter().zip(&re).zip(
                        left.region_scope()
                            .catches
                            .iter()
                            .zip(&right.region_scope().catches),
                    ) {
                        let (Some(lt), Some(rt)) =
                            (left.catch_type(&lc.key), right.catch_type(&rc.key))
                        else {
                            break;
                        };
                        if lt != rt
                            || left.is_catch_all(&lc.key) != right.is_catch_all(&rc.key)
                            || left
                                .hierarchy()
                                .assignable(&lt, &JavaType::Class("java.lang.Throwable".into()))
                                != Compatibility::Assignable
                            || right
                                .hierarchy()
                                .assignable(&rt, &JavaType::Class("java.lang.Throwable".into()))
                                != Compatibility::Assignable
                        {
                            break;
                        }
                        let (Some(a), Some(b)) = (
                            adapter(ir, metadata, start, *l, HashMap::new()),
                            adapter(ir, metadata, start, *r, gap_state.clone()),
                        ) else {
                            break;
                        };
                        let Some(values) = equivalent(ir, start, &a, &b) else {
                            break;
                        };
                        transfers.push((a.entry_edge, b.entry_edge, a.tail, values));
                    }
                    if transfers.len() == le.len() {
                        output.insert(
                            chain.clone(),
                            Witness {
                                scopes: (ls.ordinal, rs.ordinal),
                                transfers,
                            },
                        );
                    }
                    break;
                }
                _ => break,
            }
        }
    }
    output
}

#[cfg(test)]
#[path = "exception_ordered_literal_scope_tests.rs"]
mod tests;
