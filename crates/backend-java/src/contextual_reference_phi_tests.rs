//! Exact reference-producing contracts refine complete materialized Phi families.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{JavaField, JavaMethod, ParameterBindings, SourceResult};
use decomp_core::{
    cfg::{Cfg, EdgeKind},
    ids::BlockId,
};
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    ops::InvokeKind,
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
    types::ManagedType,
    value::Value,
};

fn base() -> JavaType {
    JavaType::Class("Fixture.Base".into())
}
struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            match r.index() {
                0 => "Fixture.Base",
                1 => "Fixture.Child",
                2 => "java.lang.Object",
                _ => return None,
            }
            .into(),
        )
    }
    fn instruction_type(&self, r: TypeRef) -> Option<JavaType> {
        match r.index() {
            0..=2 => self.class(ClassRef::new(r.index())).map(JavaType::Class),
            3 => Some(JavaType::Array(Box::new(base()))),
            4 => Some(JavaType::Int),
            _ => None,
        }
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let (owner, name, is_static, parameters, return_type) = match r.index() {
            0 => ("Fixture.Child", "parent", false, vec![], base()),
            1 => (
                "Fixture",
                "observe",
                true,
                vec![JavaType::Boolean],
                JavaType::Void,
            ),
            2 => (
                "Fixture",
                "childOnly",
                true,
                vec![JavaType::Class("Fixture.Child".into())],
                JavaType::Void,
            ),
            _ => return None,
        };
        Some(JavaMethod {
            owner: owner.into(),
            name: name.into(),
            is_static,
            parameters,
            return_type,
            throws: vec![],
        })
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        Some(if r.index() == 0 { 1 } else { 9 })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture.Child".into(),
            name: "parentField".into(),
            ty: base(),
            is_static: false,
        })
    }
    fn field_access(&self, _: FieldRef) -> Option<u32> {
        Some(1)
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || actual.reference() && *expected == JavaType::Class("java.lang.Object".into())
            || *actual == JavaType::Class("Fixture.Child".into()) && *expected == base()
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
        let entry = insns.insert(ManagedInsn::Return { value: None });
        Self {
            cfg: Cfg::new(entry),
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
    fn link(&mut self, a: BlockId, b: BlockId) -> decomp_core::ids::EdgeId {
        self.cfg.add_edge(a, b, EdgeKind::Unconditional).unwrap()
    }
}
// kind0/1/2: exact invocation, field or original cast, annotated Object.
// negatives: off-CFG narrow use/copy, opaque, unknown type, static mismatch.
fn fixture(
    kind: u8,
    negative: u8,
    target: u32,
) -> (
    MethodIr<SsaForm>,
    ParameterBindings,
    ValueId,
    ValueId,
    ValueId,
) {
    let mut b = Builder::new();
    let choose = b.incoming(ManagedType::Boolean);
    let input = b.incoming(ManagedType::Reference(ClassRef::new(0)));
    let child = b.incoming(ManagedType::Reference(ClassRef::new(1)));
    let object = b.incoming(ManagedType::Reference(ClassRef::new(2)));
    let entry = b.cfg.entry();
    let yes = b.block();
    let no = b.block();
    let ye = b.cfg.add_edge(entry, yes, EdgeKind::BranchTrue).unwrap();
    let ne = b.cfg.add_edge(entry, no, EdgeKind::BranchFalse).unwrap();
    b.set(
        entry,
        ManagedInsn::Branch {
            condition: Some(choose),
            then_edge: ye,
            else_edge: Some(ne),
        },
    );
    let produced = b.value(
        yes,
        if negative == 3 {
            ManagedType::Unknown
        } else {
            ManagedType::Reference(ClassRef::new(2))
        },
    );
    b.set(
        yes,
        if negative == 2 {
            ManagedInsn::Opaque {
                family_tag: 7,
                reads: vec![],
                writes: vec![produced],
                effects: EffectFlags::NONE,
            }
        } else {
            match kind {
                0 => ManagedInsn::Invoke {
                    result: Some(produced),
                    kind: if negative == 4 {
                        InvokeKind::Static
                    } else {
                        InvokeKind::Virtual
                    },
                    method: MethodRef::new(0),
                    args: vec![child],
                },
                1 => ManagedInsn::FieldGet {
                    result: produced,
                    field: FieldRef::new(0),
                    object: Some(child),
                },
                _ => ManagedInsn::Cast {
                    result: produced,
                    value: object,
                    target: TypeRef::new(0),
                },
            }
        },
    );
    let join = b.block();
    let yj = b.link(yes, join);
    let nj = b.link(no, join);
    b.set(
        no,
        ManagedInsn::Branch {
            condition: None,
            then_edge: nj,
            else_edge: None,
        },
    );
    let phi = b.value(join, ManagedType::Unknown);
    b.set(
        join,
        ManagedInsn::Phi {
            result: phi,
            inputs: vec![(yj, produced), (nj, input)],
        },
    );
    let cp = b.block();
    b.link(join, cp);
    let copy = b.value(cp, ManagedType::Reference(ClassRef::new(2)));
    b.set(
        cp,
        ManagedInsn::Move {
            result: copy,
            value: phi,
        },
    );
    if negative == 1 {
        // Every arena use counts, including a copy outside the CFG.
        let id = b.insns.insert(ManagedInsn::Return { value: None });
        let extra = b
            .values
            .insert(Value::new(id, ManagedType::Reference(ClassRef::new(2))));
        b.insns[id] = ManagedInsn::Move {
            result: extra,
            value: produced,
        };
        b.insns.insert(ManagedInsn::Invoke {
            result: None,
            kind: InvokeKind::Static,
            method: MethodRef::new(2),
            args: vec![extra],
        });
    }
    let check = b.block();
    b.link(cp, check);
    let bit = b.value(check, ManagedType::Boolean);
    b.set(
        check,
        ManagedInsn::InstanceOf {
            result: bit,
            value: copy,
            target: TypeRef::new(target),
        },
    );
    let observe = b.block();
    b.link(check, observe);
    b.set(
        observe,
        ManagedInsn::Invoke {
            result: None,
            kind: InvokeKind::Static,
            method: MethodRef::new(1),
            args: vec![bit],
        },
    );
    let ret = b.block();
    b.link(observe, ret);
    b.set(ret, ManagedInsn::Return { value: Some(copy) });
    (
        MethodIr::<SsaForm>::from_parts(b.cfg, b.insns, b.values),
        ParameterBindings {
            parameters: vec![choose, input, child, object],
            ..Default::default()
        },
        phi,
        copy,
        produced,
    )
}
fn method(kind: u8) -> JavaMethod {
    JavaMethod {
        owner: "Fixture".into(),
        name: format!("test{kind}"),
        is_static: true,
        parameters: vec![
            JavaType::Boolean,
            base(),
            JavaType::Class("Fixture.Child".into()),
            JavaType::Class("java.lang.Object".into()),
        ],
        return_type: base(),
        throws: vec![],
    }
}

#[test]
fn exact_reference_producers_refine_every_phi_copy_and_producer_slot() {
    for kind in 0..=2 {
        for target in [1, 3] {
            let (ir, _, phi, copy, produced) = fixture(kind, 0, target);
            let plan =
                checked_phi_plan(&ir, phi, &base(), &base(), &Hierarchy::default(), &Resolver)
                    .unwrap();
            for v in [phi, copy, produced] {
                assert!(plan.locals.contains(&v));
            }
            assert!(plan.inputs.iter().all(|(_, c)| *c == Coercion::Identity));
            let declarations = declaration_plan(&ir, &base(), &Resolver).unwrap();
            for v in [phi, copy, produced] {
                assert_eq!(declarations.get(&v), Some(&base()));
            }
        }
    }
}
#[test]
fn reference_refinement_keeps_all_arena_unknown_and_primitive_target_denials() {
    for kind in 0..=2 {
        for (negative, target) in [(1, 1), (2, 1), (3, 1), (0, 4), (0, 99)] {
            let (ir, _, phi, _, _) = fixture(kind, negative, target);
            assert!(
                checked_phi_plan(&ir, phi, &base(), &base(), &Hierarchy::default(), &Resolver)
                    .is_err(),
                "kind={kind} negative={negative} target={target}"
            );
            assert!(
                !declaration_plan(&ir, &base(), &Resolver)
                    .unwrap()
                    .contains_key(&phi)
            );
        }
    }
    let (ir, _, phi, _, _) = fixture(0, 4, 1);
    assert!(
        checked_phi_plan(&ir, phi, &base(), &base(), &Hierarchy::default(), &Resolver).is_err()
    );
}
#[test]
fn unchanged_reference_family_sources_compile_and_preserve_failures_and_identity() {
    let mut source = String::new();
    for kind in 0..=2 {
        let (ir, bindings, _, _, _) = fixture(kind, 0, 1);
        let emitted = crate::emit_method(&ir, &method(kind), &bindings, &Resolver);
        let SourceResult::Complete { source: s, .. } = emitted else {
            panic!("{emitted:?}")
        };
        source.push_str(&s);
    }
    let dir = std::env::temp_dir().join(format!("contextual-reference-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class Fixture {{static String trace="";static RuntimeException failure=new RuntimeException("same");
public static class Base {{}} public static class Child extends Base {{public Base parentField;public Base parent(){{trace+="P";if(fail)throw failure;return parentField;}}}} static boolean fail;
public static void observe(boolean v){{trace+=v?"1":"0";}}public static void childOnly(Child c){{}}
{source}
static Base call(int kind,boolean choose,Base input,Child child,Object obj){{return switch(kind){{case 0->test0(choose,input,child,obj);case 1->test1(choose,input,child,obj);default->test2(choose,input,child,obj);}};}}
public static void main(String[]args){{Base parent=new Base();Child child=new Child();child.parentField=parent;
for(int k=0;k<3;k++){{trace="";if(call(k,true,null,child,parent)!=parent||!trace.equals(k==0?"P0":"0"))throw new AssertionError(trace);trace="";if(call(k,false,child,null,new Object())!=child||!trace.equals("1"))throw new AssertionError(trace);trace="";if(call(k,true,null,child,null)!=(k==2?null:parent))throw new AssertionError();}}
child.parentField=null;trace="";if(test0(true,null,child,null)!=null||!trace.equals("P0"))throw new AssertionError(trace);
trace="";try{{test0(true,null,null,null);throw new AssertionError();}}catch(NullPointerException expected){{}}if(!trace.isEmpty())throw new AssertionError(trace);
trace="";try{{test1(true,null,null,null);throw new AssertionError();}}catch(NullPointerException expected){{}}if(!trace.isEmpty())throw new AssertionError(trace);
trace="";try{{test2(true,null,null,new Object());throw new AssertionError();}}catch(ClassCastException expected){{}}if(!trace.isEmpty())throw new AssertionError(trace);
fail=true;trace="";try{{test0(true,null,child,null);throw new AssertionError();}}catch(RuntimeException actual){{if(actual!=failure)throw new AssertionError();}}if(!trace.equals("P"))throw new AssertionError(trace);
}}}}"#
    );
    std::fs::write(dir.join("Fixture.java"), java).unwrap();
    let home = std::env::var_os("JAVA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (tool, args) in [
        ("javac", vec!["--release", "17", "Fixture.java"]),
        ("java", vec!["-Xverify:all", "-cp", ".", "Fixture"]),
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
