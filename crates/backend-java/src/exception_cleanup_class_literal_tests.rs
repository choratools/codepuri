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
    target: Option<JavaType>,
}
impl JavaResolver for Resolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            if r.index() == 0 {
                "java.lang.Class"
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
        self.target.clone()
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(9)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: "ClassLiteralHarness".into(),
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
    let literal = b.cfg.entry();
    let class = b.value(literal, ManagedType::Reference(ClassRef::new(0)));
    b.set(
        literal,
        ManagedInsn::Const {
            result: class,
            value: Constant::ClassLiteral(TypeRef::new(0)),
        },
    );
    let body = b.block();
    b.link(literal, body);
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
        b.edge(source, handler, EdgeKind::Exception("all".into()));
    }
    let original = crate::exceptions::OriginalScope {
        ordinal: 0,
        start: 0,
        end: 2,
        handler_offset: 1,
        catches: vec![("all".into(), 100)],
        offsets: HashMap::from([
            (*b.cfg.block(literal).unwrap().payload(), 0),
            (*b.cfg.block(body).unwrap().payload(), 1),
            (*b.cfg.block(normal).unwrap().payload(), 2),
            (*b.cfg.block(outside).unwrap().payload(), 3),
            (*b.cfg.block(handler).unwrap().payload(), 100),
        ]),
    };
    let mut h = Hierarchy::default();
    for (n, p) in [
        ("java.lang.Object", None),
        ("java.lang.Throwable", Some("java.lang.Object")),
        ("java.lang.Exception", Some("java.lang.Throwable")),
        ("java.lang.RuntimeException", Some("java.lang.Exception")),
        ("java.lang.Error", Some("java.lang.Throwable")),
        ("java.lang.Class", Some("java.lang.Object")),
    ] {
        h.insert(Declaration {
            name: n.into(),
            superclass: p.map(Into::into),
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
                protected: vec![literal, body],
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: handler,
                }],
                continuation: Some(normal),
            },
            h,
        },
        DeclarationBindings {
            receiver: None,
            parameters: vec![Some(mode)],
            names: vec![],
        },
        JavaMethod {
            owner: "ClassLiteralHarness".into(),
            name: "generated".into(),
            is_static: true,
            parameters: vec![JavaType::Int],
            return_type: JavaType::Void,
            throws: vec![],
        },
    )
}
#[test]
fn class_literal_preserves_exact_cleanup_range_and_linkage_error_identity() {
    let (ir, md, b, m) = fixture();
    for missing in [false, true] {
        let r = Resolver {
            target: Some(JavaType::Class(
                if missing {
                    "MissingLiteralTarget"
                } else {
                    "ClassLiteralHarness.Bomb"
                }
                .into(),
            )),
        };
        let out = crate::exceptions::dispatch::emit_method(&ir, &m, &b, &r, &[&md]);
        let SourceResult::Complete { source, .. } = out else {
            panic!("{out:?}")
        };
        assert!(!source.contains("throws java.lang.Throwable"));
        let dir =
            std::env::temp_dir().join(format!("literal-cleanup-{}-{missing}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let java = format!(
            r#"public class ClassLiteralHarness {{
 static int count,initialized;static String trace;static Throwable captured;
 static final Error PRIMARY=new AssertionError("body"),OVERRIDE=new AssertionError("cleanup"),OUTSIDE=new AssertionError("outside");
 static class Bomb {{static {{initialized++;if(initialized>0)throw new AssertionError("must not initialize");}}}}
 static void body(int mode){{trace+="B";if(mode==1||mode==2)throw PRIMARY;}}
 static void observe(Throwable value){{captured=value;trace+="R";}}
 static void cleanup(int mode){{trace+="C";count++;if(mode==2||mode==4)throw OVERRIDE;}}
 static void outside(int mode){{trace+="O";if(mode==3)throw OUTSIDE;}}
 {source}
 public static void main(String[]args){{for(int mode=0;mode<5;mode++){{count=0;trace="";captured=null;Throwable failure=null;try{{generated(mode);}}catch(Throwable got){{failure=got;}}if(count!=1||initialized!=0)throw new AssertionError("count/init");
 if({missing}){{if(!(captured instanceof NoClassDefFoundError))throw new AssertionError("linkage");if(mode==2||mode==4){{if(failure!=OVERRIDE)throw new AssertionError("override");}}else if(failure!=captured)throw new AssertionError("linkage identity");if(!trace.equals("RC"))throw new AssertionError(trace);}}
 else{{if(mode==0&&failure!=null)throw new AssertionError("normal");if(mode==1&&failure!=PRIMARY)throw new AssertionError("primary");if((mode==2||mode==4)&&failure!=OVERRIDE)throw new AssertionError("override");if(mode==3&&failure!=OUTSIDE)throw new AssertionError("outside");String expected=mode==1||mode==2?"BRC":mode==4?"BC":"BCO";if(!trace.equals(expected))throw new AssertionError(trace);}}
 }}}}
 }}"#
        );
        std::fs::write(dir.join("ClassLiteralHarness.java"), java).unwrap();
        std::fs::write(
            dir.join("MissingLiteralTarget.java"),
            "public class MissingLiteralTarget {}",
        )
        .unwrap();
        let compile = std::process::Command::new("javac")
            .current_dir(&dir)
            .args(["ClassLiteralHarness.java", "MissingLiteralTarget.java"])
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        if missing {
            std::fs::remove_file(dir.join("MissingLiteralTarget.class")).unwrap();
        }
        let run = std::process::Command::new("java")
            .current_dir(&dir)
            .args(["-cp", ".", "ClassLiteralHarness"])
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
}
#[test]
fn class_literal_missing_and_invalid_metadata_remain_closed() {
    let (ir, md, b, m) = fixture();
    for target in [None, Some(JavaType::Array(Box::new(JavaType::Void)))] {
        let r = Resolver { target };
        assert!(matches!(
            crate::exceptions::dispatch::emit_method(&ir, &m, &b, &r, &[&md]),
            SourceResult::Unsupported(_)
        ));
        let first = ir
            .instruction(*ir.cfg().block(ir.cfg().entry()).unwrap().payload())
            .unwrap();
        assert!(!candidate(first, &r));
    }
}
