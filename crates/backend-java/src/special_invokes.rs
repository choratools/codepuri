//! Proof-carrying direct/private and super invocation planning.
use managed_ir::ops::InvokeKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Exact source dispatch selected by declaration proof.
pub enum InvocationPlan {
    /// Static target.
    Static,
    /// Ordinary class dispatch.
    Virtual,
    /// Interface dispatch.
    Interface,
    /// Same-owner private direct call.
    DirectPrivate,
    /// Proven exact superclass dispatch.
    Super,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Missing or conflicting dispatch evidence.
pub enum DispatchBlocker {
    /// Opcode and declaration staticness disagree.
    StaticMismatch,
    /// Private declaration cannot use virtual dispatch.
    PrivateVirtual,
    /// Special selection has no exact source proof.
    UnprovenSpecial,
    /// Initializers require the allocation planner.
    Constructor,
    /// Abstract superclass target has no implementation.
    AbstractSuper,
}

/// Frontend must resolve the exact target selected by DEX invoke-super and the
/// exact declaration selected by Java `super.name(args)`. Equality includes the
/// original owner, name, parameters and return descriptor, before aliases.
pub trait SpecialInvokeResolver {
    /// Whether both dispatch rules select the identical original declaration.
    fn java_super_selects_dex_target(&self) -> bool;
}

/// Attested invocation facts in the original naming domain.
pub struct DispatchContext<'a> {
    /// Calling class.
    pub caller: &'a str,
    /// Exact declaring class.
    pub declaration_owner: &'a str,
    /// Original method name.
    pub declaration_name: &'a str,
    /// Declaration access bits.
    pub access_flags: u32,
    /// Receiver is the incoming receiver identity.
    pub receiver_is_this: bool,
    /// Exact superclass selection proof.
    pub super_resolver: Option<&'a dyn SpecialInvokeResolver>,
}
impl DispatchContext<'_> {
    /// Reject dispatch that cannot be represented exactly.
    pub fn plan(&self, kind: InvokeKind) -> Result<InvocationPlan, DispatchBlocker> {
        if self.declaration_name == "<init>" || self.declaration_name == "<clinit>" {
            return Err(DispatchBlocker::Constructor);
        }
        if (kind == InvokeKind::Static) != (self.access_flags & 8 != 0) {
            return Err(DispatchBlocker::StaticMismatch);
        }
        match kind {
            InvokeKind::Static => Ok(InvocationPlan::Static),
            InvokeKind::Virtual | InvokeKind::Interface if self.access_flags & 2 != 0 => {
                Err(DispatchBlocker::PrivateVirtual)
            }
            InvokeKind::Virtual => Ok(InvocationPlan::Virtual),
            InvokeKind::Interface => Ok(InvocationPlan::Interface),
            InvokeKind::Special
                if self.access_flags & 2 != 0 && self.caller == self.declaration_owner =>
            {
                Ok(InvocationPlan::DirectPrivate)
            }
            InvokeKind::Super
                if self.access_flags & (2 | 0x40 | 0x80) == 0
                    && self.receiver_is_this
                    && self
                        .super_resolver
                        .is_some_and(|r| r.java_super_selects_dex_target()) =>
            {
                if self.access_flags & 0x400 != 0 {
                    Err(DispatchBlocker::AbstractSuper)
                } else {
                    Ok(InvocationPlan::Super)
                }
            }
            _ => Err(DispatchBlocker::UnprovenSpecial),
        }
    }
}

/// An exact-target frontend attestation used only with retained Super provenance.
pub struct SuperProof(pub bool);
impl SpecialInvokeResolver for SuperProof {
    fn java_super_selects_dex_target(&self) -> bool {
        self.0
    }
}
