//! Different checked contracts share their original linear source scope.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::{
    DeclarationBindings, JavaField, SourceResult,
    type_compatibility::{Declaration, Hierarchy},
};
use decomp_core::cfg::Cfg;
use managed_ir::{
    method::{InsnArena, ValueArena},
    ops::{Constant, InvokeKind},
    refs::{ClassRef, FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};
struct Metadata {
    scope: scopes::RegionScope,
    hierarchy: Hierarchy,
}
impl RegionMetadata for Metadata {
    fn is_catch_all(&self, _: &str) -> bool {
        false
    }
    fn region_scope(&self) -> &scopes::RegionScope {
        &self.scope
    }
    fn hierarchy(&self) -> &Hierarchy {
        &self.hierarchy
    }
    fn catch_type(&self, key: &str) -> Option<JavaType> {
        Some(JavaType::Class(key.into()))
    }
}
struct Resolver {
    bad: bool,
    unresolved: bool,
}
impl JavaResolver for Resolver {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        if self.unresolved && r.index() == 1 {
            return None;
        }
        Some(JavaMethod {
            owner: "GroupedContractHarness".into(),
            name: if r.index() == 0 { "first" } else { "second" }.into(),
            is_static: true,
            return_type: JavaType::Void,
            parameters: vec![JavaType::Int],
            throws: if self.bad {
                vec![]
            } else {
                vec![
                    if r.index() == 0 {
                        "java.io.IOException"
                    } else {
                        "java.lang.ClassNotFoundException"
                    }
                    .into(),
                ]
            },
        })
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
fn fixture() -> (MethodIr<SsaForm>, Metadata, DeclarationBindings) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let incoming = insns.insert(ManagedInsn::Return { value: None });
    let mode = values.insert(Value::new(incoming, ManagedType::Int));
    insns[incoming] = ManagedInsn::Opaque {
        family_tag: 0x8001,
        reads: vec![],
        writes: vec![mode],
        effects: EffectFlags::NONE,
    };
    let first = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(0),
        args: vec![mode],
    });
    let mut cfg = Cfg::new(first);
    let entry = cfg.entry();
    let second = cfg.add_block(insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(1),
        args: vec![mode],
    }));
    cfg.add_edge(entry, second, EdgeKind::Unconditional)
        .unwrap();
    let doneid = insns.insert(ManagedInsn::Return { value: Some(mode) });
    let done = cfg.add_block(doneid);
    cfg.add_edge(second, done, EdgeKind::Unconditional).unwrap();
    let mut catches = vec![];
    for (index, name) in ["java.io.IOException", "java.lang.ClassNotFoundException"]
        .iter()
        .enumerate()
    {
        let constid = insns.insert(ManagedInsn::Return { value: None });
        let value = values.insert(Value::new(constid, ManagedType::Int));
        insns[constid] = ManagedInsn::Const {
            result: value,
            value: Constant::Int(-(index as i32) - 1),
        };
        let handler = cfg.add_block(constid);
        let ret = cfg.add_block(insns.insert(ManagedInsn::Return { value: Some(value) }));
        cfg.add_edge(handler, ret, EdgeKind::Unconditional).unwrap();
        for block in [entry, second] {
            cfg.add_edge(block, handler, EdgeKind::Exception((*name).into()))
                .unwrap();
        }
        catches.push(scopes::Catch {
            key: (*name).into(),
            entry: handler,
        });
    }
    let mut hierarchy = Hierarchy::default();
    for (name, parent) in [
        ("java.lang.Object", None),
        ("java.lang.Throwable", Some("java.lang.Object")),
        ("java.lang.Exception", Some("java.lang.Throwable")),
        ("java.lang.RuntimeException", Some("java.lang.Exception")),
        ("java.lang.Error", Some("java.lang.Throwable")),
        ("java.io.IOException", Some("java.lang.Exception")),
        (
            "java.lang.ClassNotFoundException",
            Some("java.lang.Exception"),
        ),
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
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        Metadata {
            scope: scopes::RegionScope {
                protected: vec![entry, second],
                catches,
                continuation: Some(done),
            },
            hierarchy,
        },
        DeclarationBindings {
            parameters: vec![Some(mode)],
            ..Default::default()
        },
    )
}
fn declaration() -> JavaMethod {
    JavaMethod {
        owner: "GroupedContractHarness".into(),
        name: "generated".into(),
        is_static: true,
        return_type: JavaType::Int,
        parameters: vec![JavaType::Int],
        throws: vec![],
    }
}
#[test]
fn linear_checked_contracts_share_exact_native_scope_and_runtime_priority() {
    let (ir, metadata, bindings) = fixture();
    let result = super::super::emit_method(
        &ir,
        &declaration(),
        &bindings,
        &Resolver {
            bad: false,
            unresolved: false,
        },
        &[&metadata],
    );
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert_eq!(source.matches("try {").count(), 1);
    assert_eq!(source.matches("first(").count(), 1);
    assert_eq!(source.matches("second(").count(), 1);
    let root =
        std::env::temp_dir().join(format!("decomphify-linear-contract-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let java = format!(
        r#"public class GroupedContractHarness {{
 static String trace="";
 static final java.io.IOException IO=new java.io.IOException();
 static final ClassNotFoundException CN=new ClassNotFoundException();
 static void first(int mode) throws java.io.IOException {{trace+="a";if(mode==1)throw IO;}}
 static void second(int mode) throws ClassNotFoundException {{trace+="b";if(mode==2)throw CN;}}
 {source}
 public static void main(String[]args) {{for(int mode=0;mode<3;mode++){{trace="";int expected=mode==0?0:-mode;if(generated(mode)!=expected)throw new AssertionError();if(!trace.equals(mode==1?"a":"ab"))throw new AssertionError(trace);}}}}
}}"#
    );
    std::fs::write(root.join("GroupedContractHarness.java"), java).unwrap();
    let javac = std::process::Command::new("javac")
        .arg(root.join("GroupedContractHarness.java"))
        .output()
        .unwrap();
    assert!(
        javac.status.success(),
        "{}",
        String::from_utf8_lossy(&javac.stderr)
    );
    let run = std::process::Command::new("java")
        .arg("-cp")
        .arg(&root)
        .arg("GroupedContractHarness")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
#[test]
fn missing_or_contradictory_checked_contracts_do_not_gain_group_permission() {
    let (ir, metadata, bindings) = fixture();
    for resolver in [
        Resolver {
            bad: true,
            unresolved: false,
        },
        Resolver {
            bad: false,
            unresolved: true,
        },
    ] {
        let result =
            super::super::emit_method(&ir, &declaration(), &bindings, &resolver, &[&metadata]);
        // Empty genuine contracts may use the existing exact Throwable dispatcher;
        // they cannot obtain a native checked grouping proof.
        if resolver.bad
            && let SourceResult::Complete { source, .. } = &result
        {
            assert!(!source.contains("catch (java.io.IOException"));
        }
        if resolver.unresolved {
            assert!(matches!(result, SourceResult::Unsupported(_)));
        }
    }
}

#[test]
fn shared_target_does_not_erase_different_encoded_catch_kinds() {
    let (ir, _, _) = fixture();
    let source = ir.cfg().entry();
    let edges = ir.cfg().block(source).unwrap().outgoing();
    let io=*edges.iter().find(|e|matches!(ir.cfg().edge(**e).unwrap().kind(),EdgeKind::Exception(name) if name=="java.io.IOException")).unwrap();
    let mut instructions = InsnArena::with_key();
    let id = instructions.insert(ManagedInsn::Return { value: None });
    let mut cfg = Cfg::new(id);
    let from = cfg.entry();
    let target = cfg.add_block(id);
    let a = cfg
        .add_edge(
            from,
            target,
            EdgeKind::Exception("java.io.IOException".into()),
        )
        .unwrap();
    let b = cfg
        .add_edge(
            from,
            target,
            EdgeKind::Exception("java.lang.ClassNotFoundException".into()),
        )
        .unwrap();
    let forged = MethodIr::<SsaForm>::from_parts(cfg, instructions, ValueArena::with_key());
    assert!(!super::same_handler(&forged, a, b));
    assert!(super::same_handler(&ir, io, io));
}
