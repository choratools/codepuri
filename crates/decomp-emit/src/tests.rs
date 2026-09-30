use crate::document::{Doc, Renderer};
use crate::error::EmitError;
use crate::naming::NameAllocator;
use crate::precedence::{Associativity, Expr, ExpressionWriter, OperatorSpec, PrecedenceTable};
use crate::sourcemap::{Position, ProvenanceId};

fn expression_table() -> Result<PrecedenceTable, EmitError> {
    PrecedenceTable::from_specs([
        OperatorSpec {
            key: "add".into(),
            text: " + ".into(),
            precedence: 10,
            associativity: Associativity::Left,
        },
        OperatorSpec {
            key: "mul".into(),
            text: " * ".into(),
            precedence: 20,
            associativity: Associativity::Left,
        },
        OperatorSpec {
            key: "pow".into(),
            text: " ^ ".into(),
            precedence: 30,
            associativity: Associativity::Right,
        },
    ])
}

fn atom(text: &str) -> Expr {
    Expr::atom(Doc::text(text))
}

fn binary(operator: &str, left: Expr, right: Expr) -> Expr {
    Expr::binary(operator, left, right)
}

fn emit(expression: &Expr) -> Result<String, EmitError> {
    let table = expression_table()?;
    let document = ExpressionWriter::new(&table).write(expression)?;
    Ok(Renderer::new(4).render(&document)?.text)
}

#[test]
fn minimizes_parentheses_when_child_binds_more_tightly() -> Result<(), EmitError> {
    // Given: addition containing multiplication.
    let expression = binary("add", atom("a"), binary("mul", atom("b"), atom("c")));

    // When: the expression is emitted.
    let rendered = emit(&expression)?;

    // Then: precedence makes parentheses unnecessary.
    assert_eq!(rendered, "a + b * c");
    Ok(())
}

#[test]
fn adds_parentheses_when_child_binds_less_tightly() -> Result<(), EmitError> {
    // Given: multiplication containing addition.
    let expression = binary("mul", binary("add", atom("a"), atom("b")), atom("c"));

    // When: the expression is emitted.
    let rendered = emit(&expression)?;

    // Then: the lower-precedence child is parenthesized.
    assert_eq!(rendered, "(a + b) * c");
    Ok(())
}

#[test]
fn preserves_right_associative_chains_with_only_required_parentheses() -> Result<(), EmitError> {
    // Given: both possible shapes of a right-associative chain.
    let natural = binary("pow", atom("a"), binary("pow", atom("b"), atom("c")));
    let forced = binary("pow", binary("pow", atom("a"), atom("b")), atom("c"));

    // When: both shapes are emitted.
    let natural_rendered = emit(&natural)?;
    let forced_rendered = emit(&forced)?;

    // Then: only the left-nested shape needs parentheses.
    assert_eq!(natural_rendered, "a ^ b ^ c");
    assert_eq!(forced_rendered, "(a ^ b) ^ c");
    Ok(())
}

#[test]
fn indents_each_nested_line() -> Result<(), EmitError> {
    // Given: nested indentation in a recursive document.
    let document = Doc::concat([
        Doc::text("root"),
        Doc::line(),
        Doc::indent(Doc::concat([
            Doc::text("child"),
            Doc::line(),
            Doc::indent(Doc::text("leaf")),
        ])),
    ]);

    // When: the document is rendered with two-column indentation.
    let rendered = Renderer::new(2).render(&document)?;

    // Then: indentation reflects the nesting depth.
    assert_eq!(rendered.text, "root\n  child\n    leaf");
    Ok(())
}

#[test]
fn allocates_sanitized_unique_non_keyword_names() {
    // Given: one reserved spelling and colliding invalid inputs.
    let mut allocator = NameAllocator::new(["class"]);

    // When: names are allocated in a fixed order.
    let names = [
        allocator.allocate("class"),
        allocator.allocate("a-b"),
        allocator.allocate("a b"),
        allocator.allocate("9lives"),
    ];

    // Then: reserved and colliding names receive deterministic suffixes.
    assert_eq!(names, ["class_2", "a_b", "a_b_2", "_9lives"]);
}

#[test]
fn records_provenance_at_rendered_positions_without_claiming_indentation() -> Result<(), EmitError>
{
    // Given: annotated text before and after an indented line break.
    let document = Doc::concat([
        Doc::text("alpha").with_provenance(ProvenanceId::new(7)),
        Doc::line(),
        Doc::indent(Doc::text("beta").with_provenance(ProvenanceId::new(9))),
    ]);

    // When: the document is rendered.
    let rendered = Renderer::new(2).render(&document)?;

    // Then: source positions cover text but not generated whitespace.
    assert_eq!(rendered.text, "alpha\n  beta");
    assert_eq!(
        rendered.source_map.lookup(Position::new(0, 4)),
        Some(ProvenanceId::new(7))
    );
    assert_eq!(rendered.source_map.lookup(Position::new(1, 0)), None);
    assert_eq!(
        rendered.source_map.lookup(Position::new(1, 2)),
        Some(ProvenanceId::new(9))
    );
    assert_eq!(rendered.source_map.lookup(Position::new(1, 6)), None);
    Ok(())
}
