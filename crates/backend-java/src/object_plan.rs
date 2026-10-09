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
#[cfg(test)]
#[path = "object_success_dominance_tests.rs"]
mod success_dominance_tests;

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

type CachedSuccessCut = Result<std::rc::Rc<HashSet<InsnId>>, Unsupported>;

/// The IR, resolver and declaration types remain fixed throughout one emitter.
/// Immutable analysis, lookup evidence and plan shape are cached. Whether an
/// allocation is still pending belongs to the emitter's changing value
/// environment and is checked on every use.
#[derive(Default)]
pub(super) struct ObjectPlanning {
    dominance: OnceCell<std::rc::Rc<decomp_core::analysis::Dominators>>,
    post_dominance: OnceCell<std::rc::Rc<decomp_core::analysis::PostDominators>>,
    extended_structure: OnceCell<
        std::rc::Rc<
            Result<decomp_structure::ExtendedStructuredCfg, decomp_structure::StructureError>,
        >,
    >,
    operands: OnceCell<std::rc::Rc<ObjectOperandIndex>>,
    contextual_index: OnceCell<std::rc::Rc<contextual_locals::MethodIndex>>,
    register_values: OnceCell<std::rc::Rc<HashSet<ValueId>>>,
    normalized_reads: RefCell<HashMap<InsnId, Vec<ValueId>>>,
    invoke_signatures: RefCell<HashMap<MethodRef, Option<JavaMethod>>>,
    value_reads: RefCell<HashMap<ValueId, Vec<InsnId>>>,
    allocation_roots: RefCell<HashMap<ValueId, Option<InsnId>>>,
    value_evidence: RefCell<
        HashMap<
            ValueId,
            std::rc::Rc<Result<type_compatibility::ValueType, type_compatibility::Compatibility>>,
        >,
    >,
    success_dominance: OnceCell<Option<std::rc::Rc<SuccessDominance>>>,
    success_cuts: RefCell<HashMap<InsnId, CachedSuccessCut>>,
    linear_order: OnceCell<Option<std::rc::Rc<LinearObjectOrder>>>,
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

#[derive(Clone)]
pub(super) struct LinearObjectOrder {
    pub(super) instructions: Vec<InsnId>,
    pub(super) positions: HashMap<InsnId, usize>,
}

struct ObjectOperandIndex {
    instructions: Vec<InsnId>,
    positions: HashMap<InsnId, usize>,
    consumers: HashMap<ValueId, Vec<usize>>,
    special_receivers: HashMap<ValueId, Vec<InsnId>>,
    conditions: HashMap<ValueId, Vec<InsnId>>,
}

pub(super) struct SuccessDominance {
    dominance: decomp_core::analysis::Dominators,
    original: HashMap<InsnId, BlockId>,
    success: HashMap<InsnId, BlockId>,
}

impl SuccessDominance {
    fn contains(&self, cut: InsnId, instruction: &InsnId) -> bool {
        if !self.original.contains_key(&cut) {
            return false;
        }
        if cut == *instruction {
            return true;
        }
        self.success
            .get(&cut)
            .zip(self.original.get(instruction))
            .is_some_and(|(&success, &target)| self.dominance.dominates(success, target))
    }
}

pub(crate) enum InstructionAvailability {
    Linear {
        order: std::rc::Rc<LinearObjectOrder>,
        start: usize,
    },
    Graph(HashSet<InsnId>),
    SharedGraph(std::rc::Rc<HashSet<InsnId>>),
    SharedPair([std::rc::Rc<HashSet<InsnId>>; 2]),
    Dominance {
        index: std::rc::Rc<SuccessDominance>,
        cuts: [Option<InsnId>; 2],
    },
}

impl InstructionAvailability {
    pub(crate) fn contains(&self, instruction: &InsnId) -> bool {
        match self {
            Self::Linear { order, start } => {
                order.positions.get(instruction).is_some_and(|p| p >= start)
            }
            Self::Graph(instructions) => instructions.contains(instruction),
            Self::SharedGraph(instructions) => instructions.contains(instruction),
            Self::SharedPair(sets) => sets.iter().all(|set| set.contains(instruction)),
            Self::Dominance { index, cuts } => cuts
                .iter()
                .flatten()
                .all(|cut| index.contains(*cut, instruction)),
        }
    }

    pub(crate) fn intersect(&self, other: &Self) -> Self {
        if let (Self::SharedGraph(left), Self::SharedGraph(right)) = (self, other) {
            return if std::rc::Rc::ptr_eq(left, right) {
                Self::SharedGraph(left.clone())
            } else {
                Self::SharedPair([left.clone(), right.clone()])
            };
        }
        if let (
            Self::Linear { order, start },
            Self::Linear {
                order: other_order,
                start: other_start,
            },
        ) = (self, other)
            && std::rc::Rc::ptr_eq(order, other_order)
        {
            return Self::Linear {
                order: order.clone(),
                start: (*start).max(*other_start),
            };
        }
        if let (
            Self::Dominance { index, cuts },
            Self::Dominance {
                index: other_index,
                cuts: other_cuts,
            },
        ) = (self, other)
            && std::rc::Rc::ptr_eq(index, other_index)
        {
            let mut combined = [None, None];
            let mut count = 0;
            for cut in cuts.iter().chain(other_cuts).flatten() {
                if combined.contains(&Some(*cut)) {
                    continue;
                }
                if count == 2 {
                    break;
                }
                combined[count] = Some(*cut);
                count += 1;
            }
            if cuts
                .iter()
                .chain(other_cuts)
                .flatten()
                .all(|cut| combined.contains(&Some(*cut)))
            {
                return Self::Dominance {
                    index: index.clone(),
                    cuts: combined,
                };
            }
        }
        Self::Graph(match self {
            Self::Linear { order, start } => order.instructions[*start..]
                .iter()
                .copied()
                .filter(|id| other.contains(id))
                .collect(),
            Self::Graph(instructions) => instructions
                .iter()
                .copied()
                .filter(|id| other.contains(id))
                .collect(),
            Self::SharedGraph(instructions) => instructions
                .iter()
                .copied()
                .filter(|id| other.contains(id))
                .collect(),
            Self::SharedPair(sets) => sets[0]
                .iter()
                .copied()
                .filter(|id| self.contains(id) && other.contains(id))
                .collect(),
            Self::Dominance { index, .. } => index
                .original
                .keys()
                .copied()
                .filter(|id| self.contains(id) && other.contains(id))
                .collect(),
        })
    }

    pub(crate) fn into_set(self) -> HashSet<InsnId> {
        match self {
            Self::Linear { order, start } => order.instructions[start..].iter().copied().collect(),
            Self::Graph(instructions) => instructions,
            Self::SharedGraph(instructions) => std::rc::Rc::try_unwrap(instructions)
                .unwrap_or_else(|instructions| (*instructions).clone()),
            Self::SharedPair(sets) => sets[0].intersection(&sets[1]).copied().collect(),
            Self::Dominance { ref index, .. } => index
                .original
                .keys()
                .copied()
                .filter(|id| self.contains(id))
                .collect(),
        }
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Self::Linear { order, start } => *start == order.instructions.len(),
            Self::Graph(instructions) => instructions.is_empty(),
            Self::SharedGraph(instructions) => instructions.is_empty(),
            Self::SharedPair(sets) => !sets[0].iter().any(|id| sets[1].contains(id)),
            Self::Dominance { index, .. } => !index.original.keys().any(|id| self.contains(id)),
        }
    }
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
            dominance: ordinary.dominance.clone(),
            post_dominance: ordinary.post_dominance.clone(),
            extended_structure: ordinary.extended_structure.clone(),
            operands: ordinary.operands.clone(),
            contextual_index: ordinary.contextual_index.clone(),
            register_values: ordinary.register_values.clone(),
            normalized_reads: RefCell::new(ordinary.normalized_reads.borrow().clone()),
            invoke_signatures: RefCell::new(ordinary.invoke_signatures.borrow().clone()),
            value_reads: RefCell::new(ordinary.value_reads.borrow().clone()),
            allocation_roots: RefCell::new(ordinary.allocation_roots.borrow().clone()),
            value_evidence: RefCell::new(ordinary.value_evidence.borrow().clone()),
            success_dominance: ordinary.success_dominance.clone(),
            success_cuts: RefCell::new(ordinary.success_cuts.borrow().clone()),
            linear_order: ordinary.linear_order.clone(),
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
    /// Split normal transfers through an analysis-only success node; exception
    /// transfers bypass it. The source CFG and instruction identities stay intact.
    pub(super) fn object_success_dominance(&self) -> Option<&std::rc::Rc<SuccessDominance>> {
        self.object_planning
            .success_dominance
            .get_or_init(|| {
                let original = self.ir.cfg();
                original.validate().ok()?;
                let mut cfg = decomp_core::cfg::Cfg::new(());
                let mut blocks = HashMap::from([(original.entry(), cfg.entry())]);
                let mut instructions = HashMap::new();
                for block in original.block_ids() {
                    let mapped = *blocks.entry(block).or_insert_with(|| cfg.add_block(()));
                    let id = *original.block(block).ok()?.payload();
                    if instructions.insert(id, mapped).is_some() {
                        return None;
                    }
                }
                let mut success = HashMap::new();
                for block in original.block_ids() {
                    let node = original.block(block).ok()?;
                    let source = blocks[&block];
                    let mut completed = None;
                    for edge_id in node.outgoing() {
                        let edge = original.edge(*edge_id).ok()?;
                        let target = blocks[&edge.target()];
                        if matches!(edge.kind(), EdgeKind::Exception(_)) {
                            cfg.add_edge(source, target, edge.kind().clone()).ok()?;
                        } else {
                            let complete = match completed {
                                Some(complete) => complete,
                                None => {
                                    let complete = cfg.add_block(());
                                    cfg.add_edge(source, complete, EdgeKind::Unconditional)
                                        .ok()?;
                                    completed = Some(complete);
                                    success.insert(*node.payload(), complete);
                                    complete
                                }
                            };
                            cfg.add_edge(complete, target, edge.kind().clone()).ok()?;
                        }
                    }
                }
                let dominance = decomp_core::analysis::dominators(&cfg);
                let entry = cfg.entry();
                instructions.retain(|_, mapped| dominance.dominates(entry, *mapped));
                Some(std::rc::Rc::new(SuccessDominance {
                    dominance,
                    original: instructions,
                    success,
                }))
            })
            .as_ref()
    }

    pub(super) fn cached_value_evidence(
        &self,
        value: ValueId,
    ) -> std::rc::Rc<Result<type_compatibility::ValueType, type_compatibility::Compatibility>> {
        self.object_planning
            .value_evidence
            .borrow_mut()
            .entry(value)
            .or_insert_with(|| {
                std::rc::Rc::new(contextual_locals::evidence(self.ir, value, self.resolver))
            })
            .clone()
    }
    pub(super) fn object_contextual_index(&self) -> &contextual_locals::MethodIndex {
        self.object_planning
            .contextual_index
            .get_or_init(|| std::rc::Rc::new(contextual_locals::MethodIndex::new(self.ir)))
    }

    fn object_operands(&self) -> &ObjectOperandIndex {
        self.object_planning.operands.get_or_init(|| {
            let mut index = ObjectOperandIndex {
                instructions: Vec::with_capacity(self.ir.instructions().len()),
                positions: HashMap::with_capacity(self.ir.instructions().len()),
                consumers: HashMap::new(),
                special_receivers: HashMap::new(),
                conditions: HashMap::new(),
            };
            for (id, instruction) in self.ir.instructions() {
                let position = index.instructions.len();
                index.instructions.push(id);
                index.positions.insert(id, position);
                for value in arguments::reads(instruction) {
                    let sites = index.consumers.entry(value).or_default();
                    if sites.last() != Some(&position) {
                        sites.push(position);
                    }
                }
                if let ManagedInsn::Invoke {
                    kind: InvokeKind::Special,
                    args,
                    ..
                } = instruction
                    && let Some(receiver) = args.first()
                {
                    index
                        .special_receivers
                        .entry(*receiver)
                        .or_default()
                        .push(id);
                }
                if let ManagedInsn::Branch {
                    condition: Some(value),
                    ..
                } = instruction
                {
                    index.conditions.entry(*value).or_default().push(id);
                }
            }
            std::rc::Rc::new(index)
        })
    }

    /// Reads retain original arena order, duplicates and descriptor-normalized
    /// wide arguments. Resolve only relevant invokes and cache missing metadata.
    pub(super) fn object_read_index(
        &self,
        wanted: &HashSet<ValueId>,
    ) -> HashMap<ValueId, Vec<InsnId>> {
        let index = self.object_operands();
        let mut result = HashMap::new();
        for value in wanted {
            if let Some(sites) = self.object_planning.value_reads.borrow().get(value) {
                if !sites.is_empty() {
                    result.insert(*value, sites.clone());
                }
                continue;
            }
            let mut sites = Vec::new();
            for position in index.consumers.get(value).into_iter().flatten() {
                let id = index.instructions[*position];
                let instruction = match self.ir.instruction(id) {
                    Ok(instruction) => instruction,
                    Err(_) => continue,
                };
                if let ManagedInsn::Invoke { method, args, .. } = instruction {
                    let mut normalized = self.object_planning.normalized_reads.borrow_mut();
                    let reads = normalized.entry(id).or_insert_with(|| {
                        self.object_planning
                            .invoke_signatures
                            .borrow_mut()
                            .entry(*method)
                            .or_insert_with(|| self.resolver.method(*method))
                            .as_ref()
                            .and_then(|m| invoke_arguments::operands(m, args).ok())
                            .unwrap_or_else(|| args.clone())
                    });
                    sites.extend(reads.iter().filter(|read| *read == value).map(|_| id));
                } else {
                    sites.extend(
                        arguments::reads(instruction)
                            .iter()
                            .filter(|read| *read == value)
                            .map(|_| id),
                    );
                }
            }
            self.object_planning
                .value_reads
                .borrow_mut()
                .insert(*value, sites.clone());
            if !sites.is_empty() {
                result.insert(*value, sites);
            }
        }
        result
    }

    pub(super) fn object_special_invokes(&self, receiver: ValueId) -> &[InsnId] {
        self.object_operands()
            .special_receivers
            .get(&receiver)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn object_raw_uses(&self, value: ValueId) -> impl Iterator<Item = InsnId> + '_ {
        let index = self.object_operands();
        index
            .consumers
            .get(&value)
            .into_iter()
            .flatten()
            .map(|position| index.instructions[*position])
    }

    pub(super) fn object_uses_for_values(&self, values: &HashSet<ValueId>) -> Vec<InsnId> {
        let index = self.object_operands();
        let mut positions = values
            .iter()
            .flat_map(|value| index.consumers.get(value).into_iter().flatten().copied())
            .collect::<Vec<_>>();
        positions.sort_unstable();
        positions.dedup();
        positions
            .into_iter()
            .map(|position| index.instructions[position])
            .collect()
    }

    pub(super) fn object_register_bound(&self, value: &ValueId) -> bool {
        self.object_planning
            .register_values
            .get_or_init(|| {
                std::rc::Rc::new(
                    self.ir
                        .register_slots()
                        .values()
                        .flat_map(|slot| slot.values().iter().copied())
                        .collect(),
                )
            })
            .contains(value)
    }

    pub(super) fn object_alias_special_invokes(
        &self,
        aliases: &[(ValueId, InsnId)],
    ) -> Vec<InsnId> {
        let index = self.object_operands();
        let mut invokes = aliases
            .iter()
            .flat_map(|(value, _)| self.object_special_invokes(*value).iter().copied())
            .collect::<Vec<_>>();
        invokes.sort_unstable_by_key(|id| index.positions.get(id));
        invokes.dedup();
        invokes
    }

    pub(super) fn object_conditions(&self, condition: ValueId) -> &[InsnId] {
        self.object_operands()
            .conditions
            .get(&condition)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub(super) fn object_dominance(&self) -> &decomp_core::analysis::Dominators {
        self.object_planning
            .dominance
            .get_or_init(|| std::rc::Rc::new(decomp_core::analysis::dominators(self.ir.cfg())))
    }

    pub(super) fn object_post_dominance(&self) -> &decomp_core::analysis::PostDominators {
        self.object_planning
            .post_dominance
            .get_or_init(|| std::rc::Rc::new(decomp_core::analysis::post_dominators(self.ir.cfg())))
    }

    pub(super) fn object_structure_extended(
        &self,
    ) -> &Result<decomp_structure::ExtendedStructuredCfg, decomp_structure::StructureError> {
        self.object_planning
            .extended_structure
            .get_or_init(|| std::rc::Rc::new(decomp_structure::structure_extended(self.ir.cfg())))
            .as_ref()
    }

    /// Exact order only for a fully reachable, unique unconditional chain.
    /// Exception edges, joins, duplicate payloads and cycles keep the general
    /// success-path traversal. No per-initializer suffix cache is retained.
    pub(super) fn linear_object_order(&self) -> Option<&std::rc::Rc<LinearObjectOrder>> {
        self.object_planning
            .linear_order
            .get_or_init(|| {
                let cfg = self.ir.cfg();
                let mut block = cfg.entry();
                let mut seen = HashSet::new();
                let mut instructions = Vec::with_capacity(cfg.block_count());
                let mut positions = HashMap::with_capacity(cfg.block_count());
                loop {
                    if !seen.insert(block) {
                        return None;
                    }
                    let node = cfg.block(block).ok()?;
                    if instructions.is_empty() && !node.incoming().is_empty() {
                        return None;
                    }
                    if positions
                        .insert(*node.payload(), instructions.len())
                        .is_some()
                    {
                        return None;
                    }
                    instructions.push(*node.payload());
                    match node.outgoing() {
                        [] => break,
                        [edge_id] => {
                            let edge = cfg.edge(*edge_id).ok()?;
                            if *edge.kind() != EdgeKind::Unconditional || edge.source() != block {
                                return None;
                            }
                            let next = cfg.block(edge.target()).ok()?;
                            if next.incoming() != [*edge_id] {
                                return None;
                            }
                            block = edge.target();
                        }
                        _ => return None,
                    }
                }
                (seen.len() == cfg.block_count()).then(|| {
                    std::rc::Rc::new(LinearObjectOrder {
                        instructions,
                        positions,
                    })
                })
            })
            .as_ref()
    }

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
        self.ir.cfg().block(self.object_block_id(instruction)?).ok()
    }

    pub(super) fn object_block_id(&self, instruction: InsnId) -> Option<BlockId> {
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
        blocks.get(&instruction).copied()
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
        if let Some(root) = self.object_planning.allocation_roots.borrow().get(&value) {
            return *root;
        }
        let mut visited = HashSet::new();
        let root = loop {
            if let Some(root) = self.object_planning.allocation_roots.borrow().get(&value) {
                break *root;
            }
            if !visited.insert(value) {
                break None;
            }
            let Ok(record) = self.ir.value(value) else {
                break None;
            };
            let def = record.def();
            match self.ir.instruction(def) {
                Ok(ManagedInsn::NewInstance { .. }) => break Some(def),
                Ok(ManagedInsn::Move { value: source, .. }) => value = *source,
                _ => break None,
            }
        };
        let mut roots = self.object_planning.allocation_roots.borrow_mut();
        for value in visited {
            roots.insert(value, root);
        }
        root
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
