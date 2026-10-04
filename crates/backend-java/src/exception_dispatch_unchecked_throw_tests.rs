//! Explicit unchecked throws retain their original ordered exception dispatch.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{
    DeclarationBindings, JavaField, JavaResolver, JavaType, SourceResult,
    type_compatibility::{Declaration, Hierarchy},
};
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    ops::Constant,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};

const CLASSES: [&str; 6] = [
    "java.lang.RuntimeException",
    "java.lang.IllegalArgumentException",
    "java.lang.Error",
    "java.io.IOException",
    "UncheckedThrowHarness.Hidden",
    "java.lang.Throwable",
];

fn hierarchy(missing: Option<&str>) -> Hierarchy {
    // Exact JDK declarations, plus the fixture's declared Hidden subclass.
    let mut hierarchy = Hierarchy::default();
    for (name, superclass) in [
        ("java.lang.Object", None),
        ("java.lang.Throwable", Some("java.lang.Object")),
        ("java.lang.Exception", Some("java.lang.Throwable")),
        ("java.lang.RuntimeException", Some("java.lang.Exception")),
        ("java.lang.Error", Some("java.lang.Throwable")),
        (
            "java.lang.NullPointerException",
            Some("java.lang.RuntimeException"),
        ),
        (
            "java.lang.IllegalArgumentException",
            Some("java.lang.RuntimeException"),
        ),
        ("java.io.IOException", Some("java.lang.Exception")),
        (
            "java.lang.InterruptedException",
            Some("java.lang.Exception"),
        ),
        (
            "UncheckedThrowHarness.Hidden",
            Some("java.lang.RuntimeException"),
        ),
    ] {
        if missing == Some(name) {
            continue;
        }
        hierarchy
            .insert(Declaration {
                name: name.into(),
                superclass: superclass.map(Into::into),
                interfaces: vec![],
                is_interface: false,
            })
            .unwrap();
    }
    hierarchy
}

struct Metadata {
    scope: scopes::RegionScope,
    hierarchy: Hierarchy,
}
impl RegionMetadata for Metadata {
    fn region_scope(&self) -> &scopes::RegionScope {
        &self.scope
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.hierarchy
    }
    fn catch_type(&self, key: &str) -> Option<JavaType> {
        Some(JavaType::Class(
            match key {
                "checked" => "java.io.IOException",
                "other_checked" => "java.lang.InterruptedException",
                "narrow" => "java.lang.NullPointerException",
                "broad" => "java.lang.Exception",
                _ => return None,
            }
            .into(),
        ))
    }
    fn is_catch_all(&self, _: &str) -> bool {
        false
    }
}
struct Resolver {
    hierarchy: Hierarchy,
}
impl JavaResolver for Resolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(CLASSES.get(r.index() as usize)?.to_string())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(9)
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "UncheckedThrowHarness".into(),
            name: "observe".into(),
            is_static: true,
            parameters: vec![
                JavaType::Class("java.lang.Throwable".into()),
                JavaType::Int,
                JavaType::Int,
            ],
            return_type: JavaType::Int,
            throws: vec![],
        })
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        self.hierarchy.assignable(actual, expected) == Compatibility::Assignable
    }
}
struct Builder {
    cfg: Cfg<InsnId>,
    insns: InsnArena,
    values: ValueArena,
}
impl Builder {
    fn new() -> Self {
        let mut insns = InsnArena::default();
        let id = insns.insert(ManagedInsn::Return { value: None });
        Self {
            cfg: Cfg::new(id),
            insns,
            values: ValueArena::default(),
        }
    }
    fn block(&mut self) -> BlockId {
        self.cfg
            .add_block(self.insns.insert(ManagedInsn::Return { value: None }))
    }
    fn value(&mut self, block: BlockId, ty: ManagedType) -> ValueId {
        self.values
            .insert(Value::new(*self.cfg.block(block).unwrap().payload(), ty))
    }
    fn set(&mut self, block: BlockId, insn: ManagedInsn) {
        self.insns[*self.cfg.block(block).unwrap().payload()] = insn;
    }
    fn input(&mut self, ty: ManagedType) -> ValueId {
        let id = self.insns.insert(ManagedInsn::Return { value: None });
        let value = self.values.insert(Value::new(id, ty));
        self.insns[id] = ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![value],
            effects: EffectFlags::NONE,
        };
        value
    }
    fn link(&mut self, from: BlockId, to: BlockId) {
        self.cfg
            .add_edge(from, to, EdgeKind::Unconditional)
            .unwrap();
    }
}

struct Fixture {
    ir: MethodIr<SsaForm>,
    metadata: Metadata,
    resolver: Resolver,
    method: JavaMethod,
    bindings: DeclarationBindings,
}
fn fixture(
    name: &str,
    class: u32,
    keys: &[&str],
    missing: Option<&str>,
    bad_snapshot: bool,
) -> Fixture {
    let mut b = Builder::new();
    let exception = b.input(ManagedType::Reference(ClassRef::new(class)));
    let old = b.input(ManagedType::Int);
    let throw = b.cfg.entry();
    b.set(throw, ManagedInsn::Throw { value: exception });
    let mut catches = vec![];
    for (index, key) in keys.iter().enumerate() {
        let join = b.block();
        let copied = b.value(join, ManagedType::Int);
        let edge = b
            .cfg
            .add_edge(throw, join, EdgeKind::Exception((*key).into()))
            .unwrap();
        let capture = b.block();
        let caught = b.value(capture, ManagedType::Unknown);
        b.set(
            capture,
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads: vec![],
                writes: vec![caught],
                effects: EffectFlags::NONE,
            },
        );
        b.link(join, capture);
        let tag_block = b.block();
        let tag = b.value(tag_block, ManagedType::Int);
        b.set(
            tag_block,
            ManagedInsn::Const {
                result: tag,
                value: Constant::Int(index as i32 + 1),
            },
        );
        b.link(capture, tag_block);
        let call = b.block();
        let result = b.value(call, ManagedType::Int);
        b.set(
            call,
            ManagedInsn::Invoke {
                kind: managed_ir::insn::InvokeKind::Static,
                result: Some(result),
                method: MethodRef::new(0),
                args: vec![caught, copied, tag],
            },
        );
        b.link(tag_block, call);
        b.set(
            join,
            ManagedInsn::Phi {
                result: copied,
                inputs: vec![(edge, if bad_snapshot { result } else { old })],
            },
        );
        let ret = b.block();
        b.set(
            ret,
            ManagedInsn::Return {
                value: Some(result),
            },
        );
        b.link(call, ret);
        catches.push(scopes::Catch {
            key: (*key).into(),
            entry: join,
        });
    }
    let h = hierarchy(missing);
    Fixture {
        ir: MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values),
        metadata: Metadata {
            scope: scopes::RegionScope {
                protected: vec![throw],
                catches,
                continuation: None,
            },
            hierarchy: h.clone(),
        },
        resolver: Resolver { hierarchy: h },
        method: JavaMethod {
            owner: "UncheckedThrowHarness".into(),
            name: name.into(),
            is_static: true,
            return_type: JavaType::Int,
            parameters: vec![
                JavaType::Class(CLASSES[class as usize].into()),
                JavaType::Int,
            ],
            // These negative fixtures preserve their declaration's checked contract.
            throws: if class >= 3 {
                vec![CLASSES[class as usize].into()]
            } else {
                vec![]
            },
        },
        bindings: DeclarationBindings {
            parameters: vec![Some(exception), Some(old)],
            ..Default::default()
        },
    }
}
fn emit(f: &Fixture) -> SourceResult {
    emit_method(&f.ir, &f.method, &f.bindings, &f.resolver, &[&f.metadata])
}
fn source(f: &Fixture) -> String {
    match emit(f) {
        SourceResult::Complete { source, .. } => source,
        other => panic!("{other:?}"),
    }
}

#[test]
fn unchecked_throw_dispatch_keeps_original_priority_and_snapshot_copies() {
    for class in [0, 1, 2, 4] {
        let f = fixture("generated", class, &["checked", "narrow"], None, false);
        let s = source(&f);
        assert!(s.contains("catch (java.lang.Throwable"), "{s}");
        assert!(
            s.find("instanceof java.io.IOException").unwrap()
                < s.find("instanceof java.lang.NullPointerException").unwrap()
        );
        assert!(s.contains("snapshot"));
        assert!(s.contains("else { throw "));
    }
    let s = source(&fixture(
        "priority",
        0,
        &["broad", "narrow", "checked"],
        None,
        false,
    ));
    assert!(
        s.find("instanceof java.lang.Exception").unwrap()
            < s.find("instanceof java.lang.NullPointerException").unwrap()
    );
}

#[test]
fn checked_or_unproved_throw_and_unavailable_prethrow_snapshots_do_not_upgrade() {
    for (class, missing) in [
        (3, None),
        (4, Some("UncheckedThrowHarness.Hidden")),
        (1, Some("java.lang.IllegalArgumentException")),
        (0, Some("java.lang.RuntimeException")),
    ] {
        let f = fixture(
            "denied",
            class,
            &["other_checked", "narrow"],
            missing,
            false,
        );
        assert!(
            matches!(emit(&f), SourceResult::Unsupported(_)),
            "class={class} missing={missing:?}: {:?}",
            emit(&f)
        );
        assert!(!proven_unchecked_throw_type(
            &f.method.parameters[0],
            &f.metadata.hierarchy
        ));
    }
    // A broad declared checked contract already admits these native catches;
    // retaining that existing path is distinct from the unchecked extension.
    let broad = fixture(
        "nativeThrowable",
        5,
        &["other_checked", "narrow"],
        None,
        false,
    );
    let native = source(&broad);
    assert!(!proven_unchecked_throw_type(
        &broad.method.parameters[0],
        &broad.metadata.hierarchy
    ));
    assert!(native.contains("catch (java.lang.InterruptedException"));
    assert!(!native.contains("catch (java.lang.Throwable"));
    let f = fixture("badSnapshot", 0, &["checked", "narrow"], None, true);
    assert!(matches!(emit(&f), SourceResult::Unsupported(_)));
}

#[test]
#[ignore = "requires JDK25 ClassFile API; original exception-table runtime oracle"]
fn original_unchecked_athrow_table_matches_source_identity_priority_and_handler_failure() {
    let generated = source(&fixture(
        "generated",
        0,
        &["checked", "narrow"],
        None,
        false,
    ));
    let priority = source(&fixture(
        "priority",
        0,
        &["broad", "narrow", "checked"],
        None,
        false,
    ));
    let error = source(&fixture(
        "errorGenerated",
        2,
        &["checked", "narrow"],
        None,
        false,
    ));
    let inherited = source(&fixture(
        "inheritedGenerated",
        4,
        &["checked", "narrow"],
        None,
        false,
    ));
    let dir = std::env::temp_dir().join(format!("original-unchecked-throw-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let unit = format!(
        r#"public class UncheckedThrowHarness {{
static int mode; static String events=""; static Throwable seen;
static final RuntimeException handlerFailure=new RuntimeException("handler");
static class Hidden extends RuntimeException {{}}
public static int observe(Throwable t,int old,int tag){{seen=t;events+="H"+tag;if(mode==3)throw handlerFailure;return old+tag;}}
static int original(RuntimeException t,int old){{return -1;}}
static int originalPriority(RuntimeException t,int old){{return -1;}}
static int originalError(Error t,int old){{return -1;}}
static int originalInherited(Hidden t,int old){{return -1;}}
{generated}{priority}{error}{inherited}
public static void main(String[]args){{mode=Integer.parseInt(args[0]);boolean lowered=Boolean.parseBoolean(args[1]);Throwable input=mode==1?null:mode==2||mode==3?new NullPointerException("original"):mode==4?new AssertionError("original"):mode==5?new Hidden():new IllegalArgumentException("original");Throwable escaped=null;int result=-1;try{{if(mode==4)result=lowered?errorGenerated((Error)input,7):originalError((Error)input,7);else if(mode==5)result=lowered?inheritedGenerated((Hidden)input,7):originalInherited((Hidden)input,7);else if(mode==2)result=lowered?priority((RuntimeException)input,7):originalPriority((RuntimeException)input,7);else result=lowered?generated((RuntimeException)input,7):original((RuntimeException)input,7);}}catch(Throwable t){{escaped=t;}}System.out.println(result+":"+events+":"+(seen==input)+":"+(escaped==input&&input!=null)+":"+(escaped==handlerFailure)+":"+(seen==null?"none":seen.getClass().getSimpleName()));}}
}}"#
    );
    std::fs::write(dir.join("UncheckedThrowHarness.java"), unit).unwrap();
    let original = r#"import java.lang.classfile.*;import java.lang.constant.*;import java.nio.file.*;
public class Original {public static void main(String[]args)throws Exception{ClassFile cf=ClassFile.of();ClassDesc self=ClassDesc.of("UncheckedThrowHarness");Path p=Path.of("UncheckedThrowHarness.class");Files.write(p,cf.transformClass(cf.parse(p),(cb,e)->{if(e instanceof MethodModel mm&&mm.methodName().stringValue().startsWith("original"))cb.transformMethod(mm,(mb,me)->{if(me instanceof CodeModel)mb.withCode(c->{Label start=c.newLabel(),end=c.newLabel();c.labelBinding(start).aload(0).athrow().labelBinding(end);String[] types=mm.methodName().equalsString("originalPriority")?new String[]{"java.lang.Exception","java.lang.NullPointerException","java.io.IOException"}:new String[]{"java.io.IOException","java.lang.NullPointerException"};for(int i=0;i<types.length;i++){Label handler=c.newLabel();c.labelBinding(handler).astore(2).aload(2).iload(1).ldc(i+1).invokestatic(self,"observe",MethodTypeDesc.ofDescriptor("(Ljava/lang/Throwable;II)I")).ireturn();c.exceptionCatch(start,end,handler,ClassDesc.of(types[i]));}});else mb.with(me);});else cb.with(e);}));}}"#;
    std::fs::write(dir.join("Original.java"), original).unwrap();
    let home = std::env::var("DECOMPHIFY_JDK25_HOME")
        .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into());
    let run = |tool: &str, args: &[&str]| {
        let output = std::process::Command::new(format!("{home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    };
    run(
        "javac",
        &[
            "--release",
            "25",
            "UncheckedThrowHarness.java",
            "Original.java",
        ],
    );
    run("java", &["Original"]);
    let mut evidence = String::new();
    for (mode, expected) in [
        (0, "-1::false:true:false:none\n"),
        (1, "9:H2:false:false:false:NullPointerException\n"),
        (2, "8:H1:true:false:false:NullPointerException\n"),
        (3, "-1:H2:true:false:true:NullPointerException\n"),
        (4, "-1::false:true:false:none\n"),
        (5, "-1::false:true:false:none\n"),
    ] {
        let mode = mode.to_string();
        let before = run(
            "java",
            &[
                "-Xverify:all",
                "-cp",
                ".",
                "UncheckedThrowHarness",
                &mode,
                "false",
            ],
        );
        let after = run(
            "java",
            &[
                "-Xverify:all",
                "-cp",
                ".",
                "UncheckedThrowHarness",
                &mode,
                "true",
            ],
        );
        assert_eq!(String::from_utf8_lossy(&before), expected);
        assert_eq!(before, after);
        evidence.push_str(&format!("mode={mode} original=lowered {expected}"));
    }
    std::fs::write(dir.join("runtime-evidence.txt"), evidence).unwrap();
    eprintln!("original athrow exception-table oracle: {}", dir.display());
}
