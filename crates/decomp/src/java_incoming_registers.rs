//! Descriptor-ordered physical DEX incoming-register layout.
//!
//! This metadata is independent of SSA: unused incoming words still have slots,
//! and a category-two parameter is one semantic parameter occupying two words.

use dex_format::{DexFile, ids::ProtoId};
use std::fmt;

/// Physical words occupied by one receiver or declared parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IncomingSlot {
    /// First (low) physical register.
    pub register: u16,
    /// Second physical register for `J` or `D`; absent for references and arrays.
    pub high_register: Option<u16>,
}

/// Validated incoming suffix of a DEX register frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingLayout {
    /// Total physical register count (also the exclusive incoming end).
    pub registers_size: u16,
    /// Physical incoming word count, including the instance receiver.
    pub ins_size: u16,
    /// First incoming register; equals the frame size for an empty suffix.
    pub incoming_start: u16,
    /// One-word receiver preceding declared parameters for instance methods.
    pub receiver: Option<IncomingSlot>,
    /// One entry per declared parameter, in prototype order.
    pub parameters: Vec<IncomingSlot>,
}

/// Invalid signature metadata or incoming-frame accounting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncomingLayoutError {
    /// A prototype type reference could not be resolved.
    UnresolvedType(u32),
    /// A parameter is not a legal non-void DEX type descriptor.
    InvalidParameterDescriptor {
        /// Zero-based parameter position in the prototype.
        index: usize,
        /// Rejected original type descriptor.
        descriptor: String,
    },
    /// The descriptor word count cannot fit in a DEX `u16` field.
    WordCountOverflow,
    /// The prototype and receiver do not account for exactly `ins_size` words.
    WordCountMismatch {
        /// Word count calculated from the parameters and receiver.
        expected: u16,
        /// Incoming word count recorded in the code item.
        actual: u16,
    },
    /// Incoming words exceed the physical register frame.
    IncomingExceedsFrame {
        /// Total physical register count in the code item.
        registers_size: u16,
        /// Incoming word count that exceeds the register frame.
        ins_size: u16,
    },
    /// A physical register calculation overflowed.
    RegisterOverflow,
}

impl fmt::Display for IncomingLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnresolvedType(i) => write!(f, "unresolved incoming prototype type {i}"),
            Self::InvalidParameterDescriptor { index, descriptor } => {
                write!(
                    f,
                    "invalid incoming parameter {index} descriptor {descriptor:?}"
                )
            }
            Self::WordCountOverflow => write!(f, "incoming descriptor word count overflow"),
            Self::WordCountMismatch { expected, actual } => write!(
                f,
                "incoming descriptor word count mismatch: expected {expected}, got {actual}"
            ),
            Self::IncomingExceedsFrame {
                registers_size,
                ins_size,
            } => write!(
                f,
                "incoming registers exceed frame: {ins_size} words in {registers_size} registers"
            ),
            Self::RegisterOverflow => write!(f, "incoming register arithmetic overflow"),
        }
    }
}

impl std::error::Error for IncomingLayoutError {}

impl IncomingLayout {
    /// Resolve the full prototype descriptors (not the shorty) from a DEX pool.
    /// `is_static` must come from the encoded method's access flags.
    pub fn from_proto(
        dex: &DexFile,
        proto: &ProtoId,
        is_static: bool,
        registers_size: u16,
        ins_size: u16,
    ) -> Result<Self, IncomingLayoutError> {
        let descriptors = proto
            .parameters
            .iter()
            .flat_map(|p| &p.types)
            .map(|index| {
                let index = u32::from(*index);
                let ty = dex
                    .type_id(index)
                    .map_err(|_| IncomingLayoutError::UnresolvedType(index))?;
                dex.string(ty.descriptor_idx)
                    .map_err(|_| IncomingLayoutError::UnresolvedType(index))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::from_descriptors(&descriptors, is_static, registers_size, ins_size)
    }

    /// Calculate the incoming suffix from standard parameter descriptors.
    /// Return descriptors do not consume incoming words and are not an input.
    pub fn from_descriptors(
        descriptors: &[&str],
        is_static: bool,
        registers_size: u16,
        ins_size: u16,
    ) -> Result<Self, IncomingLayoutError> {
        let mut expected = u16::from(!is_static);
        let mut widths = Vec::with_capacity(descriptors.len());
        for (index, descriptor) in descriptors.iter().enumerate() {
            let width = parameter_width(descriptor).ok_or_else(|| {
                IncomingLayoutError::InvalidParameterDescriptor {
                    index,
                    descriptor: (*descriptor).into(),
                }
            })?;
            expected = expected
                .checked_add(width)
                .ok_or(IncomingLayoutError::WordCountOverflow)?;
            widths.push(width);
        }
        if expected != ins_size {
            return Err(IncomingLayoutError::WordCountMismatch {
                expected,
                actual: ins_size,
            });
        }
        let incoming_start = registers_size.checked_sub(ins_size).ok_or(
            IncomingLayoutError::IncomingExceedsFrame {
                registers_size,
                ins_size,
            },
        )?;
        let mut next = incoming_start;
        let receiver = if is_static {
            None
        } else {
            Some(take_slot(&mut next, 1)?)
        };
        let parameters = widths
            .into_iter()
            .map(|width| take_slot(&mut next, width))
            .collect::<Result<Vec<_>, _>>()?;
        debug_assert_eq!(next, registers_size);
        Ok(Self {
            registers_size,
            ins_size,
            incoming_start,
            receiver,
            parameters,
        })
    }
}

fn take_slot(next: &mut u16, width: u16) -> Result<IncomingSlot, IncomingLayoutError> {
    let register = *next;
    let high_register = if width == 2 {
        Some(
            register
                .checked_add(1)
                .ok_or(IncomingLayoutError::RegisterOverflow)?,
        )
    } else {
        None
    };
    *next = register
        .checked_add(width)
        .ok_or(IncomingLayoutError::RegisterOverflow)?;
    Ok(IncomingSlot {
        register,
        high_register,
    })
}

fn parameter_width(descriptor: &str) -> Option<u16> {
    let element = descriptor.trim_start_matches('[');
    let dimensions = descriptor.len() - element.len();
    if dimensions > 255 {
        return None;
    }
    let valid = match element {
        "Z" | "B" | "S" | "C" | "I" | "F" | "J" | "D" => true,
        _ => element
            .strip_prefix('L')
            .and_then(|s| s.strip_suffix(';'))
            .is_some_and(|name| {
                !name.is_empty()
                    && name.split('/').all(|part| {
                        !part.is_empty()
                            && !part
                                .chars()
                                .any(|c| matches!(c, '.' | ';' | '[' | '(' | ')' | '\0'))
                    })
            }),
    };
    if !valid {
        return None;
    }
    Some(if dimensions == 0 && matches!(element, "J" | "D") {
        2
    } else {
        1
    })
}
