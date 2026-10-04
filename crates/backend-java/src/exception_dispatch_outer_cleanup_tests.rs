//! Post-release catches compose only with exact original handler transfers.
#![allow(clippy::unwrap_used)]
use super::*;
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

use super::super::{emit_method, plan};
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
    h.insert(Declaration {
        name: "NestedCleanup.Checked".into(),
        superclass: Some("java.io.IOException".into()),
        interfaces: vec![],
        is_interface: false,
    })
    .unwrap();
    h.insert(Declaration {
        name: "java.lang.IllegalArgumentException".into(),
        superclass: Some("java.lang.RuntimeException".into()),
        interfaces: vec![],
        is_interface: false,
    })
    .unwrap();
    h
}

struct Metadata {
    scope: scopes::RegionScope,
    h: Hierarchy,
}
impl RegionMetadata for Metadata {
    fn region_scope(&self) -> &scopes::RegionScope {
        &self.scope
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.h
    }
    fn is_catch_all(&self, key: &str) -> bool {
        key == "all"
    }
    fn catch_type(&self, key: &str) -> Option<JavaType> {
        Some(JavaType::Class(
            match key {
                "io" => "java.io.IOException",
                "iae" => "java.lang.IllegalArgumentException",
                _ => "java.lang.Throwable",
            }
            .into(),
        ))
    }
}
struct Resolver {
    missing: bool,
}
impl JavaResolver for Resolver {
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
        if self.missing && r.index() == 0 {
            return None;
        }
        Some(JavaMethod {
            owner: "NestedCleanup".into(),
            name: if r.index() == 0 { "work" } else { "failure" }.into(),
            is_static: true,
            parameters: vec![JavaType::Class("java.lang.Object".into()), JavaType::Int],
            return_type: JavaType::Int,
            throws: if r.index() == 0 {
                vec!["NestedCleanup.Checked".into()]
            } else {
                vec![]
            },
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        hierarchy().assignable(a, b) == Compatibility::Assignable
    }
}
fn method() -> JavaMethod {
    JavaMethod {
        owner: "NestedCleanup".into(),
        name: "run".into(),
        is_static: true,
        parameters: vec![JavaType::Class("java.lang.Object".into()), JavaType::Int],
        return_type: JavaType::Int,
        throws: vec![],
    }
}
fn fixture(
    mode: u8,
) -> (
    MethodIr<SsaForm>,
    Vec<Metadata>,
    DeclarationBindings,
    BlockId,
    BlockId,
) {
    let mut b = Builder::new();
    let lock = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let arg = b.incoming(ManagedType::Int);
    let enter = b.cfg.entry();
    b.set(enter, ManagedInsn::MonitorEnter { object: lock });
    let work = b.block();
    b.link(enter, work);
    let result = b.value(work, ManagedType::Int);
    b.set(
        work,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(0),
            args: vec![lock, arg],
        },
    );
    let exit = b.block();
    b.link(work, exit);
    b.set(exit, ManagedInsn::MonitorExit { object: lock });
    let ret = b.block();
    b.link(exit, ret);
    b.set(
        ret,
        ManagedInsn::Return {
            value: Some(result),
        },
    );
    let capture = b.block();
    b.edge(work, capture, EdgeKind::Exception("all".into()));
    b.edge(exit, capture, EdgeKind::Exception("all".into()));
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
    let release = b.block();
    b.link(capture, release);
    b.set(release, ManagedInsn::MonitorExit { object: lock });
    b.edge(release, capture, EdgeKind::Exception("all".into()));
    let throw = b.block();
    b.link(release, throw);
    b.set(throw, ManagedInsn::Throw { value: caught });
    let failure = b.block();
    let mut input = Vec::new();
    for site in [enter, throw] {
        for key in if mode == 1 && site == throw {
            ["iae", "io"]
        } else {
            ["io", "iae"]
        } {
            input.push(b.edge(site, failure, EdgeKind::Exception(key.into())));
        }
    }
    if mode == 2 {
        let phi = b.value(failure, ManagedType::Int);
        b.set(
            failure,
            ManagedInsn::Phi {
                result: phi,
                inputs: input.iter().map(|&e| (e, arg)).collect(),
            },
        );
        let next = b.block();
        b.link(failure, next);
        let out = b.value(next, ManagedType::Int);
        b.set(
            next,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(out),
                method: MethodRef::new(1),
                args: vec![lock, phi],
            },
        );
        let ret = b.block();
        b.link(next, ret);
        b.set(ret, ManagedInsn::Return { value: Some(out) });
    } else {
        let out = b.value(failure, ManagedType::Int);
        b.set(
            failure,
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(out),
                method: MethodRef::new(1),
                args: vec![lock, arg],
            },
        );
        let ret = b.block();
        b.link(failure, ret);
        b.set(ret, ManagedInsn::Return { value: Some(out) });
    }
    let outer = |site, reverse| Metadata {
        scope: scopes::RegionScope {
            protected: vec![site],
            catches: if reverse {
                vec![
                    scopes::Catch {
                        key: "iae".into(),
                        entry: failure,
                    },
                    scopes::Catch {
                        key: "io".into(),
                        entry: failure,
                    },
                ]
            } else {
                vec![
                    scopes::Catch {
                        key: "io".into(),
                        entry: failure,
                    },
                    scopes::Catch {
                        key: "iae".into(),
                        entry: failure,
                    },
                ]
            },
            continuation: None,
        },
        h: hierarchy(),
    };
    let md = vec![
        outer(enter, false),
        Metadata {
            scope: scopes::RegionScope {
                protected: vec![work, exit, release],
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: capture,
                }],
                continuation: None,
            },
            h: hierarchy(),
        },
        outer(throw, mode == 1),
    ];
    (
        b.finish(),
        md,
        DeclarationBindings {
            parameters: vec![Some(lock), Some(arg)],
            ..Default::default()
        },
        enter,
        capture,
    )
}
#[test]
fn exact_post_release_table_and_checked_contract_are_proved() {
    let (ir, md, bindings, owner, capture) = fixture(0);
    let tables = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let p = plan(
        &ir,
        &tables,
        &bindings
            .parameters
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>(),
        &Resolver { missing: false },
        &HashMap::new(),
    )
    .unwrap();
    assert_eq!(
        p.rethrows.outer_monitors[&owner],
        [JavaType::Class("NestedCleanup.Checked".into())]
    );
    assert_eq!(
        p.rethrows.enclosing[&capture],
        [
            JavaType::Class("java.io.IOException".into()),
            JavaType::Class("java.lang.IllegalArgumentException".into())
        ]
    );
}
#[test]
fn changed_handler_order_and_prethrow_phi_snapshots_remain_uncomposed() {
    for mode in [1, 2] {
        let (ir, md, bindings, _, _) = fixture(mode);
        let tables = md
            .iter()
            .map(|m| m as &dyn RegionMetadata)
            .collect::<Vec<_>>();
        let p = plan(
            &ir,
            &tables,
            &bindings
                .parameters
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>(),
            &Resolver { missing: false },
            &HashMap::new(),
        )
        .unwrap();
        assert!(p.rethrows.outer_monitors.is_empty(), "mode={mode}");
        assert!(
            matches!(
                emit_method(
                    &ir,
                    &method(),
                    &bindings,
                    &Resolver { missing: false },
                    &tables
                ),
                SourceResult::Unsupported(_)
            ),
            "mode={mode}"
        );
    }
}
#[test]
fn checked_cleanup_missing_call_contract_remains_uncomposed() {
    let (ir, md, bindings, _, _) = fixture(0);
    let tables = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let p = plan(
        &ir,
        &tables,
        &bindings
            .parameters
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>(),
        &Resolver { missing: true },
        &HashMap::new(),
    )
    .unwrap();
    assert!(p.rethrows.outer_monitors.is_empty());
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

#[test]
fn composed_cleanup_executes_original_order_and_foreign_exception_identity() {
    let (ir, md, bindings, _, _) = fixture(0);
    let tables = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let SourceResult::Complete { source, .. } = emit_method(
        &ir,
        &method(),
        &bindings,
        &Resolver { missing: false },
        &tables,
    ) else {
        panic!("composed body refused")
    };
    assert!(!source.contains("throws java.io.IOException"));
    let dir = std::path::PathBuf::from("/tmp/opencode/wave13-composed-cleanup-runtime");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("emitted-method.java.txt"), &source).unwrap();
    let java = format!(
        r#"public class NestedCleanup {{
      static class Checked extends java.io.IOException {{}}
      static final Checked io=new Checked();static final IllegalArgumentException iae=new IllegalArgumentException();static final Error error=new AssertionError();static final NullPointerException npe=new NullPointerException();static final java.io.IOException foreign=new java.io.IOException();static final InterruptedException other=new InterruptedException();static String events="";
      @SuppressWarnings("unchecked") static <T extends Throwable> void boundary(Throwable t)throws T {{throw (T)t;}}
      static int work(Object lock,int mode)throws Checked {{if(!Thread.holdsLock(lock))throw new AssertionError("before release");events+="W";switch(mode){{case 1:throw io;case 2:throw iae;case 3:throw error;case 4:throw npe;case 5:boundary(foreign);case 6:boundary(other);}}return 7;}}
      static int failure(Object lock,int mode){{if(Thread.holdsLock(lock))throw new AssertionError("handler before release");events+="F";return -1;}}
      static int original(Object lock,int mode){{try{{synchronized(lock){{return work(lock,mode);}}}}catch(java.io.IOException|IllegalArgumentException e){{return failure(lock,mode);}}}}
      {source}
      static String observe(boolean emitted,Object lock,int mode){{events="";String outcome;try{{outcome="return:"+(emitted?run(lock,mode):original(lock,mode));}}catch(Throwable e){{if(e!=error&&e!=npe&&e!=other&&lock!=null)throw new AssertionError("changed identity",e);outcome="throw:"+e.getClass().getName();}}if(Thread.holdsLock(lock==null?NestedCleanup.class:lock))throw new AssertionError("leaked monitor");return outcome+":"+events;}}
      public static void main(String[]args){{for(int mode=0;mode<=6;mode++){{Object lock=new Object();String reference=observe(false,lock,mode),actual=observe(true,lock,mode);if(!reference.equals(actual))throw new AssertionError(mode+" "+reference+" != "+actual);System.out.println(mode+" "+actual);}}String reference=observe(false,null,0),actual=observe(true,null,0);if(!reference.equals(actual))throw new AssertionError("acquisition "+actual);System.out.println("null "+actual);}}
    }}"#
    );
    std::fs::write(dir.join("NestedCleanup.java"), java).unwrap();
    let home = std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11");
    for (tool, args) in [
        ("javac", vec!["--release", "17", "NestedCleanup.java"]),
        ("java", vec!["-Xverify:all", "-cp", ".", "NestedCleanup"]),
    ] {
        let result = std::process::Command::new(home.join("bin").join(tool))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        if tool == "java" {
            std::fs::write(dir.join("runtime.txt"), result.stdout).unwrap();
        }
    }
}

struct UncheckedResolver;
impl JavaResolver for UncheckedResolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Resolver { missing: false }.class(r)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let mut m = Resolver { missing: false }.method(r)?;
        if r.index() == 0 {
            m.name = "workUnchecked".into();
            m.throws.clear();
        }
        Some(m)
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        Resolver { missing: false }.field(r)
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        Resolver { missing: false }.string(r)
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        Resolver { missing: false }.assignable(a, b)
    }
}
struct UncheckedMetadata<'a>(&'a Metadata);
impl RegionMetadata for UncheckedMetadata<'_> {
    fn region_scope(&self) -> &scopes::RegionScope {
        self.0.region_scope()
    }
    fn hierarchy(&self) -> &Hierarchy {
        self.0.hierarchy()
    }
    fn is_catch_all(&self, k: &str) -> bool {
        self.0.is_catch_all(k)
    }
    fn catch_type(&self, k: &str) -> Option<JavaType> {
        if k == "io" {
            Some(JavaType::Class("java.lang.RuntimeException".into()))
        } else {
            self.0.catch_type(k)
        }
    }
}
#[test]
fn zero_checked_contract_preserves_post_release_runtime_catch() {
    let (ir, md, bindings, owner, _) = fixture(0);
    let md = md.iter().map(UncheckedMetadata).collect::<Vec<_>>();
    let tables = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let p = plan(
        &ir,
        &tables,
        &bindings
            .parameters
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>(),
        &UncheckedResolver,
        &HashMap::new(),
    )
    .unwrap();
    assert!(p.rethrows.outer_monitors.contains_key(&owner));
    assert!(p.rethrows.outer_monitors[&owner].is_empty());
    let SourceResult::Complete { source, .. } =
        emit_method(&ir, &method(), &bindings, &UncheckedResolver, &tables)
    else {
        panic!("unchecked composed body refused")
    };
    let dir = std::path::PathBuf::from("/tmp/opencode/wave13-unchecked-composed-cleanup-runtime");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("emitted-method.java.txt"), &source).unwrap();
    let java = format!(
        r#"public class NestedCleanup {{static String events="";static final RuntimeException sentinel=new RuntimeException();static int workUnchecked(Object lock,int mode){{if(!Thread.holdsLock(lock))throw new AssertionError("lost lock");events+="W";throw sentinel;}}static int failure(Object lock,int mode){{if(Thread.holdsLock(lock))throw new AssertionError("handler before release");events+="F";return -1;}}static int original(Object lock,int mode){{try{{synchronized(lock){{return workUnchecked(lock,mode);}}}}catch(RuntimeException e){{if(e!=sentinel)throw new AssertionError("changed caught identity");return failure(lock,mode);}}}}{source}public static void main(String[]args){{Object lock=new Object();int expected=original(lock,0);String reference=events;events="";int actual=run(lock,0);if(actual!=expected||!events.equals(reference)||!events.equals("WF")||Thread.holdsLock(lock))throw new AssertionError(actual+events);System.out.println("unchecked WF released");}}}}"#
    );
    std::fs::write(dir.join("NestedCleanup.java"), java).unwrap();
    let home = std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11");
    for (tool, args) in [
        ("javac", vec!["--release", "17", "NestedCleanup.java"]),
        ("java", vec!["-Xverify:all", "-cp", ".", "NestedCleanup"]),
    ] {
        let result = std::process::Command::new(home.join("bin").join(tool))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        if tool == "java" {
            std::fs::write(dir.join("runtime.txt"), result.stdout).unwrap();
        }
    }
}

#[test]
fn zero_checked_contract_preserves_encoded_io_catch_and_foreign_identity() {
    let (ir, md, bindings, owner, _) = fixture(0);
    let tables = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let p = plan(
        &ir,
        &tables,
        &bindings
            .parameters
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>(),
        &UncheckedResolver,
        &HashMap::new(),
    )
    .unwrap();
    assert!(p.rethrows.outer_monitors[&owner].is_empty());
    let SourceResult::Complete { source, .. } =
        emit_method(&ir, &method(), &bindings, &UncheckedResolver, &tables)
    else {
        panic!("encoded checked catch lost with zero checked contract")
    };
    // An unchecked source domain retains the encoded checked table through
    // ordered guarded dispatch; its immutable catch still has precise rethrow.
    assert!(!source.contains("catch (java.io.IOException"));
    assert!(source.contains("instanceof java.io.IOException"));
    let dir = std::path::PathBuf::from("/tmp/opencode/wave14-zero-checked-encoded-io-runtime");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("emitted-method.java.txt"), &source).unwrap();
    let java = format!(
        r#"public class NestedCleanup {{
      static String events="";static final java.io.IOException io=new java.io.IOException("same");static final Error error=new AssertionError("same");static final InterruptedException foreign=new InterruptedException("same");
      @SuppressWarnings("unchecked") static <T extends Throwable> void boundary(Throwable t)throws T{{throw (T)t;}}
      static int workUnchecked(Object lock,int mode){{if(!Thread.holdsLock(lock))throw new AssertionError("lost lock");events+="W";boundary(mode==0?io:mode==1?error:foreign);return 0;}}
      static int failure(Object lock,int mode){{if(Thread.holdsLock(lock))throw new AssertionError("handler before release");events+="F";return -1;}}
      // This reference uses the original encoded IOException/IAE selection
      // explicitly because a native IOException catch is unreachable in Java.
      static int original(Object lock,int mode){{try{{synchronized(lock){{return workUnchecked(lock,mode);}}}}catch(Throwable e){{if(e instanceof java.io.IOException||e instanceof IllegalArgumentException)return failure(lock,mode);throw e;}}}}
      {source}
      static String observe(boolean emitted,Object lock,int mode){{events="";String outcome;try{{outcome="return:"+(emitted?run(lock,mode):original(lock,mode));}}catch(Throwable e){{if(e!=(mode==1?error:foreign))throw new AssertionError("changed identity",e);outcome="throw:"+e.getClass().getName();}}if(Thread.holdsLock(lock))throw new AssertionError("lock leak");return outcome+":"+events;}}
      public static void main(String[]args){{Object lock=new Object();for(int mode=0;mode<3;mode++){{String expected=observe(false,lock,mode),actual=observe(true,lock,mode);if(!expected.equals(actual))throw new AssertionError(mode+" "+actual);System.out.println(mode+" "+actual);}}}}
    }}"#
    );
    std::fs::write(dir.join("NestedCleanup.java"), java).unwrap();
    let home = std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11");
    for (tool, args) in [
        ("javac", vec!["--release", "17", "NestedCleanup.java"]),
        ("java", vec!["-Xverify:all", "-cp", ".", "NestedCleanup"]),
    ] {
        let result = std::process::Command::new(home.join("bin").join(tool))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        if tool == "java" {
            std::fs::write(dir.join("runtime.txt"), result.stdout).unwrap();
        }
    }
}

struct ExtraCheckedMetadata<'a> {
    base: &'a Metadata,
    h: Hierarchy,
}
impl RegionMetadata for ExtraCheckedMetadata<'_> {
    fn region_scope(&self) -> &scopes::RegionScope {
        self.base.region_scope()
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.h
    }
    fn is_catch_all(&self, key: &str) -> bool {
        self.base.is_catch_all(key)
    }
    fn catch_type(&self, key: &str) -> Option<JavaType> {
        if key == "iae" {
            Some(JavaType::Class("java.lang.InterruptedException".into()))
        } else {
            self.base.catch_type(key)
        }
    }
}
#[test]
fn nonempty_checked_contract_still_rejects_unreachable_native_catch() {
    let (ir, md, bindings, owner, _) = fixture(0);
    let md = md
        .iter()
        .map(|base| {
            let mut h = hierarchy();
            h.insert(Declaration {
                name: "java.lang.InterruptedException".into(),
                superclass: Some("java.lang.Exception".into()),
                interfaces: vec![],
                is_interface: false,
            })
            .unwrap();
            ExtraCheckedMetadata { base, h }
        })
        .collect::<Vec<_>>();
    let tables = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let p = plan(
        &ir,
        &tables,
        &bindings
            .parameters
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>(),
        &Resolver { missing: false },
        &HashMap::new(),
    )
    .unwrap();
    assert!(!p.rethrows.outer_monitors[&owner].is_empty());
    let result = emit_method(
        &ir,
        &method(),
        &bindings,
        &Resolver { missing: false },
        &tables,
    );
    assert!(
        matches!(result, SourceResult::Unsupported(Unsupported::Type(ref reason)) if reason.contains("encoded checked catch is unreachable")),
        "{result:?}"
    );
}

struct ResourceResolver;
impl JavaResolver for ResourceResolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            match r.index() {
                0 => "java.lang.Object",
                1 => "java.lang.Throwable",
                _ => "ResourceCleanup.Cursor",
            }
            .into(),
        )
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "ResourceCleanup".into(),
            name: match r.index() {
                0 => "acquire",
                1 => "work",
                _ => "close",
            }
            .into(),
            is_static: true,
            parameters: vec![
                JavaType::Class("ResourceCleanup.Cursor".into()),
                JavaType::Int,
            ],
            return_type: match r.index() {
                0 => JavaType::Class("ResourceCleanup.Cursor".into()),
                1 => JavaType::Int,
                _ => JavaType::Void,
            },
            throws: vec![],
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        let mut h = hierarchy();
        h.insert(Declaration {
            name: "ResourceCleanup.Cursor".into(),
            superclass: Some("java.lang.Object".into()),
            interfaces: vec![],
            is_interface: false,
        })
        .unwrap();
        h.assignable(a, b) == Compatibility::Assignable
    }
}
fn resource_fixture(
    changed_identity: bool,
) -> (
    MethodIr<SsaForm>,
    Vec<Metadata>,
    DeclarationBindings,
    BlockId,
) {
    resource_fixture_variant(changed_identity, false)
}
fn resource_fixture_variant(
    changed_identity: bool,
    wrong_release: bool,
) -> (
    MethodIr<SsaForm>,
    Vec<Metadata>,
    DeclarationBindings,
    BlockId,
) {
    let mut b = Builder::new();
    let lock = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let supplied = b.incoming(ManagedType::Reference(ClassRef::new(2)));
    let mode = b.incoming(ManagedType::Int);
    let foreign = b.incoming(ManagedType::Reference(ClassRef::new(1)));
    let first = b.cfg.entry();
    let null = b.value(first, ManagedType::Null);
    b.set(
        first,
        ManagedInsn::Const {
            result: null,
            value: managed_ir::ops::Constant::Zero,
        },
    );
    let enter = b.block();
    b.link(first, enter);
    b.set(enter, ManagedInsn::MonitorEnter { object: lock });
    let acquire = b.block();
    b.link(enter, acquire);
    let cursor = b.value(acquire, ManagedType::Reference(ClassRef::new(2)));
    b.set(
        acquire,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(cursor),
            method: MethodRef::new(0),
            args: vec![supplied, mode],
        },
    );
    let work = b.block();
    b.link(acquire, work);
    let result = b.value(work, ManagedType::Int);
    b.set(
        work,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(1),
            args: vec![cursor, mode],
        },
    );
    let normal_close = b.block();
    b.link(work, normal_close);
    b.set(
        normal_close,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(2),
            args: vec![cursor, mode],
        },
    );
    let normal_release = b.block();
    b.link(normal_close, normal_release);
    b.set(normal_release, ManagedInsn::MonitorExit { object: lock });
    let ret = b.block();
    b.link(normal_release, ret);
    b.set(
        ret,
        ManagedInsn::Return {
            value: Some(result),
        },
    );
    let entry = b.block();
    let a = b.edge(acquire, entry, EdgeKind::Exception("all".into()));
    let w = b.edge(work, entry, EdgeKind::Exception("all".into()));
    let before = b.value(entry, ManagedType::Reference(ClassRef::new(2)));
    b.set(
        entry,
        ManagedInsn::Phi {
            result: before,
            inputs: vec![(a, null), (w, cursor)],
        },
    );
    let capture = b.block();
    b.link(entry, capture);
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
    let resource_close = b.block();
    b.link(capture, resource_close);
    b.set(
        resource_close,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(2),
            args: vec![before, mode],
        },
    );
    let resource_throw = b.block();
    b.link(resource_close, resource_throw);
    b.set(
        resource_throw,
        ManagedInsn::Throw {
            value: if changed_identity { foreign } else { caught },
        },
    );
    let monitor_capture = b.block();
    for site in [normal_close, resource_close, resource_throw] {
        b.edge(site, monitor_capture, EdgeKind::Exception("all".into()));
    }
    let replacement = b.value(monitor_capture, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        monitor_capture,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![replacement],
            effects: EffectFlags::MAY_THROW,
        },
    );
    let release = b.block();
    b.link(monitor_capture, release);
    b.set(
        release,
        ManagedInsn::MonitorExit {
            object: if wrong_release { supplied } else { lock },
        },
    );
    let escaped = b.block();
    b.link(release, escaped);
    b.set(escaped, ManagedInsn::Throw { value: replacement });
    let md = vec![
        Metadata {
            scope: scopes::RegionScope {
                protected: vec![acquire, work],
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry,
                }],
                continuation: None,
            },
            h: hierarchy(),
        },
        Metadata {
            scope: scopes::RegionScope {
                protected: vec![normal_close, resource_close, resource_throw],
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: monitor_capture,
                }],
                continuation: None,
            },
            h: hierarchy(),
        },
    ];
    (
        b.finish(),
        md,
        DeclarationBindings {
            parameters: vec![Some(lock), Some(supplied), Some(mode), Some(foreign)],
            ..Default::default()
        },
        entry,
    )
}
fn resource_method() -> JavaMethod {
    JavaMethod {
        owner: "ResourceCleanup".into(),
        name: "run".into(),
        is_static: true,
        parameters: vec![
            JavaType::Class("java.lang.Object".into()),
            JavaType::Class("ResourceCleanup.Cursor".into()),
            JavaType::Int,
            JavaType::Class("java.lang.Throwable".into()),
        ],
        return_type: JavaType::Int,
        throws: vec![],
    }
}
#[test]
fn held_resource_cleanup_keeps_precise_rethrow_snapshot_and_replacement_identity() {
    let (ir, md, bindings, _) = resource_fixture(false);
    let tables = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let output = emit_method(
        &ir,
        &resource_method(),
        &bindings,
        &ResourceResolver,
        &tables,
    );
    let SourceResult::Complete { source, .. } = output else {
        panic!("{output:?}")
    };
    let dir = std::path::PathBuf::from("/tmp/opencode/wave14-resource-cleanup-runtime");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("emitted-method.java.txt"), &source).unwrap();
    let java = format!(
        r#"public class ResourceCleanup {{static class Cursor{{int closes;}}static Object lock;static String events;static final Error first=new AssertionError("first"),second=new AssertionError("close");static final java.io.IOException foreign=new java.io.IOException("first"),replacement=new java.io.IOException("close");@SuppressWarnings("unchecked")static <T extends Throwable>void boundary(Throwable t)throws T{{throw(T)t;}}
 static Cursor acquire(Cursor supplied,int mode){{if(!Thread.holdsLock(lock))throw new AssertionError("acquire scope");events+="A";if(mode==1||mode==7)throw first;if(mode==8)boundary(foreign);return supplied;}}
 static int work(Cursor cursor,int mode){{if(!Thread.holdsLock(lock)||cursor==null)throw new AssertionError("work snapshot");events+="W";if(mode==2||mode==6)throw first;if(mode==3||mode==5)boundary(foreign);return 7;}}
 static void close(Cursor cursor,int mode){{if(!Thread.holdsLock(lock))throw new AssertionError("close after release");events+=cursor==null?"N":"C";if(cursor!=null)cursor.closes++;if(mode==4||mode==5||mode==7)throw second;if(mode==6)boundary(replacement);}}
 static int original(Object held,Cursor supplied,int mode){{synchronized(held){{Cursor cursor=null;int value;try{{cursor=acquire(supplied,mode);value=work(cursor,mode);}}catch(Throwable caught){{close(cursor,mode);throw caught;}}close(cursor,mode);return value;}}}}
 {source}
 static String observe(boolean emitted,int mode){{lock=new Object();Cursor supplied=new Cursor();events="";String outcome;try{{outcome="return:"+(emitted?run(lock,supplied,mode,null):original(lock,supplied,mode));}}catch(Throwable t){{Throwable expected=mode==4||mode==5||mode==7?second:mode==6?replacement:mode==3||mode==8?foreign:first;if(t!=expected)throw new AssertionError("changed identity",t);outcome="throw:"+t.getClass().getName();}}if(Thread.holdsLock(lock))throw new AssertionError("leaked lock");return outcome+":"+events+":closes="+supplied.closes;}}
 public static void main(String[]args){{for(int mode=0;mode<9;mode++){{String expected=observe(false,mode),actual=observe(true,mode);if(!expected.equals(actual))throw new AssertionError(mode+" "+expected+" != "+actual);System.out.println(mode+" "+actual);}}}}
 }}"#
    );
    std::fs::write(dir.join("ResourceCleanup.java"), java).unwrap();
    let home = std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11");
    for (tool, args) in [
        ("javac", vec!["--release", "17", "ResourceCleanup.java"]),
        ("java", vec!["-Xverify:all", "-cp", ".", "ResourceCleanup"]),
    ] {
        let r = std::process::Command::new(home.join("bin").join(tool))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        if tool == "java" {
            std::fs::write(dir.join("runtime.txt"), r.stdout).unwrap();
        }
    }
}
#[test]
fn held_resource_cleanup_rejects_replaced_caught_value() {
    let (ir, md, bindings, _) = resource_fixture(true);
    let tables = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    assert!(matches!(
        emit_method(
            &ir,
            &resource_method(),
            &bindings,
            &ResourceResolver,
            &tables
        ),
        SourceResult::Unsupported(_)
    ));
}

struct ResourceBoundaryResolver {
    missing: bool,
    checked: bool,
}
impl JavaResolver for ResourceBoundaryResolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        ResourceResolver.class(r)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        if self.missing && r.index() == 2 {
            return None;
        }
        let mut m = ResourceResolver.method(r)?;
        if self.checked && r.index() == 2 {
            m.throws.push("java.io.IOException".into());
        }
        Some(m)
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        ResourceResolver.field(r)
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        ResourceResolver.string(r)
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        ResourceResolver.assignable(a, b)
    }
}
#[test]
fn held_resource_cleanup_rejects_missing_cleanup_contract_or_uncovered_checked_throw() {
    let (ir, md, bindings, _) = resource_fixture(false);
    let tables = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    for resolver in [
        ResourceBoundaryResolver {
            missing: true,
            checked: false,
        },
        ResourceBoundaryResolver {
            missing: false,
            checked: true,
        },
    ] {
        let result = emit_method(&ir, &resource_method(), &bindings, &resolver, &tables);
        assert!(matches!(result, SourceResult::Unsupported(_)), "{result:?}");
    }
}
#[test]
fn held_resource_cleanup_rejects_other_monitor_identity() {
    let (ir, md, bindings, _) = resource_fixture_variant(false, true);
    let tables = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let result = emit_method(
        &ir,
        &resource_method(),
        &bindings,
        &ResourceResolver,
        &tables,
    );
    assert!(matches!(result, SourceResult::Unsupported(_)), "{result:?}");
}
#[test]
fn held_resource_cleanup_rejects_typed_release_handler() {
    let (ir, mut md, bindings, _) = resource_fixture(false);
    md[1].scope.catches[0].key = "io".into();
    let tables = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let result = emit_method(
        &ir,
        &resource_method(),
        &bindings,
        &ResourceResolver,
        &tables,
    );
    assert!(matches!(result, SourceResult::Unsupported(_)), "{result:?}");
}

#[path = "exception_dispatch_wait_cleanup_tests.rs"]
mod wait_cleanup_tests;
