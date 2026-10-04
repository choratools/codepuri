//! Executable ownership, release ordering, and original-handler monitor tests.
#![allow(clippy::unwrap_used)]
use super::super::super::*;
use crate::{
    DeclarationBindings, JavaField, SourceResult,
    type_compatibility::{Declaration, Hierarchy},
};
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    ops::InvokeKind,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};

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
                "broad" => "java.lang.RuntimeException",
                "narrow" => "java.lang.ArithmeticException",
                _ => "java.lang.Throwable",
            }
            .into(),
        ))
    }
    fn is_catch_all(&self, key: &str) -> bool {
        matches!(key, "all" | "acquire")
    }
}
fn hierarchy() -> Hierarchy {
    let mut h = Hierarchy::default();
    for (name, parent) in [
        ("java.lang.Object", None),
        ("java.lang.Throwable", Some("java.lang.Object")),
        ("java.lang.Exception", Some("java.lang.Throwable")),
        ("java.lang.RuntimeException", Some("java.lang.Exception")),
        (
            "java.lang.ArithmeticException",
            Some("java.lang.RuntimeException"),
        ),
        ("java.lang.Error", Some("java.lang.Throwable")),
        ("java.io.IOException", Some("java.lang.Exception")),
    ] {
        h.insert(Declaration {
            name: name.into(),
            superclass: parent.map(str::to_owned),
            interfaces: vec![],
            is_interface: false,
        })
        .unwrap();
    }
    h
}
struct Resolver;
impl JavaResolver for Resolver {
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        hierarchy().assignable(a, b) == Compatibility::Assignable
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r.index() == 0 {
                "java.lang.Object"
            } else {
                "java.lang.Throwable"
            }
            .into(),
        )
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "MonitorDispatch".into(),
            name: if r.index() == 0 { "work" } else { "after" }.into(),
            is_static: true,
            return_type: JavaType::Int,
            parameters: vec![JavaType::Class("java.lang.Object".into()), JavaType::Int],
            throws: vec![],
        })
    }
    fn field_access(&self, _: FieldRef) -> Option<u32> {
        Some(8)
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "MonitorDispatch".into(),
            name: "seen".into(),
            ty: JavaType::Int,
            is_static: true,
        })
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
fn method() -> JavaMethod {
    JavaMethod {
        owner: "MonitorDispatch".into(),
        name: "run".into(),
        is_static: true,
        return_type: JavaType::Int,
        parameters: vec![
            JavaType::Class("java.lang.Object".into()),
            JavaType::Class("java.lang.Object".into()),
            JavaType::Int,
        ],
        throws: vec![],
    }
}

fn fixture(bad: u8) -> (MethodIr<SsaForm>, Vec<Metadata>, DeclarationBindings) {
    let mut b = Builder::new();
    let lock = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let other = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let mode = b.incoming(ManagedType::Int);
    let entry = b.cfg.entry();
    b.set(entry, ManagedInsn::MonitorEnter { object: lock });
    let w1 = b.block();
    b.link(entry, w1);
    let v1 = b.value(w1, ManagedType::Int);
    b.set(
        w1,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(v1),
            method: MethodRef::new(0),
            args: vec![lock, mode],
        },
    );
    let w2 = b.block();
    b.link(w1, w2);
    let v2 = b.value(w2, ManagedType::Int);
    b.set(
        w2,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(v2),
            method: MethodRef::new(0),
            args: vec![lock, v1],
        },
    );
    let exit = b.block();
    b.link(w2, exit);
    b.set(exit, ManagedInsn::MonitorExit { object: lock });
    let ret = b.block();
    b.link(exit, ret);
    b.set(ret, ManagedInsn::Return { value: Some(v2) });
    let h1 = b.block();
    let h2 = b.block();
    let e1 = b.edge(w1, h1, EdgeKind::Exception("all".into()));
    let e2 = b.edge(w2, h2, EdgeKind::Exception("all".into()));
    let cap1 = b.block();
    b.link(h1, cap1);
    let c1 = b.value(cap1, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        cap1,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![c1],
            effects: EffectFlags::MAY_THROW,
        },
    );
    let c2 = b.value(h2, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        h2,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![c2],
            effects: EffectFlags::MAY_THROW,
        },
    );
    let shared = b.block();
    let a = b.link(cap1, shared);
    let z = b.link(h2, shared);
    let primary = b.value(shared, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        shared,
        ManagedInsn::Phi {
            result: primary,
            inputs: vec![(a, c1), (z, c2)],
        },
    );
    let lp = b.block();
    b.link(shared, lp);
    let lk = b.value(lp, ManagedType::Reference(ClassRef::new(0)));
    let cycle = b.value(h1, ManagedType::Reference(ClassRef::new(0)));
    b.set(
        lp,
        ManagedInsn::Phi {
            result: lk,
            inputs: vec![(a, cycle), (z, lock)],
        },
    );
    let release = b.block();
    if bad != 3 {
        b.link(lp, release);
    }
    b.set(
        release,
        ManagedInsn::MonitorExit {
            object: if bad == 1 { other } else { lk },
        },
    );
    let selfe = b.edge(release, h1, EdgeKind::Exception("all".into()));
    b.set(
        h1,
        ManagedInsn::Phi {
            result: cycle,
            inputs: vec![(e1, lock), (selfe, lk)],
        },
    );
    let throw = b.block();
    b.link(release, throw);
    b.set(throw, ManagedInsn::Throw { value: primary });
    if bad == 2 {
        b.set(throw, ManagedInsn::Throw { value: c1 });
    }
    if bad == 3 {
        let side = b.block();
        b.link(lp, side);
        b.set(
            side,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: None,
                method: MethodRef::new(0),
                args: vec![lock, mode],
            },
        );
        b.link(side, release);
    }
    if bad == 4 {
        b.link(entry, shared);
    }

    let md = vec![
        Metadata {
            scope: scopes::RegionScope {
                protected: vec![w1],
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: h1,
                }],
                continuation: None,
            },
            hierarchy: hierarchy(),
        },
        Metadata {
            scope: scopes::RegionScope {
                protected: vec![w2],
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: h2,
                }],
                continuation: None,
            },
            hierarchy: hierarchy(),
        },
        Metadata {
            scope: scopes::RegionScope {
                protected: vec![release],
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: h1,
                }],
                continuation: None,
            },
            hierarchy: hierarchy(),
        },
    ];
    let _ = e2;
    (
        b.finish(),
        md,
        DeclarationBindings {
            parameters: vec![Some(lock), Some(other), Some(mode)],
            ..Default::default()
        },
    )
}
#[test]
fn shared_capture_selected_phi_and_cyclic_owned_lock() {
    let (ir, md, bindings) = fixture(0);
    let tables: Vec<_> = md.iter().map(|x| x as &dyn RegionMetadata).collect();
    let source = match emit_method(&ir, &method(), &bindings, &Resolver, &tables) {
        SourceResult::Complete { source, .. } => source,
        e => panic!("{e:?}"),
    };
    assert!(source.contains("synchronized ("));
    assert!(!source.contains("throws java.lang.Throwable"));
    let java = format!(
        r#"public class MonitorDispatch {{
 static int count, fail; static Throwable primary; static Object lock;
 @SuppressWarnings("unchecked") static <T extends Throwable> void escape(Throwable t) throws T {{throw (T)t;}}
 static int work(Object l,int n) {{if(!Thread.holdsLock(l))throw new AssertionError();if(++count==fail)MonitorDispatch.<RuntimeException>escape(primary);return n+1;}}
 static int after(Object l,int n) {{return n;}} static int seen;
 {source}
 public static void main(String[] args)throws Exception {{
  lock=new Object(); Object other=new Object(); count=0;fail=0;
  if(run(lock,other,4)!=6)throw new AssertionError();
  for(int site=1;site<=2;site++)for(Throwable t:new Throwable[]{{new RuntimeException("r"),new java.io.IOException("c"),new Error("e")}}){{
   primary=t;fail=site;count=0;try{{run(lock,other,4);throw new AssertionError();}}catch(Throwable caught){{if(caught!=t)throw new AssertionError("identity");}}
   if(Thread.holdsLock(lock))throw new AssertionError("leak");
   Thread worker=new Thread(()->{{synchronized(lock){{}}}});worker.start();worker.join(2000);if(worker.isAlive())throw new AssertionError("release");
  }}
 }}
}}"#
    );
    let dir = std::env::temp_dir().join(format!("shared-monitor-runtime-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("MonitorDispatch.java"), java).unwrap();
    for home in [
        "/home/jtjisgod/tools/jdk-17.0.13+11",
        "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1",
    ] {
        let out = std::process::Command::new(format!("{home}/bin/javac"))
            .arg("MonitorDispatch.java")
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let out = std::process::Command::new(format!("{home}/bin/java"))
            .arg("MonitorDispatch")
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
#[test]
fn different_monitor_cannot_gain_shared_rethrow_proof() {
    let (ir, md, bindings) = fixture(1);
    let tables: Vec<_> = md.iter().map(|x| x as &dyn RegionMetadata).collect();
    assert!(!matches!(
        emit_method(&ir, &method(), &bindings, &Resolver, &tables),
        SourceResult::Complete { .. }
    ));
}
#[test]
fn unselected_primary_extra_effect_and_external_entry_reject() {
    for variant in [2, 3, 4] {
        let (ir, md, bindings) = fixture(variant);
        let tables: Vec<_> = md.iter().map(|x| x as &dyn RegionMetadata).collect();
        assert!(
            !matches!(
                emit_method(&ir, &method(), &bindings, &Resolver, &tables),
                SourceResult::Complete { .. }
            ),
            "variant {variant}"
        );
    }
}
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
    fn value(&mut self, b: BlockId, ty: ManagedType) -> ValueId {
        self.values
            .insert(Value::new(*self.cfg.block(b).unwrap().payload(), ty))
    }
    fn set(&mut self, b: BlockId, i: ManagedInsn) {
        self.insns[*self.cfg.block(b).unwrap().payload()] = i;
    }
    fn edge(&mut self, a: BlockId, b: BlockId, kind: EdgeKind) -> EdgeId {
        self.cfg.add_edge(a, b, kind).unwrap()
    }
    fn link(&mut self, a: BlockId, b: BlockId) -> EdgeId {
        self.edge(a, b, EdgeKind::Unconditional)
    }
    fn incoming(&mut self, ty: ManagedType) -> ValueId {
        let id = self.insns.insert(ManagedInsn::Return { value: None });
        let v = self.values.insert(Value::new(id, ty));
        self.insns[id] = ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        };
        v
    }
    fn finish(self) -> MethodIr<SsaForm> {
        MethodIr::<SsaForm>::from_parts(self.cfg, self.insns, self.values)
    }
}
