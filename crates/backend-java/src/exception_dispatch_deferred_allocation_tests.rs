#![allow(clippy::unwrap_used)]
use super::*;
use crate::{
    DeclarationBindings, SourceResult,
    type_compatibility::{Declaration, Hierarchy},
};
use decomp_core::cfg::Cfg;
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    ops::InvokeKind,
    refs::{ClassRef, MethodRef},
    types::ManagedType,
    value::Value,
};
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
        let i = self.insns.insert(ManagedInsn::Return { value: None });
        self.cfg.add_block(i)
    }
    fn value(&mut self, b: BlockId, t: ManagedType) -> ValueId {
        self.values
            .insert(Value::new(*self.cfg.block(b).unwrap().payload(), t))
    }
    fn set(&mut self, b: BlockId, i: ManagedInsn) {
        self.insns[*self.cfg.block(b).unwrap().payload()] = i;
    }
    fn incoming(&mut self, t: ManagedType) -> ValueId {
        let id = self.insns.insert(ManagedInsn::Return { value: None });
        let v = self.values.insert(Value::new(id, t));
        self.insns[id] = ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        };
        v
    }
    fn edge(&mut self, a: BlockId, b: BlockId, k: EdgeKind) -> EdgeId {
        self.cfg.add_edge(a, b, k).unwrap()
    }
    fn link(&mut self, a: BlockId, b: BlockId) -> EdgeId {
        self.edge(a, b, EdgeKind::Unconditional)
    }
}

fn hierarchy() -> Hierarchy {
    let mut h = Hierarchy::default();
    for (n, p) in [
        ("java.lang.Object", None),
        ("java.lang.Throwable", Some("java.lang.Object")),
        ("java.lang.Exception", Some("java.lang.Throwable")),
        ("java.lang.RuntimeException", Some("java.lang.Exception")),
        ("java.lang.Error", Some("java.lang.Throwable")),
        ("java.io.IOException", Some("java.lang.Exception")),
        ("AllocationHarness.Item", Some("java.lang.Object")),
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
struct R {
    chain: Vec<InsnId>,
}
impl JavaResolver for R {
    fn exception_allocation_chain(&self, chain: &[InsnId]) -> bool {
        self.chain == chain
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r.index() == 0 {
                "AllocationHarness.Item"
            } else {
                "java.lang.Throwable"
            }
            .into(),
        )
    }
    fn field(&self, _: managed_ir::refs::FieldRef) -> Option<crate::JavaField> {
        None
    }
    fn string(&self, _: managed_ir::refs::StringRef) -> Option<Vec<u16>> {
        None
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        Some(if r.index() == 0 { 1 } else { 9 })
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(if r.index() == 0 {
            JavaMethod {
                owner: "AllocationHarness.Item".into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: vec![JavaType::Int],
                throws: vec!["java.io.IOException".into()],
            }
        } else {
            JavaMethod {
                owner: "AllocationHarness".into(),
                name: "cleanup".into(),
                is_static: true,
                return_type: JavaType::Void,
                parameters: vec![],
                throws: vec![],
            }
        })
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        hierarchy().assignable(a, b) == Compatibility::Assignable
    }
}
struct Md {
    original: crate::exceptions::OriginalScope,
    scope: scopes::RegionScope,
    h: Hierarchy,
}
impl RegionMetadata for Md {
    fn original_scope(&self) -> Option<&crate::exceptions::OriginalScope> {
        Some(&self.original)
    }
    fn region_scope(&self) -> &scopes::RegionScope {
        &self.scope
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.h
    }
    fn catch_type(&self, _: &str) -> Option<JavaType> {
        Some(JavaType::Class("java.lang.Throwable".into()))
    }
    fn is_catch_all(&self, _: &str) -> bool {
        true
    }
}
fn fixture() -> (MethodIr<SsaForm>, Md, DeclarationBindings, InsnId) {
    let mut b = Builder::new();
    let mode = b.incoming(ManagedType::Int);
    let allocation = b.cfg.entry();
    let init = b.block();
    let normal = b.block();
    let handler = b.block();
    let cleanup = b.block();
    let terminal = b.block();
    let value = b.value(allocation, ManagedType::Reference(ClassRef::new(0)));
    b.set(
        allocation,
        ManagedInsn::NewInstance {
            result: value,
            class: ClassRef::new(0),
        },
    );
    b.link(allocation, init);
    b.set(
        init,
        ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(0),
            args: vec![value, mode],
        },
    );
    b.link(init, normal);
    b.set(normal, ManagedInsn::Return { value: Some(value) });
    let caught = b.value(handler, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        handler,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![caught],
            effects: EffectFlags::MAY_THROW,
        },
    );
    b.link(handler, cleanup);
    b.set(
        cleanup,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(1),
            args: vec![],
        },
    );
    b.link(cleanup, terminal);
    b.set(terminal, ManagedInsn::Throw { value: caught });
    b.edge(allocation, handler, EdgeKind::Exception("all".into()));
    b.edge(init, handler, EdgeKind::Exception("all".into()));
    let id = *b.cfg.block(allocation).unwrap().payload();
    let offsets = HashMap::from([
        (id, 0),
        (*b.cfg.block(init).unwrap().payload(), 1),
        (*b.cfg.block(handler).unwrap().payload(), 99),
    ]);
    (
        MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values),
        Md {
            original: crate::exceptions::OriginalScope {
                ordinal: 0,
                start: 0,
                end: 2,
                handler_offset: 1,
                catches: vec![("all".into(), 99)],
                offsets,
            },
            scope: scopes::RegionScope {
                protected: vec![allocation, init],
                continuation: Some(normal),
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: handler,
                }],
            },
            h: hierarchy(),
        },
        DeclarationBindings {
            receiver: None,
            parameters: vec![Some(mode)],
            names: vec![],
        },
        id,
    )
}
fn resolver(ir: &MethodIr<SsaForm>) -> R {
    // This fixture's exact two-instruction original chain has identical ordered
    // catch-all tables and no handler can observe the pending object.
    let start = *ir.cfg().block(ir.cfg().entry()).unwrap().payload();
    let edge = *ir
        .cfg()
        .block(ir.cfg().entry())
        .unwrap()
        .outgoing()
        .iter()
        .find(|e| matches!(ir.cfg().edge(**e).unwrap().kind(), EdgeKind::Unconditional))
        .unwrap();
    let init = *ir
        .cfg()
        .block(ir.cfg().edge(edge).unwrap().target())
        .unwrap()
        .payload();
    R {
        chain: vec![start, init],
    }
}
fn method() -> JavaMethod {
    JavaMethod {
        owner: "AllocationHarness".into(),
        name: "f".into(),
        is_static: true,
        return_type: JavaType::Class("AllocationHarness.Item".into()),
        parameters: vec![JavaType::Int],
        throws: vec!["java.io.IOException".into()],
    }
}
#[test]
fn deferred_source_fragment_keeps_constructor_checked_contract_and_cleanup() {
    let (ir, md, b, _) = fixture();
    let refs = [&md as &dyn RegionMetadata];
    let r = resolver(&ir);
    let result = super::super::emit_method(&ir, &method(), &b, &r, &refs);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    let dir = std::env::temp_dir().join(format!("deferred-allocation-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class AllocationHarness {{
 static String trace=""; static Throwable primary;static boolean replace;
 static int initialize(){{trace+="A";if(primary instanceof Error)throw (Error)primary;return 0;}}
 static class Item{{ static int initialized=initialize();Item(int mode)throws java.io.IOException{{trace+="I";if(primary instanceof java.io.IOException)throw(java.io.IOException)primary;if(primary instanceof RuntimeException)throw(RuntimeException)primary;}}}}
 static void cleanup(){{trace+="C";if(replace)throw new IllegalStateException("replacement");}}
 {source}
 public static void main(String[]args)throws Exception{{int kind=Integer.parseInt(args[0]);replace=Boolean.parseBoolean(args[1]);primary=kind==1?new java.io.IOException():kind==2?new RuntimeException():kind==3?new Error():null;
 try{{f(kind);if(primary!=null)throw new AssertionError();}}catch(Throwable t){{if(replace){{if(!(t instanceof IllegalStateException))throw new AssertionError(t);}}else if(t!=primary)throw new AssertionError(t);}}
 String expected=kind==3?"AC":kind==0?"AI":"AIC";if(!trace.equals(expected))throw new AssertionError(trace);
 }} }}"#
    );
    std::fs::write(dir.join("AllocationHarness.java"), java).unwrap();
    let out = std::process::Command::new("javac")
        .current_dir(&dir)
        .arg("AllocationHarness.java")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for kind in 0..4 {
        for replacement in [false, true] {
            assert!(
                std::process::Command::new("java")
                    .current_dir(&dir)
                    .args([
                        "-cp",
                        ".",
                        "AllocationHarness",
                        &kind.to_string(),
                        &replacement.to_string()
                    ])
                    .status()
                    .unwrap()
                    .success()
            );
        }
    }
}
#[test]
fn token_rejects_nonempty_wrong_identity_scope_and_order() {
    let (ir, md, b, id) = fixture();
    let refs = [&md as &dyn RegionMetadata];
    let bindings = b.parameters.iter().flatten().copied().collect::<Vec<_>>();
    let r = resolver(&ir);
    let mut plan = super::super::plan(&ir, &refs, &bindings, &r, &HashMap::new()).unwrap();
    let mut emitter = crate::Emitter {
        allocation_readable: None,
        object_planning: Default::default(),
        ir: &ir,
        resolver: &r,
        values: HashMap::new(),
        lines: vec![],
        next: 0,
        parameters: HashSet::new(),
        reserved: HashSet::new(),
        current: Some(id),
        hoisted: HashSet::new(),
        declarations: HashMap::new(),
    };
    let insn = ir.instruction(id).unwrap();
    assert!(deferred_allocation(&emitter, &plan, id, insn, 0).is_some());
    emitter.lines.push("actualNewExpression();".into());
    assert!(deferred_allocation(&emitter, &plan, id, insn, 0).is_none());
    emitter.lines.clear();
    emitter.current = None;
    assert!(deferred_allocation(&emitter, &plan, id, insn, 0).is_none());
    emitter.current = Some(id);
    let init = emitter.object_plan(id).unwrap().initializer;
    let block = ir
        .cfg()
        .block_ids()
        .find(|&b| *ir.cfg().block(b).unwrap().payload() == init)
        .unwrap();
    plan.regions.coverage.insert(block, 99);
    assert!(deferred_allocation(&emitter, &plan, id, insn, 0).is_none());
    plan.regions.coverage.insert(block, 0);
    let origin = ir.cfg().entry();
    let exceptional = plan.regions.handlers[&origin][0];
    let normal = *ir
        .cfg()
        .block(origin)
        .unwrap()
        .outgoing()
        .iter()
        .find(|e| matches!(ir.cfg().edge(**e).unwrap().kind(), EdgeKind::Unconditional))
        .unwrap();
    // A corrupted/reordered canonical receipt must not become an attestation,
    // even though the immutable original allocation plan itself remains valid.
    plan.regions
        .handlers
        .insert(origin, vec![exceptional, normal]);
    plan.regions
        .handlers
        .insert(block, vec![normal, exceptional]);
    assert!(deferred_allocation(&emitter, &plan, id, insn, 0).is_none());
    plan.regions.handlers.get_mut(&block).unwrap().clear();
    assert!(deferred_allocation(&emitter, &plan, id, insn, 0).is_none());
}
