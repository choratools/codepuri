//! Register-slot to semantic-value binding for the lifter.
//!
//! Physical register indices are treated as unbounded slot identifiers: slots
//! and their value histories are allocated on demand and never validated
//! against the declared `registers_size`, matching the recovering decoder.
//! A category-2 (wide) value occupies the register pair `vN, vN+1` atomically:
//! both halves resolve to the same [`ValueId`].

use std::collections::HashMap;

use managed_ir::effects::EffectFlags;
use managed_ir::insn::ManagedInsn;
use managed_ir::method::{InsnArena, RegisterSlot, RegisterSlotArena, ValueArena};
use managed_ir::ops::{CompareOp, Constant};
use managed_ir::types::{ManagedType, ValueCategory};
use managed_ir::value::{InsnId, Value, ValueId};

/// Family tag for synthetic values that exist before any definition:
/// parameters and reads of never-written registers.
pub(crate) const TAG_INITIAL: u16 = 0x8001;
/// Family tag for the synthetic value a `move-exception` receives at a
/// handler entry.
pub(crate) const TAG_HANDLER: u16 = 0x8002;

const PLACEHOLDER: ManagedInsn = ManagedInsn::Opaque {
    family_tag: 0,
    reads: Vec::new(),
    writes: Vec::new(),
    effects: EffectFlags::NONE,
};

/// Arenas plus the register-file view used while walking one method.
pub(crate) struct ValueTable {
    initial_types: HashMap<(u16, bool), ManagedType>,
    pub(crate) insns: InsnArena,
    pub(crate) values: ValueArena,
    pub(crate) reg_slots: RegisterSlotArena,
    slot_ids: HashMap<u16, managed_ir::method::RegisterSlotId>,
    latest: HashMap<u16, ValueId>,
    by_width: HashMap<(u16, ValueCategory), ValueId>,
    /// Per-value width promises fixed by the defining opcode (`aget-wide`,
    /// `move-wide`, `iget-wide`, fused `move-result-wide`, ...). Dalvik
    /// opcodes fix operand widths independent of type flow, so these claims
    /// let SSA split register lineages by width category even where the
    /// lifted type is still `Unknown`.
    widths: HashMap<ValueId, ValueCategory>,
}

impl ValueTable {
    pub(crate) fn new(initial_types: HashMap<(u16, bool), ManagedType>) -> Self {
        Self {
            initial_types,
            insns: InsnArena::with_key(),
            values: ValueArena::with_key(),
            reg_slots: RegisterSlotArena::with_key(),
            slot_ids: HashMap::new(),
            latest: HashMap::new(),
            by_width: HashMap::new(),
            widths: HashMap::new(),
        }
    }

    /// Reserves an instruction slot and returns its identity. The reserved
    /// slot is patched with the final instruction after its defined values
    /// (which reference this identity) have been created.
    pub(crate) fn reserve(&mut self) -> InsnId {
        self.insns.insert(PLACEHOLDER)
    }

    /// Creates a standalone synthetic definition of one value.
    ///
    /// Synthetic definitions (literal operands, comparison results, handler
    /// inputs, initial register states) live in the arena without owning a
    /// CFG block, because each block carries exactly one lifted instruction.
    pub(crate) fn synthetic_def(
        &mut self,
        ty: ManagedType,
        build: impl FnOnce(ValueId) -> ManagedInsn,
    ) -> ValueId {
        let def = self.reserve();
        let value = self.values.insert(Value::new(def, ty));
        self.insns[def] = build(value);
        value
    }

    fn slot(&mut self, index: u16) -> managed_ir::method::RegisterSlotId {
        match self.slot_ids.get(&index) {
            Some(&existing) => existing,
            None => {
                let created = self.reg_slots.insert(RegisterSlot::new(index));
                self.slot_ids.insert(index, created);
                created
            }
        }
    }

    fn append_definition(&mut self, reg: u16, value: ValueId) {
        let slot = self.slot(reg);
        self.reg_slots[slot].append(value);
        self.latest.insert(reg, value);
    }

    fn bind(&mut self, def: InsnId, reg: u16, ty: ManagedType, wide: bool) -> ValueId {
        let value = self.values.insert(Value::new(def, ty));
        let width = if wide {
            ValueCategory::Two
        } else {
            ValueCategory::One
        };
        self.widths.insert(value, width);
        self.by_width.insert((reg, width), value);
        self.append_definition(reg, value);
        if wide {
            self.append_definition(reg.wrapping_add(1), value);
        }
        value
    }

    /// Defines a new category-1 value for `reg`.
    pub(crate) fn define(&mut self, def: InsnId, reg: u16, ty: ManagedType) -> ValueId {
        self.bind(def, reg, ty, false)
    }

    /// Detaches the recorded width claims.
    pub(crate) fn take_width_claims(&mut self) -> HashMap<ValueId, ValueCategory> {
        std::mem::take(&mut self.widths)
    }

    /// Defines one atomic category-2 value covering the pair `reg, reg+1`.
    pub(crate) fn define_wide(&mut self, def: InsnId, reg: u16, ty: ManagedType) -> ValueId {
        let value = self.bind(def, reg, ty, true);
        self.widths.insert(value, ValueCategory::Two);
        value
    }

    /// Returns the most recent value stored in `reg`, synthesizing an
    /// initial value when the register has never been written.
    pub(crate) fn read(&mut self, reg: u16) -> ValueId {
        self.read_typed(reg, false)
    }

    /// Without a prototype, retain register-word identities for invocation
    /// recovery. Context-aware lifting uses opcode/descriptor-directed reads.
    pub(crate) fn read_word(&mut self, reg: u16) -> ValueId {
        match self.latest.get(&reg) {
            Some(&value) => value,
            None => self.read(reg),
        }
    }

    /// Opcode-directed reads select a physical register's width lineage,
    /// independently of the last definition encountered in file order.
    pub(crate) fn read_typed(&mut self, reg: u16, wide: bool) -> ValueId {
        let width = if wide {
            ValueCategory::Two
        } else {
            ValueCategory::One
        };
        if let Some(value) = self.by_width.get(&(reg, width)) {
            return *value;
        }
        let initial_type = self
            .initial_types
            .get(&(reg, wide))
            .cloned()
            .unwrap_or(ManagedType::Unknown);
        let value = self.synthetic_def(initial_type, |value| ManagedInsn::Opaque {
            family_tag: TAG_INITIAL,
            reads: Vec::new(),
            writes: vec![value],
            effects: EffectFlags::NONE,
        });
        self.widths.insert(value, width);
        self.by_width.insert((reg, width), value);
        self.append_definition(reg, value);
        if wide {
            self.append_definition(reg.wrapping_add(1), value);
        }
        value
    }

    /// Creates a standalone constant value usable as a synthetic operand.
    pub(crate) fn const_value(&mut self, constant: Constant, ty: ManagedType) -> ValueId {
        self.synthetic_def(ty, |value| ManagedInsn::Const {
            result: value,
            value: constant,
        })
    }

    /// Creates the synthetic boolean result of a two-way comparison.
    pub(crate) fn compare_value(&mut self, op: CompareOp, lhs: ValueId, rhs: ValueId) -> ValueId {
        self.synthetic_def(ManagedType::Boolean, |value| ManagedInsn::Compare {
            op,
            result: value,
            lhs,
            rhs,
        })
    }

    /// Creates the per-handler exception value a `move-exception` receives.
    pub(crate) fn handler_value(&mut self) -> ValueId {
        self.synthetic_def(ManagedType::Unknown, |value| ManagedInsn::Opaque {
            family_tag: TAG_HANDLER,
            reads: Vec::new(),
            writes: vec![value],
            effects: EffectFlags::MAY_THROW,
        })
    }
}
