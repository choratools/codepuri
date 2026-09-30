//! Typed identities and records for managed values and instructions.

use slotmap::new_key_type;

use crate::types::ManagedType;

new_key_type! {
    /// A generational identity for one semantic value.
    pub struct ValueId;

    /// A generational identity for one managed instruction.
    pub struct InsnId;
}

/// Metadata associated with one semantic value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Value {
    def: InsnId,
    ty: ManagedType,
    name_hint: Option<String>,
}

impl Value {
    /// Creates an unnamed value defined by `def`.
    pub const fn new(def: InsnId, ty: ManagedType) -> Self {
        Self {
            def,
            ty,
            name_hint: None,
        }
    }

    /// Attaches a non-semantic source-level naming hint.
    pub fn with_name_hint(mut self, name_hint: impl Into<String>) -> Self {
        self.name_hint = Some(name_hint.into());
        self
    }

    /// Returns the instruction that defines this value.
    pub const fn def(&self) -> InsnId {
        self.def
    }

    /// Returns the inferred managed type.
    pub const fn ty(&self) -> &ManagedType {
        &self.ty
    }

    /// Returns the optional source-level naming hint.
    pub fn name_hint(&self) -> Option<&str> {
        self.name_hint.as_deref()
    }
}
