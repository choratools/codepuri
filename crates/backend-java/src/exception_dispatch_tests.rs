//! Runtime and proof tests for original per-instruction exception dispatch.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::{
    DeclarationBindings, SourceResult,
    type_compatibility::{Declaration, Hierarchy},
};
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    ops::{CompareOp, Constant, InvokeKind},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};
struct Metadata {
    scope: scopes::RegionScope,
    hierarchy: Hierarchy,
    all: bool,
}

fn fixture(
    bad_result: bool,
    reverse: bool,
) -> (MethodIr<SsaForm>, Metadata, DeclarationBindings, ValueId) {
    let mut b = Builder::new();
    let mode = b.incoming(ManagedType::Int);
    let object = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let choose = b.incoming(ManagedType::Boolean);
    let entry = b.cfg.entry();
    let old = b.value(entry, ManagedType::Int);
    b.set(
        entry,
        ManagedInsn::Const {
            result: old,
            value: Constant::Int(10),
        },
    );
    let zero_block = b.block();
    let zero = b.value(zero_block, ManagedType::Int);
    b.set(
        zero_block,
        ManagedInsn::Const {
            result: zero,
            value: Constant::Int(0),
        },
    );
    b.link(entry, zero_block);
    let head = b.block();
    let count = b.value(head, ManagedType::Int);
    let first = b.link(zero_block, head);
    let work = b.block();
    b.link(head, work);
    let result = b.value(work, ManagedType::Int);
    b.set(
        work,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(result),
            method: MethodRef::new(0),
            args: vec![count, object],
        },
    );
    let check = b.block();
    b.link(work, check);
    let keep = b.value(check, ManagedType::Boolean);
    b.set(
        check,
        ManagedInsn::Compare {
            op: CompareOp::GreaterThan,
            result: keep,
            lhs: result,
            rhs: zero,
        },
    );
    let branch = b.block();
    b.link(check, branch);
    let after = b.block();
    let back = b.edge(branch, head, EdgeKind::BranchTrue);
    let stop = b.edge(branch, after, EdgeKind::BranchFalse);
    b.set(
        branch,
        ManagedInsn::Branch {
            condition: Some(keep),
            then_edge: back,
            else_edge: Some(stop),
        },
    );
    b.set(
        head,
        ManagedInsn::Phi {
            result: count,
            inputs: vec![(first, mode), (back, result)],
        },
    );
    let ret1 = b.block();
    let ret2 = b.block();
    b.set(
        ret1,
        ManagedInsn::Return {
            value: Some(result),
        },
    );
    b.set(ret2, ManagedInsn::Return { value: Some(old) });
    let yes = b.edge(after, ret1, EdgeKind::BranchTrue);
    let no = b.edge(after, ret2, EdgeKind::BranchFalse);
    b.set(
        after,
        ManagedInsn::Branch {
            condition: Some(choose),
            then_edge: yes,
            else_edge: Some(no),
        },
    );
    let mut catches = Vec::new();
    for (index, key) in ["broad", "narrow", "all"].iter().enumerate() {
        let join = b.block();
        let copied = b.value(join, ManagedType::Int);
        // Deliberately insert CFG edges opposite to encoded priority.
        let edge = b.edge(work, join, EdgeKind::Exception((*key).into()));
        b.set(
            join,
            ManagedInsn::Phi {
                result: copied,
                inputs: vec![(
                    edge,
                    if bad_result && index == 0 {
                        result
                    } else {
                        old
                    },
                )],
            },
        );
        let capture = b.block();
        let caught = b.value(capture, ManagedType::Unknown);
        b.set(
            capture,
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads: vec![],
                writes: vec![caught],
                effects: EffectFlags::NONE,
            },
        );
        b.link(join, capture);
        let ret = b.block();
        b.link(capture, ret);
        if index == 0 {
            b.set(
                ret,
                ManagedInsn::Return {
                    value: Some(copied),
                },
            );
        } else {
            let value = b.value(ret, ManagedType::Int);
            b.set(
                ret,
                ManagedInsn::Const {
                    result: value,
                    value: Constant::Int(if index == 1 { 20 } else { 30 }),
                },
            );
            let terminal = b.block();
            b.link(ret, terminal);
            b.set(terminal, ManagedInsn::Return { value: Some(value) });
        }
        catches.push(scopes::Catch {
            key: (*key).into(),
            entry: join,
        });
    }
    if reverse {
        catches.swap(0, 1);
    }
    (
        b.finish(),
        Metadata {
            scope: scopes::RegionScope {
                protected: vec![head, work, check, branch, after],
                catches,
                continuation: None,
            },
            hierarchy: hierarchy(),
            all: true,
        },
        DeclarationBindings {
            parameters: vec![Some(mode), Some(object), Some(choose)],
            ..Default::default()
        },
        result,
    )
}

fn method(name: &str) -> JavaMethod {
    JavaMethod {
        owner: "RegionRuntime".into(),
        name: name.into(),
        is_static: true,
        return_type: JavaType::Int,
        parameters: vec![
            JavaType::Int,
            JavaType::Class("java.lang.Object".into()),
            JavaType::Boolean,
        ],
        throws: vec![],
    }
}

#[test]
fn shared_handler_uses_each_original_site_snapshot_and_rethrows_unmatched_identity() {
    let mut b = Builder::new();
    let n = b.incoming(ManagedType::Int);
    let object = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let choose = b.incoming(ManagedType::Boolean);
    let first = b.cfg.entry();
    let one = b.value(first, ManagedType::Int);
    b.set(
        first,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(one),
            method: MethodRef::new(0),
            args: vec![n, object],
        },
    );
    let second = b.block();
    let two = b.value(second, ManagedType::Int);
    b.set(
        second,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(two),
            method: MethodRef::new(0),
            args: vec![one, object],
        },
    );
    b.link(first, second);
    let success = b.block();
    b.set(success, ManagedInsn::Return { value: Some(two) });
    b.link(second, success);
    let join = b.block();
    let before = b.value(join, ManagedType::Int);
    let mut inputs = Vec::new();
    for (source, value) in [(first, n), (second, one)] {
        for key in ["narrow", "broad"] {
            let edge = b.edge(source, join, EdgeKind::Exception(key.into()));
            inputs.push((edge, value));
        }
    }
    b.set(
        join,
        ManagedInsn::Phi {
            result: before,
            inputs,
        },
    );
    let capture = b.block();
    let caught = b.value(capture, ManagedType::Unknown);
    b.set(
        capture,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![caught],
            effects: EffectFlags::NONE,
        },
    );
    b.link(join, capture);
    let ret = b.block();
    b.set(
        ret,
        ManagedInsn::Return {
            value: Some(before),
        },
    );
    b.link(capture, ret);
    let md = Metadata {
        scope: scopes::RegionScope {
            protected: vec![first, second],
            catches: vec![
                scopes::Catch {
                    key: "broad".into(),
                    entry: join,
                },
                scopes::Catch {
                    key: "narrow".into(),
                    entry: join,
                },
            ],
            continuation: Some(success),
        },
        hierarchy: hierarchy(),
        all: false,
    };
    let ir = b.finish();
    let bindings = DeclarationBindings {
        parameters: vec![Some(n), Some(object), Some(choose)],
        ..Default::default()
    };
    let source = match emit_method(&ir, &method("shared"), &bindings, &Resolver, &[&md]) {
        SourceResult::Complete { source, .. } => source,
        error => panic!("shared handler: {error:?}"),
    };
    let dir = std::env::temp_dir().join(format!(
        "decomphify-exception-shared-dispatch-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class RegionRuntime {{
      static int stores; static AssertionError original;
      public static int work(int n,Object o) {{stores++; if(n == -1) throw new ArithmeticException(); if(n == -2) {{original=new AssertionError();throw original;}} return n-1;}}
      {source}
      public static void main(String[] args) {{
        stores=0;if(shared(-1,null,true)!=-1||stores!=1)throw new AssertionError("first snapshot");
        stores=0;if(shared(0,null,true)!=-1||stores!=2)throw new AssertionError("second snapshot");
        stores=0;if(shared(4,null,true)!=2||stores!=2)throw new AssertionError("normal result");
        try {{shared(-2,null,true);throw new AssertionError("lost throw");}}catch(AssertionError e){{if(e!=original)throw new AssertionError("changed throw identity");}}
        System.out.print("shared-dispatch-ok");
      }}
    }}"#
    );
    std::fs::write(dir.join("RegionRuntime.java"), &java).unwrap();
    let compile = std::process::Command::new("javac")
        .arg("--release")
        .arg("17")
        .arg("RegionRuntime.java")
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
        .arg("RegionRuntime")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "shared-dispatch-ok");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn prethrow_result_cannot_feed_exception_phi_even_inside_a_loop() {
    let (ir, md, b, result) = fixture(true, false);
    assert!(
        matches!(plan(&ir,&[&md],&b.parameters.iter().flatten().copied().collect::<Vec<_>>(),&Resolver,&HashMap::new()), Err(Error::Unavailable(v)) if v == result)
    );
}

#[test]
fn loop_multi_exit_priority_prethrow_phi_and_store_effects_compile_and_run() {
    let mut declarations = Vec::new();
    for (name, reverse) in [("broadFirst", false), ("narrowFirst", true)] {
        let (ir, md, b, _) = fixture(false, reverse);
        let source = match emit_method(&ir, &method(name), &b, &Resolver, &[&md]) {
            SourceResult::Complete { source, .. } => source,
            error => panic!("dispatch failed: {error:?}"),
        };
        assert!(source.contains("snapshot"));
        assert!(source.contains("catch (java.lang.Throwable"));
        declarations.push(source);
    }
    let dir = std::env::temp_dir().join(format!(
        "decomphify-exception-dispatch-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class RegionRuntime {{
      static int stores;
      public static int work(int n,Object o) {{ stores++; if(n == -1) throw new ArithmeticException(); if(n == -2) throw new AssertionError(); return n-1; }}
      {}
      public static void main(String[] args) {{
        stores=0; if(broadFirst(3,null,true)!=0 || stores!=3) throw new AssertionError("loop");
        stores=0; if(broadFirst(3,null,false)!=10 || stores!=3) throw new AssertionError("multi-exit");
        stores=0; if(broadFirst(-1,null,true)!=10 || stores!=1) throw new AssertionError("priority/prethrow/store");
        stores=0; if(narrowFirst(-1,null,true)!=20 || stores!=1) throw new AssertionError("encoded priority");
        stores=0; if(broadFirst(-2,null,true)!=30 || stores!=1) throw new AssertionError("catch-all");
        System.out.print("exception-dispatch-ok");
      }}
    }}"#,
        declarations.join("\n")
    );
    std::fs::write(dir.join("RegionRuntime.java"), &java).unwrap();
    let compile = std::process::Command::new("javac")
        .arg("--release")
        .arg("17")
        .arg("RegionRuntime.java")
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
        .arg("RegionRuntime")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "exception-dispatch-ok"
    );
    std::fs::remove_dir_all(dir).unwrap();
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
                "narrow" => "java.lang.ArithmeticException",
                "broad" => "java.lang.RuntimeException",
                _ => "java.lang.Throwable",
            }
            .into(),
        ))
    }
    fn is_catch_all(&self, key: &str) -> bool {
        self.all && key == "all"
    }
}
fn hierarchy() -> Hierarchy {
    let mut h = Hierarchy::default();
    for (n, p) in [
        ("java.lang.Throwable", "java.lang.Object"),
        ("java.lang.Exception", "java.lang.Throwable"),
        ("java.lang.RuntimeException", "java.lang.Exception"),
        ("java.io.IOException", "java.lang.Exception"),
        ("java.lang.Error", "java.lang.Throwable"),
        (
            "java.lang.ArithmeticException",
            "java.lang.RuntimeException",
        ),
        (
            "java.lang.IllegalArgumentException",
            "java.lang.RuntimeException",
        ),
    ] {
        h.insert(Declaration {
            name: n.into(),
            superclass: Some(p.into()),
            interfaces: vec![],
            is_interface: false,
        })
        .unwrap();
    }
    h
}

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
        target.name = "checkedWork".into();
        target.throws = vec!["java.io.IOException".into()];
        Some(target)
    }
    fn field(&self, r: FieldRef) -> Option<crate::JavaField> {
        Resolver.field(r)
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        Resolver.string(r)
    }
}
struct CheckedMetadata(Metadata);
impl RegionMetadata for CheckedMetadata {
    fn region_scope(&self) -> &scopes::RegionScope {
        &self.0.scope
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.0.hierarchy
    }
    fn catch_type(&self, key: &str) -> Option<JavaType> {
        Some(JavaType::Class(
            match key {
                "broad" => "java.lang.Exception",
                "narrow" => "java.io.IOException",
                _ => "java.lang.Throwable",
            }
            .into(),
        ))
    }
    fn is_catch_all(&self, key: &str) -> bool {
        key == "all"
    }
}

#[test]
fn declared_checked_calls_and_shadowed_encoded_handlers_compile_and_run() {
    let mut declarations = Vec::new();
    for (name, reverse) in [("broadFirst", false), ("narrowFirst", true)] {
        let (ir, md, b, _) = fixture(false, reverse);
        let source = match emit_method(
            &ir,
            &method(name),
            &b,
            &CheckedResolver,
            &[&CheckedMetadata(md)],
        ) {
            SourceResult::Complete { source, .. } => source,
            error => panic!("checked dispatch failed: {error:?}"),
        };
        declarations.push(source);
    }
    let dir = std::env::temp_dir().join(format!(
        "decomphify-exception-checked-dispatch-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class RegionRuntime {{
      static int stores;
      public static int checkedWork(int n,Object o) throws java.io.IOException {{ stores++; if(n == -1) throw new java.io.IOException(); if(n == -2) throw new AssertionError(); if(n == -3) throw new IllegalArgumentException(); return n-1; }}
      {}
      public static void main(String[] args) {{
        stores=0; if(broadFirst(-1,null,true)!=10 || stores!=1) throw new AssertionError("checked broad priority");
        stores=0; if(narrowFirst(-1,null,true)!=20 || stores!=1) throw new AssertionError("checked narrow priority");
        if(narrowFirst(-3,null,true)!=10) throw new AssertionError("runtime hierarchy");
        if(broadFirst(-2,null,true)!=30) throw new AssertionError("checked catch-all");
        System.out.print("checked-dispatch-ok");
      }}
    }}"#,
        declarations.join("\n")
    );
    std::fs::write(dir.join("RegionRuntime.java"), &java).unwrap();
    let compile = std::process::Command::new("javac")
        .arg("--release")
        .arg("17")
        .arg("RegionRuntime.java")
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
        .arg("RegionRuntime")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "checked-dispatch-ok");
    std::fs::remove_dir_all(dir).unwrap();
}
struct Resolver;
impl JavaResolver for Resolver {
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        hierarchy().assignable(a, b) == crate::type_compatibility::Compatibility::Assignable
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
            owner: "RegionRuntime".into(),
            name: if r.index() == 0 { "work" } else { "cleanup" }.into(),
            is_static: true,
            return_type: if r.index() == 0 {
                JavaType::Int
            } else {
                JavaType::Void
            },
            parameters: if r.index() == 0 {
                vec![JavaType::Int, JavaType::Class("java.lang.Object".into())]
            } else {
                vec![]
            },
            throws: vec![],
        })
    }
    fn field(&self, _: FieldRef) -> Option<crate::JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
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
