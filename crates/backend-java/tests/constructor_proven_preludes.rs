//! Proof-gated Java17 pre-super checks and captured own-field stores.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, constructor_prechecks::*,
    constructors::*,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{Constant, InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{Value, ValueId},
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

struct R {
    proof: bool,
    finalizer: bool,
    nonfinal: bool,
    effectful: bool,
    source_level: u16,
    early_fields: bool,
}
fn good() -> R {
    R {
        proof: true,
        finalizer: false,
        nonfinal: false,
        effectful: false,
        source_level: 17,
        early_fields: true,
    }
}
fn object() -> JavaType {
    JavaType::Class("java.lang.Object".into())
}
impl JavaResolver for R {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            match r.index() {
                0 => "Fixture",
                1 => "java.lang.Object",
                _ => "java.lang.String",
            }
            .into(),
        )
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        Some("value".encode_utf16().collect())
    }
    fn field(&self, f: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture".into(),
            name: if f.index() == 0 { "first" } else { "second" }.into(),
            ty: object(),
            is_static: false,
        })
    }
    fn field_access(&self, _: FieldRef) -> Option<u32> {
        Some(0x11)
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        if m.index() < 2 {
            return self.initializer(m).map(|i| i.method);
        }
        Some(JavaMethod {
            owner: "Check".into(),
            name: "check".into(),
            parameters: vec![object(), JavaType::Class("java.lang.String".into())],
            return_type: JavaType::Void,
            is_static: true,
            throws: vec![],
        })
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(9)
    }
}
impl ConstructorResolver for R {
    fn constructor_source_level(&self) -> u16 {
        self.source_level
    }
    fn constructor_early_field_store(&self, _: FieldRef) -> bool {
        self.early_fields
    }
    fn initializer(&self, m: MethodRef) -> Option<Initializer> {
        if m.index() > 1 {
            return None;
        }
        Some(Initializer {
            method: JavaMethod {
                owner: if m.index() == 0 {
                    "Fixture"
                } else {
                    "java.lang.Object"
                }
                .into(),
                name: "<init>".into(),
                parameters: if m.index() == 0 {
                    vec![object(), object()]
                } else {
                    vec![]
                },
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: None,
        })
    }
    fn constructor_prelude_chain(
        &self,
        caller: MethodRef,
        target: MethodRef,
    ) -> Option<InertChainProof> {
        if !self.proof || caller.index() != 0 || target.index() != 1 {
            return None;
        }
        let mut methods = BTreeSet::from([(
            "<init>".into(),
            "(Ljava/lang/Object;Ljava/lang/Object;)V".into(),
        )]);
        if self.finalizer {
            methods.insert(("finalize".into(), "()V".into()));
        }
        let classes = BTreeMap::from([(
            "LFixture;".into(),
            ClassEvidence {
                owner: "LFixture;".into(),
                flags: if self.nonfinal { 1 } else { 0x11 },
                superclass: Some("Ljava/lang/Object;".into()),
                methods,
            },
        )]);
        let identity = ConstructorIdentity {
            owner: "Ljava/lang/Object;".into(),
            descriptor: "()V".into(),
        };
        let root = ObjectRootEvidence {
            declaration: ClassEvidence {
                owner: identity.owner.clone(),
                flags: 1,
                superclass: None,
                methods: BTreeSet::from([("<init>".into(), "()V".into())]),
            },
            constructor: ConstructorEvidence {
                identity: identity.clone(),
                operations: if self.effectful {
                    vec![Operation::Observable, Operation::Return]
                } else {
                    vec![Operation::Return]
                },
                has_control_or_handlers: false,
            },
            finalizer_operations: None,
        };
        prove_inert_chain("LFixture;", &identity, &classes, &BTreeMap::new(), &root).ok()
    }
}
impl TrustedConstructorResolver for R {
    fn constructor_class(&self, name: &str) -> Option<ConstructorClass> {
        Some(ConstructorClass {
            name: name.into(),
            superclass: if name == "Fixture" {
                Some("java.lang.Object".into())
            } else {
                None
            },
            access_flags: if name == "Fixture" { 0x11 } else { 1 },
        })
    }
    fn constructor_access(&self, _: MethodRef) -> Option<u32> {
        Some(1)
    }
    fn declared_constructors(&self, owner: &str) -> Option<Vec<MethodRef>> {
        Some(vec![MethodRef::new(if owner == "Fixture" { 0 } else { 1 })])
    }
}
fn define(
    insns: &mut InsnArena,
    values: &mut ValueArena,
    ty: ManagedType,
    f: impl FnOnce(ValueId) -> ManagedInsn,
) -> ValueId {
    let mut value = None;
    insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ty));
        value = Some(v);
        f(v)
    });
    value.unwrap()
}
fn body(escape: bool, handler: bool) -> (MethodIr<SsaForm>, ParameterBindings) {
    body_with_check(escape, handler, true)
}
fn body_with_check(
    escape: bool,
    handler: bool,
    checks: bool,
) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let mut incoming = |ty| {
        define(&mut insns, &mut values, ty, |v| ManagedInsn::Opaque {
            family_tag: 0,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        })
    };
    let this = incoming(ManagedType::Reference(ClassRef::new(0)));
    let a = incoming(ManagedType::Reference(ClassRef::new(1)));
    let b = incoming(ManagedType::Reference(ClassRef::new(1)));
    let label = define(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(2)),
        |result| ManagedInsn::Const {
            result,
            value: Constant::String(StringRef::new(0)),
        },
    );
    let entry = values.get(label).unwrap().def();
    let check = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(2),
        args: vec![if escape { this } else { a }, label],
    });
    let first = insns.insert(ManagedInsn::FieldPut {
        object: Some(this),
        field: FieldRef::new(0),
        value: a,
    });
    let second = insns.insert(ManagedInsn::FieldPut {
        object: Some(this),
        field: FieldRef::new(1),
        value: b,
    });
    let chain = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![this],
    });
    let ret = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(if checks { entry } else { first });
    let mut prev = cfg.entry();
    let remaining = if checks {
        vec![check, first, second, chain, ret]
    } else {
        vec![second, chain, ret]
    };
    for id in remaining {
        let next = cfg.add_block(id);
        cfg.add_edge(prev, next, EdgeKind::Unconditional).unwrap();
        prev = next;
    }
    if handler {
        let caught = cfg.add_block(ret);
        cfg.add_edge(
            cfg.entry(),
            caught,
            EdgeKind::Exception("java.lang.Throwable".into()),
        )
        .unwrap();
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        ParameterBindings {
            receiver: Some(this),
            parameters: vec![a, b],
            names: vec!["value".into(), "capture".into()],
        },
    )
}
#[test]
fn preserves_descriptor_check_frame_and_capture_assignments() {
    let (ir, b) = body(false, false);
    let r = good();
    let plan = plan_constructor(&ir, MethodRef::new(0), "java.lang.Object", &b, &r).unwrap();
    assert_eq!(plan.first_statement, "super();");
    assert_eq!(plan.parameter_names, ["value", "capture"]);
    assert_eq!(plan.relocated_pre_chain_fields.len(), 2);
    assert!(plan.source.find("Check.check").unwrap() < plan.source.find("this.first").unwrap());
    let writes = HashMap::from([(MethodRef::new(0), 1)]);
    let finals = [
        BlankFinal {
            reference: FieldRef::new(0),
            writes: writes.clone(),
        },
        BlankFinal {
            reference: FieldRef::new(1),
            writes,
        },
    ];
    let inputs = [ConstructorInput {
        reference: MethodRef::new(0),
        ir: &ir,
        bindings: &b,
    }];
    assert!(plan_constructor_family("Fixture", &inputs, &finals, &r).is_ok());
}
#[test]
fn captured_fields_without_checks_use_the_same_bounded_proof() {
    let (ir, b) = body_with_check(false, false, false);
    let plan = plan_constructor(&ir, MethodRef::new(0), "java.lang.Object", &b, &good()).unwrap();
    assert_eq!(plan.first_statement, "super();");
    assert_eq!(plan.relocated_pre_chain_fields.len(), 2);
    assert!(!plan.source.contains("Check.check"));
    assert_eq!(plan.parameter_names, ["value", "capture"]);
}
#[test]
fn java25_preserves_prelude_without_reordering_proofs() {
    let (ir, b) = body(false, false);
    // All these facts prohibit Java17 commuting; Java25 retains original order.
    let resolver = R {
        source_level: 25,
        proof: false,
        nonfinal: true,
        finalizer: true,
        effectful: true,
        ..good()
    };
    let plan = plan_constructor(&ir, MethodRef::new(0), "java.lang.Object", &b, &resolver).unwrap();
    // Literal resolution is retained at its original site before the call.
    assert_eq!(plan.prologue.len(), 4);
    assert_eq!(plan.preserved_pre_chain_fields.len(), 2);
    assert!(plan.relocated_pre_chain_fields.is_empty());
    assert!(plan.source.find("Check.check").unwrap() < plan.source.find("super();").unwrap());
    assert!(plan.source.find("this.second").unwrap() < plan.source.find("super();").unwrap());
    let writes = HashMap::from([(MethodRef::new(0), 1)]);
    let finals = [
        BlankFinal {
            reference: FieldRef::new(0),
            writes: writes.clone(),
        },
        BlankFinal {
            reference: FieldRef::new(1),
            writes,
        },
    ];
    let inputs = [ConstructorInput {
        reference: MethodRef::new(0),
        ir: &ir,
        bindings: &b,
    }];
    assert!(plan_constructor_family("Fixture", &inputs, &finals, &resolver).is_ok());
    let unknown_fields = R {
        early_fields: false,
        ..resolver
    };
    assert!(
        plan_constructor(
            &ir,
            MethodRef::new(0),
            "java.lang.Object",
            &b,
            &unknown_fields
        )
        .is_err()
    );
}
#[test]
fn java25_early_receiver_uses_and_handlers_still_reject() {
    let resolver = R {
        source_level: 25,
        proof: false,
        ..good()
    };
    for (escape, handler) in [(true, false), (false, true)] {
        let (ir, b) = body(escape, handler);
        assert!(
            plan_constructor(&ir, MethodRef::new(0), "java.lang.Object", &b, &resolver).is_err()
        );
    }
    let (ir, b) = body(false, false);
    let mut insns = ir.instructions().clone();
    let mut values = ir.values().clone();
    let store = insns
        .iter()
        .find_map(|(id, insn)| matches!(insn, ManagedInsn::FieldPut { .. }).then_some(id))
        .unwrap();
    let result = values.insert(Value::new(store, ManagedType::Reference(ClassRef::new(1))));
    insns[store] = ManagedInsn::FieldGet {
        result,
        object: b.receiver,
        field: FieldRef::new(0),
    };
    let read = MethodIr::<SsaForm>::from_parts(copy_linear_cfg(&ir), insns, values);
    assert!(plan_constructor(&read, MethodRef::new(0), "java.lang.Object", &b, &resolver).is_err());
    let mut insns = ir.instructions().clone();
    let check = insns
        .iter()
        .find_map(|(id, insn)| {
            matches!(
                insn,
                ManagedInsn::Invoke {
                    kind: InvokeKind::Static,
                    ..
                }
            )
            .then_some(id)
        })
        .unwrap();
    insns[check] = ManagedInsn::Invoke {
        kind: InvokeKind::Virtual,
        result: None,
        method: MethodRef::new(2),
        args: vec![b.receiver.unwrap()],
    };
    let call = MethodIr::<SsaForm>::from_parts(copy_linear_cfg(&ir), insns, ir.values().clone());
    assert!(plan_constructor(&call, MethodRef::new(0), "java.lang.Object", &b, &resolver).is_err());
}
fn copy_linear_cfg(ir: &MethodIr<SsaForm>) -> Cfg<managed_ir::value::InsnId> {
    let old = ir.cfg();
    let mut source = old.entry();
    let mut cfg = Cfg::new(*old.block(source).unwrap().payload());
    let mut target = cfg.entry();
    loop {
        let node = old.block(source).unwrap();
        if node.outgoing().is_empty() {
            break;
        }
        assert_eq!(node.outgoing().len(), 1);
        let edge = old.edge(node.outgoing()[0]).unwrap();
        assert_eq!(*edge.kind(), EdgeKind::Unconditional);
        source = edge.target();
        let next = cfg.add_block(*old.block(source).unwrap().payload());
        cfg.add_edge(target, next, EdgeKind::Unconditional).unwrap();
        target = next;
    }
    cfg
}
#[test]
fn missing_finalizer_effectful_and_receiver_escape_proofs_deny() {
    let (ir, b) = body(false, false);
    for r in [
        R {
            proof: false,
            ..good()
        },
        R {
            finalizer: true,
            ..good()
        },
        R {
            nonfinal: true,
            ..good()
        },
        R {
            effectful: true,
            ..good()
        },
    ] {
        assert!(plan_constructor(&ir, MethodRef::new(0), "java.lang.Object", &b, &r).is_err());
    }
    for (escape, handler) in [(true, false), (false, true)] {
        let (ir, b) = body(escape, handler);
        assert!(plan_constructor(&ir, MethodRef::new(0), "java.lang.Object", &b, &good()).is_err());
    }
}

#[test]
fn jvm_original_pre_super_bytecode_matches_emitted_checks_and_captures() {
    runtime_compare(None);
}
#[test]
fn java25_jvm_preserves_original_order_including_parent_effects_and_finalizers() {
    let home = std::env::var("DECOMPHIFY_JDK25_HOME")
        .unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-25".into());
    if !std::path::Path::new(&home)
        .join("bin")
        .join("javac")
        .exists()
    {
        eprintln!("skipping JDK25 runtime comparison: {home} is unavailable");
        return;
    }
    runtime_compare(Some(home));
}
fn runtime_compare(jdk25: Option<String>) {
    let (ir, b) = body(false, false);
    let plan = plan_constructor(&ir, MethodRef::new(0), "java.lang.Object", &b, &good()).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "constructor-proven-prelude-{}-{}",
        std::process::id(),
        if jdk25.is_some() { 25 } else { 17 }
    ));
    let original = dir.join("original");
    let lowered = dir.join("lowered");
    std::fs::create_dir_all(&original).unwrap();
    std::fs::create_dir_all(&lowered).unwrap();
    let main = r#"
 public static void main(String[] args) {
  Object a=new Object(),b=new Object();
  Fixture c=new Fixture(a,b);
  if(c.first!=a||c.second!=b)throw new AssertionError("capture changed");
  System.out.println(Trace.log);
  Trace.log="";
  try { new Fixture(null,b);throw new AssertionError("missing check"); }
  catch(IllegalArgumentException e) {System.out.println(Trace.log+"|"+e.getMessage());}
 }
}
class Trace {static String log="";}
class Check {
 static {Trace.log+="C";}
 public static void check(Object a,String name) {
  Trace.log+=name;
  if(a==null) {
   StackTraceElement caller=new Throwable().getStackTrace()[1];
   throw new IllegalArgumentException(caller.getClassName()+"."+caller.getMethodName()+":"+name);
  }
 }
}
"#;
    let source = format!(
        "public final class Fixture {{public final Object first,second;\n{}{}",
        plan.source, main
    );
    let plan25 = plan_constructor(
        &ir,
        MethodRef::new(0),
        "java.lang.Object",
        &b,
        &R {
            source_level: 25,
            proof: false,
            ..good()
        },
    )
    .unwrap();
    std::fs::write(lowered.join("Fixture.java"), &source).unwrap();
    // Produce genuine JVM-valid original bytecode with check + own putfield
    // before Object.<init>, which Java17 source cannot express. The call itself
    // stays in Fixture.<init>; no wrapper changes the stack-sensitive frame.
    let rewrite = r#"
import java.nio.file.*;
import jdk.internal.org.objectweb.asm.*;
import jdk.internal.org.objectweb.asm.tree.*;
public class OriginalPrelude {
 public static void main(String[] args)throws Exception {
  ClassNode c=new ClassNode();
  new ClassReader(Files.readAllBytes(Path.of(args[0]))).accept(c,0);
  for(MethodNode m:c.methods) if(m.name.equals("<init>")) {
   MethodInsnNode parent=null;
   for(AbstractInsnNode n:m.instructions.toArray())
    if(n instanceof MethodInsnNode call&&call.getOpcode()==Opcodes.INVOKESPECIAL&&call.name.equals("<init>")) {parent=call;break;}
   AbstractInsnNode receiver=parent.getPrevious();
   if(receiver.getOpcode()!=Opcodes.ALOAD)throw new AssertionError("noncanonical parent receiver");
   m.instructions.remove(receiver);m.instructions.remove(parent);
   AbstractInsnNode ret=null;
   for(AbstractInsnNode n:m.instructions.toArray())if(n.getOpcode()==Opcodes.RETURN){ret=n;break;}
   m.instructions.insertBefore(ret,receiver);m.instructions.insertBefore(ret,parent);
  }
  ClassWriter writer=new ClassWriter(ClassWriter.COMPUTE_MAXS);c.accept(writer);
  Files.write(Path.of(args[1]),writer.toByteArray());
 }
}
"#;
    std::fs::write(dir.join("OriginalPrelude.java"), rewrite).unwrap();
    let bin = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
    let run = |tool: &str, args: &[&str], cwd: &std::path::Path| {
        let result = std::process::Command::new(format!("{bin}/{tool}"))
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        result.stdout
    };
    let compare25 = |variant: &str, opens: bool| {
        if let Some(home) = &jdk25 {
            let path = dir.join("java25");
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(
                path.join("Fixture.java"),
                variant.replace(&plan.source, &plan25.source),
            )
            .unwrap();
            let run25 = |tool: &str, args: &[&str], cwd: &std::path::Path| {
                let result =
                    std::process::Command::new(std::path::Path::new(home).join("bin").join(tool))
                        .args(args)
                        .current_dir(cwd)
                        .output()
                        .unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                result.stdout
            };
            run25("javac", &["--release", "25", "Fixture.java"], &path);
            let args = if opens {
                vec![
                    "-Xverify:all",
                    "-Xms64m",
                    "-Xmx64m",
                    "--add-opens",
                    "java.base/java.lang.ref=ALL-UNNAMED",
                    "-cp",
                    ".",
                    "Fixture",
                ]
            } else {
                vec!["-Xverify:all", "-cp", ".", "Fixture"]
            };
            // Same original classfile and same JDK25 runtime isolate lowering
            // from any unrelated platform-version behavior difference.
            assert_eq!(run25("java", &args, &original), run25("java", &args, &path));
        }
    };
    run("javac", &["--release", "17", "Fixture.java"], &lowered);
    let exports = [
        "--add-exports",
        "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
        "--add-exports",
        "java.base/jdk.internal.org.objectweb.asm.tree=ALL-UNNAMED",
    ];
    run(
        "javac",
        &[
            exports[0],
            exports[1],
            exports[2],
            exports[3],
            "-source",
            "17",
            "-target",
            "17",
            "OriginalPrelude.java",
        ],
        &dir,
    );
    for class in ["Trace.class", "Check.class"] {
        std::fs::copy(lowered.join(class), original.join(class)).unwrap();
    }
    run(
        "java",
        &[
            exports[0],
            exports[1],
            exports[2],
            exports[3],
            "OriginalPrelude",
            lowered.join("Fixture.class").to_str().unwrap(),
            original.join("Fixture.class").to_str().unwrap(),
        ],
        &dir,
    );
    let before = run("java", &["-Xverify:all", "-cp", ".", "Fixture"], &original);
    let after = run("java", &["-Xverify:all", "-cp", ".", "Fixture"], &lowered);
    assert_eq!(before, after);
    assert_eq!(
        String::from_utf8(before).unwrap(),
        "Cvalue\nvalue|Fixture.<init>:value\n"
    );
    compare25(&source, false);
    // An effectful parent observes both the moved capture and whether a failed
    // check reached construction. This negative runtime oracle is why the
    // evidence contract rejects parent field reads, effects and receiver escape.
    let effectful = source
        .replace("class Fixture {", "class Fixture extends Parent {")
        .replace(
            "System.out.println(Trace.log);",
            "System.out.println(Trace.log+\":\"+(Trace.seen==a));",
        )
        .replace(
            "class Trace {static String log=\"\";}",
            "class Trace {static String log=\"\";static Object seen;}",
        )
        + "class Parent {Parent(){Trace.log+=\"P\";Trace.seen=((Fixture)this).first;}}";
    std::fs::write(lowered.join("Fixture.java"), &effectful).unwrap();
    run("javac", &["--release", "17", "Fixture.java"], &lowered);
    for class in ["Trace.class", "Check.class", "Parent.class"] {
        std::fs::copy(lowered.join(class), original.join(class)).unwrap();
    }
    run(
        "java",
        &[
            exports[0],
            exports[1],
            exports[2],
            exports[3],
            "OriginalPrelude",
            lowered.join("Fixture.class").to_str().unwrap(),
            original.join("Fixture.class").to_str().unwrap(),
        ],
        &dir,
    );
    let before = run("java", &["-Xverify:all", "-cp", ".", "Fixture"], &original);
    let after = run("java", &["-Xverify:all", "-cp", ".", "Fixture"], &lowered);
    assert_eq!(
        String::from_utf8(before).unwrap(),
        "CvalueP:true\nvalue|Fixture.<init>:value\n"
    );
    assert_eq!(
        String::from_utf8(after).unwrap(),
        "PCvalue:false\nPvalue|Fixture.<init>:value\n"
    );
    compare25(&effectful, false);
    // Inspect JDK17's registration list directly instead of relying on GC
    // timing. A failed pre-super check is not finalizer-eligible; moving it
    // after Object.<init> registers another partially initialized object.
    let finalizing = source
        .replace("main(String[] args)", "main(String[] args)throws Exception")
        .replace("System.out.println(Trace.log+\"|\"+e.getMessage());", "System.out.println(Trace.log+\"|\"+e.getMessage());System.out.println(\"registered=\"+registered());")
        .replace("\n}\nclass Trace", r#"
 protected void finalize() {Trace.log+="F";}
 static int registered()throws Exception {
  Class<?> type=Class.forName("java.lang.ref.Finalizer");
  java.lang.reflect.Field head=type.getDeclaredField("unfinalized"),next=type.getDeclaredField("next");
  java.lang.reflect.Field referent=java.lang.ref.Reference.class.getDeclaredField("referent");
  head.setAccessible(true);next.setAccessible(true);referent.setAccessible(true);
  int count=0,budget=10000;
  for(Object node=head.get(null);node!=null;node=next.get(node)) {
   if(--budget==0)throw new AssertionError("finalizer cycle");
   if(referent.get(node) instanceof Fixture)count++;
  }
  return count;
 }
}
class Trace"#);
    std::fs::write(lowered.join("Fixture.java"), &finalizing).unwrap();
    run("javac", &["--release", "17", "Fixture.java"], &lowered);
    for class in ["Trace.class", "Check.class"] {
        std::fs::copy(lowered.join(class), original.join(class)).unwrap();
    }
    run(
        "java",
        &[
            exports[0],
            exports[1],
            exports[2],
            exports[3],
            "OriginalPrelude",
            lowered.join("Fixture.class").to_str().unwrap(),
            original.join("Fixture.class").to_str().unwrap(),
        ],
        &dir,
    );
    let args = [
        "-Xverify:all",
        "-Xms64m",
        "-Xmx64m",
        "--add-opens",
        "java.base/java.lang.ref=ALL-UNNAMED",
        "-cp",
        ".",
        "Fixture",
    ];
    let before = run("java", &args, &original);
    let after = run("java", &args, &lowered);
    assert_eq!(
        String::from_utf8(before).unwrap(),
        "Cvalue\nvalue|Fixture.<init>:value\nregistered=1\n"
    );
    assert_eq!(
        String::from_utf8(after).unwrap(),
        "Cvalue\nvalue|Fixture.<init>:value\nregistered=2\n"
    );
    compare25(&finalizing, true);
}
