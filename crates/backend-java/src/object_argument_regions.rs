//! Canonical acyclic argument regions lowered inside NEW's first argument.
//! Original edges/Phi copies remain in the switch expression, after allocation.
use super::*;
use crate::branching::Step;
#[path = "object_local_argument_region.rs"]
mod local_region;

#[derive(Clone)]
pub(super) struct ArgumentRegion {
    initializer: InsnId,
    value: ValueId,
    owner: String,
    steps: Vec<Step>,
    instructions: HashSet<InsnId>,
    conditions: HashSet<ValueId>,
    branches: HashSet<InsnId>,
    edges: HashSet<decomp_core::ids::EdgeId>,
    definitions: HashSet<ValueId>,
    children: Vec<ObjectPlan>,
}
fn denied() -> Unsupported {
    Unsupported::Instruction("unproved branching allocation arguments".into())
}
fn collect(steps: &[Step], plan: &mut ArgumentRegion) -> Result<(), Unsupported> {
    for step in steps {
        match step {
            Step::Instruction(id) => {
                plan.instructions.insert(*id);
            }
            Step::If {
                condition,
                then_steps,
                else_steps,
            } => {
                plan.conditions.insert(*condition);
                collect(then_steps, plan)?;
                collect(else_steps, plan)?;
            }
            Step::Copies { edge, .. } => {
                plan.edges.insert(*edge);
            }
            _ => return Err(denied()),
        }
    }
    Ok(())
}
fn span(steps: &[Step], root: InsnId, init: InsnId) -> Option<Vec<Step>> {
    if let Some(start) = steps
        .iter()
        .position(|s| matches!(s,Step::Instruction(id) if *id==root))
        && let Some(end) = steps
            .iter()
            .position(|s| matches!(s,Step::Instruction(id) if *id==init))
        && end > start
    {
        return Some(steps[start + 1..end].to_vec());
    }
    for step in steps {
        if let Step::If {
            then_steps,
            else_steps,
            ..
        } = step
            && let Some(found) =
                span(then_steps, root, init).or_else(|| span(else_steps, root, init))
        {
            return Some(found);
        }
    }
    None
}
fn exact_if(steps: &[Step], candidate: &Step) -> bool {
    steps.iter().any(|step| match step {
        Step::If {
            then_steps,
            else_steps,
            ..
        } => {
            step == candidate || exact_if(then_steps, candidate) || exact_if(else_steps, candidate)
        }
        _ => false,
    })
}
impl<R: JavaResolver> Emitter<'_, R> {
    fn argument_region(&self, root: InsnId) -> Result<ArgumentRegion, Unsupported> {
        if let Some(result) = self.object_planning.regions.borrow().get(&root) {
            return result.clone();
        }
        let result = self.build_argument_region(root);
        self.object_planning
            .regions
            .borrow_mut()
            .insert(root, result.clone());
        result
    }
    fn build_argument_region(&self, root: InsnId) -> Result<ArgumentRegion, Unsupported> {
        let ManagedInsn::NewInstance {
            result: value,
            class,
        } = self.ir.instruction(root).map_err(|_| denied())?
        else {
            return Err(denied());
        };
        let owner = self.resolver.class(*class).ok_or_else(denied)?;
        let mut initializers = vec![];
        for (id, insn) in self.ir.instructions() {
            if let ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                result: None,
                method,
                args,
            } = insn
                && args.first() == Some(value)
                && self
                    .resolver
                    .method(*method)
                    .is_some_and(|m| m.name == "<init>")
            {
                initializers.push(id);
            }
        }
        let [initializer] = initializers.as_slice() else {
            return Err(denied());
        };
        let declarations = self.declarations.keys().copied().collect();
        let bindings = self.parameters.iter().copied().collect::<Vec<_>>();
        let steps = match decomp_structure::structure_extended(self.ir.cfg()) {
            Ok(structure) => {
                let canonical = branching::plan_extended_checked(
                    self.ir,
                    &structure.root,
                    &bindings,
                    &declarations,
                )
                .map_err(|_| denied())?;
                span(&canonical.steps, root, *initializer).ok_or_else(denied)?
            }
            Err(_) => local_region::steps(self, root, *initializer, &bindings, &declarations)?,
        };
        let mut plan = ArgumentRegion {
            initializer: *initializer,
            value: *value,
            owner,
            steps,
            instructions: HashSet::new(),
            conditions: HashSet::new(),
            branches: HashSet::new(),
            edges: HashSet::new(),
            definitions: HashSet::new(),
            children: vec![],
        };
        collect(&plan.steps.clone(), &mut plan)?;
        if plan.conditions.is_empty() {
            return Err(denied());
        }
        for condition in &plan.conditions {
            let original = self
                .ir
                .instructions()
                .iter()
                .filter_map(|(id, insn)| {
                    matches!(insn, ManagedInsn::Branch {condition:Some(c),..} if c == condition)
                        .then_some(id)
                })
                .collect::<Vec<_>>();
            let [branch] = original.as_slice() else {
                return Err(denied());
            };
            plan.branches.insert(*branch);
        }
        for id in &plan.instructions {
            if matches!(
                self.ir.instruction(*id),
                Ok(ManagedInsn::NewInstance { .. })
            ) {
                let child = self.object_plan(*id).map_err(|_| denied())?;
                if !plan.instructions.contains(&child.initializer)
                    || child
                        .aliases
                        .iter()
                        .any(|(_, id)| !plan.instructions.contains(id))
                    || child
                        .argument_gap
                        .iter()
                        .any(|id| !plan.instructions.contains(id))
                {
                    return Err(denied());
                }
                // Each allocated reference has exactly its verified original
                // initializer, including off-CFG observations. Do not admit a
                // second initializer merely because another path is hidden.
                let mut initializers = vec![];
                for (candidate, insn) in self.ir.instructions() {
                    if let ManagedInsn::Invoke {
                        kind: InvokeKind::Special,
                        method,
                        args,
                        ..
                    } = insn
                        && args.first().is_some_and(|receiver| {
                            child.aliases.iter().any(|(alias, _)| alias == receiver)
                        })
                        && self
                            .resolver
                            .method(*method)
                            .is_some_and(|m| m.name == "<init>")
                    {
                        initializers.push(candidate);
                    }
                }
                if initializers.as_slice() != [child.initializer] {
                    return Err(denied());
                }
                plan.children.push(child);
            }
        }
        for id in &plan.instructions {
            let insn = self.ir.instruction(*id).map_err(|_| denied())?;
            match insn {
                ManagedInsn::Const { result, .. }
                | ManagedInsn::Move { result, .. }
                | ManagedInsn::Cast { result, .. }
                | ManagedInsn::Compare { result, .. }
                | ManagedInsn::Arithmetic { result, .. }
                | ManagedInsn::FieldGet { result, .. }
                | ManagedInsn::NewInstance { result, .. }
                | ManagedInsn::NewFilledArray {
                    result: Some(result),
                    ..
                }
                | ManagedInsn::ArrayGet { result, .. }
                | ManagedInsn::ArrayLength { result, .. } => {
                    plan.definitions.insert(*result);
                }
                ManagedInsn::Invoke {
                    kind: InvokeKind::Static | InvokeKind::Virtual | InvokeKind::Interface,
                    result,
                    ..
                } => {
                    if let ManagedInsn::Invoke {
                        method,
                        result: None,
                        ..
                    } = insn
                        && self
                            .resolver
                            .method(*method)
                            .ok_or_else(denied)?
                            .return_type
                            != JavaType::Void
                        && !self.resolver.discarded_invoke_source_target(*method)
                    {
                        return Err(denied());
                    }
                    plan.definitions.extend(result);
                }
                ManagedInsn::Invoke {
                    kind: InvokeKind::Special,
                    result: None,
                    ..
                } if plan.children.iter().any(|child| child.initializer == *id) => {}
                _ => return Err(denied()),
            }
        }
        fn phi_destinations(steps: &[Step], out: &mut HashSet<ValueId>) {
            for s in steps {
                match s {
                    Step::Copies { copies, .. } => out.extend(copies.iter().map(|c| c.destination)),
                    Step::If {
                        then_steps,
                        else_steps,
                        ..
                    } => {
                        phi_destinations(then_steps, out);
                        phi_destinations(else_steps, out)
                    }
                    _ => {}
                }
            }
        }
        phi_destinations(&plan.steps, &mut plan.definitions);
        let root_block = self
            .ir
            .cfg()
            .block_ids()
            .find(|id| self.ir.cfg().block(*id).is_ok_and(|b| *b.payload() == root))
            .ok_or_else(denied)?;
        let mut pending = vec![root_block];
        let mut visited = HashSet::new();
        let dominance = decomp_core::analysis::dominators(self.ir.cfg());
        while let Some(block_id) = pending.pop() {
            if !visited.insert(block_id) {
                continue;
            }
            if visited.len() > 128 || !dominance.dominates(root_block, block_id) {
                return Err(denied());
            }
            let block = self.ir.cfg().block(block_id).map_err(|_| denied())?;
            let id = *block.payload();
            let insn = self.ir.instruction(id).map_err(|_| denied())?;
            let belongs = id == root
                || id == *initializer
                || plan.instructions.contains(&id)
                || plan.branches.contains(&id)
                || matches!(insn,ManagedInsn::Phi{result,..} if plan.definitions.contains(result))
                || matches!(
                    insn,
                    ManagedInsn::Branch {
                        condition: None,
                        ..
                    }
                );
            if !belongs {
                return Err(denied());
            }
            for edge_id in block.outgoing() {
                let edge = self.ir.cfg().edge(*edge_id).map_err(|_| denied())?;
                if matches!(edge.kind(), EdgeKind::Exception(_)) {
                    return Err(denied());
                }
                if id != *initializer {
                    pending.push(edge.target());
                }
            }
            if id != *initializer && block.outgoing().is_empty() {
                return Err(denied());
            }
        }
        let init_block = self.object_block(*initializer).ok_or_else(denied)?;
        let init_id = self
            .ir
            .cfg()
            .block_ids()
            .find(|id| {
                self.ir
                    .cfg()
                    .block(*id)
                    .is_ok_and(|b| std::ptr::eq(b, init_block))
            })
            .ok_or_else(denied)?;
        let dominance = decomp_core::analysis::dominators(self.ir.cfg());
        for (id, insn) in self.ir.instructions() {
            let reads = arguments::reads(insn);
            if id != *initializer && reads.contains(value) {
                let block_id = self
                    .ir
                    .cfg()
                    .block_ids()
                    .find(|bid| self.ir.cfg().block(*bid).is_ok_and(|b| *b.payload() == id))
                    .ok_or_else(denied)?;
                if !dominance.dominates(init_id, block_id) {
                    return Err(denied());
                }
            }
            if id != *initializer
                && reads.iter().any(|v| plan.definitions.contains(v))
                && !plan.instructions.contains(&id)
                && !plan.branches.contains(&id)
                && !matches!(insn,ManagedInsn::Phi{result,..} if plan.definitions.contains(result))
            {
                return Err(denied());
            }
        }
        Ok(plan)
    }
    /// Suppress only an already atomically emitted canonical argument region.
    pub(crate) fn folded_argument_region_step(&self, step: &Step) -> Result<bool, Unsupported> {
        for plan in self
            .object_planning
            .regions
            .borrow()
            .values()
            .filter_map(|p| p.as_ref().ok())
        {
            if self.get(plan.value).is_err() || self.pending_allocation(plan.value) {
                continue;
            }
            let covered = match step {
                Step::Instruction(id) => *id == plan.initializer || plan.instructions.contains(id),
                Step::If { .. } => exact_if(&plan.steps, step),
                Step::Copies { edge, .. } => plan.edges.contains(edge),
                _ => false,
            };
            if covered {
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// The original condition has executed inside a fully consumed argument
    /// region. Residual forward scaffolding may take either proved convergent
    /// empty path; no source condition or edge copy is evaluated twice.
    pub(crate) fn folded_argument_region_branch(&self, id: InsnId) -> bool {
        self.object_planning
            .regions
            .borrow()
            .values()
            .filter_map(|p| p.as_ref().ok())
            .any(|plan| {
                plan.branches.contains(&id)
                    && self.get(plan.value).is_ok()
                    && !self.pending_allocation(plan.value)
            })
    }
    pub(in crate::object_plan) fn branching_argument_instruction(
        &mut self,
        insn: &ManagedInsn,
        signature: &JavaMethod,
        current: InsnId,
    ) -> Result<bool, Unsupported> {
        if !matches!(insn, ManagedInsn::NewInstance { .. }) || self.object_plan(current).is_ok() {
            return Ok(false);
        }
        let Ok(plan) = self.argument_region(current) else {
            return Ok(false);
        };
        let ManagedInsn::Invoke { method, args, .. } = self
            .ir
            .instruction(plan.initializer)
            .map_err(|_| denied())?
        else {
            return Err(denied());
        };
        let target = self.resolver.method(*method).ok_or_else(denied)?;
        let operands = invoke_arguments::operands(&target, args)?;
        let flags = self.resolver.method_access(*method).ok_or_else(denied)?;
        if target.owner != plan.owner
            || target.name != "<init>"
            || target.is_static
            || target.return_type != JavaType::Void
            || target.parameters.is_empty()
            || flags & (8 | 0x100 | 0x400) != 0
            || flags & 1 == 0 && target.owner != signature.owner
        {
            return Err(denied());
        }
        checked_exceptions::check(self.resolver, &target.throws, &signature.throws)
            .map_err(|_| denied())?;
        let first = *operands.get(1).ok_or_else(denied)?;
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
            object_planning: std::rc::Rc::new(ObjectPlanning::default()),
        };
        let mut child_owners: HashMap<InsnId, Vec<ValueId>> = HashMap::new();
        for child in &plan.children {
            scratch
                .object_planning
                .plans
                .borrow_mut()
                .insert(child.allocation, Ok(child.clone()));
            if self.object_argument_sequence(child) {
                scratch
                    .object_planning
                    .sequences
                    .borrow_mut()
                    .insert(child.allocation);
            }
            for id in &child.argument_gap {
                child_owners
                    .entry(*id)
                    .or_default()
                    .push(child.aliases[0].0);
            }
        }
        let _ = scratch.object_planning.argument_owners.set(child_owners);
        for value in &plan.definitions {
            scratch.values.remove(value);
        }
        // Edge copies require a predeclared destination even when the phi is
        // consumed by a nested call rather than by a later outer argument.
        for value in &plan.definitions {
            if self.hoisted.contains(value)
                && self.ir.value(*value).is_ok_and(|v| {
                    matches!(self.ir.instruction(v.def()), Ok(ManagedInsn::Phi { .. }))
                })
            {
                scratch.values.insert(*value, self.get(*value)?);
                scratch.hoisted.insert(*value);
            }
        }
        for value in operands
            .iter()
            .skip(2)
            .filter(|v| plan.definitions.contains(v))
        {
            let slot = self.values.get(value).cloned().ok_or_else(denied)?;
            if !self.hoisted.contains(value) {
                return Err(denied());
            }
            scratch.values.insert(*value, slot);
            scratch.hoisted.insert(*value);
        }
        render_steps_at(&mut scratch, &plan.steps, signature, 0)?;
        let (yielded, ty) = scratch.operand(first, &target.parameters[0])?;
        self.compatible(&ty, &target.parameters[0])?;
        let mut expressions = vec![format!(
            "({}) (switch (0) {{ default -> {{ {} yield {yielded}; }} }})",
            target.parameters[0].source()?,
            scratch.lines.join(" ")
        )];
        for (value, expected) in operands
            .iter()
            .skip(2)
            .zip(target.parameters.iter().skip(1))
        {
            let (source, ty) = scratch.operand(*value, expected)?;
            self.compatible(&ty, expected)?;
            expressions.push(format!("({}) ({source})", expected.source()?));
        }
        let expression = format!(
            "new {}({})",
            qualified(&plan.owner)?,
            expressions.join(", ")
        );
        let saved_next = self.next;
        let saved_reserved = self.reserved.clone();
        let saved_values = self.values.clone();
        let saved_lines = self.lines.clone();
        let saved_hoisted = self.hoisted.clone();
        self.next = scratch.next;
        self.reserved = scratch.reserved;
        if let Err(error) = self.define(plan.value, expression, JavaType::Class(plan.owner)) {
            self.next = saved_next;
            self.reserved = saved_reserved;
            self.values = saved_values;
            self.lines = saved_lines;
            self.hoisted = saved_hoisted;
            return Err(error);
        }
        self.hoisted.remove(&plan.value);
        Ok(true)
    }
}
