//! Property tests for precedence-aware expression rendering.

use decomp_emit::document::{Doc, Renderer};
use decomp_emit::error::EmitError;
use decomp_emit::precedence::{
    Associativity, Expr, ExpressionWriter, OperatorSpec, PrecedenceTable,
};
use proptest::prelude::*;

fn table() -> Result<PrecedenceTable, EmitError> {
    PrecedenceTable::from_specs([
        OperatorSpec {
            key: "+".into(),
            text: "+".into(),
            precedence: 10,
            associativity: Associativity::Left,
        },
        OperatorSpec {
            key: "*".into(),
            text: "*".into(),
            precedence: 20,
            associativity: Associativity::Left,
        },
        OperatorSpec {
            key: "^".into(),
            text: "^".into(),
            precedence: 30,
            associativity: Associativity::Right,
        },
    ])
}

fn expressions() -> impl Strategy<Value = Expr> {
    prop::sample::select(vec!["a", "b", "c", "d"])
        .prop_map(|name| Expr::atom(Doc::text(name)))
        .prop_recursive(5, 96, 2, |inner| {
            (
                prop::sample::select(vec!["+", "*", "^"]),
                inner.clone(),
                inner,
            )
                .prop_map(|(operator, left, right)| Expr::binary(operator, left, right))
        })
}

struct Parser<'a> {
    input: &'a [u8],
    offset: usize,
    table: &'a PrecedenceTable,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str, table: &'a PrecedenceTable) -> Self {
        Self {
            input: input.as_bytes(),
            offset: 0,
            table,
        }
    }

    fn parse(mut self) -> Result<Expr, String> {
        let expression = self.parse_precedence(0)?;
        if self.offset == self.input.len() {
            Ok(expression)
        } else {
            Err(format!("trailing input at {}", self.offset))
        }
    }

    fn parse_precedence(&mut self, minimum: u16) -> Result<Expr, String> {
        let mut left = self.parse_primary()?;
        while let Some(operator) = self.peek_operator() {
            let spec = self
                .table
                .get(operator)
                .ok_or_else(|| format!("missing operator {operator}"))?;
            if spec.precedence < minimum {
                break;
            }
            self.offset += 1;
            let right_minimum = match spec.associativity {
                Associativity::Left | Associativity::None => spec.precedence + 1,
                Associativity::Right => spec.precedence,
            };
            let right = self.parse_precedence(right_minimum)?;
            left = Expr::binary(operator, left, right);
        }
        Ok(left)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.input.get(self.offset).copied() {
            Some(b'a'..=b'd') => {
                let name = char::from(self.input[self.offset]);
                self.offset += 1;
                Ok(Expr::atom(Doc::text(name.to_string())))
            }
            Some(b'(') => {
                self.offset += 1;
                let expression = self.parse_precedence(0)?;
                match self.input.get(self.offset) {
                    Some(b')') => {
                        self.offset += 1;
                        Ok(expression)
                    }
                    _ => Err(format!("missing closing parenthesis at {}", self.offset)),
                }
            }
            _ => Err(format!("expected primary at {}", self.offset)),
        }
    }

    fn peek_operator(&self) -> Option<&'static str> {
        match self.input.get(self.offset) {
            Some(b'+') => Some("+"),
            Some(b'*') => Some("*"),
            Some(b'^') => Some("^"),
            Some(b'a'..=b'd' | b'(' | b')') | None => None,
            Some(_) => None,
        }
    }
}

fn render(expression: &Expr, table: &PrecedenceTable) -> Result<String, EmitError> {
    let document = ExpressionWriter::new(table).write(expression)?;
    Ok(Renderer::new(4).render(&document)?.text)
}

fn matching_parentheses(input: &str) -> Vec<(usize, usize)> {
    let mut stack = Vec::new();
    let mut pairs = Vec::new();
    for (offset, byte) in input.bytes().enumerate() {
        match byte {
            b'(' => stack.push(offset),
            b')' => {
                let Some(open) = stack.pop() else {
                    panic!("rendered parentheses must balance");
                };
                pairs.push((open, offset));
            }
            _ => {}
        }
    }
    pairs
}

fn without_pair(input: &str, pair: (usize, usize)) -> String {
    input
        .char_indices()
        .filter_map(|(offset, character)| {
            (offset != pair.0 && offset != pair.1).then_some(character)
        })
        .collect()
}

proptest! {
    #[test]
    fn rendered_expression_roundtrips_and_each_parenthesis_pair_is_required(expression in expressions()) {
        // Given: a random expression and one shared precedence table.
        let precedence = table().map_err(|error| TestCaseError::fail(error.to_string()))?;

        // When: it is rendered and parsed with that table.
        let rendered = render(&expression, &precedence)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        let reparsed = Parser::new(&rendered, &precedence).parse();

        // Then: structure round-trips and deleting any emitted pair changes it.
        prop_assert_eq!(reparsed.as_ref(), Ok(&expression));
        for pair in matching_parentheses(&rendered) {
            let reduced = without_pair(&rendered, pair);
            prop_assert_ne!(Parser::new(&reduced, &precedence).parse(), Ok(expression.clone()));
        }
    }
}
