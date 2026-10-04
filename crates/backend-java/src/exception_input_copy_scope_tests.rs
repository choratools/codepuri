//! Original operand copies and synthetic producer commits have different provenance.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{
    DeclarationBindings, SourceResult,
    exceptions::{emit_method_region, scopes},
    type_compatibility::{Declaration, Hierarchy},
};
use decomp_core::cfg::Cfg;
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    ops::Constant,
    types::ManagedType,
    value::Value,
};
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
    fn id(&self, b: BlockId) -> InsnId {
        *self.cfg.block(b).unwrap().payload()
    }
    fn set(&mut self, b: BlockId, i: ManagedInsn) {
        let id = self.id(b);
        self.instructions[id] = i;
    }
    fn value(&mut self, b: BlockId, class: u32) -> ValueId {
        self.values.insert(Value::new(
            self.id(b),
            ManagedType::Reference(ClassRef::new(class)),
        ))
    }
    fn link(&mut self, a: BlockId, b: BlockId) {
        self.cfg.add_edge(a, b, EdgeKind::Unconditional).unwrap();
    }
}
struct Metadata {
    scope: scopes::RegionScope,
    original: OriginalScope,
    hierarchy: Hierarchy,
}
impl RegionMetadata for Metadata {
    fn original_scope(&self) -> Option<&OriginalScope> {
        Some(&self.original)
    }
    fn region_scope(&self) -> &scopes::RegionScope {
        &self.scope
    }
    fn catch_type(&self, _: &str) -> Option<JavaType> {
        Some(JavaType::Class("java.lang.RuntimeException".into()))
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.hierarchy
    }
    fn is_catch_all(&self, _: &str) -> bool {
        false
    }
}
struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            match r.index() {
                0 => "ScopeCopyHarness.Item",
                1 => "java.lang.String",
                _ => "java.lang.RuntimeException",
            }
            .into(),
        )
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        Some("input".encode_utf16().collect())
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(1)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: if r.index() == 0 {
                "ScopeCopyHarness.Item"
            } else {
                "ScopeCopyHarness"
            }
            .into(),
            name: if r.index() == 0 {
                "<init>"
            } else if r.index() == 1 {
                "captureLog"
            } else {
                "observe"
            }
            .into(),
            is_static: r.index() != 0,
            return_type: JavaType::Void,
            parameters: vec![JavaType::Class(
                if r.index() == 1 {
                    "java.lang.RuntimeException"
                } else {
                    "java.lang.String"
                }
                .into(),
            )],
            throws: vec![],
        })
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        a == b
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Original,
    SyntheticInput,
    SyntheticDelayed,
    ObservedCopy,
    ChangedDispatch,
    UninitializedEscape,
}
struct Fixture {
    ir: MethodIr<SsaForm>,
    metadata: Metadata,
    chain: Vec<InsnId>,
    input: ValueId,
}
fn fixture(mode: Mode) -> Fixture {
    let mut b = Builder::new();
    let outside = b.instructions.insert(ManagedInsn::Return { value: None });
    let input = b.values.insert(Value::new(
        outside,
        ManagedType::Reference(ClassRef::new(1)),
    ));
    let new = b.cfg.entry();
    let allocated = b.value(new, 0);
    b.set(
        new,
        ManagedInsn::NewInstance {
            result: allocated,
            class: ClassRef::new(0),
        },
    );
    let alias = b.block();
    let object = b.value(alias, 0);
    b.set(
        alias,
        ManagedInsn::Move {
            result: object,
            value: allocated,
        },
    );
    b.link(new, alias);
    let mut protected = vec![new, alias];
    let mut offsets = HashMap::from([(b.id(new), 27)]);
    let source = if matches!(mode, Mode::SyntheticDelayed) {
        let literal = b.block();
        let result = b.value(literal, 1);
        b.set(
            literal,
            ManagedInsn::Const {
                result,
                value: Constant::String(StringRef::new(0)),
            },
        );
        b.link(alias, literal);
        protected.push(literal);
        offsets.insert(b.id(literal), 29);
        result
    } else {
        input
    };
    let copy = b.block();
    let copied = b.value(copy, 1);
    b.set(
        copy,
        if matches!(mode, Mode::UninitializedEscape) {
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(2),
                args: vec![object],
            }
        } else {
            ManagedInsn::Move {
                result: copied,
                value: source,
            }
        },
    );
    b.link(*protected.last().unwrap(), copy);
    protected.push(copy);
    if !matches!(mode, Mode::SyntheticInput | Mode::SyntheticDelayed) {
        offsets.insert(b.id(copy), 29);
    }
    let ctor = b.block();
    b.set(
        ctor,
        ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(0),
            args: vec![object, copied],
        },
    );
    b.link(copy, ctor);
    protected.push(ctor);
    offsets.insert(b.id(ctor), 30);
    let normal = b.block();
    b.link(ctor, normal);
    let handler = b.block();
    let caught = b.value(handler, 2);
    b.set(
        handler,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![caught],
            effects: EffectFlags::NONE,
        },
    );
    let log = b.block();
    b.set(
        log,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(1),
            args: vec![caught],
        },
    );
    b.link(handler, log);
    offsets.insert(b.id(log), 40);
    let end = b.block();
    b.link(log, end);
    if matches!(mode, Mode::ObservedCopy) {
        b.set(
            end,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(2),
                args: vec![copied],
            },
        );
        let ret = b.block();
        b.link(end, ret);
    }
    b.cfg
        .add_edge(new, handler, EdgeKind::Exception("broad".into()))
        .unwrap();
    b.cfg
        .add_edge(
            ctor,
            handler,
            EdgeKind::Exception(
                if matches!(mode, Mode::ChangedDispatch) {
                    "different"
                } else {
                    "broad"
                }
                .into(),
            ),
        )
        .unwrap();
    if matches!(mode, Mode::SyntheticDelayed) {
        let literal = protected[2];
        b.cfg
            .add_edge(literal, handler, EdgeKind::Exception("broad".into()))
            .unwrap();
    }
    let chain = protected.iter().map(|block| b.id(*block)).collect();
    let mut hierarchy = Hierarchy::default();
    for (name, parent) in [
        ("Object", None),
        ("Throwable", Some("Object")),
        ("Exception", Some("Throwable")),
        ("RuntimeException", Some("Exception")),
    ] {
        hierarchy
            .insert(Declaration {
                name: format!("java.lang.{name}"),
                superclass: parent.map(|p| format!("java.lang.{p}")),
                interfaces: vec![],
                is_interface: false,
            })
            .unwrap();
    }
    let metadata = Metadata {
        scope: scopes::RegionScope {
            protected,
            catches: vec![scopes::Catch {
                key: "broad".into(),
                entry: handler,
            }],
            continuation: Some(normal),
        },
        original: OriginalScope {
            ordinal: 0,
            start: 27,
            end: 31,
            handler_offset: 1,
            catches: vec![("broad".into(), 40)],
            offsets,
        },
        hierarchy,
    };
    Fixture {
        ir: MethodIr::<SsaForm>::from_parts(b.cfg, b.instructions, b.values),
        metadata,
        chain,
        input,
    }
}
fn emitted(f: &Fixture, bind: bool) -> SourceResult {
    emit_method_region(
        &f.ir,
        &JavaMethod {
            owner: "ScopeCopyHarness".into(),
            name: "generated".into(),
            is_static: true,
            return_type: JavaType::Void,
            parameters: vec![JavaType::Class("java.lang.String".into())],
            throws: vec![],
        },
        &DeclarationBindings {
            parameters: vec![bind.then_some(f.input)],
            ..Default::default()
        },
        &Resolver,
        &f.metadata,
    )
}
#[test]
fn original_input_move_is_admitted_without_admitting_synthetic_input_copies() {
    let f = fixture(Mode::Original);
    assert!(prove(&f.ir, &[&f.metadata], &Resolver).contains(&f.chain));
    assert!(matches!(emitted(&f, true), SourceResult::Complete { .. }));
    assert!(matches!(emitted(&f, false), SourceResult::Unsupported(_)));
    let f = fixture(Mode::SyntheticInput);
    assert!(prove(&f.ir, &[&f.metadata], &Resolver).is_empty());
    let f = fixture(Mode::SyntheticDelayed);
    assert!(prove(&f.ir, &[&f.metadata], &Resolver).contains(&f.chain));
}
#[test]
fn original_range_dispatch_copy_observations_and_uninitialized_escape_still_reject() {
    let mut f = fixture(Mode::Original);
    f.metadata.original.offsets.insert(f.chain[2], 31);
    assert!(prove(&f.ir, &[&f.metadata], &Resolver).is_empty());
    let mut f = fixture(Mode::Original);
    f.metadata.original.end = 30;
    assert!(prove(&f.ir, &[&f.metadata], &Resolver).is_empty());
    let mut f = fixture(Mode::Original);
    f.metadata.original.offsets.remove(&f.chain[2]);
    assert!(prove(&f.ir, &[&f.metadata], &Resolver).is_empty());
    for mode in [Mode::ObservedCopy, Mode::ChangedDispatch] {
        let f = fixture(mode);
        assert!(prove(&f.ir, &[&f.metadata], &Resolver).is_empty());
        assert!(matches!(emitted(&f, true), SourceResult::Unsupported(_)));
    }
    let f = fixture(Mode::UninitializedEscape);
    assert!(prove(&f.ir, &[&f.metadata], &Resolver).is_empty());
}
#[test]
#[ignore = "requires genuine JDK25 ClassFile API"]
fn original_new_copy_init_bytecode_matches_success_ctor_failure_and_bounded_oom_order() {
    let f = fixture(Mode::Original);
    let SourceResult::Complete { source, .. } = emitted(&f, true) else {
        panic!("{:?}", emitted(&f, true))
    };
    let dir =
        std::env::temp_dir().join(format!("original-scope-input-copy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let unit = format!(
        r#"public class ScopeCopyHarness {{static int mode;static String events;static Throwable seen;static String input;static final RuntimeException failure=new RuntimeException("original");static class Item{{static{{events+="A";if(mode==3)throw new OutOfMemoryError("class-init boundary");}}Item(String s){{events+="C";if(s!=input)throw new AssertionError("argument identity");if(mode==2)throw failure;if(mode==5){{byte[] b=new byte[Integer.MAX_VALUE];events+=b.length;}}}}}}static void captureLog(RuntimeException e){{events+="H";seen=e;}}static void reference(String s){{try{{new Item(s);}}catch(RuntimeException e){{captureLog(e);}}}}{source}public static void main(String[]args){{mode=Integer.parseInt(args[0]);events="";input=new String("input");Throwable escaped=null;try{{if(Boolean.parseBoolean(args[1]))generated(input);else reference(input);}}catch(Throwable e){{escaped=e;}}System.out.println(events+":"+(seen==failure)+":"+(escaped==null?"none":escaped.getClass().getSimpleName()));}}}}"#
    );
    std::fs::write(dir.join("ScopeCopyHarness.java"), unit).unwrap();
    let original = r#"import java.lang.classfile.*;import java.lang.constant.*;import java.nio.file.*;public class Original{public static void main(String[]args)throws Exception{ClassFile cf=ClassFile.of();ClassDesc self=ClassDesc.of("ScopeCopyHarness"),item=ClassDesc.of("ScopeCopyHarness$Item"),runtime=ClassDesc.of("java.lang.RuntimeException");Path path=Path.of("ScopeCopyHarness.class");Files.write(path,cf.transformClass(cf.parse(path),(cb,e)->{if(e instanceof MethodModel mm&&mm.methodName().equalsString("reference"))cb.transformMethod(mm,(mb,me)->{if(me instanceof CodeModel)mb.withCode(c->{Label start=c.newLabel(),end=c.newLabel(),handler=c.newLabel();c.labelBinding(start).new_(item).astore(1);c.aload(0).astore(2);c.aload(1).aload(2).invokespecial(item,"<init>",MethodTypeDesc.ofDescriptor("(Ljava/lang/String;)V"));c.labelBinding(end).return_();c.labelBinding(handler).astore(3).aload(3).invokestatic(self,"captureLog",MethodTypeDesc.ofDescriptor("(Ljava/lang/RuntimeException;)V")).return_();c.exceptionCatch(start,end,handler,runtime);});else mb.with(me);});else cb.with(e);}));}}"#;
    std::fs::write(dir.join("Original.java"), original).unwrap();
    let home = std::env::var("DECOMPHIFY_JDK25_HOME")
        .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into());
    let run = |tool: &str, args: &[&str]| {
        let r = std::process::Command::new(format!("{home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        r.stdout
    };
    run(
        "javac",
        &["--release", "25", "ScopeCopyHarness.java", "Original.java"],
    );
    run("java", &["Original"]);
    let mut traces = vec![];
    for (mode, expected) in [
        (0, "AC:false:none\n"),
        (2, "ACH:true:none\n"),
        (3, "A:false:OutOfMemoryError\n"),
        (5, "AC:false:OutOfMemoryError\n"),
    ] {
        let mode = mode.to_string();
        let before = run(
            "java",
            &[
                "-Xverify:all",
                "-Xmx32m",
                "-cp",
                ".",
                "ScopeCopyHarness",
                &mode,
                "false",
            ],
        );
        let after = run(
            "java",
            &[
                "-Xverify:all",
                "-Xmx32m",
                "-cp",
                ".",
                "ScopeCopyHarness",
                &mode,
                "true",
            ],
        );
        assert_eq!(before, after);
        assert_eq!(String::from_utf8(after.clone()).unwrap(), expected);
        traces.extend(after);
    }
    std::fs::write(dir.join("runtime-traces.txt"), traces).unwrap();
}
