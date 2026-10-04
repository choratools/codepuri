//! Operand folding must retain the original exception scope and prethrow state.
#![allow(clippy::unwrap_used)]
use backend_java::{
    DeclarationBindings, SourceResult,
    exceptions::{emit_method_region, scopes},
    type_compatibility::{Declaration, Hierarchy},
};
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType,
    exceptions::{OriginalScope, RegionMetadata, original_allocation_chains as prove},
};
use decomp_core::{cfg::Cfg, ids::EdgeId};
use decomp_core::{cfg::EdgeKind, ids::BlockId};
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    ops::Constant,
    types::ManagedType,
    value::Value,
};
use managed_ir::{
    insn::{InvokeKind, ManagedInsn},
    method::{MethodIr, SsaForm},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    value::{InsnId, ValueId},
};
use std::collections::HashMap;

struct Builder {
    cfg: Cfg<InsnId>,
    insns: InsnArena,
    values: ValueArena,
}
impl Builder {
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
        let id = self.insns.insert(ManagedInsn::Return { value: None });
        self.cfg.add_block(id)
    }
    fn id(&self, b: BlockId) -> InsnId {
        *self.cfg.block(b).unwrap().payload()
    }
    fn set(&mut self, b: BlockId, i: ManagedInsn) {
        let id = self.id(b);
        self.insns[id] = i;
    }
    fn value(&mut self, b: BlockId, class: u32) -> ValueId {
        self.values.insert(Value::new(
            self.id(b),
            ManagedType::Reference(ClassRef::new(class)),
        ))
    }
    fn edge(&mut self, a: BlockId, b: BlockId, k: EdgeKind) -> EdgeId {
        self.cfg.add_edge(a, b, k).unwrap()
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
        Some(JavaType::Class("java.io.IOException".into()))
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
                0 => "CheckedScopeArguments.Item",
                1 => "java.lang.String",
                _ => "java.io.IOException",
            }
            .into(),
        )
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        Some("message".encode_utf16().collect())
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        a == b
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(1)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: if r.index() == 0 {
                "CheckedScopeArguments.Item"
            } else {
                "CheckedScopeArguments"
            }
            .into(),
            name: match r.index() {
                0 => "<init>",
                1 => "message",
                2 => "captureLog",
                _ => "check",
            }
            .into(),
            is_static: r.index() != 0,
            return_type: if r.index() == 1 {
                JavaType::Class("java.lang.String".into())
            } else {
                JavaType::Void
            },
            parameters: vec![JavaType::Class(
                if r.index() == 2 {
                    "java.io.IOException"
                } else {
                    "java.lang.String"
                }
                .into(),
            )],
            throws: if matches!(r.index(), 0 | 1) {
                vec!["java.io.IOException".into()]
            } else {
                vec![]
            },
        })
    }
}
struct Fixture {
    ir: MethodIr<SsaForm>,
    md: Metadata,
    chain: Vec<InsnId>,
}
fn fixture(void_check: bool, unused: bool) -> Fixture {
    fixture_mode(void_check, unused, false, false, 0)
}
fn fixture_mode(
    void_check: bool,
    unused: bool,
    observe: bool,
    mismatch: bool,
    phi_mode: u8,
) -> Fixture {
    let mut b = Builder::new();
    let root = b.cfg.entry();
    let allocated = b.value(root, 0);
    b.set(
        root,
        ManagedInsn::NewInstance {
            result: allocated,
            class: ClassRef::new(0),
        },
    );
    let alias = b.block();
    let copied = b.value(alias, 0);
    b.set(
        alias,
        ManagedInsn::Move {
            result: copied,
            value: allocated,
        },
    );
    let literal = b.block();
    let string = b.value(literal, 1);
    b.set(
        literal,
        ManagedInsn::Const {
            result: string,
            value: Constant::String(StringRef::new(0)),
        },
    );
    let string_alias = b.block();
    let copied_string = b.value(string_alias, 1);
    b.set(
        string_alias,
        ManagedInsn::Move {
            result: copied_string,
            value: string,
        },
    );
    let producer = b.block();
    let produced = b.value(producer, 1);
    b.set(
        producer,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: if void_check { None } else { Some(produced) },
            method: MethodRef::new(if void_check { 3 } else { 1 }),
            args: vec![copied_string],
        },
    );
    let second = if phi_mode >= 2 {
        let block = b.block();
        let value = b.value(block, 1);
        b.set(
            block,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(value),
                method: MethodRef::new(1),
                args: vec![produced],
            },
        );
        Some((block, value))
    } else {
        None
    };
    let ctor = b.block();
    b.set(
        ctor,
        ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(0),
            args: vec![
                copied,
                if void_check || unused {
                    copied_string
                } else {
                    second.map_or(produced, |(_, value)| value)
                },
            ],
        },
    );
    let mut protected = vec![root, alias, literal, string_alias, producer];
    if let Some((block, _)) = second {
        protected.push(block);
    }
    protected.push(ctor);
    for pair in protected.windows(2) {
        b.edge(pair[0], pair[1], EdgeKind::Unconditional);
    }
    let normal = b.block();
    b.edge(ctor, normal, EdgeKind::Unconditional);
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
            method: MethodRef::new(2),
            args: vec![caught],
        },
    );
    b.edge(handler, log, EdgeKind::Unconditional);
    let end = b.block();
    b.edge(log, end, EdgeKind::Unconditional);
    if observe {
        b.set(
            end,
            ManagedInsn::Return {
                value: Some(produced),
            },
        );
    }
    if mismatch {
        b.edge(producer, end, EdgeKind::Exception("different".into()));
    }
    let entry = if phi_mode == 1 { b.block() } else { handler };
    let mut inputs = Vec::new();
    let mut operations = vec![root, literal, producer, ctor];
    if let Some((block, _)) = second {
        operations.push(block);
    }
    let foreign = (phi_mode == 3).then(|| b.block());
    for operation in operations {
        let destination = if second.is_some_and(|(block, _)| block == operation) {
            foreign.unwrap_or(entry)
        } else {
            entry
        };
        let edge = b.edge(operation, destination, EdgeKind::Exception("broad".into()));
        inputs.push((
            edge,
            if operation == root || operation == literal {
                string
            } else {
                produced
            },
        ));
    }
    if phi_mode == 1 {
        let selected = b.value(entry, 1);
        b.set(
            entry,
            ManagedInsn::Phi {
                result: selected,
                inputs,
            },
        );
        b.edge(entry, handler, EdgeKind::Unconditional);
    }
    let chain = protected.iter().map(|id| b.id(*id)).collect::<Vec<_>>();
    let mut offsets = HashMap::from([
        (b.id(root), 27),
        (b.id(literal), 29),
        (b.id(producer), 31),
        (b.id(ctor), 34),
        (b.id(log), 40),
    ]);
    if let Some((block, _)) = second {
        offsets.insert(b.id(block), 33);
    }
    let mut hierarchy = Hierarchy::default();
    for (name, parent) in [
        ("java.lang.Throwable", "java.lang.Object"),
        ("java.lang.Exception", "java.lang.Throwable"),
        ("java.io.IOException", "java.lang.Exception"),
    ] {
        hierarchy
            .insert(Declaration {
                name: name.into(),
                superclass: Some(parent.into()),
                interfaces: vec![],
                is_interface: false,
            })
            .unwrap();
    }
    let md = Metadata {
        scope: scopes::RegionScope {
            protected,
            catches: vec![scopes::Catch {
                key: "broad".into(),
                entry,
            }],
            continuation: Some(normal),
        },
        original: OriginalScope {
            ordinal: 0,
            start: 27,
            end: 35,
            handler_offset: 1,
            catches: vec![("broad".into(), 40)],
            offsets,
        },
        hierarchy,
    };
    Fixture {
        ir: MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values),
        md,
        chain,
    }
}
fn emitted(f: &Fixture) -> SourceResult {
    emit_method_region(
        &f.ir,
        &JavaMethod {
            owner: "CheckedScopeArguments".into(),
            name: "generated".into(),
            is_static: true,
            return_type: JavaType::Void,
            parameters: vec![],
            throws: vec![],
        },
        &DeclarationBindings::default(),
        &Resolver,
        &f.md,
    )
}
#[test]
fn original_operand_scope_exact_provenance_and_handler_states() {
    let mut f = fixture(false, false);
    assert!(prove(&f.ir, &[&f.md], &Resolver).contains(&f.chain));
    let SourceResult::Complete { source, .. } = emitted(&f) else {
        panic!("{:?}", emitted(&f))
    };
    assert!(
        source.contains("new CheckedScopeArguments.Item("),
        "{source}"
    );
    assert!(
        source.contains("CheckedScopeArguments.message("),
        "{source}"
    );
    assert_eq!(source.matches("CheckedScopeArguments.message(").count(), 1);
    f.md.original.offsets.insert(f.chain[2], 35);
    assert!(prove(&f.ir, &[&f.md], &Resolver).is_empty());
    let mut f = fixture(false, false);
    f.md.original.offsets.remove(&f.chain[2]);
    assert!(prove(&f.ir, &[&f.md], &Resolver).is_empty());
    let mut f = fixture(false, false);
    f.md.original.catches[0].1 = 41;
    assert!(prove(&f.ir, &[&f.md], &Resolver).is_empty());
    let f = fixture_mode(false, false, true, false, 0);
    assert!(prove(&f.ir, &[&f.md], &Resolver).is_empty());
}
#[test]
fn original_operand_scope_different_intermediate_dispatch_rejects() {
    let f = fixture_mode(false, false, false, true, 0);
    assert!(prove(&f.ir, &[&f.md], &Resolver).is_empty());
}
#[test]
fn original_operand_scope_intermediate_prethrow_phi_changes_reject() {
    let f = fixture_mode(false, false, false, false, 1);
    assert!(prove(&f.ir, &[&f.md], &Resolver).is_empty());
}
#[test]
fn original_operand_scope_runtime_effects_failure_and_throw_identity() {
    runtime_fixture(false, false);
}
#[test]
#[ignore = "requires explicit genuine JDK25 public ClassFile API"]
fn original_operand_scope_original_bytecode_runtime() {
    runtime_fixture(true, false);
}
fn runtime_fixture(original_bytecode: bool, void_check: bool) {
    let f = fixture(void_check, false);
    let SourceResult::Complete { source, .. } = emitted(&f) else {
        panic!("{:?}", emitted(&f))
    };
    let dir = std::env::temp_dir().join(format!(
        "decomphify-checked-object-argument-scope-{}-{}-{}",
        std::process::id(),
        original_bytecode,
        void_check
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("CheckedScopeArguments.java"),format!(r#"public class CheckedScopeArguments {{
static int mode,calls;static java.util.LinkedList<byte[]> keep;static String events;static Throwable seen;static final java.io.IOException failure=new java.io.IOException("original");
static class Item {{{large_fields} static {{events+="A";if(mode==3)throw new RuntimeException("classinit");}} Item(String s) throws java.io.IOException {{events+="C";if(mode==2)throw failure;if(mode==5){{byte[] b=new byte[Integer.MAX_VALUE];events+=b.length;}}if(!s.equals("{expected_message}"))throw new AssertionError();}} }}
static String message(String s) throws java.io.IOException {{calls++;events+="P";if(mode==1)throw failure;if(mode==4){{byte[] b=new byte[Integer.MAX_VALUE];events+=b.length;}}return s+"!";}}
static void check(String s) throws java.io.IOException {{message(s);}}
static void captureLog(java.io.IOException e) {{events+="H";seen=e;}}
static void reference() {{try {{new Item({reference_expression});}}catch(java.io.IOException e) {{captureLog(e);}}}}
{source}
public static void main(String[] args) throws Exception {{mode=Integer.parseInt(args[0]);boolean generated=Boolean.parseBoolean(args[1]);events="";seen=null;if(mode==6){{Class.forName("CheckedScopeArguments$Item");events="";keep=new java.util.LinkedList<>();try{{while(true)keep.add(new byte[8192]);}}catch(OutOfMemoryError exhausted){{}}}}Throwable escaped=null;try{{if(generated)generated();else reference();}}catch(Throwable e){{escaped=e;}}finally{{keep=null;}}if(mode==6){{System.gc();if(calls!=0||!(escaped instanceof OutOfMemoryError))throw new AssertionError("NEW-before-producer");}}System.out.println(events+":"+(seen==failure)+":"+(escaped==null?"none":escaped.getClass().getSimpleName())+":"+(escaped!=null&&escaped.getCause()==failure));}}
}}"#,large_fields=(0..4096).map(|n|format!("long f{n};")).collect::<String>(),expected_message=if void_check {"message"}else{"message!"},reference_expression=if void_check {"switch (0) { default -> {String s=\"message\";check(s);yield s;} }"}else{"message(\"message\")"})).unwrap();
    let home = if original_bytecode {
        std::env::var("DECOMPHIFY_JDK25_HOME")
            .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into())
    } else {
        std::env::var("DECOMPHIFY_JDK17_HOME")
            .unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into())
    };
    let compile = std::process::Command::new(format!("{home}/bin/javac"))
        .arg("CheckedScopeArguments.java")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    if original_bytecode {
        let original = r#"import java.lang.classfile.*;import java.lang.constant.*;import java.nio.file.*;
public class Original {public static void main(String[]args)throws Exception {ClassFile cf=ClassFile.of();ClassDesc self=ClassDesc.of("CheckedScopeArguments"),item=ClassDesc.of("CheckedScopeArguments$Item"),runtime=ClassDesc.of("java.io.IOException");Path path=Path.of("CheckedScopeArguments.class");Files.write(path,cf.transformClass(cf.parse(path),(cb,element)-> {if(element instanceof MethodModel mm&&mm.methodName().equalsString("reference"))cb.transformMethod(mm,(mb,me)-> {if(me instanceof CodeModel)mb.withCode(c->{Label start=c.newLabel(),end=c.newLabel(),handler=c.newLabel();
c.labelBinding(start).new_(item).astore(0);c.ldc("message").astore(1);c.aload(1).invokestatic(self,"message",MethodTypeDesc.ofDescriptor("(Ljava/lang/String;)Ljava/lang/String;")).astore(2);c.aload(0).aload(2).invokespecial(item,"<init>",MethodTypeDesc.ofDescriptor("(Ljava/lang/String;)V"));c.labelBinding(end).return_();c.labelBinding(handler).astore(3).aload(3).invokestatic(self,"captureLog",MethodTypeDesc.ofDescriptor("(Ljava/io/IOException;)V")).return_();c.exceptionCatch(start,end,handler,runtime);
});else mb.with(me);});else cb.with(element);}));}}
"#;
        let original = if void_check {
            original.replace("c.aload(1).invokestatic(self,\"message\",MethodTypeDesc.ofDescriptor(\"(Ljava/lang/String;)Ljava/lang/String;\")).astore(2);c.aload(0).aload(2)","c.aload(1).invokestatic(self,\"check\",MethodTypeDesc.ofDescriptor(\"(Ljava/lang/String;)V\"));c.aload(0).aload(1)")
        } else {
            original.into()
        };
        std::fs::write(dir.join("Original.java"), original).unwrap();
        for (tool, args) in [
            ("javac", vec!["--release", "25", "Original.java"]),
            ("java", vec!["Original"]),
        ] {
            let out = std::process::Command::new(format!("{home}/bin/{tool}"))
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
    let mut traces = Vec::new();
    for mode in 0..7 {
        let run = |generated: &str| {
            let out = std::process::Command::new(format!("{home}/bin/java"))
                .args([
                    "-Xmx32m",
                    "-Xverify:all",
                    "-cp",
                    ".",
                    "CheckedScopeArguments",
                    &mode.to_string(),
                    generated,
                ])
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            out.stdout
        };
        let expected = run("false");
        let actual = run("true");
        assert_eq!(expected, actual);
        traces.extend(actual);
    }
    assert_eq!(
        String::from_utf8(traces.clone()).unwrap(),
        "APC:false:none:false\nAPH:true:none:false\nAPCH:true:none:false\nA:false:ExceptionInInitializerError:false\nAP:false:OutOfMemoryError:false\nAPC:false:OutOfMemoryError:false\n:false:OutOfMemoryError:false\n"
    );
    std::fs::write(dir.join("runtime-traces.txt"), traces).unwrap();
}

#[test]
fn unrelated_checked_type_is_not_authorized_by_original_handler_identity() {
    struct WrongChecked;
    impl JavaResolver for WrongChecked {
        fn class(&self, r: ClassRef) -> Option<String> {
            Resolver.class(r)
        }
        fn field(&self, r: FieldRef) -> Option<JavaField> {
            Resolver.field(r)
        }
        fn string(&self, r: StringRef) -> Option<Vec<u16>> {
            Resolver.string(r)
        }
        fn method_access(&self, r: MethodRef) -> Option<u32> {
            Resolver.method_access(r)
        }
        fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
            Resolver.assignable(a, b)
        }
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            let mut method = Resolver.method(r)?;
            if r.index() == 1 {
                method.throws = vec!["java.sql.SQLException".into()];
            }
            Some(method)
        }
    }
    let f = fixture(false, false);
    let result = emit_method_region(
        &f.ir,
        &JavaMethod {
            owner: "CheckedScopeArguments".into(),
            name: "generated".into(),
            is_static: true,
            return_type: JavaType::Void,
            parameters: vec![],
            throws: vec![],
        },
        &DeclarationBindings::default(),
        &WrongChecked,
        &f.md,
    );
    assert!(matches!(result, SourceResult::Unsupported(_)), "{result:?}");
}

#[test]
fn matching_checked_producer_cannot_lend_permission_to_foreign_handler_producer() {
    let same = fixture_mode(false, false, false, false, 2);
    let SourceResult::Complete { source, .. } = emitted(&same) else {
        panic!("{:?}", emitted(&same))
    };
    assert_eq!(source.matches("CheckedScopeArguments.message(").count(), 2);
    let foreign = fixture_mode(false, false, false, false, 3);
    assert!(
        matches!(emitted(&foreign), SourceResult::Unsupported(_)),
        "{:?}",
        emitted(&foreign)
    );
}
