//! Definite-assignment evidence must inspect branches and failed stores/invokes.
#![allow(clippy::unwrap_used)]
use backend_java::constructors::*;
use backend_java::{DeclarationBindings, JavaField, JavaMethod, JavaResolver, JavaType};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::{Value, ValueId},
};
use std::collections::HashMap;

struct Resolver {
    cycle: bool,
    source_level: u16,
    early: bool,
}
impl JavaResolver for Resolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        self.initializer(r).map(|i| i.method)
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture".into(),
            name: "number".into(),
            ty: JavaType::Int,
            is_static: false,
        })
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Fixture".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
impl ConstructorResolver for Resolver {
    fn constructor_source_level(&self) -> u16 {
        self.source_level
    }
    fn constructor_early_field_store(&self, _: FieldRef) -> bool {
        self.early
    }
    fn initializer(&self, r: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: if r.index() == 3 { "Base" } else { "Fixture" }.into(),
                name: "<init>".into(),
                parameters: vec![
                    JavaType::Int;
                    if r.index() == 3 {
                        1
                    } else {
                        r.index() as usize + 1
                    }
                ],
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: if self.cycle && r.index() == 0 {
                Some(MethodRef::new(1))
            } else {
                (r.index() == 1).then_some(MethodRef::new(0))
            },
        })
    }
}
impl TrustedConstructorResolver for Resolver {
    fn constructor_class(&self, name: &str) -> Option<ConstructorClass> {
        Some(ConstructorClass {
            name: name.into(),
            superclass: match name {
                "Fixture" => Some("Base".into()),
                "Base" => Some("java.lang.Object".into()),
                "java.lang.Object" => None,
                _ => return None,
            },
            access_flags: 1,
        })
    }
    fn constructor_access(&self, _: MethodRef) -> Option<u32> {
        Some(1)
    }
    fn declared_constructors(&self, _: &str) -> Option<Vec<MethodRef>> {
        Some(vec![MethodRef::new(0), MethodRef::new(1)])
    }
}
fn parameter(insns: &mut InsnArena, values: &mut ValueArena, ty: ManagedType) -> ValueId {
    let mut value = None;
    insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ty));
        value = Some(v);
        ManagedInsn::Opaque {
            family_tag: 0,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        }
    });
    value.unwrap()
}
fn invocation(receiver: ValueId, p: ValueId, method: u32) -> ManagedInsn {
    ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        method: MethodRef::new(method),
        args: vec![receiver, p],
        result: None,
    }
}
fn store(receiver: ValueId, p: ValueId) -> ManagedInsn {
    ManagedInsn::FieldPut {
        field: FieldRef::new(0),
        object: Some(receiver),
        value: p,
    }
}

fn body(target: u32, writes: usize, arity: usize) -> (MethodIr<SsaForm>, DeclarationBindings) {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let receiver = parameter(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(0)),
    );
    let p = parameter(&mut insns, &mut values, ManagedType::Int);
    let init = insns.insert(invocation(receiver, p, target));
    let mut cfg = Cfg::new(init);
    let mut tail = cfg.entry();
    for _ in 0..writes {
        let node = cfg.add_block(insns.insert(store(receiver, p)));
        cfg.add_edge(tail, node, EdgeKind::Unconditional).unwrap();
        tail = node;
    }
    let end = cfg.add_block(insns.insert(ManagedInsn::Return { value: None }));
    cfg.add_edge(tail, end, EdgeKind::Unconditional).unwrap();
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        DeclarationBindings {
            receiver: Some(receiver),
            parameters: (0..arity).map(|i| (i == 0).then_some(p)).collect(),
            names: vec![],
        },
    )
}

#[test]
fn allocation_default_is_entry_recipe_only_in_complete_java25_family() {
    let (root, rb) = body(3, 0, 1);
    let (caller, cb) = body(0, 0, 2);
    let inputs = [
        ConstructorInput {
            reference: MethodRef::new(0),
            ir: &root,
            bindings: &rb,
        },
        ConstructorInput {
            reference: MethodRef::new(1),
            ir: &caller,
            bindings: &cb,
        },
    ];
    let finals = [BlankFinal {
        reference: FieldRef::new(0),
        writes: HashMap::new(),
    }];
    let resolver = Resolver {
        cycle: false,
        source_level: 25,
        early: true,
    };
    let plans = plan_constructor_family("Fixture", &inputs, &finals, &resolver).unwrap();
    assert!(
        plans[0]
            .source
            .contains("{\n    this.number = 0;\n    super(")
    );
    assert_eq!(plans[0].allocation_default_finals, vec![FieldRef::new(0)]);
    assert!(plans[0].preserved_pre_chain_fields.is_empty());
    assert!(plans[0].relocated_pre_chain_fields.is_empty());
    assert!(plans[1].allocation_default_finals.is_empty());
    assert!(plans[1].prologue.is_empty());
    for resolver in [
        Resolver {
            cycle: false,
            source_level: 17,
            early: true,
        },
        Resolver {
            cycle: false,
            source_level: 25,
            early: false,
        },
    ] {
        assert!(plan_constructor_family("Fixture", &inputs, &finals, &resolver).is_err());
    }
    assert!(plan_constructor_family("Fixture", &inputs[..1], &finals, &resolver).is_err());
    let contrary = [BlankFinal {
        reference: FieldRef::new(0),
        writes: [(MethodRef::new(0), 1)].into(),
    }];
    assert!(plan_constructor_family("Fixture", &inputs, &contrary, &resolver).is_err());
}

#[test]
fn actual_nonzero_store_is_never_erased_by_default_recipe() {
    let (root, rb) = body(3, 1, 1);
    let (caller, cb) = body(0, 0, 2);
    let inputs = [
        ConstructorInput {
            reference: MethodRef::new(0),
            ir: &root,
            bindings: &rb,
        },
        ConstructorInput {
            reference: MethodRef::new(1),
            ir: &caller,
            bindings: &cb,
        },
    ];
    let resolver = Resolver {
        cycle: false,
        source_level: 25,
        early: true,
    };
    assert!(
        plan_constructor_family(
            "Fixture",
            &inputs,
            &[BlankFinal {
                reference: FieldRef::new(0),
                writes: HashMap::new()
            }],
            &resolver
        )
        .is_err()
    );
    let plans = plan_constructor_family(
        "Fixture",
        &inputs,
        &[BlankFinal {
            reference: FieldRef::new(0),
            writes: [(MethodRef::new(0), 1)].into(),
        }],
        &resolver,
    )
    .unwrap();
    assert!(plans.iter().all(|p| p.allocation_default_finals.is_empty()));
}

#[test]
#[ignore = "requires JDK25 ClassFile API; byte-derived original no-store constructor"]
fn exact_generated_recipe_matches_original_bytecode_and_preserves_parent_writes() {
    let (root, rb) = body(3, 0, 1);
    let (caller, cb) = body(0, 0, 2);
    let inputs = [
        ConstructorInput {
            reference: MethodRef::new(0),
            ir: &root,
            bindings: &rb,
        },
        ConstructorInput {
            reference: MethodRef::new(1),
            ir: &caller,
            bindings: &cb,
        },
    ];
    let resolver = Resolver {
        cycle: false,
        source_level: 25,
        early: true,
    };
    let plans = plan_constructor_family(
        "Fixture",
        &inputs,
        &[BlankFinal {
            reference: FieldRef::new(0),
            writes: HashMap::new(),
        }],
        &resolver,
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!(
        "decomphify-default-final-oracle-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let fixture = format!(
        "class Fixture extends Base {{ final int number; int peek() {{ return number; }} {} {} }}",
        plans[0].source, plans[1].source
    );
    let harness = r#"
import java.util.*;
public class Harness {
 static final RuntimeException ARG=new RuntimeException("arg");static final Error PARENT=new AssertionError("parent");static final List<String> TRACE=new ArrayList<>();
 static int arg(int x){TRACE.add("arg:"+x);if(x<0)throw ARG;return x;}
 interface Make {Base get(int x);}
 static String run(Make m,int x){TRACE.clear();try{Base p=m.get(x);TRACE.add("result:"+p.peek());}catch(Throwable t){if(t!=ARG&&t!=PARENT)throw new AssertionError(t);TRACE.add("throw:"+(t==ARG?"arg":"parent"));}return String.join(",",TRACE);}
 public static void main(String[] a){int n=0;for(int x:new int[]{-1,0,1,2}){String o=run(p->new Original(arg(p)),x),g=run(p->new Fixture(arg(p)),x);if(!o.equals(g))throw new AssertionError(o+" != "+g);n++;}String o=run(p->new Original(arg(p)),1),bad=run(p->new Naive(arg(p)),1);if(o.equals(bad))throw new AssertionError("after-super counterexample missing");TRACE.clear();Base orig=new Original(arg(1),arg(0));String t=String.join(",",TRACE);TRACE.clear();Base gen=new Fixture(arg(1),arg(0));if(!t.equals(String.join(",",TRACE))||orig.peek()!=gen.peek())throw new AssertionError("this chain");System.out.println("cases="+(n+1)+";after-super-denied");}
}
abstract class Base {Base(int x){Harness.TRACE.add("parent-entry");try{var f=getClass().getDeclaredField("number");f.setAccessible(true);if(!java.lang.reflect.Modifier.isFinal(f.getModifiers()))throw new AssertionError("final lost");Harness.TRACE.add("default:"+f.getInt(this));if(x>0)f.setInt(this,7);Harness.TRACE.add("callback:"+peek());}catch(ReflectiveOperationException e){throw new AssertionError(e);}if(x==2)throw Harness.PARENT;Harness.TRACE.add("parent-exit");}abstract int peek();}
class Original extends Base {final int number;Original(int p,int unused){this(p);}Original(int p){super(p);number=0;}int peek(){return number;}}
class Naive extends Base {final int number;Naive(int p){super(p);number=0;}int peek(){return number;}}
"#;
    std::fs::write(dir.join("Harness.java"), format!("{harness}\n{fixture}")).unwrap();
    std::fs::write(dir.join("Transform.java"), r#"import java.lang.classfile.*;import java.lang.constant.*;import java.nio.file.*;
public class Transform{public static void main(String[]args)throws Exception{ClassFile cf=ClassFile.of();Path p=Path.of("Original.class");Files.write(p,cf.transformClass(cf.parse(p),(cb,e)->{if(e instanceof MethodModel m&&m.methodName().equalsString("<init>")&&m.methodType().equalsString("(I)V"))cb.transformMethod(m,(mb,me)->{if(me instanceof CodeModel)mb.withCode(c->c.aload(0).iload(1).invokespecial(ClassDesc.of("Base"),"<init>",MethodTypeDesc.ofDescriptor("(I)V")).return_());else mb.with(me);});else cb.with(e);}));}}"#).unwrap();
    let home = std::env::var("DECOMPHIFY_JDK25_HOME")
        .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into());
    let run = |tool: &str, args: &[&str]| {
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
        String::from_utf8(out.stdout).unwrap()
    };
    run(
        "javac",
        &["--release", "25", "Harness.java", "Transform.java"],
    );
    run("java", &["Transform"]);
    assert_eq!(
        run("java", &["Harness"]).trim(),
        "cases=5;after-super-denied"
    );
    let bytecode = run("javap", &["-c", "-p", "Original", "Fixture"]);
    let (original, generated) = bytecode.split_once("class Fixture").unwrap();
    assert!(!original.contains("putfield"));
    let ctor = generated.split_once("Fixture(int);").unwrap().1;
    assert!(ctor.find("putfield").unwrap() < ctor.find("invokespecial").unwrap());
    std::fs::write(dir.join("bytecode.txt"), bytecode).unwrap();
    eprintln!(
        "exact generated recipe/original JVM oracle: {}",
        dir.display()
    );
}
