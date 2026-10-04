//! Exact category-one consumers retain available materialized expressions.
#![allow(clippy::unwrap_used)]
use backend_java::*;
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    insn::{Constant, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};
struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Fixture".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture".into(),
            name: "cached".into(),
            ty: JavaType::Byte,
            is_static: true,
        })
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(signature("mark", JavaType::Void, vec![JavaType::Int]))
    }
    fn instruction_type(&self, _: TypeRef) -> Option<JavaType> {
        Some(JavaType::Boolean)
    }
}
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
    fn push(&mut self, insn: ManagedInsn) {
        self.order.push(self.insns.insert(insn));
    }
    fn finish(self) -> MethodIr<SsaForm> {
        let mut cfg = Cfg::new(self.order[0]);
        let mut previous = cfg.entry();
        for insn in self.order.into_iter().skip(1) {
            let next = cfg.add_block(insn);
            cfg.add_edge(previous, next, EdgeKind::Unconditional)
                .unwrap();
            previous = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}
fn signature(name: &str, ty: JavaType, parameters: Vec<JavaType>) -> JavaMethod {
    JavaMethod {
        owner: "Fixture".into(),
        name: name.into(),
        return_type: ty,
        parameters,
        is_static: true,
        throws: vec![],
    }
}

fn bit_array(bit: i32) -> SourceResult {
    let mut b = Builder::default();
    let length = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    let arr = b.value(
        ManagedType::Array {
            element: Box::new(ManagedType::Boolean),
            nullable: true,
        },
        |result| ManagedInsn::NewArray {
            result,
            element: TypeRef::new(0),
            length,
        },
    );
    let zero = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(0),
    });
    let literal = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(bit),
    });
    let copy = b.value(ManagedType::Int, |result| ManagedInsn::Move {
        result,
        value: literal,
    });
    b.push(ManagedInsn::ArrayPut {
        array: arr,
        index: zero,
        value: copy,
    });
    b.push(ManagedInsn::Return { value: Some(arr) });
    emit_method(
        &b.finish(),
        &signature("bits", JavaType::Array(Box::new(JavaType::Boolean)), vec![]),
        &ParameterBindings::default(),
        &Resolver,
    )
}
fn byte_store(bit: i32) -> SourceResult {
    let mut b = Builder::default();
    let x = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(bit),
    });
    let y = b.value(ManagedType::Int, |result| ManagedInsn::Move {
        result,
        value: x,
    });
    b.push(ManagedInsn::FieldPut {
        object: None,
        field: FieldRef::new(0),
        value: y,
    });
    b.push(ManagedInsn::Return { value: None });
    emit_method(
        &b.finish(),
        &signature("store", JavaType::Void, vec![]),
        &ParameterBindings::default(),
        &Resolver,
    )
}
fn selected_bits() -> SourceResult {
    let mut b = Builder::default();
    let choose = b.value(ManagedType::Boolean, |x| ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![x],
        effects: managed_ir::effects::EffectFlags::NONE,
    });
    let len = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    let arr = b.value(
        ManagedType::Array {
            element: Box::new(ManagedType::Boolean),
            nullable: true,
        },
        |result| ManagedInsn::NewArray {
            result,
            element: TypeRef::new(0),
            length: len,
        },
    );
    let zero = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(0),
    });
    b.push(ManagedInsn::Return { value: None });
    let branch = *b.order.last().unwrap();
    let one = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    let yes = *b.order.last().unwrap();
    b.push(ManagedInsn::Invoke {
        kind: managed_ir::insn::InvokeKind::Static,
        result: None,
        method: MethodRef::new(0),
        args: vec![one],
    });
    let mark_yes = *b.order.last().unwrap();
    b.push(ManagedInsn::Invoke {
        kind: managed_ir::insn::InvokeKind::Static,
        result: None,
        method: MethodRef::new(0),
        args: vec![zero],
    });
    let mark_no = *b.order.last().unwrap();
    let selected = b.value(ManagedType::Int, |result| ManagedInsn::Phi {
        result,
        inputs: vec![],
    });
    let phi = *b.order.last().unwrap();
    b.push(ManagedInsn::ArrayPut {
        array: arr,
        index: zero,
        value: selected,
    });
    let put = *b.order.last().unwrap();
    b.push(ManagedInsn::Return { value: Some(arr) });
    let ret = *b.order.last().unwrap();
    let mut cfg = Cfg::new(b.order[1]);
    let mut previous = cfg.entry();
    for id in b.order[2..5].iter() {
        let next = cfg.add_block(*id);
        cfg.add_edge(previous, next, EdgeKind::Unconditional)
            .unwrap();
        previous = next;
    }
    let y = cfg.add_block(yes);
    let my = cfg.add_block(mark_yes);
    let mn = cfg.add_block(mark_no);
    let ph = cfg.add_block(phi);
    let pb = cfg.add_block(put);
    let rb = cfg.add_block(ret);
    let true_edge = cfg.add_edge(previous, y, EdgeKind::BranchTrue).unwrap();
    let false_edge = cfg.add_edge(previous, mn, EdgeKind::BranchFalse).unwrap();
    cfg.add_edge(y, my, EdgeKind::Unconditional).unwrap();
    let ye = cfg.add_edge(my, ph, EdgeKind::Unconditional).unwrap();
    let ne = cfg.add_edge(mn, ph, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(ph, pb, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(pb, rb, EdgeKind::Unconditional).unwrap();
    *b.insns.get_mut(branch).unwrap() = ManagedInsn::Branch {
        condition: Some(choose),
        then_edge: true_edge,
        else_edge: Some(false_edge),
    };
    *b.insns.get_mut(phi).unwrap() = ManagedInsn::Phi {
        result: selected,
        inputs: vec![(ye, one), (ne, zero)],
    };
    let ir = MethodIr::<SsaForm>::from_parts(cfg, b.insns, b.values);
    emit_method(
        &ir,
        &signature(
            "selected",
            JavaType::Array(Box::new(JavaType::Boolean)),
            vec![JavaType::Boolean],
        ),
        &ParameterBindings {
            parameters: vec![choose],
            ..Default::default()
        },
        &Resolver,
    )
}
fn complete(x: SourceResult) -> String {
    match x {
        SourceResult::Complete { source, .. } => source,
        x => panic!("{x:?}"),
    }
}
#[test]
fn proven_bits_keep_materialized_copy_and_array_store() {
    for n in 0..=1 {
        assert!(complete(bit_array(n)).contains("["));
        assert!(complete(byte_store(n)).contains("byte"));
    }
}
#[test]
fn arbitrary_int_cannot_become_boolean_or_narrow_field() {
    assert!(!matches!(bit_array(2), SourceResult::Complete { .. }));
    assert!(!matches!(byte_store(2), SourceResult::Complete { .. }));
}
#[test]
fn generated_selected_values_run_on_java17_and25() {
    let bits = complete(bit_array(1));
    let store = complete(byte_store(1));
    let selected = complete(selected_bits());
    let text = format!(
        "public class Fixture {{static byte cached; static int log; static boolean fail; static RuntimeException ERROR=new RuntimeException(); public static void mark(int n){{log=log*10+n+1;if(fail)throw ERROR;}} {bits} {store} {selected} public static void main(String[]x){{store();if(cached!=1||!bits()[0])throw new AssertionError();for(boolean b:new boolean[]{{false,true}}){{log=0;if(selected(b)[0]!=b||log!=(b?2:1))throw new AssertionError(); fail=true;try{{selected(b);throw new AssertionError();}}catch(RuntimeException e){{if(e!=ERROR)throw new AssertionError();}}fail=false;}}}} }}"
    );
    for (release, home) in [
        ("17", "/home/jtjisgod/tools/jdk-17.0.13+11"),
        ("25", "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1"),
    ] {
        let dir =
            std::env::temp_dir().join(format!("word-coercion-{}-{release}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Fixture.java"), &text).unwrap();
        for (tool, args) in [
            ("javac", vec!["--release", release, "Fixture.java"]),
            ("java", vec!["-cp", ".", "Fixture"]),
        ] {
            let o = std::process::Command::new(format!("{home}/bin/{tool}"))
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "{text}\n{}",
                String::from_utf8_lossy(&o.stderr)
            );
        }
    }
}
