//! Executable ownership, release ordering, and original-handler monitor tests.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{
    DeclarationBindings, JavaField, SourceResult,
    type_compatibility::{Declaration, Hierarchy},
};
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    ops::{Constant, InvokeKind},
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

fn fixture(
    reassigned: bool,
    unbalanced: bool,
) -> (MethodIr<SsaForm>, Vec<Metadata>, DeclarationBindings) {
    let mut b = Builder::new();
    let lock = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let other = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let mode = b.incoming(ManagedType::Int);
    let original = b.cfg.entry();
    let alias = b.value(original, ManagedType::Reference(ClassRef::new(0)));
    b.set(
        original,
        ManagedInsn::Move {
            result: alias,
            value: lock,
        },
    );
    let enter = b.block();
    b.link(original, enter);
    b.set(enter, ManagedInsn::MonitorEnter { object: lock });
    let store = b.block();
    b.link(enter, store);
    b.set(
        store,
        ManagedInsn::FieldPut {
            object: None,
            field: FieldRef::new(0),
            value: mode,
        },
    );
    let work = b.block();
    b.link(store, work);
    let result = b.value(work, ManagedType::Int);
    b.set(
        work,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(0),
            args: vec![lock, mode],
        },
    );
    let normal_exit = b.block();
    b.link(work, normal_exit);
    if unbalanced {
        b.set(
            normal_exit,
            ManagedInsn::FieldPut {
                object: None,
                field: FieldRef::new(0),
                value: mode,
            },
        );
    } else {
        b.set(
            normal_exit,
            ManagedInsn::MonitorExit {
                object: if reassigned { other } else { alias },
            },
        );
    }
    let after = b.block();
    b.link(normal_exit, after);
    let post = b.value(after, ManagedType::Int);
    b.set(
        after,
        if unbalanced {
            ManagedInsn::Move {
                result: post,
                value: result,
            }
        } else {
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(post),
                method: MethodRef::new(1),
                args: vec![lock, result],
            }
        },
    );
    let ret = b.block();
    b.link(after, ret);
    b.set(ret, ManagedInsn::Return { value: Some(post) });
    let mut catches = vec![];
    let mut all_exit = None;
    let mut all_entry = None;
    for (i, key) in ["broad", "narrow", "all"].into_iter().enumerate() {
        let head = b.block();
        let edges = [store, work, normal_exit]
            .map(|site| b.edge(site, head, EdgeKind::Exception(key.into())));
        let copied = b.value(head, ManagedType::Int);
        b.set(
            head,
            ManagedInsn::Phi {
                result: copied,
                inputs: edges.iter().map(|&e| (e, mode)).collect(),
            },
        );
        let capture = b.block();
        b.link(head, capture);
        let caught = b.value(capture, ManagedType::Reference(ClassRef::new(1)));
        b.set(
            capture,
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads: vec![],
                writes: vec![caught],
                effects: EffectFlags::MAY_THROW,
            },
        );
        let exit = b.block();
        b.link(capture, exit);
        b.set(exit, ManagedInsn::MonitorExit { object: alias });
        let terminal = b.block();
        b.link(exit, terminal);
        if i == 2 {
            b.set(terminal, ManagedInsn::Throw { value: caught });
            all_exit = Some(exit);
            all_entry = Some(head);
        } else if i == 0 {
            let v = b.value(terminal, ManagedType::Int);
            b.set(
                terminal,
                ManagedInsn::Invoke {
                    kind: InvokeKind::Static,
                    result: Some(v),
                    method: MethodRef::new(1),
                    args: vec![lock, copied],
                },
            );
            let ret = b.block();
            b.link(terminal, ret);
            b.set(ret, ManagedInsn::Return { value: Some(v) });
        } else {
            let v = b.value(terminal, ManagedType::Int);
            b.set(
                terminal,
                ManagedInsn::Const {
                    result: v,
                    value: Constant::Int(99),
                },
            );
            let ret = b.block();
            b.link(terminal, ret);
            b.set(ret, ManagedInsn::Return { value: Some(v) });
        }
        catches.push(scopes::Catch {
            key: key.into(),
            entry: head,
        });
    }
    // Android cleanup commonly protects its own release with a self-handler.
    // The owned same-object exit proves these exceptional paths impossible.
    let exit = all_exit.unwrap();
    let entry = all_entry.unwrap();
    let cleanup_edge = b.edge(exit, entry, EdgeKind::Exception("all".into()));
    if let ManagedInsn::Phi { inputs, .. } = &mut b.insns[*b.cfg.block(entry).unwrap().payload()] {
        inputs.push((cleanup_edge, mode));
    }
    let acquire = b.block();
    let edge = b.edge(enter, acquire, EdgeKind::Exception("acquire".into()));
    let _ = edge;
    let failure = b.value(acquire, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        acquire,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![failure],
            effects: EffectFlags::MAY_THROW,
        },
    );
    let minus = b.block();
    b.link(acquire, minus);
    let v = b.value(minus, ManagedType::Int);
    b.set(
        minus,
        ManagedInsn::Const {
            result: v,
            value: Constant::Int(-1),
        },
    );
    let ret = b.block();
    b.link(minus, ret);
    b.set(ret, ManagedInsn::Return { value: Some(v) });
    (
        b.finish(),
        vec![
            Metadata {
                scope: scopes::RegionScope {
                    protected: vec![enter],
                    catches: vec![scopes::Catch {
                        key: "acquire".into(),
                        entry: acquire,
                    }],
                    continuation: None,
                },
                hierarchy: hierarchy(),
            },
            Metadata {
                scope: scopes::RegionScope {
                    protected: vec![store, work, normal_exit],
                    catches,
                    continuation: None,
                },
                hierarchy: hierarchy(),
            },
            Metadata {
                scope: scopes::RegionScope {
                    protected: vec![exit],
                    catches: vec![scopes::Catch {
                        key: "all".into(),
                        entry,
                    }],
                    continuation: None,
                },
                hierarchy: hierarchy(),
            },
        ],
        DeclarationBindings {
            parameters: vec![Some(lock), Some(other), Some(mode)],
            ..Default::default()
        },
    )
}
#[test]
fn balanced_multiple_releases_priority_prethrow_visibility_and_null_acquisition_execute() {
    let (ir, md, bindings) = fixture(false, false);
    let tables = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let source = match emit_method(&ir, &method(), &bindings, &Resolver, &tables) {
        SourceResult::Complete { source, .. } => source,
        error => panic!("{error:?}"),
    };
    assert!(source.contains("synchronized ("));
    assert!(!source.contains("throws java.lang.Throwable"));
    let dir = std::env::temp_dir().join(format!(
        "decomphify-monitor-dispatch-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class MonitorDispatch {{
      static int seen=-10, released; static final AssertionError sentinel=new AssertionError("same-object");
      static final java.io.IOException foreign=new java.io.IOException("foreign checked identity");
      @SuppressWarnings("unchecked") static <T extends Throwable> void boundaryThrow(Throwable t) throws T {{ throw (T)t; }}
      static int work(Object lock,int mode) {{ if(!Thread.holdsLock(lock)) throw new AssertionError("lost lock"); if(mode==1) throw new ArithmeticException(); if(mode==2) throw sentinel; if(mode==3) boundaryThrow(foreign); return mode+10; }}
      static int after(Object lock,int value) {{ if(Thread.holdsLock(lock)) throw new AssertionError("release ordering"); released++; return value; }}
      {source}
      public static void main(String[] args) throws Throwable {{
        Object lock=new Object(); if(run(null,lock,0)!=-1 || seen!=-10) throw new AssertionError("null acquisition effects");
        if(run(lock,new Object(),0)!=10 || seen!=0 || released!=1) throw new AssertionError("normal release");
        if(run(lock,new Object(),1)!=1 || seen!=1 || released!=2) throw new AssertionError("priority or prethrow snapshot");
        try {{run(lock,new Object(),2);throw new AssertionError("missing throw");}} catch(AssertionError e) {{if(e!=sentinel) throw e;}}
        try {{run(lock,new Object(),3);throw new AssertionError("missing foreign throw");}} catch(Throwable e) {{if(e!=foreign) throw new AssertionError("changed checked identity",e);}}
        Thread t=new Thread(()->{{ synchronized(lock) {{ if(seen!=3) throw new AssertionError("lock visibility"); }} }});t.start();t.join(2000);if(t.isAlive()) throw new AssertionError("lock leak");
        System.out.print("monitor-dispatch-ok");
      }}
    }}"#
    );
    std::fs::write(dir.join("MonitorDispatch.java"), &java).unwrap();
    let compile = std::process::Command::new("javac")
        .args(["--release", "17", "MonitorDispatch.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}\n{java}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new("java")
        .arg("-cp")
        .arg(&dir)
        .arg("MonitorDispatch")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "monitor-dispatch-ok");
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn reassigned_and_unbalanced_original_monitors_fail_closed() {
    for (reassigned, unbalanced) in [(true, false), (false, true)] {
        let (ir, md, bindings) = fixture(reassigned, unbalanced);
        let tables = md
            .iter()
            .map(|m| m as &dyn RegionMetadata)
            .collect::<Vec<_>>();
        let result = emit_method(&ir, &method(), &bindings, &Resolver, &tables);
        assert!(matches!(result, SourceResult::Unsupported(_)), "{result:?}");
        assert!(
            format!("{result:?}").contains(if reassigned {
                "monitor release identity differs"
            } else {
                "return still owns explicit monitor"
            }),
            "{result:?}"
        );
    }
}

#[test]
fn failed_acquisition_handler_cannot_release_a_lock_it_does_not_own() {
    let mut b = Builder::new();
    let lock = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let enter = b.cfg.entry();
    b.set(enter, ManagedInsn::MonitorEnter { object: lock });
    let exit = b.block();
    b.link(enter, exit);
    b.set(exit, ManagedInsn::MonitorExit { object: lock });
    let ret = b.block();
    b.link(exit, ret);
    let caught = b.block();
    b.edge(enter, caught, EdgeKind::Exception("all".into()));
    let value = b.value(caught, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        caught,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![value],
            effects: EffectFlags::MAY_THROW,
        },
    );
    let invalid_exit = b.block();
    b.link(caught, invalid_exit);
    b.set(invalid_exit, ManagedInsn::MonitorExit { object: lock });
    let invalid_ret = b.block();
    b.link(invalid_exit, invalid_ret);
    let md = Metadata {
        scope: scopes::RegionScope {
            protected: vec![enter],
            catches: vec![scopes::Catch {
                key: "all".into(),
                entry: caught,
            }],
            continuation: None,
        },
        hierarchy: hierarchy(),
    };
    let ir = b.finish();
    let method = JavaMethod {
        return_type: JavaType::Void,
        parameters: vec![JavaType::Class("java.lang.Object".into())],
        ..method()
    };
    let bindings = DeclarationBindings {
        parameters: vec![Some(lock)],
        ..Default::default()
    };
    let result = emit_method(&ir, &method, &bindings, &Resolver, &[&md]);
    assert!(matches!(result, SourceResult::Unsupported(_)), "{result:?}");
    assert!(
        format!("{result:?}").contains("monitor exit without successful acquisition"),
        "{result:?}"
    );
}

#[test]
fn checked_original_throw_provenance_cannot_be_rewritten_as_unchecked() {
    struct CheckedResolver;
    impl JavaResolver for CheckedResolver {
        fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
            Resolver.assignable(a, b)
        }
        fn class(&self, r: ClassRef) -> Option<String> {
            Resolver.class(r)
        }
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            let mut target = Resolver.method(r)?;
            if r.index() == 0 {
                target.throws.push("java.io.IOException".into());
            }
            Some(target)
        }
        fn field_access(&self, r: FieldRef) -> Option<u32> {
            Resolver.field_access(r)
        }
        fn field(&self, r: FieldRef) -> Option<JavaField> {
            Resolver.field(r)
        }
        fn string(&self, r: StringRef) -> Option<Vec<u16>> {
            Resolver.string(r)
        }
    }
    let (ir, md, bindings) = fixture(false, false);
    let tables = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let result = emit_method(&ir, &method(), &bindings, &CheckedResolver, &tables);
    assert!(matches!(result, SourceResult::Unsupported(_)), "{result:?}");
    assert!(
        format!("{result:?}").contains("cleanup rethrow has undeclared checked contract"),
        "{result:?}"
    );
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
