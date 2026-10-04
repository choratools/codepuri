//! Descriptor-exact inherited public class methods and interface defaults.
//! This is a separate capability: exact declared `method()` lookup is unchanged.
use crate::{
    java_aliases::MethodKey,
    java_external_metadata::{ClassMetadata, Classpath, MetadataChain, ParsedMethod},
};
use std::collections::{BTreeMap, BTreeSet};

/// A genuine selected JVM class declaration, including every declared method.
/// Missing owners and selected runtime-DEX owners provide no JVM proof here.
pub trait InheritedMethodMetadata {
    /// Return the complete selected, parsed class; never synthesize inherited members.
    fn default_class(&self, descriptor: &str) -> Option<&ClassMetadata>;
}
impl InheritedMethodMetadata for Classpath {
    fn default_class(&self, descriptor: &str) -> Option<&ClassMetadata> {
        self.class(descriptor)
    }
}
impl InheritedMethodMetadata for MetadataChain<'_> {
    fn default_class(&self, descriptor: &str) -> Option<&ClassMetadata> {
        self.jvm_class(descriptor)
    }
}
/// A unique descriptor-exact inherited public instance method, with its actual
/// declaring class or interface. This is declaration selection, never a concrete receiver-body
/// devirtualization; the original symbolic receiver and opcode must be retained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InheritedPublicMethod {
    // A cache hit must not copy every attested JVM Code byte or owner string.
    // No mutable accessor exists: each proof retains one immutable snapshot.
    evidence: std::sync::Arc<InheritedEvidence>,
}
#[derive(Debug, PartialEq, Eq)]
struct InheritedEvidence {
    reference: MethodKey,
    declaration: ParsedMethod,
    owners: Vec<String>,
    kind: InheritedMethodKind,
}
/// The genuine declaration selection route; neither route devirtualizes its body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InheritedMethodKind {
    /// Nearest class declaration dominates all interface defaults.
    ClassInstance,
    /// Unique source-compatible most-specific inherited interface default.
    InterfaceDefault,
    /// Public class-instance declaration through a selected runtime DEX owner.
    /// This carries no JVM Code, interface-default or compiler-availability proof.
    RuntimeClassInstance,
}
impl InheritedPublicMethod {
    fn new(
        reference: MethodKey,
        declaration: ParsedMethod,
        owners: Vec<String>,
        kind: InheritedMethodKind,
    ) -> Self {
        Self {
            evidence: std::sync::Arc::new(InheritedEvidence {
                reference,
                declaration,
                owners,
                kind,
            }),
        }
    }
    /// Exact declaration selection route.
    pub fn kind(&self) -> InheritedMethodKind {
        self.evidence.kind
    }

    /// Original symbolic method reference, before inheritance resolution.
    pub fn reference(&self) -> &MethodKey {
        &self.evidence.reference
    }
    /// Genuine selected declaration, including actual owner, flags, throws and Code.
    pub fn declaration(&self) -> &ParsedMethod {
        &self.evidence.declaration
    }
    /// Every complete selected class/interface consulted by this proof.
    pub fn owners(&self) -> &[String] {
        &self.evidence.owners
    }
}
/// This bounded source capability rejects uncertain or non-default cases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InheritedMethodError {
    /// Missing complete genuine JVM owner.
    Missing(String),
    /// Invalid, duplicate, cyclic or bounded-out metadata.
    Invalid(String),
    /// Generic signatures require a separate source-substitution proof.
    Generic(String),
    /// A declaration on the symbolic root is not an inherited method.
    ClassDominance(MethodKey),
    /// Method is absent, inaccessible, static/private or has contradictory body evidence.
    NoMethod(MethodKey),
    /// Conflicting maximally-specific declarations, including abstract/default mixes.
    Ambiguous(MethodKey),
}
fn descriptor(name: &str) -> String {
    format!("L{};", name.replace('.', "/"))
}
fn valid_owner(owner: &str) -> bool {
    owner.starts_with('L')
        && owner.ends_with(';')
        && owner.len() > 2
        && !owner[1..owner.len() - 1].contains(['.', ';', '['])
}
fn same_parameters(method: &MethodKey, reference: &MethodKey) -> bool {
    method.name == reference.name && method.parameters == reference.parameters
}
fn same_descriptor(method: &MethodKey, reference: &MethodKey) -> bool {
    same_parameters(method, reference) && method.return_type == reference.return_type
}
struct Graph<'a, M: InheritedMethodMetadata + ?Sized> {
    metadata: &'a M,
    nodes: BTreeMap<String, &'a ClassMetadata>,
    visiting: BTreeSet<String>,
}
impl<'a, M: InheritedMethodMetadata + ?Sized> Graph<'a, M> {
    fn load(&mut self, owner: &str) -> Result<(), InheritedMethodError> {
        if self.visiting.contains(owner) {
            return Err(InheritedMethodError::Invalid(owner.into()));
        }
        if self.nodes.contains_key(owner) {
            return Ok(());
        }
        if self.nodes.len() + self.visiting.len() >= 256 || !valid_owner(owner) {
            return Err(InheritedMethodError::Invalid(owner.into()));
        }
        let class = self
            .metadata
            .default_class(owner)
            .ok_or_else(|| InheritedMethodError::Missing(owner.into()))?;
        let h = &class.declaration;
        if class.descriptor != owner
            || descriptor(&h.name) != owner
            || h.is_interface != (class.access_flags & 0x200 != 0)
            || class.access_flags & 0x8000 != 0
            || class.methods.iter().any(|m| {
                m.key.owner != owner
                    || m.descriptor
                        != format!("({}){}", m.key.parameters.join(""), m.key.return_type)
            })
            || class
                .methods
                .iter()
                .map(|m| &m.key)
                .collect::<BTreeSet<_>>()
                .len()
                != class.methods.len()
            || h.interfaces.iter().collect::<BTreeSet<_>>().len() != h.interfaces.len()
        {
            return Err(InheritedMethodError::Invalid(owner.into()));
        }
        if class.signature.is_some() {
            return Err(InheritedMethodError::Generic(owner.into()));
        }
        // Interfaces' classfile super_class must be genuine Object. Only Object
        // is permitted to terminate a class hierarchy; a missing parent is not
        // evidence of a root or of absence of a dominant class method.
        if h.is_interface {
            if h.superclass.as_deref() != Some("java.lang.Object") {
                return Err(InheritedMethodError::Invalid(owner.into()));
            }
        } else if (owner == "Ljava/lang/Object;") != h.superclass.is_none() {
            return Err(InheritedMethodError::Invalid(owner.into()));
        }
        self.visiting.insert(owner.into());
        if let Some(parent) = &h.superclass {
            let parent = descriptor(parent);
            self.load(&parent)?;
            if self.nodes[&parent].declaration.is_interface {
                return Err(InheritedMethodError::Invalid(parent));
            }
        }
        for parent in &h.interfaces {
            let parent = descriptor(parent);
            self.load(&parent)?;
            if !self.nodes[&parent].declaration.is_interface {
                return Err(InheritedMethodError::Invalid(parent));
            }
        }
        self.visiting.remove(owner);
        self.nodes.insert(owner.into(), class);
        Ok(())
    }
    fn interface_extends(&self, child: &str, parent: &str) -> bool {
        let mut todo = vec![child.to_owned()];
        let mut seen = BTreeSet::new();
        while let Some(owner) = todo.pop() {
            if !seen.insert(owner.clone()) {
                continue;
            }
            if owner == parent {
                return true;
            }
            todo.extend(
                self.nodes[&owner]
                    .declaration
                    .interfaces
                    .iter()
                    .map(|s| descriptor(s)),
            );
        }
        false
    }
}

/// Prove descriptor-exact public instance lookup through complete JVM metadata.
/// A *class* reference first searches its nearest class ancestor; every class
/// declaration dominates interface defaults, including inaccessible declarations.
/// Inherited native/abstract class declarations attest a callable static contract
/// only, with their genuine absent Code and known throws; no body is fabricated.
/// Ordinary concrete class methods require Code. Interface defaults require Code
/// and a single source-compatible most-specific declaration. An abstract interface
/// redeclaration suppresses its ancestor default; incomparable mixes are denied.
/// Generic/covariant/bridge source uncertainty, static/private methods and
/// constructors stay closed. Retain the original virtual receiver/ref/opcode;
/// this evidence does not attest source argument selection or concrete dispatch.
///
/// # Errors
/// Fails closed for all incomplete, malformed, conflicting or unsupported evidence.
pub fn resolve_inherited_public_instance(
    metadata: &(impl InheritedMethodMetadata + ?Sized),
    reference: &MethodKey,
) -> Result<InheritedPublicMethod, InheritedMethodError> {
    if reference.name.starts_with('<') || !valid_owner(&reference.owner) {
        return Err(InheritedMethodError::NoMethod(reference.clone()));
    }
    let mut graph = Graph {
        metadata,
        nodes: BTreeMap::new(),
        visiting: BTreeSet::new(),
    };
    graph.load(&reference.owner)?;
    let root = graph.nodes[&reference.owner];
    if root.declaration.is_interface || root.access_flags & 1 == 0 {
        return Err(InheritedMethodError::NoMethod(reference.clone()));
    }
    let mut current = Some(reference.owner.clone());
    while let Some(owner) = current {
        let class = graph.nodes[&owner];
        let family = class
            .methods
            .iter()
            .filter(|m| same_parameters(&m.key, reference))
            .collect::<Vec<_>>();
        if !family.is_empty() {
            if owner == reference.owner {
                return Err(InheritedMethodError::ClassDominance(family[0].key.clone()));
            }
            let [method] = family.as_slice() else {
                return Err(InheritedMethodError::Generic(owner));
            };
            if !same_descriptor(&method.key, reference) || method.signature.is_some() {
                return Err(InheritedMethodError::Generic(owner));
            }
            let flags = method.metadata.access_flags;
            let bodyless = flags & (0x100 | 0x400);
            if class.access_flags & 1 == 0
                || flags & 1 == 0
                || flags & !(1 | 0x10 | 0x20 | 0x80 | 0x100 | 0x400 | 0x800) != 0
                || bodyless == 0x500
                || method.metadata.exceptions.is_none()
                || (bodyless != 0 && method.code.is_some())
                || (bodyless == 0 && method.code.as_ref().is_none_or(|c| c.bytes.is_empty()))
            {
                // Never skip a dominant private/static/abstract declaration in
                // search of a more permissive interface method.
                return Err(InheritedMethodError::NoMethod(reference.clone()));
            }
            return Ok(InheritedPublicMethod::new(
                reference.clone(),
                (*method).clone(),
                graph.nodes.keys().cloned().collect(),
                InheritedMethodKind::ClassInstance,
            ));
        }
        current = class.declaration.superclass.as_deref().map(descriptor);
    }
    let mut candidates = Vec::new();
    for class in graph.nodes.values().filter(|c| c.declaration.is_interface) {
        for method in &class.methods {
            if !same_parameters(&method.key, reference) {
                continue;
            }
            if !same_descriptor(&method.key, reference) || method.signature.is_some() {
                return Err(InheritedMethodError::Generic(method.key.owner.clone()));
            }
            // Noninherited interface members never supply or suppress a default.
            if method.metadata.access_flags & (2 | 8) == 0 {
                candidates.push(method);
            }
        }
    }
    let maximal = candidates
        .iter()
        .copied()
        .filter(|method| {
            !candidates.iter().any(|other| {
                method.key.owner != other.key.owner
                    && graph.interface_extends(&other.key.owner, &method.key.owner)
            })
        })
        .collect::<Vec<_>>();
    let [method] = maximal.as_slice() else {
        return Err(if maximal.is_empty() {
            InheritedMethodError::NoMethod(reference.clone())
        } else {
            InheritedMethodError::Ambiguous(reference.clone())
        });
    };
    if graph.nodes[&method.key.owner].access_flags & 1 == 0
        || method.metadata.access_flags & 1 == 0
        // Bridges/synthetic defaults need their own erased-source proof;
        // final/synchronized/native/abstract flags cannot attest this case.
        || method.metadata.access_flags & !(1 | 0x80 | 0x800) != 0
        || method.metadata.exceptions.is_none()
        || method
            .code
            .as_ref()
            .is_none_or(|code| code.bytes.is_empty())
    {
        return Err(InheritedMethodError::NoMethod(reference.clone()));
    }
    Ok(InheritedPublicMethod::new(
        reference.clone(),
        (*method).clone(),
        graph.nodes.into_keys().collect(),
        InheritedMethodKind::InterfaceDefault,
    ))
}

#[cfg(test)]
#[path = "java_external_inherited_methods_tests.rs"]
mod tests;

#[path = "java_runtime_inherited_class.rs"]
mod runtime_class;
pub(crate) use runtime_class::resolve_runtime_class_instance;
