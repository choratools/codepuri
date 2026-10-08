//! Java has no expression for an allocated but uninitialized reference. The
//! ordinary plan delays `new` only across nonthrowing pure operations. A bounded
//! argument tree instead preserves allocation-before-arguments via JLS 15.9.4.
//! Never use effect annotations as a purity proof.
use super::*;
use decomp_core::{cfg::Block, ids::BlockId};
use std::cell::{OnceCell, RefCell};

#[path = "object_arguments.rs"]
mod arguments;
#[path = "object_conditional_allocations.rs"]
mod conditional;
#[path = "object_argument_regions.rs"]
mod regions;

#[cfg(test)]
#[path = "object_plan_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "object_planning_cache_tests.rs"]
mod cache_tests;

#[cfg(test)]
#[path = "object_capture_publication_tests.rs"]
mod capture_publication_tests;

#[derive(Clone)]
pub(super) struct ObjectPlan {
    pub allocation: InsnId,
    pub initializer: InsnId,
    pub aliases: Vec<(ValueId, InsnId)>,
    pub owner: String,
    /// Exact expression evaluation trace; empty for the ordinary pure gap.
    pub argument_gap: Vec<InsnId>,
    /// Inner allocation/initializer pairs covered by the exact expression tree.
    pub argument_allocations: Vec<(ValueId, InsnId, InsnId)>,
    /// Producer/literal results assigned once inside the argument tree and used later.
    pub argument_captures: Vec<ValueId>,
}

/// The IR, resolver and declaration types remain fixed throughout one emitter.
/// Only plan shape is cached: whether an allocation is still pending belongs
/// to the emitter's changing value environment and is checked on every use.
#[derive(Default)]
pub(super) struct ObjectPlanning {
    blocks: OnceCell<HashMap<InsnId, BlockId>>,
    plans: RefCell<HashMap<InsnId, Result<ObjectPlan, Unsupported>>>,
    conditional: RefCell<HashMap<InsnId, Result<conditional::ConditionalPlan, Unsupported>>>,
    regions: RefCell<HashMap<InsnId, Result<regions::ArgumentRegion, Unsupported>>>,
    sequences: RefCell<HashSet<InsnId>>,
    argument_owners: OnceCell<HashMap<InsnId, Vec<ValueId>>>,
    pub(super) pure_expressions: OnceCell<HashSet<ValueId>>,
    pure_expression_enabled: bool,
    exception_scopes_empty: bool,
}

impl ObjectPlanning {
    pub(super) fn for_source_emission(exception_scopes_empty: bool) -> Self {
        Self {
            pure_expression_enabled: true,
            exception_scopes_empty,
            ..Self::default()
        }
    }

    /// A static initializer validates through the ordinary method emitter and
    /// then renders the same IR with final-field rules. Reuse immutable object
    /// plan evidence, but retain the static emitter's distinct pure-expression
    /// policy and its own mutable value environment.
    pub(super) fn for_static_from(ordinary: &Self) -> Self {
        Self {
            blocks: ordinary.blocks.clone(),
            plans: RefCell::new(ordinary.plans.borrow().clone()),
            conditional: RefCell::new(ordinary.conditional.borrow().clone()),
            regions: RefCell::new(ordinary.regions.borrow().clone()),
            sequences: RefCell::new(ordinary.sequences.borrow().clone()),
            argument_owners: ordinary.argument_owners.clone(),
            ..Self::default()
        }
    }
}

impl<R: JavaResolver> Emitter<'_, R> {
    pub(super) fn pure_expression_values(&self) -> &HashSet<ValueId> {
        self.object_planning.pure_expressions.get_or_init(|| {
            let receiver = self
                .values
                .iter()
                .find_map(|(value, (name, _))| (name == "this").then_some(*value));
            let exception_scopes_empty = self.object_planning.pure_expression_enabled
                && self.object_planning.exception_scopes_empty;
            super::pure_expression::plan(
                self.ir,
                receiver,
                &self.declarations,
                exception_scopes_empty,
                self.resolver,
            )
        })
    }

    pub(super) fn is_pure_expression(&self, value: ValueId) -> bool {
        self.pure_expression_values().contains(&value)
    }

    pub(super) fn object_plan(&self, allocation: InsnId) -> Result<ObjectPlan, Unsupported> {
        if let Some(plan) = self.object_planning.plans.borrow().get(&allocation) {
            return plan.clone();
        }
        // Preserve pure-plan preference and its original error if both paths
        // fail. Argument folding must never hide an invalid ordinary plan.
        let plan = self.pure_object_plan(allocation).or_else(|original| {
            self.argument_object_plan(allocation)
                .or_else(|_| {
                    let plan = self.sequence_object_plan(allocation)?;
                    self.object_planning
                        .sequences
                        .borrow_mut()
                        .insert(allocation);
                    Ok::<_, Unsupported>(plan)
                })
                .map_err(|_| original)
        });
        self.object_planning
            .plans
            .borrow_mut()
            .insert(allocation, plan.clone());
        plan
    }

    fn object_block(&self, instruction: InsnId) -> Option<&Block<InsnId>> {
        let blocks = self.object_planning.blocks.get_or_init(|| {
            let mut blocks = HashMap::new();
            for id in self.ir.cfg().block_ids() {
                if let Ok(block) = self.ir.cfg().block(id) {
                    // Match the old find_map even for repeated CFG payloads.
                    blocks.entry(*block.payload()).or_insert(id);
                }
            }
            blocks
        });
        self.ir.cfg().block(*blocks.get(&instruction)?).ok()
    }

    fn object_argument_owners(&self, current: InsnId) -> &[ValueId] {
        self.object_planning
            .argument_owners
            .get_or_init(|| {
                let mut owners: HashMap<InsnId, Vec<ValueId>> = HashMap::new();
                // Preserve arena ordering and include off-CFG instructions:
                // failed plans retain their ordinary rejection and no owners.
                for (allocation, insn) in self.ir.instructions() {
                    if let ManagedInsn::NewInstance { result, .. } = insn
                        && let Ok(plan) = self.object_plan(allocation)
                    {
                        for instruction in plan.argument_gap {
                            owners.entry(instruction).or_default().push(*result);
                        }
                    }
                }
                owners
            })
            .get(&current)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    fn pure_object_plan(&self, allocation: InsnId) -> Result<ObjectPlan, Unsupported> {
        let ManagedInsn::NewInstance { result, class } = self
            .ir
            .instruction(allocation)
            .map_err(|e| Unsupported::Instruction(e.to_string()))?
        else {
            return Err(Unsupported::Instruction("unpaired initializer".into()));
        };
        let owner = self
            .resolver
            .class(*class)
            .ok_or_else(|| Unsupported::Metadata("unresolved allocation class".into()))?;
        if self.ty(*result)? != JavaType::Class(owner.clone()) {
            return Err(Unsupported::Type("allocation result owner".into()));
        }
        let mut aliases = vec![(*result, allocation)];
        let mut current = allocation;
        let mut visited = HashSet::new();
        let mut chain = vec![allocation];
        let mut exceptional = false;
        let mut alias_only = true;
        loop {
            if !visited.insert(current) {
                return Err(Unsupported::ControlFlow("allocation cycle".into()));
            }
            let node = self
                .object_block(current)
                .ok_or_else(|| Unsupported::ControlFlow("allocation outside CFG".into()))?;
            let mut normal = Vec::new();
            for id in node.outgoing() {
                let edge = self
                    .ir
                    .cfg()
                    .edge(*id)
                    .map_err(|e| Unsupported::ControlFlow(e.to_string()))?;
                if matches!(edge.kind(), EdgeKind::Exception(_)) {
                    exceptional = true;
                } else {
                    normal.push(*id);
                }
            }
            if normal.len() != 1 {
                return Err(Unsupported::Instruction(
                    "allocation control-flow barrier".into(),
                ));
            }
            let edge = self
                .ir
                .cfg()
                .edge(normal[0])
                .map_err(|e| Unsupported::ControlFlow(e.to_string()))?;
            let next = self
                .ir
                .cfg()
                .block(edge.target())
                .map_err(|e| Unsupported::ControlFlow(e.to_string()))?;
            if *edge.kind() != EdgeKind::Unconditional || next.incoming().len() != 1 {
                return Err(Unsupported::Instruction(
                    "allocation initializer boundary".into(),
                ));
            }
            current = *next.payload();
            chain.push(current);
            let insn = self
                .ir
                .instruction(current)
                .map_err(|e| Unsupported::Instruction(e.to_string()))?;
            let is_alias = |v: &ValueId| aliases.iter().any(|(a, _)| a == v);
            match insn {
                ManagedInsn::Move { result, value } if is_alias(value) => {
                    if self.ty(*result)? != JavaType::Class(owner.clone())
                        || self.ty(*value)? != self.ty(*result)?
                    {
                        return Err(Unsupported::Type("preinit alias type".into()));
                    }
                    aliases.push((*result, current));
                    continue;
                }
                ManagedInsn::Invoke {
                    kind: InvokeKind::Special,
                    result: None,
                    method,
                    args,
                } if args.first().is_some_and(is_alias) => {
                    let m = self.resolver.method(*method).ok_or_else(|| {
                        Unsupported::Metadata("unresolved allocation initializer".into())
                    })?;
                    let args = invoke_arguments::operands(&m, args)?;
                    if m.owner != owner
                        || m.name != "<init>"
                        || m.is_static
                        || m.return_type != JavaType::Void
                        || args.len() != m.parameters.len() + 1
                        || args[1..].iter().any(is_alias)
                    {
                        return Err(Unsupported::Metadata(
                            "allocation initializer signature".into(),
                        ));
                    }
                    // Constructor-only protection is also a scope boundary:
                    // absence of allocation exceptions cannot authorize moving
                    // the allocation into a different original try range.
                    for id in next.outgoing() {
                        let edge = self
                            .ir
                            .cfg()
                            .edge(*id)
                            .map_err(|e| Unsupported::ControlFlow(e.to_string()))?;
                        exceptional |= matches!(edge.kind(), EdgeKind::Exception(_));
                    }
                    if exceptional
                        && !((alias_only && self.resolver.exception_allocation_chain(&chain))
                            || self.resolver.exception_allocation_pure_chain(&chain))
                    {
                        return Err(Unsupported::Instruction(
                            "allocation control-flow barrier".into(),
                        ));
                    }
                    return Ok(ObjectPlan {
                        allocation,
                        initializer: current,
                        aliases,
                        owner,
                        argument_gap: Vec::new(),
                        argument_allocations: Vec::new(),
                        argument_captures: Vec::new(),
                    });
                }
                _ => {}
            }
            alias_only = false;
            // Explicit allowlist: everything else is a may-throw/effect/flow barrier.
            let operands = match insn {
                ManagedInsn::Const {
                    value:
                        Constant::Zero
                        | Constant::Boolean(_)
                        | Constant::Int(_)
                        | Constant::Long(_)
                        | Constant::FloatBits(_)
                        | Constant::DoubleBits(_)
                        | Constant::Null,
                    ..
                } => vec![],
                ManagedInsn::Move { value, .. }
                | ManagedInsn::Neg { value, .. }
                | ManagedInsn::Not { value, .. } => vec![*value],
                ManagedInsn::Compare { lhs, rhs, .. }
                | ManagedInsn::OrderingCompare { lhs, rhs, .. } => vec![*lhs, *rhs],
                ManagedInsn::Arithmetic {
                    op,
                    result,
                    lhs,
                    rhs,
                } => {
                    if matches!(op, ArithOp::Div | ArithOp::Rem)
                        && !matches!(self.ty(*result)?, JavaType::Float | JavaType::Double)
                    {
                        return Err(Unsupported::Instruction(
                            "throwing computation before initializer".into(),
                        ));
                    }
                    vec![*lhs, *rhs]
                }
                ManagedInsn::Cast { value, target, .. }
                    if self.ty(*value)?.numeric()
                        && self
                            .resolver
                            .instruction_type(*target)
                            .is_some_and(|t| t.numeric()) =>
                {
                    vec![*value]
                }
                ManagedInsn::NewInstance { .. }
                | ManagedInsn::NewArray { .. }
                | ManagedInsn::NewFilledArray { .. } => {
                    return Err(Unsupported::Instruction(
                        "allocation may throw OOM before initializer".into(),
                    ));
                }
                _ => {
                    return Err(Unsupported::Instruction(
                        "observable or throwing operation before initializer".into(),
                    ));
                }
            };
            if operands.iter().any(is_alias) {
                return Err(Unsupported::Instruction(
                    "uninitialized reference use".into(),
                ));
            }
        }
    }

    pub(super) fn allocation_root(&self, mut value: ValueId) -> Option<InsnId> {
        let mut visited = HashSet::new();
        while visited.insert(value) {
            let def = self.ir.value(value).ok()?.def();
            match self.ir.instruction(def).ok()? {
                ManagedInsn::NewInstance { .. } => return Some(def),
                ManagedInsn::Move { value: source, .. } => value = *source,
                _ => return None,
            }
        }
        None
    }

    /// A hoisted allocation slot is declared but unreadable until its exact
    /// paired initializer materializes it. This preserves lexical scope without
    /// treating a declaration or default null as an initialized DEX object.
    pub(super) fn pending_allocation(&self, value: ValueId) -> bool {
        self.hoisted.contains(&value)
            && self.allocation_root(value).is_some()
            && !self.current.is_some_and(|current| {
                self.allocation_readable
                    .as_ref()
                    .and_then(|available| available.get(&value))
                    .is_some_and(|available| available.contains(&current))
            })
    }

    /// Narrow emitter hook; no resolver changes required.
    pub(super) fn planned_object_instruction(
        &mut self,
        insn: &ManagedInsn,
        signature: &JavaMethod,
    ) -> Result<bool, Unsupported> {
        let current = self
            .current
            .ok_or_else(|| Unsupported::Instruction("object instruction identity".into()))?;
        if self.branching_argument_instruction(insn, signature, current)? {
            return Ok(true);
        }
        if self.conditional_object_instruction(insn, signature, current)? {
            return Ok(true);
        }
        if self.object_argument_instruction(current) {
            return Ok(true);
        }
        match insn {
            ManagedInsn::NewInstance { .. } => {
                self.object_plan(current)?;
                Ok(true)
            }
            ManagedInsn::Move { result, value } if self.get(*value).is_err() => {
                let Some(root) = self.allocation_root(*value) else {
                    return Ok(false);
                };
                let plan = self.object_plan(root)?;
                Ok(plan.aliases.contains(&(*result, current)))
            }
            ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                result,
                method,
                args,
            } => {
                let m = self
                    .resolver
                    .method(*method)
                    .ok_or_else(|| Unsupported::Metadata("unresolved method".into()))?;
                if m.name != "<init>" {
                    return Ok(false);
                }
                let root = args
                    .first()
                    .and_then(|v| self.allocation_root(*v))
                    .ok_or_else(|| Unsupported::Instruction("unpaired initializer".into()))?;
                let plan = self.object_plan(root)?;
                if result.is_some()
                    || plan.initializer != current
                    || (self.values.contains_key(&plan.aliases[0].0)
                        && !self.pending_allocation(plan.aliases[0].0))
                {
                    return Err(Unsupported::Instruction(
                        "repeated or unpaired initializer".into(),
                    ));
                }
                if checked_exceptions::check(self.resolver, &m.throws, &signature.throws).is_err() {
                    return Err(Unsupported::Metadata(
                        "allocation checked exceptions".into(),
                    ));
                }
                let args = invoke_arguments::operands(&m, args)?;
                let capture_slots = self.object_argument_capture_slots(&plan)?;
                let expressions =
                    self.object_argument_expressions(&plan, signature, &capture_slots)?;
                let sequenced = self.object_argument_sequence(&plan);
                let actual = args[1..]
                    .iter()
                    .zip(&m.parameters)
                    .enumerate()
                    .map(|(position, (v, t))| {
                        let selected = if sequenced && position > 0 {
                            capture_slots.get(v)
                        } else {
                            expressions.get(v)
                        };
                        let (name, ty) = selected
                            .cloned()
                            .map(Ok)
                            .unwrap_or_else(|| self.operand(*v, t))?;
                        let (name, ty) = if let Some(converted) =
                            self.category_one_materialized_expression(*v, &name, &ty, t)
                        {
                            (converted, t.clone())
                        } else {
                            (name, ty)
                        };
                        self.compatible(&ty, t)?;
                        Ok(format!("({}) {name}", t.source()?))
                    })
                    .collect::<Result<Vec<_>, Unsupported>>()?;
                self.current = Some(plan.allocation);
                let emitted = self.define(
                    plan.aliases[0].0,
                    format!("new {}({})", qualified(&plan.owner)?, actual.join(", ")),
                    JavaType::Class(plan.owner.clone()),
                );
                self.current = Some(current);
                emitted?;
                // These slots are assigned by the emitted initializer expression.
                // Branch/static planning may have predeclared them as hoisted
                // allocation-derived aliases; after this exact successful site they
                // are ordinary readable locals. Do not publish them earlier: a
                // declaration or default null would not be the DEX object.
                for (value, slot) in capture_slots {
                    self.values.insert(value, slot);
                    self.hoisted.remove(&value);
                }
                self.hoisted.remove(&plan.aliases[0].0);
                let (name, ty) = self.get(plan.aliases[0].0)?;
                for (alias, def) in plan.aliases.into_iter().skip(1) {
                    self.current = Some(def);
                    let emitted = self.define(alias, name.clone(), ty.clone());
                    self.current = Some(current);
                    emitted?;
                    self.hoisted.remove(&alias);
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

#[cfg(test)]
#[path = "object_exception_pair_tests.rs"]
mod exception_pair_tests;

#[cfg(test)]
#[path = "object_sequence_tests.rs"]
mod sequence_tests;

#[cfg(test)]
#[path = "object_category_one_tests.rs"]
mod category_one_tests;
