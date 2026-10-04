#![allow(clippy::unwrap_used)]
use super::*;
use backend_java::{JavaField, JavaMethod};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    value::Value,
};
use std::cell::Cell;
struct Resolver {
    known: bool,
    calls: Cell<usize>,
}
impl JavaResolver for Resolver {
    fn class(&self, c: ClassRef) -> Option<String> {
        Some(
            match c.index() {
                0 => "ChildC",
                1 => "ChildB",
                2 => "Base",
                3 => "Foreign",
                _ => return None,
            }
            .into(),
        )
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        a == b
            || self.known
                && matches!((a,b),(JavaType::Class(a),JavaType::Class(b)) if matches!(a.as_str(),"ChildC"|"ChildB")&&b=="Base")
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        self.calls.set(self.calls.get() + 1);
        Some(JavaField {
            owner: "Base".into(),
            name: "name".into(),
            is_static: false,
            ty: JavaType::Class("java.lang.String".into()),
        })
    }
    fn method(&self, m: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: if m.index() == 0 { "ChildC" } else { "ChildB" }.into(),
            name: "<init>".into(),
            is_static: false,
            parameters: vec![],
            return_type: JavaType::Void,
            throws: vec![],
        })
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
fn fixture(foreign: bool, unknown: bool) -> (MethodIr<SsaForm>, ValueId, ValueId, ValueId) {
    let mut arena = InsnArena::default();
    let mut values = ValueArena::default();
    let condition = arena.insert(ManagedInsn::Return { value: None });
    let input = values.insert(Value::new(condition, ManagedType::Boolean));
    arena[condition] = ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![input],
        effects: EffectFlags::NONE,
    };
    let entry = arena.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(entry);
    let mut make = |class: ClassRef| {
        let id = arena.insert(ManagedInsn::Return { value: None });
        let v = values.insert(Value::new(id, ManagedType::Reference(class)));
        arena[id] = ManagedInsn::NewInstance { result: v, class };
        (id, v)
    };
    let (left, l) = make(ClassRef::new(0));
    let (right, r) = make(ClassRef::new(if foreign { 3 } else { 1 }));
    if unknown {
        values[r] = Value::new(right, ManagedType::Unknown);
        arena[right] = ManagedInsn::Opaque {
            family_tag: 0x8001,
            reads: vec![],
            writes: vec![r],
            effects: EffectFlags::NONE,
        };
    }
    let a = cfg.add_block(left);
    let b = cfg.add_block(right);
    let ea = cfg.add_edge(cfg.entry(), a, EdgeKind::BranchTrue).unwrap();
    let eb = cfg.add_edge(cfg.entry(), b, EdgeKind::BranchFalse).unwrap();
    arena[entry] = ManagedInsn::Branch {
        condition: Some(input),
        then_edge: ea,
        else_edge: Some(eb),
    };
    let joinid = arena.insert(ManagedInsn::Return { value: None });
    let phi = values.insert(Value::new(joinid, ManagedType::Unknown));
    let join = cfg.add_block(joinid);
    let x = cfg.add_edge(a, join, EdgeKind::Unconditional).unwrap();
    let y = cfg.add_edge(b, join, EdgeKind::Unconditional).unwrap();
    arena[joinid] = ManagedInsn::Phi {
        result: phi,
        inputs: vec![(x, l), (y, r)],
    };
    let copyid = arena.insert(ManagedInsn::Return { value: None });
    let copy = values.insert(Value::new(copyid, ManagedType::Unknown));
    arena[copyid] = ManagedInsn::Move {
        result: copy,
        value: phi,
    };
    let cp = cfg.add_block(copyid);
    cfg.add_edge(join, cp, EdgeKind::Unconditional).unwrap();
    let read = arena.insert(ManagedInsn::Return { value: None });
    let value = values.insert(Value::new(read, ManagedType::Unknown));
    arena[read] = ManagedInsn::FieldGet {
        result: value,
        object: Some(copy),
        field: FieldRef::new(0),
    };
    let rd = cfg.add_block(read);
    cfg.add_edge(cp, rd, EdgeKind::Unconditional).unwrap();
    let ret = cfg.add_block(arena.insert(ManagedInsn::Return { value: None }));
    cfg.add_edge(rd, ret, EdgeKind::Unconditional).unwrap();
    (
        MethodIr::<SsaForm>::from_parts(cfg, arena, values),
        phi,
        copy,
        r,
    )
}
#[test]
fn genuine_selected_children_project_once_for_all_consumers_without_mutating_ir() {
    let (ir, phi, copy, initial) = fixture(false, false);
    let r = Resolver {
        known: true,
        calls: Cell::new(0),
    };
    let mut p = Projection::default();
    assert_eq!(
        p.selected_class(&ir, phi, &JavaType::Void, &r),
        Some("Base")
    );
    let once = r.calls.get();
    assert!(once > 0);
    assert_eq!(
        p.selected_class(&ir, copy, &JavaType::Void, &r),
        Some("Base")
    );
    assert_eq!(r.calls.get(), once);
    assert_eq!(p.selected_class(&ir, initial, &JavaType::Void, &r), None);
    assert_eq!(ir.value(phi).unwrap().ty(), &ManagedType::Unknown);
}
#[test]
fn missing_ancestry_foreign_inputs_and_unconstrained_initials_do_not_project() {
    for (foreign, unknown, known) in [
        (false, false, false),
        (true, false, true),
        (false, true, true),
    ] {
        let (ir, phi, copy, initial) = fixture(foreign, unknown);
        let r = Resolver {
            known,
            calls: Cell::new(0),
        };
        let mut p = Projection::default();
        assert_eq!(p.selected_class(&ir, phi, &JavaType::Void, &r), None);
        assert_eq!(p.selected_class(&ir, copy, &JavaType::Void, &r), None);
        assert_eq!(p.selected_class(&ir, initial, &JavaType::Void, &r), None);
    }
}
