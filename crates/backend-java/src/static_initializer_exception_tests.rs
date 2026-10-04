//! Normal final assignment and prethrow exceptional state stay distinct.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::{cfg::Cfg, ids::BlockId};
use exceptions::{
    RegionMetadata,
    scopes::{Catch, RegionScope},
};
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
};
use type_compatibility::{Compatibility, Declaration, Hierarchy};
struct Builder {
    cfg: Cfg<InsnId>,
    instructions: InsnArena,
    values: ValueArena,
}
impl Builder {
    fn new() -> Self {
        let mut instructions = InsnArena::default();
        let id = instructions.insert(ManagedInsn::Return { value: None });
        Self {
            cfg: Cfg::new(id),
            instructions,
            values: ValueArena::default(),
        }
    }
    fn block(&mut self) -> BlockId {
        self.cfg.add_block(
            self.instructions
                .insert(ManagedInsn::Return { value: None }),
        )
    }
    fn set(&mut self, b: BlockId, i: ManagedInsn) {
        self.instructions[*self.cfg.block(b).unwrap().payload()] = i;
    }
    fn value(&mut self, b: BlockId, ty: ManagedType) -> ValueId {
        self.values
            .insert(Value::new(*self.cfg.block(b).unwrap().payload(), ty))
    }
    fn link(&mut self, a: BlockId, b: BlockId) {
        self.cfg.add_edge(a, b, EdgeKind::Unconditional).unwrap();
    }
    fn finish(self) -> MethodIr<SsaForm> {
        MethodIr::<SsaForm>::from_parts(self.cfg, self.instructions, self.values)
    }
}
fn hierarchy() -> Hierarchy {
    let mut h = Hierarchy::default();
    for (name, parent) in [
        ("Object", None),
        ("Throwable", Some("Object")),
        ("Exception", Some("Throwable")),
        ("RuntimeException", Some("Exception")),
        ("IllegalArgumentException", Some("RuntimeException")),
        ("Error", Some("Throwable")),
        ("LinkageError", Some("Error")),
        ("NoSuchFieldError", Some("LinkageError")),
    ] {
        h.insert(Declaration {
            name: format!("java.lang.{name}"),
            superclass: parent.map(|p| format!("java.lang.{p}")),
            interfaces: vec![],
            is_interface: false,
        })
        .unwrap();
    }
    h
}
struct Resolver {
    hierarchy: Hierarchy,
}
impl JavaResolver for Resolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let (name, return_type, parameters) = match r.index() {
            0 => ("seed", JavaType::Int, vec![]),
            1 => ("work", JavaType::Void, vec![]),
            2 => (
                "capture",
                JavaType::Void,
                vec![JavaType::Int, JavaType::Class("java.lang.Throwable".into())],
            ),
            _ => ("finish", JavaType::Int, vec![]),
        };
        Some(JavaMethod {
            owner: "Probe".into(),
            name: name.into(),
            is_static: true,
            return_type,
            parameters,
            throws: vec![],
        })
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture".into(),
            name: if r.index() == 0 { "a" } else { "b" }.into(),
            ty: JavaType::Int,
            is_static: true,
        })
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r.index() == 0 {
                "java.lang.NoSuchFieldError"
            } else {
                "java.lang.IllegalArgumentException"
            }
            .into(),
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        self.hierarchy.assignable(a, b) == Compatibility::Assignable
    }
}
struct Metadata {
    scope: RegionScope,
    hierarchy: Hierarchy,
}
impl RegionMetadata for Metadata {
    fn region_scope(&self) -> &RegionScope {
        &self.scope
    }
    fn catch_type(&self, key: &str) -> Option<JavaType> {
        Some(JavaType::Class(
            if key == "error" {
                "java.lang.NoSuchFieldError"
            } else {
                "java.lang.IllegalArgumentException"
            }
            .into(),
        ))
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.hierarchy
    }
    fn is_catch_all(&self, _: &str) -> bool {
        false
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Normal,
    Missing,
    Double,
    WrongScope,
}
fn fixture(mode: Mode) -> (MethodIr<SsaForm>, Metadata) {
    let mut f = Builder::new();
    let first = f.cfg.entry();
    let seed = f.value(first, ManagedType::Int);
    f.set(
        first,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(seed),
            method: MethodRef::new(0),
            args: vec![],
        },
    );
    let store = f.block();
    f.set(
        store,
        ManagedInsn::FieldPut {
            object: None,
            field: FieldRef::new(0),
            value: seed,
        },
    );
    f.link(first, store);
    let work = f.block();
    f.set(
        work,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(1),
            args: vec![],
        },
    );
    f.link(store, work);
    let finish = f.block();
    let value = f.value(finish, ManagedType::Int);
    f.set(
        finish,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(value),
            method: MethodRef::new(3),
            args: vec![],
        },
    );
    f.link(work, finish);
    let mut catches = vec![];
    for (index, key) in ["error", "argument"].into_iter().enumerate() {
        let capture = f.block();
        let caught = f.value(capture, ManagedType::Reference(ClassRef::new(index as u32)));
        f.set(
            capture,
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads: vec![],
                writes: vec![caught],
                effects: managed_ir::effects::EffectFlags::NONE,
            },
        );
        f.cfg
            .add_edge(work, capture, EdgeKind::Exception(key.into()))
            .unwrap();
        let number = f.block();
        let n = f.value(number, ManagedType::Int);
        f.set(
            number,
            ManagedInsn::Const {
                result: n,
                value: Constant::Int(index as i32 + 1),
            },
        );
        f.link(capture, number);
        let call = f.block();
        f.set(
            call,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(2),
                args: vec![n, caught],
            },
        );
        f.link(number, call);
        f.link(call, finish);
        catches.push(Catch {
            key: key.into(),
            entry: capture,
        });
    }
    let suffix = f.block();
    f.link(finish, suffix);
    f.set(
        suffix,
        if matches!(mode, Mode::Missing) {
            ManagedInsn::Return { value: None }
        } else {
            ManagedInsn::FieldPut {
                object: None,
                field: FieldRef::new(1),
                value,
            }
        },
    );
    if !matches!(mode, Mode::Missing) {
        let ret = f.block();
        f.link(suffix, ret);
    }
    if matches!(mode, Mode::Double) {
        f.set(
            suffix,
            ManagedInsn::FieldPut {
                object: None,
                field: FieldRef::new(0),
                value,
            },
        );
    }
    let md = Metadata {
        scope: RegionScope {
            protected: vec![if matches!(mode, Mode::WrongScope) {
                store
            } else {
                work
            }],
            catches,
            continuation: Some(finish),
        },
        hierarchy: hierarchy(),
    };
    (f.finish(), md)
}
fn source(mode: Mode) -> Result<String, Unsupported> {
    let (ir, metadata) = fixture(mode);
    emit_static_initializer_body_subset_with_exceptions(
        &ir,
        &JavaMethod {
            owner: "Fixture".into(),
            name: "<clinit>".into(),
            is_static: true,
            return_type: JavaType::Void,
            parameters: vec![],
            throws: vec![],
        },
        &ParameterBindings::default(),
        &Resolver {
            hierarchy: hierarchy(),
        },
        &StaticInitializerContext {
            owner: "Fixture".into(),
            writable_static_fields: vec![],
        },
        &[FieldRef::new(0), FieldRef::new(1)],
        &[&metadata],
    )
}
#[test]
fn normal_and_caught_paths_keep_original_final_stores() {
    let s = source(Mode::Normal).unwrap();
    assert_eq!(s.matches("a = ").count(), 1);
    assert_eq!(s.matches("b = ").count(), 1);
    assert_eq!(s.matches("catch (").count(), 2);
    assert!(!s.contains("return;"));
    for mode in [Mode::Missing, Mode::Double, Mode::WrongScope] {
        assert!(source(mode).is_err());
    }
}
#[test]
fn failed_store_never_commits_final_assignment_to_caught_normal_exit() {
    let mut f = Builder::new();
    let first = f.cfg.entry();
    let value = f.value(first, ManagedType::Int);
    f.set(
        first,
        ManagedInsn::Const {
            result: value,
            value: Constant::Int(7),
        },
    );
    let store = f.block();
    f.set(
        store,
        ManagedInsn::FieldPut {
            object: None,
            field: FieldRef::new(0),
            value,
        },
    );
    f.link(first, store);
    let normal = f.block();
    let caught = f.block();
    f.link(store, normal);
    f.cfg
        .add_edge(store, caught, EdgeKind::Exception("error".into()))
        .unwrap();
    let fields = HashMap::from([(
        FieldRef::new(0),
        (
            Resolver {
                hierarchy: hierarchy(),
            }
            .field(FieldRef::new(0))
            .unwrap(),
            false,
        ),
    )]);
    assert!(
        matches!(prove_final_paths(&f.finish(),&fields,true),Err(Unsupported::Metadata(reason)) if reason=="blank final missing on normal path")
    );
}
#[test]
#[ignore = "requires JDK17"]
fn compiled_original_exception_priority_identity_defaults_and_effects_match() {
    use std::{path::PathBuf, process::Command};
    let home = std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let root = std::env::temp_dir().join(format!("static-final-exceptions-{}", std::process::id()));
    let original = "static {a=Probe.seed();try{Probe.work();}catch(NoSuchFieldError e){Probe.capture(1,e);}catch(IllegalArgumentException e){Probe.capture(2,e);}b=Probe.finish();}";
    let probe = r#"public class Probe {static String trace="";static Throwable saved;static int seed(){trace+="seed:"+Fixture.a+","+Fixture.b+";";return 5;}static void work(){trace+="work:"+Fixture.a+","+Fixture.b+";";int n=Integer.getInteger("mode",0);if(n==1){NoSuchFieldError e=new NoSuchFieldError();saved=e;throw e;}if(n==2){IllegalArgumentException e=new IllegalArgumentException();saved=e;throw e;}if(n==3){IllegalStateException e=new IllegalStateException();saved=e;throw e;}}static void capture(int n,Throwable e){trace+="caught:"+n+":"+(e==saved)+":"+Fixture.a+","+Fixture.b+";";if(Boolean.getBoolean("handlerThrow")){IllegalStateException e2=new IllegalStateException();saved=e2;throw e2;}}static int finish(){trace+="finish:"+Fixture.a+","+Fixture.b+";";return 9;}public static void main(String[]args){try{System.out.print(Fixture.a+","+Fixture.b+"|");}catch(ExceptionInInitializerError e){System.out.print("failed:"+(e.getCause()==saved)+"|");}System.out.print(trace);}}"#;
    for (index, body) in [original.into(), source(Mode::Normal).unwrap()]
        .into_iter()
        .enumerate()
    {
        let dir = root.join(index.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Fixture.java"),
            format!(
                "public class Fixture{{public static final int a;public static final int b;{body}}}"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("Probe.java"), probe).unwrap();
        let result = Command::new(home.join("bin/javac"))
            .args(["--release", "17", "Fixture.java", "Probe.java"])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{body}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    for (mode, throw) in [
        (0, false),
        (1, false),
        (2, false),
        (3, false),
        (1, true),
        (2, true),
    ] {
        let mut expected = None;
        for index in 0..2 {
            let run = Command::new(home.join("bin/java"))
                .args([format!("-Dmode={mode}"), format!("-DhandlerThrow={throw}")])
                .args(["-cp", ".", "Probe"])
                .current_dir(root.join(index.to_string()))
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
            if let Some(old) = &expected {
                assert_eq!(&run.stdout, old);
            } else {
                expected = Some(run.stdout);
            }
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
