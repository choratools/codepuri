//! Descriptor-proven literal declarations must exist before branch hoisting.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, SourceResult,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::Constant,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};
struct Resolver;
impl JavaResolver for Resolver {
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("java.lang.String".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
fn fixture(constant: Constant) -> SourceResult {
    let mut instructions = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut parameter = None;
    instructions.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Boolean));
        parameter = Some(v);
        ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        }
    });
    let mut null = None;
    let first = instructions.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Null));
        null = Some(v);
        ManagedInsn::Const {
            result: v,
            value: constant,
        }
    });
    let mut copied = None;
    let mov = instructions.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Null));
        copied = Some(v);
        ManagedInsn::Move {
            result: v,
            value: null.unwrap(),
        }
    });
    let branch = instructions.insert(ManagedInsn::Return { value: None });
    let left = instructions.insert(ManagedInsn::Return { value: null });
    let right = instructions.insert(ManagedInsn::Return { value: copied });
    let mut cfg = Cfg::new(first);
    let m = cfg.add_block(mov);
    let b = cfg.add_block(branch);
    let l = cfg.add_block(left);
    let r = cfg.add_block(right);
    cfg.add_edge(cfg.entry(), m, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(m, b, EdgeKind::Unconditional).unwrap();
    let then_edge = cfg.add_edge(b, l, EdgeKind::BranchTrue).unwrap();
    let else_edge = cfg.add_edge(b, r, EdgeKind::BranchFalse).unwrap();
    instructions[branch] = ManagedInsn::Branch {
        condition: parameter,
        then_edge,
        else_edge: Some(else_edge),
    };
    let ir = MethodIr::<SsaForm>::from_parts(cfg, instructions, values);
    backend_java::emit_method(
        &ir,
        &JavaMethod {
            owner: "NullLocals".into(),
            name: "pick".into(),
            parameters: vec![JavaType::Boolean],
            return_type: JavaType::Class("java.lang.String".into()),
            is_static: true,
            throws: vec![],
        },
        &ParameterBindings {
            receiver: None,
            parameters: vec![parameter.unwrap()],
            names: vec![],
        },
        &Resolver,
    )
}
#[test]
fn hoisted_null_and_copy_use_genuine_return_descriptor() {
    for constant in [Constant::Null, Constant::Zero] {
        let SourceResult::Complete { source, .. } = fixture(constant) else {
            panic!("null family rejected")
        };
        assert!(source.contains("java.lang.String v"), "{source}");
        let dir =
            std::env::temp_dir().join(format!("null-locals-{}-{constant:?}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("NullLocals.java");
        std::fs::write(&file,format!("public class NullLocals {{{source} public static void main(String[] a){{if(pick(true)!=null||pick(false)!=null)throw new AssertionError();}}}}")).unwrap();
        let compile = std::process::Command::new("javac")
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let run = std::process::Command::new("java")
            .arg("-cp")
            .arg(&dir)
            .arg("NullLocals")
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
}
#[test]
fn null_annotation_does_not_turn_nonzero_bits_into_null() {
    assert!(matches!(
        fixture(Constant::Int(1)),
        SourceResult::Unsupported(_)
    ));
}

struct SplitResolver(JavaType);
impl JavaResolver for SplitResolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "SplitNull".into(),
            name: if r.index() == 0 {
                "number"
            } else {
                "reference"
            }
            .into(),
            parameters: vec![if r.index() == 0 {
                self.0.clone()
            } else {
                JavaType::Class("java.lang.String".into())
            }],
            return_type: JavaType::Void,
            is_static: true,
            throws: vec![],
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("java.lang.String".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
fn split_fixture(constant: Constant, numeric: JavaType) -> SourceResult {
    let mut instructions = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut zero = None;
    let first = instructions.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Null));
        zero = Some(v);
        ManagedInsn::Const {
            result: v,
            value: constant,
        }
    });
    let mut copied = None;
    let mov = instructions.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Null));
        copied = Some(v);
        ManagedInsn::Move {
            result: v,
            value: zero.unwrap(),
        }
    });
    let number = instructions.insert(ManagedInsn::Invoke {
        kind: managed_ir::ops::InvokeKind::Static,
        result: None,
        method: MethodRef::new(0),
        args: vec![copied.unwrap()],
    });
    let reference = instructions.insert(ManagedInsn::Invoke {
        kind: managed_ir::ops::InvokeKind::Static,
        result: None,
        method: MethodRef::new(1),
        args: vec![copied.unwrap()],
    });
    let ret = instructions.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(first);
    let mut previous = cfg.entry();
    for id in [mov, number, reference, ret] {
        let next = cfg.add_block(id);
        cfg.add_edge(previous, next, EdgeKind::Unconditional)
            .unwrap();
        previous = next;
    }
    let ir = MethodIr::<SsaForm>::from_parts(cfg, instructions, values);
    backend_java::emit_method(
        &ir,
        &JavaMethod {
            owner: "SplitNull".into(),
            name: "run".into(),
            parameters: vec![],
            return_type: JavaType::Void,
            is_static: true,
            throws: vec![],
        },
        &ParameterBindings {
            receiver: None,
            parameters: vec![],
            names: vec![],
        },
        &SplitResolver(numeric),
    )
}
#[test]
fn raw_zero_with_null_annotation_and_move_splits_numeric_reference_uses() {
    let SourceResult::Complete { source, .. } = split_fixture(Constant::Zero, JavaType::Int) else {
        panic!("split raw zero rejected")
    };
    let dir = std::env::temp_dir().join(format!("split-null-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("SplitNull.java");
    std::fs::write(&file,format!("public class SplitNull {{{source} static int calls; static void number(int n){{if(n!=0)throw new AssertionError();calls++;}} static void reference(String s){{if(s!=null)throw new AssertionError();calls++;}} public static void main(String[] a){{run();if(calls!=2)throw new AssertionError();}}}}")).unwrap();
    let compile = std::process::Command::new("javac")
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new("java")
        .arg("-cp")
        .arg(&dir)
        .arg("SplitNull")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
#[test]
fn true_null_numeric_and_category_one_wide_uses_remain_rejected() {
    assert!(matches!(
        split_fixture(Constant::Null, JavaType::Int),
        SourceResult::Unsupported(_)
    ));
    for wide in [JavaType::Long, JavaType::Double] {
        assert!(matches!(
            split_fixture(Constant::Null, wide.clone()),
            SourceResult::Unsupported(_)
        ));
        assert!(matches!(
            split_fixture(Constant::Zero, wide),
            SourceResult::Unsupported(_)
        ));
    }
}
