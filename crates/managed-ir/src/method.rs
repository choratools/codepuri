//! Type-state method containers and register assignment histories.

use core::marker::PhantomData;

use decomp_core::cfg::Cfg;
use slotmap::{SlotMap, new_key_type};

use crate::error::IrError;
use crate::insn::ManagedInsn;
use crate::value::{InsnId, Value, ValueId};

new_key_type! {
    /// A generational identity for one physical register history.
    pub struct RegisterSlotId;
}

/// Arena storing managed instructions.
pub type InsnArena = SlotMap<InsnId, ManagedInsn>;

/// Arena storing semantic values.
pub type ValueArena = SlotMap<ValueId, Value>;

/// Arena storing physical register histories.
pub type RegisterSlotArena = SlotMap<RegisterSlotId, RegisterSlot>;

/// Marker for register form, where physical registers may be reassigned.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RegisterForm;

/// Marker for SSA form, where each semantic value must have one definition.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SsaForm;

/// The values assigned to one physical register in definition order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterSlot {
    index: u16,
    values: Vec<ValueId>,
}

impl RegisterSlot {
    /// Creates an empty history for a physical register index.
    pub const fn new(index: u16) -> Self {
        Self {
            index,
            values: Vec::new(),
        }
    }

    /// Returns the physical register index.
    pub const fn index(&self) -> u16 {
        self.index
    }

    /// Returns values in definition order.
    pub fn values(&self) -> &[ValueId] {
        &self.values
    }

    /// Appends a definition in source-offset order.
    pub fn append(&mut self, value: ValueId) {
        self.values.push(value);
    }
}

/// A managed method whose stage is encoded by marker `F`.
///
/// Each CFG block carries one [`InsnId`], while instructions and values live in
/// generational arenas. Frontend lifters build [`RegisterForm`] and append each
/// register definition in source-offset order. SSA transformations live in
/// pass crates: they construct [`SsaForm`] with [`MethodIr::from_parts`] and
/// must run the structural verifier before publishing the result.
#[derive(Debug)]
pub struct MethodIr<F> {
    cfg: Cfg<InsnId>,
    insns: InsnArena,
    values: ValueArena,
    reg_slots: RegisterSlotArena,
    form: PhantomData<F>,
}

impl MethodIr<RegisterForm> {
    /// Creates register-form IR from independently populated arenas.
    pub fn from_parts(
        cfg: Cfg<InsnId>,
        insns: InsnArena,
        values: ValueArena,
        reg_slots: RegisterSlotArena,
    ) -> Self {
        Self::new(cfg, insns, values, reg_slots)
    }

    /// Returns mutable register histories for a frontend lifter.
    pub const fn register_slots_mut(&mut self) -> &mut RegisterSlotArena {
        &mut self.reg_slots
    }
}

impl MethodIr<SsaForm> {
    /// Creates SSA-form IR without physical register provenance.
    ///
    /// This constructor does not perform an SSA transformation or silently
    /// certify its inputs. Call [`crate::verify::SsaVerifier`] after assembly.
    pub fn from_parts(cfg: Cfg<InsnId>, insns: InsnArena, values: ValueArena) -> Self {
        Self::new(cfg, insns, values, RegisterSlotArena::with_key())
    }

    /// Creates SSA-form IR while retaining register histories as provenance.
    pub fn from_parts_with_registers(
        cfg: Cfg<InsnId>,
        insns: InsnArena,
        values: ValueArena,
        reg_slots: RegisterSlotArena,
    ) -> Self {
        Self::new(cfg, insns, values, reg_slots)
    }
}

impl<F> MethodIr<F> {
    fn new(
        cfg: Cfg<InsnId>,
        insns: InsnArena,
        values: ValueArena,
        reg_slots: RegisterSlotArena,
    ) -> Self {
        Self {
            cfg,
            insns,
            values,
            reg_slots,
            form: PhantomData,
        }
    }

    /// Returns the control-flow graph.
    pub const fn cfg(&self) -> &Cfg<InsnId> {
        &self.cfg
    }

    /// Returns the instruction arena.
    pub const fn instructions(&self) -> &InsnArena {
        &self.insns
    }

    /// Returns the value arena.
    pub const fn values(&self) -> &ValueArena {
        &self.values
    }

    /// Returns retained physical register histories.
    pub const fn register_slots(&self) -> &RegisterSlotArena {
        &self.reg_slots
    }

    /// Returns an instruction or a typed stale-key error.
    pub fn instruction(&self, id: InsnId) -> Result<&ManagedInsn, IrError> {
        self.insns.get(id).ok_or(IrError::MissingInstruction(id))
    }

    /// Returns a value or a typed stale-key error.
    pub fn value(&self, id: ValueId) -> Result<&Value, IrError> {
        self.values.get(id).ok_or(IrError::MissingValue(id))
    }
}
