//! Structured source cleanup after typed lowering. Raw statements are opaque:
//! only explicit source nodes are rearranged, never parsed Java source text.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Node {
    Raw {
        lines: Vec<String>,
        terminal: bool,
    },
    ConditionAssignment {
        target: String,
        expression: String,
    },
    Assignment {
        target: String,
        expression: String,
        inline: bool,
    },
    Return {
        expression: String,
        ty: String,
        nesting: usize,
    },
    If {
        condition: String,
        yes: Vec<Node>,
        no: Vec<Node>,
    },
    Loop {
        label: Option<String>,
        condition: String,
        body: Vec<Node>,
    },
    Break(Option<String>),
    Continue(Option<String>),
    Scope(Vec<Node>),
    For {
        label: Option<String>,
        init: String,
        condition: String,
        update: String,
        body: Vec<Node>,
    },
}

fn terminal(nodes: &[Node]) -> bool {
    match nodes.last() {
        Some(Node::Raw { terminal, .. }) => *terminal,
        Some(Node::Return { .. }) => true,
        Some(Node::Break(_) | Node::Continue(_)) => true,
        Some(Node::If { yes, no, .. }) => terminal(yes) && terminal(no),
        Some(Node::Scope(body)) => terminal(body),
        _ => false,
    }
}

pub(super) fn simplify(nodes: Vec<Node>) -> Vec<Node> {
    let mut out = Vec::new();
    for node in nodes {
        match node {
            Node::Assignment {
                target,
                mut expression,
                inline,
            } => {
                if let Some(Node::Assignment {
                    target: source,
                    expression: producer,
                    inline: true,
                }) = out.last()
                    && expression == *source
                {
                    expression = producer.clone();
                    out.pop();
                }
                out.push(Node::Assignment {
                    target,
                    expression,
                    inline,
                });
            }
            Node::If {
                mut condition,
                yes,
                no,
            } => {
                if let Some(Node::ConditionAssignment { target, expression }) = out.last() {
                    let replacement = if condition == *target {
                        Some(expression.clone())
                    } else if condition == format!("!({target})") {
                        Some(format!("!({expression})"))
                    } else {
                        None
                    };
                    if let Some(replacement) = replacement {
                        condition = replacement;
                        out.pop();
                    }
                }
                let mut yes = simplify(yes);
                let mut no = simplify(no);
                if yes.is_empty() && !no.is_empty() {
                    condition = negate(&condition);
                    std::mem::swap(&mut yes, &mut no);
                }
                // No intervening statement is crossed. In particular the RHS
                // remains short-circuited, including effects in its expression.
                loop {
                    if no.is_empty()
                        && let [
                            Node::If {
                                condition: inner,
                                yes: body,
                                no: empty,
                            },
                        ] = yes.as_slice()
                        && empty.is_empty()
                    {
                        condition = format!("({condition}) && ({inner})");
                        yes = body.clone();
                        continue;
                    }
                    if let [
                        Node::If {
                            condition: inner,
                            yes: same,
                            no: tail,
                        },
                    ] = no.as_slice()
                        && *same == yes
                    {
                        condition = format!("({condition}) || ({inner})");
                        no = tail.clone();
                        continue;
                    }
                    break;
                }
                if let (
                    [
                        Node::Return {
                            expression: left,
                            ty,
                            nesting: left_depth,
                        },
                    ],
                    [
                        Node::Return {
                            expression: right,
                            ty: right_ty,
                            nesting: right_depth,
                        },
                    ],
                ) = (yes.as_slice(), no.as_slice())
                    && ty == right_ty
                    && (*left_depth).max(*right_depth) < 16
                    && condition.len() + left.len() + right.len() <= 4096
                {
                    let expression = if ty == "boolean" && left == "true" && right == "false" {
                        condition
                    } else {
                        format!("({condition} ? ({left}) : ({right}))")
                    };
                    out.push(Node::Return {
                        expression,
                        ty: ty.clone(),
                        nesting: 1 + (*left_depth).max(*right_depth),
                    });
                    continue;
                }
                // Only hoisted, primitive assignments can move out of both
                // arms; opaque statements may depend on branch-local scopes.
                let mut tail = Vec::new();
                while yes.last() == no.last() && matches!(yes.last(), Some(Node::Assignment { .. }))
                {
                    if let Some(node) = yes.pop() {
                        tail.push(node);
                    }
                    no.pop();
                }
                if !no.is_empty() && terminal(&yes) {
                    out.push(Node::If {
                        condition,
                        yes,
                        no: Vec::new(),
                    });
                    // Preserve the former else-local declaration scope.
                    if scope_safe(&no) {
                        out.extend(no);
                    } else {
                        out.push(Node::Scope(no));
                    }
                } else {
                    out.push(Node::If { condition, yes, no });
                }
                out.extend(tail.into_iter().rev());
            }
            Node::Loop {
                mut label,
                mut condition,
                body,
            } => {
                let mut body = simplify(body);
                // A direct tail continue is identical to normal loop iteration.
                if matches!(body.last(), Some(Node::Continue(target)) if *target == label) {
                    body.pop();
                }
                // Strengthen a pretest without moving any preceding operation.
                if let Some(Node::If {
                    condition: test,
                    yes,
                    no,
                }) = body.first()
                    && no.is_empty()
                    && matches!(yes.as_slice(), [Node::Break(target)] if *target == label)
                {
                    condition = if condition == "true" {
                        negate(test)
                    } else {
                        format!("({condition}) && ({})", negate(test))
                    };
                    body.remove(0);
                }
                if let Some(name) = &label
                    && !nested_transfer(&body, name, false)
                {
                    unlabel(&mut body, name);
                    label = None;
                }
                let induction = match (out.last(), body.last()) {
                    (
                        Some(Node::Assignment {
                            target: initial,
                            expression: init,
                            ..
                        }),
                        Some(Node::Assignment {
                            target: updated,
                            expression: update,
                            ..
                        }),
                    ) if initial == updated && !has_continue(&body, &label) => Some((
                        format!("{initial} = {init}"),
                        format!("{updated} = {update}"),
                    )),
                    _ => None,
                };
                if let Some((init, update)) = induction {
                    out.pop();
                    body.pop();
                    out.push(Node::For {
                        label,
                        init,
                        condition,
                        update,
                        body,
                    });
                } else {
                    out.push(Node::Loop {
                        label,
                        condition,
                        body,
                    });
                }
            }
            Node::Scope(body) => out.push(Node::Scope(simplify(body))),
            Node::Raw { lines, .. } if lines.is_empty() => {}
            other => out.push(other),
        }
    }
    out
}

fn scope_safe(nodes: &[Node]) -> bool {
    nodes.iter().all(|node| match node {
        Node::Raw { lines, terminal: true } => matches!(lines.as_slice(), [line] if line.starts_with("return ") || line == "return;" || line.starts_with("throw ")),
        Node::Raw { .. } => false,
        // These either declare no locals or provide their own nested scope.
        _ => true,
    })
}

fn negate(condition: &str) -> String {
    // Remove only a complete, balanced outer logical negation. Quoted source
    // atoms stay opaque, so parentheses inside literals never affect this check.
    if !condition.contains(['\'', '"'])
        && let Some(inner) = condition
            .strip_prefix("!(")
            .and_then(|text| text.strip_suffix(')'))
    {
        let mut depth = 0isize;
        let balanced = inner.chars().all(|c| {
            if c == '(' {
                depth += 1;
            }
            if c == ')' {
                depth -= 1;
            }
            depth >= 0
        }) && depth == 0;
        if balanced {
            return format!("({inner})");
        }
    }
    format!("!({condition})")
}

fn has_continue(nodes: &[Node], label: &Option<String>) -> bool {
    nodes.iter().any(|node| match node {
        Node::Continue(target) => target == label,
        Node::If { yes, no, .. } => has_continue(yes, label) || has_continue(no, label),
        Node::Scope(body) => has_continue(body, label),
        // Unlabeled continues inside a nested loop belong to that loop.
        Node::Loop { body, .. } | Node::For { body, .. } if label.is_some() => {
            has_continue(body, label)
        }
        _ => false,
    })
}

fn nested_transfer(nodes: &[Node], label: &str, nested: bool) -> bool {
    nodes.iter().any(|node| match node {
        Node::Break(Some(target)) | Node::Continue(Some(target)) => nested && target == label,
        Node::If { yes, no, .. } => {
            nested_transfer(yes, label, nested) || nested_transfer(no, label, nested)
        }
        Node::Scope(body) => nested_transfer(body, label, nested),
        Node::Loop { body, .. } | Node::For { body, .. } => nested_transfer(body, label, true),
        _ => false,
    })
}

fn unlabel(nodes: &mut [Node], label: &str) {
    for node in nodes {
        match node {
            Node::Break(target) | Node::Continue(target) if target.as_deref() == Some(label) => {
                *target = None
            }
            Node::If { yes, no, .. } => {
                unlabel(yes, label);
                unlabel(no, label);
            }
            Node::Scope(body) => unlabel(body, label),
            _ => {}
        }
    }
}

pub(super) fn render(nodes: &[Node], depth: usize, out: &mut Vec<String>) {
    let indent = "    ".repeat(depth);
    for node in nodes {
        match node {
            Node::Raw { lines, .. } => {
                out.extend(lines.iter().map(|line| format!("{indent}{line}")))
            }
            Node::ConditionAssignment { target, expression } => {
                out.push(format!("{indent}{target} = {expression};"))
            }
            Node::Assignment {
                target, expression, ..
            } => out.push(format!("{indent}{target} = {expression};")),
            Node::Return { expression, .. } => out.push(format!("{indent}return {expression};")),
            Node::If { condition, yes, no } => {
                out.push(format!("{indent}if ({condition}) {{"));
                render(yes, depth + 1, out);
                if !no.is_empty() {
                    out.push(format!("{indent}}} else {{"));
                    render(no, depth + 1, out);
                }
                out.push(format!("{indent}}}"));
            }
            Node::Loop {
                label,
                condition,
                body,
            } => {
                let label = label
                    .as_ref()
                    .map_or(String::new(), |label| format!("{label}: "));
                out.push(format!("{indent}{label}while ({condition}) {{"));
                render(body, depth + 1, out);
                out.push(format!("{indent}}}"));
            }
            Node::For {
                label,
                init,
                condition,
                update,
                body,
            } => {
                let label = label
                    .as_ref()
                    .map_or(String::new(), |label| format!("{label}: "));
                out.push(format!(
                    "{indent}{label}for ({init}; {condition}; {update}) {{"
                ));
                render(body, depth + 1, out);
                out.push(format!("{indent}}}"));
            }
            Node::Break(label) | Node::Continue(label) => {
                let keyword = if matches!(node, Node::Break(_)) {
                    "break"
                } else {
                    "continue"
                };
                let label = label
                    .as_ref()
                    .map_or(String::new(), |label| format!(" {label}"));
                out.push(format!("{indent}{keyword}{label};"));
            }
            Node::Scope(body) => {
                out.push(format!("{indent}{{"));
                render(body, depth + 1, out);
                out.push(format!("{indent}}}"));
            }
        }
    }
}

pub(super) fn condition_assignments(nodes: &[Node], names: &mut std::collections::HashSet<String>) {
    for node in nodes {
        match node {
            Node::ConditionAssignment { target, .. } => {
                names.insert(target.clone());
            }
            Node::If { yes, no, .. } => {
                condition_assignments(yes, names);
                condition_assignments(no, names);
            }
            Node::Loop { body, .. } | Node::For { body, .. } | Node::Scope(body) => {
                condition_assignments(body, names)
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn raw(text: &str, terminal: bool) -> Node {
        Node::Raw {
            lines: vec![text.into()],
            terminal,
        }
    }
    fn source(nodes: &[Node]) -> String {
        let mut lines = Vec::new();
        render(nodes, 0, &mut lines);
        lines.join("\n")
    }
    fn assignment(target: &str, expression: &str) -> Node {
        Node::Assignment {
            target: target.into(),
            expression: expression.into(),
            inline: false,
        }
    }

    #[test]
    fn structural_rewrites_preserve_java_execution_and_exception_traces() {
        let work = vec![raw("trace += 7;", false)];
        let and = vec![
            Node::If {
                condition: "test(a, 1)".into(),
                yes: vec![Node::If {
                    condition: "test(b, 2)".into(),
                    yes: work.clone(),
                    no: vec![],
                }],
                no: vec![],
            },
            raw("return trace;", true),
        ];
        let or = vec![
            Node::If {
                condition: "test(a, 1)".into(),
                yes: work.clone(),
                no: vec![Node::If {
                    condition: "test(b, 2)".into(),
                    yes: work,
                    no: vec![],
                }],
            },
            raw("return trace;", true),
        ];
        // The else-local x must stay scoped when its enclosing else is removed.
        let guard = vec![
            Node::If {
                condition: "a > 0".into(),
                yes: vec![raw("return trace;", true)],
                no: vec![raw("int x = 3; trace += x;", false)],
            },
            raw("int x = 4; return trace + x;", true),
        ];
        let counted = vec![
            assignment("i", "0"),
            Node::Loop {
                label: None,
                condition: "true".into(),
                body: vec![
                    Node::ConditionAssignment {
                        target: "c".into(),
                        expression: "i < a".into(),
                    },
                    Node::If {
                        condition: "!(c)".into(),
                        yes: vec![Node::Break(None)],
                        no: vec![],
                    },
                    raw("trace = trace * 3 + i;", false),
                    assignment("i", "i + 1"),
                ],
            },
            raw("return trace;", true),
        ];
        let continue_loop = vec![
            assignment("i", "0"),
            Node::Loop {
                label: None,
                condition: "i < a".into(),
                body: vec![
                    raw("i++;", false),
                    Node::If {
                        condition: "i % 2 == 0".into(),
                        yes: vec![Node::Continue(None)],
                        no: vec![],
                    },
                    raw("trace += i;", false),
                    assignment("i", "i + 1"),
                ],
            },
            raw("return trace;", true),
        ];
        let cases = [and, or, guard, counted, continue_loop];
        assert!(source(&simplify(cases[0].clone())).contains("&&"));
        assert!(source(&simplify(cases[1].clone())).contains("||"));
        assert!(source(&simplify(cases[3].clone())).contains("for ("));
        assert!(!source(&simplify(cases[4].clone())).contains("for ("));
        let mut methods = String::new();
        for (index, nodes) in cases.iter().enumerate() {
            for (suffix, nodes) in [
                ("Before", nodes.clone()),
                ("After", simplify(nodes.clone())),
            ] {
                methods.push_str(&format!("static int case{index}{suffix}(int a, int b) {{ int i = 0; boolean c = false; {} }}\n", source(&nodes)));
            }
        }
        let java = format!(
            r#"public class ReadableOracle {{
static int trace;
static boolean test(int x, int tag) {{ trace = trace * 10 + tag; if (x == 9) throw new IllegalArgumentException(); return x > 0; }}
{methods}
static String run(String name, int a, int b) throws Exception {{ trace = 0; try {{ Object result = ReadableOracle.class.getDeclaredMethod(name, int.class, int.class).invoke(null, a, b); return result + ":" + trace; }} catch (java.lang.reflect.InvocationTargetException e) {{ return e.getCause().getClass().getName() + ":" + trace; }} }}
public static void main(String[] args) throws Exception {{ for (int k = 0; k < 5; k++) for (int a : new int[]{{-1,0,1,4,9}}) for (int b : new int[]{{-1,0,1,9}}) {{ String before = run("case" + k + "Before", a, b), after = run("case" + k + "After", a, b); if (!before.equals(after)) throw new AssertionError(k + ":" + a + ":" + b + ":" + before + ":" + after); }} }}
}}"#
        );
        let dir =
            std::env::temp_dir().join(format!("codepuri-readable-oracle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ReadableOracle.java"), java).unwrap();
        for (tool, args) in [
            ("javac", vec!["--release", "17", "ReadableOracle.java"]),
            ("java", vec!["-cp", ".", "ReadableOracle"]),
        ] {
            let result = std::process::Command::new(tool)
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn nested_outer_transfers_keep_labels_and_common_tails_are_hoisted_only() {
        let outer = Node::Loop {
            label: Some("outer".into()),
            condition: "true".into(),
            body: vec![Node::Loop {
                label: Some("inner".into()),
                condition: "true".into(),
                body: vec![Node::Break(Some("outer".into()))],
            }],
        };
        assert!(source(&simplify(vec![outer])).contains("break outer;"));
        let tail = assignment("v", "3");
        let nodes = simplify(vec![Node::If {
            condition: "test()".into(),
            yes: vec![tail.clone()],
            no: vec![tail],
        }]);
        assert!(matches!(
            nodes.as_slice(),
            [Node::If { .. }, Node::Assignment { .. }]
        ));
        // The condition still executes even when the two arms became empty.
        assert!(source(&nodes).contains("if (test())"));
    }
}
