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
        ("SharedCleanupFixture.R", Some("java.lang.Object")),
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
                "SharedCleanupFixture.R"
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
            0 => ("SharedCleanupFixture.R", "body", vec![], vec![]),
            1 => (
                "SharedCleanupFixture.R",
                "close",
                vec![],
                vec!["java.io.IOException".into()],
            ),
            _ => (
                "SharedCleanupFixture.R",
                "log",
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
    fn catch_type(&self, key: &str) -> Option<JavaType> {
        Some(JavaType::Class(
            if key == "io" {
                "java.io.IOException"
            } else {
                "java.lang.Throwable"
            }
            .into(),
        ))
    }
    fn is_catch_all(&self, key: &str) -> bool {
        key != "io"
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

fn fixture(
    foreign: bool,
    cycle: bool,
    foreign_entry: bool,
) -> (MethodIr<SsaForm>, Vec<Md>, DeclarationBindings) {
    let mut b = Builder::new();
    let input = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let first = b.cfg.entry();
    let second = b.block();
    let ret = b.block();
    let one = b.block();
    let two = b.block();
    let join = b.block();
    let close = b.block();
    let secondary = b.block();
    let log = b.block();
    let end = b.block();
    for block in [first, second] {
        b.set(
            block,
            ManagedInsn::Invoke {
                kind: InvokeKind::Virtual,
                result: None,
                method: MethodRef::new(0),
                args: vec![input],
            },
        );
    }
    b.link(first, second);
    b.link(second, ret);
    b.edge(first, one, EdgeKind::Exception("all".into()));
    b.edge(second, two, EdgeKind::Exception("all".into()));
    let p = capture(&mut b, one);
    let q = capture(&mut b, two);
    let e1 = b.link(one, join);
    let e2 = b.link(two, join);
    let selected = b.value(join, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        join,
        ManagedInsn::Phi {
            result: selected,
            inputs: vec![(e1, p), (e2, q)],
        },
    );
    b.link(join, close);
    b.set(
        close,
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: None,
            method: MethodRef::new(1),
            args: vec![input],
        },
    );
    b.link(close, end);
    b.edge(close, secondary, EdgeKind::Exception("io".into()));
    let s = capture(&mut b, secondary);
    b.link(secondary, log);
    b.set(
        log,
        ManagedInsn::Invoke {
            kind: InvokeKind::Virtual,
            result: None,
            method: MethodRef::new(2),
            args: vec![input, s],
        },
    );
    b.link(log, if cycle { close } else { end });
    if foreign_entry {
        let edge = b.link(ret, join);
        b.set(
            ret,
            ManagedInsn::Branch {
                condition: None,
                then_edge: edge,
                else_edge: None,
            },
        );
    }
    b.set(
        end,
        ManagedInsn::Throw {
            value: if foreign { input } else { selected },
        },
    );
    let md = vec![
        Md {
            scope: scopes::RegionScope {
                protected: vec![first, second],
                continuation: None,
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: one,
                }],
            },
            h: hierarchy(),
        },
        Md {
            scope: scopes::RegionScope {
                protected: vec![second],
                continuation: None,
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: two,
                }],
            },
            h: hierarchy(),
        },
        Md {
            scope: scopes::RegionScope {
                protected: vec![close],
                continuation: None,
                catches: vec![scopes::Catch {
                    key: "io".into(),
                    entry: secondary,
                }],
            },
            h: hierarchy(),
        },
    ];
    // Each original throwing site has its own exact encoded primary target.
    let mut md = md;
    md[0].scope.protected = vec![first];
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
        owner: "SharedCleanupFixture".into(),
        name: "f".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![JavaType::Class("SharedCleanupFixture.R".into())],
        throws: vec![],
    }
}
#[test]
fn shared_primary_cleanup_compiles_and_preserves_identity() {
    let (ir, md, b) = fixture(false, false, false);
    let metadata: Vec<&dyn RegionMetadata> = md.iter().map(|m| m as &dyn RegionMetadata).collect();
    let result =
        super::super::super::emit_method(&ir, &method(), &b, &R { missing: false }, &metadata);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    let dir = std::env::temp_dir().join(format!("decomp-shared-cleanup-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class SharedCleanupFixture {{
      static class R {{ int calls, failAt; Throwable primary, logger; boolean failClose; String trace="";
        void body() {{ trace+="b"; if (++calls==failAt) sneaky(primary); }}
        void close() throws java.io.IOException {{ trace+="c"; if(failClose) throw new java.io.IOException(); }}
        void log(Throwable x) {{ trace+="l"; if(logger!=null) sneaky(logger); }}
      }}
      static <E extends Throwable> void sneaky(Throwable t) throws E {{ throw (E)t; }}
      {source}
      public static void main(String[] args) {{
        for (int at=1;at<=2;at++) for (int kind=0;kind<3;kind++) for(boolean close:new boolean[]{{false,true}}) for(boolean log:new boolean[]{{false,true}}) {{
          R r=new R(); r.failAt=at; r.primary=kind==0?new RuntimeException():kind==1?new Error():new java.io.IOException(); r.failClose=close; r.logger=log?new Error():null;
          try {{ f(r); throw new AssertionError(); }} catch(Throwable t) {{ if(t!=(close&&log?r.logger:r.primary)) throw new AssertionError(t); }}
          if(!r.trace.equals((at==1?"b":"bb")+(close?"cl":"c"))) throw new AssertionError(r.trace);
        }}
      }}
    }}"#
    );
    std::fs::write(dir.join("SharedCleanupFixture.java"), java).unwrap();
    let out = std::process::Command::new("javac")
        .current_dir(&dir)
        .arg("SharedCleanupFixture.java")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        std::process::Command::new("java")
            .current_dir(&dir)
            .args(["-cp", ".", "SharedCleanupFixture"])
            .status()
            .unwrap()
            .success()
    );
}
#[test]
fn foreign_primary_and_missing_close_contract_reject() {
    for (foreign, missing, cycle, foreign_entry) in [
        (true, false, false, false),
        (false, true, false, false),
        (false, false, true, false),
        (false, false, false, true),
    ] {
        let (ir, md, b) = fixture(foreign, cycle, foreign_entry);
        let metadata: Vec<&dyn RegionMetadata> =
            md.iter().map(|m| m as &dyn RegionMetadata).collect();
        assert!(matches!(
            super::super::super::emit_method(&ir, &method(), &b, &R { missing }, &metadata),
            SourceResult::Unsupported(_)
        ));
    }
}
