//! Bounded allocation argument trees. JLS 15.9.4 allocates before evaluating
//! arguments; their expression evaluation trace must exactly match the DEX gap.
use super::*;
#[path = "object_sequences.rs"]
mod sequences;

fn expression(insn: &ManagedInsn) -> Option<(ValueId, Vec<ValueId>)> {
    Some(match insn {
        ManagedInsn::FieldGet { result, object, .. } => (*result, object.iter().copied().collect()),
        ManagedInsn::Invoke {
            result: Some(result),
            // A selected super result is still an argument producer. Rendering
            // reuses the ordinary emitter's exact super binding and receiver
            // checks; it must never become a virtual call or move before NEW.
            kind:
                InvokeKind::Static
                | InvokeKind::Virtual
                | InvokeKind::Interface
                | InvokeKind::Super
                | InvokeKind::Special,
            args,
            ..
        } => (*result, args.clone()),
        ManagedInsn::Move { result, value } | ManagedInsn::Cast { result, value, .. } => {
            (*result, vec![*value])
        }
        ManagedInsn::Arithmetic {
            result, lhs, rhs, ..
        } => (*result, vec![*lhs, *rhs]),
        ManagedInsn::NewArray { result, length, .. } => (*result, vec![*length]),
        ManagedInsn::ArrayLength { result, array } => (*result, vec![*array]),
        ManagedInsn::ArrayGet {
            result,
            array,
            index,
        } => (*result, vec![*array, *index]),
        ManagedInsn::NewFilledArray {
            result: Some(result),
            elements,
            ..
        } => (*result, elements.clone()),
        // String retains the ordinary emitter's UTF-16/interner expression;
        // the planner only places that existing expression after allocation.
        ManagedInsn::Const {
            result,
            value:
                Constant::Zero
                | Constant::Boolean(_)
                | Constant::Int(_)
                | Constant::Long(_)
                | Constant::FloatBits(_)
                | Constant::DoubleBits(_)
                | Constant::Null
                | Constant::String(_)
                | Constant::ClassLiteral(_),
        } => (*result, vec![]),
        // Other expression families can have lowering that duplicates operands.
        _ => return None,
    })
}
// Integral/boolean/null literals introduce no allocation, linkage or exception event.
// Their source positions may commute across the event trace; string/class
// literals and raw floating bits remain exact: their existing lowering may
// resolve a class or call a wrapper's bit-conversion method.
fn eventful(insn: &ManagedInsn) -> bool {
    !matches!(
        insn,
        ManagedInsn::Const {
            value: Constant::Zero
                | Constant::Boolean(_)
                | Constant::Int(_)
                | Constant::Long(_)
                | Constant::Null,
            ..
        }
    )
}
pub(in crate::object_plan) fn reads(insn: &ManagedInsn) -> Vec<ValueId> {
    if let Some((_, reads)) = expression(insn) {
        return reads;
    }
    match insn {
        ManagedInsn::Phi { inputs, .. } => inputs.iter().map(|(_, v)| *v).collect(),
        ManagedInsn::Move { value, .. }
        | ManagedInsn::Neg { value, .. }
        | ManagedInsn::Not { value, .. }
        | ManagedInsn::Cast { value, .. }
        | ManagedInsn::InstanceOf { value, .. }
        | ManagedInsn::Throw { value } => vec![*value],
        ManagedInsn::Arithmetic { lhs, rhs, .. }
        | ManagedInsn::Compare { lhs, rhs, .. }
        | ManagedInsn::OrderingCompare { lhs, rhs, .. } => vec![*lhs, *rhs],
        ManagedInsn::FieldGet { object, .. } => object.iter().copied().collect(),
        ManagedInsn::FieldPut { object, value, .. } => {
            object.iter().copied().chain([*value]).collect()
        }
        ManagedInsn::ArrayGet { array, index, .. } => vec![*array, *index],
        ManagedInsn::ArrayPut {
            array,
            index,
            value,
        } => vec![*array, *index, *value],
        ManagedInsn::ArrayLength { array, .. } | ManagedInsn::FillArray { array, .. } => {
            let mut out = vec![*array];
            if let ManagedInsn::FillArray { values, .. } = insn {
                out.extend(values);
            }
            out
        }
        ManagedInsn::NewArray { length, .. } => vec![*length],
        ManagedInsn::NewFilledArray { elements, .. } => elements.clone(),
        ManagedInsn::Invoke { args, .. } => args.clone(),
        ManagedInsn::MonitorEnter { object } | ManagedInsn::MonitorExit { object } => vec![*object],
        ManagedInsn::Branch { condition, .. } => condition.iter().copied().collect(),
        ManagedInsn::Switch { key, .. } => vec![*key],
        ManagedInsn::Return { value } => value.iter().copied().collect(),
        ManagedInsn::Opaque { reads, .. } => reads.clone(),
        ManagedInsn::Const { .. } | ManagedInsn::NewInstance { .. } => vec![],
    }
}
/// Index only the folded values in one arena pass. Normalization metadata is
/// local to this immutable resolver context and resolved once per relevant
/// MethodRef. A missing/invalid descriptor retains raw reads conservatively.
/// Vectors retain multiplicity: two logical args are two uses, even if equal.
#[cfg(test)]
fn read_index<R: JavaResolver>(
    ir: &MethodIr<SsaForm>,
    wanted: &HashSet<ValueId>,
    resolver: &R,
) -> HashMap<ValueId, Vec<InsnId>> {
    let mut methods = HashMap::new();
    let mut index: HashMap<ValueId, Vec<InsnId>> = HashMap::new();
    for (id, insn) in ir.instructions() {
        let mut operands = reads(insn);
        if !operands.iter().any(|v| wanted.contains(v)) {
            continue;
        }
        if let ManagedInsn::Invoke { method, args, .. } = insn {
            let target = methods
                .entry(*method)
                .or_insert_with(|| resolver.method(*method));
            operands = target
                .as_ref()
                .and_then(|m| invoke_arguments::operands(m, args).ok())
                .unwrap_or_else(|| args.clone());
        }
        for value in operands.into_iter().filter(|v| wanted.contains(v)) {
            index.entry(value).or_default().push(id);
        }
    }
    index
}
// Occurrence nodes preserve each effectful operation. A trusted discarded
// append updates its private builder's expression head, not its SSA definition.
struct ArgumentNode {
    operands: Vec<InsnId>,
    allocation: Option<InsnId>,
}
fn insert_node(
    value: ValueId,
    id: InsnId,
    operands: &[ValueId],
    allocation: Option<InsnId>,
    gap: &mut HashMap<ValueId, InsnId>,
    nodes: &mut HashMap<InsnId, ArgumentNode>,
) {
    let operands = operands
        .iter()
        .filter_map(|v| gap.get(v).copied())
        .collect();
    nodes.insert(
        id,
        ArgumentNode {
            operands,
            allocation,
        },
    );
    gap.insert(value, id);
}
fn postorder(
    id: InsnId,
    nodes: &HashMap<InsnId, ArgumentNode>,
    seen: &mut HashSet<InsnId>,
    out: &mut Vec<InsnId>,
    captures: &HashSet<InsnId>,
) -> Result<(), Unsupported> {
    let node = nodes
        .get(&id)
        .ok_or_else(|| Unsupported::Instruction("missing argument occurrence".into()))?;
    if !seen.insert(id) {
        if captures.contains(&id) {
            return Ok(());
        }
        return Err(Unsupported::Instruction(
            "shared allocation argument expression".into(),
        ));
    }
    // Nested objects allocate before constructor arguments. Filled arrays use
    // their own instruction as the pre-element allocation marker (JLS 10.6).
    if let Some(allocation) = node.allocation {
        out.push(allocation);
    }
    for operand in &node.operands {
        postorder(*operand, nodes, seen, out, captures)?;
    }
    if node.allocation != Some(id) {
        out.push(id);
    }
    Ok(())
}
impl<R: JavaResolver> Emitter<'_, R> {
    fn literal_capture_type_preserved(
        &self,
        value: ValueId,
        consumer: InsnId,
    ) -> Result<bool, Unsupported> {
        let actual = self.ty(value)?;
        let insn = self
            .ir
            .instruction(consumer)
            .map_err(|e| Unsupported::Instruction(e.to_string()))?;
        let expected = match insn {
            ManagedInsn::Invoke { method, args, .. } => {
                let Some(m) = self.resolver.method(*method) else {
                    return Ok(false);
                };
                let operands = invoke_arguments::operands(&m, args)?;
                let types = if m.is_static {
                    m.parameters
                } else {
                    std::iter::once(JavaType::Class(m.owner))
                        .chain(m.parameters)
                        .collect()
                };
                operands
                    .iter()
                    .zip(types)
                    .find_map(|(operand, ty)| (*operand == value).then_some(ty))
            }
            ManagedInsn::NewArray { length, .. } if *length == value => Some(JavaType::Int),
            _ => None,
        };
        // Operand lowering can replace a raw zero/narrow literal with a
        // context-specific literal. Such substitution would discard the
        // capture assignment and leave later slot reads uninitialized.
        Ok(expected.as_ref() == Some(&actual))
    }

    pub(crate) fn after_initializer(
        &self,
        initializer: InsnId,
    ) -> Result<InstructionAvailability, Unsupported> {
        if let Some(order) = self.linear_object_order() {
            return Ok(InstructionAvailability::Linear {
                start: order
                    .positions
                    .get(&initializer)
                    .copied()
                    .unwrap_or(order.instructions.len()),
                order: order.clone(),
            });
        }
        if let Some(cached) = self.object_planning.success_cuts.borrow().get(&initializer) {
            return cached.clone().map(InstructionAvailability::SharedGraph);
        }
        // Avoid a second whole-CFG analysis for one-off/small query batches.
        // Retain at most four explicit sets, then promote repeated queries to
        // one success-dominance snapshot with O(1) membership.
        if (self.object_planning.success_dominance.get().is_some()
            || self.object_planning.success_cuts.borrow().len() >= 4)
            && let Some(index) = self.object_success_dominance()
        {
            self.object_planning.success_cuts.borrow_mut().clear();
            return Ok(InstructionAvailability::Dominance {
                index: index.clone(),
                cuts: [Some(initializer), None],
            });
        }
        let result = self
            .after_initializer_traversal(initializer)
            .map(std::rc::Rc::new);
        if self.object_planning.success_cuts.borrow().len() < 4 {
            self.object_planning
                .success_cuts
                .borrow_mut()
                .insert(initializer, result.clone());
        }
        result.map(InstructionAvailability::SharedGraph)
    }

    fn after_initializer_traversal(
        &self,
        initializer: InsnId,
    ) -> Result<HashSet<InsnId>, Unsupported> {
        let reject = || Unsupported::Instruction("allocation capture availability".into());
        // A use is available only if every CFG path to it passes the successful
        // initializer. Traversal includes exceptional edges and is deliberately
        // conservative for Phi inputs and off-CFG consumers.
        let mut without = HashSet::new();
        let mut visited = HashSet::new();
        let mut pending = vec![self.ir.cfg().entry()];
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            let block = self.ir.cfg().block(id).map_err(|_| reject())?;
            if *block.payload() == initializer {
                // A throwing initializer does not commit its object identity.
                // Its exceptional successors remain reachable without success.
                for edge in block.outgoing() {
                    let edge = self.ir.cfg().edge(*edge).map_err(|_| reject())?;
                    if matches!(edge.kind(), EdgeKind::Exception(_)) {
                        pending.push(edge.target());
                    }
                }
                continue;
            }
            without.insert(*block.payload());
            for edge in block.outgoing() {
                pending.push(self.ir.cfg().edge(*edge).map_err(|_| reject())?.target());
            }
        }
        let mut reachable = HashSet::new();
        visited.clear();
        pending.push(self.ir.cfg().entry());
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
                continue;
            }
            let block = self.ir.cfg().block(id).map_err(|_| reject())?;
            reachable.insert(*block.payload());
            for edge in block.outgoing() {
                pending.push(self.ir.cfg().edge(*edge).map_err(|_| reject())?.target());
            }
        }
        Ok(reachable.difference(&without).copied().collect())
    }
    fn builder_member(&self, reference: MethodRef) -> Option<JavaMethod> {
        let m = self.resolver.method(reference)?;
        let flags = self.resolver.method_access(reference)?;
        (m.owner == "java.lang.StringBuilder"
            && !m.is_static
            && m.throws.is_empty()
            && flags & 1 != 0
            && flags & (2 | 4 | 8 | 0x20 | 0x40 | 0x80 | 0x100 | 0x400) == 0)
            .then_some(m)
    }
    fn fresh_trusted_builder(
        &self,
        value: ValueId,
        allocations: &[(ValueId, InsnId, InsnId)],
    ) -> bool {
        let Some((_, allocation, initializer)) = allocations.iter().find(|(v, _, _)| *v == value)
        else {
            return false;
        };
        let Ok(ManagedInsn::NewInstance { class, .. }) = self.ir.instruction(*allocation) else {
            return false;
        };
        if !self.resolver.trusted_string_builder_class(*class)
            || self.resolver.class(*class).as_deref() != Some("java.lang.StringBuilder")
            || self.ty(value).ok() != Some(JavaType::Class("java.lang.StringBuilder".into()))
        {
            return false;
        }
        let Ok(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method,
            args,
        }) = self.ir.instruction(*initializer)
        else {
            return false;
        };
        let Some(m) = self.builder_member(*method) else {
            return false;
        };
        let Ok(args) = invoke_arguments::operands(&m, args) else {
            return false;
        };
        let parameters = match m.parameters.as_slice() {
            [] | [JavaType::Int] => true,
            [JavaType::Class(name)] => {
                matches!(name.as_str(), "java.lang.String" | "java.lang.CharSequence")
            }
            _ => false,
        };
        m.name == "<init>"
            && m.return_type == JavaType::Void
            && parameters
            && args.first() == Some(&value)
            && args.len() == m.parameters.len() + 1
    }
    fn discarded_builder_append(
        &self,
        insn: &ManagedInsn,
        allocations: &[(ValueId, InsnId, InsnId)],
    ) -> Option<ValueId> {
        let ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: None,
            method,
            args,
        } = insn
        else {
            return None;
        };
        let receiver = *args.first()?;
        if !self.fresh_trusted_builder(receiver, allocations) || args[1..].contains(&receiver) {
            return None;
        }
        let m = self.builder_member(*method)?;
        let args = invoke_arguments::operands(&m, args).ok()?;
        // These are the genuine Java 17 append descriptors, whose API contract
        // returns this final standard-library receiver. No custom fluent call.
        let parameters = match m.parameters.as_slice() {
            [
                JavaType::Boolean
                | JavaType::Char
                | JavaType::Int
                | JavaType::Long
                | JavaType::Float
                | JavaType::Double,
            ] => true,
            [JavaType::Class(name)] => matches!(
                name.as_str(),
                "java.lang.Object"
                    | "java.lang.String"
                    | "java.lang.CharSequence"
                    | "java.lang.StringBuffer"
            ),
            [JavaType::Array(component)] => **component == JavaType::Char,
            [JavaType::Array(component), JavaType::Int, JavaType::Int] => {
                **component == JavaType::Char
            }
            [JavaType::Class(name), JavaType::Int, JavaType::Int] => {
                name == "java.lang.CharSequence"
            }
            _ => false,
        };
        (m.name == "append"
            && m.return_type == JavaType::Class("java.lang.StringBuilder".into())
            && parameters
            && args.len() == m.parameters.len() + 1)
            .then_some(receiver)
    }
    fn builder_terminal(&self, insn: &ManagedInsn, receiver: ValueId) -> bool {
        let ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: Some(_),
            method,
            args,
        } = insn
        else {
            return false;
        };
        let Some(m) = self.builder_member(*method) else {
            return false;
        };
        args.as_slice() == [receiver]
            && m.name == "toString"
            && m.parameters.is_empty()
            && m.return_type == JavaType::Class("java.lang.String".into())
    }
    pub(super) fn argument_object_plan(
        &self,
        allocation: InsnId,
    ) -> Result<ObjectPlan, Unsupported> {
        let reject = || Unsupported::Instruction("allocation argument expression boundary".into());
        let ManagedInsn::NewInstance { result, class } =
            self.ir.instruction(allocation).map_err(|_| reject())?
        else {
            return Err(reject());
        };
        let owner = self.resolver.class(*class).ok_or_else(reject)?;
        if self.ty(*result)? != JavaType::Class(owner.clone()) {
            return Err(reject());
        }
        let mut aliases = vec![(*result, allocation)];
        let mut current = allocation;
        let mut scoped_chain = vec![allocation];
        let mut exceptional = false;
        let mut visited = HashSet::new();
        let mut gap = HashMap::new();
        let mut nodes = HashMap::new();
        let mut builder_steps: HashMap<ValueId, Vec<InsnId>> = HashMap::new();
        let mut order = vec![];
        let mut argument_allocations = vec![];
        let mut pending_nested: Vec<(ValueId, InsnId, String)> = vec![];
        loop {
            if !visited.insert(current) || visited.len() > 64 {
                return Err(reject());
            }
            let node = self.object_block(current).ok_or_else(reject)?;
            let mut normals = Vec::new();
            for edge_id in node.outgoing() {
                let edge = self.ir.cfg().edge(*edge_id).map_err(|_| reject())?;
                if matches!(edge.kind(), EdgeKind::Exception(_)) {
                    exceptional = true;
                } else {
                    normals.push(*edge_id);
                }
            }
            let [normal] = normals.as_slice() else {
                return Err(reject());
            };
            let edge = self.ir.cfg().edge(*normal).map_err(|_| reject())?;
            let next = self.ir.cfg().block(edge.target()).map_err(|_| reject())?;
            if *edge.kind() != EdgeKind::Unconditional || next.incoming().len() != 1 {
                return Err(reject());
            }
            current = *next.payload();
            scoped_chain.push(current);
            let insn = self.ir.instruction(current).map_err(|_| reject())?;
            if let ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                result: None,
                method,
                args,
            } = insn
                && pending_nested
                    .last()
                    .is_some_and(|(value, _, _)| args.first() == Some(value))
            {
                let (value, nested_allocation, nested_owner) =
                    pending_nested.pop().ok_or_else(reject)?;
                let m = self.resolver.method(*method).ok_or_else(reject)?;
                let args = invoke_arguments::operands(&m, args)?;
                if args.first() != Some(&value)
                    || m.owner != nested_owner
                    || m.name != "<init>"
                    || m.is_static
                    || m.return_type != JavaType::Void
                    || args.len() != m.parameters.len() + 1
                    || args[1..].contains(&value)
                    || args[1..].iter().any(|v| {
                        aliases.iter().any(|(a, _)| a == v)
                            || pending_nested.iter().any(|(a, _, _)| a == v)
                    })
                {
                    return Err(reject());
                }
                insert_node(
                    value,
                    current,
                    &args[1..],
                    Some(nested_allocation),
                    &mut gap,
                    &mut nodes,
                );
                argument_allocations.push((value, nested_allocation, current));
                order.push(current);
                continue;
            }
            if let ManagedInsn::NewInstance { result, class } = insn {
                let nested_owner = self.resolver.class(*class).ok_or_else(reject)?;
                if self.ty(*result)? != JavaType::Class(nested_owner.clone()) {
                    return Err(reject());
                }
                // Every pending allocation is represented by a Java nested-new
                // node. Initializers must close in stack order; the final exact
                // event trace still proves allocation precedes its arguments.
                pending_nested.push((*result, current, nested_owner));
                order.push(current);
                continue;
            }
            let alias = |v: &ValueId| aliases.iter().any(|(a, _)| a == v);
            if let ManagedInsn::Move { result, value } = insn
                && alias(value)
            {
                if self.ty(*result)? != JavaType::Class(owner.clone()) {
                    return Err(reject());
                }
                aliases.push((*result, current));
                continue;
            }
            if let ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                result: None,
                method,
                args,
            } = insn
                && args.first().is_some_and(alias)
            {
                let m = self.resolver.method(*method).ok_or_else(reject)?;
                let args = invoke_arguments::operands(&m, args)?;
                if !pending_nested.is_empty()
                    || m.owner != owner
                    || m.name != "<init>"
                    || m.is_static
                    || m.return_type != JavaType::Void
                    || args.len() != m.parameters.len() + 1
                    || args[1..].iter().any(alias)
                {
                    return Err(reject());
                }
                // Every allocation and producer stays in one original scope,
                // with the same dispatch and no handler-visible delayed value.
                // Matching catch names alone never proves this capability.
                exceptional |= next.outgoing().iter().any(|edge| {
                    self.ir
                        .cfg()
                        .edge(*edge)
                        .is_ok_and(|edge| matches!(edge.kind(), EdgeKind::Exception(_)))
                });
                if exceptional && !self.resolver.exception_allocation_chain(&scoped_chain) {
                    return Err(reject());
                }
                let wanted = gap.keys().copied().collect();
                let mut use_index = self.object_read_index(&wanted);
                let mut after = None;
                let mut argument_captures = Vec::new();
                let mut capture_occurrences = HashSet::new();
                for (value, definition) in &gap {
                    let uses = use_index.get(value).cloned().unwrap_or_default();
                    let inside = uses
                        .iter()
                        .filter(|id| **id == current || order.contains(id));
                    let outside: Vec<_> = uses
                        .iter()
                        .filter(|id| **id != current && !order.contains(id))
                        .collect();
                    if outside.is_empty() {
                        continue;
                    }
                    if after.is_none() {
                        after = Some(self.after_initializer(current)?);
                    }
                    // A nonthrowing literal can use the same typed capture as
                    // an ordinary producer: its original definition remains
                    // inside this argument, and later consumers read that slot.
                    // Never capture an uninitialized object, mutable builder,
                    // resolving string/class literal or wrapper bit conversion.
                    if !self.ir.instruction(*definition).is_ok_and(|insn| {
                        !eventful(insn)
                            || matches!(
                                insn,
                                ManagedInsn::FieldGet { .. }
                                    | ManagedInsn::ArrayGet { .. }
                                    | ManagedInsn::ArrayLength { .. }
                                    | ManagedInsn::Invoke {
                                        result: Some(_),
                                        ..
                                    }
                            )
                    }) || builder_steps.contains_key(value)
                        || inside.clone().count() != 1
                        || outside
                            .iter()
                            .any(|id| !after.as_ref().is_some_and(|after| after.contains(id)))
                    {
                        return Err(reject());
                    }
                    if self
                        .ir
                        .instruction(*definition)
                        .is_ok_and(|insn| !eventful(insn))
                        && !self.literal_capture_type_preserved(
                            *value,
                            *inside.clone().next().ok_or_else(reject)?,
                        )?
                    {
                        return Err(reject());
                    }
                    argument_captures.push(*value);
                    capture_occurrences.insert(*definition);
                }
                // Order reservations by their original definition sites, not
                // the hash table's iteration order.
                argument_captures
                    .sort_by_key(|value| order.iter().position(|id| gap[value] == *id));
                let mut actual_order = vec![];
                let mut seen = HashSet::new();
                for value in &args[1..] {
                    if let Some(id) = gap.get(value) {
                        postorder(
                            *id,
                            &nodes,
                            &mut seen,
                            &mut actual_order,
                            &capture_occurrences,
                        )?;
                    }
                }
                let events = |trace: &[InsnId]| {
                    trace
                        .iter()
                        .copied()
                        .filter(|id| self.ir.instruction(*id).is_ok_and(eventful))
                        .collect::<Vec<_>>()
                };
                if order.is_empty()
                    || seen.len() != nodes.len()
                    || events(&actual_order) != events(&order)
                {
                    return Err(reject());
                }
                // Every folded definition is consumed once, in its tree or initializer.
                for value in gap.keys() {
                    let uses = use_index.remove(value).unwrap_or_default();
                    let uses: Vec<_> = uses
                        .into_iter()
                        .filter(|id| {
                            !builder_steps
                                .get(value)
                                .is_some_and(|steps| steps.contains(id))
                        })
                        .filter(|id| {
                            !argument_allocations
                                .iter()
                                .any(|(v, _, init)| v == value && init == id)
                        })
                        .collect();
                    if argument_captures.contains(value) {
                        continue;
                    }
                    if uses.len() != 1 || (uses[0] != current && !order.contains(&uses[0])) {
                        return Err(reject());
                    }
                    if builder_steps.contains_key(value)
                        && !self
                            .ir
                            .instruction(uses[0])
                            .is_ok_and(|insn| self.builder_terminal(insn, *value))
                    {
                        return Err(reject());
                    }
                }
                return Ok(ObjectPlan {
                    allocation,
                    initializer: current,
                    aliases,
                    owner,
                    argument_gap: order,
                    argument_allocations,
                    argument_captures,
                });
            }
            if let Some(receiver) = self.discarded_builder_append(insn, &argument_allocations) {
                let ManagedInsn::Invoke { args, .. } = insn else {
                    return Err(reject());
                };
                if args
                    .iter()
                    .any(|v| alias(v) || pending_nested.iter().any(|(a, _, _)| a == v))
                {
                    return Err(reject());
                }
                let ManagedInsn::Invoke { method, .. } = insn else {
                    return Err(reject());
                };
                let m = self.resolver.method(*method).ok_or_else(reject)?;
                let operands = invoke_arguments::operands(&m, args)?;
                insert_node(receiver, current, &operands, None, &mut gap, &mut nodes);
                builder_steps.entry(receiver).or_default().push(current);
                order.push(current);
                continue;
            }
            match insn {
                ManagedInsn::Const {
                    value: Constant::ClassLiteral(target),
                    ..
                } if !matches!(
                    self.resolver.instruction_type(*target),
                    Some(JavaType::Class(_) | JavaType::Array(_))
                ) =>
                {
                    // Reference/array literals retain a single LDC resolution
                    // event. Primitive literals instead lower to wrapper.TYPE
                    // and need a separate wrapper-initialization proof.
                    return Err(reject());
                }
                ManagedInsn::Cast {
                    result,
                    value,
                    target,
                } => {
                    let source = self.ty(*value)?;
                    let target = self.resolver.instruction_type(*target).ok_or_else(reject)?;
                    if !(source.numeric() && target.numeric()
                        || (source.reference()
                            && target.reference()
                            && self.ty(*result)? == target))
                    {
                        return Err(reject());
                    }
                    // A reference cast is an explicit potentially throwing node.
                    // The same exact physical trace and single-use closure as all
                    // argument nodes keeps allocation before CHECKCAST, including
                    // its failure; no instanceof/dominance assumption is needed.
                }
                ManagedInsn::Arithmetic {
                    op,
                    result,
                    lhs,
                    rhs,
                } => {
                    let result_ty = self.ty(*result)?;
                    let numeric = result_ty.numeric()
                        && self.ty(*lhs)?.numeric()
                        && self.ty(*rhs)?.numeric()
                        && (!matches!(op, ArithOp::Div | ArithOp::Rem)
                            || matches!(result_ty, JavaType::Float | JavaType::Double));
                    let boolean_bits = matches!(op, ArithOp::And | ArithOp::Or | ArithOp::Xor)
                        && matches!(result_ty, JavaType::Int | JavaType::Boolean)
                        && self.category_one_bits(*result);
                    if !(numeric || boolean_bits) {
                        return Err(reject());
                    }
                }
                _ => {}
            }
            let (value, mut operands) = expression(insn).ok_or_else(reject)?;
            if let ManagedInsn::Invoke { method, args, .. } = insn {
                let m = self.resolver.method(*method).ok_or_else(reject)?;
                operands = invoke_arguments::operands(&m, args)?;
            }
            if operands
                .iter()
                .any(|v| alias(v) || pending_nested.iter().any(|(a, _, _)| a == v))
            {
                return Err(reject());
            }
            // Dimension expressions run before NewArray allocation (15.10.2),
            // while filled-array allocation precedes element evaluation (10.6).
            // Exact trace equality rejects a filled-array element computed in
            // the gap: folding it would move this possible OOM ahead of it.
            let before_operands =
                matches!(insn, ManagedInsn::NewFilledArray { .. }).then_some(current);
            insert_node(
                value,
                current,
                &operands,
                before_operands,
                &mut gap,
                &mut nodes,
            );
            order.push(current);
        }
    }

    pub(super) fn object_argument_instruction(&self, current: InsnId) -> bool {
        // The reverse index contains only the selected argument plans. Pure
        // gaps remain materialized by the ordinary path; active ownership is
        // re-evaluated after each initializer and under hoisted environments.
        self.object_argument_owners(current)
            .iter()
            .any(|result| !self.values.contains_key(result) || self.pending_allocation(*result))
    }

    pub(super) fn object_argument_expressions(
        &mut self,
        plan: &ObjectPlan,
        signature: &JavaMethod,
        captures: &HashMap<ValueId, (String, JavaType)>,
    ) -> Result<HashMap<ValueId, (String, JavaType)>, Unsupported> {
        for id in &plan.argument_gap {
            if let ManagedInsn::Invoke {
                kind: InvokeKind::Super,
                method,
                ..
            } = self.ir.instruction(*id).map_err(|_| {
                Unsupported::Instruction("super argument instruction unavailable".into())
            })? {
                let binding = self
                    .resolver
                    .super_invocation(&signature.owner, *method)
                    .ok_or_else(|| {
                        Unsupported::Metadata("exact super invocation binding unavailable".into())
                    })?;
                // Even a supplied binding cannot name the caller itself: a
                // lexical super invocation selects an ancestor declaration.
                if binding.method.owner == signature.owner {
                    return Err(Unsupported::Metadata(
                        "super argument selected caller declaration".into(),
                    ));
                }
            }
        }
        if self.object_argument_sequence(plan) {
            return self.object_sequence_expressions(plan, signature, captures);
        }
        // Reuse the standard emitter's exact descriptor, access, dispatch and
        // checked-exception proofs. Accept only its single definition contract.
        // The outer plan already proves the complete nested evaluation trace.
        // Lower its physical nodes once, rather than recursively folding an
        // inner gap whose producers are already scratch expressions. This local
        // cache has no active argument owners and never mutates shared plans.
        let scratch_planning = std::rc::Rc::new(ObjectPlanning::default());
        let _ = scratch_planning.argument_owners.set(HashMap::new());
        for (value, allocation, initializer) in &plan.argument_allocations {
            let JavaType::Class(owner) = self.ty(*value)? else {
                return Err(Unsupported::Type("nested allocation owner".into()));
            };
            scratch_planning.plans.borrow_mut().insert(
                *allocation,
                Ok(ObjectPlan {
                    allocation: *allocation,
                    initializer: *initializer,
                    aliases: vec![(*value, *allocation)],
                    owner,
                    argument_gap: vec![],
                    argument_allocations: vec![],
                    argument_captures: vec![],
                }),
            );
        }
        let mut scratch = Emitter {
            ir: self.ir,
            resolver: self.resolver,
            values: self.values.clone(),
            lines: vec![],
            next: self.next,
            parameters: self.parameters.clone(),
            reserved: self.reserved.clone(),
            current: None,
            hoisted: HashSet::new(),
            declarations: self.declarations.clone(),
            allocation_readable: self.allocation_readable.clone(),
            object_planning: scratch_planning,
        };
        // Branch planning reserves names for all definitions. A covered gap
        // definition has not executed: its unique-use/postorder proof places it
        // solely inside this constructor. Discard only scratch placeholders;
        // the parent's definite-availability environment remains untouched.
        for id in &plan.argument_gap {
            let insn = self
                .ir
                .instruction(*id)
                .map_err(|e| Unsupported::Instruction(e.to_string()))?;
            if self
                .discarded_builder_append(insn, &plan.argument_allocations)
                .is_some()
            {
                continue;
            }
            let result = match insn {
                ManagedInsn::NewInstance { result, .. } => *result,
                _ if plan
                    .argument_allocations
                    .iter()
                    .any(|(_, _, init)| init == id) =>
                {
                    continue;
                }
                _ => {
                    expression(insn)
                        .ok_or_else(|| Unsupported::Instruction("argument expression kind".into()))?
                        .0
                }
            };
            if self.hoisted.contains(&result) || captures.contains_key(&result) {
                scratch.values.remove(&result);
            } else if self.values.contains_key(&result) {
                return Err(Unsupported::Instruction(
                    "already materialized allocation argument".into(),
                ));
            }
        }
        scratch.values.insert(
            plan.aliases[0].0,
            (
                "__allocation_validation".into(),
                JavaType::Class(plan.owner.clone()),
            ),
        );
        let mut expressions = HashMap::new();
        for id in &plan.argument_gap {
            let insn = self
                .ir
                .instruction(*id)
                .map_err(|e| Unsupported::Instruction(e.to_string()))?;
            scratch.current = Some(*id);
            scratch.lines.clear();
            if matches!(insn, ManagedInsn::NewInstance { .. }) {
                scratch.instruction(insn, signature)?;
                if !scratch.lines.is_empty() {
                    return Err(Unsupported::Instruction(
                        "nested allocation materialized early".into(),
                    ));
                }
                continue;
            }
            if let Some(receiver) = self.discarded_builder_append(insn, &plan.argument_allocations)
            {
                // This exact validated Invoke(None) is the ordinary emitter's
                // single invocation statement. Capture that known expression;
                // do not interpret arbitrary statements or invent a call target.
                scratch.instruction(insn, signature)?;
                let [line] = scratch.lines.as_slice() else {
                    return Err(Unsupported::Instruction(
                        "builder append emitted extra statements".into(),
                    ));
                };
                let rhs = line.strip_suffix(';').ok_or_else(|| {
                    Unsupported::Instruction("builder append expression contract".into())
                })?;
                let ty = JavaType::Class("java.lang.StringBuilder".into());
                let value = (format!("(({}) ({rhs}))", ty.source()?), ty);
                scratch.values.insert(receiver, value.clone());
                expressions.insert(receiver, value);
                continue;
            }
            let result = if let Some((result, _, _)) = plan
                .argument_allocations
                .iter()
                .find(|(_, _, init)| init == id)
            {
                *result
            } else {
                expression(insn)
                    .ok_or_else(|| Unsupported::Instruction("argument expression kind".into()))?
                    .0
            };
            scratch.instruction(insn, signature)?;
            let (name, ty) = scratch.get(result)?;
            let prefix = format!("{} {name} = ", ty.source()?);
            let [line] = scratch.lines.as_slice() else {
                return Err(Unsupported::Instruction(
                    "argument expression has extra statements".into(),
                ));
            };
            let rhs = line
                .strip_prefix(&prefix)
                .and_then(|s| s.strip_suffix(';'))
                .ok_or_else(|| {
                    Unsupported::Instruction("argument expression definition contract".into())
                })?;
            // A typed local fixes overload selection and primitive promotion.
            // Pin that same static type when its initializer becomes an expression
            // (notably null and byte/short/char conversions represented as int).
            let rhs = if let Some((slot, slot_ty)) = captures.get(&result) {
                self.compatible(&ty, slot_ty)?;
                format!("({slot} = {rhs})")
            } else {
                rhs.to_owned()
            };
            let value = (format!("(({}) ({rhs}))", ty.source()?), ty);
            scratch.values.insert(result, value.clone());
            expressions.insert(result, value);
            // The first tree consumer embeds the assignment. Later source
            // consumers read its typed slot, never repeat the producer.
            let operands = if let ManagedInsn::Invoke { method, args, .. } = insn {
                let target = self
                    .resolver
                    .method(*method)
                    .ok_or_else(|| Unsupported::Metadata("capture consumer descriptor".into()))?;
                invoke_arguments::operands(&target, args)?
            } else {
                reads(insn)
            };
            for operand in operands {
                if let Some(slot) = captures.get(&operand) {
                    scratch.values.insert(operand, slot.clone());
                    expressions.insert(operand, slot.clone());
                }
            }
        }
        Ok(expressions)
    }

    pub(super) fn object_argument_capture_slots(
        &mut self,
        plan: &ObjectPlan,
    ) -> Result<HashMap<ValueId, (String, JavaType)>, Unsupported> {
        let mut slots = HashMap::new();
        for value in &plan.argument_captures {
            if let Some(slot) = self.values.get(value) {
                if !self.hoisted.contains(value) {
                    return Err(Unsupported::Instruction(
                        "already materialized captured argument".into(),
                    ));
                }
                slots.insert(*value, slot.clone());
                continue;
            }
            let ty = self.ty(*value)?;
            let mut name = format!("v{}", self.next);
            self.next += 1;
            while self.reserved.contains(&name) {
                name = format!("v{}", self.next);
                self.next += 1;
            }
            self.reserved.insert(name.clone());
            self.lines.push(format!("{} {name};", ty.source()?));
            slots.insert(*value, (name, ty));
        }
        Ok(slots)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use decomp_core::cfg::Cfg;
    use managed_ir::{
        method::{InsnArena, ValueArena},
        value::Value,
    };
    struct Resolver;
    impl JavaResolver for Resolver {
        fn discarded_invoke_source_target(&self, r: MethodRef) -> bool {
            // The runtime Fixture declares exactly static int tick(int).
            r.index() == 1
        }
        fn class(&self, r: ClassRef) -> Option<String> {
            Some(
                if r.index() == 0 {
                    "Fixture.A"
                } else {
                    "Fixture.Holder"
                }
                .into(),
            )
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
        fn field(&self, _: FieldRef) -> Option<JavaField> {
            Some(JavaField {
                owner: "Fixture.Holder".into(),
                name: "x".into(),
                is_static: false,
                ty: JavaType::Int,
            })
        }
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            Some(JavaMethod {
                owner: if r.index() == 0 {
                    "Fixture.A"
                } else {
                    "Fixture"
                }
                .into(),
                name: if r.index() == 0 { "<init>" } else { "tick" }.into(),
                is_static: r.index() != 0,
                return_type: if r.index() == 0 {
                    JavaType::Void
                } else {
                    JavaType::Int
                },
                parameters: if r.index() == 0 {
                    vec![JavaType::Int, JavaType::Int]
                } else {
                    vec![JavaType::Int]
                },
                throws: if r.index() == 2 {
                    vec!["java.io.IOException".into()]
                } else {
                    vec![]
                },
            })
        }
    }
    #[derive(Default)]
    struct Builder {
        insns: InsnArena,
        values: ValueArena,
        order: Vec<InsnId>,
    }
    impl Builder {
        fn value(&mut self, ty: ManagedType, f: impl FnOnce(ValueId) -> ManagedInsn) -> ValueId {
            let mut value = None;
            let id = self.insns.insert_with_key(|id| {
                let v = self.values.insert(Value::new(id, ty));
                value = Some(v);
                f(v)
            });
            self.order.push(id);
            value.unwrap()
        }
        fn push(&mut self, insn: ManagedInsn) {
            self.order.push(self.insns.insert(insn));
        }
        fn finish(self) -> MethodIr<SsaForm> {
            let mut cfg = Cfg::new(self.order[0]);
            let mut last = cfg.entry();
            for id in &self.order[1..] {
                let next = cfg.add_block(*id);
                cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
                last = next;
            }
            MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
        }
    }
    fn emit(mode: u8) -> SourceResult {
        let mut b = Builder::default();
        let holder = b.value(ManagedType::Reference(ClassRef::new(1)), |v| {
            ManagedInsn::Opaque {
                family_tag: 0x8001,
                reads: vec![],
                writes: vec![v],
                effects: managed_ir::effects::EffectFlags::NONE,
            }
        });
        b.order.pop();
        let n = b.value(ManagedType::Int, |v| ManagedInsn::Const {
            result: v,
            value: Constant::Int(2),
        });
        let branch_prefix = if mode == 6 {
            let condition = b.value(ManagedType::Boolean, |result| ManagedInsn::Const {
                result,
                value: Constant::Boolean(true),
            });
            b.push(ManagedInsn::Return { value: None }); // Replaced with exact branch edges below.
            for _ in 0..2 {
                b.push(ManagedInsn::Invoke {
                    kind: InvokeKind::Static,
                    result: None,
                    method: MethodRef::new(1),
                    args: vec![n],
                });
            }
            Some(condition)
        } else {
            None
        };
        let obj = b.value(ManagedType::Reference(ClassRef::new(0)), |v| {
            ManagedInsn::NewInstance {
                result: v,
                class: ClassRef::new(0),
            }
        });
        let field = b.value(ManagedType::Int, |v| {
            if mode == 7 {
                ManagedInsn::Const {
                    result: v,
                    value: Constant::Int(1),
                }
            } else {
                ManagedInsn::FieldGet {
                    result: v,
                    object: Some(holder),
                    field: FieldRef::new(0),
                }
            }
        });
        let first = b.value(ManagedType::Int, |v| {
            if mode == 7 {
                ManagedInsn::Move {
                    result: v,
                    value: field,
                }
            } else {
                ManagedInsn::Invoke {
                    kind: InvokeKind::Static,
                    result: Some(v),
                    method: MethodRef::new(if mode == 5 { 2 } else { 1 }),
                    args: vec![field],
                }
            }
        });
        let second = b.value(ManagedType::Int, |v| {
            if mode == 7 {
                ManagedInsn::Move {
                    result: v,
                    value: n,
                }
            } else {
                ManagedInsn::Invoke {
                    kind: InvokeKind::Static,
                    result: Some(v),
                    method: MethodRef::new(1),
                    args: vec![if mode == 2 { field } else { n }],
                }
            }
        });
        if mode == 3 {
            b.push(ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(1),
                args: vec![n],
            });
        }
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(0),
            args: if mode == 1 {
                vec![obj, second, first]
            } else {
                vec![obj, first, second]
            },
        });
        if mode == 4 {
            b.push(ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(1),
                args: vec![first],
            });
        }
        b.push(ManagedInsn::Return { value: Some(obj) });
        let ir = if let Some(condition) = branch_prefix {
            let mut cfg = Cfg::new(b.order[0]);
            let blocks: Vec<_> = b.order[1..].iter().map(|id| cfg.add_block(*id)).collect();
            cfg.add_edge(cfg.entry(), blocks[0], EdgeKind::Unconditional)
                .unwrap();
            cfg.add_edge(blocks[0], blocks[1], EdgeKind::Unconditional)
                .unwrap();
            let yes = cfg
                .add_edge(blocks[1], blocks[2], EdgeKind::BranchTrue)
                .unwrap();
            let no = cfg
                .add_edge(blocks[1], blocks[3], EdgeKind::BranchFalse)
                .unwrap();
            cfg.add_edge(blocks[2], blocks[4], EdgeKind::Unconditional)
                .unwrap();
            cfg.add_edge(blocks[3], blocks[4], EdgeKind::Unconditional)
                .unwrap();
            for pair in blocks[4..].windows(2) {
                cfg.add_edge(pair[0], pair[1], EdgeKind::Unconditional)
                    .unwrap();
            }
            b.insns[b.order[2]] = ManagedInsn::Branch {
                condition: Some(condition),
                then_edge: yes,
                else_edge: Some(no),
            };
            MethodIr::<SsaForm>::from_parts(cfg, b.insns, b.values)
        } else {
            b.finish()
        };
        emit_method(
            &ir,
            &JavaMethod {
                owner: "Fixture".into(),
                name: "construct".into(),
                is_static: true,
                return_type: JavaType::Class("Fixture.A".into()),
                parameters: vec![JavaType::Class("Fixture.Holder".into())],
                throws: vec![],
            },
            &ParameterBindings {
                parameters: vec![holder],
                ..Default::default()
            },
            &Resolver,
        )
    }
    #[test]
    fn allocation_argument_tree_preserves_class_initialization_and_argument_effect_order() {
        let result = emit(0);
        let SourceResult::Complete { source, .. } = result else {
            panic!("{result:?}")
        };
        assert_eq!(source.matches("Fixture.tick").count(), 2, "{source}");
        assert!(source.contains("new Fixture.A("), "{source}");
        assert!(
            !source.lines().any(|line| line.contains(" = Fixture.tick")),
            "{source}"
        );
        check_java(&source, false);
    }
    fn check_java(source: &str, branched: bool) {
        let before = if branched { "2" } else { "" };
        let dir = std::env::temp_dir().join(format!(
            "object-argument-plan-{}-{branched}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let fixture = format!(
            r#"public class Fixture {{
            static String log="";
            static class Holder {{ int x=1; }}
            static class A {{ static {{ log += "A"; }} A(int x,int y) {{ log += "C"; }} }}
            static int tick(int n) {{log += n; if(n<0) throw new IllegalArgumentException(); return n;}}
            {source}
            public static void main(String[] args) {{
                if(args.length>0) {{try{{construct(null);throw new AssertionError();}}catch(NullPointerException expected){{}}if(!log.equals("{before}A"))throw new AssertionError(log);return;}}
                construct(new Holder());if(!log.equals("{before}A12C"))throw new AssertionError(log);
                log="";Holder h=new Holder();h.x=-1;try{{construct(h);throw new AssertionError();}}catch(IllegalArgumentException expected){{}}if(!log.equals("{before}-1"))throw new AssertionError(log);
            }} }}"#
        );
        std::fs::write(dir.join("Fixture.java"), fixture).unwrap();
        let home = std::env::var("JAVA_HOME")
            .unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
        for (tool, args) in [
            ("javac", vec!["--release", "17", "Fixture.java"]),
            ("java", vec!["-cp", ".", "Fixture"]),
            ("java", vec!["-cp", ".", "Fixture", "null"]),
        ] {
            let out = std::process::Command::new(format!("{home}/bin/{tool}"))
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}\n{source}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn branch_shared_tail_folds_hoisted_argument_placeholders() {
        let result = emit(6);
        let SourceResult::Complete { source, .. } = result else {
            panic!("{result:?}");
        };
        assert!(source.contains("if ("), "{source}");
        assert!(source.contains("new Fixture.A("), "{source}");
        assert_eq!(source.matches("Fixture.tick").count(), 4, "{source}");
        assert!(
            source
                .lines()
                .filter(|line| line.contains("new Fixture.A("))
                .any(|line| line.matches("Fixture.tick").count() == 2),
            "{source}"
        );
        check_java(&source, true);
    }

    #[test]
    fn ordinary_pure_gap_materializes_constants_and_moves_under_the_selected_plan() {
        let result = emit(7);
        let SourceResult::Complete { source, .. } = result else {
            panic!("{result:?}");
        };
        assert!(source.contains("new Fixture.A("), "{source}");
        assert!(
            source.lines().any(|line| line.contains(" = 1;")),
            "{source}"
        );
        assert!(!source.contains("Fixture.tick"), "{source}");
    }

    #[test]
    fn sequenced_reordered_shared_and_discarded_calls_retain_checked_throw_denial() {
        for mode in [1, 2, 3, 5] {
            let result = emit(mode);
            if mode == 5 {
                assert!(
                    matches!(result, SourceResult::Unsupported(_)),
                    "mode {mode}: {result:?}"
                );
            } else {
                let SourceResult::Complete { source, .. } = result else {
                    panic!("mode{mode}:{result:?}")
                };
                crate::object_plan::sequence_tests::compile_fixture(
                    &source,
                    "static class Holder { int x; } static class A { A(int x,int y) {} } static int tick(int x) { return x; }",
                );
            }
        }
    }
    #[test]
    fn dominated_outside_use_captures_the_invocation_result_once() {
        let result = emit(4);
        let SourceResult::Complete { source, .. } = result else {
            panic!("{result:?}");
        };
        assert_eq!(source.matches("Fixture.tick(").count(), 3, "{source}");
        assert!(
            source
                .lines()
                .any(|line| line.contains("new Fixture.A(") && line.contains(" = Fixture.tick(")),
            "{source}"
        );
    }
    struct StringResolver;
    impl JavaResolver for StringResolver {
        fn discarded_invoke_source_target(&self, r: MethodRef) -> bool {
            // The runtime Fixture declares exactly String mark(String) and int tick(int).
            matches!(r.index(), 1 | 2)
        }
        fn class(&self, r: ClassRef) -> Option<String> {
            Some(
                if r.index() == 0 {
                    "Fixture.Text"
                } else {
                    "java.lang.String"
                }
                .into(),
            )
        }
        fn field(&self, _: FieldRef) -> Option<JavaField> {
            None
        }
        fn string(&self, r: StringRef) -> Option<Vec<u16>> {
            (r.index() == 0).then(|| "one".encode_utf16().collect())
        }
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            let string = JavaType::Class("java.lang.String".into());
            Some(match r.index() {
                0 | 3 => JavaMethod {
                    owner: "Fixture.Text".into(),
                    name: "<init>".into(),
                    is_static: false,
                    return_type: JavaType::Void,
                    parameters: if r.index() == 3 {
                        vec![JavaType::Int, string]
                    } else {
                        vec![string, JavaType::Int]
                    },
                    throws: vec![],
                },
                1 => JavaMethod {
                    owner: "Fixture".into(),
                    name: "mark".into(),
                    is_static: true,
                    return_type: string.clone(),
                    parameters: vec![string],
                    throws: vec![],
                },
                _ => JavaMethod {
                    owner: "Fixture".into(),
                    name: "tick".into(),
                    is_static: true,
                    return_type: JavaType::Int,
                    parameters: vec![JavaType::Int],
                    throws: vec![],
                },
            })
        }
    }
    fn emit_string(mode: u8) -> SourceResult {
        let mut b = Builder::default();
        let n = b.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(2),
        });
        let obj = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
            ManagedInsn::NewInstance {
                result,
                class: ClassRef::new(0),
            }
        });
        let text = b.value(ManagedType::Reference(ClassRef::new(1)), |result| {
            ManagedInsn::Const {
                result,
                value: Constant::String(StringRef::new(if mode == 5 { 1 } else { 0 })),
            }
        });
        let marked = b.value(ManagedType::Reference(ClassRef::new(1)), |result| {
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(result),
                method: MethodRef::new(1),
                args: vec![text],
            }
        });
        if mode == 3 {
            b.push(ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(1),
                args: vec![text],
            });
        }
        if mode == 4 {
            b.push(ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(2),
                args: vec![n],
            });
        }
        let second = b.value(ManagedType::Int, |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(2),
            args: vec![n],
        });
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(if mode == 1 { 3 } else { 0 }),
            args: if mode == 1 {
                vec![obj, second, marked]
            } else {
                vec![obj, marked, second]
            },
        });
        if mode == 2 {
            b.push(ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(1),
                args: vec![text],
            });
        }
        b.push(ManagedInsn::Return { value: Some(obj) });
        emit_method(
            &b.finish(),
            &JavaMethod {
                owner: "Fixture".into(),
                name: "construct".into(),
                is_static: true,
                return_type: JavaType::Class("Fixture.Text".into()),
                parameters: vec![],
                throws: vec![],
            },
            &ParameterBindings::default(),
            &StringResolver,
        )
    }
    #[test]
    fn string_argument_expression_is_single_and_follows_outer_class_initialization() {
        let result = emit_string(0);
        let SourceResult::Complete { source, .. } = result else {
            panic!("{result:?}")
        };
        assert_eq!(source.matches("\"one\"").count(), 1, "{source}");
        assert!(
            !source.contains(".intern()") && !source.contains("new java.lang.String("),
            "{source}"
        );
        assert!(
            source.lines().any(|line| line.contains("new Fixture.Text(")
                && line.contains("\"one\"")
                && line.contains("Fixture.mark(")),
            "{source}"
        );
        let dir = std::env::temp_dir().join(format!(
            "object-string-argument-plan-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Fixture.java"),format!(r#"public class Fixture {{
            static String log="";
            static class Text {{static {{log += "A";}} Text(String text,int n) {{if(text != "one" || n != 2) throw new AssertionError();log += "C";}}}}
            static String mark(String text) {{if(!log.equals("A") || text != "one") throw new AssertionError(log);log += "S";return text;}}
            static int tick(int n) {{log += n;return n;}}
            {source}
            public static void main(String[] args) {{construct();if(!log.equals("AS2C"))throw new AssertionError(log);}}
        }}"#)).unwrap();
        let home = std::env::var("JAVA_HOME")
            .unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
        for (tool, args) in [
            ("javac", vec!["--release", "17", "Fixture.java"]),
            ("java", vec!["-cp", ".", "Fixture"]),
        ] {
            let output = std::process::Command::new(format!("{home}/bin/{tool}"))
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{source}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn sequenced_strings_retain_original_calls_and_reject_unresolved_literal() {
        for mode in 1..=5 {
            let result = emit_string(mode);
            if mode == 5 {
                assert!(
                    matches!(result, SourceResult::Unsupported(_)),
                    "mode{mode}:{result:?}"
                );
            } else {
                let SourceResult::Complete { source, .. } = result else {
                    panic!("mode{mode}:{result:?}")
                };
                crate::object_plan::sequence_tests::compile_fixture(
                    &source,
                    "static class Text { Text(String s,int n) {} Text(int n,String s) {} } static String mark(String s) { return s; } static int tick(int n) { return n; }",
                );
            }
        }
    }
    struct NumericResolver;
    impl JavaResolver for NumericResolver {
        fn class(&self, r: ClassRef) -> Option<String> {
            Some(
                if r.index() == 0 {
                    "Fixture.Numeric"
                } else {
                    "Fixture.Holder"
                }
                .into(),
            )
        }
        fn field(&self, _: FieldRef) -> Option<JavaField> {
            Some(JavaField {
                owner: "Fixture.Holder".into(),
                name: "x".into(),
                is_static: false,
                ty: JavaType::Int,
            })
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
        fn instruction_type(&self, r: TypeRef) -> Option<JavaType> {
            Some(match r.index() {
                0 => JavaType::Float,
                2 => JavaType::Byte,
                _ => JavaType::Class("Fixture.Holder".into()),
            })
        }
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            Some(match r.index() {
                0 | 3 => JavaMethod {
                    owner: "Fixture.Numeric".into(),
                    name: "<init>".into(),
                    is_static: false,
                    return_type: JavaType::Void,
                    parameters: if r.index() == 3 {
                        vec![JavaType::Int, JavaType::Float]
                    } else {
                        vec![JavaType::Float, JavaType::Int]
                    },
                    throws: vec![],
                },
                1 => JavaMethod {
                    owner: "Fixture".into(),
                    name: "mark".into(),
                    is_static: true,
                    return_type: JavaType::Float,
                    parameters: vec![JavaType::Float],
                    throws: vec![],
                },
                4 => JavaMethod {
                    owner: "Fixture".into(),
                    name: "narrow".into(),
                    is_static: true,
                    return_type: JavaType::Float,
                    parameters: vec![JavaType::Int],
                    throws: vec![],
                },
                _ => JavaMethod {
                    owner: "Fixture".into(),
                    name: "tick".into(),
                    is_static: true,
                    return_type: JavaType::Int,
                    parameters: vec![JavaType::Int],
                    throws: vec![],
                },
            })
        }
    }
    fn emit_numeric(mode: u8) -> SourceResult {
        let mut b = Builder::default();
        let holder = b.value(ManagedType::Reference(ClassRef::new(1)), |v| {
            ManagedInsn::Opaque {
                family_tag: 0x8001,
                reads: vec![],
                writes: vec![v],
                effects: managed_ir::effects::EffectFlags::NONE,
            }
        });
        b.order.pop();
        let one = b.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(1),
        });
        let two = b.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(2),
        });
        let obj = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
            ManagedInsn::NewInstance {
                result,
                class: ClassRef::new(0),
            }
        });
        let field = b.value(ManagedType::Int, |result| ManagedInsn::FieldGet {
            result,
            object: Some(holder),
            field: FieldRef::new(0),
        });
        let converted = b.value(
            if mode == 5 {
                ManagedType::Reference(ClassRef::new(1))
            } else if mode == 7 {
                ManagedType::Int
            } else {
                ManagedType::Float
            },
            |result| ManagedInsn::Cast {
                result,
                value: if mode == 5 { holder } else { field },
                target: TypeRef::new(if mode == 5 {
                    1
                } else if mode == 7 {
                    2
                } else {
                    0
                }),
            },
        );
        let first = b.value(ManagedType::Float, |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(if mode == 7 { 4 } else { 1 }),
            args: vec![converted],
        });
        let tick = b.value(ManagedType::Int, |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(2),
            args: vec![two],
        });
        let second = b.value(
            if mode == 4 {
                ManagedType::Short
            } else {
                ManagedType::Int
            },
            |result| ManagedInsn::Arithmetic {
                op: if mode == 3 {
                    ArithOp::Div
                } else {
                    ArithOp::Add
                },
                result,
                lhs: tick,
                rhs: if mode == 2 { tick } else { one },
            },
        );
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(if mode == 1 { 3 } else { 0 }),
            args: if mode == 1 {
                vec![obj, second, first]
            } else {
                vec![obj, first, second]
            },
        });
        if mode == 6 {
            b.push(ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(2),
                args: vec![second],
            });
        }
        b.push(ManagedInsn::Return { value: Some(obj) });
        emit_method(
            &b.finish(),
            &JavaMethod {
                owner: "Fixture".into(),
                name: "construct".into(),
                is_static: true,
                return_type: JavaType::Class("Fixture.Numeric".into()),
                parameters: vec![JavaType::Class("Fixture.Holder".into())],
                throws: vec![],
            },
            &ParameterBindings {
                parameters: vec![holder],
                ..Default::default()
            },
            &NumericResolver,
        )
    }
    #[test]
    fn numeric_argument_tree_preserves_float_rounding_allocation_and_effect_order() {
        let result = emit_numeric(0);
        let SourceResult::Complete { source, .. } = result else {
            panic!("{result:?}")
        };
        assert!(
            source
                .lines()
                .any(|line| line.contains("new Fixture.Numeric(")
                    && line.contains("(float)")
                    && line.contains(" + ")),
            "{source}"
        );
        assert_eq!(source.matches("Fixture.mark(").count(), 1, "{source}");
        assert_eq!(source.matches("Fixture.tick(").count(), 1, "{source}");
        check_numeric_java(&source);
    }
    #[test]
    fn narrowed_primitive_argument_keeps_its_ssa_int_overload() {
        let result = emit_numeric(7);
        let SourceResult::Complete { source, .. } = result else {
            panic!("{result:?}");
        };
        assert!(source.contains("(int) ((byte)"), "{source}");
        check_numeric_java(&source);
    }
    fn check_numeric_java(source: &str) {
        let dir = std::env::temp_dir().join(format!(
            "numeric-argument-plan-{}-{}",
            std::process::id(),
            source.contains("Fixture.narrow(")
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Fixture.java"),format!(r#"public class Fixture {{
            static String log="";static class Holder {{int x=16777217;}}
            static class Numeric {{static {{log += "A";}} Numeric(float x,int y) {{if(x!=16777216f || y!=3)throw new AssertionError(x+" "+y);log += "C";}}}}
            static float mark(float x) {{log += "F";return x;}} static int tick(int n) {{log += n;return n;}}
            static float narrow(byte x) {{throw new AssertionError("wrong overload");}}
            static float narrow(int x) {{if(x!=1)throw new AssertionError(x);log += "F";return 16777216f;}}
            {source}
            public static void main(String[] args) {{if(args.length>0){{try{{construct(null);throw new AssertionError();}}catch(NullPointerException expected){{}}if(!log.equals("A"))throw new AssertionError(log);}}else{{construct(new Holder());if(!log.equals("AF2C"))throw new AssertionError(log);}}}}
        }}"#)).unwrap();
        let home = std::env::var("JAVA_HOME")
            .unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
        for (tool, args) in [
            ("javac", vec!["--release", "17", "Fixture.java"]),
            ("java", vec!["-cp", ".", "Fixture"]),
            ("java", vec!["-cp", ".", "Fixture", "null"]),
        ] {
            let out = std::process::Command::new(format!("{home}/bin/{tool}"))
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}\n{source}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn sequenced_numeric_producers_retain_typed_narrow_and_reference_denials() {
        for mode in 1..=6 {
            let result = emit_numeric(mode);
            if matches!(mode, 4 | 5) {
                assert!(
                    matches!(result, SourceResult::Unsupported(_)),
                    "mode{mode}:{result:?}"
                );
            } else {
                let SourceResult::Complete { source, .. } = result else {
                    panic!("mode{mode}:{result:?}")
                };
                crate::object_plan::sequence_tests::compile_fixture(
                    &source,
                    "static class Holder { int x; } static class Numeric { Numeric(float x,int y) {} Numeric(int x,float y) {} } static float mark(float x) { return x; } static int tick(int n) { return n; }",
                );
            }
        }
    }
    struct NestedResolver;
    impl JavaResolver for NestedResolver {
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
        fn field(&self, _: FieldRef) -> Option<JavaField> {
            None
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            let inner = JavaType::Class("Fixture.Inner".into());
            Some(match r.index() {
                0 | 3 => JavaMethod {
                    owner: "Fixture.Outer".into(),
                    name: "<init>".into(),
                    is_static: false,
                    return_type: JavaType::Void,
                    parameters: if r.index() == 3 {
                        vec![JavaType::Int, inner]
                    } else {
                        vec![inner, JavaType::Int]
                    },
                    throws: vec![],
                },
                1 | 4 | 5 => JavaMethod {
                    owner: if r.index() == 4 {
                        "Fixture.Outer"
                    } else {
                        "Fixture.Inner"
                    }
                    .into(),
                    name: "<init>".into(),
                    is_static: false,
                    return_type: JavaType::Void,
                    parameters: vec![JavaType::Int],
                    throws: if r.index() == 5 {
                        vec!["java.io.IOException".into()]
                    } else {
                        vec![]
                    },
                },
                6 => JavaMethod {
                    owner: "Fixture".into(),
                    name: "consume".into(),
                    is_static: true,
                    return_type: JavaType::Void,
                    parameters: vec![inner],
                    throws: vec![],
                },
                _ => JavaMethod {
                    owner: "Fixture".into(),
                    name: "tick".into(),
                    is_static: true,
                    return_type: JavaType::Int,
                    parameters: vec![JavaType::Int],
                    throws: vec![],
                },
            })
        }
    }
    fn emit_nested(mode: u8) -> SourceResult {
        let mut b = Builder::default();
        let flag = b.value(ManagedType::Int, |v| ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![v],
            effects: managed_ir::effects::EffectFlags::NONE,
        });
        b.order.pop();
        let outer = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
            ManagedInsn::NewInstance {
                result,
                class: ClassRef::new(0),
            }
        });
        let inner_arg = if mode == 9 {
            b.value(ManagedType::Int, |result| ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(result),
                method: MethodRef::new(2),
                args: vec![flag],
            })
        } else {
            flag
        };
        let inner = b.value(ManagedType::Reference(ClassRef::new(1)), |result| {
            ManagedInsn::NewInstance {
                result,
                class: ClassRef::new(1),
            }
        });
        if mode == 3 {
            b.push(ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(2),
                args: vec![flag],
            });
        }
        if mode == 4 {
            b.push(ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(6),
                args: vec![inner],
            });
        }
        if mode == 8 {
            b.value(ManagedType::Reference(ClassRef::new(1)), |result| {
                ManagedInsn::Move {
                    result,
                    value: inner,
                }
            });
        }
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(if mode == 5 {
                4
            } else if mode == 6 {
                5
            } else {
                1
            }),
            args: vec![inner, if mode == 7 { outer } else { inner_arg }],
        });
        let last = b.value(ManagedType::Int, |result| ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(2),
            args: vec![flag],
        });
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(if mode == 1 { 3 } else { 0 }),
            args: if mode == 1 {
                vec![outer, last, inner]
            } else {
                vec![outer, inner, last]
            },
        });
        if mode == 2 {
            b.push(ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(6),
                args: vec![inner],
            });
        }
        b.push(ManagedInsn::Return { value: Some(outer) });
        emit_method(
            &b.finish(),
            &JavaMethod {
                owner: "Fixture".into(),
                name: "construct".into(),
                is_static: true,
                return_type: JavaType::Class("Fixture.Outer".into()),
                parameters: vec![JavaType::Int],
                throws: vec![],
            },
            &ParameterBindings {
                parameters: vec![flag],
                ..Default::default()
            },
            &NestedResolver,
        )
    }
    #[test]
    fn immediate_nested_argument_preserves_outer_inner_allocation_and_constructor_order() {
        let result = emit_nested(0);
        let SourceResult::Complete { source, .. } = result else {
            panic!("{result:?}")
        };
        assert_eq!(source.matches("new Fixture.Inner(").count(), 1, "{source}");
        assert!(
            source
                .lines()
                .any(|line| line.contains("new Fixture.Outer(")
                    && line.contains("new Fixture.Inner(")
                    && line.contains("Fixture.tick(")),
            "{source}"
        );
        let dir = std::env::temp_dir().join(format!("nested-argument-plan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Fixture.java"),format!(r#"public class Fixture {{
            static String log=""; static class Inner {{static{{log += "I";}}final int flag;Inner(int n){{log += "i";flag=n;if(n<0)throw new IllegalArgumentException();}}}}
            static class Outer {{static{{log += "O";}}Outer(Inner inner,int n){{log += "C";if(inner.flag!=n)throw new AssertionError();if(n==2)throw new IllegalStateException();}}}}
            static int tick(int n){{log += "T";return n;}} {source}
            public static void main(String[] args){{int n=Integer.parseInt(args[0]);if(n<0){{try{{construct(n);throw new AssertionError();}}catch(IllegalArgumentException expected){{}}if(!log.equals("OIi"))throw new AssertionError(log);}}else if(n==2){{try{{construct(n);throw new AssertionError();}}catch(IllegalStateException expected){{}}if(!log.equals("OIiTC"))throw new AssertionError(log);}}else{{construct(n);if(!log.equals("OIiTC"))throw new AssertionError(log);}}}}
        }}"#)).unwrap();
        let home = std::env::var("JAVA_HOME")
            .unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
        for (tool, args) in [
            ("javac", vec!["--release", "17", "Fixture.java"]),
            ("java", vec!["-cp", ".", "Fixture", "1"]),
            ("java", vec!["-cp", ".", "Fixture", "-1"]),
            ("java", vec!["-cp", ".", "Fixture", "2"]),
        ] {
            let out = std::process::Command::new(format!("{home}/bin/{tool}"))
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}\n{source}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn sequence_nested_arguments_keep_reversed_positions_and_precomputed_effects() {
        for mode in [1, 8, 9] {
            let SourceResult::Complete { source, .. } = emit_nested(mode) else {
                panic!("faithful sequence mode{mode} must emit");
            };
            assert!(source.contains("switch (0)"));
            let dir = std::env::temp_dir().join(format!(
                "nested-sequence-position-{}-{mode}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("Fixture.java"),format!(r#"public class Fixture {{
                static String log="";static class Inner{{static{{log+="I";}}final int flag;Inner(int n){{log+="i";flag=n;if(n<0)throw new IllegalArgumentException();}}}}
                static class Outer{{static{{log+="O";}}Outer(Inner inner,int n){{log+="C";if(inner.flag!=n)throw new AssertionError();if(n==2)throw new IllegalStateException();}}Outer(int n,Inner inner){{this(inner,n);}}}}
                static int tick(int n){{log+="T";return n;}}{source}
                public static void main(String[]args){{int n=Integer.parseInt(args[0]);try{{construct(n);if(n<0||n==2)throw new AssertionError();}}catch(IllegalArgumentException expected){{if(n>=0)throw expected;}}catch(IllegalStateException expected){{if(n!=2)throw expected;}}String prefix={prefix};String expected=n<0?prefix+"Ii":prefix+"IiTC";if(!log.equals(expected))throw new AssertionError(log+" != "+expected);}}
            }}"#,prefix=if mode==9{"\"OT\""}else{"\"O\""})).unwrap();
            let home = "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1";
            for (tool, args) in [
                ("javac", vec!["--release", "25", "Fixture.java"]),
                ("java", vec!["-cp", ".", "Fixture", "1"]),
                ("java", vec!["-cp", ".", "Fixture", "-1"]),
                ("java", vec!["-cp", ".", "Fixture", "2"]),
            ] {
                let out = std::process::Command::new(format!("{home}/bin/{tool}"))
                    .args(args)
                    .current_dir(&dir)
                    .output()
                    .unwrap();
                assert!(
                    out.status.success(),
                    "mode{mode}: {}\n{source}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
        }
    }
    #[test]
    fn nested_argument_rejects_order_escape_effect_wrong_owner_checked_throw_alias_and_preinit_use()
    {
        for mode in 2..=7 {
            let result = emit_nested(mode);
            assert!(
                matches!(result, SourceResult::Unsupported(_)),
                "mode {mode}: {result:?}"
            );
        }
    }
}

#[cfg(test)]
#[path = "object_array_arguments_tests.rs"]
mod array_tests;

#[cfg(test)]
#[path = "object_producer_arguments_tests.rs"]
mod producer_tests;

#[cfg(test)]
#[path = "object_array_read_arguments_tests.rs"]
mod array_read_tests;

#[cfg(test)]
#[path = "object_builder_arguments_tests.rs"]
mod builder_tests;

#[cfg(test)]
#[path = "object_class_literal_arguments_tests.rs"]
mod class_literal_tests;

#[cfg(test)]
#[path = "object_argument_reads_tests.rs"]
mod read_tests;

#[cfg(test)]
#[path = "object_cast_arguments_tests.rs"]
mod cast_tests;

#[cfg(test)]
#[path = "object_nested_arguments_tests.rs"]
mod nested_tests;
