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
    ops::{Constant, InvokeKind},
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
    missing: bool,
    primitive: bool,
}
impl JavaResolver for Resolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r.index() == 0 {
                "java.lang.Object"
            } else {
                "java.lang.String"
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
        if self.missing {
            None
        } else {
            Some(if self.primitive {
                JavaType::Int
            } else {
                JavaType::Class("java.lang.String".into())
            })
        }
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "ReferenceCastHarness".into(),
            name: match r.index() {
                0 => "produce",
                1 => "recycleFirst",
                _ => "recycleSecond",
            }
            .into(),
            is_static: true,
            parameters: match r.index() {
                0 => vec![JavaType::Int],
                1 => vec![JavaType::Class("java.lang.Throwable".into())],
                _ => vec![],
            },
            return_type: if r.index() == 0 {
                JavaType::Class("java.lang.Object".into())
            } else {
                JavaType::Void
            },
            throws: if r.index() == 0 {
                vec!["java.io.IOException".into()]
            } else {
                vec![]
            },
        })
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        a == b || matches!(b,JavaType::Class(n) if n=="java.lang.Object")
    }
}
struct Metadata {
    original: crate::exceptions::OriginalScope,
    scope: scopes::RegionScope,
    hierarchy: Hierarchy,
}
impl RegionMetadata for Metadata {
    fn original_scope(&self) -> Option<&crate::exceptions::OriginalScope> {
        Some(&self.original)
    }
    fn region_scope(&self) -> &scopes::RegionScope {
        &self.scope
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.hierarchy
    }
    fn catch_type(&self, _: &str) -> Option<JavaType> {
        Some(JavaType::Class("java.lang.Throwable".into()))
    }
    fn is_catch_all(&self, _: &str) -> bool {
        true
    }
}
fn fixture() -> (MethodIr<SsaForm>, Metadata, DeclarationBindings, JavaMethod) {
    let mut b = Builder::new();
    let mode = b.incoming(ManagedType::Int);
    let first = b.cfg.entry();
    let object = b.value(first, ManagedType::Reference(ClassRef::new(0)));
    b.set(
        first,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: Some(object),
            method: MethodRef::new(0),
            args: vec![mode],
        },
    );
    let cast = b.block();
    let result = b.value(cast, ManagedType::Reference(ClassRef::new(1)));
    b.set(
        cast,
        ManagedInsn::Cast {
            result,
            value: object,
            target: TypeRef::new(0),
        },
    );
    b.link(first, cast);
    let normal = b.block();
    let null = b.value(normal, ManagedType::Null);
    b.set(
        normal,
        ManagedInsn::Const {
            result: null,
            value: Constant::Null,
        },
    );
    b.link(cast, normal);
    let normal_first = b.block();
    b.set(
        normal_first,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(1),
            args: vec![null],
        },
    );
    b.link(normal, normal_first);
    let normal_second = b.block();
    b.set(
        normal_second,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(2),
            args: vec![],
        },
    );
    b.link(normal_first, normal_second);
    let done = b.block();
    b.set(
        done,
        ManagedInsn::Return {
            value: Some(result),
        },
    );
    b.link(normal_second, done);
    let handler = b.block();
    let caught = b.value(handler, ManagedType::Unknown);
    b.set(
        handler,
        ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![caught],
            effects: EffectFlags::NONE,
        },
    );
    let close1 = b.block();
    b.set(
        close1,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(1),
            args: vec![caught],
        },
    );
    b.link(handler, close1);
    let close2 = b.block();
    b.set(
        close2,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(2),
            args: vec![],
        },
    );
    b.link(close1, close2);
    let rethrow = b.block();
    b.set(rethrow, ManagedInsn::Throw { value: caught });
    b.link(close2, rethrow);
    for source in [first, cast] {
        b.edge(source, handler, EdgeKind::Exception("all".into()));
    }
    let offsets = [
        (*b.cfg.block(first).unwrap().payload(), 0),
        (*b.cfg.block(cast).unwrap().payload(), 1),
        (*b.cfg.block(handler).unwrap().payload(), 100),
    ]
    .into_iter()
    .collect();
    let original = crate::exceptions::OriginalScope {
        ordinal: 0,
        start: 0,
        end: 2,
        handler_offset: 1,
        catches: vec![("all".into(), 100)],
        offsets,
    };
    let mut hierarchy = Hierarchy::default();
    for (name, parent) in [
        ("java.lang.Object", None),
        ("java.lang.Throwable", Some("java.lang.Object")),
        ("java.lang.Exception", Some("java.lang.Throwable")),
        ("java.lang.RuntimeException", Some("java.lang.Exception")),
        ("java.lang.Error", Some("java.lang.Throwable")),
        ("java.io.IOException", Some("java.lang.Exception")),
        ("java.lang.String", Some("java.lang.Object")),
    ] {
        hierarchy
            .insert(Declaration {
                name: name.into(),
                superclass: parent.map(Into::into),
                interfaces: vec![],
                is_interface: false,
            })
            .unwrap();
    }
    (
        MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values),
        Metadata {
            original,
            scope: scopes::RegionScope {
                protected: vec![first, cast],
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: handler,
                }],
                continuation: Some(normal),
            },
            hierarchy,
        },
        DeclarationBindings {
            parameters: vec![Some(mode)],
            ..Default::default()
        },
        JavaMethod {
            owner: "ReferenceCastHarness".into(),
            name: "generated".into(),
            is_static: true,
            parameters: vec![JavaType::Int],
            return_type: JavaType::Class("java.lang.String".into()),
            throws: vec!["java.io.IOException".into()],
        },
    )
}
#[test]
fn exact_reference_cast_recycles_once_with_original_exception_and_cleanup_order() {
    let (ir, md, b, m) = fixture();
    let r = Resolver {
        missing: false,
        primitive: false,
    };
    let result = crate::exceptions::dispatch::emit_method(&ir, &m, &b, &r, &[&md]);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    let root = std::env::temp_dir().join(format!("reference-cast-cleanup-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let java = format!(
        r#"public class ReferenceCastHarness {{
 static String trace;static Throwable seen;static int override;
 static final java.io.IOException IO=new java.io.IOException();static final Error LINK=new NoClassDefFoundError();static final Error FIRST=new Error(),SECOND=new Error();
 static Object produce(int m)throws java.io.IOException{{trace+="b";if(m==3)throw LINK;if(m==4)throw IO;return m==0?null:m==1?"ok":Integer.valueOf(7);}}
 static void recycleFirst(Throwable t){{trace+="r";seen=t;if(override==1)throw FIRST;}}
 static void recycleSecond(){{trace+="s";if(override==2)throw SECOND;}}
 {source}
 public static void main(String[]args){{for(int m=0;m<5;m++)for(int o=0;o<3;o++){{trace="";seen=null;override=o;Throwable caught=null;String result=null;try{{result=generated(m);}}catch(Throwable t){{caught=t;}}
 if(!trace.equals(o==1?"br":"brs"))throw new AssertionError(trace);
 if(o==1&&caught!=FIRST||o==2&&caught!=SECOND)throw new AssertionError("replacement");
 if(o==0){{if(m==0&&result!=null||m==1&&!"ok".equals(result)||m==2&&(!(caught instanceof ClassCastException)||caught!=seen)||m==3&&(caught!=LINK||seen!=LINK)||m==4&&(caught!=IO||seen!=IO))throw new AssertionError("original identity");}}
 }}}}}}"#
    );
    std::fs::write(root.join("ReferenceCastHarness.java"), java).unwrap();
    let c = std::process::Command::new("javac")
        .arg(root.join("ReferenceCastHarness.java"))
        .output()
        .unwrap();
    assert!(c.status.success(), "{}", String::from_utf8_lossy(&c.stderr));
    let run = std::process::Command::new("java")
        .arg("-cp")
        .arg(&root)
        .arg("ReferenceCastHarness")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
#[test]
fn missing_or_primitive_cast_metadata_never_gets_reference_cleanup_proof() {
    let (ir, md, b, m) = fixture();
    for r in [
        Resolver {
            missing: true,
            primitive: false,
        },
        Resolver {
            missing: false,
            primitive: true,
        },
    ] {
        let cast = ir
            .instructions()
            .values()
            .find(|i| matches!(i, ManagedInsn::Cast { .. }))
            .unwrap();
        assert!(!candidate(cast, &r));
        assert!(matches!(
            crate::exceptions::dispatch::emit_method(&ir, &m, &b, &r, &[&md]),
            SourceResult::Unsupported(_)
        ));
    }
}
