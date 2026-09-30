//! Data-driven binary-expression precedence and minimal parentheses.

use std::collections::BTreeMap;

use crate::document::Doc;
use crate::error::EmitError;

/// How operators at the same precedence group.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Associativity {
    /// Equal-precedence chains group from the left.
    Left,
    /// Equal-precedence chains group from the right.
    Right,
    /// Equal-precedence children require explicit parentheses.
    None,
}

/// Backend-supplied data for one binary operator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorSpec {
    /// Stable key referenced by expression nodes.
    pub key: String,
    /// Exact text emitted between operands, including desired spacing.
    pub text: String,
    /// Binding strength, where larger values bind more tightly.
    pub precedence: u16,
    /// Grouping behavior for equal-precedence chains.
    pub associativity: Associativity,
}

/// A validated, backend-owned binary-operator table.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PrecedenceTable {
    operators: BTreeMap<String, OperatorSpec>,
}

impl PrecedenceTable {
    /// Loads and validates operator profile data.
    ///
    /// # Errors
    /// Returns an error for duplicate keys or inconsistent associativity at a
    /// shared precedence level.
    pub fn from_specs(specs: impl IntoIterator<Item = OperatorSpec>) -> Result<Self, EmitError> {
        let mut table = Self::default();
        for spec in specs {
            table.insert(spec)?;
        }
        Ok(table)
    }

    /// Adds one operator specification.
    ///
    /// # Errors
    /// Returns an error for a duplicate key or inconsistent associativity at
    /// the same precedence level.
    pub fn insert(&mut self, spec: OperatorSpec) -> Result<(), EmitError> {
        if self.operators.contains_key(&spec.key) {
            return Err(EmitError::DuplicateOperator(spec.key));
        }
        if self.operators.values().any(|existing| {
            existing.precedence == spec.precedence && existing.associativity != spec.associativity
        }) {
            return Err(EmitError::InconsistentAssociativity {
                precedence: spec.precedence,
            });
        }
        self.operators.insert(spec.key.clone(), spec);
        Ok(())
    }

    /// Looks up an operator by stable key.
    pub fn get(&self, key: &str) -> Option<&OperatorSpec> {
        self.operators.get(key)
    }
}

/// A language-neutral binary expression tree whose leaves are documents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Expr {
    /// An indivisible expression document.
    Atom(Doc),
    /// A binary operation referenced through a precedence-table key.
    Binary {
        /// The backend-supplied operator key.
        operator: String,
        /// The left operand.
        left: Box<Self>,
        /// The right operand.
        right: Box<Self>,
    },
}

impl Expr {
    /// Creates an atomic expression.
    pub fn atom(document: Doc) -> Self {
        Self::Atom(document)
    }

    /// Creates a binary expression.
    pub fn binary(operator: impl Into<String>, left: Self, right: Self) -> Self {
        Self::Binary {
            operator: operator.into(),
            left: Box::new(left),
            right: Box::new(right),
        }
    }
}

/// Writes expressions using a caller-owned precedence table.
#[derive(Clone, Copy, Debug)]
pub struct ExpressionWriter<'a> {
    table: &'a PrecedenceTable,
}

impl<'a> ExpressionWriter<'a> {
    /// Creates a writer over validated operator profile data.
    pub const fn new(table: &'a PrecedenceTable) -> Self {
        Self { table }
    }

    /// Produces a document with only structurally required parentheses.
    ///
    /// # Errors
    /// Returns an error when an expression references an unknown operator key.
    pub fn write(self, expression: &Expr) -> Result<Doc, EmitError> {
        self.write_expression(expression)
    }

    fn write_expression(self, expression: &Expr) -> Result<Doc, EmitError> {
        match expression {
            Expr::Atom(document) => Ok(document.clone()),
            Expr::Binary {
                operator,
                left,
                right,
            } => {
                let parent = self.operator(operator)?;
                let left_doc = self.write_child(left, parent, ChildSide::Left)?;
                let right_doc = self.write_child(right, parent, ChildSide::Right)?;
                Ok(Doc::group(Doc::concat([
                    left_doc,
                    Doc::text(&parent.text),
                    right_doc,
                ])))
            }
        }
    }

    fn write_child(
        self,
        child: &Expr,
        parent: &OperatorSpec,
        side: ChildSide,
    ) -> Result<Doc, EmitError> {
        let needs_parentheses = self.needs_parentheses(child, parent, side)?;
        let document = self.write_expression(child)?;
        if needs_parentheses {
            Ok(Doc::concat([Doc::text("("), document, Doc::text(")")]))
        } else {
            Ok(document)
        }
    }

    fn needs_parentheses(
        self,
        child: &Expr,
        parent: &OperatorSpec,
        side: ChildSide,
    ) -> Result<bool, EmitError> {
        let Expr::Binary { operator, .. } = child else {
            return Ok(false);
        };
        let child_spec = self.operator(operator)?;
        Ok(match child_spec.precedence.cmp(&parent.precedence) {
            std::cmp::Ordering::Less => true,
            std::cmp::Ordering::Greater => false,
            std::cmp::Ordering::Equal => match (parent.associativity, side) {
                (Associativity::Left, ChildSide::Left)
                | (Associativity::Right, ChildSide::Right) => false,
                (Associativity::Left, ChildSide::Right)
                | (Associativity::Right, ChildSide::Left)
                | (Associativity::None, ChildSide::Left | ChildSide::Right) => true,
            },
        })
    }

    fn operator(self, key: &str) -> Result<&'a OperatorSpec, EmitError> {
        self.table
            .get(key)
            .ok_or_else(|| EmitError::UnknownOperator(key.to_owned()))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChildSide {
    Left,
    Right,
}
