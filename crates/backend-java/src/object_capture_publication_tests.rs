#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    value::Value,
};

struct Resolver;
impl JavaResolver for Resolver {
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Fixture.Box".into())
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn instruction_type(&self, _: TypeRef) -> Option<JavaType> {
        Some(JavaType::Array(Box::new(JavaType::Class(
            "Fixture.Box".into(),
        ))))
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(match r.index() {
            0 | 2 => JavaMethod {
                owner: "Fixture.Box".into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: vec![JavaType::Int],
                throws: if r.index() == 2 {
                    vec!["java.io.IOException".into()]
                } else {
                    vec![]
                },
            },
            _ => JavaMethod {
                owner: "Fixture".into(),
                name: "id".into(),
                is_static: true,
                return_type: JavaType::Int,
                parameters: vec![JavaType::Int],
                throws: vec![],
            },
        })
    }
}

#[derive(Default)]
struct Builder {
    insns: InsnArena,
    values: ValueArena,
}
impl Builder {
    fn value(
        &mut self,
        ty: ManagedType,
        f: impl FnOnce(ValueId) -> ManagedInsn,
    ) -> (InsnId, ValueId) {
        let mut value = None;
        let id = self.insns.insert_with_key(|id| {
            let v = self.values.insert(Value::new(id, ty));
            value = Some(v);
            f(v)
        });
        (id, value.unwrap())
    }
    fn insn(&mut self, insn: ManagedInsn) -> InsnId {
        self.insns.insert(insn)
    }
    fn finish(self, cfg: Cfg<InsnId>) -> MethodIr<SsaForm> {
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}

fn signature() -> JavaMethod {
    JavaMethod {
        owner: "Fixture".into(),
        name: "make".into(),
        is_static: true,
        return_type: JavaType::Array(Box::new(JavaType::Class("Fixture.Box".into()))),
        parameters: vec![JavaType::Boolean],
        throws: vec![],
    }
}

fn fixture(pre_init_alias: bool, throwing_initializer: bool) -> (MethodIr<SsaForm>, ValueId) {
    let mut b = Builder::default();
    let (flag_def, flag) = b.value(ManagedType::Boolean, |v| ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![v],
        effects: managed_ir::effects::EffectFlags::NONE,
    });
    let branch = b.insn(ManagedInsn::Return { value: None });
    let (first_new, first) = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    let (one_def, one) = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    let first_init = b.insn(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![first, one],
    });
    let (second_new, second) = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    let (alias_def, alias) = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::Move {
            result,
            value: first,
        }
    });
    let (two_def, two) = b.value(ManagedType::Int, |result| ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: Some(result),
        method: MethodRef::new(1),
        args: vec![one],
    });
    let second_init = b.insn(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(if throwing_initializer { 2 } else { 0 }),
        args: vec![second, two],
    });
    let (array_def, array) = b.value(
        ManagedType::Array {
            element: Box::new(ManagedType::Reference(ClassRef::new(0))),
            nullable: true,
        },
        |result| ManagedInsn::NewFilledArray {
            result: Some(result),
            target: TypeRef::new(0),
            elements: vec![alias, second],
        },
    );
    let ret = b.insn(ManagedInsn::Return { value: Some(array) });

    let mut cfg = Cfg::new(branch);
    let first_block = cfg.add_block(first_new);
    let one_block = cfg.add_block(one_def);
    let first_init_block = cfg.add_block(first_init);
    let second_block = cfg.add_block(second_new);
    let alias_block = cfg.add_block(alias_def);
    let two_block = cfg.add_block(two_def);
    let second_init_block = cfg.add_block(second_init);
    let array_block = cfg.add_block(array_def);
    let ret_block = cfg.add_block(ret);

    let yes = cfg
        .add_edge(cfg.entry(), first_block, EdgeKind::BranchTrue)
        .unwrap();
    let no = cfg
        .add_edge(cfg.entry(), first_block, EdgeKind::BranchFalse)
        .unwrap();
    b.insns[branch] = ManagedInsn::Branch {
        condition: Some(flag),
        then_edge: yes,
        else_edge: Some(no),
    };
    if pre_init_alias {
        cfg.add_edge(first_block, one_block, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(one_block, second_block, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(second_block, alias_block, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(alias_block, first_init_block, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(first_init_block, two_block, EdgeKind::Unconditional)
            .unwrap();
    } else {
        cfg.add_edge(first_block, one_block, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(one_block, first_init_block, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(first_init_block, second_block, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(second_block, alias_block, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(alias_block, two_block, EdgeKind::Unconditional)
            .unwrap();
    }
    cfg.add_edge(two_block, second_init_block, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(second_init_block, array_block, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(array_block, ret_block, EdgeKind::Unconditional)
        .unwrap();

    let _ = flag_def;
    (b.finish(cfg), flag)
}

fn emit_path_conditional_alias() -> SourceResult {
    let mut b = Builder::default();
    let (flag_def, flag) = b.value(ManagedType::Boolean, |v| ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![v],
        effects: managed_ir::effects::EffectFlags::NONE,
    });
    let (new_id, object) = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    let (one_id, one) = b.value(ManagedType::Int, |result| ManagedInsn::Const {
        result,
        value: Constant::Int(1),
    });
    let init = b.insn(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(0),
        args: vec![object, one],
    });
    let branch = b.insn(ManagedInsn::Return { value: None });
    let (alias_id, alias) = b.value(ManagedType::Reference(ClassRef::new(0)), |result| {
        ManagedInsn::Move {
            result,
            value: object,
        }
    });
    let (array_id, array) = b.value(
        ManagedType::Array {
            element: Box::new(ManagedType::Reference(ClassRef::new(0))),
            nullable: true,
        },
        |result| ManagedInsn::NewFilledArray {
            result: Some(result),
            target: TypeRef::new(0),
            elements: vec![alias],
        },
    );
    let ret = b.insn(ManagedInsn::Return { value: Some(array) });
    let mut cfg = Cfg::new(new_id);
    let one_block = cfg.add_block(one_id);
    let init_block = cfg.add_block(init);
    let branch_block = cfg.add_block(branch);
    let alias_block = cfg.add_block(alias_id);
    let array_block = cfg.add_block(array_id);
    let ret_block = cfg.add_block(ret);
    cfg.add_edge(cfg.entry(), one_block, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(one_block, init_block, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(init_block, branch_block, EdgeKind::Unconditional)
        .unwrap();
    let yes = cfg
        .add_edge(branch_block, alias_block, EdgeKind::BranchTrue)
        .unwrap();
    let no = cfg
        .add_edge(branch_block, array_block, EdgeKind::BranchFalse)
        .unwrap();
    b.insns[branch] = ManagedInsn::Branch {
        condition: Some(flag),
        then_edge: yes,
        else_edge: Some(no),
    };
    cfg.add_edge(alias_block, array_block, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(array_block, ret_block, EdgeKind::Unconditional)
        .unwrap();
    let _ = flag_def;
    let ir = b.finish(cfg);
    emit_method(
        &ir,
        &signature(),
        &ParameterBindings {
            parameters: vec![flag],
            ..Default::default()
        },
        &Resolver,
    )
}

fn emit(pre_init_alias: bool, throwing_initializer: bool) -> SourceResult {
    let (ir, flag) = fixture(pre_init_alias, throwing_initializer);
    emit_method(
        &ir,
        &signature(),
        &ParameterBindings {
            parameters: vec![flag],
            ..Default::default()
        },
        &Resolver,
    )
}

#[test]
fn sequence_capture_publication_makes_initialized_move_alias_readable_for_array() {
    let result = emit(false, false);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert!(
        source.contains("compiler scaffold: original allocation argument sequence"),
        "{source}"
    );
    assert!(source.contains("new Fixture.Box[]"), "{source}");
}

#[test]
fn sequence_capture_publication_preserves_preinit_and_path_denial() {
    assert!(matches!(emit(true, false), SourceResult::Unsupported(_)));
    assert!(matches!(emit(false, true), SourceResult::Unsupported(_)));
    assert!(matches!(
        emit_path_conditional_alias(),
        SourceResult::Unsupported(_)
    ));
}
