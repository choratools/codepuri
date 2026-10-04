//! Declaration-based access checks in the original naming domain.
use backend_java::{
    JavaType,
    type_compatibility::{Compatibility, Hierarchy},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessBlocker {
    InvalidName,
    InvalidFlags,
    ClassInaccessible,
    Private,
    Package,
    ProtectedCaller,
    ProtectedReceiver,
    AliasPackageChanged,
}

/// Only explicit, trusted nest metadata is admissible. A `$` name is not proof.
pub trait NestEvidence {
    fn same_nest(&self, caller: &str, owner: &str) -> bool;
}

pub struct AccessContext<'a> {
    /// Original dotted binary names, before sanitization or aliases.
    pub caller: &'a str,
    pub owner: &'a str,
    pub receiver: Option<&'a str>,
    pub hierarchy: &'a Hierarchy,
    pub nests: Option<&'a dyn NestEvidence>,
    /// Emitted dotted names. Cross-package aliasing must preserve access.
    pub emitted_caller: &'a str,
    pub emitted_owner: &'a str,
}

pub fn package(name: &str) -> Result<&str, AccessBlocker> {
    if name.is_empty()
        || name.contains('/')
        || name.contains(';')
        || name.split('.').any(str::is_empty)
    {
        return Err(AccessBlocker::InvalidName);
    }
    Ok(name.rsplit_once('.').map_or("", |(p, _)| p))
}

impl AccessContext<'_> {
    /// Verify access in both the original and emitted naming domains. The two
    /// graphs must be independently attested; aliases are never type evidence.
    pub fn method_access_with_source_hierarchy(
        &self,
        flags: u32,
        source_hierarchy: &Hierarchy,
        source_receiver: Option<&str>,
    ) -> Result<(), AccessBlocker> {
        self.method_access(flags)?;
        let source = AccessContext {
            caller: self.emitted_caller,
            owner: self.emitted_owner,
            receiver: source_receiver,
            hierarchy: source_hierarchy,
            nests: None,
            emitted_caller: self.emitted_caller,
            emitted_owner: self.emitted_owner,
        };
        source.method_access(flags)
    }

    fn subtype(&self, actual: &str, expected: &str) -> bool {
        self.hierarchy.assignable(
            &JavaType::Class(actual.into()),
            &JavaType::Class(expected.into()),
        ) == Compatibility::Assignable
    }

    /// Top-level declaration access. Nested class access needs its enclosing
    /// declaration chain checked separately by the trusted metadata provider.
    pub fn class_access(&self, flags: u32) -> Result<(), AccessBlocker> {
        if flags & 6 != 0 {
            return Err(AccessBlocker::ClassInaccessible);
        }
        if flags & 1 != 0 {
            return Ok(());
        }
        self.package_access()
    }

    fn package_access(&self) -> Result<(), AccessBlocker> {
        if package(self.caller)? != package(self.owner)? {
            return Err(AccessBlocker::Package);
        }
        if package(self.emitted_caller)? != package(self.emitted_owner)? {
            return Err(AccessBlocker::AliasPackageChanged);
        }
        Ok(())
    }

    /// Check a resolved declaration, never a method-pool reference. For protected
    /// instance access the receiver's *static* type must be caller or a subtype.
    /// `super` supplies caller as receiver after dispatch has independently been proven.
    pub fn method_access(&self, flags: u32) -> Result<(), AccessBlocker> {
        if (flags & 7).count_ones() > 1 {
            return Err(AccessBlocker::InvalidFlags);
        }
        package(self.caller)?;
        package(self.owner)?;
        match flags & 7 {
            1 => Ok(()),
            2 if self.caller == self.owner => Ok(()),
            2 if self
                .nests
                .is_some_and(|n| n.same_nest(self.caller, self.owner)) =>
            {
                Ok(())
            }
            2 => Err(AccessBlocker::Private),
            4 => {
                if package(self.caller)? == package(self.owner)? {
                    return self.package_access();
                }
                if !self.subtype(self.caller, self.owner) {
                    return Err(AccessBlocker::ProtectedCaller);
                }
                if flags & 8 == 0 && !self.receiver.is_some_and(|r| self.subtype(r, self.caller)) {
                    return Err(AccessBlocker::ProtectedReceiver);
                }
                Ok(())
            }
            _ => self.package_access(),
        }
    }
}
