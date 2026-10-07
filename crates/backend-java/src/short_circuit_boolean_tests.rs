#![allow(clippy::unwrap_used)]

use super::*;
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    method::{InsnArena, ValueArena},
    ops::Constant,
    value::{InsnId, Value},
};
use std::{fs, process::Command};

struct ShortCircuitResolver;

impl JavaResolver for ShortCircuitResolver {
    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        let (name, return_type) = match reference.index() {
            0 => ("left", JavaType::Boolean),
            1 => ("right", JavaType::Boolean),
            2 => ("audit", JavaType::Void),
            3 | 4 => ("j", JavaType::Boolean),
            _ => return None,
        };
        Some(JavaMethod {
            owner: if reference.index() >= 3 {
                "Operand".into()
            } else {
                "Fixture".into()
            },
            name: name.into(),
            is_static: reference.index() < 3,
            return_type,
            parameters: vec![],
            throws: vec![],
        })
    }

    fn field(&self, reference: FieldRef) -> Option<JavaField> {
        let name = match reference.index() {
            0 => "a",
            1 => "b",
            _ => return None,
        };
        Some(JavaField {
            owner: "Fixture".into(),
            name: name.into(),
            ty: JavaType::Class("Operand".into()),
            is_static: false,
        })
    }

    fn class(&self, reference: ClassRef) -> Option<String> {
        match reference.index() {
            0 => Some("Fixture".into()),
            1 => Some("Operand".into()),
            _ => None,
        }
    }

    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}

struct Graph {
    cfg: Cfg<InsnId>,
    instructions: InsnArena,
    values: ValueArena,
}

impl Graph {
    fn new() -> Self {
        let mut instructions = InsnArena::with_key();
        let entry = instructions.insert(ManagedInsn::Return { value: None });
        Self {
            cfg: Cfg::new(entry),
            instructions,
            values: ValueArena::with_key(),
        }
    }

    fn block(&mut self) -> decomp_core::ids::BlockId {
        let instruction = self
            .instructions
            .insert(ManagedInsn::Return { value: None });
        self.cfg.add_block(instruction)
    }

    fn parameter(&mut self, ty: ManagedType) -> (ValueId, decomp_core::ids::BlockId) {
        let mut result = None;
        let instructions = &mut self.instructions;
        let values = &mut self.values;
        let instruction = instructions.insert_with_key(|instruction| {
            let value = values.insert(Value::new(instruction, ty));
            result = Some(value);
            ManagedInsn::Opaque {
                family_tag: 0x8001,
                reads: vec![],
                writes: vec![value],
                effects: managed_ir::effects::EffectFlags::NONE,
            }
        });
        let block = self.cfg.add_block(instruction);
        (result.unwrap(), block)
    }

    fn define(
        &mut self,
        block: decomp_core::ids::BlockId,
        ty: ManagedType,
        make: impl FnOnce(ValueId) -> ManagedInsn,
    ) -> ValueId {
        let instruction = *self.cfg.block(block).unwrap().payload();
        let value = self.values.insert(Value::new(instruction, ty));
        self.instructions[instruction] = make(value);
        value
    }

    fn set(&mut self, block: decomp_core::ids::BlockId, instruction: ManagedInsn) {
        let id = *self.cfg.block(block).unwrap().payload();
        self.instructions[id] = instruction;
    }

    fn link(
        &mut self,
        source: decomp_core::ids::BlockId,
        target: decomp_core::ids::BlockId,
        kind: EdgeKind,
    ) -> decomp_core::ids::EdgeId {
        self.cfg.add_edge(source, target, kind).unwrap()
    }

    fn finish(self) -> MethodIr<SsaForm> {
        MethodIr::<SsaForm>::from_parts(self.cfg, self.instructions, self.values)
    }
}

fn boolean_conjunction(with_terminal_effect: bool) -> MethodIr<SsaForm> {
    let mut graph = Graph::new();
    let entry = graph.cfg.entry();
    let left_call = graph.block();
    let left_compare = graph.block();
    let left_branch = graph.block();
    let right_call = graph.block();
    let right_compare = graph.block();
    let right_branch = graph.block();
    let true_constant = graph.block();
    let true_return = graph.block();
    let false_return = graph.block();
    let false_effect = with_terminal_effect.then(|| graph.block());

    let false_value = graph.define(entry, ManagedType::Boolean, |result| ManagedInsn::Const {
        result,
        value: Constant::Boolean(false),
    });
    let left_value = graph.define(left_call, ManagedType::Boolean, |result| {
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(0),
            args: vec![],
        }
    });
    let left_condition = graph.define(left_compare, ManagedType::Boolean, |result| {
        ManagedInsn::Compare {
            op: CompareOp::Equal,
            result,
            lhs: left_value,
            rhs: false_value,
        }
    });
    let right_value = graph.define(right_call, ManagedType::Boolean, |result| {
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(1),
            args: vec![],
        }
    });
    let right_condition = graph.define(right_compare, ManagedType::Boolean, |result| {
        ManagedInsn::Compare {
            op: CompareOp::Equal,
            result,
            lhs: right_value,
            rhs: false_value,
        }
    });
    let true_value = graph.define(true_constant, ManagedType::Boolean, |result| {
        ManagedInsn::Const {
            result,
            value: Constant::Boolean(true),
        }
    });
    graph.set(
        true_return,
        ManagedInsn::Return {
            value: Some(true_value),
        },
    );
    graph.set(
        false_return,
        ManagedInsn::Return {
            value: Some(false_value),
        },
    );

    graph.link(entry, left_call, EdgeKind::Unconditional);
    graph.link(left_call, left_compare, EdgeKind::Unconditional);
    graph.link(left_compare, left_branch, EdgeKind::Unconditional);
    let early_false = if let Some(block) = false_effect {
        graph.set(
            block,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(2),
                args: vec![],
            },
        );
        graph.link(block, false_return, EdgeKind::Unconditional);
        block
    } else {
        false_return
    };
    let left_false = graph.link(left_branch, early_false, EdgeKind::BranchTrue);
    let left_true = graph.link(left_branch, right_call, EdgeKind::BranchFalse);
    graph.set(
        left_branch,
        ManagedInsn::Branch {
            condition: Some(left_condition),
            then_edge: left_false,
            else_edge: Some(left_true),
        },
    );
    graph.link(right_call, right_compare, EdgeKind::Unconditional);
    graph.link(right_compare, right_branch, EdgeKind::Unconditional);
    let right_false_value = graph.link(right_branch, early_false, EdgeKind::BranchTrue);
    let right_true_value = graph.link(right_branch, true_constant, EdgeKind::BranchFalse);
    graph.set(
        right_branch,
        ManagedInsn::Branch {
            condition: Some(right_condition),
            then_edge: right_false_value,
            else_edge: Some(right_true_value),
        },
    );
    graph.link(true_constant, true_return, EdgeKind::Unconditional);
    graph.finish()
}

fn field_boolean_conjunction(with_prefix_effect: bool) -> (MethodIr<SsaForm>, ValueId) {
    let mut graph = Graph::new();
    let (receiver, receiver_binding) = graph.parameter(ManagedType::Reference(ClassRef::new(0)));
    let entry = graph.cfg.entry();
    let prefix_effect = with_prefix_effect.then(|| graph.block());
    let left_field = graph.block();
    let left_call = graph.block();
    let left_compare = graph.block();
    let left_branch = graph.block();
    let right_field = graph.block();
    let right_call = graph.block();
    let right_compare = graph.block();
    let right_branch = graph.block();
    let true_constant = graph.block();
    let true_return = graph.block();
    let false_return = graph.block();

    let false_value = graph.define(entry, ManagedType::Boolean, |result| ManagedInsn::Const {
        result,
        value: Constant::Boolean(false),
    });
    if let Some(block) = prefix_effect {
        graph.set(
            block,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(2),
                args: vec![],
            },
        );
    }
    let left_field_value = graph.define(
        left_field,
        ManagedType::Reference(ClassRef::new(1)),
        |result| ManagedInsn::FieldGet {
            result,
            object: Some(receiver),
            field: FieldRef::new(0),
        },
    );
    let left_value = graph.define(left_call, ManagedType::Boolean, |result| {
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: Some(result),
            method: MethodRef::new(3),
            args: vec![left_field_value],
        }
    });
    let left_condition = graph.define(left_compare, ManagedType::Boolean, |result| {
        ManagedInsn::Compare {
            op: CompareOp::Equal,
            result,
            lhs: left_value,
            rhs: false_value,
        }
    });
    let right_field_value = graph.define(
        right_field,
        ManagedType::Reference(ClassRef::new(1)),
        |result| ManagedInsn::FieldGet {
            result,
            object: Some(receiver),
            field: FieldRef::new(1),
        },
    );
    let right_value = graph.define(right_call, ManagedType::Boolean, |result| {
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: Some(result),
            method: MethodRef::new(4),
            args: vec![right_field_value],
        }
    });
    let right_condition = graph.define(right_compare, ManagedType::Boolean, |result| {
        ManagedInsn::Compare {
            op: CompareOp::Equal,
            result,
            lhs: right_value,
            rhs: false_value,
        }
    });
    let true_value = graph.define(true_constant, ManagedType::Boolean, |result| {
        ManagedInsn::Const {
            result,
            value: Constant::Boolean(true),
        }
    });
    graph.set(
        true_return,
        ManagedInsn::Return {
            value: Some(true_value),
        },
    );
    graph.set(
        false_return,
        ManagedInsn::Return {
            value: Some(false_value),
        },
    );

    graph.link(entry, receiver_binding, EdgeKind::Unconditional);
    graph.link(
        receiver_binding,
        prefix_effect.unwrap_or(left_field),
        EdgeKind::Unconditional,
    );
    if let Some(block) = prefix_effect {
        graph.link(block, left_field, EdgeKind::Unconditional);
    }
    graph.link(left_field, left_call, EdgeKind::Unconditional);
    graph.link(left_call, left_compare, EdgeKind::Unconditional);
    graph.link(left_compare, left_branch, EdgeKind::Unconditional);
    let left_false = graph.link(left_branch, false_return, EdgeKind::BranchTrue);
    let left_true = graph.link(left_branch, right_field, EdgeKind::BranchFalse);
    graph.set(
        left_branch,
        ManagedInsn::Branch {
            condition: Some(left_condition),
            then_edge: left_false,
            else_edge: Some(left_true),
        },
    );
    graph.link(right_field, right_call, EdgeKind::Unconditional);
    graph.link(right_call, right_compare, EdgeKind::Unconditional);
    graph.link(right_compare, right_branch, EdgeKind::Unconditional);
    let right_false = graph.link(right_branch, false_return, EdgeKind::BranchTrue);
    let right_true = graph.link(right_branch, true_constant, EdgeKind::BranchFalse);
    graph.set(
        right_branch,
        ManagedInsn::Branch {
            condition: Some(right_condition),
            then_edge: right_false,
            else_edge: Some(right_true),
        },
    );
    graph.link(true_constant, true_return, EdgeKind::Unconditional);
    (graph.finish(), receiver)
}

fn conjunction_method() -> JavaMethod {
    JavaMethod {
        owner: "Fixture".into(),
        name: "conjunction".into(),
        is_static: true,
        return_type: JavaType::Boolean,
        parameters: vec![],
        throws: vec![],
    }
}

fn instance_conjunction_method() -> JavaMethod {
    JavaMethod {
        owner: "Fixture".into(),
        name: "conjunction".into(),
        is_static: false,
        return_type: JavaType::Boolean,
        parameters: vec![],
        throws: vec![],
    }
}

fn emitted(ir: &MethodIr<SsaForm>) -> String {
    emitted_with_bindings(ir, &conjunction_method(), &ParameterBindings::default())
}

fn emitted_with_bindings(
    ir: &MethodIr<SsaForm>,
    method: &JavaMethod,
    bindings: &ParameterBindings,
) -> String {
    match emit_method(ir, method, bindings, &ShortCircuitResolver) {
        SourceResult::Complete { source, .. } => source,
        other => panic!("expected complete method, got {other:?}"),
    }
}

#[test]
fn short_circuit_boolean_return_preserves_effect_order_and_skips_right_operand() {
    let source = emitted(&boolean_conjunction(false));
    assert!(source.contains("if (!"), "{source}");
    assert!(source.contains(") return false;"), "{source}");
    assert!(source.contains("return v"), "{source}");
    assert!(!source.contains("forwardBlock_"), "{source}");
    assert!(!source.contains("== false"), "{source}");

    if Command::new("javac").arg("-version").output().is_err()
        || Command::new("java").arg("-version").output().is_err()
    {
        eprintln!("SKIP short-circuit runtime assertions: javac/java unavailable");
        return;
    }
    let root =
        std::env::temp_dir().join(format!("backend-java-short-circuit-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("Fixture.java");
    let fixture = format!(
        "public class Fixture {{\nstatic boolean leftResult, rightResult;\nstatic StringBuilder events;\nstatic boolean left() {{ events.append('L'); return leftResult; }}\nstatic boolean right() {{ events.append('R'); return rightResult; }}\nstatic void check(boolean l, boolean r, boolean expected, String order) {{ leftResult=l; rightResult=r; events=new StringBuilder(); boolean actual=conjunction(); if (actual != expected || !events.toString().equals(order)) throw new AssertionError(l+\"/\"+r+\": \"+actual+\" \"+events); }}\n{source}\npublic static void main(String[] args) {{ check(false,false,false,\"L\"); check(false,true,false,\"L\"); check(true,false,false,\"LR\"); check(true,true,true,\"LR\"); }}\n}}"
    );
    fs::write(&path, fixture).unwrap();
    let compile = Command::new("javac")
        .args(["--release", "17", "-d"])
        .arg(&root)
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new("java")
        .args(["-cp"])
        .arg(&root)
        .arg("Fixture")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn short_circuit_field_calls_inline_exactly_and_preserve_runtime_order() {
    let (ir, receiver) = field_boolean_conjunction(false);
    let source = emitted_with_bindings(
        &ir,
        &instance_conjunction_method(),
        &ParameterBindings {
            receiver: Some(receiver),
            ..ParameterBindings::default()
        },
    );
    assert!(
        source.contains("if (!this.a.j()) return false;\n    return this.b.j();"),
        "{source}"
    );
    assert!(!source.contains("v0"), "{source}");

    if Command::new("javac").arg("-version").output().is_err()
        || Command::new("java").arg("-version").output().is_err()
    {
        eprintln!("SKIP short-circuit field-call runtime assertions: javac/java unavailable");
        return;
    }
    let root = std::env::temp_dir().join(format!(
        "backend-java-short-circuit-fields-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("Fixture.java");
    let fixture = format!(
        "public class Fixture {{\nstatic boolean leftResult, rightResult;\nstatic StringBuilder events;\nstatic final class Operand {{ final char id; Operand(char id) {{ this.id=id; }} boolean j() {{ events.append(id); return id=='L' ? leftResult : rightResult; }} }}\nfinal Operand a=new Operand('L'), b=new Operand('R');\n{source}\nstatic void check(boolean l, boolean r, boolean expected, String order) {{ leftResult=l; rightResult=r; events=new StringBuilder(); boolean actual=new Fixture().conjunction(); if (actual != expected || !events.toString().equals(order)) throw new AssertionError(l+\"/\"+r+\": \"+actual+\" \"+events); }}\npublic static void main(String[] args) {{ check(false,false,false,\"L\"); check(false,true,false,\"L\"); check(true,false,false,\"LR\"); check(true,true,true,\"LR\"); }}\n}}"
    );
    fs::write(&path, fixture).unwrap();
    let compile = Command::new("javac")
        .args(["--release", "17", "-d"])
        .arg(&root)
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new("java")
        .args(["-cp"])
        .arg(&root)
        .arg("Fixture")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn short_circuit_field_call_inliner_falls_back_when_segment_has_extra_call() {
    let (ir, receiver) = field_boolean_conjunction(true);
    let source = emitted_with_bindings(
        &ir,
        &instance_conjunction_method(),
        &ParameterBindings {
            receiver: Some(receiver),
            ..ParameterBindings::default()
        },
    );
    assert!(source.contains("Fixture.audit();"), "{source}");
    assert!(source.contains("if (!v"), "{source}");
    assert!(!source.contains("if (!this.a.j())"), "{source}");
}

#[test]
fn short_circuit_guard_rejects_observable_false_terminal_and_keeps_forward_plan() {
    let ir = boolean_conjunction(true);
    assert!(branching::plan_short_circuit_boolean_return(&ir, &[], &HashSet::new()).is_none());
    let source = emitted(&ir);
    assert!(source.contains("forwardBlock_"), "{source}");
    assert!(source.contains("Fixture.audit()"), "{source}");
}
