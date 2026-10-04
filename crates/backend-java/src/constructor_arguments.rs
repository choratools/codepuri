//! Bounded original-order constructor argument allocations. No statements are
//! moved ahead of a Java initializer call; only single-consumer expression trees
//! whose complete allocation/initialization trace agrees are admissible.
use super::*;

#[derive(Default)]
pub(super) struct PrefixArguments {
    physical: Vec<InsnId>,
    roots: HashMap<ValueId, Vec<InsnId>>,
    allocations: HashMap<InsnId, ValueId>,
    native_strings: HashSet<ValueId>,
    native_string_events: HashSet<InsnId>,
    conditional: bool,
}
impl PrefixArguments {
    pub(super) fn literal(
        &mut self,
        value: ValueId,
        id: InsnId,
        native_string: bool,
    ) -> Result<(), Unsupported> {
        if self.physical.len() >= 64 || self.roots.insert(value, vec![id]).is_some() {
            return Err(Unsupported::Instruction(
                "pre-chain literal expression proof".into(),
            ));
        }
        if native_string {
            self.native_strings.insert(value);
            self.native_string_events.insert(id);
        }
        self.physical.push(id);
        Ok(())
    }
    pub(super) fn allocation<R: ConstructorResolver>(
        &mut self,
        e: &Emitter<'_, R>,
        id: InsnId,
    ) -> Result<(), Unsupported> {
        let fail = || Unsupported::Instruction("pre-chain allocation expression proof".into());
        if self.physical.len() >= 64 {
            return Err(fail());
        }
        let plan = e.object_plan(id)?;
        if plan.aliases.len() != 1 || !plan.argument_gap.is_empty() {
            return Err(fail());
        }
        let value = plan.aliases[0].0;
        if self.allocations.insert(id, value).is_some() {
            return Err(fail());
        }
        self.physical.push(id);
        Ok(())
    }
    pub(super) fn copy(&mut self, result: ValueId, value: ValueId) {
        if let Some(events) = self.roots.get(&value).cloned() {
            self.roots.insert(result, events);
        }
        if self.native_strings.contains(&value) {
            self.native_strings.insert(result);
        }
    }
    pub(super) fn expression<R: ConstructorResolver>(
        &mut self,
        e: &mut Emitter<'_, R>,
        id: InsnId,
        signature: &JavaMethod,
    ) -> Result<(), Unsupported> {
        let fail = || Unsupported::Instruction("pre-chain expression proof".into());
        if self.physical.len() >= 64 {
            return Err(fail());
        }
        let (result, dependencies, expression, ty) = match e
            .ir
            .instruction(id)
            .map_err(|_| fail())?
        {
            ManagedInsn::NewArray {
                result,
                element,
                length,
            } => {
                let target = e.resolver.instruction_type(*element).ok_or_else(fail)?;
                let ty = e.ty(*result)?;
                if target != ty && JavaType::Array(Box::new(target)) != ty {
                    return Err(fail());
                }
                let JavaType::Array(component) = &ty else {
                    return Err(fail());
                };
                let (length_expression, length_type) = e.operand(*length, &JavaType::Int)?;
                if length_type != JavaType::Int {
                    return Err(fail());
                }
                let mut base = component.as_ref();
                let mut dimensions = String::new();
                while let JavaType::Array(inner) = base {
                    dimensions.push_str("[]");
                    base = inner;
                }
                (
                    *result,
                    vec![*length],
                    format!("new {}[{length_expression}]{dimensions}", base.source()?),
                    ty,
                )
            }
            ManagedInsn::Invoke {
                kind: kind @ (InvokeKind::Static | InvokeKind::Virtual | InvokeKind::Interface),
                result: Some(result),
                method,
                args,
            } => {
                let declaration = e.resolver.method(*method).ok_or_else(fail)?;
                let flags = e.resolver.method_access(*method).ok_or_else(fail)?;
                let static_call = *kind == InvokeKind::Static;
                if declaration.is_static != static_call
                    || (flags & 8 != 0) != static_call
                    || (!static_call && flags & 7 != 1)
                    || declaration.return_type == JavaType::Void
                    || !constructor_call_access(&signature.owner, &declaration.owner, flags)
                    || declaration
                        .throws
                        .iter()
                        .any(|t| !e.resolver.exception_covered(t, &signature.throws))
                {
                    return Err(fail());
                }
                member(&declaration.name)?;
                let dependencies = invoke_arguments::operands(&declaration, args)?;
                let target = if static_call {
                    qualified(&declaration.owner)?
                } else {
                    let receiver = *dependencies.first().ok_or_else(fail)?;
                    let (expression, ty) = e.get(receiver)?;
                    let owner = JavaType::Class(declaration.owner.clone());
                    e.compatible(&ty, &owner)?;
                    if expression == "this" {
                        return Err(fail());
                    }
                    if ty == owner {
                        expression
                    } else {
                        format!("(({}) {expression})", owner.source()?)
                    }
                };
                let offset = usize::from(!static_call);
                let mut actual = Vec::new();
                for (value, expected) in dependencies[offset..].iter().zip(&declaration.parameters)
                {
                    let (expression, ty) = e.operand(*value, expected)?;
                    if expression == "this" || !e.resolver.assignable(&ty, expected) {
                        return Err(fail());
                    }
                    actual.push(format!("({}) ({expression})", expected.source()?));
                }
                let expression = format!("{}.{}({})", target, declaration.name, actual.join(", "));
                (*result, dependencies, expression, declaration.return_type)
            }
            ManagedInsn::FieldGet {
                result,
                field,
                object: None,
            } => {
                let declaration = e.resolver.field(*field).ok_or_else(fail)?;
                let flags = e.resolver.field_access(*field).ok_or_else(fail)?;
                let visible = constructor_call_access(&signature.owner, &declaration.owner, flags)
                    || (flags & 7 == 4
                        && e.resolver.assignable(
                            &JavaType::Class(signature.owner.clone()),
                            &JavaType::Class(declaration.owner.clone()),
                        ));
                if !declaration.is_static || flags & 8 == 0 || !visible {
                    return Err(fail());
                }
                let expression = e.field_target(None, &declaration, *field, &signature.owner)?;
                (*result, Vec::new(), expression, declaration.ty)
            }
            _ => return Err(fail()),
        };
        let mut events = Vec::new();
        for value in dependencies {
            if let Some(trace) = self.roots.get(&value) {
                events.extend(trace);
            }
        }
        events.push(id);
        inline(e, result, expression, ty)?;
        self.roots.insert(result, events);
        self.physical.push(id);
        Ok(())
    }
    pub(super) fn initialize<R: ConstructorResolver>(
        &mut self,
        e: &mut Emitter<'_, R>,
        id: InsnId,
        signature: &JavaMethod,
    ) -> Result<(), Unsupported> {
        let fail = || Unsupported::Instruction("pre-chain initializer expression proof".into());
        let ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            method,
            args,
            result: None,
        } = e.ir.instruction(id).map_err(|_| fail())?
        else {
            return Err(fail());
        };
        let receiver = *args.first().ok_or_else(fail)?;
        let root = e.allocation_root(receiver).ok_or_else(fail)?;
        let plan = e.object_plan(root)?;
        if plan.initializer != id
            || plan.aliases.len() != 1
            || !plan.argument_gap.is_empty()
            || self.allocations.get(&root) != Some(&receiver)
            || self.roots.contains_key(&receiver)
        {
            return Err(fail());
        }
        let target = e.resolver.method(*method).ok_or_else(fail)?;
        let args = invoke_arguments::operands(&target, args)?;
        let flags = e.resolver.method_access(*method).ok_or_else(fail)?;
        if flags & (8 | 0x80 | 0x400) != 0
            || target.owner != plan.owner
            || target.parameters.len() + 1 != args.len()
            || !constructor_call_access(&signature.owner, &target.owner, flags)
            || target
                .throws
                .iter()
                .any(|t| !e.resolver.exception_covered(t, &signature.throws))
        {
            return Err(fail());
        }
        let mut events = vec![root];
        let mut actual = Vec::new();
        for (value, ty) in args[1..].iter().zip(&target.parameters) {
            let (expression, actual_ty) = e.operand(*value, ty)?;
            if expression == "this" || !e.resolver.assignable(&actual_ty, ty) {
                return Err(fail());
            }
            if let Some(trace) = self.roots.get(value) {
                events.extend(trace);
            }
            actual.push(format!("({}) ({expression})", ty.source()?));
        }
        events.push(id);
        self.physical.push(id);
        let old = e.current;
        e.current = Some(root);
        let inserted = inline(
            e,
            receiver,
            format!("new {}({})", qualified(&plan.owner)?, actual.join(", ")),
            JavaType::Class(plan.owner),
        );
        e.current = old;
        inserted?;
        self.roots.insert(receiver, events);
        Ok(())
    }
    pub(super) fn diamond<R: ConstructorResolver>(
        &mut self,
        e: &mut Emitter<'_, R>,
        signature: &JavaMethod,
        branch: decomp_core::ids::BlockId,
        aliases: &HashSet<ValueId>,
        seen: &mut HashSet<decomp_core::ids::BlockId>,
    ) -> Result<decomp_core::ids::BlockId, Unsupported> {
        let fail = || Unsupported::ControlFlow("unproved pre-chain conditional diamond".into());
        if self.conditional || !self.physical.is_empty() {
            return Err(fail());
        }
        let cfg = e.ir.cfg();
        if cfg.block_count() > 256 {
            return Err(fail());
        }
        let node = cfg.block(branch).map_err(|_| fail())?;
        let branch_id = *node.payload();
        let ManagedInsn::Branch {
            condition: Some(condition),
            then_edge,
            else_edge: Some(else_edge),
        } = e.ir.instruction(branch_id).map_err(|_| fail())?
        else {
            return Err(fail());
        };
        if node.outgoing().len() != 2
            || !node.outgoing().contains(then_edge)
            || !node.outgoing().contains(else_edge)
            || then_edge == else_edge
        {
            return Err(fail());
        }
        let yes_edge = cfg.edge(*then_edge).map_err(|_| fail())?;
        let no_edge = cfg.edge(*else_edge).map_err(|_| fail())?;
        if yes_edge.kind() != &decomp_core::cfg::EdgeKind::BranchTrue
            || no_edge.kind() != &decomp_core::cfg::EdgeKind::BranchFalse
        {
            return Err(fail());
        }
        let merge = decomp_core::analysis::post_dominators(cfg)
            .immediate_post_dominator(branch)
            .ok_or_else(fail)?;
        let merge_node = cfg.block(merge).map_err(|_| fail())?;
        let phi_id = *merge_node.payload();
        let ManagedInsn::Phi { result, inputs } = e.ir.instruction(phi_id).map_err(|_| fail())?
        else {
            return Err(fail());
        };
        if inputs.len() != 2 || merge_node.incoming().len() != 2 || merge_node.outgoing().len() != 1
        {
            return Err(fail());
        }
        let next_edge = cfg.edge(merge_node.outgoing()[0]).map_err(|_| fail())?;
        if next_edge.kind() != &decomp_core::cfg::EdgeKind::Unconditional {
            return Err(fail());
        }
        let next = next_edge.target();
        let next_node = cfg.block(next).map_err(|_| fail())?;
        let ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method,
            args,
        } = e.ir.instruction(*next_node.payload()).map_err(|_| fail())?
        else {
            return Err(fail());
        };
        if next_node.incoming().len() != 1 || !args.first().is_some_and(|v| aliases.contains(v)) {
            return Err(fail());
        }
        let target = e.resolver.initializer(*method).ok_or_else(fail)?;
        let args = invoke_arguments::operands(&target.method, args)?;
        if target.method.parameters.len() + 1 != args.len()
            || args[1..].iter().filter(|v| **v == *result).count() != 1
        {
            return Err(fail());
        }
        let position = args[1..]
            .iter()
            .position(|v| v == result)
            .ok_or_else(fail)?;
        let ty = target.method.parameters[position].clone();
        if aliases.contains(result)
            || !pure_condition(e, *condition, aliases, &mut HashSet::new(), 0)
        {
            return Err(fail());
        }
        let (condition_expression, condition_type) = e.get(*condition)?;
        if condition_type != JavaType::Boolean {
            return Err(fail());
        }
        let base = e.values.clone();
        let mut expressions = Vec::new();
        let mut path_blocks = HashSet::new();
        for first in [*then_edge, *else_edge] {
            e.values = base.clone();
            let mut arm = PrefixArguments::default();
            let mut edge_id = first;
            let mut steps = 0;
            loop {
                let edge = cfg.edge(edge_id).map_err(|_| fail())?;
                let block = edge.target();
                if block == merge {
                    break;
                }
                steps += 1;
                if steps > 32 || seen.contains(&block) || !path_blocks.insert(block) {
                    return Err(fail());
                }
                let arm_node = cfg.block(block).map_err(|_| fail())?;
                if arm_node.incoming() != [edge_id] || arm_node.outgoing().len() != 1 {
                    return Err(fail());
                }
                let id = *arm_node.payload();
                e.current = Some(id);
                match e.ir.instruction(id).map_err(|_| fail())? {
                    ManagedInsn::NewInstance { .. } => arm.allocation(e, id)?,
                    ManagedInsn::NewArray { .. } | ManagedInsn::FieldGet { object: None, .. } => {
                        arm.expression(e, id, signature)?
                    }
                    ManagedInsn::Invoke {
                        kind: InvokeKind::Static | InvokeKind::Virtual | InvokeKind::Interface,
                        result: Some(_),
                        args,
                        ..
                    } if !args.iter().any(|v| aliases.contains(v)) => {
                        arm.expression(e, id, signature)?
                    }
                    ManagedInsn::Invoke {
                        kind: InvokeKind::Special,
                        args,
                        ..
                    } if args.first().is_some_and(|v| !aliases.contains(v)) => {
                        arm.initialize(e, id, signature)?
                    }
                    _ => return Err(fail()),
                }
                edge_id = arm_node.outgoing()[0];
                if cfg.edge(edge_id).map_err(|_| fail())?.kind()
                    != &decomp_core::cfg::EdgeKind::Unconditional
                {
                    return Err(fail());
                }
            }
            let value = inputs
                .iter()
                .find(|(edge, _)| *edge == edge_id)
                .map(|(_, v)| *v)
                .ok_or_else(fail)?;
            if aliases.contains(&value) {
                return Err(fail());
            }
            arm.check(e, &[value])?;
            let (expression, actual) = e.operand(value, &ty)?;
            if !e.resolver.assignable(&actual, &ty) {
                return Err(fail());
            }
            expressions.push(format!("({}) ({expression})", ty.source()?));
        }
        e.values = base;
        e.current = Some(phi_id);
        inline(
            e,
            *result,
            format!(
                "(({condition_expression}) ? {} : {})",
                expressions[0], expressions[1]
            ),
            ty,
        )?;
        self.physical.push(branch_id);
        self.roots.insert(*result, vec![branch_id]);
        self.conditional = true;
        seen.extend(path_blocks);
        seen.insert(merge);
        Ok(next)
    }
    pub(super) fn check<R: ConstructorResolver>(
        &self,
        e: &Emitter<'_, R>,
        arguments: &[ValueId],
    ) -> Result<(), Unsupported> {
        let fail = || Unsupported::Instruction("pre-chain argument order/escape".into());
        let mut resolved_strings = HashSet::new();
        let events = arguments
            .iter()
            .flat_map(|v| self.roots.get(v).into_iter().flatten().copied())
            .filter(|id| !self.native_string_events.contains(id) || resolved_strings.insert(*id))
            .collect::<Vec<_>>();
        if events != self.physical {
            return Err(fail());
        }
        // Count logical reads once, preserving duplicate logical operands while
        // excluding only attested duplicate physical high words. Unknown or
        // invalid signatures retain every raw read conservatively.
        let mut targets = HashMap::new();
        let mut counts: HashMap<ValueId, usize> = HashMap::new();
        for (_, insn) in e.ir.instructions() {
            let mut operands = reads(insn);
            if !operands.iter().any(|v| self.roots.contains_key(v)) {
                continue;
            }
            if let ManagedInsn::Invoke { method, args, .. } = insn {
                let target = targets.entry(*method).or_insert_with(|| {
                    e.resolver
                        .method(*method)
                        .or_else(|| e.resolver.initializer(*method).map(|i| i.method))
                });
                operands = target
                    .as_ref()
                    .and_then(|t| invoke_arguments::operands(t, args).ok())
                    .unwrap_or_else(|| args.clone());
            }
            for v in operands.into_iter().filter(|v| self.roots.contains_key(v)) {
                *counts.entry(v).or_default() += 1;
            }
        }
        for value in self.roots.keys() {
            // Receiver initialization is one additional logical use.
            let uses = counts.get(value).copied().unwrap_or_default();
            let allocation = self.allocations.values().any(|v| v == value);
            if if self.native_strings.contains(value) {
                uses == 0
            } else {
                uses != if allocation { 2 } else { 1 }
            } {
                return Err(fail());
            }
        }
        Ok(())
    }
}
pub(super) fn constructor_call_access(caller: &str, owner: &str, flags: u32) -> bool {
    flags & 7 == 1
        || caller == owner
        || (flags & 7 == 0
            && caller.rsplit_once('.').map(|p| p.0) == owner.rsplit_once('.').map(|p| p.0))
}
fn reads(insn: &ManagedInsn) -> Vec<ValueId> {
    match insn {
        ManagedInsn::Move { value, .. }
        | ManagedInsn::Neg { value, .. }
        | ManagedInsn::Not { value, .. }
        | ManagedInsn::Cast { value, .. }
        | ManagedInsn::InstanceOf { value, .. }
        | ManagedInsn::Throw { value } => vec![*value],
        ManagedInsn::Arithmetic { lhs, rhs, .. }
        | ManagedInsn::Compare { lhs, rhs, .. }
        | ManagedInsn::OrderingCompare { lhs, rhs, .. } => {
            vec![*lhs, *rhs]
        }
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
        ManagedInsn::ArrayLength { array, .. } => vec![*array],
        ManagedInsn::NewArray { length, .. } => vec![*length],
        ManagedInsn::NewFilledArray { elements, .. } => elements.clone(),
        ManagedInsn::FillArray { array, values } => std::iter::once(*array)
            .chain(values.iter().copied())
            .collect(),
        ManagedInsn::Invoke { args, .. } => args.clone(),
        ManagedInsn::MonitorEnter { object } | ManagedInsn::MonitorExit { object } => vec![*object],
        ManagedInsn::Branch { condition, .. } => condition.iter().copied().collect(),
        ManagedInsn::Switch { key, .. } => vec![*key],
        ManagedInsn::Return { value } => value.iter().copied().collect(),
        ManagedInsn::Opaque { reads, .. } => reads.clone(),
        ManagedInsn::Phi { inputs, .. } => inputs.iter().map(|i| i.1).collect(),
        ManagedInsn::Const { .. } | ManagedInsn::NewInstance { .. } => Vec::new(),
    }
}

fn pure_condition<R: ConstructorResolver>(
    e: &Emitter<'_, R>,
    value: ValueId,
    aliases: &HashSet<ValueId>,
    active: &mut HashSet<ValueId>,
    depth: usize,
) -> bool {
    if depth > 32 || aliases.contains(&value) || !active.insert(value) {
        return false;
    }
    let answer = if e.parameters.contains(&value) {
        true
    } else if let Ok(record) = e.ir.value(value) {
        match e.ir.instruction(record.def()) {
            Ok(ManagedInsn::Const {
                value: Constant::Int(_) | Constant::Zero,
                ..
            }) => true,
            Ok(ManagedInsn::Move { value, .. }) => {
                pure_condition(e, *value, aliases, active, depth + 1)
            }
            Ok(ManagedInsn::Arithmetic { op, lhs, rhs, .. })
                if !matches!(op, ArithOp::Div | ArithOp::Rem) =>
            {
                pure_condition(e, *lhs, aliases, active, depth + 1)
                    && pure_condition(e, *rhs, aliases, active, depth + 1)
            }
            Ok(ManagedInsn::Compare { lhs, rhs, .. }) => {
                pure_condition(e, *lhs, aliases, active, depth + 1)
                    && pure_condition(e, *rhs, aliases, active, depth + 1)
            }
            _ => false,
        }
    } else {
        false
    };
    active.remove(&value);
    answer
}
