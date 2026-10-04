//! A preserved copy must agree with its actual unadopted materialized slots.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{JavaField, JavaMethod, ParameterBindings, SourceResult};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
    types::ManagedType,
    value::Value,
};
struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, key: ClassRef) -> Option<String> {
        Some(
            match key.index() {
                0 => "java.lang.Object",
                1 => "java.lang.reflect.Method",
                2 => "java.lang.Long",
                3 => "java.lang.String",
                _ => return None,
            }
            .into(),
        )
    }
    fn instruction_type(&self, key: TypeRef) -> Option<JavaType> {
        if key.index() == 4 {
            return Some(JavaType::Array(Box::new(object())));
        }
        self.class(ClassRef::new(key.index())).map(JavaType::Class)
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
    }
    fn string(&self, key: StringRef) -> Option<Vec<u16>> {
        (key.index() == 0).then(Vec::new)
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected || actual.reference() && *expected == object()
    }
}
fn object() -> JavaType {
    JavaType::Class("java.lang.Object".into())
}
fn method_type() -> JavaType {
    JavaType::Class("java.lang.reflect.Method".into())
}
fn fixture(
    target: u32,
    boundary: bool,
) -> (
    MethodIr<SsaForm>,
    ParameterBindings,
    ValueId,
    ValueId,
    ValueId,
) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let initial = insns.insert(ManagedInsn::Return { value: None });
    let obj = values.insert(Value::new(
        initial,
        ManagedType::Reference(ClassRef::new(0)),
    ));
    insns[initial] = ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![obj],
        effects: EffectFlags::NONE,
    };
    let cond_id = insns.insert(ManagedInsn::Return { value: None });
    let cond = values.insert(Value::new(cond_id, ManagedType::Boolean));
    insns[cond_id] = ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![cond],
        effects: EffectFlags::NONE,
    };
    let entry_id = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(entry_id);
    let cast_id = insns.insert(ManagedInsn::Return { value: None });
    let cast = values.insert(Value::new(
        cast_id,
        ManagedType::Reference(ClassRef::new(if boundary { 1 } else { 0 })),
    ));
    insns[cast_id] = ManagedInsn::Cast {
        result: cast,
        value: obj,
        target: TypeRef::new(target),
    };
    let yes = cfg.add_block(cast_id);
    let copy_id = insns.insert(ManagedInsn::Return { value: None });
    let copy = values.insert(Value::new(
        copy_id,
        ManagedType::Reference(ClassRef::new(1)),
    ));
    insns[copy_id] = ManagedInsn::Move {
        result: copy,
        value: cast,
    };
    let cb = cfg.add_block(copy_id);
    cfg.add_edge(yes, cb, EdgeKind::Unconditional).unwrap();
    let (cb, copy) = if boundary {
        let id = insns.insert(ManagedInsn::Return { value: None });
        let widened = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
        insns[id] = ManagedInsn::Move {
            result: widened,
            value: copy,
        };
        let next = cfg.add_block(id);
        cfg.add_edge(cb, next, EdgeKind::Unconditional).unwrap();
        (next, widened)
    } else {
        (cb, copy)
    };
    let null_id = insns.insert(ManagedInsn::Return { value: None });
    let null = values.insert(Value::new(null_id, ManagedType::Null));
    insns[null_id] = ManagedInsn::Const {
        result: null,
        value: Constant::Null,
    };
    let no = cfg.add_block(null_id);
    let ye = cfg
        .add_edge(cfg.entry(), yes, EdgeKind::BranchTrue)
        .unwrap();
    let ne = cfg
        .add_edge(cfg.entry(), no, EdgeKind::BranchFalse)
        .unwrap();
    insns[entry_id] = ManagedInsn::Branch {
        condition: Some(cond),
        then_edge: ye,
        else_edge: Some(ne),
    };
    let phi_id = insns.insert(ManagedInsn::Return { value: None });
    let phi = values.insert(Value::new(phi_id, ManagedType::Reference(ClassRef::new(1))));
    let join = cfg.add_block(phi_id);
    let yj = cfg.add_edge(cb, join, EdgeKind::Unconditional).unwrap();
    let nj = cfg.add_edge(no, join, EdgeKind::Unconditional).unwrap();
    insns[phi_id] = ManagedInsn::Phi {
        result: phi,
        inputs: vec![(yj, copy), (nj, null)],
    };
    let ret = insns.insert(ManagedInsn::Return { value: Some(phi) });
    let rb = cfg.add_block(ret);
    cfg.add_edge(join, rb, EdgeKind::Unconditional).unwrap();
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        ParameterBindings {
            parameters: vec![obj, cond],
            ..Default::default()
        },
        cast,
        copy,
        phi,
    )
}
#[test]
fn mismatched_preserved_copy_reenters_atomic_producer_refinement() {
    let (ir, bindings, cast, copy, phi) = fixture(1, false);
    assert!(preserved_reference_copy(&ir, copy, &object(), &Resolver).is_none());
    let declarations = declaration_plan(&ir, &method_type(), &Resolver).unwrap();
    for value in [cast, copy, phi] {
        assert_eq!(declarations.get(&value), Some(&method_type()));
    }
    let method = JavaMethod {
        owner: "PreservedCopy".into(),
        name: "select".into(),
        is_static: true,
        parameters: vec![object(), JavaType::Boolean],
        return_type: method_type(),
        throws: vec![],
    };
    let emitted = crate::emit_method(&ir, &method, &bindings, &Resolver);
    let SourceResult::Complete { source, .. } = emitted else {
        panic!("{emitted:?}")
    };
    assert!(source.contains("(java.lang.reflect.Method) (java.lang.Object)"));
    let dir = std::env::temp_dir().join(format!("preserved-cast-emitter-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class PreservedCopy {{
{source}
public static void main(String[] args) throws Exception {{
java.lang.reflect.Method m=String.class.getMethod("length");
if(select(m,true)!=m||select(null,true)!=null||select(new Object(),false)!=null)throw new AssertionError();
try{{select(new Object(),true);throw new AssertionError();}}catch(ClassCastException expected){{}}
}}
}}"#
    );
    std::fs::write(dir.join("PreservedCopy.java"), java).unwrap();
    let jdk = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
    let output = std::process::Command::new(format!("{jdk}/javac"))
        .arg("--release")
        .arg("17")
        .arg(dir.join("PreservedCopy.java"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = std::process::Command::new(format!("{jdk}/java"))
        .arg("-cp")
        .arg(&dir)
        .arg("PreservedCopy")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn missing_or_incompatible_original_cast_contract_cannot_refine_copy() {
    for target in [0, 99] {
        let (ir, _, _, _, phi) = fixture(target, false);
        assert!(
            checked_phi_plan(
                &ir,
                phi,
                &method_type(),
                &method_type(),
                &Hierarchy::default(),
                &Resolver
            )
            .is_err()
        );
    }
}

#[test]
fn actual_emitter_preserves_array_put_producer_before_object_join() {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let array_type = ManagedType::Array {
        element: Box::new(ManagedType::Double),
        nullable: true,
    };
    let mut params = Vec::new();
    for ty in [
        array_type.clone(),
        ManagedType::Boolean,
        ManagedType::Int,
        ManagedType::Double,
    ] {
        let id = insns.insert(ManagedInsn::Return { value: None });
        let v = values.insert(Value::new(id, ty));
        insns[id] = ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        };
        params.push(v);
    }
    let entry_id = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(entry_id);
    let copy_id = insns.insert(ManagedInsn::Return { value: None });
    let copy = values.insert(Value::new(copy_id, array_type));
    insns[copy_id] = ManagedInsn::Move {
        result: copy,
        value: params[0],
    };
    let yes = cfg.add_block(copy_id);
    let put_id = insns.insert(ManagedInsn::ArrayPut {
        array: copy,
        index: params[2],
        value: params[3],
    });
    let put = cfg.add_block(put_id);
    cfg.add_edge(yes, put, EdgeKind::Unconditional).unwrap();
    let null_id = insns.insert(ManagedInsn::Return { value: None });
    let null = values.insert(Value::new(null_id, ManagedType::Null));
    insns[null_id] = ManagedInsn::Const {
        result: null,
        value: Constant::Null,
    };
    let no = cfg.add_block(null_id);
    let ye = cfg
        .add_edge(cfg.entry(), yes, EdgeKind::BranchTrue)
        .unwrap();
    let ne = cfg
        .add_edge(cfg.entry(), no, EdgeKind::BranchFalse)
        .unwrap();
    insns[entry_id] = ManagedInsn::Branch {
        condition: Some(params[1]),
        then_edge: ye,
        else_edge: Some(ne),
    };
    let phi_id = insns.insert(ManagedInsn::Return { value: None });
    let phi = values.insert(Value::new(phi_id, ManagedType::Reference(ClassRef::new(0))));
    let join = cfg.add_block(phi_id);
    let yj = cfg.add_edge(put, join, EdgeKind::Unconditional).unwrap();
    let nj = cfg.add_edge(no, join, EdgeKind::Unconditional).unwrap();
    insns[phi_id] = ManagedInsn::Phi {
        result: phi,
        inputs: vec![(yj, copy), (nj, null)],
    };
    let ret = insns.insert(ManagedInsn::Return { value: Some(phi) });
    let rb = cfg.add_block(ret);
    cfg.add_edge(join, rb, EdgeKind::Unconditional).unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    let method = JavaMethod {
        owner: "ArrayPreservation".into(),
        name: "select".into(),
        is_static: true,
        parameters: vec![
            JavaType::Array(Box::new(JavaType::Double)),
            JavaType::Boolean,
            JavaType::Int,
            JavaType::Double,
        ],
        return_type: object(),
        throws: vec![],
    };
    let bindings = ParameterBindings {
        parameters: params,
        ..Default::default()
    };
    let emitted = crate::emit_method(&ir, &method, &bindings, &Resolver);
    let SourceResult::Complete { source, .. } = emitted else {
        panic!("{emitted:?}")
    };
    let dir = std::env::temp_dir().join(format!("array-copy-emitter-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = format!(
        r#"public class ArrayPreservation {{
{source}
public static void main(String[] args) {{
double[] a={{1,2}}; if(select(a,true,1,7)!=a||a[1]!=7||select(a,false,100,9)!=null||a[1]!=7)throw new AssertionError();
if(select(null,false,0,1)!=null)throw new AssertionError();
try{{select(null,true,0,1);throw new AssertionError();}}catch(NullPointerException expected){{}}
try{{select(a,true,3,1);throw new AssertionError();}}catch(ArrayIndexOutOfBoundsException expected){{}}
if(a[1]!=7)throw new AssertionError();
}}
}}"#
    );
    std::fs::write(dir.join("ArrayPreservation.java"), java).unwrap();
    let jdk = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
    let output = std::process::Command::new(format!("{jdk}/javac"))
        .arg("--release")
        .arg("17")
        .arg(dir.join("ArrayPreservation.java"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = std::process::Command::new(format!("{jdk}/java"))
        .arg("-cp")
        .arg(&dir)
        .arg("ArrayPreservation")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn preserved_producer_copy_into_selected_family_has_exact_destination_contract() {
    let (ir, bindings, _, widened, phi) = fixture(1, true);
    let ManagedInsn::Move {
        value: producer, ..
    } = ir.instruction(ir.value(widened).unwrap().def()).unwrap()
    else {
        panic!("copy required")
    };
    let producer = *producer;
    let plan = checked_phi_plan(
        &ir,
        phi,
        &method_type(),
        &method_type(),
        &Hierarchy::default(),
        &Resolver,
    )
    .unwrap();
    assert!(plan.locals.contains(&widened));
    assert!(!plan.locals.contains(&producer));
    let method = JavaMethod {
        owner: "ProducerBoundary".into(),
        name: "select".into(),
        is_static: true,
        parameters: vec![object(), JavaType::Boolean],
        return_type: method_type(),
        throws: vec![],
    };
    let emitted = crate::emit_method(&ir, &method, &bindings, &Resolver);
    let SourceResult::Complete { source, .. } = emitted else {
        panic!("{emitted:?}")
    };
    let dir = std::env::temp_dir().join(format!("producer-copy-boundary-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ProducerBoundary.java"), format!(r#"public class ProducerBoundary {{
{source}
public static void main(String[] args) throws Exception {{
java.lang.reflect.Method m=String.class.getMethod("length");
if(select(m,true)!=m||select(null,true)!=null||select(new Object(),false)!=null)throw new AssertionError();
try{{select(new Object(),true);throw new AssertionError();}}catch(ClassCastException expected){{}}
}}
}}"#)).unwrap();
    let jdk = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
    let output = std::process::Command::new(format!("{jdk}/javac"))
        .arg("--release")
        .arg("17")
        .arg(dir.join("ProducerBoundary.java"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = std::process::Command::new(format!("{jdk}/java"))
        .arg("-cp")
        .arg(&dir)
        .arg("ProducerBoundary")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // A destination not in the exact selected declaration plan remains unknown.
    let index = MethodIndex::new(&ir);
    let family = HashSet::from([producer]);
    let positions = index.consumer_positions(&family);
    assert!(
        consumer_constraints_at_positions(
            &ir,
            &index,
            &family,
            &positions,
            &method_type(),
            &Resolver,
            &HashMap::new()
        )
        .is_err()
    );
    // An extra unmodeled use of the adopted destination cannot be hidden.
    let mut instructions = ir.instructions().clone();
    instructions.insert(ManagedInsn::Opaque {
        family_tag: 7,
        reads: vec![widened],
        writes: vec![],
        effects: EffectFlags::NONE,
    });
    let invalid = MethodIr::<SsaForm>::from_parts(
        Cfg::new(*ir.cfg().block(ir.cfg().entry()).unwrap().payload()),
        instructions,
        ir.values().clone(),
    );
    assert!(
        checked_phi_plan(
            &invalid,
            phi,
            &method_type(),
            &method_type(),
            &Hierarchy::default(),
            &Resolver
        )
        .is_err()
    );
}

#[test]
fn sibling_selected_joins_share_preserved_concrete_producer_atomically() {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut parameters = Vec::new();
    for ty in [
        ManagedType::Reference(ClassRef::new(2)),
        ManagedType::Boolean,
        ManagedType::Boolean,
    ] {
        let id = insns.insert(ManagedInsn::Return { value: None });
        let value = values.insert(Value::new(id, ty));
        insns[id] = ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![value],
            effects: EffectFlags::NONE,
        };
        parameters.push(value);
    }
    let id = insns.insert(ManagedInsn::Return { value: None });
    let producer = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(2))));
    insns[id] = ManagedInsn::Move {
        result: producer,
        value: parameters[0],
    };
    let mut cfg = Cfg::new(id);
    let mut prior = cfg.entry();
    let mut phis = Vec::new();
    let mut sibling_copy = None;
    for position in 0..2 {
        let branch = insns.insert(ManagedInsn::Return { value: None });
        let entry = cfg.add_block(branch);
        cfg.add_edge(prior, entry, EdgeKind::Unconditional).unwrap();
        let yes_id = insns.insert(ManagedInsn::Return { value: None });
        let yes = cfg.add_block(yes_id);
        let input = if position == 0 {
            producer
        } else {
            let copy = values.insert(Value::new(yes_id, ManagedType::Reference(ClassRef::new(2))));
            insns[yes_id] = ManagedInsn::Move {
                result: copy,
                value: producer,
            };
            sibling_copy = Some(copy);
            copy
        };
        let no_id = insns.insert(ManagedInsn::Return { value: None });
        let empty = values.insert(Value::new(no_id, ManagedType::Reference(ClassRef::new(3))));
        insns[no_id] = ManagedInsn::Const {
            result: empty,
            value: Constant::String(StringRef::new(0)),
        };
        let no = cfg.add_block(no_id);
        let true_edge = cfg.add_edge(entry, yes, EdgeKind::BranchTrue).unwrap();
        let false_edge = cfg.add_edge(entry, no, EdgeKind::BranchFalse).unwrap();
        insns[branch] = ManagedInsn::Branch {
            condition: Some(parameters[position + 1]),
            then_edge: true_edge,
            else_edge: Some(false_edge),
        };
        let phi_id = insns.insert(ManagedInsn::Return { value: None });
        let phi = values.insert(Value::new(phi_id, ManagedType::Unknown));
        let join = cfg.add_block(phi_id);
        let ye = cfg.add_edge(yes, join, EdgeKind::Unconditional).unwrap();
        let ne = cfg.add_edge(no, join, EdgeKind::Unconditional).unwrap();
        if position == 0 {
            insns[yes_id] = ManagedInsn::Branch {
                condition: None,
                then_edge: ye,
                else_edge: None,
            };
        }
        insns[phi_id] = ManagedInsn::Phi {
            result: phi,
            inputs: vec![(ye, input), (ne, empty)],
        };
        phis.push(phi);
        prior = join;
    }
    let array_id = insns.insert(ManagedInsn::Return { value: None });
    let array = values.insert(Value::new(
        array_id,
        ManagedType::Array {
            element: Box::new(ManagedType::Reference(ClassRef::new(0))),
            nullable: true,
        },
    ));
    insns[array_id] = ManagedInsn::NewFilledArray {
        result: Some(array),
        target: TypeRef::new(4),
        elements: phis.clone(),
    };
    let ab = cfg.add_block(array_id);
    cfg.add_edge(prior, ab, EdgeKind::Unconditional).unwrap();
    let ret_id = insns.insert(ManagedInsn::Return { value: Some(array) });
    let rb = cfg.add_block(ret_id);
    cfg.add_edge(ab, rb, EdgeKind::Unconditional).unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
    let return_type = JavaType::Array(Box::new(object()));
    let plan = checked_phi_plan(
        &ir,
        phis[0],
        &object(),
        &return_type,
        &Hierarchy::default(),
        &Resolver,
    )
    .unwrap();
    assert!(phis.iter().all(|value| plan.locals.contains(value)));
    assert!(!plan.locals.contains(&producer));
    assert!(!plan.locals.contains(&sibling_copy.unwrap()));
    let declarations = declaration_plan(&ir, &return_type, &Resolver).unwrap();
    for phi in &phis {
        assert_eq!(declarations.get(phi), Some(&object()));
    }
    let method = JavaMethod {
        owner: "SiblingJoins".into(),
        name: "select".into(),
        is_static: true,
        parameters: vec![
            JavaType::Class("java.lang.Long".into()),
            JavaType::Boolean,
            JavaType::Boolean,
        ],
        return_type,
        throws: vec![],
    };
    let emitted = crate::emit_method(
        &ir,
        &method,
        &ParameterBindings {
            parameters,
            ..Default::default()
        },
        &Resolver,
    );
    let SourceResult::Complete { source, .. } = emitted else {
        panic!("{emitted:?}")
    };
    let dir = std::env::temp_dir().join(format!("sibling-copy-joins-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SiblingJoins.java"),format!(r#"public class SiblingJoins {{
{source}
public static void main(String[] args) {{
Long v=Long.valueOf(987654321L); for(int mask=0;mask<4;mask++){{boolean a=(mask&1)!=0,b=(mask&2)!=0;Object[] r=select(v,a,b);if(r[0]!=(a?v:"")||r[1]!=(b?v:""))throw new AssertionError();Object[] n=select(null,a,b);if(n[0]!=(a?null:"")||n[1]!=(b?null:""))throw new AssertionError();}}
}}
}}"#)).unwrap();
    let jdk = "/home/jtjisgod/tools/jdk-17.0.13+11/bin";
    let output = std::process::Command::new(format!("{jdk}/javac"))
        .arg("--release")
        .arg("17")
        .arg(dir.join("SiblingJoins.java"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = std::process::Command::new(format!("{jdk}/java"))
        .arg("-cp")
        .arg(&dir)
        .arg("SiblingJoins")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut instructions = ir.instructions().clone();
    instructions.insert(ManagedInsn::Opaque {
        family_tag: 7,
        reads: vec![sibling_copy.unwrap()],
        writes: vec![],
        effects: EffectFlags::NONE,
    });
    let invalid = MethodIr::<SsaForm>::from_parts(
        Cfg::new(*ir.cfg().block(ir.cfg().entry()).unwrap().payload()),
        instructions,
        ir.values().clone(),
    );
    assert!(
        checked_phi_plan(
            &invalid,
            phis[0],
            &object(),
            &method.return_type,
            &Hierarchy::default(),
            &Resolver
        )
        .is_err()
    );
}
