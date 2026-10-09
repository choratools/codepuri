//! Statement sequences inside a primitive constant switch expression keep NEW
//! before every original argument producer, including genuine void invocations.
use super::*;

fn sequence_result(insn: &ManagedInsn) -> Option<ValueId> {
    expression(insn).map(|(value, _)| value).or(match insn {
        ManagedInsn::NewInstance { result, .. } => Some(*result),
        ManagedInsn::Neg { result, .. }
        | ManagedInsn::Not { result, .. }
        | ManagedInsn::InstanceOf { result, .. }
        | ManagedInsn::Compare { result, .. }
        | ManagedInsn::OrderingCompare { result, .. } => Some(*result),
        _ => None,
    })
}
fn sequence_statement(insn: &ManagedInsn) -> bool {
    sequence_result(insn).is_some()
        || matches!(
            insn,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static | InvokeKind::Virtual | InvokeKind::Interface,
                result: None,
                ..
            } | ManagedInsn::FieldPut { .. }
                | ManagedInsn::ArrayPut { .. }
                | ManagedInsn::FillArray { .. }
        )
}
impl<R: JavaResolver> Emitter<'_, R> {
    fn initialized_sequence_use(
        &self,
        consumer: InsnId,
        aliases: &[(ValueId, InsnId)],
        after: &InstructionAvailability,
    ) -> bool {
        if after.contains(&consumer) {
            return true;
        }
        let Ok(ManagedInsn::Phi { inputs, .. }) = self.ir.instruction(consumer) else {
            return false;
        };
        if self.object_block(consumer).is_none() {
            return false;
        }
        let mut matched = false;
        for (edge, value) in inputs {
            if !aliases.iter().any(|(alias, _)| alias == value) {
                continue;
            }
            matched = true;
            let Ok(edge) = self.ir.cfg().edge(*edge) else {
                return false;
            };
            if matches!(edge.kind(), EdgeKind::Exception(_)) {
                return false;
            }
            let Ok(source) = self.ir.cfg().block(edge.source()) else {
                return false;
            };
            if !after.contains(source.payload()) {
                return false;
            }
            // A Phi reads at its selected predecessor exit. For consecutive
            // Phi clusters, retain the exact incoming edge to the cluster head.
            let mut target = edge.target();
            let mut visited = HashSet::new();
            loop {
                if !visited.insert(target) {
                    return false;
                }
                let Ok(block) = self.ir.cfg().block(target) else {
                    return false;
                };
                if !matches!(
                    self.ir.instruction(*block.payload()),
                    Ok(ManagedInsn::Phi { .. })
                ) {
                    return false;
                }
                if *block.payload() == consumer {
                    break;
                }
                let [next] = block.outgoing() else {
                    return false;
                };
                let Ok(next) = self.ir.cfg().edge(*next) else {
                    return false;
                };
                if *next.kind() != EdgeKind::Unconditional {
                    return false;
                }
                target = next.target();
            }
        }
        matched
    }
    // A Phi consumes its selected edge, not every path into its containing block.
    // Retain both successful construction and the exact capture definition.
    fn initialized_capture_use(
        &self,
        consumer: InsnId,
        value: ValueId,
        after_constructor: &InstructionAvailability,
    ) -> Result<bool, Unsupported> {
        let definition = self
            .ir
            .value(value)
            .map_err(|_| Unsupported::Instruction("allocation capture definition".into()))?
            .def();
        let after_definition = self.after_initializer(definition)?;
        let available = after_constructor.intersect(&after_definition);
        Ok(self.initialized_sequence_use(consumer, &[(value, definition)], &available))
    }
    pub(in crate::object_plan) fn sequence_object_plan(
        &self,
        allocation: InsnId,
    ) -> Result<ObjectPlan, Unsupported> {
        let reject = || Unsupported::Instruction("allocation statement sequence boundary".into());
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
        let mut chain = vec![allocation];
        let mut gap = Vec::new();
        let mut definitions = HashSet::new();
        let mut nested = Vec::new();
        let mut pending_nested: Vec<ObjectPlan> = Vec::new();
        let mut exceptional = false;
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(current) || seen.len() > 64 {
                return Err(reject());
            }
            let node = self.object_block(current).ok_or_else(reject)?;
            let mut normal = None;
            for edge in node.outgoing() {
                let edge = self.ir.cfg().edge(*edge).map_err(|_| reject())?;
                if matches!(edge.kind(), EdgeKind::Exception(_)) {
                    exceptional = true;
                } else if normal.replace(edge).is_some() {
                    return Err(reject());
                }
            }
            let edge = normal.ok_or_else(reject)?;
            let next = self.ir.cfg().block(edge.target()).map_err(|_| reject())?;
            if edge.kind() != &EdgeKind::Unconditional || next.incoming().len() != 1 {
                return Err(reject());
            }
            current = *next.payload();
            chain.push(current);
            let insn = self.ir.instruction(current).map_err(|_| reject())?;
            let alias = |value: &ValueId| aliases.iter().any(|(v, _)| v == value);
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
                let target = self.resolver.method(*method).ok_or_else(reject)?;
                let args = invoke_arguments::operands(&target, args)?;
                if target.owner != owner
                    || target.name != "<init>"
                    || target.is_static
                    || target.return_type != JavaType::Void
                    || target.parameters.is_empty()
                    || args.len() != target.parameters.len() + 1
                    || args[1..].iter().any(alias)
                    || gap.is_empty()
                    || !pending_nested.is_empty()
                {
                    return Err(reject());
                }
                exceptional |= next.outgoing().iter().any(|id| {
                    self.ir
                        .cfg()
                        .edge(*id)
                        .is_ok_and(|e| matches!(e.kind(), EdgeKind::Exception(_)))
                });
                if exceptional
                    && !(self.resolver.exception_allocation_chain(&chain)
                        || nested.is_empty()
                            && self.resolver.exception_allocation_sequence_chain(&chain))
                {
                    return Err(reject());
                }
                let outer_values = aliases.iter().map(|(value, _)| *value).collect();
                let outer_uses = self.object_read_index(&outer_values);
                let after_outer = self.after_initializer(current)?;
                for uses in outer_uses.values() {
                    if uses.iter().any(|id| {
                        *id != current
                            && !aliases.iter().any(|(_, definition)| definition == id)
                            && !self.initialized_sequence_use(*id, &aliases, &after_outer)
                    }) {
                        return Err(reject());
                    }
                }
                let index = self.object_read_index(&definitions);
                for (_, allocation, _) in &nested {
                    let proof = self.pure_object_plan(*allocation)?;
                    if proof.aliases.iter().any(|(value, _)| {
                        index.get(value).is_some_and(|uses| {
                            uses.iter().any(|id| *id != current && !gap.contains(id))
                        })
                    }) {
                        return Err(reject());
                    }
                }
                let mut captures = HashSet::new();
                for value in &args[2..] {
                    if definitions.contains(value) {
                        captures.insert(*value);
                    }
                }
                let mut after = None;
                for value in &definitions {
                    if let Some(uses) = index.get(value) {
                        for consumer in uses {
                            if *consumer == current || gap.contains(consumer) {
                                continue;
                            }
                            if after.is_none() {
                                after = Some(self.after_initializer(current)?);
                            }
                            if !self.initialized_capture_use(
                                *consumer,
                                *value,
                                after.as_ref().ok_or_else(reject)?,
                            )? {
                                return Err(reject());
                            }
                            captures.insert(*value);
                        }
                    }
                }
                let captures = gap
                    .iter()
                    .filter_map(|id| self.ir.instruction(*id).ok().and_then(sequence_result))
                    .filter(|v| captures.contains(v))
                    .collect();
                return Ok(ObjectPlan {
                    allocation,
                    initializer: current,
                    aliases,
                    owner,
                    argument_gap: gap,
                    argument_allocations: nested,
                    argument_captures: captures,
                });
            }
            if let ManagedInsn::NewInstance { result, .. } = insn {
                // A nested pure pairing attests exact owner, initializer and
                // all pre-initialization reads. Its source NEW remains a real
                // statement inside the outer allocation's argument block.
                let proof = self.pure_object_plan(current)?;
                if proof.aliases.first().map(|a| a.0) != Some(*result)
                    || proof
                        .aliases
                        .iter()
                        .any(|(alias, _)| definitions.contains(alias))
                {
                    return Err(reject());
                }
                definitions.extend(proof.aliases.iter().map(|(alias, _)| *alias));
                nested.push((*result, current, proof.initializer));
                pending_nested.push(proof);
                gap.push(current);
                continue;
            }
            if let ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                result: None,
                ..
            } = insn
            {
                let Some(proof) = pending_nested.last() else {
                    return Err(reject());
                };
                if proof.initializer != current {
                    return Err(reject());
                }
                pending_nested.pop();
                gap.push(current);
                continue;
            }
            // A nested pure plan attests the exact uninitialized identity chain.
            // Carry its synthetic commits without making the reference readable.
            if let ManagedInsn::Move { result, value } = insn
                && pending_nested.last().is_some_and(|proof| {
                    proof.aliases.contains(&(*result, current))
                        && proof.aliases.iter().any(|(alias, _)| alias == value)
                })
            {
                gap.push(current);
                continue;
            }
            if reads(insn).iter().any(|v| {
                pending_nested
                    .iter()
                    .any(|p| p.aliases.iter().any(|(a, _)| a == v))
            }) {
                return Err(reject());
            }
            match insn {
                ManagedInsn::Invoke {
                    method,
                    args,
                    result,
                    ..
                } => {
                    let target = self.resolver.method(*method).ok_or_else(reject)?;
                    if result.is_none()
                        && target.return_type != JavaType::Void
                        && !self.resolver.discarded_invoke_source_target(*method)
                    {
                        return Err(reject());
                    }
                    invoke_arguments::operands(&target, args)?;
                }
                ManagedInsn::FieldGet { field, .. } | ManagedInsn::FieldPut { field, .. } => {
                    self.resolver.field(*field).ok_or_else(reject)?;
                }
                ManagedInsn::Const {
                    value: Constant::ClassLiteral(target),
                    ..
                } if !matches!(
                    self.resolver.instruction_type(*target),
                    Some(JavaType::Class(_) | JavaType::Array(_))
                ) =>
                {
                    // Primitive wrapper.TYPE needs its own initialization proof.
                    return Err(reject());
                }
                _ => {}
            }
            if !sequence_statement(insn) || reads(insn).iter().any(alias) {
                return Err(reject());
            }
            if let Some(value) = sequence_result(insn)
                && !definitions.insert(value)
            {
                return Err(reject());
            }
            gap.push(current);
        }
    }
    /// Retain the accepted planner route alongside its immutable IR trace.
    pub(in crate::object_plan) fn object_argument_sequence(&self, plan: &ObjectPlan) -> bool {
        self.object_planning
            .sequences
            .borrow()
            .contains(&plan.allocation)
    }
    pub(in crate::object_plan) fn object_sequence_expressions(
        &mut self,
        plan: &ObjectPlan,
        signature: &JavaMethod,
        captures: &HashMap<ValueId, (String, JavaType)>,
    ) -> Result<HashMap<ValueId, (String, JavaType)>, Unsupported> {
        let reject = || Unsupported::Instruction("allocation statement sequence lowering".into());
        let ManagedInsn::Invoke { method, args, .. } = self
            .ir
            .instruction(plan.initializer)
            .map_err(|_| reject())?
        else {
            return Err(reject());
        };
        let target = self.resolver.method(*method).ok_or_else(reject)?;
        let args = invoke_arguments::operands(&target, args)?;
        let first = *args.get(1).ok_or_else(reject)?;
        let planning = std::rc::Rc::new(ObjectPlanning::default());
        let mut nested_owners: HashMap<InsnId, Vec<ValueId>> = HashMap::new();
        for (value, allocation, _) in &plan.argument_allocations {
            let nested = self
                .argument_object_plan(*allocation)
                .or_else(|_| self.object_plan(*allocation))?;
            planning
                .plans
                .borrow_mut()
                .insert(*allocation, Ok(nested.clone()));
            for id in nested.argument_gap {
                nested_owners.entry(id).or_default().push(*value);
            }
        }
        let _ = planning.argument_owners.set(nested_owners);
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
            object_planning: planning,
        };
        for id in &plan.argument_gap {
            if let Some(value) = sequence_result(self.ir.instruction(*id).map_err(|_| reject())?) {
                if self.values.contains_key(&value) && !self.hoisted.contains(&value) {
                    return Err(reject());
                }
                scratch.values.remove(&value);
                if let Some(slot) = captures.get(&value) {
                    scratch.values.insert(value, slot.clone());
                    scratch.hoisted.insert(value);
                }
            }
        }
        for id in &plan.argument_gap {
            scratch.current = Some(*id);
            if scratch.object_argument_instruction(*id) {
                continue;
            }
            scratch.instruction(self.ir.instruction(*id).map_err(|_| reject())?, signature)?;
        }
        let (yielded, ty) =
            scratch.operand(first, target.parameters.first().ok_or_else(reject)?)?;
        self.compatible(&ty, &target.parameters[0])?;
        let mut expressions = captures.clone();
        // Int selector and a single default arm create no helper frame, boxing,
        // extra object allocation or alternate normal path. Every physical
        // producer is a statement, with the standard emitter's original calls.
        let block = scratch.lines.join(" ");
        // Block locals must not shadow the root local whose initializer owns
        // this switch. Advance the parent name allocator before defining NEW.
        self.next = scratch.next;
        let sequence = format!(
            "(switch (0) {{ default -> {{ /* compiler scaffold: original allocation argument sequence */ {block} yield {yielded}; }} }})"
        );
        expressions.insert(first, (sequence, ty));
        Ok(expressions)
    }
}

#[cfg(test)]
#[path = "object_sequence_phi_availability_tests.rs"]
mod phi_availability_tests;

#[cfg(test)]
#[path = "object_sequence_nested_alias_tests.rs"]
mod nested_alias_tests;

#[cfg(test)]
#[path = "object_sequence_capture_phi_tests.rs"]
mod capture_phi_tests;
