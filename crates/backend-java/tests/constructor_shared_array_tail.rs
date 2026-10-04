//! Shared acyclic constructor tails execute once along each selected path.
#![allow(clippy::unwrap_used)]
use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings, constructors::*,
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{CompareOp, Constant, InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};
fn array() -> JavaType {
    JavaType::Array(Box::new(JavaType::Long))
}
fn managed_array() -> ManagedType {
    ManagedType::Array {
        element: Box::new(ManagedType::Long),
        nullable: true,
    }
}
struct R;
impl JavaResolver for R {
    fn field_access(&self, _: FieldRef) -> Option<u32> {
        // The unchanged javac fixture declares private final long[] m_ints.
        Some(0x0012)
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("LongArray".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "LongArray".into(),
            name: "m_ints".into(),
            ty: array(),
            is_static: false,
        })
    }
    fn instruction_type(&self, _: TypeRef) -> Option<JavaType> {
        Some(array())
    }
    fn method_access(&self, m: MethodRef) -> Option<u32> {
        Some(if m.index() == 2 { 9 } else { 1 })
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        if m.index() == 2 {
            Some(JavaMethod {
                owner: "Copy".into(),
                name: "run".into(),
                parameters: vec![
                    JavaType::Class("java.lang.Object".into()),
                    JavaType::Int,
                    JavaType::Class("java.lang.Object".into()),
                    JavaType::Int,
                    JavaType::Int,
                ],
                return_type: JavaType::Void,
                is_static: true,
                throws: vec![],
            })
        } else {
            self.initializer(m).map(|i| i.method)
        }
    }
}
impl ConstructorResolver for R {
    fn initializer(&self, m: MethodRef) -> Option<Initializer> {
        Some(Initializer {
            method: JavaMethod {
                owner: if m.index() == 0 {
                    "LongArray"
                } else {
                    "java.lang.Object"
                }
                .into(),
                name: "<init>".into(),
                parameters: if m.index() == 0 {
                    vec![array(), JavaType::Int, JavaType::Int]
                } else {
                    vec![]
                },
                return_type: JavaType::Void,
                is_static: false,
                throws: vec![],
            },
            delegates_to: None,
        })
    }
}
fn define(
    insns: &mut InsnArena,
    values: &mut ValueArena,
    ty: ManagedType,
    f: impl FnOnce(ValueId) -> ManagedInsn,
) -> (InsnId, ValueId) {
    let mut value = None;
    let id = insns.insert_with_key(|id| {
        let v = values.insert(Value::new(id, ty));
        value = Some(v);
        f(v)
    });
    (id, value.unwrap())
}
fn param(insns: &mut InsnArena, values: &mut ValueArena, ty: ManagedType) -> ValueId {
    define(insns, values, ty, |v| ManagedInsn::Opaque {
        family_tag: 0,
        reads: vec![],
        writes: vec![v],
        effects: EffectFlags::NONE,
    })
    .1
}
fn body(cycle: bool) -> (MethodIr<SsaForm>, ParameterBindings) {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let this = param(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(0)),
    );
    let input = param(&mut insns, &mut values, managed_array());
    let offset = param(&mut insns, &mut values, ManagedType::Int);
    let length = param(&mut insns, &mut values, ManagedType::Int);
    let init = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![this],
    });
    let (zid, zero) = define(&mut insns, &mut values, ManagedType::Int, |v| {
        ManagedInsn::Const {
            result: v,
            value: Constant::Zero,
        }
    });
    let (c1id, c1) = define(&mut insns, &mut values, ManagedType::Boolean, |v| {
        ManagedInsn::Compare {
            op: CompareOp::NotEqual,
            result: v,
            lhs: offset,
            rhs: zero,
        }
    });
    let b1id = insns.insert(ManagedInsn::Return { value: None });
    let (alenid, alen) = define(&mut insns, &mut values, ManagedType::Int, |v| {
        ManagedInsn::ArrayLength {
            result: v,
            array: input,
        }
    });
    let (c2id, c2) = define(&mut insns, &mut values, ManagedType::Boolean, |v| {
        ManagedInsn::Compare {
            op: CompareOp::NotEqual,
            result: v,
            lhs: length,
            rhs: alen,
        }
    });
    let b2id = insns.insert(ManagedInsn::Return { value: None });
    let (newid, new) = define(&mut insns, &mut values, managed_array(), |v| {
        ManagedInsn::NewArray {
            result: v,
            element: TypeRef::new(0),
            length,
        }
    });
    let copy_store = insns.insert(ManagedInsn::FieldPut {
        object: Some(this),
        field: FieldRef::new(0),
        value: new,
    });
    let copy = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(2),
        args: vec![input, offset, new, zero, length],
    });
    let copyret = insns.insert(ManagedInsn::Return { value: None });
    let alias_store = insns.insert(ManagedInsn::FieldPut {
        object: Some(this),
        field: FieldRef::new(0),
        value: input,
    });
    let aliasret = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(init);
    let mut prev = cfg.entry();
    for id in [zid, c1id, b1id] {
        let b = cfg.add_block(id);
        cfg.add_edge(prev, b, EdgeKind::Unconditional).unwrap();
        prev = b;
    }
    let b1 = prev;
    let copy_start = cfg.add_block(newid);
    let alenblock = cfg.add_block(alenid);
    let yes = cfg.add_edge(b1, copy_start, EdgeKind::BranchTrue).unwrap();
    let no = cfg.add_edge(b1, alenblock, EdgeKind::BranchFalse).unwrap();
    insns[b1id] = ManagedInsn::Branch {
        condition: Some(c1),
        then_edge: yes,
        else_edge: Some(no),
    };
    prev = alenblock;
    for id in [c2id, b2id] {
        let b = cfg.add_block(id);
        cfg.add_edge(prev, b, EdgeKind::Unconditional).unwrap();
        prev = b;
    }
    let b2 = prev;
    let alias = cfg.add_block(alias_store);
    let yes = cfg.add_edge(b2, copy_start, EdgeKind::BranchTrue).unwrap();
    let no = cfg.add_edge(b2, alias, EdgeKind::BranchFalse).unwrap();
    insns[b2id] = ManagedInsn::Branch {
        condition: Some(c2),
        then_edge: yes,
        else_edge: Some(no),
    };
    let ret = cfg.add_block(aliasret);
    cfg.add_edge(alias, ret, EdgeKind::Unconditional).unwrap();
    prev = copy_start;
    for id in [copy_store, copy, copyret] {
        let b = cfg.add_block(id);
        cfg.add_edge(prev, b, EdgeKind::Unconditional).unwrap();
        prev = b;
    }
    if cycle {
        cfg.add_edge(prev, b1, EdgeKind::Unconditional).unwrap();
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        ParameterBindings {
            receiver: Some(this),
            parameters: vec![input, offset, length],
            names: vec![],
        },
    )
}
#[test]
fn shared_allocation_tail_preserves_per_execution_effect_count_alias_and_copy() {
    let (ir, b) = body(false);
    let plan = plan_constructor(&ir, MethodRef::new(0), "java.lang.Object", &b, &R).unwrap();
    let source = format!(
        "class Copy {{static int calls;static void run(Object a,int i,Object b,int j,int n){{calls++;System.arraycopy(a,i,b,j,n);}}}}public class LongArray {{private final long[] m_ints;{}public static void main(String[]a){{long[] x={{1,2,3}};LongArray same=new LongArray(x,0,3);if(same.m_ints!=x||Copy.calls!=0)throw new AssertionError();LongArray first=new LongArray(x,1,2);if(first.m_ints==x||first.m_ints[0]!=2||Copy.calls!=1)throw new AssertionError();LongArray second=new LongArray(x,0,2);if(second.m_ints==x||second.m_ints[1]!=2||Copy.calls!=2)throw new AssertionError();x[0]=99;if(second.m_ints[0]!=1)throw new AssertionError();try{{new LongArray(x,1,-1);throw new AssertionError();}}catch(NegativeArraySizeException expected){{}}if(Copy.calls!=2)throw new AssertionError();try{{new LongArray(x,5,1);throw new AssertionError();}}catch(IndexOutOfBoundsException expected){{}}if(Copy.calls!=3)throw new AssertionError();}}}}",
        plan.source
    );
    let dir = std::env::temp_dir().join(format!("constructor-shared-tail-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("LongArray.java"), source).unwrap();
    for (tool, args) in [
        ("javac", vec!["LongArray.java"]),
        ("java", vec!["-cp", ".", "LongArray"]),
    ] {
        let o =
            std::process::Command::new(format!("/home/jtjisgod/tools/jdk-17.0.13+11/bin/{tool}"))
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
}
#[test]
fn suffix_backedges_still_reject() {
    let (ir, b) = body(true);
    assert!(plan_constructor(&ir, MethodRef::new(0), "java.lang.Object", &b, &R).is_err());
}

#[test]
fn parallel_shared_tail_has_bounded_output_and_one_store() {
    let mut insns = InsnArena::default();
    let mut values = ValueArena::default();
    let this = param(
        &mut insns,
        &mut values,
        ManagedType::Reference(ClassRef::new(0)),
    );
    let input = param(&mut insns, &mut values, managed_array());
    let offset = param(&mut insns, &mut values, ManagedType::Int);
    let length = param(&mut insns, &mut values, ManagedType::Int);
    let init = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(1),
        args: vec![this],
    });
    let (cmp, condition) = define(&mut insns, &mut values, ManagedType::Boolean, |v| {
        ManagedInsn::Compare {
            op: CompareOp::Equal,
            result: v,
            lhs: offset,
            rhs: length,
        }
    });
    let mut cfg = Cfg::new(init);
    let comparison = cfg.add_block(cmp);
    cfg.add_edge(cfg.entry(), comparison, EdgeKind::Unconditional)
        .unwrap();
    let mut previous = comparison;
    let mut branches = vec![];
    for _ in 0..16 {
        let id = insns.insert(ManagedInsn::Return { value: None });
        let block = cfg.add_block(id);
        branches.push((id, block));
        if branches.len() == 1 {
            cfg.add_edge(previous, block, EdgeKind::Unconditional)
                .unwrap();
        }
        previous = block;
    }
    let store = insns.insert(ManagedInsn::FieldPut {
        object: Some(this),
        field: FieldRef::new(0),
        value: input,
    });
    let end = cfg.add_block(store);
    let ret = cfg.add_block(insns.insert(ManagedInsn::Return { value: None }));
    cfg.add_edge(end, ret, EdgeKind::Unconditional).unwrap();
    for n in 0..branches.len() {
        let (id, block) = branches[n];
        let next = branches.get(n + 1).map_or(end, |(_, b)| *b);
        let yes = cfg.add_edge(block, next, EdgeKind::BranchTrue).unwrap();
        let no = cfg.add_edge(block, next, EdgeKind::BranchFalse).unwrap();
        insns[id] = ManagedInsn::Branch {
            condition: Some(condition),
            then_edge: yes,
            else_edge: Some(no),
        };
    }
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    let b = ParameterBindings {
        receiver: Some(this),
        parameters: vec![input, offset, length],
        names: vec![],
    };
    let plan = plan_constructor(&ir, MethodRef::new(0), "java.lang.Object", &b, &R).unwrap();
    // Sixteen two-way edges share the same tail. Source size must follow the
    // real graph, rather than expand into 2^16 copies of the assignment.
    assert!(plan.source.len() < 50_000);
    assert_eq!(plan.source.matches("m_ints =").count(), 1);
}
