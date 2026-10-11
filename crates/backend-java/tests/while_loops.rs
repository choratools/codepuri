//! Compiled execution evidence for loop evaluation and edge-bound Phi copies.
#![allow(clippy::unwrap_used)]
use backend_java::{JavaMethod, JavaResolver, JavaType, ParameterBindings, SourceResult};
use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::BlockId,
};
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::{ArithOp, CompareOp, Constant, InvokeKind},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};

struct Fixture {
    cfg: Cfg<InsnId>,
    insns: InsnArena,
    values: ValueArena,
}
impl Fixture {
    fn new() -> Self {
        let mut insns = InsnArena::with_key();
        let id = insns.insert(ManagedInsn::Return { value: None });
        Self {
            cfg: Cfg::new(id),
            insns,
            values: ValueArena::with_key(),
        }
    }
    fn block(&mut self) -> BlockId {
        self.cfg
            .add_block(self.insns.insert(ManagedInsn::Return { value: None }))
    }
    fn set(&mut self, b: BlockId, i: ManagedInsn) {
        self.insns[*self.cfg.block(b).unwrap().payload()] = i;
    }
    fn value(&mut self, b: BlockId, ty: ManagedType) -> ValueId {
        self.values
            .insert(Value::new(*self.cfg.block(b).unwrap().payload(), ty))
    }
    fn parameter(&mut self) -> ValueId {
        let id = self.insns.insert(ManagedInsn::Return { value: None });
        self.values.insert(Value::new(id, ManagedType::Int))
    }
    fn link(&mut self, a: BlockId, b: BlockId) -> decomp_core::ids::EdgeId {
        self.cfg.add_edge(a, b, EdgeKind::Unconditional).unwrap()
    }
    fn branch(&mut self, b: BlockId, c: ValueId, yes: BlockId, no: BlockId) {
        let else_edge = self.cfg.add_edge(b, no, EdgeKind::BranchFalse).unwrap();
        let then_edge = self.cfg.add_edge(b, yes, EdgeKind::BranchTrue).unwrap();
        self.set(
            b,
            ManagedInsn::Branch {
                condition: Some(c),
                then_edge,
                else_edge: Some(else_edge),
            },
        );
    }
}
struct Resolver;
impl JavaResolver for Resolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "WhileMain".into(),
            name: format!("effect{}", r.index()),
            is_static: true,
            return_type: JavaType::Int,
            parameters: vec![JavaType::Int],
            throws: vec![],
        })
    }
    fn field(&self, _: FieldRef) -> Option<backend_java::JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}

fn fixture_parts(
    swap: bool,
    false_polarity: bool,
    effects: bool,
    nested: bool,
    count: bool,
) -> (Fixture, Vec<ValueId>) {
    let mut f = Fixture::new();
    let parameters = vec![f.parameter(), f.parameter(), f.parameter()];
    let entry = f.cfg.entry();
    let one = f.value(entry, ManagedType::Int);
    f.set(
        entry,
        if effects {
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(one),
                method: MethodRef::new(103),
                args: vec![parameters[0]],
            }
        } else {
            ManagedInsn::Const {
                result: one,
                value: Constant::Int(1),
            }
        },
    );
    let zero_block = f.block();
    let zero = f.value(zero_block, ManagedType::Int);
    f.set(
        zero_block,
        ManagedInsn::Const {
            result: zero,
            value: Constant::Int(0),
        },
    );
    f.link(entry, zero_block);
    let h = f.block();
    let ha = f.block();
    let hb = f.block();
    let n = f.value(h, ManagedType::Int);
    let a = f.value(ha, ManagedType::Int);
    let b = f.value(hb, ManagedType::Int);
    let initial = f.link(zero_block, h);
    f.link(h, ha);
    f.link(ha, hb);
    let mut last = hb;
    let test_value = if effects {
        let e = f.block();
        let v = f.value(e, ManagedType::Int);
        f.set(
            e,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(v),
                method: MethodRef::new(100),
                args: vec![n],
            },
        );
        f.link(last, e);
        last = e;
        v
    } else {
        n
    };
    let compare = f.block();
    let c = f.value(compare, ManagedType::Boolean);
    f.set(
        compare,
        ManagedInsn::Compare {
            result: c,
            op: if false_polarity {
                CompareOp::LessEqual
            } else {
                CompareOp::GreaterThan
            },
            lhs: test_value,
            rhs: zero,
        },
    );
    f.link(last, compare);
    let branch = f.block();
    f.link(compare, branch);
    let body = f.block();
    let sum = f.value(body, ManagedType::Int);
    f.set(
        body,
        ManagedInsn::Arithmetic {
            result: sum,
            op: ArithOp::Add,
            lhs: a,
            rhs: if count { one } else { n },
        },
    );
    let exit = f.block();
    f.set(exit, ManagedInsn::Return { value: Some(a) });
    f.branch(
        branch,
        c,
        if false_polarity { exit } else { body },
        if false_polarity { body } else { exit },
    );
    let mut last = body;
    if nested {
        let cmp = f.block();
        let d = f.value(cmp, ManagedType::Boolean);
        f.set(
            cmp,
            ManagedInsn::Compare {
                result: d,
                op: CompareOp::GreaterThan,
                lhs: n,
                rhs: one,
            },
        );
        f.link(last, cmp);
        let br = f.block();
        f.link(cmp, br);
        let yes = f.block();
        let no = f.block();
        for (block, method) in [(yes, 101), (no, 102)] {
            let v = f.value(block, ManagedType::Int);
            f.set(
                block,
                ManagedInsn::Invoke {
                    kind: InvokeKind::Static,
                    result: Some(v),
                    method: MethodRef::new(method),
                    args: vec![n],
                },
            );
        }
        f.branch(br, d, yes, no);
        let join = f.block();
        let v = f.value(join, ManagedType::Int);
        f.set(
            join,
            ManagedInsn::Move {
                result: v,
                value: n,
            },
        );
        f.link(yes, join);
        f.link(no, join);
        last = join;
    } else if effects {
        let e = f.block();
        let v = f.value(e, ManagedType::Int);
        f.set(
            e,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(v),
                method: MethodRef::new(101),
                args: vec![n],
            },
        );
        f.link(last, e);
        last = e;
    }
    let decrement = f.block();
    let next = f.value(decrement, ManagedType::Int);
    f.set(
        decrement,
        ManagedInsn::Arithmetic {
            result: next,
            op: ArithOp::Sub,
            lhs: n,
            rhs: one,
        },
    );
    f.link(last, decrement);
    let back = f.link(decrement, h);
    for (block, result, source, update) in [
        (h, n, parameters[0], next),
        (ha, a, parameters[1], if swap { b } else { sum }),
        (hb, b, parameters[2], a),
    ] {
        f.set(
            block,
            ManagedInsn::Phi {
                result,
                inputs: vec![(initial, source), (back, update)],
            },
        );
    }
    (f, parameters)
}

fn fixture(
    swap: bool,
    false_polarity: bool,
    effects: bool,
    nested: bool,
    count: bool,
) -> (MethodIr<SsaForm>, Vec<ValueId>) {
    let (f, bindings) = fixture_parts(swap, false_polarity, effects, nested, count);
    (
        MethodIr::<SsaForm>::from_parts(f.cfg, f.insns, f.values),
        bindings,
    )
}

#[test]
fn compiled_sum_count_zero_false_swap_effects_and_nested_if() {
    let mut methods = String::new();
    for (name, swap, polarity, effects, nested) in [
        ("sum", false, false, false, false),
        ("falseSum", false, true, false, false),
        ("swap", true, false, false, false),
        ("effects", false, false, true, false),
        ("nested", false, false, true, true),
        ("count", false, false, false, false),
    ] {
        let (ir, parameters) = fixture(swap, polarity, effects, nested, name == "count");
        let result = backend_java::emit_method(
            &ir,
            &JavaMethod {
                owner: "WhileMain".into(),
                name: name.into(),
                is_static: true,
                return_type: JavaType::Int,
                parameters: vec![JavaType::Int; 3],
                throws: vec![],
            },
            &ParameterBindings {
                parameters,
                ..Default::default()
            },
            &Resolver,
        );
        let SourceResult::Complete { source, .. } = result else {
            panic!("{result:?}")
        };
        assert!(source.contains("while ("), "{source}");
        if effects {
            assert!(
                source.find("while (true)").unwrap() < source.find("effect100").unwrap(),
                "{source}"
            );
        }
        methods.push_str(&source);
    }
    let source = format!(
        r#"public class WhileMain {{
{methods}
static int tests, bodies, left, right, initializations;
public static int effect100(int n) {{ tests++; return n; }}
public static int effect101(int n) {{ bodies++; left++; return n; }}
public static int effect102(int n) {{ bodies++; right++; return n; }}
public static int effect103(int n) {{ initializations++; return 1; }}
static void eq(int a,int b) {{ if(a!=b) throw new AssertionError(a+" != "+b); }}
public static void main(String[] args) {{
for(int n=0;n<10;n++) {{
eq(sum(n,0,7),n*(n+1)/2); eq(falseSum(n,0,7),n*(n+1)/2);
eq(sum(n,12,7),12+n*(n+1)/2);
eq(count(n,0,7),n);
eq(swap(n,3,8),(n%2==0)?3:8);
tests=bodies=left=right=initializations=0; eq(effects(n,0,7),n*(n+1)/2); eq(tests,n+1); eq(bodies,n); eq(initializations,1);
tests=bodies=left=right=initializations=0; eq(nested(n,0,7),n*(n+1)/2); eq(tests,n+1); eq(bodies,n); eq(initializations,1); eq(left,Math.max(0,n-1)); eq(right,n==0?0:1);
}}
eq(sum(-3,5,8),5); eq(falseSum(-3,5,8),5);
System.out.println("while semantic assertions passed");
}}
}}"#
    );
    let dir = std::env::temp_dir().join(format!("decomphify-while-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("WhileMain.java"), source).unwrap();
    let compiled = std::process::Command::new("javac")
        .arg("WhileMain.java")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = std::process::Command::new("java")
        .args(["-cp", ".", "WhileMain"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        executed.status.success(),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
    eprintln!(
        "Java evidence: {}: {}",
        dir.display(),
        String::from_utf8_lossy(&executed.stdout)
    );
}

#[test]
fn loop_plan_rejects_missing_phi_inputs_and_unavailable_values() {
    use backend_java::branching::{PlanError, plan};
    for case in 0..5 {
        let (f, bindings) = fixture_parts(false, false, false, false, false);
        let cfg = f.cfg;
        let mut insns = f.insns;
        let values = f.values;
        let body_result = insns
            .values()
            .find_map(|i| match i {
                ManagedInsn::Arithmetic {
                    result,
                    op: ArithOp::Add,
                    ..
                } => Some(*result),
                _ => None,
            })
            .unwrap();
        match case {
            0 => {
                let phi = insns
                    .values_mut()
                    .find(|i| matches!(i, ManagedInsn::Phi { .. }))
                    .unwrap();
                if let ManagedInsn::Phi { inputs, .. } = phi {
                    inputs.pop();
                }
            }
            1 => {
                let phi = insns
                    .values_mut()
                    .find(|i| matches!(i, ManagedInsn::Phi { .. }))
                    .unwrap();
                if let ManagedInsn::Phi { result, inputs } = phi {
                    inputs[0].1 = *result;
                }
            }
            2 => {
                for i in insns.values_mut() {
                    if let ManagedInsn::Return { value: Some(v) } = i {
                        *v = body_result;
                    }
                }
            }
            3 => {
                for i in insns.values_mut() {
                    if let ManagedInsn::Compare { lhs, .. } = i {
                        *lhs = body_result;
                    }
                }
            }
            4 => {
                let boolean = values
                    .iter()
                    .find_map(|(id, v)| (v.ty() == &ManagedType::Boolean).then_some(id))
                    .unwrap();
                let phi = insns
                    .values_mut()
                    .find(|i| matches!(i, ManagedInsn::Phi { .. }))
                    .unwrap();
                if let ManagedInsn::Phi { inputs, .. } = phi {
                    inputs[1].1 = boolean;
                }
            }
            _ => unreachable!(),
        }
        let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
        let region = decomp_structure::structure(ir.cfg()).unwrap().root;
        let result = plan(&ir, &region, &bindings);
        if case == 0 {
            assert!(
                matches!(result, Err(PlanError::InvalidPhi(_))),
                "{result:?}"
            );
        } else if case == 4 {
            assert!(
                matches!(result, Err(PlanError::TypeMismatch(_))),
                "{result:?}"
            );
        } else {
            assert!(
                matches!(result, Err(PlanError::Unavailable(_))),
                "{result:?}"
            );
        }
    }
}

#[test]
fn production_nested_loops_and_distinct_latch_phi_copies_execute() {
    let mut f = Fixture::new();
    let inputs = vec![f.parameter(), f.parameter()];
    let entry = f.cfg.entry();
    let one = f.value(entry, ManagedType::Int);
    f.set(
        entry,
        ManagedInsn::Const {
            result: one,
            value: Constant::Int(1),
        },
    );
    let z = f.block();
    let zero = f.value(z, ManagedType::Int);
    f.set(
        z,
        ManagedInsn::Const {
            result: zero,
            value: Constant::Int(0),
        },
    );
    f.link(entry, z);
    let h = f.block();
    let ha = f.block();
    let cmp = f.block();
    let br = f.block();
    let ih = f.block();
    let ia = f.block();
    let ic = f.block();
    let ib = f.block();
    let body = f.block();
    let decj = f.block();
    let after = f.block();
    let choose = f.block();
    let left = f.block();
    let adjust = f.block();
    let right = f.block();
    let exit = f.block();
    let n = f.value(h, ManagedType::Int);
    let a = f.value(ha, ManagedType::Int);
    let j = f.value(ih, ManagedType::Int);
    let inner = f.value(ia, ManagedType::Int);
    let initial = f.link(z, h);
    f.link(h, ha);
    f.link(ha, cmp);
    f.link(cmp, br);
    let c = f.value(cmp, ManagedType::Boolean);
    f.set(
        cmp,
        ManagedInsn::Compare {
            result: c,
            op: CompareOp::GreaterThan,
            lhs: n,
            rhs: zero,
        },
    );
    f.branch(br, c, ih, exit);
    let inner_initial = f.cfg.block(ih).unwrap().incoming()[0];
    f.link(ih, ia);
    f.link(ia, ic);
    f.link(ic, ib);
    let ci = f.value(ic, ManagedType::Boolean);
    f.set(
        ic,
        ManagedInsn::Compare {
            result: ci,
            op: CompareOp::GreaterThan,
            lhs: j,
            rhs: zero,
        },
    );
    f.branch(ib, ci, body, after);
    let sum = f.value(body, ManagedType::Int);
    f.set(
        body,
        ManagedInsn::Arithmetic {
            result: sum,
            op: ArithOp::Add,
            lhs: inner,
            rhs: one,
        },
    );
    f.link(body, decj);
    let nextj = f.value(decj, ManagedType::Int);
    f.set(
        decj,
        ManagedInsn::Arithmetic {
            result: nextj,
            op: ArithOp::Sub,
            lhs: j,
            rhs: one,
        },
    );
    let inner_back = f.link(decj, ih);
    f.set(
        ih,
        ManagedInsn::Phi {
            result: j,
            inputs: vec![(inner_initial, n), (inner_back, nextj)],
        },
    );
    f.set(
        ia,
        ManagedInsn::Phi {
            result: inner,
            inputs: vec![(inner_initial, a), (inner_back, sum)],
        },
    );
    let d = f.value(after, ManagedType::Boolean);
    f.set(
        after,
        ManagedInsn::Compare {
            result: d,
            op: CompareOp::GreaterThan,
            lhs: n,
            rhs: one,
        },
    );
    f.link(after, choose);
    f.branch(choose, d, left, adjust);
    let ten = f.value(adjust, ManagedType::Int);
    f.set(
        adjust,
        ManagedInsn::Const {
            result: ten,
            value: Constant::Int(10),
        },
    );
    let add = f.block();
    f.link(adjust, add);
    let adjusted = f.value(add, ManagedType::Int);
    f.set(
        add,
        ManagedInsn::Arithmetic {
            result: adjusted,
            op: ArithOp::Add,
            lhs: inner,
            rhs: ten,
        },
    );
    f.link(add, right);
    let mut backs = Vec::new();
    for latch in [left, right] {
        let next = f.value(latch, ManagedType::Int);
        f.set(
            latch,
            ManagedInsn::Arithmetic {
                result: next,
                op: ArithOp::Sub,
                lhs: n,
                rhs: one,
            },
        );
        backs.push((f.link(latch, h), next));
    }
    f.set(
        h,
        ManagedInsn::Phi {
            result: n,
            inputs: vec![(initial, inputs[0]), backs[0], backs[1]],
        },
    );
    f.set(
        ha,
        ManagedInsn::Phi {
            result: a,
            inputs: vec![
                (initial, inputs[1]),
                (backs[0].0, inner),
                (backs[1].0, adjusted),
            ],
        },
    );
    f.set(exit, ManagedInsn::Return { value: Some(a) });
    let ir = MethodIr::<SsaForm>::from_parts(f.cfg, f.insns, f.values);
    let result = backend_java::emit_method(
        &ir,
        &JavaMethod {
            owner: "WhileMain".into(),
            name: "nestedLatches".into(),
            is_static: true,
            return_type: JavaType::Int,
            parameters: vec![JavaType::Int; 2],
            throws: vec![],
        },
        &ParameterBindings {
            parameters: inputs,
            ..Default::default()
        },
        &Resolver,
    );
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}");
    };
    assert_eq!(source.matches("while (").count(), 2, "{source}");
    let source = format!(
        "public class WhileMain {{ {source} public static void main(String[] args) {{ for(int n=-2;n<9;n++) for(int seed:new int[]{{0,7}}) {{ int expected=seed+(n>0?n*(n+1)/2+10:0); if(nestedLatches(n,seed)!=expected) throw new AssertionError(n+\":\"+seed); }} System.out.println(\"nested/multilatch assertions passed\"); }} }}"
    );
    let dir =
        std::env::temp_dir().join(format!("decomphify-nested-latches-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("WhileMain.java"), &source).unwrap();
    let compiled = std::process::Command::new("javac")
        .arg("WhileMain.java")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = std::process::Command::new("java")
        .args(["-cp", ".", "WhileMain"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        executed.status.success(),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        "nested/multilatch assertions passed\n"
    );
}
