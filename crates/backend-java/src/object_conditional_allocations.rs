//! One allocation followed by a pure null selector and two exact initializers.
//!
//! The selector reads an already materialized reference identity. Its null test
//! cannot throw, allocate, invoke code, or observe fields. Moving only that test
//! before the allocation is unobservable, including when class initialization
//! or allocation fails. Both source arms still allocate the same exact class
//! before invoking the selected original constructor. No effectful argument is
//! moved; no exception scope or uninitialized-reference escape is accepted.

use super::*;
use decomp_core::ids::BlockId;

#[derive(Clone)]
pub(super) struct ConditionalPlan {
    allocation: InsnId,
    value: ValueId,
    owner: String,
    selector: ValueId,
    null_is_true: bool,
    arms: [Arm; 2],
}

#[derive(Clone)]
struct Arm {
    initializer: InsnId,
    method: MethodRef,
    args: Vec<ValueId>,
}

fn fail() -> Unsupported {
    Unsupported::Instruction("unproved conditional allocation initializer".into())
}

impl<R: JavaResolver> Emitter<'_, R> {
    pub(in crate::object_plan) fn conditional_object_instruction(
        &mut self,
        insn: &ManagedInsn,
        signature: &JavaMethod,
        current: InsnId,
    ) -> Result<bool, Unsupported> {
        let root = match insn {
            ManagedInsn::NewInstance { .. } => current,
            ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                args,
                result: None,
                method,
            } if self
                .resolver
                .method(*method)
                .is_some_and(|m| m.name == "<init>") =>
            {
                let Some(root) = args.first().and_then(|v| self.allocation_root(*v)) else {
                    return Ok(false);
                };
                root
            }
            _ => return Ok(false),
        };
        // Preserve every established linear/argument allocation proof.
        if self.object_plan(root).is_ok() {
            return Ok(false);
        }
        let plan = match self.conditional_plan(root) {
            Ok(plan) => plan,
            Err(_) => return Ok(false),
        };
        if current != plan.allocation {
            if !plan.arms.iter().any(|arm| arm.initializer == current)
                || self.get(plan.value).is_err()
                || self.pending_allocation(plan.value)
            {
                return Err(fail());
            }
            // The selected initializer was emitted atomically at the original
            // allocation site. The surviving encoded branch has only a pure
            // repeated null comparison; no call, store or effect is duplicated.
            return Ok(true);
        }
        let (selector, selector_type) = self.get(plan.selector)?;
        if !selector_type.reference() || !self.bound_before_allocation(plan.selector, root) {
            return Err(fail());
        }
        let expressions = plan
            .arms
            .iter()
            .map(|arm| {
                let method = self.resolver.method(arm.method).ok_or_else(fail)?;
                let flags = self.resolver.method_access(arm.method).ok_or_else(fail)?;
                if flags & (8 | 0x100 | 0x400) != 0
                    || flags & 1 == 0 && method.owner != signature.owner
                    || checked_exceptions::check(self.resolver, &method.throws, &signature.throws)
                        .is_err()
                {
                    return Err(fail());
                }
                let args = arm
                    .args
                    .iter()
                    .zip(&method.parameters)
                    .map(|(value, expected)| {
                        if !self.bound_before_allocation(*value, root) {
                            return Err(fail());
                        }
                        let (expression, actual) = self.operand(*value, expected)?;
                        self.compatible(&actual, expected)?;
                        Ok(format!("({}) ({expression})", expected.source()?))
                    })
                    .collect::<Result<Vec<_>, Unsupported>>()?;
                Ok(format!(
                    "new {}({})",
                    qualified(&plan.owner)?,
                    args.join(", ")
                ))
            })
            .collect::<Result<Vec<_>, Unsupported>>()?;
        self.define(
            plan.value,
            format!(
                "({selector} {} null) ? {} : {}",
                if plan.null_is_true { "==" } else { "!=" },
                expressions[0],
                expressions[1]
            ),
            JavaType::Class(plan.owner),
        )?;
        self.hoisted.remove(&plan.value);
        Ok(true)
    }

    fn bound_before_allocation(&self, value: ValueId, allocation: InsnId) -> bool {
        if self.get(value).is_err() {
            return false;
        }
        if self.parameters.contains(&value) {
            return true;
        }
        let Ok(value) = self.ir.value(value) else {
            return false;
        };
        let Some(def) = self.conditional_block_id(value.def()) else {
            return false;
        };
        let Some(site) = self.conditional_block_id(allocation) else {
            return false;
        };
        def != site && self.object_dominance().dominates(def, site)
    }

    fn conditional_plan(&self, allocation: InsnId) -> Result<ConditionalPlan, Unsupported> {
        if let Some(plan) = self.object_planning.conditional.borrow().get(&allocation) {
            return plan.clone();
        }
        let plan = self.build_conditional_plan(allocation);
        self.object_planning
            .conditional
            .borrow_mut()
            .insert(allocation, plan.clone());
        plan
    }

    fn build_conditional_plan(&self, allocation: InsnId) -> Result<ConditionalPlan, Unsupported> {
        let ManagedInsn::NewInstance { result, class } =
            self.ir.instruction(allocation).map_err(|_| fail())?
        else {
            return Err(fail());
        };
        let value = *result;
        let owner = self.resolver.class(*class).ok_or_else(fail)?;
        if self.ty(value)? != JavaType::Class(owner.clone()) {
            return Err(fail());
        }
        let start = self.conditional_block_id(allocation).ok_or_else(fail)?;
        let mut block = self.single_next(start)?;
        let mut gap = HashSet::new();
        let (selector, null_is_true, edges) = loop {
            if gap.len() > 4 || !gap.insert(block) {
                return Err(fail());
            }
            let node = self.ir.cfg().block(block).map_err(|_| fail())?;
            if node.incoming().len() != 1 {
                return Err(fail());
            }
            match self.ir.instruction(*node.payload()).map_err(|_| fail())? {
                ManagedInsn::Const {
                    value: Constant::Zero | Constant::Null,
                    ..
                }
                | ManagedInsn::Compare {
                    op: CompareOp::Equal | CompareOp::NotEqual,
                    ..
                } => {
                    block = self.single_next(block)?;
                }
                ManagedInsn::Branch {
                    condition: Some(condition),
                    then_edge,
                    else_edge: Some(else_edge),
                } => {
                    if node.outgoing().len() != 2
                        || !node.outgoing().contains(then_edge)
                        || !node.outgoing().contains(else_edge)
                        || then_edge == else_edge
                    {
                        return Err(fail());
                    }
                    let condition_def = self.ir.value(*condition).map_err(|_| fail())?.def();
                    let ManagedInsn::Compare { op, lhs, rhs, .. } =
                        self.ir.instruction(condition_def).map_err(|_| fail())?
                    else {
                        return Err(fail());
                    };
                    let (selector, zero) = if self.is_null_literal(*rhs) {
                        (*lhs, *rhs)
                    } else if self.is_null_literal(*lhs) {
                        (*rhs, *lhs)
                    } else {
                        return Err(fail());
                    };
                    if !matches!(op, CompareOp::Equal | CompareOp::NotEqual)
                        || selector == value
                        || zero == value
                        || !self.ty(selector)?.reference()
                    {
                        return Err(fail());
                    }
                    break (selector, *op == CompareOp::Equal, [*then_edge, *else_edge]);
                }
                _ => return Err(fail()),
            }
        };
        let mut arms = Vec::new();
        let mut joins = Vec::new();
        for (index, edge_id) in edges.into_iter().enumerate() {
            let edge = self.ir.cfg().edge(edge_id).map_err(|_| fail())?;
            if edge.source() != block
                || edge.kind()
                    != if index == 0 {
                        &EdgeKind::BranchTrue
                    } else {
                        &EdgeKind::BranchFalse
                    }
            {
                return Err(fail());
            }
            let target = self.ir.cfg().block(edge.target()).map_err(|_| fail())?;
            if target.incoming() != [edge_id] {
                return Err(fail());
            }
            let initializer = *target.payload();
            let ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                result: None,
                method,
                args,
            } = self.ir.instruction(initializer).map_err(|_| fail())?
            else {
                return Err(fail());
            };
            let declaration = self.resolver.method(*method).ok_or_else(fail)?;
            let operands = invoke_arguments::operands(&declaration, args)?;
            if declaration.owner != owner
                || declaration.name != "<init>"
                || declaration.is_static
                || declaration.return_type != JavaType::Void
                || operands.first() != Some(&value)
                || operands[1..].contains(&value)
            {
                return Err(fail());
            }
            arms.push(Arm {
                initializer,
                method: *method,
                args: operands[1..].to_vec(),
            });
            let mut next = self.single_next(edge.target())?;
            let next_node = self.ir.cfg().block(next).map_err(|_| fail())?;
            if let ManagedInsn::Branch {
                condition: None,
                then_edge,
                else_edge: None,
            } = self
                .ir
                .instruction(*next_node.payload())
                .map_err(|_| fail())?
            {
                if next_node.incoming().len() != 1 || next_node.outgoing() != [*then_edge] {
                    return Err(fail());
                }
                next = self.single_next(next)?;
            }
            joins.push(next);
        }
        if joins[0] != joins[1]
            || self
                .ir
                .cfg()
                .block(joins[0])
                .map_err(|_| fail())?
                .incoming()
                .len()
                != 2
        {
            return Err(fail());
        }
        let dominators = self.object_dominance();
        for id in self.object_raw_uses(value) {
            if arms.iter().any(|arm| arm.initializer == id) {
                continue;
            }
            let use_block = self.conditional_block_id(id).ok_or_else(fail)?;
            if !dominators.dominates(joins[0], use_block) {
                return Err(fail());
            }
        }
        Ok(ConditionalPlan {
            allocation,
            value,
            owner,
            selector,
            null_is_true,
            arms: arms.try_into().map_err(|_| fail())?,
        })
    }

    fn conditional_block_id(&self, instruction: InsnId) -> Option<BlockId> {
        self.object_block(instruction)?;
        self.object_planning
            .blocks
            .get()?
            .get(&instruction)
            .copied()
    }

    fn single_next(&self, block: BlockId) -> Result<BlockId, Unsupported> {
        let node = self.ir.cfg().block(block).map_err(|_| fail())?;
        if node.outgoing().len() != 1 {
            return Err(fail());
        }
        let edge = self.ir.cfg().edge(node.outgoing()[0]).map_err(|_| fail())?;
        if edge.kind() != &EdgeKind::Unconditional {
            return Err(fail());
        }
        Ok(edge.target())
    }

    fn is_null_literal(&self, value: ValueId) -> bool {
        self.ir
            .value(value)
            .ok()
            .and_then(|v| self.ir.instruction(v.def()).ok())
            .is_some_and(|instruction| {
                matches!(
                    instruction,
                    ManagedInsn::Const {
                        value: Constant::Null | Constant::Zero,
                        ..
                    }
                )
            })
    }
}

#[cfg(test)]
pub(super) fn reads(instruction: &ManagedInsn) -> Vec<ValueId> {
    match instruction {
        ManagedInsn::Const { .. } | ManagedInsn::NewInstance { .. } => vec![],
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
    }
}
