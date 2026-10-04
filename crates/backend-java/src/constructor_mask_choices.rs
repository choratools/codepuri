//! Exact effect-free nullable mask choices before an original this-delegation.
//! The plan retains instruction order and original edge-selected simultaneous
//! Phi copies. It is not a field-assignment or complete constructor-family proof.
use super::{ConstructorInput, ConstructorResolver};
use crate::{
    JavaType, Unsupported,
    branching::{Copy, Step},
};
use decomp_core::{
    cfg::EdgeKind,
    ids::{BlockId, EdgeId},
};
use managed_ir::{
    effects::EffectFlags,
    insn::{ArithOp, CompareOp, Constant, InvokeKind, ManagedInsn},
    method::{MethodIr, SsaForm},
    refs::MethodRef,
    value::{InsnId, ValueId},
};
use std::collections::{HashMap, HashSet};

/// A narrow original prefix, ending immediately before the selected this call.
/// Rendering requires Java25, typed uninitialized local declarations, one
/// evaluation of each Instruction and snapshots of every Copies source first.
#[derive(Debug)]
pub struct MaskChoicePlan {
    /// Original initializer instruction's block; it is not consumed by steps.
    pub chain_block: BlockId,
    /// Exact same-class original initializer; ordinary overload/access checks remain.
    pub target: MethodRef,
    /// Proven source local types; no initialized Java defaults are synthesized.
    pub locals: Vec<(ValueId, JavaType)>,
    /// Proven nullable-default zero carriers that must remain integral locals.
    pub null_defaults: HashSet<ValueId>,
    /// Original primitive evaluations and selected, simultaneous Phi transfers.
    pub steps: Vec<Step>,
    /// Exactly the original prefix blocks covered by steps and Phi transfers.
    pub prefix_blocks: HashSet<BlockId>,
}

fn fail() -> Unsupported {
    Unsupported::ControlFlow("unproved effect-free constructor mask choices".into())
}
#[derive(Clone)]
struct State {
    available: HashSet<ValueId>,
    types: HashMap<ValueId, JavaType>,
}
struct Choice {
    bit: u32,
    inputs: Vec<(ValueId, ValueId, ValueId)>,
}
struct Proof<'a> {
    ir: &'a MethodIr<SsaForm>,
    receiver: ValueId,
    mask: ValueId,
    initial: HashSet<ValueId>,
    blocks: HashSet<BlockId>,
    definitions: HashMap<ValueId, JavaType>,
    null_defaults: HashSet<ValueId>,
    choices: Vec<Choice>,
}
impl Proof<'_> {
    fn definition(
        &mut self,
        id: InsnId,
        result: ValueId,
        ty: JavaType,
        state: &mut State,
    ) -> Result<(), Unsupported> {
        if state.available.contains(&result)
            || self.initial.contains(&result)
            || self.definitions.insert(result, ty.clone()).is_some()
            || self.ir.value(result).map_err(|_| fail())?.def() != id
        {
            return Err(fail());
        }
        state.available.insert(result);
        state.types.insert(result, ty);
        Ok(())
    }
    fn nullable_default_zero(&mut self, value: ValueId) -> Result<ValueId, Unsupported> {
        let value = self.root(value).ok_or_else(fail)?;
        if !self.zero(value) {
            return Err(fail());
        }
        self.definitions.insert(value, JavaType::Int);
        self.null_defaults.insert(value);
        Ok(value)
    }
    fn zero(&self, value: ValueId) -> bool {
        self.root(value).is_some_and(|root| {
            self.ir
                .value(root)
                .ok()
                .and_then(|record| self.ir.instruction(record.def()).ok())
                .is_some_and(|i| {
                    matches!(
                        i,
                        ManagedInsn::Const {
                            value: Constant::Zero | Constant::Int(0),
                            ..
                        }
                    )
                })
        })
    }
    fn root(&self, mut value: ValueId) -> Option<ValueId> {
        let mut seen = HashSet::new();
        for _ in 0..64 {
            if !seen.insert(value) {
                return None;
            }
            let record = self.ir.value(value).ok()?;
            match self.ir.instruction(record.def()).ok()? {
                ManagedInsn::Move { value: source, .. } => value = *source,
                _ => return Some(value),
            }
        }
        None
    }
    fn mask_bit(&self, value: ValueId) -> Option<u32> {
        let value = self.root(value)?;
        let instruction = self.ir.instruction(self.ir.value(value).ok()?.def()).ok()?;
        let ManagedInsn::Arithmetic {
            op: ArithOp::And,
            lhs,
            rhs,
            ..
        } = instruction
        else {
            return None;
        };
        let literal = if self.root(*lhs)? == self.mask {
            *rhs
        } else if self.root(*rhs)? == self.mask {
            *lhs
        } else {
            return None;
        };
        let literal = self.root(literal)?;
        let ManagedInsn::Const {
            value: Constant::Int(bits),
            ..
        } = self
            .ir
            .instruction(self.ir.value(literal).ok()?.def())
            .ok()?
        else {
            return None;
        };
        let bits = u32::from_ne_bytes(bits.to_ne_bytes());
        bits.is_power_of_two().then_some(bits)
    }
    fn guard(&self, condition: ValueId) -> Result<(u32, bool), Unsupported> {
        let value = self.root(condition).ok_or_else(fail)?;
        let ManagedInsn::Compare { op, lhs, rhs, .. } = self
            .ir
            .instruction(self.ir.value(value).map_err(|_| fail())?.def())
            .map_err(|_| fail())?
        else {
            return Err(fail());
        };
        let tested = if self.zero(*rhs) {
            *lhs
        } else if self.zero(*lhs) {
            *rhs
        } else {
            return Err(fail());
        };
        let default_on_true = match op {
            CompareOp::Equal => false,
            CompareOp::NotEqual => true,
            _ => return Err(fail()),
        };
        Ok((self.mask_bit(tested).ok_or_else(fail)?, default_on_true))
    }
    fn visit(&mut self, block: BlockId) -> Result<InsnId, Unsupported> {
        if self.blocks.len() >= 256 || !self.blocks.insert(block) {
            return Err(fail());
        }
        Ok(*self.ir.cfg().block(block).map_err(|_| fail())?.payload())
    }
    fn successor(&self, block: BlockId) -> Result<(EdgeId, BlockId), Unsupported> {
        let node = self.ir.cfg().block(block).map_err(|_| fail())?;
        let [id] = node.outgoing() else {
            return Err(fail());
        };
        let edge = self.ir.cfg().edge(*id).map_err(|_| fail())?;
        if edge.kind() != &EdgeKind::Unconditional {
            return Err(fail());
        }
        if let ManagedInsn::Branch { then_edge, .. } =
            self.ir.instruction(*node.payload()).map_err(|_| fail())?
            && then_edge != id
        {
            return Err(fail());
        }
        Ok((*id, edge.target()))
    }
    fn primitive(
        &mut self,
        id: InsnId,
        state: &mut State,
        arm: bool,
    ) -> Result<Option<Step>, Unsupported> {
        let instruction = self.ir.instruction(id).map_err(|_| fail())?;
        // Frontend initial-register definitions can be placed at CFG entry.
        // They bind the already supplied original call frame; they are never
        // emitted as instructions or accepted as fresh contextual definitions.
        if let ManagedInsn::Opaque {
            reads,
            writes,
            effects,
            ..
        } = instruction
            && reads.is_empty()
            && *effects == EffectFlags::NONE
            && let [value] = writes.as_slice()
            && self.initial.contains(value)
            && self.ir.value(*value).map_err(|_| fail())?.def() == id
            && !arm
        {
            return Ok(None);
        }
        let (result, ty) = match instruction {
            ManagedInsn::Const {
                result,
                value: Constant::Int(_) | Constant::Zero,
            } => (*result, JavaType::Int),
            ManagedInsn::Move { result, value }
                if state.available.contains(value) && *value != self.receiver =>
            {
                (*result, state.types.get(value).ok_or_else(fail)?.clone())
            }
            ManagedInsn::Arithmetic {
                result,
                op: ArithOp::And,
                lhs,
                rhs,
            } if !arm
                && state.available.contains(lhs)
                && state.available.contains(rhs)
                && state.types.get(lhs) == Some(&JavaType::Int)
                && state.types.get(rhs) == Some(&JavaType::Int)
                && self.mask_bit(*result).is_some() =>
            {
                (*result, JavaType::Int)
            }
            ManagedInsn::Compare {
                result,
                op: CompareOp::Equal | CompareOp::NotEqual,
                lhs,
                rhs,
            } if !arm
                && state.available.contains(lhs)
                && state.available.contains(rhs)
                && state.types.get(lhs) == Some(&JavaType::Int)
                && state.types.get(rhs) == Some(&JavaType::Int)
                && self.guard(*result).is_ok() =>
            {
                (*result, JavaType::Boolean)
            }
            ManagedInsn::Branch {
                condition: None,
                else_edge: None,
                ..
            } => return Ok(None),
            _ => return Err(fail()),
        };
        self.definition(id, result, ty, state)?;
        Ok(Some(Step::Instruction(id)))
    }
    fn arm(
        &mut self,
        first: EdgeId,
        merge: BlockId,
        base: &State,
    ) -> Result<(Vec<Step>, State, EdgeId), Unsupported> {
        let mut edge = first;
        let mut state = base.clone();
        let mut steps = Vec::new();
        loop {
            let block = self.ir.cfg().edge(edge).map_err(|_| fail())?.target();
            if block == merge {
                return Ok((steps, state, edge));
            }
            let node = self.ir.cfg().block(block).map_err(|_| fail())?;
            if node.incoming() != [edge] {
                return Err(fail());
            }
            let id = self.visit(block)?;
            if let Some(step) = self.primitive(id, &mut state, true)? {
                steps.push(step);
            }
            edge = self.successor(block)?.0;
        }
    }
    fn diamond(
        &mut self,
        branch: BlockId,
        state: &mut State,
    ) -> Result<(Step, BlockId), Unsupported> {
        if self.choices.len() >= 32 {
            return Err(fail());
        }
        let cfg = self.ir.cfg();
        let node = cfg.block(branch).map_err(|_| fail())?;
        let ManagedInsn::Branch {
            condition: Some(condition),
            then_edge,
            else_edge: Some(else_edge),
        } = self.ir.instruction(*node.payload()).map_err(|_| fail())?
        else {
            return Err(fail());
        };
        if node.outgoing().len() != 2
            || then_edge == else_edge
            || !node.outgoing().contains(then_edge)
            || !node.outgoing().contains(else_edge)
            || !state.available.contains(condition)
            || state.types.get(condition) != Some(&JavaType::Boolean)
        {
            return Err(fail());
        }
        for (edge, kind) in [
            (*then_edge, EdgeKind::BranchTrue),
            (*else_edge, EdgeKind::BranchFalse),
        ] {
            let edge = cfg.edge(edge).map_err(|_| fail())?;
            if edge.source() != branch || edge.kind() != &kind {
                return Err(fail());
            }
        }
        let (bit, default_on_true) = self.guard(*condition)?;
        let merge = decomp_core::analysis::post_dominators(cfg)
            .immediate_post_dominator(branch)
            .ok_or_else(fail)?;
        let (mut yes_steps, yes, yes_edge) = self.arm(*then_edge, merge, state)?;
        let (mut no_steps, no, no_edge) = self.arm(*else_edge, merge, state)?;
        let head = cfg.block(merge).map_err(|_| fail())?;
        let incoming = head.incoming().iter().copied().collect::<HashSet<_>>();
        if incoming.len() != 2 || incoming != HashSet::from([yes_edge, no_edge]) {
            return Err(fail());
        }
        let mut at = merge;
        let mut selected = Vec::new();
        let mut destinations = HashSet::new();
        let mut yes_copies = Vec::new();
        let mut no_copies = Vec::new();
        loop {
            let node = cfg.block(at).map_err(|_| fail())?;
            let id = *node.payload();
            let ManagedInsn::Phi { result, inputs } =
                self.ir.instruction(id).map_err(|_| fail())?
            else {
                break;
            };
            if at != merge && node.incoming().len() != 1 {
                return Err(fail());
            }
            if inputs.len() != 2
                || inputs.iter().map(|(edge, _)| *edge).collect::<HashSet<_>>() != incoming
                || !destinations.insert(*result)
            {
                return Err(fail());
            }
            self.visit(at)?;
            let y = inputs
                .iter()
                .find(|(edge, _)| *edge == yes_edge)
                .map(|(_, value)| *value)
                .ok_or_else(fail)?;
            let n = inputs
                .iter()
                .find(|(edge, _)| *edge == no_edge)
                .map(|(_, value)| *value)
                .ok_or_else(fail)?;
            if !yes.available.contains(&y) || !no.available.contains(&n) {
                return Err(fail());
            }
            selected.push((*result, y, n, id));
            yes_copies.push(Copy {
                destination: *result,
                source: y,
            });
            no_copies.push(Copy {
                destination: *result,
                source: n,
            });
            at = self.successor(at)?.1;
        }
        if selected.is_empty()
            || selected
                .iter()
                .any(|(_, y, n, _)| destinations.contains(y) || destinations.contains(n))
        {
            return Err(fail());
        }
        let mut choices = Vec::new();
        for (result, y, n, id) in selected {
            let (default, original) = if default_on_true { (y, n) } else { (n, y) };
            if !self.zero(default) {
                return Err(fail());
            }
            let original = self.root(original).ok_or_else(fail)?;
            if !self.initial.contains(&original)
                || original == self.receiver
                || original == self.mask
            {
                return Err(fail());
            }
            let ty = state.types.get(&original).ok_or_else(fail)?.clone();
            if !matches!(ty, JavaType::Class(_) | JavaType::Array(_)) {
                return Err(fail());
            }
            let default = self.nullable_default_zero(default)?;
            self.definition(id, result, ty, state)?;
            choices.push((result, original, default));
        }
        yes_steps.push(Step::Copies {
            edge: yes_edge,
            copies: yes_copies,
        });
        no_steps.push(Step::Copies {
            edge: no_edge,
            copies: no_copies,
        });
        self.choices.push(Choice {
            bit,
            inputs: choices,
        });
        Ok((
            Step::If {
                condition: *condition,
                then_steps: yes_steps,
                else_steps: no_steps,
            },
            at,
        ))
    }
}

/// Prove an original synthetic nullable-default mask constructor. Each selected
/// argument is its matching formal or an actual null constant on the original
/// mask-bit edge. No field/effect/throw/scope/inventory obligation is relaxed.
pub fn plan(
    input: &ConstructorInput<'_>,
    resolver: &impl ConstructorResolver,
) -> Result<MaskChoicePlan, Unsupported> {
    if resolver.constructor_source_level() < 25
        || resolver
            .method_access(input.reference)
            .is_none_or(|flags| flags & 0x1000 == 0)
    {
        return Err(fail());
    }
    let caller = resolver.initializer(input.reference).ok_or_else(fail)?;
    let bindings = input.bindings.declaration_bindings();
    let count = caller.method.parameters.len();
    if count < 3
        || bindings.parameters.len() != count
        || caller.method.is_static
        || caller.method.name != "<init>"
        || caller.method.return_type != JavaType::Void
        || caller.method.parameters[count - 2] != JavaType::Int
        || !matches!(caller.method.parameters[count - 1], JavaType::Class(_))
    {
        return Err(fail());
    }
    let receiver = bindings.receiver.ok_or_else(fail)?;
    let mask = bindings.parameters[count - 2].ok_or_else(fail)?;
    let mut initial = HashSet::from([receiver]);
    let mut types = HashMap::from([(receiver, JavaType::Class(caller.method.owner.clone()))]);
    for (value, ty) in bindings.parameters.iter().zip(&caller.method.parameters) {
        if let Some(value) = value {
            if !initial.insert(*value) {
                return Err(fail());
            }
            types.insert(*value, ty.clone());
        }
    }
    for value in &initial {
        let record = input.ir.value(*value).map_err(|_| fail())?;
        if !matches!(record.ty(), managed_ir::types::ManagedType::Unknown)
            && crate::managed_type(record.ty(), resolver)? != types[value]
        {
            return Err(fail());
        }
        if !matches!(input.ir.instruction(record.def()).map_err(|_|fail())?,ManagedInsn::Opaque{reads,writes,effects,..}if reads.is_empty()&&writes.as_slice()==[*value]&&*effects==EffectFlags::NONE)
        {
            return Err(fail());
        }
    }
    let cfg = input.ir.cfg();
    cfg.validate().map_err(|_| fail())?;
    let mut placed = HashSet::new();
    for block in cfg.block_ids() {
        if !placed.insert(*cfg.block(block).map_err(|_| fail())?.payload()) {
            return Err(fail());
        }
    }
    let mut proof = Proof {
        ir: input.ir,
        receiver,
        mask,
        initial: initial.clone(),
        blocks: HashSet::new(),
        definitions: HashMap::new(),
        null_defaults: HashSet::new(),
        choices: Vec::new(),
    };
    let mut state = State {
        available: initial,
        types,
    };
    let mut steps = Vec::new();
    let mut at = cfg.entry();
    loop {
        let node = cfg.block(at).map_err(|_| fail())?;
        let id = *node.payload();
        let insn = input.ir.instruction(id).map_err(|_| fail())?;
        if let ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method,
            args,
        } = insn
        {
            let target = resolver.initializer(*method).ok_or_else(fail)?;
            let args = crate::invoke_arguments::operands(&target.method, args)?;
            if proof.choices.is_empty()
                || args.first() != Some(&receiver)
                || target.method.owner != caller.method.owner
                || target.method.name != "<init>"
                || target.method.is_static
                || target.method.return_type != JavaType::Void
                || target.method.parameters != caller.method.parameters[..count - 2]
                || args.len() != count - 1
                || args.iter().any(|value| !state.available.contains(value))
            {
                return Err(fail());
            }
            let mut selected = HashMap::new();
            for choice in &proof.choices {
                for (result, original, _) in &choice.inputs {
                    let position = bindings.parameters[..count - 2]
                        .iter()
                        .position(|v| v == &Some(*original))
                        .ok_or_else(fail)?;
                    if position >= 32
                        || choice.bit != 1u32 << position
                        || selected.insert(*result, position).is_some()
                    {
                        return Err(fail());
                    }
                }
            }
            let mut used = HashSet::new();
            for (position, value) in args[1..].iter().enumerate() {
                let root = proof.root(*value).ok_or_else(fail)?;
                if let Some(selected_position) = selected.get(&root) {
                    if *selected_position != position || !used.insert(root) {
                        return Err(fail());
                    }
                } else if bindings.parameters[position] != Some(root) {
                    return Err(fail());
                }
            }
            if used.len() != selected.len() {
                return Err(fail());
            }
            // No alternative entry from an unaccounted suffix or disconnected
            // predecessor may make these pre-chain definitions available.
            for block in proof.blocks.iter().copied().chain([at]) {
                let node = cfg.block(block).map_err(|_| fail())?;
                if node.incoming().iter().any(|edge| {
                    cfg.edge(*edge)
                        .map_or(true, |edge| !proof.blocks.contains(&edge.source()))
                }) {
                    return Err(fail());
                }
            }
            let locals = input
                .ir
                .values()
                .iter()
                .filter_map(|(value, _)| {
                    proof.definitions.get(&value).map(|ty| (value, ty.clone()))
                })
                .collect();
            return Ok(MaskChoicePlan {
                chain_block: at,
                target: *method,
                locals,
                null_defaults: proof.null_defaults,
                steps,
                prefix_blocks: proof.blocks,
            });
        }
        proof.visit(at)?;
        if matches!(
            insn,
            ManagedInsn::Branch {
                condition: Some(_),
                ..
            }
        ) {
            let (step, next) = proof.diamond(at, &mut state)?;
            steps.push(step);
            at = next;
        } else {
            if let Some(step) = proof.primitive(id, &mut state, false)? {
                steps.push(step);
            }
            at = proof.successor(at)?.1;
        }
    }
}
