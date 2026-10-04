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
        ("SuppressionFixture.R", Some("java.lang.Object")),
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
    missing: bool,
}
impl JavaResolver for R {
    fn field(&self, _: managed_ir::refs::FieldRef) -> Option<crate::JavaField> {
        None
    }
    fn string(&self, _: managed_ir::refs::StringRef) -> Option<Vec<u16>> {
        None
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r.index() == 0 {
                "SuppressionFixture.R"
            } else {
                "java.lang.Throwable"
            }
            .into(),
        )
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        if self.missing && r.index() == 1 {
            return None;
        }
        let (owner, name, parameters, throws) = match r.index() {
            0 => (
                "SuppressionFixture.R",
                "body",
                vec![],
                vec!["java.io.IOException".into()],
            ),
            1 => (
                "SuppressionFixture.R",
                "close",
                vec![],
                vec!["java.io.IOException".into()],
            ),
            _ => (
                "java.lang.Throwable",
                "addSuppressed",
                vec![JavaType::Class("java.lang.Throwable".into())],
                vec![],
            ),
        };
        Some(JavaMethod {
            owner: owner.into(),
            name: name.into(),
            is_static: false,
            return_type: JavaType::Void,
            parameters,
            throws,
        })
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(1)
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
    fn catch_type(&self, _: &str) -> Option<JavaType> {
        Some(JavaType::Class("java.lang.Throwable".into()))
    }
    fn is_catch_all(&self, _: &str) -> bool {
        true
    }
}
fn capture(b: &mut Builder, block: BlockId) -> ValueId {
    let v = b.value(block, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        block,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::MAY_THROW,
        },
    );
    v
}
fn fixture(cycle: bool, foreign: bool) -> (MethodIr<SsaForm>, Vec<Md>, DeclarationBindings) {
    let mut b = Builder::new();
    let input = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let entry = b.cfg.entry();
    let normal_close = b.block();
    let ret = b.block();
    let primary = b.block();
    let close = b.block();
    let secondary = b.block();
    let suppress = b.block();
    let terminal = b.block();
    b.set(
        entry,
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: None,
            method: MethodRef::new(0),
            args: vec![input],
        },
    );
    b.link(entry, normal_close);
    b.set(
        normal_close,
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: None,
            method: MethodRef::new(1),
            args: vec![input],
        },
    );
    b.link(normal_close, ret);
    b.edge(entry, primary, EdgeKind::Exception("all".into()));
    let p = capture(&mut b, primary);
    b.link(primary, close);
    b.set(
        close,
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: None,
            method: MethodRef::new(1),
            args: vec![input],
        },
    );
    b.link(close, terminal);
    b.edge(close, secondary, EdgeKind::Exception("all".into()));
    let s = capture(&mut b, secondary);
    b.link(secondary, suppress);
    b.set(
        suppress,
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: None,
            method: MethodRef::new(2),
            args: vec![if foreign { s } else { p }, s],
        },
    );
    b.link(suppress, if cycle { close } else { terminal });
    b.set(terminal, ManagedInsn::Throw { value: p });
    let md = vec![
        Md {
            scope: scopes::RegionScope {
                protected: vec![entry],
                continuation: None,
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: primary,
                }],
            },
            h: hierarchy(),
        },
        Md {
            scope: scopes::RegionScope {
                protected: vec![close],
                continuation: None,
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: secondary,
                }],
            },
            h: hierarchy(),
        },
    ];
    (
        MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values),
        md,
        DeclarationBindings {
            receiver: None,
            parameters: vec![Some(input)],
            names: vec![],
        },
    )
}
fn method() -> JavaMethod {
    JavaMethod {
        owner: "SuppressionFixture".into(),
        name: "f".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![JavaType::Class("SuppressionFixture.R".into())],
        throws: vec!["java.io.IOException".into()],
    }
}
#[test]
fn original_suppression_order_compiles_and_runs() {
    let (ir, md, b) = fixture(false, false);
    let refs = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let out = super::super::super::emit_method(&ir, &method(), &b, &R { missing: false }, &refs);
    let SourceResult::Complete { source, .. } = out else {
        panic!("{out:?}")
    };
    let dir = std::env::temp_dir().join(format!("suppression-order-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class SuppressionFixture{{{source}
 static final java.io.IOException body=new java.io.IOException("body"), close=new java.io.IOException("close");
 static final Error error=new AssertionError("error");
 static class R{{final int mode;int closes;R(int m){{mode=m;}}void body()throws java.io.IOException{{if(mode==1||mode==3)throw body;if(mode==4)throw error;}}void close()throws java.io.IOException{{closes++;if(mode==2||mode==3||mode==4)throw close;}}}}
 public static void main(String[]a)throws Exception{{for(int m=0;m<5;m++){{R r=new R(m);Throwable got=null;try{{f(r);}}catch(Throwable t){{got=t;}}if(r.closes!=1)throw new AssertionError("close count");if(m==0&&got!=null)throw new AssertionError();if(m==1&&got!=body)throw new AssertionError();if(m==2&&got!=close)throw new AssertionError();if(m==3&&(got!=body||got.getSuppressed().length!=1||got.getSuppressed()[0]!=close))throw new AssertionError();if(m==4&&(got!=error||got.getSuppressed().length!=1||got.getSuppressed()[0]!=close))throw new AssertionError();}}}}
 }}"#
    );
    std::fs::write(dir.join("SuppressionFixture.java"), java).unwrap();
    let home = std::env::var_os("JAVA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (tool, args) in [
        ("javac", vec!["--release", "17", "SuppressionFixture.java"]),
        (
            "java",
            vec!["-Xverify:all", "-cp", ".", "SuppressionFixture"],
        ),
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
}
#[test]
fn unknown_contract_and_cycle_stay_closed() {
    for (missing, cycle) in [(true, false), (false, true)] {
        let (ir, md, b) = fixture(cycle, false);
        let refs = md
            .iter()
            .map(|m| m as &dyn RegionMetadata)
            .collect::<Vec<_>>();
        assert!(matches!(
            super::super::super::emit_method(&ir, &method(), &b, &R { missing }, &refs),
            SourceResult::Unsupported(_)
        ));
    }
}

fn nested_fixture(cycle: bool, foreign: bool) -> (MethodIr<SsaForm>, Vec<Md>, DeclarationBindings) {
    let mut b = Builder::new();
    let input = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let outer = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let entry = b.cfg.entry();
    let normal_close = b.block();
    let ret = b.block();
    let primary = b.block();
    let close = b.block();
    let secondary = b.block();
    let suppress = b.block();
    let terminal = b.block();
    b.set(
        entry,
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: None,
            method: MethodRef::new(0),
            args: vec![input],
        },
    );
    b.link(entry, normal_close);
    b.set(
        normal_close,
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: None,
            method: MethodRef::new(1),
            args: vec![input],
        },
    );
    let normal_outer = b.block();
    b.link(normal_close, normal_outer);
    b.set(
        normal_outer,
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: None,
            method: MethodRef::new(1),
            args: vec![outer],
        },
    );
    b.link(normal_outer, ret);
    b.edge(entry, primary, EdgeKind::Exception("all".into()));
    let p = capture(&mut b, primary);
    b.link(primary, close);
    b.set(
        close,
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: None,
            method: MethodRef::new(1),
            args: vec![input],
        },
    );
    b.link(close, terminal);
    b.edge(close, secondary, EdgeKind::Exception("all".into()));
    let s = capture(&mut b, secondary);
    b.link(secondary, suppress);
    b.set(
        suppress,
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: None,
            method: MethodRef::new(2),
            args: vec![if foreign { s } else { p }, s],
        },
    );
    b.link(suppress, if cycle { close } else { terminal });
    b.set(terminal, ManagedInsn::Throw { value: p });
    let mut md = vec![
        Md {
            scope: scopes::RegionScope {
                protected: vec![entry],
                continuation: None,
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: primary,
                }],
            },
            h: hierarchy(),
        },
        Md {
            scope: scopes::RegionScope {
                protected: vec![close],
                continuation: None,
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: secondary,
                }],
            },
            h: hierarchy(),
        },
    ];
    let op = b.block();
    let oc = b.block();
    let os = b.block();
    let oa = b.block();
    let ot = b.block();
    let pv = capture(&mut b, op);
    b.link(op, oc);
    b.set(
        oc,
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: None,
            method: MethodRef::new(1),
            args: vec![outer],
        },
    );
    b.link(oc, ot);
    b.edge(oc, os, EdgeKind::Exception("all".into()));
    let sv = capture(&mut b, os);
    b.link(os, oa);
    b.set(
        oa,
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: None,
            method: MethodRef::new(2),
            args: vec![pv, sv],
        },
    );
    b.link(oa, ot);
    b.set(ot, ManagedInsn::Throw { value: pv });
    for block in [normal_close, suppress, terminal] {
        b.edge(block, op, EdgeKind::Exception("all".into()));
    }
    md.push(Md {
        scope: scopes::RegionScope {
            protected: vec![normal_close, suppress, terminal],
            continuation: None,
            catches: vec![scopes::Catch {
                key: "all".into(),
                entry: op,
            }],
        },
        h: hierarchy(),
    });
    md.push(Md {
        scope: scopes::RegionScope {
            protected: vec![oc],
            continuation: None,
            catches: vec![scopes::Catch {
                key: "all".into(),
                entry: os,
            }],
        },
        h: hierarchy(),
    });
    (
        MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values),
        md,
        DeclarationBindings {
            receiver: None,
            parameters: vec![Some(input), Some(outer)],
            names: vec![],
        },
    )
}

#[test]
fn nested_resources_preserve_primary_and_suppressed_order() {
    let (ir, md, b) = nested_fixture(false, false);
    let refs = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let mut signature = method();
    signature
        .parameters
        .push(JavaType::Class("SuppressionFixture.R".into()));
    let out = super::super::super::emit_method(&ir, &signature, &b, &R { missing: false }, &refs);
    let SourceResult::Complete { source, .. } = out else {
        panic!("{out:?}")
    };
    let dir = std::env::temp_dir().join(format!("nested-suppression-order-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class SuppressionFixture{{{source}
 static String trace="";
 static class R{{final String name;final Throwable bodyError,closeError;int closes;R(String n,Throwable b,Throwable c){{name=n;bodyError=b;closeError=c;}}void body()throws java.io.IOException{{trace+="b";fail(bodyError);}}void close()throws java.io.IOException{{closes++;trace+=name;fail(closeError);}}}}
 static void fail(Throwable t)throws java.io.IOException{{if(t instanceof java.io.IOException)throw(java.io.IOException)t;if(t instanceof Error)throw(Error)t;}}
 static Throwable failure(int n,String name){{return n==0?null:n==1?new java.io.IOException(name):new AssertionError(name);}}
 public static void main(String[]a)throws Exception{{for(int b=0;b<3;b++)for(int i=0;i<3;i++)for(int o=0;o<3;o++){{Throwable be=failure(b,"b"),ie=failure(i,"i"),oe=failure(o,"o");R inner=new R("i",be,ie),outer=new R("o",null,oe);trace="";Throwable got=null;try{{f(inner,outer);}}catch(Throwable t){{got=t;}}Throwable expected=be!=null?be:ie!=null?ie:oe;if(got!=expected||inner.closes!=1||outer.closes!=1||!trace.equals("bio"))throw new AssertionError("identity/effects "+b+i+o);java.util.ArrayList<Throwable> suppressed=new java.util.ArrayList<>();if(be!=null&&ie!=null)suppressed.add(ie);if((be!=null||ie!=null)&&oe!=null)suppressed.add(oe);if(got!=null&&!java.util.Arrays.equals(got.getSuppressed(),suppressed.toArray()))throw new AssertionError("suppression order");}}}}
 }}"#
    );
    std::fs::write(dir.join("SuppressionFixture.java"), java).unwrap();
    let home = std::env::var_os("JAVA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (tool, args) in [
        ("javac", vec!["--release", "17", "SuppressionFixture.java"]),
        (
            "java",
            vec!["-Xverify:all", "-cp", ".", "SuppressionFixture"],
        ),
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
}

#[test]
fn undeclared_checked_primary_is_not_made_unchecked() {
    let (ir, md, b) = fixture(false, false);
    let refs = md
        .iter()
        .map(|m| m as &dyn RegionMetadata)
        .collect::<Vec<_>>();
    let mut signature = method();
    signature.throws.clear();
    assert!(matches!(
        super::super::super::emit_method(&ir, &signature, &b, &R { missing: false }, &refs),
        SourceResult::Unsupported(_)
    ));
}

#[test]
fn checked_domain_narrowing_retains_checked_catch() {
    let h = hierarchy();
    assert_eq!(
        caught_domain(
            &["java.lang.Exception".into()],
            &JavaType::Class("java.io.IOException".into()),
            &h
        )
        .unwrap(),
        vec!["java.io.IOException".to_string()]
    );
    assert!(
        caught_domain(
            &["MissingException".into()],
            &JavaType::Class("java.io.IOException".into()),
            &h
        )
        .is_err()
    );
}
