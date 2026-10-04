//! Incoming DEX register provenance for explicit Java parameter declarations.
//!
//! The frontend allocates initial values on first read, not from the signature.
//! An absent initial therefore means an unused input, even when the register's
//! history contains later writes. Never substitute one of those writes.
use backend_java::{JavaMethod, JavaType};
use managed_ir::{
    effects::EffectFlags,
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
    value::ValueId,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// One signature position, including positions with no IR value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InitialParameter {
    /// A proven synthetic initial definition, never a current register value.
    Bound(ValueId),
    /// An input never read before being overwritten (or never touched).
    Unused {
        /// Declaration name retained even though no initial IR value exists.
        name: String,
    },
}

/// Backend integration must preserve every position, including unused inputs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitialParameterPlan {
    /// `None` only for static methods; an unused receiver is named `this`.
    pub receiver: Option<InitialParameter>,
    /// Descriptor order, with deterministic `pN` names for unused arguments.
    pub parameters: Vec<InitialParameter>,
}

/// Distinguishes malformed signatures from uncertain input provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InitialParameterError {
    /// Descriptor word count or frame bounds are inconsistent.
    InvalidLayout(String),
    /// Initial identities cannot be mapped uniquely and at the correct width.
    InitialParameterAmbiguity(String),
}

impl std::fmt::Display for InitialParameterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLayout(reason) => write!(f, "incoming parameter layout: {reason}"),
            Self::InitialParameterAmbiguity(reason) => {
                write!(f, "initial parameter ambiguity: {reason}")
            }
        }
    }
}
impl std::error::Error for InitialParameterError {}

fn initial(ir: &MethodIr<SsaForm>, value: ValueId) -> bool {
    ir.value(value)
        .ok()
        .and_then(|v| ir.instruction(v.def()).ok())
        .is_some_and(|insn| {
            matches!(insn,
            ManagedInsn::Opaque { family_tag: 0x8001 | 0x8003, reads, writes, effects }
            if reads.is_empty() && writes.as_slice() == [value] && *effects == EffectFlags::NONE)
        })
}

/// Build a complete declaration plan without fabricating `ValueId`s.
///
/// Histories identify inputs by synthetic definition, not by first/last value.
/// Category-2 inputs require the same initial identity in exactly both words.
/// SSA entry initials lacking register provenance are rejected: guessing their
/// register would incorrectly classify a potentially used input as unused.
pub fn parameter_bindings(
    ir: &MethodIr<SsaForm>,
    method: &JavaMethod,
    registers: u16,
    incoming: u16,
) -> Result<InitialParameterPlan, InitialParameterError> {
    parameter_bindings_with_provenance(ir, method, registers, incoming, &[])
}

/// Lifter-attested physical words for an SSA entry initial. Capture this before
/// SSA rewriting drops register histories; never derive it from current writes,
/// arena order, name hints, or the inferred type. Wide values list both words.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitialProvenance {
    /// The exact synthetic entry definition identity.
    pub value: ValueId,
    /// All physical words occupied by this entry value.
    pub registers: Vec<u16>,
}

/// Bind declaration identities using histories plus explicit lifter provenance.
/// Supplied evidence must agree exactly with any surviving initial history.
pub fn parameter_bindings_with_provenance(
    ir: &MethodIr<SsaForm>,
    method: &JavaMethod,
    registers: u16,
    incoming: u16,
    provenance: &[InitialProvenance],
) -> Result<InitialParameterPlan, InitialParameterError> {
    use InitialParameterError::{InitialParameterAmbiguity as Ambiguous, InvalidLayout};
    let width = |ty: &JavaType| match ty {
        JavaType::Long | JavaType::Double => Ok(2u32),
        JavaType::Void => Err(InvalidLayout("void parameter".into())),
        _ => Ok(1),
    };
    let widths = method
        .parameters
        .iter()
        .map(width)
        .collect::<Result<Vec<_>, _>>()?;
    let expected = widths.iter().sum::<u32>() + u32::from(!method.is_static);
    if expected != u32::from(incoming) {
        return Err(InvalidLayout(
            "incoming descriptor word count mismatch".into(),
        ));
    }
    let mut reg = registers
        .checked_sub(incoming)
        .ok_or_else(|| InvalidLayout("incoming registers exceed frame".into()))?;
    let mut histories = BTreeMap::<u16, Vec<ValueId>>::new();
    let mut value_registers = BTreeMap::<ValueId, BTreeSet<u16>>::new();
    for slot in ir.register_slots().values() {
        if histories
            .insert(slot.index(), slot.values().to_vec())
            .is_some()
        {
            return Err(Ambiguous(format!(
                "duplicate register history {}",
                slot.index()
            )));
        }
        for &value in slot.values() {
            value_registers
                .entry(value)
                .or_default()
                .insert(slot.index());
        }
    }
    let mut attested = HashSet::new();
    for entry in provenance {
        let words: BTreeSet<_> = entry.registers.iter().copied().collect();
        if !initial(ir, entry.value)
            || !attested.insert(entry.value)
            || words.is_empty()
            || words.len() != entry.registers.len()
            || words.iter().any(|r| *r >= registers)
            || value_registers
                .get(&entry.value)
                .is_some_and(|old| old != &words)
        {
            return Err(Ambiguous(
                "invalid or contradictory explicit initial provenance".into(),
            ));
        }
        for &r in &words {
            let history = histories.entry(r).or_default();
            if !history.contains(&entry.value) {
                history.push(entry.value);
            }
        }
        value_registers.insert(entry.value, words);
    }
    for (value, _) in ir.values() {
        if initial(ir, value) && !value_registers.contains_key(&value) {
            return Err(Ambiguous(
                "synthetic initial lacks register provenance".into(),
            ));
        }
    }
    let bind = |start: u16, words: u16, name: String| {
        let candidates = |r| -> Result<HashSet<ValueId>, InitialParameterError> {
            let mut found = HashSet::new();
            for &v in histories.get(&r).map(Vec::as_slice).unwrap_or(&[]) {
                ir.value(v)
                    .map_err(|_| Ambiguous(format!("stale value in register {r}")))?;
                if initial(ir, v) {
                    found.insert(v);
                }
            }
            if found.len() > 1 {
                return Err(Ambiguous(format!(
                    "multiple initial definitions for register {r}"
                )));
            }
            Ok(found)
        };
        let low = candidates(start)?;
        if words == 2 && candidates(start + 1)? != low {
            return Err(Ambiguous(format!(
                "split wide incoming parameter at register {start}"
            )));
        }
        let Some(value) = low.into_iter().next() else {
            return Ok(InitialParameter::Unused { name });
        };
        for &r in &value_registers[&value] {
            if !(start..start + words).contains(&r) {
                return Err(Ambiguous(format!(
                    "initial at register {start} aliases register {r}"
                )));
            }
        }
        Ok(InitialParameter::Bound(value))
    };
    let receiver = if method.is_static {
        None
    } else {
        let receiver = bind(reg, 1, "this".into())?;
        reg += 1;
        Some(receiver)
    };
    let mut parameters = Vec::with_capacity(widths.len());
    for (index, words) in widths.into_iter().enumerate() {
        let words = words as u16;
        parameters.push(bind(reg, words, format!("p{index}"))?);
        reg += words;
    }
    Ok(InitialParameterPlan {
        receiver,
        parameters,
    })
}

#[cfg(test)]
mod provenance_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use managed_ir::{
        method::{InsnArena, ValueArena},
        types::ManagedType,
        value::Value,
    };

    #[test]
    fn explicit_provenance_recovers_only_attested_initials() {
        let mut insns = InsnArena::with_key();
        let mut values = ValueArena::with_key();
        let def = insns.insert(ManagedInsn::Return { value: None });
        let initial = values.insert(Value::new(def, ManagedType::Unknown));
        insns[def] = ManagedInsn::Opaque {
            family_tag: 0x8003,
            reads: vec![],
            writes: vec![initial],
            effects: EffectFlags::NONE,
        };
        let later_def = insns.insert(ManagedInsn::Return { value: None });
        let later = values.insert(Value::new(later_def, ManagedType::Int));
        insns[later_def] = ManagedInsn::Const {
            result: later,
            value: managed_ir::ops::Constant::Int(7),
        };
        let ir = MethodIr::<SsaForm>::from_parts(decomp_core::cfg::Cfg::new(def), insns, values);
        let method = JavaMethod {
            owner: "fixture.Parameters".into(),
            name: "test".into(),
            is_static: true,
            return_type: JavaType::Void,
            parameters: vec![JavaType::Long],
            throws: vec![],
        };
        assert!(parameter_bindings(&ir, &method, 2, 2).is_err());
        let evidence = InitialProvenance {
            value: initial,
            registers: vec![0, 1],
        };
        let plan = parameter_bindings_with_provenance(&ir, &method, 2, 2, &[evidence]).unwrap();
        assert_eq!(plan.parameters, vec![InitialParameter::Bound(initial)]);
        for evidence in [
            InitialProvenance {
                value: later,
                registers: vec![0, 1],
            },
            InitialProvenance {
                value: initial,
                registers: vec![0],
            },
            InitialProvenance {
                value: initial,
                registers: vec![0, 0],
            },
            InitialProvenance {
                value: initial,
                registers: vec![1, 2],
            },
        ] {
            assert!(parameter_bindings_with_provenance(&ir, &method, 2, 2, &[evidence]).is_err());
        }
    }
}
