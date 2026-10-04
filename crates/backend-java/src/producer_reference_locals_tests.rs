#![allow(clippy::unwrap_used)]
use super::*;
use crate::{DeclarationBindings, JavaField, JavaMethod, SourceResult};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    ops::InvokeKind,
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};
struct Resolver(Option<JavaType>);
impl JavaResolver for Resolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        let (owner, name, static_, parameters, return_type) = match r.index() {
            0 => ("ProducerRuntime", "produce", true, vec![], self.0.clone()?),
            1 => (
                "java.lang.String",
                "equals",
                false,
                vec![JavaType::Class("java.lang.Object".into())],
                JavaType::Boolean,
            ),
            2 => (
                "java.lang.StringBuilder",
                "append",
                false,
                vec![JavaType::Class("java.lang.String".into())],
                JavaType::Class("java.lang.StringBuilder".into()),
            ),
            _ => return None,
        };
        Some(JavaMethod {
            owner: owner.into(),
            name: name.into(),
            is_static: static_,
            parameters,
            return_type,
            throws: vec![],
        })
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Some(
            match r.index() {
                0 => "java.lang.Object",
                1 => "java.lang.String",
                _ => "java.lang.StringBuilder",
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
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        a == b || a.reference() && *b == JavaType::Class("java.lang.Object".into())
    }
}
fn fixture(mode: u8) -> (MethodIr<SsaForm>, DeclarationBindings, ValueId, ValueId) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let initial = insns.insert(ManagedInsn::Return { value: None });
    let builder = values.insert(Value::new(
        initial,
        ManagedType::Reference(ClassRef::new(2)),
    ));
    insns[initial] = ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![builder],
        effects: EffectFlags::NONE,
    };
    let receiver_def = insns.insert(ManagedInsn::Return { value: None });
    let receiver = values.insert(Value::new(
        receiver_def,
        ManagedType::Reference(ClassRef::new(1)),
    ));
    insns[receiver_def] = ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![receiver],
        effects: EffectFlags::NONE,
    };
    let producer = insns.insert(ManagedInsn::Return { value: None });
    let result = values.insert(Value::new(
        producer,
        ManagedType::Reference(ClassRef::new(0)),
    ));
    insns[producer] = ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: Some(result),
        method: MethodRef::new(0),
        args: vec![],
    };
    let mut cfg = Cfg::new(producer);
    let mut previous = cfg.entry();
    let copy = insns.insert(ManagedInsn::Return { value: None });
    let copied = values.insert(Value::new(copy, ManagedType::Reference(ClassRef::new(0))));
    insns[copy] = ManagedInsn::Move {
        result: copied,
        value: result,
    };
    let copyblock = cfg.add_block(copy);
    cfg.add_edge(previous, copyblock, EdgeKind::Unconditional)
        .unwrap();
    previous = copyblock;
    let equal = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Virtual,
        result: None,
        method: MethodRef::new(1),
        args: vec![receiver, copied],
    });
    let append = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Virtual,
        result: None,
        method: MethodRef::new(2),
        args: vec![builder, copied],
    });
    for id in [equal, append] {
        let b = cfg.add_block(id);
        cfg.add_edge(previous, b, EdgeKind::Unconditional).unwrap();
        previous = b;
    }
    if mode != 0 {
        let id = insns.insert(ManagedInsn::Return { value: None });
        let block = cfg.add_block(id);
        let edge = cfg
            .add_edge(previous, block, EdgeKind::Unconditional)
            .unwrap();
        previous = block;
        insns[id] = match mode {
            1 => ManagedInsn::Opaque {
                family_tag: 123,
                reads: vec![copied],
                writes: vec![],
                effects: EffectFlags::NONE,
            },
            2 => {
                let phi = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
                ManagedInsn::Phi {
                    result: phi,
                    inputs: vec![(edge, copied)],
                }
            }
            _ => ManagedInsn::Opaque {
                family_tag: 124,
                reads: vec![],
                writes: vec![copied],
                effects: EffectFlags::NONE,
            },
        };
    }
    let ret = insns.insert(ManagedInsn::Return {
        value: Some(copied),
    });
    let end = cfg.add_block(ret);
    cfg.add_edge(previous, end, EdgeKind::Unconditional)
        .unwrap();
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        DeclarationBindings {
            receiver: None,
            parameters: vec![Some(builder), Some(receiver)],
            names: vec![],
        },
        result,
        copied,
    )
}
#[test]
fn exact_producer_contract_and_all_copy_consumers_required() {
    let string = JavaType::Class("java.lang.String".into());
    let (ir, _, seed, copied) = fixture(0);
    let plan = declarations(&ir, &string, &Resolver(Some(string.clone())));
    assert_eq!(plan.get(&seed), Some(&string));
    assert_eq!(plan.get(&copied), Some(&string));
    for contract in [None, Some(JavaType::Class("java.lang.Object".into()))] {
        assert!(declarations(&ir, &string, &Resolver(contract)).is_empty());
    }
    for mode in [1, 2, 3] {
        let (ir, _, _, _) = fixture(mode);
        assert!(declarations(&ir, &string, &Resolver(Some(string.clone()))).is_empty());
    }
}
#[test]
fn generated_producer_copy_equals_and_string_append_jdk() {
    let (ir, bindings, _, _) = fixture(0);
    let string = JavaType::Class("java.lang.String".into());
    let method = JavaMethod {
        owner: "ProducerRuntime".into(),
        name: "generated".into(),
        is_static: true,
        return_type: string.clone(),
        parameters: vec![
            JavaType::Class("java.lang.StringBuilder".into()),
            string.clone(),
        ],
        throws: vec![],
    };
    let output = crate::emit_method_declaration(&ir, &method, &bindings, &Resolver(Some(string)));
    let SourceResult::Complete { source, .. } = output else {
        panic!("{output:?}")
    };
    assert!(!source.contains("(java.lang.String)"));
    let root = std::env::temp_dir().join(format!(
        "decomphify-producer-reference-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let java = format!(
        r#"public class ProducerRuntime {{static int effects;static String produce(){{effects++;return "source";}}{source}
      public static void main(String[] args){{StringBuilder b=new StringBuilder();String result=generated(b,"source");if(!result.equals("source")||!b.toString().equals("source")||effects!=1)throw new AssertionError();System.out.println("PASS");}}}}"#
    );
    std::fs::write(root.join("ProducerRuntime.java"), java).unwrap();
    let compile = std::process::Command::new("javac")
        .arg("ProducerRuntime.java")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new("java")
        .args(["-cp", ".", "ProducerRuntime"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn same_fixture_without_producer_declarations_fails_at_string_consumer() {
    let (ir, bindings, _, copied) = fixture(0);
    let string = JavaType::Class("java.lang.String".into());
    let resolver = Resolver(Some(string.clone()));
    let method = JavaMethod {
        owner: "ProducerRuntime".into(),
        name: "before".into(),
        is_static: true,
        return_type: string.clone(),
        parameters: vec![JavaType::Class("java.lang.StringBuilder".into()), string],
        throws: vec![],
    };
    let mut emitter = crate::Emitter {
        allocation_readable: None,
        object_planning: Default::default(),
        ir: &ir,
        resolver: &resolver,
        values: HashMap::new(),
        lines: vec![],
        next: 0,
        parameters: HashSet::new(),
        reserved: HashSet::new(),
        current: None,
        hoisted: HashSet::new(),
        declarations: HashMap::new(),
    };
    for (index, value) in bindings.parameters.iter().enumerate() {
        crate::bind(
            &mut emitter,
            value.unwrap(),
            format!("p{index}"),
            method.parameters[index].clone(),
        )
        .unwrap();
    }
    let mut block = ir.cfg().entry();
    loop {
        let node = ir.cfg().block(block).unwrap();
        let id = *node.payload();
        emitter.current = Some(id);
        if let Err(error) = emitter.instruction(ir.instruction(id).unwrap(), &method) {
            assert!(
                matches!(ir.instruction(id).unwrap(), ManagedInsn::Invoke { method, args, .. } if *method == MethodRef::new(2) && args.contains(&copied))
            );
            assert_eq!(
                error,
                crate::Unsupported::Type(
                    "assignment Class(\"java.lang.Object\") to Class(\"java.lang.String\")".into()
                )
            );
            break;
        }
        block = ir.cfg().edge(node.outgoing()[0]).unwrap().target();
    }
}
