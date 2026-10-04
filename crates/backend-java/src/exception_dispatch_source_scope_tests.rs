//! Different checked contracts retain one exact branched/cyclic original source scope.
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
        if r.index() >= 2 {
            return Some(JavaMethod {
                owner: "GroupedContractHarness".into(),
                name: if r.index() == 2 {
                    "again"
                } else if r.index() == 4 {
                    "observeCounter"
                } else {
                    "observeCaught"
                }
                .into(),
                is_static: true,
                return_type: if r.index() == 2 {
                    JavaType::Boolean
                } else {
                    JavaType::Void
                },
                parameters: if r.index() == 2 {
                    vec![]
                } else if r.index() == 4 {
                    vec![JavaType::Int]
                } else {
                    vec![JavaType::Class("java.lang.Throwable".into())]
                },
                throws: vec![],
            });
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
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        a == b
            || matches!((a,b),(JavaType::Class(a),JavaType::Class(b)) if b=="java.lang.Throwable" && ["java.io.IOException","java.lang.ClassNotFoundException"].contains(&a.as_str()))
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

fn copy_cfg(source: &Cfg<InsnId>) -> Cfg<InsnId> {
    let mut cfg = Cfg::new(*source.block(source.entry()).unwrap().payload());
    assert_eq!(cfg.entry(), source.entry());
    for id in source.block_ids().filter(|id| *id != source.entry()) {
        assert_eq!(cfg.add_block(*source.block(id).unwrap().payload()), id);
    }
    for id in source.edge_ids() {
        let e = source.edge(id).unwrap();
        assert_eq!(
            cfg.add_edge(e.source(), e.target(), e.kind().clone())
                .unwrap(),
            id
        );
    }
    cfg
}
fn cyclic_fixture() -> (MethodIr<SsaForm>, Metadata, DeclarationBindings) {
    let (ir, mut md, bindings) = fixture();
    let mut cfg = copy_cfg(ir.cfg());
    let mut insns = ir.instructions().clone();
    let mut values = ir.values().clone();
    let first = cfg.entry();
    let second = md.scope.protected[1];
    let done = md.scope.continuation.unwrap();
    let old = *cfg
        .block(second)
        .unwrap()
        .outgoing()
        .iter()
        .find(|e| matches!(cfg.edge(**e).unwrap().kind(), EdgeKind::Unconditional))
        .unwrap();
    let id = insns.insert(ManagedInsn::Return { value: None });
    let again = values.insert(Value::new(id, ManagedType::Boolean));
    insns[id] = ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: Some(again),
        method: MethodRef::new(2),
        args: vec![],
    };
    let call = cfg.add_block(id);
    cfg.redirect_edge(old, call).unwrap();
    let bid = insns.insert(ManagedInsn::Return { value: None });
    let branch = cfg.add_block(bid);
    cfg.add_edge(call, branch, EdgeKind::Unconditional).unwrap();
    let yes = cfg.add_edge(branch, first, EdgeKind::BranchTrue).unwrap();
    let no = cfg.add_edge(branch, done, EdgeKind::BranchFalse).unwrap();
    insns[bid] = ManagedInsn::Branch {
        condition: Some(again),
        then_edge: yes,
        else_edge: Some(no),
    };
    for c in &md.scope.catches {
        cfg.add_edge(call, c.entry, EdgeKind::Exception(c.key.clone()))
            .unwrap();
    }
    md.scope.protected.extend([call, branch]);
    // Preserve and observe the original caught exception identities.
    for c in &mut md.scope.catches {
        let captureid = insns.insert(ManagedInsn::Return { value: None });
        let value = values.insert(Value::new(captureid, ManagedType::Unknown));
        insns[captureid] = ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![value],
            effects: EffectFlags::MAY_THROW,
        };
        let capture = cfg.add_block(captureid);
        let record = cfg.add_block(insns.insert(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            result: None,
            method: MethodRef::new(3),
            args: vec![value],
        }));
        cfg.add_edge(capture, record, EdgeKind::Unconditional)
            .unwrap();
        cfg.add_edge(record, c.entry, EdgeKind::Unconditional)
            .unwrap();
        let incoming = cfg.block(c.entry).unwrap().incoming().to_vec();
        for e in incoming {
            if matches!(cfg.edge(e).unwrap().kind(), EdgeKind::Exception(_)) {
                cfg.redirect_edge(e, capture).unwrap();
            }
        }
        c.entry = capture;
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        md,
        bindings,
    )
}
#[test]
fn cyclic_source_scope_retains_original_order_identity_and_effects() {
    let (ir, md, bindings) = cyclic_fixture();
    let resolver = Resolver {
        bad: false,
        unresolved: false,
    };
    let result = super::super::emit_method(&ir, &declaration(), &bindings, &resolver, &[&md]);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert_eq!(source.matches("try {").count(), 1);
    assert!(source.contains("_scope0: while"));
    assert!(
        source.find("catch (java.io.IOException").unwrap()
            < source
                .find("catch (java.lang.ClassNotFoundException")
                .unwrap()
    );
    let root = std::env::temp_dir().join(format!("source-scope-runtime-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let java = format!(
        r#"public class GroupedContractHarness {{
 static String trace="";static int turns;static Throwable seen;
 static final java.io.IOException IO=new java.io.IOException();static final ClassNotFoundException CN=new ClassNotFoundException();static final Error ERROR=new Error();
 static void first(int m)throws java.io.IOException{{trace+="a";if(m==1)throw IO;if(m==3)throw ERROR;}}
 static void second(int m)throws ClassNotFoundException{{trace+="b";if(m==2)throw CN;}}
 static boolean again(){{trace+="z";return ++turns<2;}}
 static void observeCaught(Throwable t){{seen=t;trace+="x";}}
 {source}
 public static void main(String[]args){{for(int m=0;m<4;m++){{trace="";turns=0;seen=null;try{{int got=generated(m);if(got!=(m==0?0:-m))throw new AssertionError(got);}}catch(Error e){{if(m!=3||e!=ERROR)throw e;}}
 if(!trace.equals(m==0?"abzabz":m==1?"ax":m==2?"abx":"a"))throw new AssertionError(trace);
 if(m==1&&seen!=IO||m==2&&seen!=CN||m==3&&seen!=null)throw new AssertionError("identity");}}}}
 }}"#
    );
    std::fs::write(root.join("GroupedContractHarness.java"), java).unwrap();
    let c = std::process::Command::new("javac")
        .arg(root.join("GroupedContractHarness.java"))
        .output()
        .unwrap();
    assert!(c.status.success(), "{}", String::from_utf8_lossy(&c.stderr));
    let r = std::process::Command::new("java")
        .arg("-cp")
        .arg(&root)
        .arg("GroupedContractHarness")
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
}
#[test]
fn unknown_contract_and_foreign_exception_entry_do_not_obtain_group() {
    let (ir, md, b) = cyclic_fixture();
    let unresolved = Resolver {
        bad: false,
        unresolved: true,
    };
    assert!(matches!(
        super::super::emit_method(&ir, &declaration(), &b, &unresolved, &[&md]),
        SourceResult::Unsupported(_)
    ));
    let mut cfg = copy_cfg(ir.cfg());
    let outside = cfg.add_block(*cfg.block(cfg.entry()).unwrap().payload());
    cfg.add_edge(
        outside,
        md.scope.protected[1],
        EdgeKind::Exception("java.io.IOException".into()),
    )
    .unwrap();
    let changed =
        MethodIr::<SsaForm>::from_parts(cfg, ir.instructions().clone(), ir.values().clone());
    assert!(matches!(
        super::super::emit_method(
            &changed,
            &declaration(),
            &b,
            &Resolver {
                bad: false,
                unresolved: false
            },
            &[&md]
        ),
        SourceResult::Unsupported(_)
    ));
}
#[test]
fn multiple_normal_exits_work_but_missing_hierarchy_and_catch_kind_mismatch_fail_closed() {
    let (ir, mut md, bindings) = cyclic_fixture();
    let mut cfg = copy_cfg(ir.cfg());
    let back = cfg
        .edge_ids()
        .find(|e| matches!(cfg.edge(*e).unwrap().kind(), EdgeKind::BranchTrue))
        .unwrap();
    cfg.redirect_edge(back, md.scope.continuation.unwrap())
        .unwrap();
    let escaped =
        MethodIr::<SsaForm>::from_parts(cfg, ir.instructions().clone(), ir.values().clone());
    let r = Resolver {
        bad: false,
        unresolved: false,
    };
    assert!(matches!(
        super::super::emit_method(&escaped, &declaration(), &bindings, &r, &[&md]),
        SourceResult::Complete { .. }
    ));
    md.hierarchy = Hierarchy::default();
    assert!(matches!(
        super::super::emit_method(&ir, &declaration(), &bindings, &r, &[&md]),
        SourceResult::Unsupported(_)
    ));
    let mut cfg = Cfg::new(*ir.cfg().block(ir.cfg().entry()).unwrap().payload());
    let from = cfg.entry();
    let target = cfg.add_block(*ir.cfg().block(ir.cfg().entry()).unwrap().payload());
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
    let changed =
        MethodIr::<SsaForm>::from_parts(cfg, ir.instructions().clone(), ir.values().clone());
    assert!(!super::same_handler(&changed, a, b));
}

fn outside_reentry_fixture() -> (MethodIr<SsaForm>, Metadata, DeclarationBindings) {
    let (ir, mut md, bindings) = cyclic_fixture();
    let mut cfg = copy_cfg(ir.cfg());
    let mut insns = ir.instructions().clone();
    let mut values = ir.values().clone();
    let first = cfg.entry();
    let second = md.scope.protected[1];
    let incoming = cfg
        .block(first)
        .unwrap()
        .outgoing()
        .iter()
        .copied()
        .find(|e| matches!(cfg.edge(*e).unwrap().kind(), EdgeKind::Unconditional))
        .unwrap();
    let back = cfg
        .edge_ids()
        .find(|e| matches!(cfg.edge(*e).unwrap().kind(), EdgeKind::BranchTrue))
        .unwrap();
    let phi_id = insns.insert(ManagedInsn::Return { value: None });
    let counter = values.insert(Value::new(phi_id, ManagedType::Int));
    let phi = cfg.add_block(phi_id);
    cfg.redirect_edge(incoming, phi).unwrap();
    let observe = cfg.add_block(insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Static,
        result: None,
        method: MethodRef::new(4),
        args: vec![counter],
    }));
    cfg.add_edge(phi, observe, EdgeKind::Unconditional).unwrap();
    cfg.add_edge(observe, second, EdgeKind::Unconditional)
        .unwrap();
    for c in &md.scope.catches {
        cfg.add_edge(observe, c.entry, EdgeKind::Exception(c.key.clone()))
            .unwrap();
    }
    let after_second = cfg
        .block(second)
        .unwrap()
        .outgoing()
        .iter()
        .copied()
        .find(|e| matches!(cfg.edge(*e).unwrap().kind(), EdgeKind::Unconditional))
        .unwrap();
    let call_again = cfg.edge(after_second).unwrap().target();
    let one_id = insns.insert(ManagedInsn::Return { value: None });
    let one = values.insert(Value::new(one_id, ManagedType::Int));
    insns[one_id] = ManagedInsn::Const {
        result: one,
        value: Constant::Int(1),
    };
    let one_block = cfg.add_block(one_id);
    cfg.redirect_edge(after_second, one_block).unwrap();
    let add_id = insns.insert(ManagedInsn::Return { value: None });
    let incremented = values.insert(Value::new(add_id, ManagedType::Int));
    insns[add_id] = ManagedInsn::Arithmetic {
        op: managed_ir::ops::ArithOp::Add,
        result: incremented,
        lhs: counter,
        rhs: one,
    };
    let add = cfg.add_block(add_id);
    cfg.add_edge(one_block, add, EdgeKind::Unconditional)
        .unwrap();
    cfg.add_edge(add, call_again, EdgeKind::Unconditional)
        .unwrap();
    // This genuine goto is outside the exact protected set. It reenters its
    // second original normal entry; its edge selects the new Phi snapshot.
    let goto_id = insns.insert(ManagedInsn::Return { value: None });
    let outside = cfg.add_block(goto_id);
    cfg.redirect_edge(back, outside).unwrap();
    let reenter = cfg.add_edge(outside, phi, EdgeKind::Unconditional).unwrap();
    insns[goto_id] = ManagedInsn::Branch {
        condition: None,
        then_edge: reenter,
        else_edge: None,
    };
    insns[phi_id] = ManagedInsn::Phi {
        result: counter,
        inputs: vec![
            (incoming, bindings.parameters[0].unwrap()),
            (reenter, incremented),
        ],
    };
    md.scope.protected.extend([phi, observe, one_block, add]);
    assert!(!md.scope.protected.contains(&outside));
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        md,
        bindings,
    )
}

#[test]
fn exact_scope_multiple_entries_exits_and_outside_goto_preserve_phi_snapshots() {
    let (ir, md, bindings) = outside_reentry_fixture();
    let r = Resolver {
        bad: false,
        unresolved: false,
    };
    let result = super::super::emit_method(&ir, &declaration(), &bindings, &r, &[&md]);
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}")
    };
    assert_eq!(source.matches("try {").count(), 1);
    assert_eq!(source.matches("GroupedContractHarness.first(").count(), 1);
    assert_eq!(source.matches("GroupedContractHarness.second(").count(), 1);
    let root = std::env::temp_dir().join(format!("source-scope-reentry-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let java = format!(
        r#"public class GroupedContractHarness {{
 static String trace="";static int turns;static Throwable seen;
 static final java.io.IOException IO=new java.io.IOException();static final ClassNotFoundException CN=new ClassNotFoundException();static final Error ERROR=new Error();
 static void first(int m)throws java.io.IOException{{trace+="a";if(m==1)throw IO;if(m==3)throw ERROR;}}
 static void second(int m)throws ClassNotFoundException{{trace+="b";if(m==2)throw CN;}}
 static boolean again(){{trace+="z";return ++turns<2;}}
 static void observeCounter(int n){{trace+="p"+n;}}
 static void observeCaught(Throwable t){{seen=t;trace+="x";}}
 {source}
 public static void main(String[]args){{for(int m=0;m<4;m++){{trace="";turns=0;seen=null;try{{int got=generated(m);if(got!=(m==0?0:-m))throw new AssertionError(got);}}catch(Error e){{if(m!=3||e!=ERROR)throw e;}}
 if(!trace.equals(m==0?"ap0bzp1bz":m==1?"ax":m==2?"ap2bx":"a"))throw new AssertionError(trace);
 if(m==1&&seen!=IO||m==2&&seen!=CN||m==3&&seen!=null)throw new AssertionError("identity");}}}}
 }}"#
    );
    std::fs::write(root.join("GroupedContractHarness.java"), java).unwrap();
    let c = std::process::Command::new("javac")
        .arg(root.join("GroupedContractHarness.java"))
        .output()
        .unwrap();
    assert!(c.status.success(), "{}", String::from_utf8_lossy(&c.stderr));
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
