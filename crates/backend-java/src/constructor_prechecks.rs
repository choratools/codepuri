//! Bounded proof for commuting pre-initializer checks over an inert parent chain.
//!
//! This module grants no emitter permission by itself. Its declarations and
//! instruction summaries must come from complete input/classpath evidence.
use std::collections::{BTreeMap, BTreeSet};

/// An exact original constructor identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConstructorIdentity {
    /// Original declaring descriptor.
    pub owner: String,
    /// Original parameter/return descriptor.
    pub descriptor: String,
}

/// Complete original class declaration evidence.
#[derive(Clone, Debug)]
pub struct ClassEvidence {
    /// Original descriptor.
    pub owner: String,
    /// Original flags, before aliases.
    pub flags: u32,
    /// Exact original direct superclass.
    pub superclass: Option<String>,
    /// Complete declared method names and descriptors, including native methods.
    pub methods: BTreeSet<(String, String)>,
}

/// Audited operations in their original execution order.
#[derive(Clone, Debug)]
pub enum Operation {
    /// Literal/copy evaluation with no receiver read, allocation or side effect.
    Pure,
    /// Exact original `this` or direct-parent initializer; arguments are inert.
    Chain(ConstructorIdentity),
    /// Ordinary successful return.
    Return,
    /// Anything else, including field access, class initialization and throwing casts.
    Observable,
}

/// Complete, straight-line constructor evidence, not a selected instruction prefix.
#[derive(Clone, Debug)]
pub struct ConstructorEvidence {
    /// Exact original identity.
    pub identity: ConstructorIdentity,
    /// Original operations; handlers, omitted instructions and branches are unsupported.
    pub operations: Vec<Operation>,
    /// An original handler or control-flow split invalidates the inert summary.
    pub has_control_or_handlers: bool,
}

/// Explicit trusted root proof; an Object pool reference is insufficient.
#[derive(Clone, Debug)]
pub struct ObjectRootEvidence {
    /// Original metadata for java.lang.Object.
    pub declaration: ClassEvidence,
    /// Complete audited constructor body from a trusted source.
    pub constructor: ConstructorEvidence,
    /// Complete audit of Object.finalize when declared; absence is not an audit.
    pub finalizer_operations: Option<Vec<Operation>>,
}

/// Why moving a check across construction remains unproved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    /// Missing or inconsistent complete declaration evidence.
    MissingDeclaration,
    /// An actual subclass may supply a finalizer.
    NonFinalCaller,
    /// Finalizer eligibility could make the ordering observable.
    Finalizer,
    /// Missing, cyclic, effectful or noncanonical chain body.
    ObservableChain,
    /// Object declaration/body was not the exact trusted inert root.
    UntrustedRoot,
}

/// Audited method identities that may be crossed, preserving the original check call frame.
#[derive(Clone, Debug)]
pub struct InertChainProof {
    caller: String,
    chain: Vec<ConstructorIdentity>,
}
impl InertChainProof {
    /// Original, final caller whose complete hierarchy was audited.
    pub fn caller(&self) -> &str {
        &self.caller
    }
    /// Exact original methods traversed, including the audited Object root.
    pub fn chain(&self) -> &[ConstructorIdentity] {
        &self.chain
    }
}

/// Prove an inert initializer chain for an originally final caller.
///
/// This does not permit helper wrapping or changing the check's arguments,
/// order, exception scopes, caller frame, or uninitialized receiver uses.
/// The caller must independently prove these check-site properties. Java17
/// helpers are especially unsafe for stack-inspecting Kotlin null checks.
pub fn prove_inert_chain(
    caller: &str,
    first: &ConstructorIdentity,
    classes: &BTreeMap<String, ClassEvidence>,
    constructors: &BTreeMap<ConstructorIdentity, ConstructorEvidence>,
    root: &ObjectRootEvidence,
) -> Result<InertChainProof, Rejection> {
    let caller_class = classes.get(caller).ok_or(Rejection::MissingDeclaration)?;
    if caller_class.owner != caller || caller_class.flags & 0x10 == 0 {
        return Err(Rejection::NonFinalCaller);
    }
    let object = ConstructorIdentity {
        owner: "Ljava/lang/Object;".into(),
        descriptor: "()V".into(),
    };
    if root.declaration.owner != object.owner
        || root.declaration.superclass.is_some()
        || root.constructor.identity != object
        || root.constructor.has_control_or_handlers
        || !matches!(root.constructor.operations.as_slice(), [Operation::Return])
    {
        return Err(Rejection::UntrustedRoot);
    }
    if root
        .declaration
        .methods
        .contains(&("finalize".into(), "()V".into()))
        && !matches!(
            root.finalizer_operations.as_deref(),
            Some([Operation::Return])
        )
    {
        return Err(Rejection::UntrustedRoot);
    }
    // Audit the complete actual hierarchy, not just owners of retained bodies.
    let mut owner = caller.to_owned();
    let mut hierarchy = BTreeSet::new();
    loop {
        if !hierarchy.insert(owner.clone()) {
            return Err(Rejection::MissingDeclaration);
        }
        let declaration = if owner == object.owner {
            &root.declaration
        } else {
            classes.get(&owner).ok_or(Rejection::MissingDeclaration)?
        };
        if declaration.owner != owner {
            return Err(Rejection::MissingDeclaration);
        }
        if owner != object.owner
            && declaration
                .methods
                .contains(&("finalize".into(), "()V".into()))
        {
            return Err(Rejection::Finalizer);
        }
        if owner == object.owner {
            break;
        }
        owner = declaration
            .superclass
            .clone()
            .ok_or(Rejection::MissingDeclaration)?;
    }
    if caller_class.superclass.as_deref() != Some(first.owner.as_str()) {
        return Err(Rejection::MissingDeclaration);
    }
    let mut current = first.clone();
    let mut visited = BTreeSet::new();
    let mut chain = Vec::new();
    while current != object {
        if chain.len() >= 32 || !visited.insert(current.clone()) {
            return Err(Rejection::ObservableChain);
        }
        let declaration = classes
            .get(&current.owner)
            .ok_or(Rejection::MissingDeclaration)?;
        if !declaration
            .methods
            .contains(&("<init>".into(), current.descriptor.clone()))
        {
            return Err(Rejection::MissingDeclaration);
        }
        let evidence = constructors
            .get(&current)
            .ok_or(Rejection::ObservableChain)?;
        if evidence.identity != current || evidence.has_control_or_handlers {
            return Err(Rejection::ObservableChain);
        }
        let mut target = None;
        let mut returned = false;
        for operation in &evidence.operations {
            match operation {
                Operation::Pure if !returned => {}
                Operation::Chain(next) if target.is_none() && !returned => {
                    if next.owner != current.owner
                        && declaration.superclass.as_deref() != Some(next.owner.as_str())
                    {
                        return Err(Rejection::ObservableChain);
                    }
                    target = Some(next.clone());
                }
                Operation::Return if target.is_some() && !returned => returned = true,
                _ => return Err(Rejection::ObservableChain),
            }
        }
        if !returned {
            return Err(Rejection::ObservableChain);
        }
        chain.push(current);
        current = target.ok_or(Rejection::ObservableChain)?;
    }
    chain.push(object);
    Ok(InertChainProof {
        caller: caller.into(),
        chain,
    })
}
