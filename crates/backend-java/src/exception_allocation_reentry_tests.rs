//! Failed-iteration values die only after exact successful dynamic definitions.
#![allow(clippy::unwrap_used)]
use super::*;
struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Fixture".into())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "Fixture".into(),
            name: "<init>".into(),
            parameters: vec![],
            return_type: JavaType::Void,
            is_static: false,
            throws: vec![],
        })
    }
}
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    types::ManagedType,
    value::Value,
};

struct Fixture {
    cfg: Cfg<InsnId>,
    insns: InsnArena,
    values: ValueArena,
    handler: BlockId,
    entry_edge: EdgeId,
    stale: ValueId,
    producer: BlockId,
}
impl Fixture {
    fn new() -> Self {
        let mut insns = InsnArena::with_key();
        let mut values = ValueArena::with_key();
        let start = insns.insert(ManagedInsn::Return { value: None });
        let mut cfg = Cfg::new(start);
        let hid = insns.insert(ManagedInsn::Return { value: None });
        let capture = values.insert(Value::new(hid, ManagedType::Int));
        insns[hid] = ManagedInsn::Const {
            result: capture,
            value: managed_ir::ops::Constant::Int(0),
        };
        let handler = cfg.add_block(hid);
        let entry_edge = cfg
            .add_edge(
                cfg.entry(),
                handler,
                EdgeKind::Exception("java.lang.Throwable".into()),
            )
            .unwrap();
        let pid = insns.insert(ManagedInsn::Return { value: None });
        let stale = values.insert(Value::new(pid, ManagedType::Reference(ClassRef::new(0))));
        insns[pid] = ManagedInsn::NewInstance {
            result: stale,
            class: ClassRef::new(0),
        };
        let producer = cfg.add_block(pid);
        cfg.add_edge(handler, producer, EdgeKind::Unconditional)
            .unwrap();
        Self {
            cfg,
            insns,
            values,
            handler,
            entry_edge,
            stale,
            producer,
        }
    }
    fn block(&mut self, insn: ManagedInsn) -> BlockId {
        self.cfg.add_block(self.insns.insert(insn))
    }
    fn check(self, stale: &[ValueId]) -> bool {
        let ir = MethodIr::<SsaForm>::from_parts(self.cfg, self.insns, self.values);
        handler_observes(
            &ir,
            self.handler,
            self.entry_edge,
            &stale.iter().copied().collect(),
            &Resolver,
        )
    }
}
#[test]
fn fresh_new_kills_only_its_own_value_after_success() {
    let mut f = Fixture::new();
    let stale = f.stale;
    let read = f.block(ManagedInsn::Return { value: Some(stale) });
    let init = f.block(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![stale],
    });
    f.cfg
        .add_edge(f.producer, init, EdgeKind::Unconditional)
        .unwrap();
    f.cfg.add_edge(init, read, EdgeKind::Unconditional).unwrap();
    assert!(!f.check(&[stale]));
}
#[test]
fn throwing_definition_does_not_kill_on_exception_edge() {
    let mut f = Fixture::new();
    let stale = f.stale;
    let read = f.block(ManagedInsn::Return { value: Some(stale) });
    f.cfg
        .add_edge(
            f.producer,
            read,
            EdgeKind::Exception("java.lang.Throwable".into()),
        )
        .unwrap();
    assert!(f.check(&[stale]));
}
#[test]
fn reading_before_fresh_definition_is_rejected() {
    let mut f = Fixture::new();
    let stale = f.stale;
    let read = f.block(ManagedInsn::Return { value: Some(stale) });
    let edge = *f.cfg.block(f.handler).unwrap().outgoing().first().unwrap();
    f.cfg.redirect_edge(edge, read).unwrap();
    assert!(f.check(&[stale]));
}
#[test]
fn new_does_not_prematurely_kill_copy_alias() {
    let mut f = Fixture::new();
    let stale = f.stale;
    let id = f.insns.insert(ManagedInsn::Return { value: None });
    let alias = f.values.insert(Value::new(id, ManagedType::Unknown));
    f.insns[id] = ManagedInsn::Move {
        result: alias,
        value: stale,
    };
    let read = f.block(ManagedInsn::Return { value: Some(alias) });
    f.cfg
        .add_edge(f.producer, read, EdgeKind::Unconditional)
        .unwrap();
    assert!(f.check(&[stale, alias]));
}
#[test]
fn phi_cluster_keeps_predecessor_snapshot_until_all_copies() {
    let mut f = Fixture::new();
    let stale = f.stale;
    let freshid = f.insns.insert(ManagedInsn::Return { value: None });
    let fresh = f.values.insert(Value::new(freshid, ManagedType::Int));
    f.insns[freshid] = ManagedInsn::Const {
        result: fresh,
        value: managed_ir::ops::Constant::Int(1),
    };
    // Replace NEW by first Phi defining its stale destination from a fresh input.
    let pid = *f.cfg.block(f.producer).unwrap().payload();
    let edge = *f.cfg.block(f.handler).unwrap().outgoing().first().unwrap();
    f.insns[pid] = ManagedInsn::Phi {
        result: stale,
        inputs: vec![(edge, fresh)],
    };
    let secondid = f.insns.insert(ManagedInsn::Return { value: None });
    let second = f.values.insert(Value::new(secondid, ManagedType::Unknown));
    f.insns[secondid] = ManagedInsn::Phi {
        result: second,
        inputs: vec![(edge, stale)],
    };
    let secondblock = f.cfg.add_block(secondid);
    f.cfg
        .add_edge(f.producer, secondblock, EdgeKind::Unconditional)
        .unwrap();
    assert!(f.check(&[stale]));
}

#[test]
fn fresh_uninitialized_receiver_remains_unavailable_to_handler() {
    let mut f = Fixture::new();
    let stale = f.stale;
    let init = f.block(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![stale],
    });
    f.cfg
        .add_edge(f.producer, init, EdgeKind::Unconditional)
        .unwrap();
    let read = f.block(ManagedInsn::Return { value: Some(stale) });
    f.cfg
        .add_edge(
            init,
            read,
            EdgeKind::Exception("java.lang.Throwable".into()),
        )
        .unwrap();
    assert!(f.check(&[stale]));
}
#[test]
fn unselected_phi_input_does_not_observe_failed_iteration_value() {
    let mut f = Fixture::new();
    let stale = f.stale;
    let pid = *f.cfg.block(f.producer).unwrap().payload();
    let edge = *f.cfg.block(f.handler).unwrap().outgoing().first().unwrap();
    let freshid = *f.cfg.block(f.handler).unwrap().payload();
    let fresh = successful_result(&f.insns[freshid]).unwrap();
    let foreign = f
        .cfg
        .add_edge(f.cfg.entry(), f.producer, EdgeKind::Unconditional)
        .unwrap();
    f.insns[pid] = ManagedInsn::Phi {
        result: stale,
        inputs: vec![(edge, fresh), (foreign, stale)],
    };
    let read = f.block(ManagedInsn::Return { value: Some(stale) });
    f.cfg
        .add_edge(f.producer, read, EdgeKind::Unconditional)
        .unwrap();
    assert!(!f.check(&[stale]));
}

struct LoopResolver;
impl JavaResolver for LoopResolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r.index() == 0 {
                "RetryHarness.Item"
            } else {
                "java.lang.Throwable"
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
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        Some(if r.index() == 0 { 1 } else { 9 })
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(match r.index() {
            0 => JavaMethod {
                owner: "RetryHarness.Item".into(),
                name: "<init>".into(),
                parameters: vec![JavaType::Int],
                return_type: JavaType::Void,
                is_static: false,
                throws: vec!["java.io.IOException".into()],
            },
            1 => JavaMethod {
                owner: "RetryHarness".into(),
                name: "argument".into(),
                parameters: vec![],
                return_type: JavaType::Int,
                is_static: true,
                throws: vec!["java.io.IOException".into()],
            },
            _ => JavaMethod {
                owner: "RetryHarness".into(),
                name: "retry".into(),
                parameters: vec![JavaType::Class("java.lang.Throwable".into())],
                return_type: JavaType::Boolean,
                is_static: true,
                throws: vec![],
            },
        })
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        loop_hierarchy().assignable(a, b) == crate::type_compatibility::Compatibility::Assignable
    }
}
fn loop_hierarchy() -> crate::type_compatibility::Hierarchy {
    use crate::type_compatibility::{Declaration, Hierarchy};
    let mut h = Hierarchy::default();
    for (n, p) in [
        ("java.lang.Object", None),
        ("java.lang.Throwable", Some("java.lang.Object")),
        ("java.lang.Exception", Some("java.lang.Throwable")),
        ("java.lang.RuntimeException", Some("java.lang.Exception")),
        ("java.lang.Error", Some("java.lang.Throwable")),
        ("java.io.IOException", Some("java.lang.Exception")),
        ("RetryHarness.Item", Some("java.lang.Object")),
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
struct LoopMetadata {
    original: OriginalScope,
    scope: crate::exceptions::scopes::RegionScope,
    h: crate::type_compatibility::Hierarchy,
}
impl RegionMetadata for LoopMetadata {
    fn original_scope(&self) -> Option<&OriginalScope> {
        Some(&self.original)
    }
    fn region_scope(&self) -> &crate::exceptions::scopes::RegionScope {
        &self.scope
    }
    fn hierarchy(&self) -> &crate::type_compatibility::Hierarchy {
        &self.h
    }
    fn catch_type(&self, _: &str) -> Option<JavaType> {
        Some(JavaType::Class("java.lang.Throwable".into()))
    }
    fn is_catch_all(&self, _: &str) -> bool {
        true
    }
}
#[test]
fn generated_retry_loop_preserves_allocation_argument_constructor_and_exception_identity() {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let new = insns.insert(ManagedInsn::Return { value: None });
    let object = values.insert(Value::new(new, ManagedType::Reference(ClassRef::new(0))));
    insns[new] = ManagedInsn::NewInstance {
        result: object,
        class: ClassRef::new(0),
    };
    let mut cfg = Cfg::new(new);
    let entry = cfg.entry();
    let arg = insns.insert(ManagedInsn::Return { value: None });
    let number = values.insert(Value::new(arg, ManagedType::Int));
    insns[arg] = ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: Some(number),
        method: MethodRef::new(1),
        args: vec![],
    };
    let argument = cfg.add_block(arg);
    cfg.add_edge(entry, argument, EdgeKind::Unconditional)
        .unwrap();
    let init = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![object, number],
    });
    let initializer = cfg.add_block(init);
    cfg.add_edge(argument, initializer, EdgeKind::Unconditional)
        .unwrap();
    let ret = cfg.add_block(insns.insert(ManagedInsn::Return {
        value: Some(object),
    }));
    cfg.add_edge(initializer, ret, EdgeKind::Unconditional)
        .unwrap();
    let cap = insns.insert(ManagedInsn::Return { value: None });
    let caught = values.insert(Value::new(cap, ManagedType::Reference(ClassRef::new(1))));
    insns[cap] = ManagedInsn::Opaque {
        family_tag: 0x8002,
        reads: vec![],
        writes: vec![caught],
        effects: managed_ir::effects::EffectFlags::MAY_THROW,
    };
    let capture = cfg.add_block(cap);
    let retryid = insns.insert(ManagedInsn::Return { value: None });
    let again = values.insert(Value::new(retryid, ManagedType::Boolean));
    insns[retryid] = ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: Some(again),
        method: MethodRef::new(2),
        args: vec![caught],
    };
    let retry = cfg.add_block(retryid);
    cfg.add_edge(capture, retry, EdgeKind::Unconditional)
        .unwrap();
    let branchid = insns.insert(ManagedInsn::Return { value: None });
    let branch = cfg.add_block(branchid);
    cfg.add_edge(retry, branch, EdgeKind::Unconditional)
        .unwrap();
    let throw = cfg.add_block(insns.insert(ManagedInsn::Throw { value: caught }));
    let yes = cfg.add_edge(branch, entry, EdgeKind::BranchTrue).unwrap();
    let no = cfg.add_edge(branch, throw, EdgeKind::BranchFalse).unwrap();
    insns[branchid] = ManagedInsn::Branch {
        condition: Some(again),
        then_edge: yes,
        else_edge: Some(no),
    };
    for b in [entry, argument, initializer] {
        cfg.add_edge(b, capture, EdgeKind::Exception("all".into()))
            .unwrap();
    }
    let md = LoopMetadata {
        original: OriginalScope {
            ordinal: 0,
            start: 0,
            end: 3,
            handler_offset: 1,
            catches: vec![("all".into(), 10)],
            offsets: HashMap::from([(new, 0), (arg, 1), (init, 2), (cap, 10)]),
        },
        scope: crate::exceptions::scopes::RegionScope {
            protected: vec![entry, argument, initializer],
            catches: vec![crate::exceptions::scopes::Catch {
                key: "all".into(),
                entry: capture,
            }],
            continuation: Some(ret),
        },
        h: loop_hierarchy(),
    };
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    assert!(original_allocation_chains(&ir, &[&md], &LoopResolver).contains(&vec![new, arg, init]));
    let method = JavaMethod {
        owner: "RetryHarness".into(),
        name: "generated".into(),
        parameters: vec![],
        return_type: JavaType::Class("RetryHarness.Item".into()),
        is_static: true,
        throws: vec!["java.lang.Throwable".into()],
    };
    let result = crate::exceptions::dispatch::emit_method(
        &ir,
        &method,
        &crate::DeclarationBindings::default(),
        &LoopResolver,
        &[&md],
    );
    let crate::SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    let dir = std::env::temp_dir().join(format!("allocation-retry-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class RetryHarness{{
static String trace="";static int mode,args,ctors,retries;static Throwable seen;static final java.io.IOException PRIMARY=new java.io.IOException();
static int initialized(){{trace+="N";return 0;}}
static class Item{{static int marker=initialized();Item(int n)throws java.io.IOException{{trace+="I";ctors++;if(mode==2&&ctors==1)throw PRIMARY;}}}}
static int argument()throws java.io.IOException{{trace+="A";args++;if(mode==1&&args==1||mode==3)throw PRIMARY;return 7;}}
static boolean retry(Throwable t){{trace+="C";if(t!=PRIMARY)throw new AssertionError(t);seen=t;return ++retries<2;}}
{source}
static Item reference()throws Throwable{{while(true){{try{{return new Item(argument());}}catch(Throwable t){{if(!retry(t))throw t;}}}}}}
public static void main(String[]argv)throws Throwable{{mode=Integer.parseInt(argv[0]);try{{Item got=argv[1].equals("generated")?generated():reference();if(mode==3||got==null)throw new AssertionError();}}catch(Throwable t){{if(mode!=3||t!=PRIMARY||seen!=PRIMARY)throw t;}}
String expected=mode==0?"NAI":mode==1?"NACAI":mode==2?"NAICAI":"NACAC";if(!trace.equals(expected))throw new AssertionError(trace+" != "+expected);System.out.print(trace);}}
}}"#
    );
    std::fs::write(dir.join("RetryHarness.java"), java).unwrap();
    let home = std::env::var("JAVA_HOME").ok();
    let javac = home
        .as_ref()
        .map(|p| std::path::PathBuf::from(p).join("bin/javac"))
        .unwrap_or_else(|| "javac".into());
    let jvm = home
        .as_ref()
        .map(|p| std::path::PathBuf::from(p).join("bin/java"))
        .unwrap_or_else(|| "java".into());
    let compile = std::process::Command::new(javac)
        .arg(dir.join("RetryHarness.java"))
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    for mode in 0..4 {
        let mut traces = vec![];
        for name in ["generated", "reference"] {
            let run = std::process::Command::new(&jvm)
                .args([
                    "-cp",
                    dir.to_str().unwrap(),
                    "RetryHarness",
                    &mode.to_string(),
                    name,
                ])
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
            traces.push(run.stdout);
        }
        assert_eq!(traces[0], traces[1]);
    }
}

#[test]
fn repeated_new_after_previous_success_remains_pending_with_other_invalid_value() {
    let mut f = Fixture::new();
    let root = f.stale;
    let orphan_id = f.insns.insert(ManagedInsn::Return { value: None });
    let other = f.values.insert(Value::new(orphan_id, ManagedType::Int));
    f.insns[orphan_id] = ManagedInsn::Const {
        result: other,
        value: managed_ir::ops::Constant::Int(3),
    };
    let init = f.block(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![root],
    });
    f.cfg
        .add_edge(f.producer, init, EdgeKind::Unconditional)
        .unwrap();
    let read = f.block(ManagedInsn::Return { value: Some(root) });
    f.cfg
        .add_edge(
            init,
            read,
            EdgeKind::Exception("java.lang.Throwable".into()),
        )
        .unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(f.cfg, f.insns, f.values);
    // Exact transition state after a prior normal initializer: root is available,
    // while an independent delayed value keeps traversal live on loop reentry.
    let tracked = HashSet::from([root, other]);
    let invalid = HashSet::from([other]);
    assert!(handler_observes_from_invalid(
        &ir,
        f.handler,
        f.entry_edge,
        &tracked,
        &invalid,
        &Resolver
    ));
}
