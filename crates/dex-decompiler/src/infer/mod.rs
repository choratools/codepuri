//! Stage 2.4c: worklist type inference over the managed type lattice.
//!
//! Inference is flow-insensitive and monotone. Consumer constraints
//! *narrow* values via the lattice meet (cast results, opcode families,
//! field and method descriptors, returns); merge points *widen* phi
//! results via the lattice join of their inputs. A constraint applied to
//! a phi result is pushed down onto its inputs instead, so every stored
//! type only ever moves downward and the worklist terminates.
//!
//! Conflicts are never stored: when a meet produces
//! [`ManagedType::Conflict`], the previous type is retained and a typed
//! diagnostic carries the instruction offset. Unresolvable pool
//! references are aggregated into one warning by the context rather than
//! flooding per-instruction.

mod context;
mod descriptor;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod array_cast_regressions;
#[cfg(test)]
mod comparison_promotion_tests;
#[cfg(test)]
mod contextual_tests;

mod expansion;
#[cfg(test)]
mod expansion_tests;

pub use context::{DexMetadata, DexTypes, NoTypes};

use std::collections::{HashMap, HashSet, VecDeque};

use decomp_core::cfg::Cfg;
use decomp_core::diagnostics::{Diagnostic, Severity};
use managed_ir::insn::{ArithOp, Constant, ManagedInsn, OrderingOp};
use managed_ir::method::InsnArena;
use managed_ir::types::{self, ManagedType};
use managed_ir::value::{InsnId, ValueId};

use crate::ssa::uses::{for_each_definition, for_each_use};

const MAX_CONFLICT_DIAGNOSTICS: usize = 16;
const MAX_ARITY_DIAGNOSTICS: usize = 4;

/// Pool-resolution and enclosing-method context for inference.
pub trait TypeContext {
    /// Identity of a `<clinit>` context whose raw lift is equivalent to a fresh
    /// `DexTypes::new` context for this DEX and method. Contexts with additional
    /// register or value seeds must not opt in.
    fn original_static_initializer_identity(&self) -> Option<(usize, u32)> {
        None
    }

    /// Declared incoming type at an attested physical low word and width.
    fn initial_register_type(&self, _register: u16, _wide: bool) -> Option<ManagedType> {
        None
    }
    /// Exact declaration type keyed by proven SSA initial identity, never a
    /// current physical register write. Declaration types are immutable seeds.
    fn initial_type(&self, _value: ValueId) -> Option<ManagedType> {
        None
    }
    /// The managed type of string-pool constants, if resolvable.
    fn string_type(&self) -> Option<ManagedType> {
        None
    }

    /// Runtime type of a class-literal result, independently of its target.
    fn class_literal_type(&self) -> Option<ManagedType> {
        None
    }

    /// The managed type of a type-pool index, if resolvable.
    fn type_at(&self, type_idx: u32) -> Option<ManagedType> {
        let _ = type_idx;
        None
    }

    /// The declared type of a field-pool index, if resolvable.
    fn field_type(&self, field_idx: u32) -> Option<ManagedType> {
        let _ = field_idx;
        None
    }

    /// The return type of a method-pool index; `None` for void or
    /// unresolved methods.
    fn method_return(&self, method_idx: u32) -> Option<ManagedType> {
        let _ = method_idx;
        None
    }

    /// Fills `params` with the declared parameter types of a method-pool
    /// index, returning whether the prototype resolved.
    fn method_params(&self, method_idx: u32, params: &mut Vec<ManagedType>) -> bool {
        let _ = (method_idx, params);
        false
    }

    /// The return type of the method under analysis, if resolvable.
    fn enclosing_return(&self) -> Option<ManagedType> {
        None
    }

    /// Records an unresolvable pool reference by kind.
    fn note_unresolved(&self, kind: &'static str, index: u32) {
        let _ = (kind, index);
    }

    /// Drains the accumulated unresolved-reference counts.
    fn take_unresolved(&self) -> Vec<(&'static str, u64)> {
        Vec::new()
    }
}

/// One refinement produced by an instruction's constraint rule.
enum Refinement {
    /// Narrow a value toward a type (or push the constraint onto phi
    /// inputs when the value is a phi result).
    Meet(ValueId, ManagedType),
    /// Recompute the defining phi's join from its input types.
    Join,
}

/// Infers lattice types for every value of a materialized SSA method.
///
/// `types` is seeded with the lifter's annotations and refined in place.
pub fn infer_types(
    _cfg: &Cfg<InsnId>,
    insns: &InsnArena,
    types: &mut HashMap<ValueId, ManagedType>,
    offsets: &HashMap<InsnId, u32>,
    ctx: &dyn TypeContext,
) -> Vec<Diagnostic> {
    let mut phi_results: HashMap<ValueId, InsnId> = HashMap::new();
    let mut definitions = HashMap::new();
    let mut raw_constants = HashSet::new();
    let mut users: HashMap<ValueId, Vec<InsnId>> = HashMap::new();
    for (insn_id, record) in insns.iter() {
        if let ManagedInsn::FillArray { values, .. } = record {
            // fill-array-data payload elements are raw little-endian bits
            // too, even though their Const definitions are synthetic.
            raw_constants.extend(values.iter().copied());
        }
        if let ManagedInsn::Const {
            result,
            value: Constant::Int(_) | Constant::Long(_) | Constant::Zero,
        } = record
            && offsets.contains_key(&insn_id)
        {
            // DEX const opcodes store raw register bits. Float/double uses
            // interpret those bits; they do not numerically convert them.
            raw_constants.insert(*result);
            types.insert(*result, ManagedType::Unknown);
        }
        for_each_definition(record, |value| {
            definitions.insert(value, insn_id);
            // A consumer may constrain a result after its defining move,
            // arithmetic, or phi was processed. Revisit that definition to
            // propagate the result constraint back to its operands.
            users.entry(value).or_default().push(insn_id);
            if matches!(record, ManagedInsn::Phi { .. }) {
                phi_results.insert(value, insn_id);
            }
        });
        for_each_use(record, |value| {
            users.entry(value).or_default().push(insn_id)
        });
    }
    for value in &raw_constants {
        types.insert(*value, ManagedType::Unknown);
    }
    for value in definitions.keys() {
        if let Some(declared) = ctx.initial_type(*value) {
            types.insert(*value, declared);
        }
    }
    // Constant/copy identity is invariant during inference. Build the proven
    // raw-one family once; never repeat definition walks in hot refinements.
    let mut raw_ones: HashSet<_> = raw_constants
        .iter()
        .copied()
        .filter(|value| {
            definitions
                .get(value)
                .and_then(|id| insns.get(*id))
                .is_some_and(|record| {
                    matches!(
                        record,
                        ManagedInsn::Const {
                            value: Constant::Int(1),
                            ..
                        }
                    )
                })
        })
        .collect();
    let mut one_pending: VecDeque<_> = raw_ones.iter().copied().collect();
    while let Some(source) = one_pending.pop_front() {
        for id in users.get(&source).into_iter().flatten() {
            if let Some(ManagedInsn::Move { result, value }) = insns.get(*id)
                && *value == source
                && raw_ones.insert(*result)
            {
                one_pending.push_back(*result);
            }
        }
    }
    let mut engine = Engine {
        insns,
        types,
        offsets,
        ctx,
        phi_results: &phi_results,
        definitions: &definitions,
        raw_constants: &raw_constants,
        raw_ones,
        users: &users,
        pending: insns.keys().collect(),
        queued: insns.keys().collect(),
        diagnostics: Vec::new(),
        conflicts_overflow: 0,
        arity_overflow: 0,
        arity_diagnostics: 0,
        params: Vec::new(),
        expansions: expansion::Expansions::default(),
        split_zeros: HashSet::new(),
        split_ones: HashSet::new(),
    };
    engine.run();
    // Unconstrained literals retain the conventional integer interpretation.
    for value in &raw_constants {
        if engine.types.get(value) == Some(&ManagedType::Unknown)
            && let Some(def) = definitions.get(value)
            && let Some(ManagedInsn::Const {
                value: constant, ..
            }) = insns.get(*def)
            && let Some(ty) = const_type(constant, ctx)
        {
            engine.meet_leaf(*value, ty, *def);
        }
    }
    engine.run();
    engine.finish()
}

struct Engine<'a> {
    insns: &'a InsnArena,
    types: &'a mut HashMap<ValueId, ManagedType>,
    offsets: &'a HashMap<InsnId, u32>,
    ctx: &'a dyn TypeContext,
    phi_results: &'a HashMap<ValueId, InsnId>,
    definitions: &'a HashMap<ValueId, InsnId>,
    raw_constants: &'a HashSet<ValueId>,
    raw_ones: HashSet<ValueId>,
    users: &'a HashMap<ValueId, Vec<InsnId>>,
    pending: VecDeque<InsnId>,
    queued: HashSet<InsnId>,
    diagnostics: Vec<Diagnostic>,
    conflicts_overflow: usize,
    arity_overflow: usize,
    arity_diagnostics: usize,
    params: Vec<ManagedType>,
    expansions: expansion::Expansions,
    // Proven pure literals with independently typed consumers have no single
    // nominal interpretation. Once split, do not oscillate between contexts.
    split_zeros: HashSet<ValueId>,
    split_ones: HashSet<ValueId>,
}

impl Engine<'_> {
    fn raw_one(&self, value: ValueId) -> bool {
        self.raw_ones.contains(&value)
    }
    fn raw_zero(&self, mut value: ValueId) -> bool {
        let mut seen = HashSet::new();
        while seen.insert(value) {
            match self
                .definitions
                .get(&value)
                .and_then(|id| self.insns.get(*id))
            {
                Some(ManagedInsn::Const {
                    value: Constant::Zero,
                    ..
                }) => return self.raw_constants.contains(&value),
                Some(ManagedInsn::Move { value: source, .. }) => value = *source,
                _ => return false,
            }
        }
        false
    }
    fn run(&mut self) {
        // Collection must finish before applying refinements: some rules read
        // both operands' pre-refinement types. Reuse storage without changing
        // that snapshot or the order in which constraints are applied.
        let mut refinements = Vec::new();
        while let Some(insn_id) = self.pending.pop_front() {
            self.queued.remove(&insn_id);
            let Some(record) = self.insns.get(insn_id) else {
                continue;
            };
            self.collect(record, insn_id, &mut refinements);
            for refinement in refinements.drain(..) {
                match refinement {
                    Refinement::Meet(value, target) => self.meet(value, target, insn_id),
                    Refinement::Join => self.join_phi(insn_id),
                }
            }
        }
    }

    fn finish(mut self) -> Vec<Diagnostic> {
        for (kind, count) in self.ctx.take_unresolved() {
            self.diagnostics.push(Diagnostic::new(
                Severity::Warning,
                "DEX_TYPE_UNRESOLVED",
                format!("{count} unresolvable {kind} pool reference(s)"),
            ));
        }
        if self.conflicts_overflow > 0 {
            self.diagnostics.push(Diagnostic::new(
                Severity::Error,
                "DEX_TYPE_CONFLICT",
                format!(
                    "{} additional type conflict(s) suppressed",
                    self.conflicts_overflow
                ),
            ));
        }
        if self.arity_overflow > 0 {
            self.diagnostics.push(Diagnostic::new(
                Severity::Warning,
                "DEX_TYPE_ARITY",
                format!(
                    "{} additional invoke arity mismatch(es) suppressed",
                    self.arity_overflow
                ),
            ));
        }
        self.diagnostics
    }

    fn offset(&self, insn_id: InsnId) -> String {
        match self.offsets.get(&insn_id) {
            Some(offset) => format!("offset 0x{offset:x}"),
            None => "synthetic".to_owned(),
        }
    }

    fn meet(&mut self, value: ValueId, target: ManagedType, insn_id: InsnId) {
        // Most constraints target ordinary SSA values. Only phi results need
        // cycle detection and expansion; leaves also need no target clone.
        if !self.phi_results.contains_key(&value) {
            self.meet_leaf(value, target, insn_id);
            return;
        }
        let leaves =
            self.expansions
                .leaves(value, false, self.insns, self.definitions, self.phi_results);
        for &value in leaves.iter() {
            self.meet_leaf_ref(value, &target, insn_id);
        }
    }

    fn meet_leaf(&mut self, value: ValueId, target: ManagedType, insn_id: InsnId) {
        self.meet_leaf_ref(value, &target, insn_id);
    }

    fn meet_leaf_ref(&mut self, value: ValueId, target: &ManagedType, insn_id: InsnId) {
        // One raw category-one literal may independently mean integer 1 or
        // boolean true. Only a definition-proven constant/copy may split;
        // arbitrary Int producers remain incompatible with Boolean consumers.
        // Once mixed, all narrow integral consumers retain the raw Int view;
        // otherwise Byte -> Int -> Byte refinements would never converge.
        // Existing nonintegral/conflicting evidence must use ordinary checks.
        if matches!(
            target,
            ManagedType::Byte
                | ManagedType::Short
                | ManagedType::Char
                | ManagedType::Int
                | ManagedType::Boolean
        ) && self.raw_one(value)
            && self.types.get(&value).is_some_and(|current| {
                matches!(
                    current,
                    ManagedType::Unknown
                        | ManagedType::Byte
                        | ManagedType::Short
                        | ManagedType::Char
                        | ManagedType::Int
                        | ManagedType::Boolean
                )
            })
        {
            let Some(current) = self.types.get(&value) else {
                return;
            };
            let mut next = if self.split_ones.contains(&value) {
                ManagedType::Int
            } else {
                types::meet(current, target)
            };
            if next == ManagedType::Conflict || next == ManagedType::Zero {
                self.split_ones.insert(value);
                next = ManagedType::Int;
            }
            if *current != next {
                self.types.insert(value, next);
                self.push_users(value);
            }
            return;
        }
        // An actual DEX zero is a use-site expression, not a nominal value.
        // Retain the established single-context annotation; mixed contexts
        // retain Zero instead of an order-dependent contradiction.
        // This exemption does not apply to arbitrary Zero annotations or phis:
        // their concrete nonconstant inputs still undergo ordinary checks.
        if self.raw_zero(value)
            && matches!(
                target,
                ManagedType::Zero
                    | ManagedType::Null
                    | ManagedType::Boolean
                    | ManagedType::Byte
                    | ManagedType::Short
                    | ManagedType::Char
                    | ManagedType::Int
                    | ManagedType::Float
                    | ManagedType::Reference(_)
                    | ManagedType::Array { nullable: true, .. }
            )
        {
            let contextual = match target {
                ManagedType::Reference(_) | ManagedType::Array { .. } => &ManagedType::Null,
                _ => target,
            };
            let Some(current) = self.types.get(&value) else {
                return;
            };
            let mut next = if self.split_zeros.contains(&value) {
                ManagedType::Zero
            } else {
                types::meet(current, contextual)
            };
            if next == ManagedType::Conflict
                || (next == ManagedType::Zero
                    && *current != ManagedType::Zero
                    && *contextual != ManagedType::Zero)
            {
                self.split_zeros.insert(value);
                next = ManagedType::Zero;
            }
            if *current != next {
                self.types.insert(value, next);
                self.push_users(value);
            }
            return;
        }
        let Some(current) = self.types.get(&value) else {
            return;
        };
        let mut next = types::meet(current, target);
        // Lattice bottom-like literal intersections are not evidence that an
        // arbitrary runtime value is actually zero/null. In particular an Int
        // parameter consumed as a reference must remain a contradiction.
        if next == ManagedType::Zero
            && *current != ManagedType::Zero
            && *target != ManagedType::Zero
        {
            next = ManagedType::Conflict;
        }
        if next == ManagedType::Null
            && *current != ManagedType::Null
            && *current != ManagedType::Unknown
        {
            if matches!(
                current,
                ManagedType::Reference(_) | ManagedType::Array { .. }
            ) && matches!(
                target,
                ManagedType::Reference(_) | ManagedType::Array { .. }
            ) {
                // Nominal assignment needs directed hierarchy evidence in the
                // checked backend; an intersection cannot invent a null value.
                return;
            }
            next = ManagedType::Conflict;
        }
        if next == ManagedType::Conflict && *current != ManagedType::Conflict {
            if self.diagnostics.len() < MAX_CONFLICT_DIAGNOSTICS {
                self.diagnostics.push(Diagnostic::new(
                    Severity::Error,
                    "DEX_TYPE_CONFLICT",
                    format!(
                        "{}: value {:?} cannot be both {:?} and {:?}",
                        self.offset(insn_id),
                        value,
                        current,
                        target
                    ),
                ));
            } else {
                self.conflicts_overflow += 1;
            }
            return;
        }
        if next != *current && self.ctx.initial_type(value).is_none() {
            self.types.insert(value, next);
            self.push_users(value);
        }
    }

    fn join_phi(&mut self, insn_id: InsnId) {
        let Some(ManagedInsn::Phi { result, .. }) = self.insns.get(insn_id) else {
            return;
        };
        // Join starts at bottom, not Unknown (the lattice's top). Starting
        // at top discards every input type, even two identical exact types.
        let mut joined = ManagedType::Conflict;
        // Expand copy/phi cycles to their concrete reaching definitions.
        // Reading a not-yet-inferred loop phi as Unknown would permanently
        // poison the join, despite exact types on every external input.
        let leaves = self.expansions.leaves(
            *result,
            true,
            self.insns,
            self.definitions,
            self.phi_results,
        );
        for value in leaves.iter() {
            let input = self.types.get(value).unwrap_or(&ManagedType::Unknown);
            joined = types::join(&joined, input);
        }
        if leaves.is_empty() {
            joined = ManagedType::Unknown;
        }
        let result = *result;
        if self.types.get(&result) != Some(&joined) {
            self.types.insert(result, joined);
            self.push_users(result);
        }
    }

    fn push_users(&mut self, value: ValueId) {
        if let Some(users) = self.users.get(&value) {
            for &user in users {
                if self.queued.insert(user) {
                    self.pending.push_back(user);
                }
            }
        }
    }

    fn collect(
        &mut self,
        record: &ManagedInsn,
        insn_id: InsnId,
        refinements: &mut Vec<Refinement>,
    ) {
        match record {
            ManagedInsn::Const { result, value } => {
                if !self.raw_constants.contains(result)
                    && let Some(ty) = const_type(value, self.ctx)
                {
                    refinements.push(Refinement::Meet(*result, ty));
                }
            }
            ManagedInsn::Move { result, value } => {
                let source = self.types.get(value).cloned();
                let target = self.types.get(result).cloned();
                if let Some(ty) = source {
                    refinements.push(Refinement::Meet(*result, ty));
                }
                if let Some(ty) = target {
                    refinements.push(Refinement::Meet(*value, ty));
                }
            }
            ManagedInsn::Phi { .. } => refinements.push(Refinement::Join),
            ManagedInsn::Arithmetic {
                op,
                result,
                lhs,
                rhs,
            } => {
                let result_ty = self.types.get(result).cloned();
                if matches!(
                    op,
                    ArithOp::ShiftLeft | ArithOp::ShiftRight | ArithOp::UnsignedShiftRight
                ) {
                    if let Some(ty) = result_ty {
                        refinements.push(Refinement::Meet(*lhs, ty));
                    }
                    refinements.push(Refinement::Meet(*rhs, ManagedType::Int));
                } else if let Some(ty) = result_ty {
                    refinements.push(Refinement::Meet(*lhs, ty.clone()));
                    refinements.push(Refinement::Meet(*rhs, ty));
                }
            }
            ManagedInsn::Compare {
                result, lhs, rhs, ..
            } => {
                let left = self.types.get(lhs).cloned();
                let right = self.types.get(rhs).cloned();
                refinements.push(Refinement::Meet(*result, ManagedType::Boolean));
                // A contextual literal one cannot establish the domain of an
                // as-yet unresolved producer (e.g. a normal-only invoke commit).
                if (self.raw_one(*lhs)
                    && right.as_ref().is_some_and(|ty| *ty == ManagedType::Unknown))
                    || (self.raw_one(*rhs)
                        && left.as_ref().is_some_and(|ty| *ty == ManagedType::Unknown))
                {
                    return;
                }
                // Comparing category-one integers does not prove that their
                // producers have the same narrow domain. For example, comparing
                // an array length with a byte must not narrow the length to Byte.
                let word_integer = |ty: &ManagedType| {
                    matches!(
                        ty,
                        ManagedType::Byte
                            | ManagedType::Short
                            | ManagedType::Char
                            | ManagedType::Int
                    )
                };
                if (left.as_ref().is_some_and(word_integer) || self.raw_one(*lhs))
                    && (right.as_ref().is_some_and(word_integer) || self.raw_one(*rhs))
                {
                    refinements.push(Refinement::Meet(*lhs, ManagedType::Int));
                    refinements.push(Refinement::Meet(*rhs, ManagedType::Int));
                    return;
                }
                if let Some(ty) = left
                    && !matches!(ty, ManagedType::Zero | ManagedType::Null)
                {
                    refinements.push(Refinement::Meet(*rhs, ty));
                }
                if let Some(ty) = right
                    && !matches!(ty, ManagedType::Zero | ManagedType::Null)
                {
                    refinements.push(Refinement::Meet(*lhs, ty));
                }
            }
            ManagedInsn::OrderingCompare {
                op,
                result,
                lhs,
                rhs,
            } => {
                refinements.push(Refinement::Meet(*result, ManagedType::Int));
                let operand = match op {
                    OrderingOp::CmpLowFloat | OrderingOp::CmpHighFloat => ManagedType::Float,
                    OrderingOp::CmpLowDouble | OrderingOp::CmpHighDouble => ManagedType::Double,
                    OrderingOp::CmpLong => ManagedType::Long,
                };
                refinements.push(Refinement::Meet(*lhs, operand.clone()));
                refinements.push(Refinement::Meet(*rhs, operand));
            }
            ManagedInsn::Neg { result, value } | ManagedInsn::Not { result, value } => {
                let source = self.types.get(value).cloned();
                let target = self.types.get(result).cloned();
                if let Some(ty) = source {
                    refinements.push(Refinement::Meet(*result, ty));
                }
                if let Some(ty) = target {
                    refinements.push(Refinement::Meet(*value, ty));
                }
            }
            ManagedInsn::FieldGet { result, field, .. } => {
                if let Some(ty) = self.ctx.field_type(field.index()) {
                    refinements.push(Refinement::Meet(*result, ty));
                } else {
                    self.ctx.note_unresolved("field", field.index());
                }
            }
            ManagedInsn::FieldPut { field, value, .. } => {
                if let Some(ty) = self.ctx.field_type(field.index()) {
                    refinements.push(Refinement::Meet(*value, ty));
                } else {
                    self.ctx.note_unresolved("field", field.index());
                }
            }
            ManagedInsn::ArrayGet {
                result,
                array,
                index,
            } => {
                refinements.push(Refinement::Meet(*index, ManagedType::Int));
                refinements.push(Refinement::Meet(*array, descriptor::array_shape()));
                let shape = self.types.get(array).cloned();
                if let Some(element) = shape.as_ref().and_then(descriptor::element_of) {
                    refinements.push(Refinement::Meet(*result, element.clone()));
                }
            }
            ManagedInsn::ArrayPut {
                array,
                index,
                value,
            } => {
                refinements.push(Refinement::Meet(*index, ManagedType::Int));
                refinements.push(Refinement::Meet(*array, descriptor::array_shape()));
                let shape = self.types.get(array).cloned();
                if let Some(element) = shape.as_ref().and_then(descriptor::element_of) {
                    refinements.push(Refinement::Meet(*value, element.clone()));
                }
            }
            ManagedInsn::ArrayLength { result, array } => {
                refinements.push(Refinement::Meet(*result, ManagedType::Int));
                refinements.push(Refinement::Meet(*array, descriptor::array_shape()));
            }
            ManagedInsn::NewInstance { result, class } => {
                refinements.push(Refinement::Meet(*result, ManagedType::Reference(*class)));
            }
            ManagedInsn::NewArray {
                result,
                element,
                length,
            } => {
                refinements.push(Refinement::Meet(*length, ManagedType::Int));
                if let Some(ty) = self.ctx.type_at(element.index()) {
                    // DEX new-array names the complete array descriptor in
                    // its type operand, rather than the component descriptor.
                    let array = if matches!(ty, ManagedType::Array { .. }) {
                        ty
                    } else {
                        ManagedType::Array {
                            element: Box::new(ty),
                            nullable: true,
                        }
                    };
                    refinements.push(Refinement::Meet(*result, array));
                } else {
                    self.ctx.note_unresolved("type", element.index());
                }
            }
            ManagedInsn::NewFilledArray {
                result,
                target,
                elements,
            } => {
                if let Some(array) = self.ctx.type_at(target.index()) {
                    if let Some(element) = descriptor::element_of(&array) {
                        for value in elements {
                            refinements.push(Refinement::Meet(*value, element.clone()));
                        }
                    }
                    if let Some(result) = result {
                        refinements.push(Refinement::Meet(*result, array));
                    }
                } else {
                    self.ctx.note_unresolved("type", target.index());
                }
            }
            ManagedInsn::FillArray { array, values } => {
                refinements.push(Refinement::Meet(*array, descriptor::array_shape()));
                let shape = self.types.get(array).cloned();
                if let Some(element) = shape.as_ref().and_then(descriptor::element_of) {
                    for value in values {
                        refinements.push(Refinement::Meet(*value, element.clone()));
                    }
                }
            }
            ManagedInsn::Invoke {
                kind,
                result,
                method,
                args,
                ..
            } => {
                self.collect_invoke(insn_id, *kind, *result, method.index(), args, refinements);
            }
            ManagedInsn::Cast {
                result,
                value,
                target,
            } => {
                // Numeric conversions carry lifter-owned synthetic targets
                // whose result types are already exact; real check-cast
                // targets describe the successful result.
                if target.index() >= crate::lift::SYNTHETIC_TYPE_BASE {
                    return;
                }
                if let Some(ty) = self.ctx.type_at(target.index()) {
                    refinements.push(Refinement::Meet(*result, ty.clone()));
                    // A runtime cast accepts any reference, not only values
                    // already typed as the exact target. Reject known scalars
                    // without narrowing an Object source to an array or Null.
                    if self.types.get(value).is_some_and(|source| {
                        !matches!(
                            source,
                            ManagedType::Unknown
                                | ManagedType::Zero
                                | ManagedType::Null
                                | ManagedType::Reference(_)
                                | ManagedType::Array { .. }
                        )
                    }) {
                        refinements.push(Refinement::Meet(*value, ManagedType::Conflict));
                    }
                } else {
                    self.ctx.note_unresolved("type", target.index());
                }
            }
            ManagedInsn::InstanceOf { result, .. } => {
                refinements.push(Refinement::Meet(*result, ManagedType::Boolean));
            }
            ManagedInsn::Branch {
                condition: Some(condition),
                ..
            } => {
                refinements.push(Refinement::Meet(*condition, ManagedType::Boolean));
            }
            ManagedInsn::Switch { key, .. } => {
                refinements.push(Refinement::Meet(*key, ManagedType::Int));
            }
            ManagedInsn::Return { value: Some(value) } => {
                if let Some(ty) = self.ctx.enclosing_return() {
                    refinements.push(Refinement::Meet(*value, ty));
                }
            }
            ManagedInsn::MonitorEnter { .. }
            | ManagedInsn::MonitorExit { .. }
            | ManagedInsn::Branch {
                condition: None, ..
            }
            | ManagedInsn::Return { value: None }
            | ManagedInsn::Throw { .. }
            | ManagedInsn::Opaque { .. } => {}
        }
    }

    fn collect_invoke(
        &mut self,
        insn_id: InsnId,
        kind: managed_ir::ops::InvokeKind,
        result: Option<ValueId>,
        method_idx: u32,
        args: &[ValueId],
        refinements: &mut Vec<Refinement>,
    ) {
        let resolved = self.ctx.method_params(method_idx, &mut self.params);
        if resolved {
            // DEX invocation arguments are register words, including a receiver
            // for instance dispatch and two words for long/double parameters.
            let receiver = usize::from(kind != managed_ir::ops::InvokeKind::Static);
            let expected = receiver
                + self
                    .params
                    .iter()
                    .map(|ty| {
                        if ty.category() == Some(managed_ir::types::ValueCategory::Two) {
                            2
                        } else {
                            1
                        }
                    })
                    .sum::<usize>();
            if args.len() != expected && self.arity_diagnostics < MAX_ARITY_DIAGNOSTICS {
                self.arity_diagnostics += 1;
                self.diagnostics.push(Diagnostic::new(
                    Severity::Warning,
                    "DEX_TYPE_ARITY",
                    format!(
                        "{}: invoke passes {} argument(s) for {} parameter(s)",
                        self.offset(insn_id),
                        args.len(),
                        expected
                    ),
                ));
            } else if args.len() != expected {
                self.arity_overflow += 1;
            }
            let mut word = receiver;
            for ty in &self.params {
                if let Some(arg) = args.get(word) {
                    refinements.push(Refinement::Meet(*arg, ty.clone()));
                }
                word += if ty.category() == Some(managed_ir::types::ValueCategory::Two) {
                    2
                } else {
                    1
                };
            }
        } else {
            self.ctx.note_unresolved("method", method_idx);
        }
        if let Some(result) = result
            && let Some(ty) = self.ctx.method_return(method_idx)
        {
            refinements.push(Refinement::Meet(result, ty));
        }
    }
}

fn const_type(value: &Constant, ctx: &dyn TypeContext) -> Option<ManagedType> {
    match value {
        Constant::Zero => Some(ManagedType::Zero),
        Constant::Boolean(_flag) => Some(ManagedType::Boolean),
        Constant::Int(_) => Some(ManagedType::Int),
        Constant::Long(_) => Some(ManagedType::Long),
        Constant::FloatBits(_) => Some(ManagedType::Float),
        Constant::DoubleBits(_) => Some(ManagedType::Double),
        Constant::Null => Some(ManagedType::Null),
        Constant::String(_) => ctx.string_type(),
        Constant::ClassLiteral(_) => ctx.class_literal_type(),
    }
}
