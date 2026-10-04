//! Concrete Java operation origins preserve literal catch-all cleanup semantics.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::exceptions::{RegionMetadata, scopes};
use crate::{
    DeclarationBindings, JavaField, SourceResult,
    type_compatibility::{Declaration, Hierarchy},
};
use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::{BlockId, EdgeId},
};
use managed_ir::{
    method::{InsnArena, ValueArena},
    ops::InvokeKind,
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
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
        let id = self.insns.insert(ManagedInsn::Return { value: None });
        self.cfg.add_block(id)
    }
    fn value(&mut self, b: BlockId, t: ManagedType) -> ValueId {
        self.values
            .insert(Value::new(*self.cfg.block(b).unwrap().payload(), t))
    }
    fn set(&mut self, b: BlockId, insn: ManagedInsn) {
        self.insns[*self.cfg.block(b).unwrap().payload()] = insn;
    }
    fn edge(&mut self, a: BlockId, b: BlockId, k: EdgeKind) -> EdgeId {
        self.cfg.add_edge(a, b, k).unwrap()
    }
    fn link(&mut self, a: BlockId, b: BlockId) {
        self.edge(a, b, EdgeKind::Unconditional);
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
}
struct Resolver {
    missing_hierarchy: bool,
}
impl JavaResolver for Resolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r.index() == 0 {
                "java.lang.UnsupportedOperationException"
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
    fn instruction_type(&self, _: TypeRef) -> Option<JavaType> {
        None
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        let mut h = make_hierarchy();
        if self.missing_hierarchy {
            h = Hierarchy::default();
        }
        h.assignable(actual, expected) == crate::type_compatibility::Compatibility::Assignable
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(9)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "UncheckedContractHarness".into(),
            name: match r.index() {
                0 => "body",
                1 => "cleanup",
                2 => "outside",
                _ => "observe",
            }
            .into(),
            is_static: true,
            parameters: vec![if r.index() == 3 {
                JavaType::Class("java.lang.Throwable".into())
            } else {
                JavaType::Int
            }],
            return_type: JavaType::Void,
            throws: vec![],
        })
    }
}
struct Metadata {
    original: crate::exceptions::OriginalScope,
    scope: scopes::RegionScope,
    h: Hierarchy,
}
impl RegionMetadata for Metadata {
    fn original_scope(&self) -> Option<&crate::exceptions::OriginalScope> {
        Some(&self.original)
    }
    fn region_scope(&self) -> &scopes::RegionScope {
        &self.scope
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.h
    }
    fn catch_type(&self, key: &str) -> Option<JavaType> {
        Some(JavaType::Class(
            if key == "runtime" {
                "java.lang.RuntimeException"
            } else {
                "java.lang.Throwable"
            }
            .into(),
        ))
    }
    fn is_catch_all(&self, key: &str) -> bool {
        key == "all"
    }
}
fn fixture(explicit: bool) -> (MethodIr<SsaForm>, Metadata, DeclarationBindings, JavaMethod) {
    fixture_ordered(explicit, false)
}
fn fixture_ordered(
    explicit: bool,
    earlier: bool,
) -> (MethodIr<SsaForm>, Metadata, DeclarationBindings, JavaMethod) {
    let mut b = Builder::new();
    let mode = b.incoming(ManagedType::Int);
    let raise = b.incoming(ManagedType::Boolean);
    let array = b.incoming(ManagedType::Array {
        element: Box::new(ManagedType::Byte),
        nullable: true,
    });
    let index = b.incoming(ManagedType::Int);
    let primary = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let first = b.cfg.entry();
    let literal = if explicit { b.block() } else { first };
    let result = b.value(literal, ManagedType::Int);
    b.set(
        literal,
        if explicit {
            ManagedInsn::Throw { value: primary }
        } else {
            ManagedInsn::ArrayGet {
                result,
                array,
                index,
            }
        },
    );
    let body = b.block();
    if explicit {
        let yes = b.edge(first, literal, EdgeKind::BranchTrue);
        let no = b.edge(first, body, EdgeKind::BranchFalse);
        b.set(
            first,
            ManagedInsn::Branch {
                condition: Some(raise),
                then_edge: yes,
                else_edge: Some(no),
            },
        );
    } else {
        b.link(literal, body);
    }
    b.set(
        body,
        ManagedInsn::Invoke {
            result: None,
            kind: InvokeKind::Static,
            method: MethodRef::new(0),
            args: vec![mode],
        },
    );
    let normal = b.block();
    b.link(body, normal);
    b.set(
        normal,
        ManagedInsn::Invoke {
            result: None,
            kind: InvokeKind::Static,
            method: MethodRef::new(1),
            args: vec![mode],
        },
    );
    let outside = b.block();
    b.link(normal, outside);
    b.set(
        outside,
        ManagedInsn::Invoke {
            result: None,
            kind: InvokeKind::Static,
            method: MethodRef::new(2),
            args: vec![mode],
        },
    );
    let ret = b.block();
    b.link(outside, ret);
    b.set(ret, ManagedInsn::Return { value: None });
    let handler = b.block();
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
    let record = b.block();
    b.link(handler, record);
    b.set(
        record,
        ManagedInsn::Invoke {
            result: None,
            kind: InvokeKind::Static,
            method: MethodRef::new(3),
            args: vec![caught],
        },
    );
    let cleanup = b.block();
    b.link(record, cleanup);
    b.set(
        cleanup,
        ManagedInsn::Invoke {
            result: None,
            kind: InvokeKind::Static,
            method: MethodRef::new(1),
            args: vec![mode],
        },
    );
    let rethrow = b.block();
    b.link(cleanup, rethrow);
    b.set(rethrow, ManagedInsn::Throw { value: caught });
    for source in [literal, body] {
        if earlier {
            b.edge(source, handler, EdgeKind::Exception("runtime".into()));
        }
        b.edge(source, handler, EdgeKind::Exception("all".into()));
    }
    let original = crate::exceptions::OriginalScope {
        ordinal: 0,
        start: 0,
        end: 3,
        handler_offset: 1,
        catches: if earlier {
            vec![("runtime".into(), 100), ("all".into(), 100)]
        } else {
            vec![("all".into(), 100)]
        },
        offsets: HashMap::from([
            (*b.cfg.block(first).unwrap().payload(), 0),
            (*b.cfg.block(literal).unwrap().payload(), 1),
            (*b.cfg.block(body).unwrap().payload(), 2),
            (*b.cfg.block(normal).unwrap().payload(), 3),
            (*b.cfg.block(outside).unwrap().payload(), 4),
            (*b.cfg.block(handler).unwrap().payload(), 100),
        ]),
    };
    let h = make_hierarchy();
    (
        MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values),
        Metadata {
            original,
            scope: scopes::RegionScope {
                protected: if explicit {
                    vec![first, literal, body]
                } else {
                    vec![literal, body]
                },
                catches: if earlier {
                    vec![
                        scopes::Catch {
                            key: "runtime".into(),
                            entry: handler,
                        },
                        scopes::Catch {
                            key: "all".into(),
                            entry: handler,
                        },
                    ]
                } else {
                    vec![scopes::Catch {
                        key: "all".into(),
                        entry: handler,
                    }]
                },
                continuation: Some(normal),
            },
            h,
        },
        DeclarationBindings {
            receiver: None,
            parameters: vec![
                Some(mode),
                Some(raise),
                Some(array),
                Some(index),
                Some(primary),
            ],
            names: vec![],
        },
        JavaMethod {
            owner: "UncheckedContractHarness".into(),
            name: "generated".into(),
            is_static: true,
            parameters: vec![
                JavaType::Int,
                JavaType::Boolean,
                JavaType::Array(Box::new(JavaType::Byte)),
                JavaType::Int,
                JavaType::Class("java.lang.UnsupportedOperationException".into()),
            ],
            return_type: JavaType::Void,
            throws: vec![],
        },
    )
}

fn make_hierarchy() -> Hierarchy {
    let mut h = Hierarchy::default();
    for (n, p) in [
        ("java.lang.Object", None),
        ("java.lang.Throwable", Some("java.lang.Object")),
        ("java.lang.Exception", Some("java.lang.Throwable")),
        ("java.lang.RuntimeException", Some("java.lang.Exception")),
        ("java.lang.Error", Some("java.lang.Throwable")),
        (
            "java.lang.UnsupportedOperationException",
            Some("java.lang.RuntimeException"),
        ),
        ("java.io.IOException", Some("java.lang.Exception")),
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

#[test]
fn unchecked_nonempty_throw_contract_keeps_identity_and_cleanup_scope() {
    let (ir, md, b, m) = fixture(true);
    let r = Resolver {
        missing_hierarchy: false,
    };
    let out = crate::exceptions::dispatch::emit_method(&ir, &m, &b, &r, &[&md]);
    let SourceResult::Complete { source, .. } = out else {
        panic!("{out:?}")
    };
    assert!(!source.contains("throws java.lang.Throwable"));
    assert!(!source.contains("throws java.lang.UnsupportedOperationException"));
    let dir = std::env::temp_dir().join(format!(
        "unchecked-original-contract-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class UncheckedContractHarness{{
 static String trace;static Throwable captured;static final Error PRIMARY_ERROR=new AssertionError("body"),OVERRIDE=new AssertionError("cleanup"),OUTSIDE=new AssertionError("outside");
 static void body(int mode){{trace+="B";if(mode==1||mode==2)throw PRIMARY_ERROR;}}
 static void observe(Throwable t){{trace+="R";captured=t;}}
 static void cleanup(int mode){{trace+="C";if(mode==2||mode==4)throw OVERRIDE;}}
 static void outside(int mode){{trace+="O";if(mode==3)throw OUTSIDE;}}
 {source}
 public static void main(String[]args){{for(int mode=0;mode<5;mode++)for(boolean raise:new boolean[]{{false,true}}){{trace="";captured=null;UnsupportedOperationException primary=new UnsupportedOperationException("original");Throwable failure=null;try{{generated(mode,raise,new byte[]{{7}},0,primary);}}catch(Throwable t){{failure=t;}}
 if(raise){{if(mode==2||mode==4){{if(failure!=OVERRIDE)throw new AssertionError("override");}}else if(failure!=primary||failure!=captured)throw new AssertionError("identity");if(!trace.equals("RC"))throw new AssertionError(trace);}}
 else{{if(mode==0&&failure!=null)throw new AssertionError("normal");if(mode==1&&failure!=PRIMARY_ERROR)throw new AssertionError("body");if((mode==2||mode==4)&&failure!=OVERRIDE)throw new AssertionError("override");if(mode==3&&failure!=OUTSIDE)throw new AssertionError("outside");String expected=mode==1||mode==2?"BRC":mode==4?"BC":"BCO";if(!trace.equals(expected))throw new AssertionError(trace);}}
 }}}}}}"#
    );
    std::fs::write(dir.join("UncheckedContractHarness.java"), java).unwrap();
    let c = std::process::Command::new("javac")
        .current_dir(&dir)
        .arg("UncheckedContractHarness.java")
        .output()
        .unwrap();
    assert!(c.status.success(), "{}", String::from_utf8_lossy(&c.stderr));
    let run = std::process::Command::new("java")
        .current_dir(&dir)
        .args(["-cp", ".", "UncheckedContractHarness"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
#[test]
fn unknown_mixed_checked_and_broad_domains_never_authorize_unchecked_rethrow() {
    let h = make_hierarchy();
    assert!(unchecked_throw_contract(
        &["java.lang.UnsupportedOperationException".into()],
        &h
    ));
    for types in [
        vec![],
        vec!["java.io.IOException".into()],
        vec![
            "java.lang.UnsupportedOperationException".into(),
            "java.io.IOException".into(),
        ],
        vec!["UnknownThrowable".into()],
        vec!["java.lang.Throwable".into()],
    ] {
        assert!(!unchecked_throw_contract(&types, &h));
    }
    assert!(!unchecked_throw_contract(
        &["java.lang.RuntimeException".into()],
        &Hierarchy::default()
    ));
}
#[test]
fn earlier_typed_catch_is_not_replaced_by_single_catchall_permission() {
    let (ir, md, b, m) = fixture_ordered(true, true);
    let r = Resolver {
        missing_hierarchy: false,
    };
    assert!(matches!(
        crate::exceptions::dispatch::emit_method(&ir, &m, &b, &r, &[&md]),
        SourceResult::Unsupported(_)
    ));
}
