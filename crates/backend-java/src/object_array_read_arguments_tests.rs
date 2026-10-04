//! Array reads retain their original NPE/bounds events inside object arguments.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    effects::EffectFlags,
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
            let result = self.values.insert(Value::new(id, ty));
            value = Some(result);
            f(result)
        });
        self.order.push(id);
        value.unwrap()
    }
    fn initial(&mut self, ty: ManagedType) -> ValueId {
        let value = self.value(ty, |result| ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![result],
            effects: EffectFlags::NONE,
        });
        self.order.pop();
        value
    }
    fn push(&mut self, insn: ManagedInsn) -> InsnId {
        let id = self.insns.insert(insn);
        self.order.push(id);
        id
    }
    fn finish(self) -> MethodIr<SsaForm> {
        let mut cfg = Cfg::new(self.order[0]);
        let mut last = cfg.entry();
        for insn in self.order.iter().skip(1) {
            let next = cfg.add_block(*insn);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}
struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, c: ClassRef) -> Option<String> {
        (c.index() == 0).then(|| "Fixture.Box".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        (m.index() == 0).then(|| JavaMethod {
            owner: "Fixture.Box".into(),
            name: "<init>".into(),
            is_static: false,
            parameters: vec![JavaType::Int],
            return_type: JavaType::Void,
            throws: vec![],
        })
    }
}
fn fixture(length: bool, mode: u8) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut b = Builder::default();
    let array = b.initial(ManagedType::Array {
        element: Box::new(ManagedType::Int),
        nullable: true,
    });
    let index = b.initial(ManagedType::Int);
    let object = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    let read = b.value(ManagedType::Int, |result| {
        if length {
            ManagedInsn::ArrayLength { result, array }
        } else {
            ManagedInsn::ArrayGet {
                result,
                array,
                index,
            }
        }
    });
    if mode == 2 {
        b.insns.insert(ManagedInsn::Return { value: Some(read) });
    }
    if mode == 3 {
        b.push(ManagedInsn::ArrayPut {
            array,
            index,
            value: read,
        });
    }
    b.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![object, read],
    });
    b.push(ManagedInsn::Return {
        value: if mode == 1 { Some(read) } else { Some(object) },
    });
    (
        b.finish(),
        ParameterBindings {
            receiver: None,
            parameters: vec![array, index],
            names: vec![],
        },
    )
}
fn emit(ir: &MethodIr<SsaForm>, bindings: &ParameterBindings, capture: bool) -> SourceResult {
    emit_method(
        ir,
        &JavaMethod {
            owner: "Fixture".into(),
            name: "construct".into(),
            is_static: true,
            parameters: vec![JavaType::Array(Box::new(JavaType::Int)), JavaType::Int],
            return_type: if capture {
                JavaType::Int
            } else {
                JavaType::Class("Fixture.Box".into())
            },
            throws: vec![],
        },
        bindings,
        &Resolver,
    )
}
fn java(label: &str, source: &str, main: &str) {
    let root = std::env::temp_dir().join(format!(
        "decomphify-array-read-args-{label}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Fixture.java"),format!("public class Fixture {{static int mode;static String log=\"\";static int[] active;static class Box {{static{{log+=\"O\";if(mode==3)throw new OutOfMemoryError();}}final int value;Box(int value){{log+=\"C\";this.value=value;if(active!=null)active[0]=99;}}}}{source}public static void main(String[]args){{mode=Integer.parseInt(args[0]);active=mode==1?null:new int[]{{7,8}};{main}}}}}")).unwrap();
    let home = std::env::var_os("JAVA_HOME").map(std::path::PathBuf::from);
    let tool = |name: &str| {
        home.as_ref()
            .map(|h| h.join("bin").join(name))
            .unwrap_or_else(|| name.into())
    };
    let compiled = std::process::Command::new(tool("javac"))
        .args(["--release", "17", "Fixture.java"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    for mode in ["0", "1", "2", "3"] {
        let out = std::process::Command::new(tool("java"))
            .args(["-Xverify:all", "Fixture", mode])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "mode={mode}: {}\n{source}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn array_length_after_outer_allocation_preserves_null_and_class_init_failure_order() {
    // Genuine112: outernew22,const24,arraylength25,init26. Genuine180:
    // buildernew0,arraylength2,const3,mul4,init5. Both reads follow allocation.
    let (ir, bindings) = fixture(true, 0);
    let result = emit(&ir, &bindings, false);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert!(
        source
            .lines()
            .any(|line| line.contains("new Fixture.Box(") && line.contains("p0.length")),
        "{source}"
    );
    java(
        "length",
        &source,
        "if(mode==0||mode==2){if(construct(active,0).value!=2||!log.equals(\"OC\"))throw new AssertionError(log);}else{try{construct(active,0);throw new AssertionError();}catch(NullPointerException|OutOfMemoryError expected){if(mode==1&&!(expected instanceof NullPointerException)||mode==3&&!(expected instanceof OutOfMemoryError))throw new AssertionError(expected);}if(!log.equals(\"O\"))throw new AssertionError(log);}",
    );
}
#[test]
fn array_read_capture_is_single_evaluation_and_preserves_bounds_failure_order() {
    let (ir, bindings) = fixture(false, 1);
    let result = emit(&ir, &bindings, true);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert_eq!(source.matches("p0[p1]").count(), 1, "{source}");
    assert!(
        source
            .lines()
            .any(|line| line.contains("new Fixture.Box(") && line.contains(" = p0[p1]")),
        "{source}"
    );
    java(
        "read-capture",
        &source,
        "if(mode==0){if(construct(active,0)!=7||active[0]!=99||!log.equals(\"OC\"))throw new AssertionError(log);}else{try{construct(active,mode==2?9:0);throw new AssertionError();}catch(NullPointerException|ArrayIndexOutOfBoundsException|OutOfMemoryError expected){if(mode==1&&!(expected instanceof NullPointerException)||mode==2&&!(expected instanceof ArrayIndexOutOfBoundsException)||mode==3&&!(expected instanceof OutOfMemoryError))throw new AssertionError(expected);}if(!log.equals(\"O\"))throw new AssertionError(log);}",
    );
}
#[test]
fn unavailable_array_read_capture_rejects_and_sequenced_array_store_compiles() {
    for mode in [2, 3] {
        let (ir, bindings) = fixture(false, mode);
        let result = emit(&ir, &bindings, false);
        if mode == 2 {
            assert!(
                matches!(result, SourceResult::Unsupported(_)),
                "mode={mode}"
            );
        } else {
            let SourceResult::Complete { source, .. } = result else {
                panic!("mode{mode}:{result:?}")
            };
            crate::object_plan::sequence_tests::compile_fixture(
                &source,
                "static class Box { Box(int value) {} }",
            );
        }
    }
}
