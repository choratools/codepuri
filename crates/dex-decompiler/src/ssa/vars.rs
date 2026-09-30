//! SSA variables: physical register lineages and ephemeral definitions.
//!
//! A *variable* is either one physical register's same-width lineage (whose
//! `ValueId` history the lifter recorded in `RegisterSlot` histories) or an
//! *ephemeral* value that belongs to no register, such as lifter-synthesized
//! constants, branch predicates, initial register reads, and handler inputs.
//! Ephemeral values have exactly one definition, so they never need phis;
//! register lineages with two or more definitions are the phi candidates.
//!
//! Dalvik registers are untyped 32-bit slots, so one register may hold a
//! category-2 value at one program point and be re-defined as a category-1
//! value later. Each register is therefore partitioned into per-width-
//! category lineages (`One`, `Two`, and an `Ambiguous` bucket for values
//! whose lifted type does not yet decide a category): a redefinition that
//! switches width category starts a new variable, so phis only ever merge
//! same-category values. This mirrors jadx's `CodeVar` merging, which also
//! refuses to unify variables of different width. Without the split, a phi
//! (or its trivial-phi replacement) would demand one `ValueId` satisfy both
//! categories, which no type assignment can.
//!
//! `home` is derived from the value, not the textual register of a read: a
//! wide pair binds both registers to one `ValueId` whose home is the low
//! register's category-two lineage, which keeps category-2 values atomic
//! through renaming.

use std::collections::HashMap;

use managed_ir::method::{RegisterSlotArena, ValueArena};
use managed_ir::types::{ManagedType, ValueCategory};
use managed_ir::value::ValueId;

/// Which lineage a variable represents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VarKind {
    /// One physical register's same-width definition history.
    Register(u16),
    /// A single-definition value outside any register history.
    Ephemeral(ValueId),
}

/// The width category a register lineage is restricted to.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
enum WidthBucket {
    /// Category-1 values only.
    One,
    /// Category-2 values only.
    Two,
    /// Values whose lifted type does not yet decide a category (`Unknown`,
    /// `Conflict`). Values with neither an opcode width claim nor a typed
    /// category land here, so genuinely unknown lineages never merge with
    /// the register's known category-1 or category-2 lineages either.
    Ambiguous,
}

/// Buckets a value by the width category its defining opcode claimed, or
/// failing that, by its lifted type's category.
fn bucket_of(
    values: &ValueArena,
    claims: &HashMap<ValueId, ValueCategory>,
    value: ValueId,
) -> WidthBucket {
    match claims.get(&value).copied() {
        Some(ValueCategory::One) => return WidthBucket::One,
        Some(ValueCategory::Two) => return WidthBucket::Two,
        _ => {}
    }
    match values
        .get(value)
        .map(|record| record.ty())
        .and_then(ManagedType::category)
    {
        Some(ValueCategory::One) => WidthBucket::One,
        Some(ValueCategory::Two) => WidthBucket::Two,
        None => WidthBucket::Ambiguous,
    }
}

/// The variable model of one method under transformation.
#[derive(Debug, Default)]
pub(crate) struct Variables {
    homes: HashMap<ValueId, u32>,
    kinds: Vec<VarKind>,
    defs: Vec<Vec<ValueId>>,
}

impl Variables {
    /// Builds the model from register histories.
    ///
    /// Slots are processed in ascending register order so a value shared by
    /// a wide pair takes the low register as its home. Each register's
    /// history is split into per-width-category lineages — keyed by the
    /// defining opcode's width claim, then the lifted type — so a category
    /// switch across redefinitions of the same register never unifies into
    /// one variable.
    pub fn build(
        reg_slots: &RegisterSlotArena,
        values: &ValueArena,
        claims: &HashMap<ValueId, ValueCategory>,
    ) -> Self {
        let mut slots: Vec<&managed_ir::method::RegisterSlot> = reg_slots.values().collect();
        slots.sort_by_key(|slot| slot.index());
        let mut register_vars: HashMap<(u16, WidthBucket), u32> = HashMap::new();
        let mut model = Self::default();
        for slot in &slots {
            for &value in slot.values() {
                let bucket = bucket_of(values, claims, value);
                let var = *register_vars
                    .entry((slot.index(), bucket))
                    .or_insert_with(|| model.push_kind(VarKind::Register(slot.index())));
                model.homes.entry(value).or_insert(var);
            }
        }
        for slot in &slots {
            for &value in slot.values() {
                let bucket = bucket_of(values, claims, value);
                if let Some(&var) = register_vars.get(&(slot.index(), bucket))
                    && model.homes.get(&value) == Some(&var)
                {
                    model.defs[var as usize].push(value);
                }
            }
        }
        for value in values.keys() {
            if !model.homes.contains_key(&value) {
                let var = model.push_kind(VarKind::Ephemeral(value));
                model.homes.insert(value, var);
                model.defs[var as usize].push(value);
            }
        }
        model
    }

    fn push_kind(&mut self, kind: VarKind) -> u32 {
        let var = self.kinds.len() as u32;
        self.kinds.push(kind);
        self.defs.push(Vec::new());
        var
    }

    /// Returns the variable a value belongs to.
    pub fn var_of(&self, value: ValueId) -> u32 {
        self.homes[&value]
    }

    /// Registers the home of a freshly created value (a phi result or an
    /// entry initial).
    pub fn bind(&mut self, value: ValueId, var: u32) {
        self.homes.insert(value, var);
    }

    /// Returns the defining values of a variable in def-site order.
    pub fn definitions(&self, var: u32) -> &[ValueId] {
        &self.defs[var as usize]
    }

    /// Returns register variables with two or more definitions, in variable
    /// order. These are the only phi candidates.
    pub fn multi_def_vars(&self) -> Vec<u32> {
        self.defs
            .iter()
            .enumerate()
            .filter(|(var, defs)| {
                defs.len() >= 2 && matches!(self.kinds[*var], VarKind::Register(_))
            })
            .map(|(var, _)| var as u32)
            .collect()
    }

    /// Returns every ephemeral variable's single definition value.
    pub fn ephemeral_defs(&self) -> impl Iterator<Item = (u32, ValueId)> + '_ {
        self.kinds
            .iter()
            .enumerate()
            .filter_map(|(var, kind)| match kind {
                VarKind::Ephemeral(value) => Some((var as u32, *value)),
                VarKind::Register(_) => None,
            })
    }

    /// Returns the number of variables.
    pub fn count(&self) -> usize {
        self.kinds.len()
    }
}
