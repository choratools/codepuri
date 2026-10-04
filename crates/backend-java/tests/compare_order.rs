//! Dalvik `cmp*` expressions must preserve NaN and signed-zero semantics.
#![allow(clippy::unwrap_used)]

#[path = "../src/compare_order.rs"]
mod compare_order;

use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, SourceResult,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::{Constant, OrderingOp},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{Value, ValueId},
};

struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}

fn ordering_method(name: &str, op: OrderingOp, lhs: Constant, rhs: Constant) -> String {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let push_const = |insns: &mut InsnArena, values: &mut ValueArena, constant, ty| {
        let mut value = None;
        let insn = insns.insert_with_key(|id| {
            let result = values.insert(Value::new(id, ty));
            value = Some(result);
            ManagedInsn::Const {
                result,
                value: constant,
            }
        });
        (insn, value.unwrap())
    };
    let (lhs_insn, lhs_value): (_, ValueId) = push_const(
        &mut insns,
        &mut values,
        lhs,
        match op {
            OrderingOp::CmpLowFloat | OrderingOp::CmpHighFloat => ManagedType::Float,
            OrderingOp::CmpLowDouble | OrderingOp::CmpHighDouble => ManagedType::Double,
            OrderingOp::CmpLong => ManagedType::Long,
        },
    );
    let (rhs_insn, rhs_value) = push_const(
        &mut insns,
        &mut values,
        rhs,
        match op {
            OrderingOp::CmpLowFloat | OrderingOp::CmpHighFloat => ManagedType::Float,
            OrderingOp::CmpLowDouble | OrderingOp::CmpHighDouble => ManagedType::Double,
            OrderingOp::CmpLong => ManagedType::Long,
        },
    );
    let mut compare = None;
    let compare_insn = insns.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Int));
        compare = Some(result);
        ManagedInsn::OrderingCompare {
            op,
            result,
            lhs: lhs_value,
            rhs: rhs_value,
        }
    });
    let return_insn = insns.insert(ManagedInsn::Return { value: compare });
    let mut cfg = Cfg::new(lhs_insn);
    let mut previous = cfg.entry();
    for insn in [rhs_insn, compare_insn, return_insn] {
        let block = cfg.add_block(insn);
        cfg.add_edge(previous, block, EdgeKind::Unconditional)
            .unwrap();
        previous = block;
    }
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    let method = JavaMethod {
        owner: "CompareOrderEmitter".into(),
        name: name.into(),
        parameters: vec![],
        return_type: JavaType::Int,
        is_static: true,
        throws: vec![],
    };
    match backend_java::emit_method(
        &ir,
        &method,
        &ParameterBindings {
            receiver: None,
            parameters: vec![],
            names: vec![],
        },
        &Resolver,
    ) {
        SourceResult::Complete { source, .. } => source,
        SourceResult::Unsupported(reason) => panic!("{reason:?}"),
    }
}

#[test]
fn generated_ordering_expressions_match_dalvik_runtime_edges() {
    let root =
        std::env::temp_dir().join(format!("decomphify-compare-order-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = format!(
        r#"
public class CompareOrder {{
    static int cmplFloat(float l, float r) {{ return {}; }}
    static int cmpgFloat(float l, float r) {{ return {}; }}
    static int cmplDouble(double l, double r) {{ return {}; }}
    static int cmpgDouble(double l, double r) {{ return {}; }}
    static int cmpLong(long l, long r) {{ return {}; }}
    static void check(int actual, int expected, String name) {{
        if (actual != expected) throw new AssertionError(name + ":" + actual + "!=" + expected);
    }}
    public static void main(String[] args) {{
        check(cmplFloat(Float.NaN, 0.0f), -1, "cmpl-float nan-left");
        check(cmplFloat(0.0f, Float.NaN), -1, "cmpl-float nan-right");
        check(cmpgFloat(Float.NaN, 0.0f), 1, "cmpg-float nan-left");
        check(cmpgFloat(-0.0f, 0.0f), 0, "cmpg-float signed-zero");
        check(cmplDouble(Double.NaN, 0.0d), -1, "cmpl-double nan-left");
        check(cmpgDouble(0.0d, Double.NaN), 1, "cmpg-double nan-right");
        check(cmplDouble(-0.0d, 0.0d), 0, "cmpl-double signed-zero");
        check(cmpLong(Long.MIN_VALUE, Long.MAX_VALUE), -1, "cmp-long minmax");
        check(cmpLong(Long.MAX_VALUE, Long.MIN_VALUE), 1, "cmp-long maxmin");
        check(cmpLong(-7L, -7L), 0, "cmp-long equal");
    }}
}}
"#,
        compare_order::ordering_expr(OrderingOp::CmpLowFloat, "l", "r"),
        compare_order::ordering_expr(OrderingOp::CmpHighFloat, "l", "r"),
        compare_order::ordering_expr(OrderingOp::CmpLowDouble, "l", "r"),
        compare_order::ordering_expr(OrderingOp::CmpHighDouble, "l", "r"),
        compare_order::ordering_expr(OrderingOp::CmpLong, "l", "r"),
    );
    let path = root.join("CompareOrder.java");
    std::fs::write(&path, source).unwrap();
    let jdk = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
    let compile = std::process::Command::new(format!("{jdk}/javac"))
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(format!("{jdk}/java"))
        .args(["-cp", root.to_str().unwrap(), "CompareOrder"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn emitter_ordering_compare_runs_with_dalvik_edges() {
    let root = std::env::temp_dir().join(format!(
        "decomphify-compare-order-emitter-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let methods = [
        ordering_method(
            "cmplFloatNan",
            OrderingOp::CmpLowFloat,
            Constant::FloatBits(f32::NAN.to_bits()),
            Constant::FloatBits(0.0f32.to_bits()),
        ),
        ordering_method(
            "cmpgFloatZero",
            OrderingOp::CmpHighFloat,
            Constant::FloatBits((-0.0f32).to_bits()),
            Constant::FloatBits(0.0f32.to_bits()),
        ),
        ordering_method(
            "cmplDoubleZero",
            OrderingOp::CmpLowDouble,
            Constant::DoubleBits((-0.0f64).to_bits()),
            Constant::DoubleBits(0.0f64.to_bits()),
        ),
        ordering_method(
            "cmpgDoubleNan",
            OrderingOp::CmpHighDouble,
            Constant::DoubleBits(f64::NAN.to_bits()),
            Constant::DoubleBits(0.0f64.to_bits()),
        ),
        ordering_method(
            "cmpLong",
            OrderingOp::CmpLong,
            Constant::Long(i64::MAX),
            Constant::Long(i64::MIN),
        ),
    ]
    .join("\n");
    let source = format!(
        "public class CompareOrderEmitter {{ {methods} static void check(int a, int e) {{ if (a != e) throw new AssertionError(a + \"!=\" + e); }} public static void main(String[] args) {{ check(cmplFloatNan(), -1); check(cmpgFloatZero(), 0); check(cmplDoubleZero(), 0); check(cmpgDoubleNan(), 1); check(cmpLong(), 1); }} }}"
    );
    let path = root.join("CompareOrderEmitter.java");
    std::fs::write(&path, source).unwrap();
    let jdk = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
    let compile = std::process::Command::new(format!("{jdk}/javac"))
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(format!("{jdk}/java"))
        .args(["-cp", root.to_str().unwrap(), "CompareOrderEmitter"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
