#![allow(clippy::unwrap_used)]
use super::super::{
    ExceptionKind, InvocationContract, MethodKey, OriginalMethodIdentity, OverrideContract,
    RequiredThrowsContext, derive_program, inspect_method,
};
use super::*;
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType};
use decomp_core::cfg::Cfg;
use dex_decompiler::{DexPasses, TypeContext};
use dex_format::code::{EncodedCatchHandler, TryItem, TypeAddrPair};
use managed_ir::{
    method::{InsnArena, ValueArena},
    refs::{FieldRef, MethodRef, StringRef},
    types::ManagedType,
    value::Value,
};

struct Context {
    declared: Vec<String>,
    missing: Option<u32>,
}
impl Context {
    fn checked_pair() -> Self {
        Self {
            declared: vec!["java.io.IOException".into(), "java.sql.SQLException".into()],
            missing: None,
        }
    }
}
impl JavaResolver for Context {
    fn class(&self, r: ClassRef) -> Option<String> {
        if self.missing == Some(r.index()) {
            return None;
        }
        match r.index() {
            0 => Some("java.io.IOException".into()),
            1 => Some("java.sql.SQLException".into()),
            2 => Some("java.lang.SecurityException".into()),
            3 => Some("java.io.FileNotFoundException".into()),
            _ => None,
        }
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        (r.index() == 0).then(|| JavaMethod {
            owner: "SharedHandlerFixture".into(),
            name: "leaf".into(),
            is_static: true,
            parameters: vec![],
            return_type: JavaType::Void,
            throws: self.declared.clone(),
        })
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(8)
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, a: &JavaType, e: &JavaType) -> bool {
        if a == e {
            return true;
        }
        let (JavaType::Class(a), JavaType::Class(e)) = (a, e) else {
            return false;
        };
        (a == "java.io.FileNotFoundException" && e == "java.io.IOException")
            || ([
                "java.io.IOException",
                "java.io.FileNotFoundException",
                "java.sql.SQLException",
                "java.lang.SecurityException",
            ]
            .contains(&a.as_str())
                && [
                    "java.lang.Exception",
                    "java.lang.Throwable",
                    "java.lang.Object",
                ]
                .contains(&e.as_str()))
    }
}
impl ThrowsHierarchy for Context {
    fn exception_kind(&self, n: &str) -> Option<ExceptionKind> {
        match n {
            "java.io.IOException"
            | "java.io.FileNotFoundException"
            | "java.sql.SQLException"
            | "java.lang.Throwable" => Some(ExceptionKind::Checked),
            "java.lang.SecurityException" => Some(ExceptionKind::Unchecked),
            _ => None,
        }
    }
}
impl RequiredThrowsContext for Context {
    fn invocation_contract(&self, r: MethodRef) -> Option<InvocationContract> {
        match r.index() {
            0 => Some(InvocationContract {
                source_target: None,
                declared: self.declared.clone(),
            }),
            10 => Some(InvocationContract {
                source_target: Some(identity().key),
                declared: vec![],
            }),
            _ => None,
        }
    }
    fn override_contract(&self, _: &OriginalMethodIdentity) -> OverrideContract {
        OverrideContract::NoInheritedDeclaration
    }
}
impl TypeContext for Context {
    fn method_params(&self, index: u32, _: &mut Vec<ManagedType>) -> bool {
        index == 0
    }
}
fn identity() -> OriginalMethodIdentity {
    OriginalMethodIdentity {
        dex: 0,
        index: 10,
        key: MethodKey {
            owner: "LSharedHandlerFixture;".into(),
            name: "wrapper".into(),
            parameters: vec![],
            return_type: "V".into(),
        },
    }
}
fn code() -> CodeItem {
    let span = dex_format::RawSpan {
        offset: 0,
        size: 16,
    };
    CodeItem {
        registers_size: 1,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: 8,
        insns: [0x0071u16, 0, 0, 0x000e, 0x000d, 0x0027, 0x000d, 0x000e]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect(),
        tries: vec![TryItem {
            start_addr: 0,
            insn_count: 3,
            handler_off: 1,
            span,
        }],
        handlers: vec![EncodedCatchHandler {
            relative_offset: 1,
            handlers: vec![
                TypeAddrPair {
                    type_idx: 0,
                    address: 4,
                },
                TypeAddrPair {
                    type_idx: 1,
                    address: 4,
                },
            ],
            catch_all_addr: None,
            span,
        }],
        span,
    }
}
fn thrown(ssa: &SsaMethod) -> ValueId {
    ssa.ir()
        .instructions()
        .iter()
        .find_map(|(_, i)| match i {
            ManagedInsn::Throw { value } => Some(*value),
            _ => None,
        })
        .unwrap()
}
fn rebuild(
    ssa: &SsaMethod,
    keep: impl Fn(&EdgeKind) -> bool,
    edit: impl FnOnce(&mut InsnArena, &mut ValueArena, &mut Cfg<InsnId>),
) -> MethodIr<SsaForm> {
    let ir = ssa.ir();
    let mut cfg = Cfg::new(*ir.cfg().block(ir.cfg().entry()).unwrap().payload());
    let mut blocks = BTreeMap::from([(ir.cfg().entry(), cfg.entry())]);
    for b in ir.cfg().block_ids().filter(|b| *b != ir.cfg().entry()) {
        blocks.insert(b, cfg.add_block(*ir.cfg().block(b).unwrap().payload()));
    }
    for id in ir.cfg().edge_ids() {
        let e = ir.cfg().edge(id).unwrap();
        if keep(e.kind()) {
            cfg.add_edge(blocks[&e.source()], blocks[&e.target()], e.kind().clone())
                .unwrap();
        }
    }
    let mut insns = ir.instructions().clone();
    let mut values = ir.values().clone();
    edit(&mut insns, &mut values, &mut cfg);
    MethodIr::<SsaForm>::from_parts(cfg, insns, values)
}
#[test]
fn shared_handler_checked_union_uses_exact_origins_without_one_nominal_type() {
    let code = code();
    let context = Context::checked_pair();
    let ssa = DexPasses::run_ssa_with(&code, &context).unwrap();
    let proof = HandlerTypes::collect(&ssa, &code, &context);
    let value = thrown(&ssa);
    assert_eq!(proof.thrown_type(ssa.ir(), value), None);
    let origins = proof.origins(ssa.ir(), value).unwrap();
    assert_eq!(origins.len(), 2);
    assert!(
        origins
            .iter()
            .any(|o| o.caught.as_deref() == Some("java.sql.SQLException")
                && o.excluded == ["java.io.IOException"])
    );
    assert_eq!(proof.definitions.len(), 1);
    let requirements = inspect_method(identity(), &ssa, &code, 8, vec![], &context).unwrap();
    let plan = derive_program(&[requirements], &context).unwrap();
    assert!(plan.unavailable.is_empty());
    assert_eq!(
        plan.methods[&identity().key].source,
        ["java.io.IOException", "java.sql.SQLException"]
    );
}
#[test]
fn shared_handler_runtime_catch_preserves_only_actual_checked_rethrow_domain() {
    let mut code = code();
    code.handlers[0].handlers[1].type_idx = 2;
    let context = Context {
        declared: vec![
            "java.io.IOException".into(),
            "java.lang.SecurityException".into(),
        ],
        missing: None,
    };
    let ssa = DexPasses::run_ssa_with(&code, &context).unwrap();
    let proof = HandlerTypes::collect(&ssa, &code, &context);
    assert!(proof.thrown_type(ssa.ir(), thrown(&ssa)).is_none());
    let requirements = inspect_method(identity(), &ssa, &code, 8, vec![], &context).unwrap();
    let plan = derive_program(&[requirements], &context).unwrap();
    assert_eq!(
        plan.methods[&identity().key].source,
        ["java.io.IOException"]
    );
}
#[test]
fn shared_handler_prior_catch_filters_shadowed_subtype_and_order_changes_domain() {
    let mut code = code();
    code.handlers[0].handlers = vec![
        TypeAddrPair {
            type_idx: 0,
            address: 6,
        },
        TypeAddrPair {
            type_idx: 3,
            address: 4,
        },
        TypeAddrPair {
            type_idx: 1,
            address: 4,
        },
    ];
    let context = Context {
        declared: vec![
            "java.io.FileNotFoundException".into(),
            "java.sql.SQLException".into(),
        ],
        missing: None,
    };
    let ssa = DexPasses::run_ssa_with(&code, &context).unwrap();
    let proof = HandlerTypes::collect(&ssa, &code, &context);
    assert!(
        proof
            .origins(ssa.ir(), thrown(&ssa))
            .unwrap()
            .iter()
            .any(
                |o| o.caught.as_deref() == Some("java.io.FileNotFoundException")
                    && o.excluded == ["java.io.IOException"]
            )
    );
    let r = inspect_method(identity(), &ssa, &code, 8, vec![], &context).unwrap();
    let plan = derive_program(&[r], &context).unwrap();
    assert_eq!(
        plan.methods[&identity().key].source,
        ["java.sql.SQLException"]
    );
    code.handlers[0].handlers.swap(0, 1);
    let ssa = DexPasses::run_ssa_with(&code, &context).unwrap();
    let r = inspect_method(identity(), &ssa, &code, 8, vec![], &context).unwrap();
    let plan = derive_program(&[r], &context).unwrap();
    assert_eq!(
        plan.methods[&identity().key].source,
        ["java.io.FileNotFoundException", "java.sql.SQLException"]
    );
}
#[test]
fn shared_handler_missing_partial_or_foreign_cfg_and_capture_alias_remain_denied() {
    let code = code();
    let context = Context::checked_pair();
    let ssa = DexPasses::run_ssa_with(&code, &context).unwrap();
    let partial = rebuild(
        &ssa,
        |k| k != &EdgeKind::Exception("1".into()),
        |_, _, _| {},
    );
    assert!(
        HandlerTypes::from_ir(&partial, &code, |id| ssa.offset(id), &context)
            .definitions
            .is_empty()
    );
    let proof = HandlerTypes::collect(&ssa, &code, &context);
    let capture = *proof.origins.keys().next().unwrap();
    let definition = ssa.ir().value(capture).unwrap().def();
    let normal = rebuild(
        &ssa,
        |_| true,
        |_, _, cfg| {
            let target = cfg
                .block_ids()
                .find(|b| cfg.block(*b).unwrap().payload() == &definition)
                .unwrap();
            cfg.add_edge(cfg.entry(), target, EdgeKind::Unconditional)
                .unwrap();
        },
    );
    assert!(
        HandlerTypes::from_ir(&normal, &code, |id| ssa.offset(id), &context)
            .definitions
            .is_empty()
    );
    let foreign = rebuild(
        &ssa,
        |_| true,
        |_, _, cfg| {
            let target = cfg
                .block_ids()
                .find(|b| cfg.block(*b).unwrap().payload() == &definition)
                .unwrap();
            cfg.add_edge(cfg.entry(), target, EdgeKind::Exception("999".into()))
                .unwrap();
        },
    );
    assert!(
        HandlerTypes::from_ir(&foreign, &code, |id| ssa.offset(id), &context)
            .definitions
            .is_empty()
    );
    let alias = rebuild(
        &ssa,
        |_| true,
        |insns, _, _| {
            insns.insert(ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads: vec![],
                writes: vec![capture],
                effects: EffectFlags::MAY_THROW,
            });
        },
    );
    assert!(
        HandlerTypes::from_ir(&alias, &code, |id| ssa.offset(id), &context)
            .definitions
            .is_empty()
    );
    let missing = Context {
        missing: Some(1),
        ..Context::checked_pair()
    };
    assert!(
        HandlerTypes::collect(&ssa, &code, &missing)
            .definitions
            .is_empty()
    );
    let mut bad = code.clone();
    bad.handlers.clear();
    assert!(
        HandlerTypes::collect(&ssa, &bad, &context)
            .definitions
            .is_empty()
    );
    bad = code.clone();
    bad.handlers.push(bad.handlers[0].clone());
    assert!(
        HandlerTypes::collect(&ssa, &bad, &context)
            .definitions
            .is_empty()
    );
    bad = code.clone();
    let duplicate_type = bad.handlers[0].handlers[0].clone();
    bad.handlers[0].handlers.push(duplicate_type);
    assert!(
        HandlerTypes::collect(&ssa, &bad, &context)
            .definitions
            .is_empty()
    );
}
#[test]
fn shared_handler_copy_phi_preserves_origin_union_but_unknown_input_does_not() {
    let code = code();
    let context = Context::checked_pair();
    let ssa = DexPasses::run_ssa_with(&code, &context).unwrap();
    let proof = HandlerTypes::collect(&ssa, &code, &context);
    let source = thrown(&ssa);
    let mut copied = None;
    let mut same = None;
    let mut mixed = None;
    let ir = rebuild(
        &ssa,
        |_| true,
        |insns, values, cfg| {
            insns.insert_with_key(|id| {
                let v = values.insert(Value::new(id, ManagedType::Unknown));
                copied = Some(v);
                ManagedInsn::Move {
                    result: v,
                    value: source,
                }
            });
            let edges: Vec<_> = cfg.edge_ids().take(2).collect();
            insns.insert_with_key(|id| {
                let v = values.insert(Value::new(id, ManagedType::Unknown));
                same = Some(v);
                ManagedInsn::Phi {
                    result: v,
                    inputs: vec![(edges[0], source), (edges[1], copied.unwrap())],
                }
            });
            let mut unknown = None;
            insns.insert_with_key(|id| {
                let v = values.insert(Value::new(id, ManagedType::Unknown));
                unknown = Some(v);
                ManagedInsn::Opaque {
                    family_tag: 0x8001,
                    reads: vec![],
                    writes: vec![v],
                    effects: EffectFlags::NONE,
                }
            });
            insns.insert_with_key(|id| {
                let v = values.insert(Value::new(id, ManagedType::Unknown));
                mixed = Some(v);
                ManagedInsn::Phi {
                    result: v,
                    inputs: vec![(edges[0], source), (edges[1], unknown.unwrap())],
                }
            });
        },
    );
    assert_eq!(proof.origins(&ir, copied.unwrap()).unwrap().len(), 2);
    assert_eq!(proof.origins(&ir, same.unwrap()).unwrap().len(), 2);
    assert!(proof.thrown_type(&ir, same.unwrap()).is_none());
    assert!(proof.origins(&ir, mixed.unwrap()).is_none());
}
fn java_bin() -> std::path::PathBuf {
    let mut candidates = Vec::new();
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        candidates.push(std::path::PathBuf::from(home).join("bin"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path));
    }
    let mut installations = vec![std::path::PathBuf::from("/usr/lib/jvm")];
    if let Some(home) = std::env::var_os("HOME") {
        installations.push(std::path::PathBuf::from(home).join("tools"));
    }
    for base in installations {
        candidates.push(base.clone());
        let mut directories: Vec<_> = std::fs::read_dir(base)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("bin"))
            .collect();
        directories.sort();
        candidates.extend(directories);
    }
    let executable = |name: &str| format!("{name}{}", std::env::consts::EXE_SUFFIX);
    candidates
        .into_iter()
        .find(|bin| {
            bin.join(executable("javac")).is_file() && bin.join(executable("java")).is_file()
        })
        .unwrap_or_else(|| panic!("Java oracle requires JDK17+; configure JAVA_HOME or PATH"))
}
#[test]
fn shared_handler_java_checked_union_priority_and_object_identity() {
    use std::process::Command;
    let bin = java_bin();
    let dir =
        std::env::temp_dir().join(format!("decomphify-shared-handler-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let java = dir.join("SharedHandlerOracle.java");
    std::fs::write(&java,r#"public class SharedHandlerOracle {
static final java.io.IOException IO=new java.io.IOException("io");
static final java.sql.SQLException SQL=new java.sql.SQLException("sql");
static final java.lang.SecurityException SEC=new java.lang.SecurityException("security");
static final java.io.FileNotFoundException FNF=new java.io.FileNotFoundException("fnf");
static void leaf(int mode) throws java.io.IOException,java.sql.SQLException {if(mode==0)throw IO;if(mode==1)throw SQL;if(mode==2)throw SEC;throw FNF;}
static void shared(int mode) throws java.io.IOException,java.sql.SQLException {try{leaf(mode);}catch(java.io.IOException|java.sql.SQLException|java.lang.SecurityException e){throw e;}}
static void prioritized(int mode) throws java.sql.SQLException {try{leaf(mode);}catch(java.io.IOException e){}catch(java.sql.SQLException e){throw e;}}
public static void main(String[] args) throws Exception {
 for(int mode=0;mode<3;mode++){try{shared(mode);throw new AssertionError("lost throw");}catch(Exception e){Object expected=mode==0?IO:mode==1?SQL:SEC;if(e!=expected)throw new AssertionError("changed object");}}
 prioritized(3);try{prioritized(1);throw new AssertionError("lost sql");}catch(java.sql.SQLException e){if(e!=SQL)throw new AssertionError("changed sql");}
}}
"#).unwrap();
    let compiled = Command::new(bin.join(format!("javac{}", std::env::consts::EXE_SUFFIX)))
        .args(["--release", "17", "-d"])
        .arg(&dir)
        .arg(&java)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let ran = Command::new(bin.join(format!("java{}", std::env::consts::EXE_SUFFIX)))
        .arg("-cp")
        .arg(&dir)
        .arg("SharedHandlerOracle")
        .output()
        .unwrap();
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let invalid = dir.join("SharedHandlerInvalid.java");
    std::fs::write(&invalid,"class SharedHandlerInvalid { static void leaf() throws java.io.IOException,java.sql.SQLException {} static void caller() throws java.sql.SQLException { try { leaf(); } catch(java.io.IOException|java.sql.SQLException e) { throw e; } } }").unwrap();
    assert!(
        !Command::new(bin.join(format!("javac{}", std::env::consts::EXE_SUFFIX)))
            .arg(&invalid)
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
