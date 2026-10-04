//! Directed, evidence-based Java assignment checks and contextual local typing.
use crate::JavaType;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

/// Lossless widening of a narrow Dalvik category-one scalar to its register
/// integer interpretation. Boolean, wide and floating-point domains are excluded.
pub fn narrow_register_widening(actual: &JavaType, expected: &JavaType) -> bool {
    *expected == JavaType::Int
        && matches!(actual, JavaType::Byte | JavaType::Short | JavaType::Char)
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Proven classpath or input declaration.
pub struct Declaration {
    /// Source-level class name.
    pub name: String,
    /// Direct superclass, when declared.
    pub superclass: Option<String>,
    /// Direct implemented or extended interfaces.
    pub interfaces: Vec<String>,
    /// Whether this declaration is an interface.
    pub is_interface: bool,
}

#[derive(Debug, Default)]
/// Directed graph of attested declarations.
pub struct Hierarchy {
    declarations: BTreeMap<String, Declaration>,
    // Cache only completed top-level queries: recursive cycle observations
    // depend on the active path and cannot safely become independent entries.
    assignments: RwLock<BTreeMap<String, BTreeMap<String, Compatibility>>>,
}

impl Clone for Hierarchy {
    fn clone(&self) -> Self {
        Self {
            declarations: self.declarations.clone(),
            assignments: RwLock::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Reason assignment evidence is unavailable.
pub enum UnsupportedType {
    /// Missing class declaration.
    MissingHierarchy(String),
    /// Cyclic inheritance graph.
    Cycle(String),
    /// Contradictory declarations.
    ConflictingDeclaration(String),
    /// No known value type.
    UnknownValue,
    /// Invalid assignment type.
    InvalidType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Three-way directed assignment result.
pub enum Compatibility {
    /// Assignment is proven valid.
    Assignable,
    /// Assignment is proven invalid.
    Incompatible,
    /// Evidence is insufficient.
    Indeterminate(UnsupportedType),
}

impl Hierarchy {
    /// Translate every declaration and edge into the emitter's naming domain.
    pub fn map_names(&self, map: impl Fn(&str) -> String) -> Result<Self, UnsupportedType> {
        let mut result = Self::default();
        for declaration in self.declarations.values() {
            result.insert(Declaration {
                name: map(&declaration.name),
                superclass: declaration.superclass.as_deref().map(&map),
                interfaces: declaration.interfaces.iter().map(|n| map(n)).collect(),
                is_interface: declaration.is_interface,
            })?;
        }
        Ok(result)
    }
    /// Insert a declaration, rejecting conflicts.
    pub fn insert(&mut self, declaration: Declaration) -> Result<(), UnsupportedType> {
        if let Some(old) = self.declarations.get(&declaration.name) {
            return if old == &declaration {
                Ok(())
            } else {
                Err(UnsupportedType::ConflictingDeclaration(declaration.name))
            };
        }
        self.assignments
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.declarations
            .insert(declaration.name.clone(), declaration);
        Ok(())
    }

    fn class_assignment(&self, actual: &str, expected: &str) -> Compatibility {
        if expected == "java.lang.Object" || actual == "java.lang.Object" {
            return self.ancestors(actual, expected, &mut BTreeSet::new());
        }
        if let Some(result) = self
            .assignments
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(actual)
            .and_then(|results| results.get(expected))
        {
            return result.clone();
        }
        // No lock is held while walking the graph. Workers may duplicate a
        // first query, but all observe this same immutable declaration graph.
        let result = self.ancestors(actual, expected, &mut BTreeSet::new());
        self.assignments
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .entry(actual.to_owned())
            .or_default()
            .entry(expected.to_owned())
            .or_insert_with(|| result.clone());
        result
    }

    fn ancestors(
        &self,
        name: &str,
        expected: &str,
        active: &mut BTreeSet<String>,
    ) -> Compatibility {
        if name == expected || expected == "java.lang.Object" {
            return Compatibility::Assignable;
        }
        if name == "java.lang.Object" {
            return Compatibility::Incompatible;
        }
        if !active.insert(name.into()) {
            return Compatibility::Indeterminate(UnsupportedType::Cycle(name.into()));
        }
        let result = match self.declarations.get(name) {
            None => Compatibility::Indeterminate(UnsupportedType::MissingHierarchy(name.into())),
            Some(d) => {
                let mut unresolved = None;
                let mut found = false;
                for parent in d.superclass.iter().chain(d.interfaces.iter()) {
                    match self.ancestors(parent, expected, active) {
                        Compatibility::Assignable => {
                            found = true;
                            break;
                        }
                        Compatibility::Indeterminate(e) => unresolved = Some(e),
                        Compatibility::Incompatible => {}
                    }
                }
                if found {
                    Compatibility::Assignable
                } else if let Some(e) = unresolved {
                    Compatibility::Indeterminate(e)
                } else {
                    Compatibility::Incompatible
                }
            }
        };
        active.remove(name);
        result
    }

    /// Check identity, reference upcasts, and array covariance.
    pub fn assignable(&self, actual: &JavaType, expected: &JavaType) -> Compatibility {
        if !valid(actual) || !valid(expected) {
            return Compatibility::Indeterminate(UnsupportedType::InvalidType);
        }
        if actual == expected {
            return Compatibility::Assignable;
        }
        if narrow_register_widening(actual, expected) {
            return Compatibility::Assignable;
        }
        match (actual, expected) {
            (JavaType::Class(a), JavaType::Class(b)) => self.class_assignment(a, b),
            (JavaType::Array(_), JavaType::Class(b))
                if matches!(
                    b.as_str(),
                    "java.lang.Object" | "java.lang.Cloneable" | "java.io.Serializable"
                ) =>
            {
                Compatibility::Assignable
            }
            (JavaType::Array(a), JavaType::Array(b)) if reference(a) && reference(b) => {
                self.assignable(a, b)
            }
            _ => Compatibility::Incompatible,
        }
    }
}

#[cfg(test)]
#[path = "hierarchy_cache_tests.rs"]
mod hierarchy_cache_tests;

fn reference(t: &JavaType) -> bool {
    matches!(t, JavaType::Class(_) | JavaType::Array(_))
}
fn valid(t: &JavaType) -> bool {
    match t {
        JavaType::Void => false,
        JavaType::Array(t) => valid(t),
        JavaType::Class(n) => !n.is_empty(),
        _ => true,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Evidence available for contextual typing.
pub enum ValueType {
    /// Exact nominal type.
    Known(JavaType),
    /// Missing type evidence.
    Unknown,
    /// Proven null value.
    Null,
    /// Proven untyped zero constant.
    RawZero,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Safe expression form for an expected type.
pub enum Coercion {
    /// Retain the existing expression.
    Identity,
    /// Emit null in a reference context.
    NullLiteral,
    /// Emit the typed zero spelling.
    ZeroLiteral(&'static str),
}

/// No downcasts or fabricated concrete types. Callers retain the expression for Identity.
pub fn expected_type(
    h: &Hierarchy,
    value: &ValueType,
    expected: &JavaType,
) -> Result<Coercion, Compatibility> {
    if !valid(expected) {
        return Err(Compatibility::Indeterminate(UnsupportedType::InvalidType));
    }
    match value {
        ValueType::Known(actual) => match h.assignable(actual, expected) {
            Compatibility::Assignable => Ok(Coercion::Identity),
            other => Err(other),
        },
        ValueType::Unknown => Err(Compatibility::Indeterminate(UnsupportedType::UnknownValue)),
        ValueType::Null if reference(expected) => Ok(Coercion::NullLiteral),
        ValueType::RawZero if reference(expected) => Ok(Coercion::NullLiteral),
        ValueType::RawZero => Ok(Coercion::ZeroLiteral(match expected {
            JavaType::Boolean => "false",
            JavaType::Long => "0L",
            JavaType::Float => "0.0f",
            JavaType::Double => "0.0d",
            JavaType::Char => "'\\0'",
            _ => "0",
        })),
        _ => Err(Compatibility::Incompatible),
    }
}

/// The declaration type must come from metadata or a caller-proven local plan.
/// Every definition and use is checked; Unknown never acquires a type by guessing.
pub fn check_local_plan(
    h: &Hierarchy,
    declared: &JavaType,
    definitions: &[ValueType],
    uses: &[JavaType],
) -> Result<Vec<Coercion>, Compatibility> {
    let coercions = definitions
        .iter()
        .map(|v| expected_type(h, v, declared))
        .collect::<Result<Vec<_>, _>>()?;
    for expected in uses {
        let result = h.assignable(declared, expected);
        if result != Compatibility::Assignable {
            return Err(result);
        }
    }
    Ok(coercions)
}
