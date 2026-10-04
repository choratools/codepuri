//! Exact selected original field-throw target evidence, not a Throwable permission.
use super::*;

/// Pool identities resolved from the same selected genuine code item.
#[derive(Clone, Debug)]
pub struct FieldThrowDefinition {
    /// Genuine selected DEX and method-table identity, retained as provenance.
    pub original_identity: String,
    /// Exact original code offset (zero means unavailable).
    pub code_offset: u32,
    /// Original access flags; native/abstract/instance methods are excluded.
    pub access_flags: u32,
    /// Original register frame and incoming word count.
    pub registers: u16,
    /// Original incoming word count.
    pub incoming: u16,
    /// Full original instruction code units, not a selected slice.
    pub words: Vec<u16>,
    /// Genuine original handler count.
    pub handlers: usize,
    /// Original encoded declared exceptions; this recipe needs an absent clause.
    pub original_throws: Vec<String>,
    /// Genuine original type operand index in this same DEX pool.
    pub checked_type_index: u16,
    /// Genuine original field operand index in this same DEX pool.
    pub field_index: u16,
    /// Original instance-of type descriptor.
    pub checked_type: String,
    /// Original check-cast type descriptor.
    pub cast_type: String,
    /// Actual declaration owner of the selected field operand.
    pub field_owner: String,
    /// Actual selected field descriptor.
    pub field_type: String,
}

/// A target-specific body witness whose source declaration remains unchanged.
/// The frontend must bind pool identities/aliases to the genuine selected owner.
#[derive(Clone, Debug)]
pub struct BoundTarget {
    reference: MethodRef,
    method: JavaMethod,
    original_identity: String,
    code_offset: u32,
}
impl BoundTarget {
    /// Validate the complete original straight-line field-throw shape and source
    /// declaration. Missing Throwable hierarchy remains a blocker. This witness
    /// can be consumed only by a whole-body outer boundary, never a site helper.
    pub fn field_throw(
        reference: MethodRef,
        method: JavaMethod,
        definition: &FieldThrowDefinition,
        resolver: &impl JavaResolver,
    ) -> Result<Self, Unsupported> {
        let w = &definition.words;
        if definition.original_identity.is_empty()
            || definition.code_offset == 0
            || definition.access_flags & 0x8 == 0
            || definition.access_flags & (0x20 | 0x100 | 0x400 | 0x20000) != 0
            || definition.registers != 2
            || definition.incoming != 1
            || definition.handlers != 0
            || !definition.original_throws.is_empty()
            || !method.is_static
            || method.name.starts_with('<')
            || method.parameters != [JavaType::Class("java.lang.Object".into())]
            || method.return_type != JavaType::Void
            || method.throws != ["java.lang.Throwable"]
            || !resolver.assignable(
                &JavaType::Class("java.lang.Throwable".into()),
                &JavaType::Class("java.lang.Object".into()),
            )
        {
            return Err(failure("bound field-throw declaration/hierarchy"));
        }
        // Includes the original ATHROW, preserving the null-created NPE there.
        if w.len() != 10
            || w[0] != 0x1020
            || w[2] != 0x0039
            || w[3] != 3
            || w[4] != 0x000e
            || w[5] != 0x011f
            || w[7] != 0x1154
            || w[9] != 0x0127
            || w[1] != w[6]
            || w[1] != definition.checked_type_index
            || w[8] != definition.field_index
            || definition.checked_type != definition.cast_type
            || definition.cast_type != definition.field_owner
            || definition.field_type != "Ljava/lang/Throwable;"
        {
            return Err(failure("bound field-throw original body"));
        }
        Ok(Self {
            reference,
            method,
            original_identity: definition.original_identity.clone(),
            code_offset: definition.code_offset,
        })
    }
    /// Exact original identity; synthetic helpers never use this as their identity.
    pub fn original_identity(&self) -> &str {
        &self.original_identity
    }
    /// Exact original code item location retained for diagnostics.
    pub fn code_offset(&self) -> u32 {
        self.code_offset
    }
    pub(super) fn matches(&self, reference: MethodRef, target: &JavaMethod) -> bool {
        self.reference == reference && same_method(&self.method, target)
    }
}
