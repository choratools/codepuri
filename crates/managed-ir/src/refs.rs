//! Opaque references into frontend-owned metadata pools.

macro_rules! opaque_reference {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u32);

        impl $name {
            /// Creates a reference from its frontend-owned pool index.
            pub const fn new(index: u32) -> Self {
                Self(index)
            }

            /// Returns the unresolved pool index.
            pub const fn index(self) -> u32 {
                self.0
            }
        }
    };
}

opaque_reference!(
    /// An unresolved method-pool index.
    ///
    /// Frontends own the referenced pool and are responsible for resolution.
    MethodRef
);
opaque_reference!(
    /// An unresolved field-pool index.
    ///
    /// Frontends own the referenced pool and are responsible for resolution.
    FieldRef
);
opaque_reference!(
    /// An unresolved class-pool index.
    ///
    /// Frontends own the referenced pool and are responsible for resolution.
    ClassRef
);
opaque_reference!(
    /// An unresolved type-pool index.
    ///
    /// Frontends own the referenced pool and are responsible for resolution.
    TypeRef
);
opaque_reference!(
    /// An unresolved string-pool index.
    ///
    /// Frontends own the referenced pool and are responsible for resolution.
    StringRef
);
