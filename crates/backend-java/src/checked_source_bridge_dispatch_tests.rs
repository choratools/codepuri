#![allow(clippy::unwrap_used)]
use super::*;
use crate::exceptions::{OriginalScope, scopes};
use crate::type_compatibility::{Declaration, Hierarchy};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    types::ManagedType,
    value::Value,
};
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
        Some(JavaType::Class("java.lang.NoSuchMethodException".into()))
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.hierarchy
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
        Some(
            if r.index() == 0 {
                "java.lang.Object"
            } else {
                "java.lang.NoSuchMethodException"
            }
            .into(),
        )
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "ReflectionTarget".into(),
            name: "call".into(),
            is_static: true,
            parameters: vec![JavaType::Int],
            return_type: JavaType::Class("java.lang.Object".into()),
            throws: vec![
                "java.lang.NoSuchMethodException".into(),
                "java.lang.IllegalAccessException".into(),
                "java.lang.reflect.InvocationTargetException".into(),
            ],
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        self.hierarchy.assignable(a, b) == crate::type_compatibility::Compatibility::Assignable
    }
}
fn hierarchy() -> Hierarchy {
    let mut h = Hierarchy::default();
    for (n, p) in [
        ("java.lang.Object", None),
        ("java.lang.Throwable", Some("java.lang.Object")),
        ("java.lang.Exception", Some("java.lang.Throwable")),
        (
            "java.lang.ReflectiveOperationException",
            Some("java.lang.Exception"),
        ),
        (
            "java.lang.NoSuchMethodException",
            Some("java.lang.ReflectiveOperationException"),
        ),
        (
            "java.lang.IllegalAccessException",
            Some("java.lang.ReflectiveOperationException"),
        ),
        (
            "java.lang.reflect.InvocationTargetException",
            Some("java.lang.ReflectiveOperationException"),
        ),
        ("java.lang.RuntimeException", Some("java.lang.Exception")),
        ("java.lang.Error", Some("java.lang.Throwable")),
    ] {
        h.insert(Declaration {
            name: n.into(),
            superclass: p.map(Into::into),
            interfaces: vec![],
            is_interface: false,
        })
        .unwrap();
    }
    h
}
fn fixture() -> (MethodIr<SsaForm>, DeclarationBindings, Metadata, JavaMethod) {
    fixture_with_capture(false, false)
}
fn fixture_with_capture(
    use_unknown_capture: bool,
    normal_handler_entry: bool,
) -> (MethodIr<SsaForm>, DeclarationBindings, Metadata, JavaMethod) {
    let mut insns = InsnArena::with_key();
    let first = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(first);
    let entry = cfg.entry();
    let mut values = ValueArena::with_key();
    let initial = insns.insert(ManagedInsn::Return { value: None });
    let arg = values.insert(Value::new(initial, ManagedType::Int));
    insns[initial] = ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![arg],
        effects: EffectFlags::NONE,
    };
    let result = values.insert(Value::new(first, ManagedType::Reference(ClassRef::new(0))));
    insns[first] = ManagedInsn::Invoke {
        result: Some(result),
        kind: InvokeKind::Static,
        method: MethodRef::new(0),
        args: vec![arg],
    };
    let retid = insns.insert(ManagedInsn::Return {
        value: Some(result),
    });
    let ret = cfg.add_block(retid);
    cfg.add_edge(entry, ret, EdgeKind::Unconditional).unwrap();
    let captureid = insns.insert(ManagedInsn::Return { value: None });
    let handler = cfg.add_block(captureid);
    let caught = values.insert(Value::new(
        captureid,
        if use_unknown_capture {
            ManagedType::Unknown
        } else {
            ManagedType::Reference(ClassRef::new(1))
        },
    ));
    insns[captureid] = ManagedInsn::Opaque {
        family_tag: 0x8002,
        reads: vec![],
        writes: vec![caught],
        effects: EffectFlags::NONE,
    };
    cfg.add_edge(entry, handler, EdgeKind::Exception("no".into()))
        .unwrap();
    if normal_handler_entry {
        cfg.add_edge(entry, handler, EdgeKind::Unconditional)
            .unwrap();
    }
    let nullid = insns.insert(ManagedInsn::Return { value: None });
    let nullblock = cfg.add_block(nullid);
    let null = values.insert(Value::new(
        nullid,
        if use_unknown_capture {
            ManagedType::Reference(ClassRef::new(1))
        } else {
            ManagedType::Null
        },
    ));
    insns[nullid] = if use_unknown_capture {
        ManagedInsn::Move {
            result: null,
            value: caught,
        }
    } else {
        ManagedInsn::Const {
            result: null,
            value: managed_ir::ops::Constant::Null,
        }
    };
    cfg.add_edge(handler, nullblock, EdgeKind::Unconditional)
        .unwrap();
    let nullretid = insns.insert(ManagedInsn::Return { value: Some(null) });
    let nullret = cfg.add_block(nullretid);
    cfg.add_edge(nullblock, nullret, EdgeKind::Unconditional)
        .unwrap();
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        DeclarationBindings {
            receiver: None,
            parameters: vec![Some(arg)],
            names: vec![],
        },
        Metadata {
            scope: scopes::RegionScope {
                protected: vec![entry],
                catches: vec![scopes::Catch {
                    key: "no".into(),
                    entry: handler,
                }],
                continuation: None,
            },
            original: OriginalScope {
                ordinal: 0,
                start: 0,
                end: 1,
                handler_offset: 1,
                catches: vec![("no".into(), 2)],
                offsets: HashMap::from([
                    (first, 0),
                    (retid, 1),
                    (captureid, 2),
                    (nullid, 3),
                    (nullretid, 4),
                ]),
            },
            hierarchy: hierarchy(),
        },
        JavaMethod {
            owner: "DispatchBridgeFixture".into(),
            name: "f".into(),
            is_static: true,
            parameters: vec![JavaType::Int],
            return_type: JavaType::Class("java.lang.Object".into()),
            throws: vec![],
        },
    )
}
#[test]
fn original_handler_metadata_types_unknown_capture_before_bridge_planning() {
    let (ir, bindings, metadata, method) = fixture_with_capture(true, false);
    let resolver = Resolver {
        hierarchy: hierarchy(),
    };
    let ordinary =
        crate::contextual_locals::declaration_plan(&ir, &method.return_type, &resolver).unwrap();
    let scoped =
        dispatch::declarations_for_scopes(&ir, &method.return_type, &resolver, &[&metadata])
            .unwrap();
    let captured = ir
        .instructions()
        .iter()
        .find_map(|(_, instruction)| match instruction {
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                writes,
                ..
            } => Some(writes[0]),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        scoped.get(&captured),
        Some(&JavaType::Class("java.lang.NoSuchMethodException".into()))
    );
    assert_ne!(ordinary.get(&captured), scoped.get(&captured));
    let recipe = plan_dispatch_body(
        &ir,
        &method,
        None,
        "identity",
        &resolver,
        &bindings,
        &[&metadata],
    )
    .unwrap()
    .unwrap();
    let source = emit_dispatch_body(&ir, &bindings, &resolver, &recipe, &[&metadata]);
    assert!(
        matches!(source, SourceResult::Complete { .. }),
        "{source:?}"
    );
    // A normal edge into the handler cannot acquire a caught-object type.
    let (malformed, bindings, metadata, method) = fixture_with_capture(true, true);
    assert!(
        plan_dispatch_body(
            &malformed,
            &method,
            None,
            "identity",
            &resolver,
            &bindings,
            &[&metadata]
        )
        .is_err()
    );
}

#[test]
fn original_reflection_scope_is_kept_under_outer_boundary() {
    let (ir, b, mut md, m) = fixture();
    let r = Resolver {
        hierarchy: hierarchy(),
    };
    let recipe = plan_dispatch_body(&ir, &m, None, "identity", &r, &b, &[&md])
        .unwrap()
        .unwrap();
    assert!(matches!(
        super::super::emit(&ir, &b, &r, &recipe),
        SourceResult::Unsupported(_)
    ));
    let output = emit_dispatch_body(&ir, &b, &r, &recipe, &[&md]);
    let SourceResult::Complete { source, .. } = output else {
        panic!("{output:?}");
    };
    assert!(
        source.contains("catch (java.lang.NoSuchMethodException"),
        "{source}"
    );
    assert!(
        source.contains("catch (java.lang.Exception checkedBodyFailure"),
        "{source}"
    );
    assert!(
        source.contains("public static java.lang.Object f(int p0) {"),
        "{source}"
    );
    let dir = std::env::temp_dir().join(format!("dispatch-bridge-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("DispatchBridgeFixture.java"),
        format!(
            "public class DispatchBridgeFixture {{\n{}\n{}\n}}",
            source,
            recipe.helper_source()
        ),
    )
    .unwrap();
    std::fs::write(dir.join("ReflectionTarget.java"), r#"
class ReflectionTarget {
 static final RuntimeException primary = new RuntimeException("primary");
 static Throwable last; static String trace;
 static Object call(int mode) throws NoSuchMethodException,IllegalAccessException,java.lang.reflect.InvocationTargetException {
  trace += "C";
  try {
   String name = new String[]{"good","hidden","bad","absent","nil"}[mode];
   java.lang.reflect.Method method = ReflectionMembers.class.getDeclaredMethod(name);
   Object value = method.invoke(null); trace += "R"; return value;
  } catch (ReflectiveOperationException failure) { last = failure; trace += "F"; throw failure; }
 }
}
class ReflectionMembers {
 public static Object good(){ ReflectionTarget.trace += "G"; return "ok"; }
 private static Object hidden(){ ReflectionTarget.trace += "H"; return "hidden"; }
 public static Object bad(){ ReflectionTarget.trace += "B"; throw ReflectionTarget.primary; }
 public static Object nil(){ ReflectionTarget.trace += "N"; return null; }
}
"#).unwrap();
    std::fs::write(dir.join("Check.java"), r#"
class Check {
 static Object reference(int mode) throws Exception {
  try { return ReflectionTarget.call(mode); } catch (NoSuchMethodException ignored) { return null; }
 }
 public static void main(String[]args) throws Exception {
  for(int mode=0;mode<5;mode++) {
   ReflectionTarget.trace=""; Object expected=null; Class<?> failureClass=null; String expectedTrace;
   try { expected=reference(mode); } catch(Exception failure) { failureClass=failure.getClass(); if(failure!=ReflectionTarget.last) throw new AssertionError("reference identity"); }
   expectedTrace=ReflectionTarget.trace; ReflectionTarget.trace="";
   try { Object got=DispatchBridgeFixture.f(mode); if(failureClass!=null||got!=expected)throw new AssertionError("result"); }
   catch(Throwable failure) {
    if(failureClass==null||failure.getClass()!=failureClass||failure!=ReflectionTarget.last)throw new AssertionError("rethrow identity");
    if(failure instanceof java.lang.reflect.InvocationTargetException && failure.getCause()!=ReflectionTarget.primary)throw new AssertionError("primary cause");
    boolean originalCall=false;
    for(StackTraceElement frame:failure.getStackTrace()) {
     if(frame.getClassName().equals("DispatchBridgeFixture")&&frame.getMethodName().equals("identity"))throw new AssertionError("altered stack origin");
     if(frame.getClassName().equals("ReflectionTarget")&&frame.getMethodName().equals("call"))originalCall=true;
    }
    if(!originalCall)throw new AssertionError("missing original body origin");
   }
   if(!expectedTrace.equals(ReflectionTarget.trace))throw new AssertionError("effect order");
  }
 }
}
"#).unwrap();
    let compile = std::process::Command::new("javac")
        .current_dir(&dir)
        .args([
            "DispatchBridgeFixture.java",
            "ReflectionTarget.java",
            "Check.java",
        ])
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let runtime = std::process::Command::new("java")
        .current_dir(&dir)
        .args(["-cp", ".", "Check"])
        .output()
        .unwrap();
    assert!(
        runtime.status.success(),
        "{}",
        String::from_utf8_lossy(&runtime.stderr)
    );
    assert!(plan_dispatch_body(&ir, &m, None, "identity", &r, &b, &[]).is_err());
    md.original.end = 2;
    assert!(matches!(
        emit_dispatch_body(&ir, &b, &r, &recipe, &[&md]),
        SourceResult::Unsupported(_)
    ));
}
