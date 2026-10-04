//! Generated ordinary checked-source bridge preserves exact Throwable identity.
#![allow(clippy::unwrap_used)]
use backend_java::{
    DeclarationBindings, JavaField, JavaMethod, JavaResolver, JavaType, SourceResult,
    checked_source_bridge,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    insn::ManagedInsn,
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    ops::InvokeKind,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};
struct Resolver;
impl JavaResolver for Resolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        (r.index() == 0).then(|| JavaMethod {
            owner: "CheckedBridge".into(),
            name: "target".into(),
            parameters: vec![],
            return_type: JavaType::Int,
            is_static: true,
            throws: vec!["java.io.IOException".into()],
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        a == b
            || matches!((a,b),(JavaType::Class(a),JavaType::Class(b)) if a=="java.io.IOException" && matches!(b.as_str(),"java.lang.Exception"|"java.lang.Throwable"))
    }
}
fn fixture() -> (MethodIr<SsaForm>, JavaMethod) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut value = None;
    let call = insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Int));
        value = Some(v);
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(v),
            method: MethodRef::new(0),
            args: vec![],
        }
    });
    let ret = insns.insert(ManagedInsn::Return { value });
    let mut cfg = Cfg::new(call);
    let exit = cfg.add_block(ret);
    cfg.add_edge(cfg.entry(), exit, EdgeKind::Unconditional)
        .unwrap();
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        JavaMethod {
            owner: "CheckedBridge".into(),
            name: "get".into(),
            parameters: vec![],
            return_type: JavaType::Int,
            is_static: true,
            throws: vec![],
        },
    )
}
#[test]
fn emitted_value_assignment_identity_and_unchecked_passthrough() {
    let (ir, caller) = fixture();
    let plan = checked_source_bridge::plan(
        &ir,
        &caller,
        &["java.io.IOException".into()],
        "bridge",
        &Resolver,
    )
    .unwrap()
    .unwrap();
    let SourceResult::Complete { source, .. } =
        checked_source_bridge::emit(&ir, &DeclarationBindings::default(), &Resolver, &plan)
    else {
        panic!("bridge rejected")
    };
    assert!(!source.contains("throws java.io.IOException"));
    assert!(source.contains("catch (java.lang.Exception"));
    assert_eq!(plan.site_count(), 1);
    let dir =
        std::env::temp_dir().join(format!("decomphify-emitted-bridge-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class CheckedBridge {{
{source}
{}
static Throwable problem; static int calls;
static int target() throws java.io.IOException {{ calls++; if(problem instanceof java.io.IOException) throw (java.io.IOException)problem; if(problem instanceof RuntimeException) throw (RuntimeException)problem; if(problem instanceof Error) throw (Error)problem; return 42; }}
public static void main(String[] args) {{ if(get()!=42) throw new AssertionError(); for(Throwable e:new Throwable[]{{new java.io.IOException("primary"),new RuntimeException("unchecked"),new OutOfMemoryError("oom")}}){{ e.addSuppressed(new Exception("suppressed")); StackTraceElement[] stack=e.getStackTrace();problem=e;try{{get();throw new AssertionError();}}catch(Throwable actual){{if(actual!=e || !java.util.Arrays.equals(stack,actual.getStackTrace()) || actual.getSuppressed().length!=1)throw new AssertionError();}}}} if(calls!=4) throw new AssertionError(); }}
}}"#,
        plan.helper_source()
    );
    std::fs::write(dir.join("CheckedBridge.java"), java).unwrap();
    for root in [
        "/home/jtjisgod/tools/jdk-17.0.13+11",
        "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1",
    ] {
        let javac = std::path::Path::new(root).join("bin/javac");
        if !javac.exists() {
            continue;
        }
        let output = std::process::Command::new(javac)
            .args(["-d"])
            .arg(&dir)
            .arg(dir.join("CheckedBridge.java"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = std::process::Command::new(std::path::Path::new(root).join("bin/java"))
            .arg("-cp")
            .arg(&dir)
            .arg("CheckedBridge")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
#[test]
fn missing_runtime_facts_and_mutated_sites_reject() {
    let (ir, caller) = fixture();
    assert!(checked_source_bridge::plan(&ir, &caller, &[], "bridge", &Resolver).is_err());
    let plan = checked_source_bridge::plan(
        &ir,
        &caller,
        &["java.io.IOException".into()],
        "bridge",
        &Resolver,
    )
    .unwrap()
    .unwrap();
    let (id, insn) = ir
        .instructions()
        .iter()
        .find(|(_, i)| matches!(i, ManagedInsn::Invoke { .. }))
        .unwrap();
    let mut target = Resolver.method(MethodRef::new(0)).unwrap();
    assert!(plan.approves_invoke(id, insn, &target, &caller));
    target.throws.clear();
    assert!(!plan.approves_invoke(id, insn, &target, &caller));
    let mut constructor = caller.clone();
    constructor.name = "<init>".into();
    assert!(
        checked_source_bridge::plan(
            &ir,
            &constructor,
            &["java.io.IOException".into()],
            "bridge",
            &Resolver
        )
        .is_err()
    );
}

struct AllocationResolver;
impl JavaResolver for AllocationResolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        if r.index() == 0 {
            Resolver.method(r)
        } else if r.index() == 1 {
            Some(JavaMethod {
                owner: "CheckedBridge.Payload".into(),
                name: "<init>".into(),
                parameters: vec![JavaType::Int],
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            })
        } else {
            None
        }
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        (r.index() == 0).then(|| "CheckedBridge.Payload".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        Resolver.assignable(a, b)
    }
}
fn allocated_fixture(before: bool) -> (MethodIr<SsaForm>, JavaMethod) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut object = None;
    let allocate = |insns: &mut InsnArena, values: &mut ValueArena, object: &mut Option<_>| {
        insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
            *object = Some(v);
            ManagedInsn::NewInstance {
                result: v,
                class: ClassRef::new(0),
            }
        })
    };
    let prior = before.then(|| allocate(&mut insns, &mut values, &mut object));
    let mut value = None;
    let call = insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ManagedType::Int));
        value = Some(v);
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(v),
            method: MethodRef::new(0),
            args: vec![],
        }
    });
    let allocation = prior.unwrap_or_else(|| allocate(&mut insns, &mut values, &mut object));
    let init = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![object.unwrap(), value.unwrap()],
    });
    let ret = insns.insert(ManagedInsn::Return { value });
    let order = if before {
        vec![allocation, call, init, ret]
    } else {
        vec![call, allocation, init, ret]
    };
    let mut cfg = Cfg::new(order[0]);
    let mut last = cfg.entry();
    for id in &order[1..] {
        let next = cfg.add_block(*id);
        cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
        last = next;
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        fixture().1,
    )
}
#[test]
fn unknown_runtime_domain_is_explicit_and_allocations_cannot_consume_bridge_sites() {
    let (ir, caller) = allocated_fixture(false);
    let plan = checked_source_bridge::plan_source_contract(
        &ir,
        &caller,
        None,
        "bridge",
        &AllocationResolver,
    )
    .unwrap()
    .unwrap();
    assert_eq!(plan.runtime_escaping(), None);
    let SourceResult::Complete { source, .. } = checked_source_bridge::emit(
        &ir,
        &DeclarationBindings::default(),
        &AllocationResolver,
        &plan,
    ) else {
        panic!("disjoint source rejected")
    };
    assert_eq!(source.matches("catch (java.lang.Exception").count(), 1);
    assert_eq!(source.matches("CheckedBridge.target()").count(), 1);
    assert!(
        source.find("CheckedBridge.target()").unwrap()
            < source.find("new CheckedBridge.Payload").unwrap()
    );
    // The already-materialized result must remain a local, not a nested call.
    assert!(!source.contains("new CheckedBridge.Payload((int) CheckedBridge.target()"));
    let (consumed, caller) = allocated_fixture(true);
    assert!(matches!(
        checked_source_bridge::emit(
            &consumed,
            &DeclarationBindings::default(),
            &AllocationResolver,
            &plan
        ),
        SourceResult::Unsupported(_)
    ));
    assert!(
        checked_source_bridge::plan_source_contract(
            &consumed,
            &caller,
            None,
            "bridge",
            &AllocationResolver
        )
        .is_err()
    );
    let java = format!(
        "public class CheckedBridge {{\n{source}\n{}\nstatic class Payload {{Payload(int value){{if(value!=42)throw new AssertionError();}}}}\nstatic Throwable problem;static int calls;\n@SuppressWarnings(\"unchecked\") static <T extends Throwable> RuntimeException escape(Throwable t)throws T{{throw(T)t;}}\nstatic class Foreign extends Throwable{{}}\nstatic int target()throws java.io.IOException{{calls++;if(problem!=null)throw CheckedBridge.<RuntimeException>escape(problem);return 42;}}\npublic static void main(String[] args)throws Throwable{{if(get()!=42)throw new AssertionError();for(Throwable e:new Throwable[]{{new java.io.IOException(),new java.sql.SQLException(),new RuntimeException(),new OutOfMemoryError(),new Foreign()}}){{e.addSuppressed(new Exception());StackTraceElement[] stack=e.getStackTrace();problem=e;try{{get();throw new AssertionError();}}catch(Throwable actual){{if(actual!=e||!java.util.Arrays.equals(stack,e.getStackTrace())||e.getSuppressed().length!=1)throw new AssertionError();}}}}if(calls!=6)throw new AssertionError();}}\n}}",
        plan.helper_source()
    );
    let dir = std::env::temp_dir().join(format!(
        "decomphify-source-bridge-allocation-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("CheckedBridge.java"), java).unwrap();
    for jdk in [
        "/home/jtjisgod/tools/jdk-17.0.13+11",
        "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1",
    ] {
        let compiler = std::process::Command::new(format!("{jdk}/bin/javac"))
            .arg("-d")
            .arg(&dir)
            .arg(dir.join("CheckedBridge.java"))
            .output()
            .unwrap();
        assert!(
            compiler.status.success(),
            "{}",
            String::from_utf8_lossy(&compiler.stderr)
        );
        let run = std::process::Command::new(format!("{jdk}/bin/java"))
            .arg("-cp")
            .arg(&dir)
            .arg("CheckedBridge")
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

struct UnprovedResolver(bool);
impl JavaResolver for UnprovedResolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        self.0.then(|| Resolver.method(r)).flatten()
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
#[test]
fn source_contract_requires_genuine_target_and_checked_hierarchy() {
    let (ir, caller) = fixture();
    for resolver in [UnprovedResolver(false), UnprovedResolver(true)] {
        assert!(
            checked_source_bridge::plan_source_contract(&ir, &caller, None, "bridge", &resolver)
                .is_err()
        );
    }
}

#[test]
fn whole_body_retains_allocation_argument_order_and_original_clauses() {
    let (ir, caller) = allocated_fixture(true);
    let plan =
        checked_source_bridge::plan_whole_body(&ir, &caller, None, "bridge", &AllocationResolver)
            .unwrap()
            .unwrap();
    assert!(plan.is_whole_body());
    assert_eq!(plan.original().throws, Vec::<String>::new());
    assert_eq!(plan.runtime_escaping(), None);
    let SourceResult::Complete { source, .. } = checked_source_bridge::emit(
        &ir,
        &DeclarationBindings::default(),
        &AllocationResolver,
        &plan,
    ) else {
        panic!("whole body rejected")
    };
    assert!(!source.lines().next().unwrap().contains(" throws "));
    assert_eq!(source.matches("catch (java.lang.Exception").count(), 1);
    assert!(source.contains("new CheckedBridge.Payload"));
    assert_eq!(source.matches("CheckedBridge.target()").count(), 1);
    let java = format!(
        r#"public class CheckedBridge {{
{source}
{}
static Throwable problem;static Error initProblem;static String trace="";static int allocations;
@SuppressWarnings("unchecked") static <T extends Throwable>RuntimeException escape(Throwable t)throws T{{throw(T)t;}}
static class Foreign extends Throwable{{}}
static class Payload {{static{{trace+="init,";if(initProblem!=null)throw initProblem;}}Payload(int value){{trace+="ctor,";allocations++;if(value!=42)throw new AssertionError();}}}}
static int target()throws java.io.IOException{{trace+="target,";if(problem!=null)throw CheckedBridge.<RuntimeException>escape(problem);return 42;}}
public static void main(String[] args)throws Throwable{{if(args.length!=0){{Error e=new LinkageError("init");initProblem=e;try{{get();throw new AssertionError();}}catch(Throwable actual){{if(actual!=e||!trace.equals("init,"))throw new AssertionError(trace);}}return;}}
if(get()!=42||!trace.equals("init,target,ctor,"))throw new AssertionError(trace);for(Throwable e:new Throwable[]{{new java.io.IOException(),new java.sql.SQLException(),new RuntimeException(),new OutOfMemoryError(),new Foreign()}}){{e.addSuppressed(new Exception());StackTraceElement[] stack=e.getStackTrace();trace="";problem=e;int before=allocations;try{{get();throw new AssertionError();}}catch(Throwable actual){{if(actual!=e||!java.util.Arrays.equals(stack,e.getStackTrace())||e.getSuppressed().length!=1||allocations!=before||!trace.equals("target,"))throw new AssertionError(trace);}}}}
}}
}}"#,
        plan.helper_source()
    );
    let dir = std::env::temp_dir().join(format!("decomphify-whole-body-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("CheckedBridge.java"), java).unwrap();
    for jdk in [
        "/home/jtjisgod/tools/jdk-17.0.13+11",
        "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1",
    ] {
        let compilation = std::process::Command::new(format!("{jdk}/bin/javac"))
            .arg("-d")
            .arg(&dir)
            .arg(dir.join("CheckedBridge.java"))
            .output()
            .unwrap();
        assert!(
            compilation.status.success(),
            "{}",
            String::from_utf8_lossy(&compilation.stderr)
        );
        for args in [vec![], vec!["init"]] {
            let run = std::process::Command::new(format!("{jdk}/bin/java"))
                .arg("-cp")
                .arg(&dir)
                .arg("CheckedBridge")
                .args(args)
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
        }
    }
}

#[test]
fn inserted_constructor_cannot_escape_whole_body_ownership() {
    let (ir, caller) = allocated_fixture(true);
    let plan =
        checked_source_bridge::plan_whole_body(&ir, &caller, None, "bridge", &AllocationResolver)
            .unwrap()
            .unwrap();
    let mut instructions = ir.instructions().clone();
    instructions.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![],
    });
    let changed = MethodIr::<SsaForm>::from_parts(
        Cfg::new(ir.instructions().iter().next().unwrap().0),
        instructions,
        ir.values().clone(),
    );
    assert!(matches!(
        checked_source_bridge::emit(
            &changed,
            &DeclarationBindings::default(),
            &AllocationResolver,
            &plan
        ),
        SourceResult::Unsupported(_)
    ));
}
