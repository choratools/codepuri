//! Errors raised by managed IR arena access.

use decomp_core::cfg::CfgError;
use thiserror::Error;

use crate::method::RegisterSlotId;
use crate::value::{InsnId, ValueId};

/// An invalid managed IR lookup or graph operation.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum IrError {
    /// An instruction identity is absent or stale.
    #[error("instruction {0:?} does not exist")]
    MissingInstruction(InsnId),
    /// A value identity is absent or stale.
    #[error("value {0:?} does not exist")]
    MissingValue(ValueId),
    /// A register-slot identity is absent or stale.
    #[error("register slot {0:?} does not exist")]
    MissingRegisterSlot(RegisterSlotId),
    /// A control-flow graph operation failed.
    #[error(transparent)]
    Cfg(#[from] CfgError),
}
