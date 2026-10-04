//! A caught object's finite cleanup stays in its immutable lexical catch.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{
    DeclarationBindings, JavaField, SourceResult,
    type_compatibility::{Declaration, Hierarchy},
};
use decomp_core::cfg::Cfg;
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    ops::InvokeKind,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};
fn hierarchy() -> Hierarchy {
    let mut h = Hierarchy::default();
    for (n, p, i) in [
        ("java.lang.Object", None, false),
        ("java.lang.Throwable", Some("java.lang.Object"), false),
        ("java.lang.Exception", Some("java.lang.Throwable"), false),
        (
            "java.lang.RuntimeException",
            Some("java.lang.Exception"),
            false,
        ),
        ("java.lang.Error", Some("java.lang.Throwable"), false),
        (
            "java.lang.InterruptedException",
            Some("java.lang.Exception"),
            false,
        ),
        (
            "java.util.concurrent.ExecutionException",
            Some("java.lang.Exception"),
            false,
        ),
        ("java.io.IOException", Some("java.lang.Exception"), false),
        (
            "java.util.concurrent.Future",
            Some("java.lang.Object"),
            true,
        ),
        ("java.lang.Thread", Some("java.lang.Object"), false),
    ] {
        h.insert(Declaration {
            name: n.into(),
            superclass: p.map(Into::into),
            interfaces: vec![],
            is_interface: i,
        })
        .unwrap();
    }
    h
}
struct Resolver {
    mode: u8,
}
impl JavaResolver for Resolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            match r.index() {
                0 => "java.util.concurrent.Future",
                1 if self.mode == 13 => "java.io.IOException",
                1 => "java.lang.Throwable",
                2 => "java.lang.Thread",
                3 if self.mode == 13 => "java.lang.String",
                3 => "java.lang.Object",
                _ => "java.lang.InterruptedException",
            }
            .into(),
        )
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        if self.mode == 4 && r.index() == 0 {
            return None;
        }
        let (owner, name, is_static, return_type, throws) = match r.index() {
            0 => (
                "java.util.concurrent.Future",
                "get",
                false,
                JavaType::Class("java.lang.Object".into()),
                if self.mode == 5 {
                    vec![]
                } else {
                    vec![
                        "java.lang.InterruptedException".into(),
                        "java.util.concurrent.ExecutionException".into(),
                    ]
                },
            ),
            1 => (
                "java.lang.Thread",
                "currentThread",
                true,
                JavaType::Class("java.lang.Thread".into()),
                if matches!(self.mode, 10..=12) {
                    vec!["SourceCheckedX".into()]
                } else {
                    vec![]
                },
            ),
            2 => (
                "java.lang.Thread",
                "interrupt",
                false,
                JavaType::Void,
                vec![],
            ),
            _ => (
                "CleanupFixture",
                "effect",
                true,
                JavaType::Void,
                vec!["java.io.IOException".into()],
            ),
        };
        Some(JavaMethod {
            owner: owner.into(),
            name: name.into(),
            is_static,
            return_type,
            parameters: vec![],
            throws,
        })
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        Some(if r.index() == 1 || r.index() == 3 {
            9
        } else {
            1
        })
    }
    fn field(&self, reference: FieldRef) -> Option<JavaField> {
        (self.mode == 8 && reference.index() == 0).then(|| JavaField {
            owner: "CleanupFixture".into(),
            name: "thread".into(),
            ty: JavaType::Class("java.lang.Thread".into()),
            is_static: true,
        })
    }
    fn field_access(&self, _: FieldRef) -> Option<u32> {
        (self.mode == 8).then_some(9)
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        (self.mode == 13).then(|| "x".encode_utf16().collect())
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        hierarchy().assignable(a, b) == Compatibility::Assignable
    }
}
struct Md {
    scope: scopes::RegionScope,
    h: Hierarchy,
}
impl RegionMetadata for Md {
    fn region_scope(&self) -> &scopes::RegionScope {
        &self.scope
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.h
    }
    fn is_catch_all(&self, k: &str) -> bool {
        k == "all"
    }
    fn catch_type(&self, k: &str) -> Option<JavaType> {
        Some(JavaType::Class(
            if k == "all" {
                "java.lang.Throwable"
            } else {
                "java.lang.InterruptedException"
            }
            .into(),
        ))
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
fn fixture(mode: u8) -> (MethodIr<SsaForm>, Md, DeclarationBindings, BlockId) {
    let mut b = Builder::new();
    let future = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let restore = b.incoming(ManagedType::Boolean);
    let foreign = b.incoming(ManagedType::Reference(ClassRef::new(1)));
    let entry = b.cfg.entry();
    let result = b.value(entry, ManagedType::Reference(ClassRef::new(3)));
    b.set(
        entry,
        ManagedInsn::Invoke {
            result: Some(result),
            kind: InvokeKind::Interface,
            method: MethodRef::new(0),
            args: vec![future],
        },
    );
    let ret = b.block();
    b.link(entry, ret);
    b.set(
        ret,
        ManagedInsn::Return {
            value: Some(result),
        },
    );
    let interrupted = b.block();
    b.edge(
        entry,
        interrupted,
        EdgeKind::Exception("interrupted".into()),
    );
    let ignored = b.value(interrupted, ManagedType::Reference(ClassRef::new(4)));
    b.set(
        interrupted,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![ignored],
            effects: EffectFlags::NONE,
        },
    );
    let null = b.block();
    b.link(interrupted, null);
    let zero = b.value(null, ManagedType::Null);
    b.set(
        null,
        ManagedInsn::Const {
            result: zero,
            value: managed_ir::ops::Constant::Null,
        },
    );
    let nr = b.block();
    b.link(null, nr);
    b.set(nr, ManagedInsn::Return { value: Some(zero) });
    let cleanup = b.block();
    b.edge(entry, cleanup, EdgeKind::Exception("all".into()));
    let caught = b.value(cleanup, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        cleanup,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![caught],
            effects: EffectFlags::NONE,
        },
    );
    let moved = b.block();
    b.link(cleanup, moved);
    let alias = b.value(moved, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        moved,
        ManagedInsn::Move {
            result: alias,
            value: caught,
        },
    );
    let branch = b.block();
    b.link(moved, branch);
    let current = b.block();
    let interrupt = b.block();
    b.link(current, interrupt);
    let thread = b.value(current, ManagedType::Reference(ClassRef::new(2)));
    b.set(
        current,
        if mode == 8 {
            ManagedInsn::FieldGet {
                result: thread,
                object: None,
                field: FieldRef::new(0),
            }
        } else {
            ManagedInsn::Invoke {
                result: Some(thread),
                kind: InvokeKind::Static,
                method: MethodRef::new(1),
                args: vec![],
            }
        },
    );
    b.set(
        interrupt,
        ManagedInsn::Invoke {
            result: None,
            kind: if mode == 6 {
                InvokeKind::Static
            } else {
                InvokeKind::Virtual
            },
            method: MethodRef::new(if mode == 6 { 3 } else { 2 }),
            args: if mode == 6 { vec![] } else { vec![thread] },
        },
    );
    let join = b.block();
    let yes = b.edge(branch, current, EdgeKind::BranchTrue);
    let no = b.edge(
        branch,
        if mode == 3 { branch } else { join },
        EdgeKind::BranchFalse,
    );
    b.set(
        branch,
        ManagedInsn::Branch {
            condition: Some(restore),
            then_edge: yes,
            else_edge: Some(no),
        },
    );
    let ie = b.link(interrupt, join);
    let merged = b.value(join, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        join,
        ManagedInsn::Phi {
            result: merged,
            inputs: if mode == 3 {
                vec![(ie, alias)]
            } else {
                vec![(ie, alias), (no, if mode == 1 { foreign } else { alias })]
            },
        },
    );
    let throw = b.block();
    b.link(join, throw);
    b.set(throw, ManagedInsn::Throw { value: merged });
    if mode == 2 {
        b.edge(ret, join, EdgeKind::Unconditional);
    }
    (
        MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values),
        Md {
            scope: scopes::RegionScope {
                protected: vec![entry],
                catches: vec![
                    scopes::Catch {
                        key: "interrupted".into(),
                        entry: interrupted,
                    },
                    scopes::Catch {
                        key: "all".into(),
                        entry: cleanup,
                    },
                ],
                continuation: None,
            },
            h: hierarchy(),
        },
        DeclarationBindings {
            parameters: vec![Some(future), Some(restore), Some(foreign)],
            ..Default::default()
        },
        cleanup,
    )
}
fn method() -> JavaMethod {
    JavaMethod {
        owner: "CleanupFixture".into(),
        name: "f".into(),
        is_static: true,
        parameters: vec![
            JavaType::Class("java.util.concurrent.Future".into()),
            JavaType::Boolean,
            JavaType::Class("java.lang.Throwable".into()),
        ],
        return_type: JavaType::Class("java.lang.Object".into()),
        throws: vec!["java.util.concurrent.ExecutionException".into()],
    }
}
#[test]
fn immutable_cleanup_phi_identity_and_ordered_catch_exclusions_are_proved() {
    let (ir, md, b, entry) = fixture(0);
    let p = super::super::plan(
        &ir,
        &[&md],
        &b.parameters.iter().flatten().copied().collect::<Vec<_>>(),
        &Resolver { mode: 0 },
        &HashMap::new(),
    )
    .unwrap();
    assert!(p.rethrows.cleanups.contains_key(&entry));
    let out = super::super::emit_method(&ir, &method(), &b, &Resolver { mode: 0 }, &[&md]);
    let SourceResult::Complete { source, .. } = out else {
        panic!("{out:?}")
    };
    assert!(source.contains("catch (java.lang.InterruptedException"));
    assert!(source.contains("catch (java.lang.Throwable"));
    let caught = source
        .split("catch (java.lang.Throwable ")
        .nth(1)
        .unwrap()
        .split(')')
        .next()
        .unwrap();
    assert!(source.contains(&format!("throw {caught};")));
    assert!(!source.contains(&format!("{caught} =")));
    std::fs::write("/tmp/opencode/wave10-cleanup-fixture.java", &source).unwrap();
    let dir = std::env::temp_dir().join(format!("proved-catch-cleanup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class CleanupFixture {{ {source}
    static final Object token=new Object();static final java.util.concurrent.ExecutionException execution=new java.util.concurrent.ExecutionException("same",null);static final RuntimeException runtime=new RuntimeException("same");
    static class F implements java.util.concurrent.Future<Object>{{final int mode;F(int n){{mode=n;}}public Object get()throws InterruptedException,java.util.concurrent.ExecutionException{{if(mode==1)throw execution;if(mode==2)throw runtime;if(mode==3)throw new InterruptedException();return token;}}public Object get(long n,java.util.concurrent.TimeUnit u)throws InterruptedException,java.util.concurrent.ExecutionException{{return get();}}public boolean cancel(boolean b){{return false;}}public boolean isCancelled(){{return false;}}public boolean isDone(){{return false;}}}}
    public static void main(String[]args)throws Exception{{for(boolean restore:new boolean[]{{false,true}}){{Thread.interrupted();if(f(new F(0),restore,null)!=token||Thread.currentThread().isInterrupted())throw new AssertionError();for(int mode:new int[]{{1,2}}){{try{{f(new F(mode),restore,null);throw new AssertionError();}}catch(Throwable t){{if(t!=(mode==1?execution:runtime)||Thread.currentThread().isInterrupted()!=restore)throw new AssertionError(t);}}Thread.interrupted();}}if(f(new F(3),restore,null)!=null||Thread.currentThread().isInterrupted())throw new AssertionError("priority");}}}}
    }}"#
    );
    std::fs::write(dir.join("CleanupFixture.java"), java).unwrap();
    let home = std::env::var_os("JAVA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (tool, args) in [
        ("javac", vec!["--release", "17", "CleanupFixture.java"]),
        ("java", vec!["-Xverify:all", "-cp", ".", "CleanupFixture"]),
    ] {
        let out = std::process::Command::new(home.join("bin").join(tool))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn finite_cleanup_rejects_changed_identity_shared_entries_loops_and_missing_contracts() {
    for mode in [1, 2, 3, 4, 6] {
        let (ir, md, b, _) = fixture(mode);
        assert!(
            matches!(
                super::super::emit_method(&ir, &method(), &b, &Resolver { mode }, &[&md]),
                SourceResult::Unsupported(_)
            ),
            "mode={mode}"
        );
    }
}
#[test]
fn unresolved_dispatch_remains_on_its_existing_noncleanup_path() {
    let (ir, md, b, entry) = fixture(4);
    let p = super::super::plan(
        &ir,
        &[&md],
        &b.parameters.iter().flatten().copied().collect::<Vec<_>>(),
        &Resolver { mode: 4 },
        &HashMap::new(),
    )
    .unwrap();
    assert!(!p.rethrows.cleanups.contains_key(&entry));
    assert!(!p.rethrows.elided.contains(&entry));
}

#[test]
fn typed_broad_capture_keeps_precise_contract_and_lexical_identity() {
    struct Typed<'a>(&'a Md);
    impl RegionMetadata for Typed<'_> {
        fn region_scope(&self) -> &scopes::RegionScope {
            self.0.region_scope()
        }
        fn hierarchy(&self) -> &Hierarchy {
            self.0.hierarchy()
        }
        fn is_catch_all(&self, _: &str) -> bool {
            false
        }
        fn catch_type(&self, key: &str) -> Option<JavaType> {
            self.0.catch_type(key)
        }
    }
    let (ir, md, bindings, entry) = fixture(0);
    let metadata = Typed(&md);
    let resolver = Resolver { mode: 0 };
    let plan = super::super::plan(
        &ir,
        &[&metadata],
        &bindings
            .parameters
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>(),
        &resolver,
        &HashMap::new(),
    )
    .unwrap();
    assert!(plan.rethrows.cleanups.contains_key(&entry));
    let output = super::super::emit_method(&ir, &method(), &bindings, &resolver, &[&metadata]);
    let SourceResult::Complete { source, .. } = output else {
        panic!("{output:?}");
    };
    let caught = source
        .split("catch (java.lang.Throwable ")
        .nth(1)
        .unwrap()
        .split(')')
        .next()
        .unwrap();
    assert!(source.contains(&format!("throw {caught};")));
    assert!(!source.contains(&format!("throw (java.lang.Throwable) {caught};")));
    let mut missing = method();
    missing.throws.clear();
    assert!(!matches!(
        super::super::emit_method(&ir, &missing, &bindings, &resolver, &[&metadata]),
        SourceResult::Complete { .. }
    ));
}

#[test]
fn cleanup_field_read_keeps_order_and_requires_genuine_declaration() {
    let (ir, md, bindings, entry) = fixture(8);
    let resolver = Resolver { mode: 8 };
    let plan = super::super::plan(
        &ir,
        &[&md],
        &bindings
            .parameters
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>(),
        &resolver,
        &HashMap::new(),
    )
    .unwrap();
    assert!(plan.rethrows.cleanups.contains_key(&entry));
    let output = super::super::emit_method(&ir, &method(), &bindings, &resolver, &[&md]);
    let SourceResult::Complete { source, .. } = output else {
        panic!("{output:?}");
    };
    assert!(source.contains("CleanupFixture.thread"));
    let missing = super::super::plan(
        &ir,
        &[&md],
        &bindings
            .parameters
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>(),
        &Resolver { mode: 9 },
        &HashMap::new(),
    )
    .unwrap();
    assert!(!missing.rethrows.cleanups.contains_key(&entry));
}

#[test]
fn empty_checked_source_uses_legal_typed_catch_and_precise_parameter() {
    struct Typed(Md);
    impl RegionMetadata for Typed {
        fn is_catch_all(&self, _: &str) -> bool {
            false
        }
        fn region_scope(&self) -> &scopes::RegionScope {
            &self.0.scope
        }
        fn hierarchy(&self) -> &Hierarchy {
            &self.0.h
        }
        fn catch_type(&self, _: &str) -> Option<JavaType> {
            Some(JavaType::Class("java.lang.Exception".into()))
        }
    }
    let mut b = Builder::new();
    let entry = b.cfg.entry();
    let result = b.value(entry, ManagedType::Reference(ClassRef::new(2)));
    b.set(
        entry,
        ManagedInsn::Invoke {
            result: Some(result),
            kind: InvokeKind::Static,
            method: MethodRef::new(1),
            args: vec![],
        },
    );
    let ret = b.block();
    b.link(entry, ret);
    b.set(ret, ManagedInsn::Return { value: None });
    let handler = b.block();
    b.edge(entry, handler, EdgeKind::Exception("typed".into()));
    let caught = b.value(handler, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        handler,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![caught],
            effects: EffectFlags::NONE,
        },
    );
    let tail = b.block();
    b.link(handler, tail);
    b.set(tail, ManagedInsn::Throw { value: caught });
    let ir = MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values);
    let metadata = Typed(Md {
        scope: scopes::RegionScope {
            protected: vec![entry],
            catches: vec![scopes::Catch {
                key: "typed".into(),
                entry: handler,
            }],
            continuation: None,
        },
        h: hierarchy(),
    });
    let method = JavaMethod {
        owner: "TypedEmpty".into(),
        name: "f".into(),
        is_static: true,
        parameters: vec![],
        return_type: JavaType::Void,
        throws: vec![],
    };
    let output = super::super::emit_method(
        &ir,
        &method,
        &DeclarationBindings::default(),
        &Resolver { mode: 0 },
        &[&metadata],
    );
    let SourceResult::Complete { source, .. } = output else {
        panic!("{output:?}");
    };
    assert!(source.contains("catch (java.lang.Exception "));
    let parameter = source
        .split("catch (java.lang.Exception ")
        .nth(1)
        .unwrap()
        .split(')')
        .next()
        .unwrap();
    assert!(source.contains(&format!("throw {parameter};")));
    // Missing or cyclic checked-origin ancestry must never be treated as
    // known disjoint from this broad Exception catch.
    for mode in [10, 11] {
        let mut unknown = Typed(Md {
            scope: metadata.0.scope.clone(),
            h: hierarchy(),
        });
        if mode == 11 {
            for (name, parent) in [
                ("SourceCheckedX", "SourceCycle"),
                ("SourceCycle", "SourceCheckedX"),
            ] {
                unknown
                    .0
                    .h
                    .insert(Declaration {
                        name: name.into(),
                        superclass: Some(parent.into()),
                        interfaces: vec![],
                        is_interface: false,
                    })
                    .unwrap();
            }
        }
        for caller in [
            method.clone(),
            JavaMethod {
                throws: vec!["java.lang.Exception".into()],
                ..method.clone()
            },
        ] {
            let output = super::super::emit_method(
                &ir,
                &caller,
                &DeclarationBindings::default(),
                &Resolver { mode },
                &[&unknown],
            );
            assert!(
                matches!(output, SourceResult::Unsupported(_)),
                "mode={mode} caller={caller:?} output={output:?}"
            );
        }
        // Explicitly cover both nominal catch type and protected source name
        // so the earlier invoke check cannot hide a precision-filter bug.
        let broad = JavaMethod {
            throws: vec!["java.lang.Exception".into(), "SourceCheckedX".into()],
            ..method.clone()
        };
        let output = super::super::emit_method(
            &ir,
            &broad,
            &DeclarationBindings::default(),
            &Resolver { mode },
            &[&unknown],
        );
        assert!(
            matches!(&output, SourceResult::Unsupported(Unsupported::Type(reason))
            if reason == "typed cleanup source/catch ancestry is indeterminate"),
            "mode={mode} output={output:?}"
        );
    }
    // A genuine closed class chain directly under Throwable is disjoint from
    // Exception in both directions. Its checked contract still escapes and is
    // explicitly declared by the enclosing method.
    let mut unrelated = Typed(Md {
        scope: metadata.0.scope.clone(),
        h: hierarchy(),
    });
    unrelated
        .0
        .h
        .insert(Declaration {
            name: "SourceCheckedX".into(),
            superclass: Some("java.lang.Throwable".into()),
            interfaces: vec![],
            is_interface: false,
        })
        .unwrap();
    let mut declaring = method.clone();
    declaring.throws = vec!["SourceCheckedX".into()];
    assert!(matches!(
        super::super::emit_method(
            &ir,
            &declaring,
            &DeclarationBindings::default(),
            &Resolver { mode: 12 },
            &[&unrelated]
        ),
        SourceResult::Complete { .. }
    ));
    let directory = std::env::temp_dir().join(format!("typed-empty-source-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("TypedEmpty.java"), format!("public class TypedEmpty {{ {source} public static void main(String[] args) {{ f(); }} }}")).unwrap();
    assert!(
        std::process::Command::new("javac")
            .arg(directory.join("TypedEmpty.java"))
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new("java")
            .arg("-cp")
            .arg(&directory)
            .arg("TypedEmpty")
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn newly_typed_source_empty_const_keeps_original_guarded_handler() {
    struct Checked(Md);
    impl RegionMetadata for Checked {
        fn is_catch_all(&self, _: &str) -> bool {
            false
        }
        fn region_scope(&self) -> &scopes::RegionScope {
            &self.0.scope
        }
        fn hierarchy(&self) -> &Hierarchy {
            &self.0.h
        }
        fn catch_type(&self, _: &str) -> Option<JavaType> {
            Some(JavaType::Class("java.io.IOException".into()))
        }
    }
    let mut b = Builder::new();
    let entry = b.cfg.entry();
    let result = b.value(entry, ManagedType::Reference(ClassRef::new(3)));
    b.set(
        entry,
        ManagedInsn::Const {
            result,
            value: managed_ir::ops::Constant::String(StringRef::new(0)),
        },
    );
    let ret = b.block();
    b.link(entry, ret);
    b.set(ret, ManagedInsn::Return { value: None });
    let handler = b.block();
    b.edge(entry, handler, EdgeKind::Exception("io".into()));
    let caught = b.value(handler, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        handler,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![caught],
            effects: EffectFlags::NONE,
        },
    );
    let tail = b.block();
    b.link(handler, tail);
    b.set(tail, ManagedInsn::Throw { value: caught });
    let ir = MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values);
    let metadata = Checked(Md {
        scope: scopes::RegionScope {
            protected: vec![entry],
            catches: vec![scopes::Catch {
                key: "io".into(),
                entry: handler,
            }],
            continuation: None,
        },
        h: hierarchy(),
    });
    let method = JavaMethod {
        owner: "GuardedConst".into(),
        name: "f".into(),
        is_static: true,
        parameters: vec![],
        return_type: JavaType::Void,
        throws: vec!["java.io.IOException".into()],
    };
    let output = super::super::emit_method(
        &ir,
        &method,
        &DeclarationBindings::default(),
        &Resolver { mode: 13 },
        &[&metadata],
    );
    let SourceResult::Complete { source, .. } = output else {
        panic!("{output:?}");
    };
    assert!(
        source.contains("instanceof java.io.IOException"),
        "{source}"
    );
    assert!(!source.contains("catch (java.io.IOException "), "{source}");
}
