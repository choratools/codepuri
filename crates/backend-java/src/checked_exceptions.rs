//! Java checked-exception coverage from attested declared hierarchy edges.
use crate::{JavaResolver, JavaType};

/// A thrown declaration without a proved unchecked or covered relationship.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UncoveredExceptions {
    /// Original source names of the uncovered thrown types, in input order.
    pub types: Vec<String>,
}

/// Checks that every callee exception is unchecked or covered by a caller's
/// declared superclass/interface type. Exact identity needs no ancestor lookup;
/// all nonexact relationships require positive resolver evidence. Unknown and
/// unrelated types remain uncovered, without invented catch blocks or throws.
pub fn check(
    resolver: &impl JavaResolver,
    thrown: &[String],
    declared: &[String],
) -> Result<(), UncoveredExceptions> {
    let uncovered: Vec<_> = thrown
        .iter()
        .filter(|name| {
            let actual = JavaType::Class((*name).clone());
            let unchecked = ["java.lang.RuntimeException", "java.lang.Error"]
                .iter()
                .any(|base| {
                    *name == base || resolver.assignable(&actual, &JavaType::Class((*base).into()))
                });
            !unchecked
                && !declared.iter().any(|expected| {
                    *name == expected
                        || resolver.assignable(&actual, &JavaType::Class(expected.clone()))
                })
        })
        .cloned()
        .collect();
    if uncovered.is_empty() {
        Ok(())
    } else {
        Err(UncoveredExceptions { types: uncovered })
    }
}
