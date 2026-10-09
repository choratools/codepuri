//! Immutable plan shape must not cache changing allocation availability.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
};
use std::cell::Cell;

#[derive(Default)]
struct Resolver {
    calls: Cell<usize>,
}
impl JavaResolver for Resolver {
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        self.calls.set(self.calls.get() + 1);
        Some("Fixture.Box".into())
    }
    fn method(&self, method: MethodRef) -> Option<JavaMethod> {
        self.calls.set(self.calls.get() + 1);
        Some(JavaMethod {
            owner: if method.index() == 0 {
                "Fixture.Box"
            } else {
                "Fixture"
            }
            .into(),
            name: if method.index() == 0 {
                "<init>"
            } else {
                "next"
            }
            .into(),
            is_static: method.index() != 0,
            parameters: if method.index() == 0 {
                vec![JavaType::Long]
            } else {
                vec![]
            },
            return_type: if method.index() == 0 {
                JavaType::Void
            } else {
                JavaType::Long
            },
            throws: vec![],
        })
    }
}

#[derive(Default)]
struct Builder {
    instructions: InsnArena,
    values: ValueArena,
    order: Vec<InsnId>,
}
impl Builder {
    fn value(&mut self, ty: ManagedType, make: impl FnOnce(ValueId) -> ManagedInsn) -> ValueId {
        let mut result = None;
        let id = self.instructions.insert_with_key(|id| {
            let value = self.values.insert(Value::new(id, ty));
            result = Some(value);
            make(value)
        });
        self.order.push(id);
        result.unwrap()
    }
    fn push(&mut self, instruction: ManagedInsn) {
        self.order.push(self.instructions.insert(instruction));
    }
    fn allocation(&mut self, argument_effect: bool) -> (ValueId, InsnId) {
        let object = self.value(ManagedType::Reference(ClassRef::new(0)), |result| {
            ManagedInsn::NewInstance {
                result,
                class: ClassRef::new(0),
            }
        });
        let argument = self.value(ManagedType::Long, |result| {
            if argument_effect {
                ManagedInsn::Invoke {
                    kind: InvokeKind::Static,
                    result: Some(result),
                    method: MethodRef::new(1),
                    args: vec![],
                }
            } else {
                ManagedInsn::Const {
                    result,
                    value: Constant::Long(7),
                }
            }
        });
        let gap = *self.order.last().unwrap();
        self.push(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method: MethodRef::new(0),
            args: vec![object, argument],
        });
        (object, gap)
    }
    fn finish(self) -> MethodIr<SsaForm> {
        let mut cfg = Cfg::new(self.order[0]);
        let mut previous = cfg.entry();
        for instruction in &self.order[1..] {
            let next = cfg.add_block(*instruction);
            cfg.add_edge(previous, next, EdgeKind::Unconditional)
                .unwrap();
            previous = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.instructions, self.values)
    }
}
fn emitter<'a>(ir: &'a MethodIr<SsaForm>, resolver: &'a Resolver) -> Emitter<'a, Resolver> {
    Emitter {
        ir,
        resolver,
        values: HashMap::new(),
        lines: vec![],
        next: 0,
        parameters: HashSet::new(),
        reserved: HashSet::new(),
        current: None,
        hoisted: HashSet::new(),
        declarations: HashMap::new(),
        allocation_readable: None,
        object_planning: Default::default(),
    }
}

#[test]
fn cached_definition_evidence_does_not_cache_dynamic_allocation_readability() {
    let mut builder = Builder::default();
    let (object, _) = builder.allocation(true);
    builder.push(ManagedInsn::Return {
        value: Some(object),
    });
    let ir = builder.finish();
    let resolver = Resolver::default();
    let mut e = emitter(&ir, &resolver);
    let allocation = ir.value(object).unwrap().def();
    assert_eq!(e.allocation_root(object), Some(allocation));
    assert!(std::rc::Rc::ptr_eq(
        &e.cached_value_evidence(object),
        &e.cached_value_evidence(object)
    ));
    let unknown = ValueId::default();
    assert!(e.cached_value_evidence(unknown).is_err());
    assert!(std::rc::Rc::ptr_eq(
        &e.cached_value_evidence(unknown),
        &e.cached_value_evidence(unknown)
    ));
    assert_eq!(e.allocation_root(unknown), None);
    assert_eq!(e.allocation_root(unknown), None);
    e.values.insert(
        object,
        ("ready".into(), JavaType::Class("Fixture.Box".into())),
    );
    assert!(e.get(object).is_ok());
    e.hoisted.insert(object);
    assert!(e.get(object).is_err());
    assert_eq!(e.allocation_root(object), Some(allocation));
    e.hoisted.remove(&object);
    assert!(e.get(object).is_ok());
}

#[test]
fn cfg_analysis_and_failures_are_reused_only_within_the_same_method_session() {
    let mut builder = Builder::default();
    let (object, gap) = builder.allocation(true);
    builder.push(ManagedInsn::Return { value: None });
    let ir = builder.finish();
    let resolver = Resolver::default();
    let e = emitter(&ir, &resolver);
    assert!(std::ptr::eq(e.object_dominance(), e.object_dominance()));
    assert!(std::ptr::eq(
        e.object_post_dominance(),
        e.object_post_dominance()
    ));
    assert!(std::ptr::eq(
        e.object_structure_extended(),
        e.object_structure_extended()
    ));
    assert!(
        e.object_dominance()
            .dominates(ir.cfg().entry(), e.object_block_id(gap).unwrap())
    );
    let allocation = ir.value(object).unwrap().def();
    let mut cfg = Cfg::new(allocation);
    cfg.add_block(gap);
    let invalid =
        MethodIr::<SsaForm>::from_parts(cfg, ir.instructions().clone(), ir.values().clone());
    let other = emitter(&invalid, &resolver);
    assert!(other.object_structure_extended().is_err());
    assert!(std::ptr::eq(
        other.object_structure_extended(),
        other.object_structure_extended()
    ));
    assert!(!std::ptr::eq(
        e.object_structure_extended(),
        other.object_structure_extended()
    ));
    let mut static_emitter = emitter(&ir, &resolver);
    static_emitter.object_planning =
        std::rc::Rc::new(ObjectPlanning::for_static_from(&e.object_planning));
    assert!(std::ptr::eq(
        e.object_dominance(),
        static_emitter.object_dominance()
    ));
    assert!(std::ptr::eq(
        e.object_structure_extended(),
        static_emitter.object_structure_extended()
    ));
}

#[test]
fn linear_availability_includes_success_boundary_but_not_preinit_values() {
    let mut builder = Builder::default();
    let (object, gap) = builder.allocation(true);
    let allocation = builder.values[object].def();
    builder.push(ManagedInsn::Return { value: None });
    let ir = builder.finish();
    let resolver = Resolver::default();
    let e = emitter(&ir, &resolver);
    let initializer = e.object_plan(allocation).unwrap().initializer;
    let available = e.after_initializer(initializer).unwrap();
    assert!(available.contains(&initializer));
    assert!(!available.contains(&allocation));
    assert!(!available.contains(&gap));
    let after_gap = e.after_initializer(gap).unwrap();
    let both = available.intersect(&after_gap);
    assert!(matches!(both, InstructionAvailability::Linear { .. }));
    let expected = e.after_initializer(initializer).unwrap().into_set();
    assert_eq!(both.into_set(), expected);
    let graph = InstructionAvailability::Graph(expected.clone());
    assert_eq!(available.intersect(&graph).into_set(), expected);
    assert_eq!(graph.intersect(&available).into_set(), expected);
    assert!(ir.instructions().iter().any(|(id, insn)| {
        matches!(insn, ManagedInsn::Return { .. }) && available.contains(&id)
    }));
    let cached = e.linear_object_order().unwrap();
    assert!(std::ptr::eq(cached, e.linear_object_order().unwrap()));
    let off_cfg = ir
        .instructions()
        .iter()
        .map(|(id, _)| id)
        .find(|id| !cached.positions.contains_key(id));
    if let Some(off_cfg) = off_cfg {
        assert!(e.after_initializer(off_cfg).unwrap().is_empty());
    }
}

#[test]
fn static_reuse_keeps_plan_evidence_but_not_materialized_state_or_expression_policy() {
    let mut builder = Builder::default();
    let (object, gap) = builder.allocation(true);
    builder.push(ManagedInsn::Return { value: None });
    let ir = builder.finish();
    let resolver = Resolver::default();
    let mut ordinary = emitter(&ir, &resolver);
    ordinary.object_planning = std::rc::Rc::new(ObjectPlanning::for_source_emission(true));
    assert!(ordinary.object_argument_instruction(gap));
    ordinary.values.insert(
        object,
        ("ready".into(), JavaType::Class("Fixture.Box".into())),
    );
    assert!(!ordinary.object_argument_instruction(gap));
    let calls = resolver.calls.get();
    let mut static_emitter = emitter(&ir, &resolver);
    static_emitter.object_planning =
        std::rc::Rc::new(ObjectPlanning::for_static_from(&ordinary.object_planning));
    assert!(static_emitter.object_argument_instruction(gap));
    assert_eq!(resolver.calls.get(), calls);
    assert!(!static_emitter.object_planning.pure_expression_enabled);
    assert!(
        static_emitter
            .object_planning
            .pure_expressions
            .get()
            .is_none()
    );
    assert!(static_emitter.values.is_empty());
}

#[test]
fn argument_membership_keeps_pending_state_dynamic_and_pure_gap_ordinary() {
    let mut builder = Builder::default();
    let (first, first_gap) = builder.allocation(true);
    let (second, second_gap) = builder.allocation(true);
    let (_, pure_gap) = builder.allocation(false);
    builder.push(ManagedInsn::Return { value: None });
    let ir = builder.finish();
    let resolver = Resolver::default();
    let mut emitter = emitter(&ir, &resolver);
    assert!(emitter.object_argument_instruction(first_gap));
    assert!(emitter.object_argument_instruction(second_gap));
    assert!(!emitter.object_argument_instruction(pure_gap));
    let calls = resolver.calls.get();
    assert!(calls > 0);
    // Values and hoisting change after the index and selected plans are built.
    emitter.values.insert(
        first,
        ("first".into(), JavaType::Class("Fixture.Box".into())),
    );
    assert!(!emitter.object_argument_instruction(first_gap));
    assert!(emitter.object_argument_instruction(second_gap));
    emitter.hoisted.insert(first);
    assert!(emitter.object_argument_instruction(first_gap));
    emitter.hoisted.remove(&first);
    emitter.values.insert(
        second,
        ("second".into(), JavaType::Class("Fixture.Box".into())),
    );
    for (instruction, _) in ir.instructions() {
        assert!(!emitter.object_argument_instruction(instruction));
    }
    // No plan or unrelated allocation is revisited on membership queries.
    assert_eq!(resolver.calls.get(), calls);
    for object in [first, second] {
        let allocation = ir.value(object).unwrap().def();
        assert_eq!(
            emitter.object_plan(allocation).unwrap().argument_gap.len(),
            1
        );
    }
    assert_eq!(resolver.calls.get(), calls);
}

#[test]
fn cached_failures_keep_original_effect_barrier_error_and_never_fold() {
    let mut builder = Builder::default();
    let (object, gap) = builder.allocation(true);
    let allocation = builder.values[object].def();
    // An uninitialized receiver use is never a representable argument sequence.
    builder.instructions[gap] = ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(1),
        args: vec![object],
    };
    builder.push(ManagedInsn::Return { value: None });
    let ir = builder.finish();
    let resolver = Resolver::default();
    let emitter = emitter(&ir, &resolver);
    let expected =
        Unsupported::Instruction("observable or throwing operation before initializer".into());
    assert!(matches!(emitter.object_plan(allocation), Err(ref e) if e == &expected));
    let calls = resolver.calls.get();
    assert!(calls > 0);
    for _ in 0..8 {
        assert!(!emitter.object_argument_instruction(gap));
        assert!(matches!(emitter.object_plan(allocation), Err(ref e) if e == &expected));
    }
    assert_eq!(resolver.calls.get(), calls);
}

#[test]
fn block_index_retains_first_payload_and_initializer_boundary_rejection() {
    let mut builder = Builder::default();
    let (object, gap) = builder.allocation(true);
    let allocation = builder.values[object].def();
    builder.push(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(allocation);
    let first = cfg.entry();
    let gap_block = cfg.add_block(gap);
    // The first occurrence is a branch barrier. A later identical payload
    // with a valid chain must not replace it in the instruction index.
    cfg.add_edge(first, gap_block, EdgeKind::BranchTrue)
        .unwrap();
    let duplicate = cfg.add_block(allocation);
    let other_gap = cfg.add_block(gap);
    cfg.add_edge(duplicate, other_gap, EdgeKind::Unconditional)
        .unwrap();
    let ir = MethodIr::<SsaForm>::from_parts(cfg, builder.instructions, builder.values);
    let resolver = Resolver::default();
    let emitter = emitter(&ir, &resolver);
    assert_eq!(
        emitter.object_block(allocation).unwrap().outgoing(),
        ir.cfg().block(first).unwrap().outgoing()
    );
    assert!(
        matches!(emitter.object_plan(allocation), Err(Unsupported::Instruction(ref reason)) if reason == "allocation initializer boundary")
    );
    assert!(!emitter.object_argument_instruction(gap));
}
