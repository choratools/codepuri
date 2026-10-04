//! Exact typed handler inputs for source-only checked-throws analysis.
use super::ThrowsHierarchy;
use decomp_core::cfg::EdgeKind;
use dex_decompiler::ssa::SsaMethod;
use dex_format::code::CodeItem;
use managed_ir::{
    effects::EffectFlags,
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
    refs::ClassRef,
    value::{InsnId, ValueId},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct HandlerTypes {
    captures: BTreeMap<ValueId, String>,
    definitions: BTreeSet<InsnId>,
    origins: BTreeMap<ValueId, Vec<Origin>>,
}
#[derive(Clone, Debug)]
pub(super) struct Origin {
    pub(super) source: InsnId,
    pub(super) caught: Option<String>,
    pub(super) excluded: Vec<String>,
}
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
enum HandlerKind {
    Typed(u32),
    All,
}
impl HandlerTypes {
    pub(super) fn collect(
        ssa: &SsaMethod,
        code: &CodeItem,
        context: &impl ThrowsHierarchy,
    ) -> Self {
        Self::from_ir(ssa.ir(), code, |id| ssa.offset(id), context)
    }
    fn from_ir(
        ir: &MethodIr<SsaForm>,
        code: &CodeItem,
        offset: impl Fn(InsnId) -> Option<u32>,
        context: &impl ThrowsHierarchy,
    ) -> Self {
        let mut result = Self::default();
        if code.tries.is_empty() {
            return result;
        }
        // Shared entries retain a finite set of genuine original catch kinds.
        // Only a singleton typed set can supply an exact nominal thrown type.
        let mut types = BTreeMap::<u32, BTreeSet<HandlerKind>>::new();
        let mut ends = Vec::new();
        for range in &code.tries {
            let Some(end) = range.start_addr.checked_add(u32::from(range.insn_count)) else {
                return result;
            };
            if end <= range.start_addr
                || end > code.insns_size
                || ends
                    .iter()
                    .any(|(start, previous_end)| range.start_addr < *previous_end && *start < end)
            {
                return result;
            }
            ends.push((range.start_addr, end));
            let tables: Vec<_> = code
                .handlers
                .iter()
                .filter(|h| h.relative_offset == u32::from(range.handler_off))
                .collect();
            let [table] = tables.as_slice() else {
                return result;
            };
            let mut unique_types = BTreeSet::new();
            for caught in &table.handlers {
                let Some(name) = context.class(ClassRef::new(caught.type_idx)) else {
                    return result;
                };
                if caught.address >= code.insns_size
                    || !unique_types.insert(caught.type_idx)
                    || context.exception_kind(&name).is_none()
                {
                    return result;
                }
                types
                    .entry(caught.address)
                    .or_default()
                    .insert(HandlerKind::Typed(caught.type_idx));
            }
            if let Some(address) = table.catch_all_addr {
                if address >= code.insns_size
                    || context.exception_kind("java.lang.Throwable").is_none()
                {
                    return result;
                }
                types.entry(address).or_default().insert(HandlerKind::All);
            }
        }
        for (address, kinds) in types {
            // The published SSA offset identifies the original instruction.
            // Read only that F11x code unit; replaying the whole decoder for
            // every source contract would duplicate the cached frontend work.
            let raw = usize::try_from(address)
                .ok()
                .and_then(|n| n.checked_mul(2))
                .and_then(|start| code.insns.get(start..start.checked_add(2)?))
                .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]));
            if address >= code.insns_size
                || !raw.is_some_and(|unit| unit & 0xff == 0x0d && unit >> 8 < code.registers_size)
            {
                continue;
            }
            let moves: Vec<_> = ir
                .instructions()
                .iter()
                .filter(|(id, _)| offset(*id) == Some(address))
                .collect();
            let [
                (
                    move_id,
                    ManagedInsn::Move {
                        result: moved,
                        value: capture,
                    },
                ),
            ] = moves.as_slice()
            else {
                continue;
            };
            let Ok(record) = ir.value(*capture) else {
                continue;
            };
            let definition = record.def();
            if offset(definition).is_some()
                || !ir.value(*moved).is_ok_and(|v| v.def() == *move_id)
                || !matches!(ir.instruction(definition), Ok(ManagedInsn::Opaque { family_tag: 0x8002, reads, writes, effects })
                    if reads.is_empty() && writes.as_slice() == [*capture] && *effects == EffectFlags::MAY_THROW)
                || ir.instructions().values().filter(|i| matches!(i, ManagedInsn::Opaque { family_tag: 0x8002, writes, .. } if writes.contains(capture))).count() != 1
                || ir.instructions().iter().filter(|(_, i)| reads(i, *capture)).map(|(id, _)| id).collect::<Vec<_>>() != [*move_id]
            { continue; }
            let capture_blocks: Vec<_> = ir
                .cfg()
                .block_ids()
                .filter(|b| {
                    ir.cfg()
                        .block(*b)
                        .is_ok_and(|block| *block.payload() == definition)
                })
                .collect();
            let move_blocks: Vec<_> = ir
                .cfg()
                .block_ids()
                .filter(|b| {
                    ir.cfg()
                        .block(*b)
                        .is_ok_and(|block| *block.payload() == *move_id)
                })
                .collect();
            let ([capture_block], [move_block]) =
                (capture_blocks.as_slice(), move_blocks.as_slice())
            else {
                continue;
            };
            // SSA can place pure Phi/move/comparison commits around capture.
            // Walk only the unique synthetic chain leading to the original
            // move-exception; its head must still have exclusively exceptional
            // entries. A normal join, another opaque input, or a loop rejects.
            let mut head = *move_block;
            let mut seen = BTreeSet::new();
            let mut saw_capture = false;
            let incoming = loop {
                if !seen.insert(head) {
                    break None;
                }
                let Ok(node) = ir.cfg().block(head) else {
                    break None;
                };
                if !node.incoming().is_empty()
                    && node.incoming().iter().all(|edge| {
                        ir.cfg()
                            .edge(*edge)
                            .is_ok_and(|e| matches!(e.kind(), EdgeKind::Exception(_)))
                    })
                {
                    break saw_capture.then(|| node.incoming().to_vec());
                }
                let [edge_id] = node.incoming() else {
                    break None;
                };
                let Ok(edge) = ir.cfg().edge(*edge_id) else {
                    break None;
                };
                if edge.kind() != &EdgeKind::Unconditional {
                    break None;
                }
                let previous = edge.source();
                let Ok(previous_node) = ir.cfg().block(previous) else {
                    break None;
                };
                let id = *previous_node.payload();
                if previous_node.outgoing() != [*edge_id] || offset(id).is_some() {
                    break None;
                }
                if previous == *capture_block {
                    saw_capture = true;
                } else if !matches!(ir.instruction(id), Ok(ManagedInsn::Phi { inputs, .. }) if !inputs.is_empty())
                    && !matches!(
                        ir.instruction(id),
                        Ok(ManagedInsn::Move { .. }
                            | ManagedInsn::Compare { .. }
                            | ManagedInsn::OrderingCompare { .. })
                    )
                {
                    break None;
                }
                head = previous;
            };
            let Some(incoming) = incoming else {
                continue;
            };
            let mut observed = BTreeMap::<InsnId, BTreeSet<HandlerKind>>::new();
            let origins = incoming
                .iter()
                .map(|edge| {
                    let edge = ir.cfg().edge(*edge).ok()?;
                    let EdgeKind::Exception(label) = edge.kind() else {
                        return None;
                    };
                    let kind = *kinds.iter().find(|kind| match kind {
                        HandlerKind::Typed(t) => label == &t.to_string(),
                        HandlerKind::All => label.is_empty(),
                    })?;
                    let source = *ir.cfg().block(edge.source()).ok()?.payload();
                    if !observed.entry(source).or_default().insert(kind) {
                        return None;
                    }
                    let source_offset = offset(source)?;
                    let ranges: Vec<_> = code
                        .tries
                        .iter()
                        .filter(|range| {
                            range.start_addr <= source_offset
                                && source_offset < range.start_addr + u32::from(range.insn_count)
                        })
                        .collect();
                    let [range] = ranges.as_slice() else {
                        return None;
                    };
                    let tables: Vec<_> = code
                        .handlers
                        .iter()
                        .filter(|h| h.relative_offset == u32::from(range.handler_off))
                        .collect();
                    let [table] = tables.as_slice() else {
                        return None;
                    };
                    let preceding = match kind {
                        HandlerKind::Typed(t) => table
                            .handlers
                            .iter()
                            .position(|h| h.type_idx == t && h.address == address)?,
                        HandlerKind::All if table.catch_all_addr == Some(address) => {
                            table.handlers.len()
                        }
                        HandlerKind::All => return None,
                    };
                    let caught = match kind {
                        HandlerKind::Typed(t) => Some(context.class(ClassRef::new(t))?),
                        HandlerKind::All => None,
                    };
                    let excluded = table.handlers[..preceding]
                        .iter()
                        .map(|h| context.class(ClassRef::new(h.type_idx)))
                        .collect::<Option<Vec<_>>>()?;
                    Some(Origin {
                        source,
                        caught,
                        excluded,
                    })
                })
                .collect::<Option<Vec<_>>>();
            let Some(origins) = origins else {
                continue;
            };
            // Every observed predecessor must have all of the original edges
            // to this shared entry. A missing typed edge cannot erase a checked
            // rethrow domain. Also require every protected Invoke/Throw source
            // represented in this SSA to appear, even if all its edges vanished.
            let mut complete = true;
            for (source, insn) in ir.instructions() {
                let Some(source_offset) = offset(source) else {
                    continue;
                };
                let Some(range) = code.tries.iter().find(|range| {
                    range.start_addr <= source_offset
                        && source_offset < range.start_addr + u32::from(range.insn_count)
                }) else {
                    continue;
                };
                let Some(table) = code
                    .handlers
                    .iter()
                    .find(|h| h.relative_offset == u32::from(range.handler_off))
                else {
                    complete = false;
                    break;
                };
                let mut expected: BTreeSet<_> = table
                    .handlers
                    .iter()
                    .filter(|h| h.address == address)
                    .map(|h| HandlerKind::Typed(h.type_idx))
                    .collect();
                if table.catch_all_addr == Some(address) {
                    expected.insert(HandlerKind::All);
                }
                if expected.is_empty() {
                    continue;
                }
                if let Some(actual) = observed.get(&source) {
                    if actual != &expected {
                        complete = false;
                        break;
                    }
                } else if matches!(insn, ManagedInsn::Invoke { .. } | ManagedInsn::Throw { .. }) {
                    complete = false;
                    break;
                }
            }
            if !complete {
                continue;
            }
            if let [HandlerKind::Typed(t)] = kinds.iter().copied().collect::<Vec<_>>().as_slice() {
                let Some(name) = context.class(ClassRef::new(*t)) else {
                    continue;
                };
                result.captures.insert(*capture, name);
            }
            result.origins.insert(*capture, origins);
            result.definitions.insert(definition);
        }
        result
    }
    pub(super) fn attests(&self, definition: InsnId) -> bool {
        self.definitions.contains(&definition)
    }

    /// Exact copies, including same-root phis, retain the actual caught object.
    /// A mixture with any other input or another capture has no single-root proof.
    pub(super) fn thrown_type<'a>(
        &'a self,
        ir: &MethodIr<SsaForm>,
        value: ValueId,
    ) -> Option<&'a str> {
        self.captures
            .get(&self.root(ir, value)?)
            .map(String::as_str)
    }
    /// Finite nominal domain for mixed caught objects and genuine method results.
    /// This proves a type, never the identity or precise rethrow domain of a catch.
    /// Unknown/opaque leaves and cycles remain closed. Null contributes only NPE.
    pub(super) fn nominal_thrown_types(
        &self,
        ir: &MethodIr<SsaForm>,
        value: ValueId,
        context: &impl ThrowsHierarchy,
    ) -> Option<BTreeSet<String>> {
        let mut nominal = BTreeSet::new();
        let mut active = BTreeSet::new();
        let mut done = BTreeSet::new();
        let mut pending = vec![(value, false)];
        while let Some((value, completed)) = pending.pop() {
            if completed {
                active.remove(&value);
                done.insert(value);
                continue;
            }
            if done.contains(&value) {
                continue;
            }
            if !active.insert(value) {
                return None;
            }
            pending.push((value, true));
            let definition = ir.instruction(ir.value(value).ok()?.def()).ok()?;
            let leaf = if let Some(name) = self.captures.get(&value) {
                Some(name.clone())
            } else if self.origins.get(&value).is_some_and(|origins| {
                !origins.is_empty() && origins.iter().any(|origin| origin.caught.is_none())
            }) {
                // A genuinely attested catch-all producer is a Throwable even
                // if ordinary field/copy constraints widened its SSA annotation
                // to Object. This is nominal evidence, not a precise rethrow.
                Some("java.lang.Throwable".into())
            } else {
                match definition {
                    ManagedInsn::Move {
                        result,
                        value: source,
                    } if *result == value => {
                        pending.push((*source, false));
                        None
                    }
                    ManagedInsn::Phi { result, inputs }
                        if *result == value && !inputs.is_empty() =>
                    {
                        pending.extend(inputs.iter().map(|(_, source)| (*source, false)));
                        None
                    }
                    ManagedInsn::Invoke {
                        result: Some(result),
                        method,
                        ..
                    } if *result == value => {
                        let backend_java::JavaType::Class(name) =
                            context.method(*method)?.return_type
                        else {
                            return None;
                        };
                        Some(name)
                    }
                    ManagedInsn::NewInstance { result, class } if *result == value => {
                        Some(context.class(*class)?)
                    }
                    ManagedInsn::Cast { result, target, .. } if *result == value => {
                        let backend_java::JavaType::Class(name) =
                            context.instruction_type(*target)?
                        else {
                            return None;
                        };
                        Some(name)
                    }
                    ManagedInsn::Const {
                        result,
                        value: managed_ir::insn::Constant::Zero | managed_ir::insn::Constant::Null,
                    } if *result == value => Some("java.lang.NullPointerException".into()),
                    _ => return None,
                }
            };
            if let Some(name) = leaf {
                context.exception_kind(&name)?;
                nominal.insert(name);
            }
        }
        (!nominal.is_empty()).then_some(nominal)
    }
    pub(super) fn origins(&self, ir: &MethodIr<SsaForm>, value: ValueId) -> Option<Vec<Origin>> {
        Some(
            self.roots(ir, value)?
                .iter()
                .flat_map(|root| &self.origins[root])
                .cloned()
                .collect(),
        )
    }
    fn root(&self, ir: &MethodIr<SsaForm>, value: ValueId) -> Option<ValueId> {
        let roots = self.roots(ir, value)?;
        if roots.len() != 1 {
            return None;
        }
        roots.into_iter().next()
    }
    fn roots(&self, ir: &MethodIr<SsaForm>, value: ValueId) -> Option<BTreeSet<ValueId>> {
        let mut roots = BTreeSet::new();
        let mut active = BTreeSet::new();
        let mut done = BTreeSet::new();
        let mut pending = vec![(value, false)];
        while let Some((value, completed)) = pending.pop() {
            if completed {
                active.remove(&value);
                done.insert(value);
                continue;
            }
            if done.contains(&value) {
                continue;
            }
            if self.origins.contains_key(&value) {
                roots.insert(value);
                done.insert(value);
                continue;
            }
            if !active.insert(value) {
                return None;
            }
            pending.push((value, true));
            let definition = ir.instruction(ir.value(value).ok()?.def()).ok()?;
            match definition {
                ManagedInsn::Move {
                    result,
                    value: source,
                } if *result == value => pending.push((*source, false)),
                ManagedInsn::Phi { result, inputs } if *result == value && !inputs.is_empty() => {
                    pending.extend(inputs.iter().map(|(_, v)| (*v, false)))
                }
                _ => return None,
            }
        }
        (!roots.is_empty()).then_some(roots)
    }
}
fn reads(i: &ManagedInsn, wanted: ValueId) -> bool {
    use ManagedInsn::*;
    match i {
        Const { .. } | NewInstance { .. } => false,
        Move { value, .. }
        | Neg { value, .. }
        | Not { value, .. }
        | Cast { value, .. }
        | InstanceOf { value, .. }
        | Throw { value } => *value == wanted,
        Phi { inputs, .. } => inputs.iter().any(|(_, v)| *v == wanted),
        Arithmetic { lhs, rhs, .. }
        | Compare { lhs, rhs, .. }
        | OrderingCompare { lhs, rhs, .. } => *lhs == wanted || *rhs == wanted,
        FieldGet { object, .. } => *object == Some(wanted),
        FieldPut { object, value, .. } => *object == Some(wanted) || *value == wanted,
        ArrayGet { array, index, .. } => *array == wanted || *index == wanted,
        ArrayPut {
            array,
            index,
            value,
            ..
        } => [*array, *index, *value].contains(&wanted),
        ArrayLength { array, .. }
        | NewArray { length: array, .. }
        | MonitorEnter { object: array }
        | MonitorExit { object: array }
        | Switch { key: array, .. } => *array == wanted,
        FillArray { array, values } => *array == wanted || values.contains(&wanted),
        NewFilledArray { elements: args, .. }
        | Invoke { args, .. }
        | Opaque { reads: args, .. } => args.contains(&wanted),
        Branch { condition, .. } | Return { value: condition } => *condition == Some(wanted),
    }
}

#[cfg(test)]
#[path = "java_required_throws_handler_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "java_required_throws_shared_handler_tests.rs"]
mod shared_handler_tests;
