//! Null-throw recovery retains definitions, availability, and checked negatives.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
};

#[derive(Default)]
struct Builder {
    insns: InsnArena,
    values: ValueArena,
    order: Vec<InsnId>,
}
impl Builder {
    fn value(&mut self, ty: ManagedType, f: impl FnOnce(ValueId) -> ManagedInsn) -> ValueId {
        let mut value = None;
        let id = self.insns.insert_with_key(|id| {
            let v = self.values.insert(Value::new(id, ty));
            value = Some(v);
            f(v)
        });
        self.order.push(id);
        value.unwrap()
    }
    fn null(&mut self, literal: Constant) -> ValueId {
        self.value(ManagedType::Zero, |result| ManagedInsn::Const {
            result,
            value: literal,
        })
    }
    fn push(&mut self, i: ManagedInsn) {
        self.order.push(self.insns.insert(i));
    }
    fn finish(self) -> MethodIr<SsaForm> {
        let mut cfg = Cfg::new(self.order[0]);
        let mut last = cfg.entry();
        for id in &self.order[1..] {
            let next = cfg.add_block(*id);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}
struct Resolver;
impl JavaResolver for Resolver {
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "NullThrowHarness".into(),
            name: "mark".into(),
            is_static: true,
            return_type: JavaType::Void,
            parameters: vec![],
            throws: vec![],
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("java.io.IOException".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        a == b
            || matches!((a,b), (JavaType::Class(a),JavaType::Class(b))
            if a == "java.io.IOException" && b == "java.lang.Throwable")
    }
}
fn signature(name: &str) -> JavaMethod {
    JavaMethod {
        owner: "NullThrowHarness".into(),
        name: name.into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![],
        throws: vec![],
    }
}
fn emitted(literal: Constant, copied: bool) -> String {
    let mut b = Builder::default();
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(0),
        args: vec![],
    });
    let mut value = b.null(literal);
    if copied {
        value = b.value(ManagedType::Zero, |result| ManagedInsn::Move {
            result,
            value,
        });
    }
    b.push(ManagedInsn::Throw { value });
    match emit_method(
        &b.finish(),
        &signature(if copied { "copied" } else { "direct" }),
        &ParameterBindings::default(),
        &Resolver,
    ) {
        SourceResult::Complete { source, .. } => source,
        other => panic!("null throw unexpectedly unavailable: {other:?}"),
    }
}
#[test]
fn exact_null_and_zero_copies_emit_the_implicit_npe_without_throws() {
    for literal in [Constant::Null, Constant::Zero] {
        for copied in [false, true] {
            let text = emitted(literal, copied);
            assert!(text.contains("throw null;"), "{text}");
            assert!(!text.contains(" throws "), "{text}");
            assert!(text.find("mark()").unwrap() < text.find("throw null;").unwrap());
        }
    }
}
#[test]
fn nominal_zero_and_null_annotations_are_not_null_definition_proofs() {
    for ty in [ManagedType::Zero, ManagedType::Null] {
        let mut b = Builder::default();
        let value = b.value(ty, |v| ManagedInsn::Opaque {
            family_tag: 32769,
            reads: vec![],
            writes: vec![v],
            effects: managed_ir::effects::EffectFlags::NONE,
        });
        b.order.pop();
        b.push(ManagedInsn::Throw { value });
        let ir = b.finish();
        assert!(!proven_null_throw(&ir, value));
        assert!(matches!(
            emit_method(
                &ir,
                &signature("unavailable"),
                &ParameterBindings::default(),
                &Resolver
            ),
            SourceResult::Unsupported(_)
        ));
    }
}
#[test]
fn a_proven_constant_outside_the_cfg_still_needs_value_availability() {
    let mut b = Builder::default();
    let value = b.null(Constant::Zero);
    b.order.pop();
    b.push(ManagedInsn::Throw { value });
    let ir = b.finish();
    assert!(proven_null_throw(&ir, value));
    assert!(matches!(
        emit_method(
            &ir,
            &signature("unavailable"),
            &ParameterBindings::default(),
            &Resolver
        ),
        SourceResult::Unsupported(_)
    ));
}
#[test]
fn nonnull_checked_parameter_still_requires_the_exact_caller_contract() {
    let mut b = Builder::default();
    let value = b.value(ManagedType::Reference(ClassRef::new(0)), |v| {
        ManagedInsn::Opaque {
            family_tag: 32769,
            reads: vec![],
            writes: vec![v],
            effects: managed_ir::effects::EffectFlags::NONE,
        }
    });
    b.order.pop();
    b.push(ManagedInsn::Throw { value });
    let ir = b.finish();
    assert!(!proven_null_throw(&ir, value));
    let mut m = signature("checked");
    m.parameters
        .push(JavaType::Class("java.io.IOException".into()));
    let bindings = ParameterBindings {
        parameters: vec![value],
        ..Default::default()
    };
    assert!(matches!(
        emit_method(&ir, &m, &bindings, &Resolver),
        SourceResult::Unsupported(Unsupported::Metadata(_))
    ));
    m.throws.push("java.io.IOException".into());
    assert!(matches!(
        emit_method(&ir, &m, &bindings, &Resolver),
        SourceResult::Complete { .. }
    ));
}
#[test]
fn all_null_phi_evidence_preserves_mixed_and_cycle_rejection() {
    let mut b = Builder::default();
    let a = b.null(Constant::Null);
    let z = b.null(Constant::Zero);
    let mut cfg = Cfg::new(b.order[0]);
    let next = cfg.add_block(b.order[1]);
    let first = cfg
        .add_edge(cfg.entry(), next, EdgeKind::Unconditional)
        .unwrap();
    let second = cfg
        .add_edge(next, cfg.entry(), EdgeKind::Unconditional)
        .unwrap();
    let phi = b.value(ManagedType::Null, |result| ManagedInsn::Phi {
        result,
        inputs: vec![(first, a), (second, z)],
    });
    let bad = b.value(ManagedType::Zero, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    let mixed = b.value(ManagedType::Null, |result| ManagedInsn::Phi {
        result,
        inputs: vec![(first, a), (second, bad)],
    });
    let cyclic = b.value(ManagedType::Null, |result| ManagedInsn::Phi {
        result,
        inputs: vec![(first, a), (second, result)],
    });
    let ir = MethodIr::<SsaForm>::from_parts(cfg, b.insns, b.values);
    assert!(proven_null_throw(&ir, phi));
    assert!(!proven_null_throw(&ir, mixed));
    assert!(!proven_null_throw(&ir, cyclic));
}

#[test]
fn an_available_all_zero_phi_emits_after_checked_branch_selection() {
    let mut b = Builder::default();
    let condition = b.value(ManagedType::Boolean, |v| ManagedInsn::Opaque {
        family_tag: 32769,
        reads: vec![],
        writes: vec![v],
        effects: managed_ir::effects::EffectFlags::NONE,
    });
    let branch = b.insns.insert(ManagedInsn::Opaque {
        family_tag: 1,
        reads: vec![],
        writes: vec![],
        effects: managed_ir::effects::EffectFlags::NONE,
    });
    let left = b.null(Constant::Zero);
    let right = b.null(Constant::Zero);
    let selected = b.null(Constant::Zero);
    let throwing = b.insns.insert(ManagedInsn::Throw { value: selected });
    let left_id = b.values[left].def();
    let right_id = b.values[right].def();
    let selected_id = b.values[selected].def();
    let mut cfg = Cfg::new(branch);
    let left_block = cfg.add_block(left_id);
    let right_block = cfg.add_block(right_id);
    let join = cfg.add_block(selected_id);
    let terminal = cfg.add_block(throwing);
    let yes = cfg
        .add_edge(cfg.entry(), left_block, EdgeKind::BranchTrue)
        .unwrap();
    let no = cfg
        .add_edge(cfg.entry(), right_block, EdgeKind::BranchFalse)
        .unwrap();
    let left_edge = cfg
        .add_edge(left_block, join, EdgeKind::Unconditional)
        .unwrap();
    let right_edge = cfg
        .add_edge(right_block, join, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(join, terminal, EdgeKind::Unconditional)
        .unwrap();
    b.insns[branch] = ManagedInsn::Branch {
        condition: Some(condition),
        then_edge: yes,
        else_edge: Some(no),
    };
    b.insns[selected_id] = ManagedInsn::Phi {
        result: selected,
        inputs: vec![(left_edge, left), (right_edge, right)],
    };
    let ir = MethodIr::<SsaForm>::from_parts(cfg, b.insns, b.values);
    let mut m = signature("selected");
    m.parameters.push(JavaType::Boolean);
    let result = emit_method(
        &ir,
        &m,
        &ParameterBindings {
            parameters: vec![condition],
            ..Default::default()
        },
        &Resolver,
    );
    let SourceResult::Complete { source, .. } = result else {
        panic!("checked all-null phi: {result:?}");
    };
    assert!(source.contains("throw null;"), "{source}");
}
#[test]
fn compiled_null_and_copy_throw_npe_after_original_side_effects() {
    let direct = emitted(Constant::Zero, false);
    let copied = emitted(Constant::Null, true);
    let source = format!(
        r#"public class NullThrowHarness {{
        static int count; public static void mark() {{ count++; }}
        {direct} {copied}
        public static void main(String[] args) {{
            for(int i=0;i<2;i++) {{
                try {{ if(i==0) direct(); else copied(); throw new AssertionError(); }}
                catch(NullPointerException e) {{ if(e.getClass()!=NullPointerException.class) throw new AssertionError(); }}
            }}
            if(count!=2) throw new AssertionError(); System.out.print("NPE:2");
        }}
    }}"#
    );
    let dir = std::env::temp_dir().join(format!("decomphify-null-throw-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("NullThrowHarness.java"), source).unwrap();
    let jdk =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let compile = std::process::Command::new(format!("{jdk}/bin/javac"))
        .args(["--release", "17", "NullThrowHarness.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(format!("{jdk}/bin/java"))
        .args(["-cp", ".", "NullThrowHarness"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "NPE:2");
    std::fs::remove_dir_all(dir).unwrap();
}
