//! Family-neutral side-effect flags for opaque instructions.

use core::ops::{BitOr, BitOrAssign};

/// A compact set of semantic effects attached to an opaque instruction.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct EffectFlags(u16);

impl EffectFlags {
    /// No externally visible effects.
    pub const NONE: Self = Self(0);
    /// Reads managed memory.
    pub const READS_MEMORY: Self = Self(1 << 0);
    /// Writes managed memory.
    pub const WRITES_MEMORY: Self = Self(1 << 1);
    /// May transfer control by throwing.
    pub const MAY_THROW: Self = Self(1 << 2);
    /// Participates in synchronization.
    pub const SYNCHRONIZES: Self = Self(1 << 3);
    /// May terminate normal control flow.
    pub const TERMINATES: Self = Self(1 << 4);

    /// Creates flags from frontend-defined bits.
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// Returns the underlying bit representation.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Returns whether every bit in `other` is present.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for EffectFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for EffectFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}
