//! Exact selected handler transfers, never a scope union.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{
    JavaResolver, JavaType,
    type_compatibility::{Declaration, Hierarchy},
};
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
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
    fn add(&mut self) -> BlockId {
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
    fn value(&mut self, b: BlockId, ty: ManagedType) -> ValueId {
        self.values.insert(Value::new(self.id(b), ty))
    }
    fn link(&mut self, a: BlockId, b: BlockId) -> EdgeId {
        self.cfg.add_edge(a, b, EdgeKind::Unconditional).unwrap()
    }
}
struct Metadata {
    region: super::super::super::scopes::RegionScope,
    original: OriginalScope,
    hierarchy: Hierarchy,
    name: String,
}
impl RegionMetadata for Metadata {
    fn original_scope(&self) -> Option<&OriginalScope> {
        Some(&self.original)
    }
    fn region_scope(&self) -> &super::super::super::scopes::RegionScope {
        &self.region
    }
    fn catch_type(&self, _: &str) -> Option<JavaType> {
        Some(JavaType::Class(self.name.clone()))
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
                0 => "CrossHarness.Item",
                1 => "java.lang.String",
                2 => "java.lang.Exception",
                _ => "java.lang.Object",
            }
            .into(),
        )
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(if r.index() == 0 {
            JavaMethod {
                owner: "CrossHarness.Item".into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: vec![
                    JavaType::Class("java.lang.String".into()),
                    JavaType::Int,
                    JavaType::Class("java.lang.Object".into()),
                ],
                throws: vec![],
            }
        } else {
            JavaMethod {
                owner: "CrossHarness".into(),
                name: "observe".into(),
                is_static: true,
                return_type: JavaType::Void,
                parameters: vec![
                    JavaType::Class("java.lang.Exception".into()),
                    JavaType::Class("java.lang.String".into()),
                    JavaType::Int,
                ],
                throws: vec![],
            }
        })
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        a == b || *b == JavaType::Class("java.lang.Object".into())
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Good,
    Changed,
    Missing,
    Prethrow,
    Synthetic,
    ThrowingGap,
    HeapGap,
    ChangedCatch,
    ChangedRange,
    EscapingUninitialized,
    ConflictingNormal,
    ReassignedInput,
    MissingExceptionEdge,
    UnknownHierarchy,
}
struct Fixture {
    ir: MethodIr<SsaForm>,
    metadata: Vec<Metadata>,
    chain: Vec<InsnId>,
    parameters: Vec<ValueId>,
}
fn fixture(mode: Mode) -> Fixture {
    let mut b = Builder::new();
    let initial = |b: &mut Builder, ty| {
        let id = b.instructions.insert(ManagedInsn::Return { value: None });
        let v = b.values.insert(Value::new(id, ty));
        b.instructions[id] = ManagedInsn::Opaque {
            family_tag: 0x8003,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        };
        v
    };
    let input = initial(&mut b, ManagedType::Reference(ClassRef::new(1)));
    let op = initial(&mut b, ManagedType::Int);
    let other_op = initial(&mut b, ManagedType::Int);
    let new = b.cfg.entry();
    let allocated = b.value(new, ManagedType::Reference(ClassRef::new(0)));
    b.set(
        new,
        ManagedInsn::NewInstance {
            result: allocated,
            class: ClassRef::new(0),
        },
    );
    let alias = b.add();
    let object = b.value(alias, ManagedType::Reference(ClassRef::new(0)));
    b.set(
        alias,
        ManagedInsn::Move {
            result: object,
            value: allocated,
        },
    );
    b.link(new, alias);
    let zero = b.add();
    let null = b.value(zero, ManagedType::Zero);
    b.set(
        zero,
        ManagedInsn::Const {
            result: null,
            value: Constant::Zero,
        },
    );
    b.link(alias, zero);
    let copy = b.add();
    let copied = b.value(copy, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        copy,
        match mode {
            Mode::ThrowingGap => ManagedInsn::Const {
                result: copied,
                value: Constant::String(StringRef::new(0)),
            },
            Mode::HeapGap => ManagedInsn::FieldGet {
                result: copied,
                field: FieldRef::new(0),
                object: None,
            },
            Mode::EscapingUninitialized => ManagedInsn::Move {
                result: copied,
                value: object,
            },
            _ => ManagedInsn::Move {
                result: copied,
                value: input,
            },
        },
    );
    b.link(zero, copy);
    let copyop = b.add();
    let copiedop = b.value(copyop, ManagedType::Int);
    b.set(
        copyop,
        ManagedInsn::Move {
            result: copiedop,
            value: if matches!(mode, Mode::ReassignedInput) {
                other_op
            } else {
                op
            },
        },
    );
    b.link(copy, copyop);
    let ctor = b.add();
    b.set(
        ctor,
        ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(0),
            args: vec![object, copied, copiedop, null],
        },
    );
    b.link(copyop, ctor);
    let normal = b.add();
    b.link(ctor, normal);
    let left = b.add();
    let lc = b.value(left, ManagedType::Reference(ClassRef::new(2)));
    b.set(
        left,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![lc],
            effects: EffectFlags::MAY_THROW,
        },
    );
    let leftcapture = b.add();
    let lcc = b.value(leftcapture, ManagedType::Reference(ClassRef::new(2)));
    b.set(
        leftcapture,
        ManagedInsn::Move {
            result: lcc,
            value: lc,
        },
    );
    b.link(left, leftcapture);
    let restore = b.add();
    let restored = b.value(restore, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        restore,
        ManagedInsn::Move {
            result: restored,
            value: if matches!(mode, Mode::Prethrow) {
                copied
            } else {
                input
            },
        },
    );
    b.link(leftcapture, restore);
    let restoreop = b.add();
    let restoredop = b.value(restoreop, ManagedType::Int);
    b.set(
        restoreop,
        if matches!(mode, Mode::Changed) {
            ManagedInsn::Const {
                result: restoredop,
                value: Constant::Int(99),
            }
        } else {
            ManagedInsn::Move {
                result: restoredop,
                value: op,
            }
        },
    );
    b.link(restore, restoreop);
    let jump = b.add();
    b.link(restoreop, jump);
    let right = b.add();
    let rc = b.value(right, ManagedType::Reference(ClassRef::new(2)));
    b.set(
        right,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![rc],
            effects: EffectFlags::MAY_THROW,
        },
    );
    let rightcapture = b.add();
    let rcc = b.value(rightcapture, ManagedType::Reference(ClassRef::new(2)));
    b.set(
        rightcapture,
        ManagedInsn::Move {
            result: rcc,
            value: rc,
        },
    );
    b.link(right, rightcapture);
    let phic = b.add();
    let phis = b.add();
    let phii = b.add();
    let le = b.link(jump, phic);
    let re = b.link(rightcapture, phic);
    b.set(
        jump,
        ManagedInsn::Branch {
            condition: None,
            then_edge: le,
            else_edge: None,
        },
    );
    let c = b.value(phic, ManagedType::Reference(ClassRef::new(2)));
    let s = b.value(phis, ManagedType::Reference(ClassRef::new(1)));
    let i = b.value(phii, ManagedType::Int);
    b.set(
        phic,
        ManagedInsn::Phi {
            result: c,
            inputs: vec![(le, lcc), (re, rcc)],
        },
    );
    b.link(phic, phis);
    b.set(
        phis,
        ManagedInsn::Phi {
            result: s,
            inputs: if matches!(mode, Mode::Missing) {
                vec![(le, restored)]
            } else {
                vec![(le, restored), (re, copied)]
            },
        },
    );
    b.link(phis, phii);
    b.set(
        phii,
        ManagedInsn::Phi {
            result: i,
            inputs: vec![
                (le, restoredop),
                (
                    re,
                    if matches!(mode, Mode::ConflictingNormal) {
                        null
                    } else {
                        copiedop
                    },
                ),
            ],
        },
    );
    let tail = b.add();
    b.link(phii, tail);
    b.set(
        tail,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(1),
            args: vec![c, s, i],
        },
    );
    let end = b.add();
    b.link(tail, end);
    b.cfg
        .add_edge(new, left, EdgeKind::Exception("left".into()))
        .unwrap();
    if !matches!(mode, Mode::MissingExceptionEdge) {
        b.cfg
            .add_edge(ctor, right, EdgeKind::Exception("right".into()))
            .unwrap();
    }
    let mut offsets = HashMap::from([
        (b.id(new), 161),
        (b.id(zero), 163),
        (b.id(copy), 164),
        (b.id(copyop), 165),
        (b.id(ctor), 168),
        (b.id(leftcapture), 190),
        (b.id(restore), 191),
        (b.id(restoreop), 192),
        (b.id(jump), 194),
        (b.id(rightcapture), 187),
        (b.id(tail), 195),
    ]);
    if matches!(mode, Mode::Synthetic) {
        offsets.remove(&b.id(copy));
    }
    let mut hierarchy = Hierarchy::default();
    for declaration in [
        Declaration {
            name: "java.lang.Object".into(),
            superclass: None,
            interfaces: vec![],
            is_interface: false,
        },
        Declaration {
            name: "java.lang.Throwable".into(),
            superclass: Some("java.lang.Object".into()),
            interfaces: vec![],
            is_interface: false,
        },
        Declaration {
            name: "java.lang.Exception".into(),
            superclass: Some("java.lang.Throwable".into()),
            interfaces: vec![],
            is_interface: false,
        },
        Declaration {
            name: "java.lang.RuntimeException".into(),
            superclass: Some("java.lang.Exception".into()),
            interfaces: vec![],
            is_interface: false,
        },
    ] {
        let mut declaration = declaration;
        if matches!(mode, Mode::UnknownHierarchy) && declaration.name == "java.lang.Exception" {
            declaration.superclass = Some("unknown.Super".into());
        }
        hierarchy.insert(declaration).unwrap();
    }
    let md = |ordinal, start, end, key: &str, entry, address, protected| Metadata {
        region: super::super::super::scopes::RegionScope {
            protected,
            catches: vec![super::super::super::scopes::Catch {
                key: key.into(),
                entry,
            }],
            continuation: Some(normal),
        },
        original: OriginalScope {
            ordinal,
            start,
            end,
            handler_offset: ordinal as u16,
            catches: vec![(key.into(), address)],
            offsets: offsets.clone(),
        },
        hierarchy: hierarchy.clone(),
        name: if ordinal == 1 && matches!(mode, Mode::ChangedCatch) {
            "java.lang.RuntimeException"
        } else {
            "java.lang.Exception"
        }
        .into(),
    };
    let metadata = vec![
        md(0, 157, 163, "left", left, 190, vec![new, alias]),
        md(
            1,
            if matches!(mode, Mode::ChangedRange) {
                169
            } else {
                168
            },
            187,
            "right",
            right,
            187,
            vec![ctor],
        ),
    ];
    let chain = [new, alias, zero, copy, copyop, ctor]
        .into_iter()
        .map(|n| b.id(n))
        .collect();
    Fixture {
        ir: MethodIr::<SsaForm>::from_parts(b.cfg, b.instructions, b.values),
        metadata,
        chain,
        parameters: vec![input, op],
    }
}
#[test]
fn genuine_split_scope_chain_has_an_independent_typed_transfer_witness() {
    let f = fixture(Mode::Good);
    let md = f
        .metadata
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let proof = super::super::prove(&f.ir, &md, &Resolver);
    assert!(proof.pure.get(&f.chain).is_some_and(Witness::attested));
    assert!(!proof.chains.contains(&f.chain));
    let resolver = super::super::ScopedResolver::new(&Resolver);
    resolver.install(proof);
    assert!(resolver.exception_allocation_pure_chain(&f.chain));
    assert!(!resolver.exception_allocation_chain(&f.chain));
    let _ = &f.parameters;
}
#[test]
fn incomplete_changed_prethrow_or_effectful_transfer_never_attests() {
    for mode in [
        Mode::Changed,
        Mode::Missing,
        Mode::Prethrow,
        Mode::Synthetic,
        Mode::ThrowingGap,
        Mode::HeapGap,
        Mode::ChangedCatch,
        Mode::ChangedRange,
        Mode::EscapingUninitialized,
        Mode::ConflictingNormal,
        Mode::ReassignedInput,
        Mode::MissingExceptionEdge,
        Mode::UnknownHierarchy,
    ] {
        let f = fixture(mode);
        let md = f
            .metadata
            .iter()
            .map(|m| m as &dyn RegionMetadata)
            .collect::<Vec<_>>();
        assert!(prove(&f.ir, &md, &Resolver).is_empty());
    }
}

fn emitted_source() -> String {
    let f = fixture(Mode::Good);
    let md = f
        .metadata
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let method = JavaMethod {
        owner: "CrossHarness".into(),
        name: "generated".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![JavaType::Class("java.lang.String".into()), JavaType::Int],
        throws: vec![],
    };
    let bindings = crate::DeclarationBindings {
        parameters: f.parameters.iter().copied().map(Some).collect(),
        ..Default::default()
    };
    let result =
        super::super::super::dispatch::emit_method(&f.ir, &method, &bindings, &Resolver, &md);
    let crate::SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}");
    };
    source
}

#[test]
fn pure_chain_capability_materializes_a_complete_original_dispatch_body() {
    let source = emitted_source();
    assert!(source.contains("new CrossHarness.Item("));
    assert!(source.contains("CrossHarness.observe("));
}

#[test]
#[ignore = "requires JDK25 ClassFile API; separate NEW/init original bytecode oracle"]
fn original_separate_new_init_tables_match_emitted_source_and_failure_identity() {
    let source = emitted_source();
    let dir = std::env::temp_dir().join(format!("original-cross-scope-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class CrossHarness {{
static int mode,observed=-1;static String events="";static final String INPUT=new String("input");static Throwable seen;static Object published;static boolean preserved;
static final RuntimeException handlerFailure=new RuntimeException("handler");static class Probe extends RuntimeException{{public String toString(){{events+="T";if(mode==7)throw handlerFailure;return "runtime";}}}}static class CheckedProbe extends java.io.IOException{{public String toString(){{events+="T";return "checked";}}}}
static final RuntimeException failure=new Probe();static final java.io.IOException checked=new CheckedProbe();static final Error error=new AssertionError("ctor"),oom=new OutOfMemoryError("clinit");static final Throwable base=new Throwable("base");
static void unexpected(Throwable t){{throw new AssertionError("untransformed");}}
static class Item{{static int initialized=init();static int init(){{events+="A";if(mode==3)throw oom;if(mode==4)throw failure;if(mode==5){{byte[] b=new byte[Integer.MAX_VALUE];return b.length;}}return 1;}}Item(String s,int op,Object marker){{events+="C";published=this;if(s!=INPUT||marker!=null)throw new AssertionError("input");if(mode==1||mode==6||mode==7)throw failure;if(mode==2)throw error;if(mode==8)unexpected(checked);if(mode==9)unexpected(base);}}}}
static void observe(Exception t,String s,int op){{events+="H";seen=t;preserved=s==INPUT&&op==7;observed=op;if(mode==6)throw handlerFailure;String message=t.toString();if(message==null)throw new AssertionError("message");}}
static void original(String s,int op){{throw new AssertionError("untransformed");}}static void originalConflict(String s,int op){{throw new AssertionError("untransformed");}}
{source}
public static void main(String[]args){{mode=Integer.parseInt(args[0]);Throwable escaped=null;try{{if(args[1].equals("original"))original(INPUT,7);else if(args[1].equals("conflict"))originalConflict(INPUT,7);else generated(INPUT,7);}}catch(Throwable t){{escaped=t;}}String out=escaped==null?"none":escaped==handlerFailure?"handler":escaped==error?"error":escaped==oom?"oom":escaped==base?"base":escaped instanceof ExceptionInInitializerError&&escaped.getCause()==failure?"EIIE_cause":escaped instanceof OutOfMemoryError?"VM_OOM":escaped.getClass().getName();System.out.println(events+":"+(seen==failure)+":"+(seen==checked)+":"+preserved+":"+observed+":"+(published!=null)+":"+out);}}
}}"#
    );
    std::fs::write(dir.join("CrossHarness.java"), java).unwrap();
    let original = r#"import java.lang.classfile.*;import java.lang.constant.*;import java.nio.file.*;public class Original{public static void main(String[]args)throws Exception{ClassFile cf=ClassFile.of();ClassDesc self=ClassDesc.of("CrossHarness"),item=ClassDesc.of("CrossHarness$Item"),exception=ClassDesc.of("java.lang.Exception");Path path=Path.of("CrossHarness.class");Files.write(path,cf.transformClass(cf.parse(path),(cb,e)->{if(e instanceof MethodModel mm&&(mm.methodName().equalsString("original")||mm.methodName().equalsString("originalConflict"))){boolean conflict=mm.methodName().equalsString("originalConflict");cb.transformMethod(mm,(mb,me)->{if(me instanceof CodeModel)mb.withCode(c->{Label a=c.newLabel(),ae=c.newLabel(),b=c.newLabel(),be=c.newLabel(),ha=c.newLabel(),hb=c.newLabel(),join=c.newLabel();c.labelBinding(a).new_(item).astore(2).labelBinding(ae).aconst_null().astore(3).aload(0).astore(4);if(conflict)c.bipush(99);else c.iload(1);c.istore(5);c.labelBinding(b).aload(2).aload(4).iload(5).aload(3).invokespecial(item,"<init>",MethodTypeDesc.ofDescriptor("(Ljava/lang/String;ILjava/lang/Object;)V")).labelBinding(be).return_();c.labelBinding(hb).astore(6).goto_(join);c.labelBinding(ha).astore(6).aload(0).astore(4).iload(1).istore(5).goto_(join);c.labelBinding(join).aload(6).aload(4).iload(5).invokestatic(self,"observe",MethodTypeDesc.ofDescriptor("(Ljava/lang/Exception;Ljava/lang/String;I)V")).return_();c.exceptionCatch(a,ae,ha,exception);c.exceptionCatch(b,be,hb,exception);});else mb.with(me);});}else if(e instanceof MethodModel mm&&mm.methodName().equalsString("unexpected")){cb.transformMethod(mm,(mb,me)->{if(me instanceof CodeModel)mb.withCode(c->c.aload(0).athrow());else mb.with(me);});}else cb.with(e);}));}}"#;
    std::fs::write(dir.join("Original.java"), original).unwrap();
    let home = std::env::var("DECOMPHIFY_JDK25_HOME")
        .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into());
    let run = |tool: &str, args: &[&str]| {
        let p = std::process::Command::new(format!("{home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(p.status.success(), "{}", String::from_utf8_lossy(&p.stderr));
        String::from_utf8(p.stdout).unwrap()
    };
    run(
        "javac",
        &["--release", "25", "CrossHarness.java", "Original.java"],
    );
    run("java", &["Original"]);
    let mut evidence = String::new();
    for (mode, expected) in [
        (0, "AC:false:false:false:-1:true:none\n"),
        (1, "ACHT:true:false:true:7:true:none\n"),
        (2, "AC:false:false:false:-1:true:error\n"),
        (3, "A:false:false:false:-1:false:oom\n"),
        (4, "A:false:false:false:-1:false:EIIE_cause\n"),
        (5, "A:false:false:false:-1:false:VM_OOM\n"),
        (6, "ACH:true:false:true:7:true:handler\n"),
        (7, "ACHT:true:false:true:7:true:handler\n"),
        (8, "ACHT:false:true:true:7:true:none\n"),
        (9, "AC:false:false:false:-1:true:base\n"),
    ] {
        let mode = mode.to_string();
        let before = run(
            "java",
            &[
                "-Xverify:all",
                "-Xmx32m",
                "-cp",
                ".",
                "CrossHarness",
                &mode,
                "original",
            ],
        );
        let after = run(
            "java",
            &[
                "-Xverify:all",
                "-Xmx32m",
                "-cp",
                ".",
                "CrossHarness",
                &mode,
                "source",
            ],
        );
        assert_eq!(before, expected);
        assert_eq!(after, expected);
        evidence.push_str(&format!(
            "mode={mode} ORIGINAL={before:?} GENERATED={after:?}\n"
        ));
    }
    let wrong = run(
        "java",
        &[
            "-Xverify:all",
            "-Xmx32m",
            "-cp",
            ".",
            "CrossHarness",
            "1",
            "conflict",
        ],
    );
    assert_eq!(wrong, "ACHT:true:false:false:99:true:none\n");
    evidence.push_str(&format!("NEGATIVE_CHANGED_SNAPSHOT={wrong:?}\n"));
    std::fs::write(dir.join("runtime-evidence.txt"), evidence).unwrap();
    std::fs::write(
        dir.join("original-bytecode.txt"),
        run("javap", &["-c", "-v", "-p", "CrossHarness"]),
    )
    .unwrap();
    println!("emitted-source runtime evidence {}", dir.display());
}
