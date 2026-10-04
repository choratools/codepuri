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
    bad_array: bool,
    missing_string: bool,
}
impl JavaResolver for Resolver {
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("java.lang.String".into())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        (!self.missing_string).then(|| "literal".encode_utf16().collect())
    }
    fn instruction_type(&self, _: TypeRef) -> Option<JavaType> {
        Some(if self.bad_array {
            JavaType::Int
        } else {
            JavaType::Array(Box::new(JavaType::Int))
        })
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(1)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(if r.index() == 1 {
            JavaMethod {
                owner: "java.lang.String".into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: vec![JavaType::Class("java.lang.String".into())],
                throws: vec![],
            }
        } else {
            JavaMethod {
                owner: "ArrayLengthCleanupHarness".into(),
                name: "cleanup".into(),
                is_static: true,
                return_type: JavaType::Void,
                parameters: vec![JavaType::Int],
                throws: vec![],
            }
        })
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
fn fixture() -> (MethodIr<SsaForm>, Metadata, DeclarationBindings) {
    fixture_invalid(false, false)
}
fn fixture_invalid(
    scalar: bool,
    wide_result: bool,
) -> (MethodIr<SsaForm>, Metadata, DeclarationBindings) {
    let mut b = Builder::new();
    let array = b.incoming(ManagedType::Array {
        element: Box::new(ManagedType::Int),
        nullable: true,
    });
    let index = b.incoming(ManagedType::Int);
    let value = b.incoming(ManagedType::Int);
    let mode = b.incoming(ManagedType::Int);
    let first = b.cfg.entry();
    let length = b.value(
        first,
        if wide_result {
            ManagedType::Long
        } else {
            ManagedType::Int
        },
    );
    b.set(
        first,
        ManagedInsn::ArrayLength {
            result: length,
            array: if scalar { index } else { array },
        },
    );
    let filled = b.block();
    let made = b.value(
        filled,
        ManagedType::Array {
            element: Box::new(ManagedType::Int),
            nullable: true,
        },
    );
    b.set(
        filled,
        ManagedInsn::NewFilledArray {
            result: Some(made),
            target: TypeRef::new(0),
            elements: vec![value],
        },
    );
    b.link(first, filled);
    let literal = b.block();
    let string = b.value(literal, ManagedType::Reference(ClassRef::new(0)));
    b.set(
        literal,
        ManagedInsn::Const {
            result: string,
            value: Constant::String(StringRef::new(0)),
        },
    );
    b.link(filled, literal);
    let allocation = b.block();
    let allocated = b.value(allocation, ManagedType::Reference(ClassRef::new(0)));
    b.set(
        allocation,
        ManagedInsn::NewInstance {
            result: allocated,
            class: ClassRef::new(0),
        },
    );
    b.link(literal, allocation);
    let init = b.block();
    b.set(
        init,
        ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(1),
            args: vec![allocated, string],
        },
    );
    b.link(allocation, init);
    let normal = b.block();
    b.set(
        normal,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(0),
            args: vec![mode],
        },
    );
    b.link(init, normal);
    let done = b.block();
    b.set(done, ManagedInsn::Return { value: None });
    b.link(normal, done);
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
    let cleanup = b.block();
    b.set(
        cleanup,
        ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(0),
            args: vec![mode],
        },
    );
    b.link(handler, cleanup);
    let rethrow = b.block();
    b.set(rethrow, ManagedInsn::Throw { value: caught });
    b.link(cleanup, rethrow);
    let mut offsets = HashMap::new();
    for (offset, block) in [first, filled, literal, allocation, init]
        .iter()
        .enumerate()
    {
        offsets.insert(*b.cfg.block(*block).unwrap().payload(), offset as u32);
    }
    offsets.insert(*b.cfg.block(handler).unwrap().payload(), 100);
    let original = crate::exceptions::OriginalScope {
        ordinal: 0,
        start: 0,
        end: 5,
        handler_offset: 1,
        catches: vec![("all".into(), 100)],
        offsets,
    };
    let protected = vec![first, filled, literal, allocation, init];
    for source in &protected {
        b.edge(*source, handler, EdgeKind::Exception("all".into()));
    }
    let mut hierarchy = Hierarchy::default();
    for (name, parent) in [
        ("java.lang.Object", None),
        ("java.lang.Throwable", Some("java.lang.Object")),
        ("java.lang.Exception", Some("java.lang.Throwable")),
        ("java.lang.RuntimeException", Some("java.lang.Exception")),
        ("java.lang.Error", Some("java.lang.Throwable")),
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
                protected,
                catches: vec![scopes::Catch {
                    key: "all".into(),
                    entry: handler,
                }],
                continuation: Some(normal),
            },
            hierarchy,
        },
        DeclarationBindings {
            parameters: vec![Some(array), Some(index), Some(value), Some(mode)],
            ..Default::default()
        },
    )
}
fn method() -> JavaMethod {
    JavaMethod {
        owner: "ArrayLengthCleanupHarness".into(),
        name: "generated".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![
            JavaType::Array(Box::new(JavaType::Int)),
            JavaType::Int,
            JavaType::Int,
            JavaType::Int,
        ],
        throws: vec![],
    }
}
#[test]
fn array_length_preserves_null_cleanup_identity_and_normal_effects() {
    let (ir, md, bindings) = fixture();
    let resolver = Resolver {
        bad_array: false,
        missing_string: false,
    };
    let result =
        crate::exceptions::dispatch::emit_method(&ir, &method(), &bindings, &resolver, &[&md]);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert!(source.contains("throw dispatch"));
    assert!(!source.contains("throws java.lang.Throwable"));
    let root = std::env::temp_dir().join(format!(
        "decomphify-array-length-cleanup-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let java = format!(
        r#"public class ArrayLengthCleanupHarness {{
 static int count; static final Error OVERRIDE=new AssertionError("cleanup");
 static void cleanup(int mode) {{count++;if(mode==1)throw OVERRIDE;}}
 {source}
 public static void main(String[] args) {{for(int index:new int[]{{0,2}})for(int mode=0;mode<2;mode++){{int[] array=index==0?new int[1]:null;count=0;Throwable caught=null;try{{generated(array,index,7,mode);}}catch(Throwable t){{caught=t;}}if(count!=1)throw new AssertionError("cleanup "+count);if(mode==1&&caught!=OVERRIDE)throw new AssertionError("identity");if(mode==0&&index==2&&!(caught instanceof NullPointerException))throw new AssertionError("body");if(mode==0&&index==0&&caught!=null)throw new AssertionError("normal");if(index==0&&array[0]!=0)throw new AssertionError("write");}}}}
}}"#
    );
    std::fs::write(root.join("ArrayLengthCleanupHarness.java"), java).unwrap();
    let compile = std::process::Command::new("javac")
        .arg(root.join("ArrayLengthCleanupHarness.java"))
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new("java")
        .arg("-cp")
        .arg(&root)
        .arg("ArrayLengthCleanupHarness")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
#[test]
fn unresolved_literal_and_nonarray_target_cannot_publish_cleanup() {
    let (ir, md, b) = fixture();
    for r in [
        Resolver {
            bad_array: true,
            missing_string: false,
        },
        Resolver {
            bad_array: false,
            missing_string: true,
        },
    ] {
        assert!(matches!(
            crate::exceptions::dispatch::emit_method(&ir, &method(), &b, &r, &[&md]),
            SourceResult::Unsupported(_)
        ));
    }
}

#[test]
fn scalar_operand_and_wide_result_never_receive_array_length_source_contract() {
    let r = Resolver {
        bad_array: false,
        missing_string: false,
    };
    for (scalar, wide) in [(true, false), (false, true)] {
        let (ir, md, b) = fixture_invalid(scalar, wide);
        assert!(matches!(
            crate::exceptions::dispatch::emit_method(&ir, &method(), &b, &r, &[&md]),
            SourceResult::Unsupported(_)
        ));
    }
}
