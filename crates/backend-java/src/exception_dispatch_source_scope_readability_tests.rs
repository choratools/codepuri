//! Readability is per successful CFG point, never a global initialized flag.
#![allow(clippy::unwrap_used)]
use super::*;
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    refs::{ClassRef, MethodRef},
    types::ManagedType,
    value::Value,
};
struct R {
    wrong: bool,
}
impl JavaResolver for R {
    fn field(&self, _: managed_ir::refs::FieldRef) -> Option<crate::JavaField> {
        None
    }
    fn string(&self, _: managed_ir::refs::StringRef) -> Option<Vec<u16>> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Thing".into())
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Some(JavaMethod {
            owner: if self.wrong { "Other" } else { "Thing" }.into(),
            name: if r.index() == 0 { "<init>" } else { "touch" }.into(),
            is_static: false,
            return_type: JavaType::Void,
            parameters: vec![],
            throws: vec![],
        })
    }
    fn exception_allocation_chain(&self, _: &[InsnId]) -> bool {
        true
    }
}
#[test]
fn successful_exit_only_no_overlay_global_initialization() {
    for mode in 0..5 {
        let mut insns = InsnArena::default();
        let mut values = ValueArena::default();
        let root = insns.insert(ManagedInsn::Return { value: None });
        let value = values.insert(Value::new(root, ManagedType::Reference(ClassRef::new(0))));
        insns[root] = ManagedInsn::NewInstance {
            result: value,
            class: ClassRef::new(0),
        };
        let init = insns.insert(ManagedInsn::Invoke {
            kind: managed_ir::ops::InvokeKind::Special,
            result: None,
            method: MethodRef::new(0),
            args: vec![value],
        });
        let call = insns.insert(ManagedInsn::Invoke {
            kind: managed_ir::ops::InvokeKind::Virtual,
            result: None,
            method: MethodRef::new(1),
            args: vec![value],
        });
        if mode == 4 {
            insns[call] = ManagedInsn::Invoke {
                kind: managed_ir::ops::InvokeKind::Special,
                result: None,
                method: MethodRef::new(0),
                args: vec![value],
            };
        }
        let handler = insns.insert(ManagedInsn::Return { value: None });
        let initial = insns.insert(ManagedInsn::Return { value: None });
        let mut cfg = Cfg::new(initial);
        let a = cfg.add_block(root);
        let i = cfg.add_block(init);
        let u = cfg.add_block(call);
        let h = cfg.add_block(handler);
        cfg.add_edge(cfg.entry(), a, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(a, i, EdgeKind::Unconditional).unwrap();
        cfg.add_edge(i, u, EdgeKind::Unconditional).unwrap();
        if mode == 1 {
            cfg.add_edge(i, h, EdgeKind::Exception("E".into())).unwrap();
            cfg.add_edge(h, u, EdgeKind::Unconditional).unwrap();
        }
        if mode == 2 {
            cfg.add_edge(cfg.entry(), u, EdgeKind::Unconditional)
                .unwrap();
        }
        let ir = MethodIr::<SsaForm>::from_parts(cfg, insns, values);
        let r = R { wrong: mode == 3 };
        let mut emitter = crate::Emitter {
            ir: &ir,
            resolver: &r,
            values: HashMap::from([(value, ("v0".into(), JavaType::Class("Thing".into())))]),
            lines: vec![],
            next: 0,
            parameters: HashSet::new(),
            reserved: HashSet::new(),
            current: Some(call),
            hoisted: HashSet::from([value]),
            declarations: HashMap::new(),
            allocation_readable: None,
            object_planning: Default::default(),
        };
        let reads = successful_allocation_reads(&emitter).unwrap();
        assert_eq!(
            reads.get(&value).is_some_and(|s| s.contains(&call)),
            mode == 0 || mode == 4
        );
        assert!(
            !reads
                .get(&value)
                .is_some_and(|s| s.contains(&init) || s.contains(&root) || s.contains(&handler))
        );
        assert!(emitter.hoisted.contains(&value));
        assert!(emitter.pending_allocation(value));
        emitter.allocation_readable = Some(std::sync::Arc::new(reads));
        assert_eq!(emitter.pending_allocation(value), !(mode == 0 || mode == 4));
        emitter.current = Some(init);
        assert!(emitter.pending_allocation(value));
        emitter.current = Some(root);
        assert!(emitter.pending_allocation(value));
        emitter.current = None;
        assert!(emitter.pending_allocation(value));
        emitter.current = Some(call);
        if mode == 0 {
            emitter.current = Some(init);
            let method = JavaMethod {
                owner: "Thing".into(),
                name: "run".into(),
                is_static: true,
                return_type: JavaType::Void,
                parameters: vec![],
                throws: vec![],
            };
            emitter
                .instruction(ir.instruction(init).unwrap(), &method)
                .unwrap();
            assert!(!emitter.hoisted.contains(&value));
            emitter.allocation_readable = None;
            assert!(!emitter.pending_allocation(value));
        }
        if mode == 4 {
            let method = JavaMethod {
                owner: "Thing".into(),
                name: "run".into(),
                is_static: true,
                return_type: JavaType::Void,
                parameters: vec![],
                throws: vec![],
            };
            assert!(
                matches!(emitter.instruction(ir.instruction(call).unwrap(), &method),
                Err(Unsupported::Instruction(reason)) if reason == "repeated or unpaired initializer")
            );
            emitter.allocation_readable = None;
            assert!(emitter.hoisted.contains(&value));
            assert!(emitter.pending_allocation(value));
        }
    }
}
