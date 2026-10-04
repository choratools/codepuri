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

struct Context;
impl JavaResolver for Context {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        (r.index() <= 1).then(|| JavaMethod {
            owner: "HandlerFixture".into(),
            name: if r.index() == 0 { "fail" } else { "cleanup" }.into(),
            is_static: true,
            return_type: JavaType::Void,
            parameters: vec![],
            throws: if r.index() == 0 {
                vec!["java.io.IOException".into()]
            } else {
                vec![]
            },
        })
    }
    fn method_access(&self, _: MethodRef) -> Option<u32> {
        Some(8)
    }
    fn field(&self, _: FieldRef) -> Option<JavaField> {
        None
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        match r.index() {
            0 => Some("java.io.IOException".into()),
            1 => Some("java.lang.RuntimeException".into()),
            2 => Some("java.lang.Exception".into()),
            _ => None,
        }
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || matches!((actual, expected), (JavaType::Class(a), JavaType::Class(e)) if ["java.io.IOException", "java.lang.RuntimeException"].contains(&a.as_str()) && ["java.lang.Exception", "java.lang.Throwable", "java.lang.Object"].contains(&e.as_str()))
    }
}
impl ThrowsHierarchy for Context {
    fn exception_kind(&self, name: &str) -> Option<ExceptionKind> {
        match name {
            "java.io.IOException" | "java.lang.Exception" => Some(ExceptionKind::Checked),
            "java.lang.Throwable" => Some(ExceptionKind::Checked),
            "java.lang.RuntimeException" => Some(ExceptionKind::Unchecked),
            _ => None,
        }
    }
}
impl RequiredThrowsContext for Context {
    fn invocation_contract(&self, r: MethodRef) -> Option<InvocationContract> {
        if r.index() == 10 {
            Some(InvocationContract {
                source_target: Some(identity().key),
                declared: vec![],
            })
        } else if r.index() == 0 {
            Some(InvocationContract {
                source_target: None,
                declared: vec!["java.io.IOException".into()],
            })
        } else if r.index() == 1 {
            Some(InvocationContract {
                source_target: None,
                declared: vec![],
            })
        } else {
            None
        }
    }
    fn override_contract(&self, _: &OriginalMethodIdentity) -> OverrideContract {
        OverrideContract::NoInheritedDeclaration
    }
}
impl TypeContext for Context {
    fn method_params(&self, index: u32, _: &mut Vec<ManagedType>) -> bool {
        index <= 1
    }
}
fn identity() -> OriginalMethodIdentity {
    OriginalMethodIdentity {
        dex: 0,
        index: 10,
        key: MethodKey {
            owner: "LHandlerFixture;".into(),
            name: "wrapper".into(),
            parameters: vec![],
            return_type: "V".into(),
        },
    }
}
fn code() -> CodeItem {
    let units = [0x0071u16, 0, 0, 0x000e, 0x000d, 0x0027];
    let span = dex_format::RawSpan {
        offset: 0,
        size: 12,
    };
    CodeItem {
        registers_size: 1,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: 6,
        insns: units.iter().flat_map(|x| x.to_le_bytes()).collect(),
        tries: vec![TryItem {
            start_addr: 0,
            insn_count: 3,
            handler_off: 1,
            span,
        }],
        handlers: vec![EncodedCatchHandler {
            relative_offset: 1,
            handlers: vec![TypeAddrPair {
                type_idx: 0,
                address: 4,
            }],
            catch_all_addr: None,
            span,
        }],
        span,
    }
}
fn thrown(ssa: &SsaMethod, offset: u32) -> ValueId {
    ssa.ir()
        .instructions()
        .iter()
        .find_map(|(id, i)| match i {
            ManagedInsn::Throw { value } if ssa.offset(id) == Some(offset) => Some(*value),
            _ => None,
        })
        .unwrap()
}
fn rebuild(
    ssa: &SsaMethod,
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
        cfg.add_edge(blocks[&e.source()], blocks[&e.target()], e.kind().clone())
            .unwrap();
    }
    let mut instructions = ir.instructions().clone();
    let mut values = ir.values().clone();
    edit(&mut instructions, &mut values, &mut cfg);
    MethodIr::<SsaForm>::from_parts(cfg, instructions, values)
}

#[test]
fn typed_handler_rethrow_derives_original_object_type_without_nominal_seed() {
    let code = code();
    let ssa = DexPasses::run_ssa_with(&code, &Context).unwrap();
    let value = thrown(&ssa, 5);
    assert_eq!(ssa.ir().value(value).unwrap().ty(), &ManagedType::Unknown);
    let proof = HandlerTypes::collect(&ssa, &code, &Context);
    assert_eq!(
        proof.thrown_type(ssa.ir(), value),
        Some("java.io.IOException")
    );
    assert_eq!(proof.definitions.len(), 1);
    let requirements = inspect_method(identity(), &ssa, &code, 8, vec![], &Context).unwrap();
    let plan = derive_program(&[requirements], &Context).unwrap();
    assert!(plan.unavailable.is_empty());
    assert!(plan.methods[&identity().key].original.is_empty());
    assert_eq!(
        plan.methods[&identity().key].derived,
        ["java.io.IOException"]
    );
}

#[test]
fn catch_all_rethrow_uses_actual_call_contract_instead_of_throwable() {
    let mut code = code();
    code.handlers[0].handlers.clear();
    code.handlers[0].catch_all_addr = Some(4);
    let ssa = DexPasses::run_ssa_with(&code, &Context).unwrap();
    let proof = HandlerTypes::collect(&ssa, &code, &Context);
    let value = thrown(&ssa, 5);
    assert_eq!(proof.thrown_type(ssa.ir(), value), None);
    assert_eq!(proof.origins(ssa.ir(), value).unwrap().len(), 1);
    let requirements = inspect_method(identity(), &ssa, &code, 8, vec![], &Context).unwrap();
    let plan = derive_program(&[requirements], &Context).unwrap();
    assert!(plan.unavailable.is_empty());
    assert_eq!(
        plan.methods[&identity().key].derived,
        ["java.io.IOException"]
    );
}

#[test]
fn catch_all_predecessor_identity_survives_empty_cleanup_call_ordering() {
    let mut code = code();
    code.insns = [0x0071u16, 0, 0, 0x000e, 0x000d, 0x0071, 1, 0, 0x0027]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    code.insns_size = 9;
    code.handlers[0].handlers.clear();
    code.handlers[0].catch_all_addr = Some(4);
    let ssa = DexPasses::run_ssa_with(&code, &Context).unwrap();
    let requirements = inspect_method(identity(), &ssa, &code, 8, vec![], &Context).unwrap();
    let plan = derive_program(&[requirements], &Context).unwrap();
    assert!(plan.unavailable.is_empty());
    assert_eq!(
        plan.methods[&identity().key].source,
        ["java.io.IOException"]
    );
}

#[test]
fn catch_all_rethrow_excludes_exceptions_selected_by_previous_typed_catch() {
    let mut code = code();
    code.insns
        .extend([0x000du16, 0x000e].into_iter().flat_map(u16::to_le_bytes));
    code.insns_size = 8;
    code.handlers[0].handlers[0].address = 6;
    code.handlers[0].catch_all_addr = Some(4);
    let ssa = DexPasses::run_ssa_with(&code, &Context).unwrap();
    let proof = HandlerTypes::collect(&ssa, &code, &Context);
    let origin = &proof.origins(ssa.ir(), thrown(&ssa, 5)).unwrap()[0];
    assert_eq!(origin.excluded, ["java.io.IOException"]);
    let requirements = inspect_method(identity(), &ssa, &code, 8, vec![], &Context).unwrap();
    let plan = derive_program(&[requirements], &Context).unwrap();
    assert!(plan.unavailable.is_empty());
    assert!(plan.methods[&identity().key].derived.is_empty());
}

#[test]
fn catch_all_rethrow_propagates_derived_callee_and_preserves_missing_override_failures() {
    use super::super::{Coverage, Event, MethodRequirements, OverrideDeclaration, ThrowsError};
    let mut code = code();
    code.handlers[0].handlers.clear();
    code.handlers[0].catch_all_addr = Some(4);
    let ssa = DexPasses::run_ssa_with(&code, &Context).unwrap();
    let mut caller = inspect_method(identity(), &ssa, &code, 8, vec![], &Context).unwrap();
    let mut callee_identity = identity();
    callee_identity.index = 11;
    callee_identity.key.name = "leaf".into();
    for event in &mut caller.events {
        if let Event::Invoke(contract, _) = event {
            contract.source_target = Some(callee_identity.key.clone());
            contract.declared.clear();
        }
    }
    let absent = derive_program(&[caller.clone()], &Context).unwrap();
    assert!(matches!(
        absent.unavailable[&identity().key],
        ThrowsError::MissingSourceTarget(_)
    ));
    let callee = MethodRequirements {
        identity: callee_identity,
        original: vec![],
        flags: 10,
        overrides: OverrideContract::NoInheritedDeclaration,
        events: vec![Event::Throw(
            "java.io.IOException".into(),
            Coverage {
                catches: vec![],
                all: false,
            },
        )],
    };
    let plan = derive_program(&[caller.clone(), callee.clone()], &Context).unwrap();
    assert!(plan.unavailable.is_empty());
    assert_eq!(
        plan.methods[&identity().key].source,
        ["java.io.IOException"]
    );
    let mut inherited = identity().key;
    inherited.owner = "LParent;".into();
    caller.overrides = OverrideContract::Declarations(vec![OverrideDeclaration {
        key: inherited,
        source_target: None,
        throws: vec![],
    }]);
    let forbidden = derive_program(&[caller, callee], &Context).unwrap();
    assert!(matches!(
        forbidden.unavailable[&identity().key],
        ThrowsError::OverrideConflict(_)
    ));
}

#[test]
fn typed_handler_missing_raw_capture_or_table_is_not_attested() {
    let code = code();
    let ssa = DexPasses::run_ssa_with(&code, &Context).unwrap();
    let mut missing = code.clone();
    missing.insns[8..10].copy_from_slice(&0u16.to_le_bytes());
    assert!(
        HandlerTypes::collect(&ssa, &missing, &Context)
            .captures
            .is_empty()
    );
    missing = code.clone();
    missing.handlers.clear();
    assert!(
        HandlerTypes::collect(&ssa, &missing, &Context)
            .captures
            .is_empty()
    );
    missing = code.clone();
    missing.handlers[0].handlers[0].address = 3;
    assert!(
        HandlerTypes::collect(&ssa, &missing, &Context)
            .captures
            .is_empty()
    );
}

#[test]
fn typed_handler_mixed_catch_identity_and_catch_all_are_not_one_type() {
    let code = code();
    let ssa = DexPasses::run_ssa_with(&code, &Context).unwrap();
    let mut mixed = code.clone();
    mixed.handlers[0].handlers.push(TypeAddrPair {
        type_idx: 1,
        address: 4,
    });
    assert!(
        HandlerTypes::collect(&ssa, &mixed, &Context)
            .captures
            .is_empty()
    );
    mixed = code.clone();
    mixed.handlers[0].catch_all_addr = Some(4);
    assert!(
        HandlerTypes::collect(&ssa, &mixed, &Context)
            .captures
            .is_empty()
    );
}

#[test]
fn typed_handler_alias_or_normal_entry_cannot_attest_a_synthetic_initial() {
    let code = code();
    let ssa = DexPasses::run_ssa_with(&code, &Context).unwrap();
    let valid = HandlerTypes::collect(&ssa, &code, &Context);
    let capture = *valid.captures.keys().next().unwrap();
    let duplicate = rebuild(&ssa, |insns, _, _| {
        insns.insert(ManagedInsn::Opaque {
            family_tag: 0x8002,
            reads: vec![],
            writes: vec![capture],
            effects: EffectFlags::MAY_THROW,
        });
    });
    assert!(
        HandlerTypes::from_ir(&duplicate, &code, |id| ssa.offset(id), &Context)
            .captures
            .is_empty()
    );
    let initial = ssa.ir().value(capture).unwrap().def();
    let normal = rebuild(&ssa, |_, _, cfg| {
        let block = cfg
            .block_ids()
            .find(|b| *cfg.block(*b).unwrap().payload() == initial)
            .unwrap();
        cfg.add_edge(cfg.entry(), block, EdgeKind::Unconditional)
            .unwrap();
    });
    assert!(
        HandlerTypes::from_ir(&normal, &code, |id| ssa.offset(id), &Context)
            .captures
            .is_empty()
    );
}

#[test]
fn typed_handler_copy_and_same_root_phi_prove_identity_but_mixed_phi_does_not() {
    let code = code();
    let ssa = DexPasses::run_ssa_with(&code, &Context).unwrap();
    let proof = HandlerTypes::collect(&ssa, &code, &Context);
    let source = thrown(&ssa, 5);
    let mut copy = None;
    let mut same = None;
    let mut mixed = None;
    let ir = rebuild(&ssa, |insns, values, cfg| {
        let id = insns.insert_with_key(|id| {
            let value = values.insert(Value::new(id, ManagedType::Unknown));
            copy = Some(value);
            ManagedInsn::Move {
                result: value,
                value: source,
            }
        });
        let copied = copy.unwrap();
        assert_eq!(values[copied].def(), id);
        let edges: Vec<_> = cfg.edge_ids().take(2).collect();
        insns.insert_with_key(|id| {
            let value = values.insert(Value::new(id, ManagedType::Unknown));
            same = Some(value);
            ManagedInsn::Phi {
                result: value,
                inputs: vec![(edges[0], source), (edges[1], copied)],
            }
        });
        let mut other = None;
        insns.insert_with_key(|id| {
            let value = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
            other = Some(value);
            ManagedInsn::Opaque {
                family_tag: 0x8001,
                reads: vec![],
                writes: vec![value],
                effects: EffectFlags::from_bits(0),
            }
        });
        insns.insert_with_key(|id| {
            let value = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
            mixed = Some(value);
            ManagedInsn::Phi {
                result: value,
                inputs: vec![(edges[0], source), (edges[1], other.unwrap())],
            }
        });
    });
    assert_eq!(
        proof.thrown_type(&ir, copy.unwrap()),
        Some("java.io.IOException")
    );
    assert_eq!(
        proof.thrown_type(&ir, same.unwrap()),
        Some("java.io.IOException")
    );
    assert_eq!(proof.thrown_type(&ir, mixed.unwrap()), None);
}

#[test]
fn typed_handler_original_samsung_available_590_handler7_rethrow24() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
    if !path.exists() {
        return;
    }
    let files = dex_format::apk::parse_apk_path(&path).unwrap();
    let dex = &files[0];
    let code = dex
        .classes
        .iter()
        .filter_map(|c| c.class_data.as_ref())
        .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
        .find(|m| m.method_idx == 590)
        .unwrap()
        .code
        .as_ref()
        .unwrap();
    let key = dex.method_id(590).unwrap();
    assert_eq!(dex.string(key.name_idx).unwrap(), "available");
    assert_eq!(
        dex.string(
            dex.type_id(u32::from(key.class_idx))
                .unwrap()
                .descriptor_idx
        )
        .unwrap(),
        "Laf/a;"
    );
    let types = dex_decompiler::DexTypes::new(dex, Some(590));
    let ssa = DexPasses::run_ssa_with(code, &types).unwrap();
    struct Original<'a>(&'a dex_format::DexFile);
    impl JavaResolver for Original<'_> {
        fn method(&self, _: MethodRef) -> Option<JavaMethod> {
            None
        }
        fn field(&self, _: FieldRef) -> Option<JavaField> {
            None
        }
        fn class(&self, reference: ClassRef) -> Option<String> {
            let descriptor = self
                .0
                .string(self.0.type_id(reference.index()).ok()?.descriptor_idx)
                .ok()?;
            Some(
                descriptor
                    .strip_prefix('L')?
                    .strip_suffix(';')?
                    .replace('/', "."),
            )
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
    }
    impl ThrowsHierarchy for Original<'_> {
        fn exception_kind(&self, name: &str) -> Option<ExceptionKind> {
            (name == "java.io.IOException").then_some(ExceptionKind::Checked)
        }
    }
    let proof = HandlerTypes::collect(&ssa, code, &Original(dex));
    let value = thrown(&ssa, 24);
    assert_eq!(ssa.ir().value(value).unwrap().ty(), &ManagedType::Unknown);
    assert_eq!(
        proof.thrown_type(ssa.ir(), value),
        Some("java.io.IOException")
    );
    assert_eq!(proof.definitions.len(), 1);
    let capture = *proof.captures.keys().next().unwrap();
    assert!(
        ssa.ir()
            .instructions()
            .iter()
            .any(|(id, i)| ssa.offset(id) == Some(7)
                && matches!(i, ManagedInsn::Move { value, .. } if *value == capture))
    );
}

#[test]
fn typed_handler_generated_java_rethrows_same_checked_object() {
    use backend_java::{
        DeclarationBindings, SourceResult,
        exceptions::{
            RegionMetadata,
            scopes::{Catch, RegionScope},
        },
        type_compatibility::{Declaration, Hierarchy},
    };
    let code = code();
    let ssa = DexPasses::run_ssa_with(&code, &Context).unwrap();
    let requirements = inspect_method(identity(), &ssa, &code, 8, vec![], &Context).unwrap();
    let plan = derive_program(&[requirements], &Context).unwrap();
    let mut hierarchy = Hierarchy::default();
    for (name, parent) in [
        ("java.lang.Object", None),
        ("java.lang.Throwable", Some("java.lang.Object")),
        ("java.lang.Exception", Some("java.lang.Throwable")),
        ("java.io.IOException", Some("java.lang.Exception")),
    ] {
        hierarchy
            .insert(Declaration {
                name: name.into(),
                superclass: parent.map(str::to_owned),
                interfaces: vec![],
                is_interface: false,
            })
            .unwrap();
    }
    let proof = HandlerTypes::collect(&ssa, &code, &Context);
    let initial = *proof.definitions.iter().next().unwrap();
    let block = |offset| {
        ssa.ir()
            .cfg()
            .block_ids()
            .find(|b| ssa.offset(*ssa.ir().cfg().block(*b).unwrap().payload()) == Some(offset))
            .unwrap()
    };
    let handler = ssa
        .ir()
        .cfg()
        .block_ids()
        .find(|b| *ssa.ir().cfg().block(*b).unwrap().payload() == initial)
        .unwrap();
    struct Metadata {
        scope: RegionScope,
        hierarchy: Hierarchy,
    }
    impl RegionMetadata for Metadata {
        fn region_scope(&self) -> &RegionScope {
            &self.scope
        }
        fn catch_type(&self, key: &str) -> Option<JavaType> {
            (key == "0").then(|| JavaType::Class("java.io.IOException".into()))
        }
        fn hierarchy(&self) -> &Hierarchy {
            &self.hierarchy
        }
        fn is_catch_all(&self, _: &str) -> bool {
            false
        }
    }
    let metadata = Metadata {
        scope: RegionScope {
            protected: vec![block(0)],
            catches: vec![Catch {
                key: "0".into(),
                entry: handler,
            }],
            continuation: Some(block(3)),
        },
        hierarchy,
    };
    let method = JavaMethod {
        owner: "HandlerFixture".into(),
        name: "wrapper".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![],
        throws: plan.methods[&identity().key].source.clone(),
    };
    let result = backend_java::exceptions::emit_method_region(
        ssa.ir(),
        &method,
        &DeclarationBindings::default(),
        &Context,
        &metadata,
    );
    let SourceResult::Complete { source, .. } = result else {
        panic!("{result:?}");
    };
    let source = format!(
        "public class HandlerFixture {{ static final java.io.IOException original = new java.io.IOException(\"original\"); public static void fail() throws java.io.IOException {{ throw original; }} {source} public static void main(String[] a) {{ try {{ wrapper(); throw new AssertionError(); }} catch (java.io.IOException e) {{ if(e != original) throw new AssertionError(); System.out.print(\"IO:identity\"); }} }} }}"
    );
    let dir = std::env::temp_dir().join(format!(
        "decomphify-handler-provenance-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("HandlerFixture.java"), &source).unwrap();
    let jdk =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let compiled = std::process::Command::new(format!("{jdk}/bin/javac"))
        .args(["--release", "17", "HandlerFixture.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let run = std::process::Command::new(format!("{jdk}/bin/java"))
        .args(["-cp", ".", "HandlerFixture"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "IO:identity");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "bounded original handler CFG diagnostic"]
fn original_catch_all_capture_topology() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
    let files = dex_format::apk::parse_apk_path(path).unwrap();
    let dex = &files[0];
    for class in &dex.classes {
        let owner = dex
            .string(dex.type_id(class.class_idx).unwrap().descriptor_idx)
            .unwrap();
        if owner != "Landroidx/fragment/app/z0;" {
            continue;
        }
        for method in class.class_data.as_ref().unwrap().direct_methods.iter() {
            let raw = dex.method_id(method.method_idx).unwrap();
            if dex.string(raw.name_idx).unwrap() != "values" {
                continue;
            }
            let code = method.code.as_ref().unwrap();
            let ssa = DexPasses::run_ssa_with(
                code,
                &dex_decompiler::DexTypes::new(dex, Some(method.method_idx)),
            )
            .unwrap();
            for (id, insn) in ssa.ir().instructions() {
                if let ManagedInsn::Invoke {
                    method: target,
                    args,
                    ..
                } = insn
                {
                    let target = dex.method_id(target.index()).unwrap();
                    let owner = dex
                        .string(
                            dex.type_id(u32::from(target.class_idx))
                                .unwrap()
                                .descriptor_idx,
                        )
                        .unwrap();
                    let name = dex.string(target.name_idx).unwrap();
                    println!(
                        "ENUM VALUES {} {:?} invoke {owner}->{name}, args {:?}",
                        method.method_idx,
                        ssa.offset(id),
                        args.iter()
                            .map(|v| ssa.ir().value(*v).unwrap().ty())
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
    }
    for index in [55, 122] {
        let code = dex
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
            .find(|m| m.method_idx == index)
            .unwrap()
            .code
            .as_ref()
            .unwrap();
        let ssa = DexPasses::run_ssa_with(code, &dex_decompiler::DexTypes::new(dex, Some(index)))
            .unwrap();
        for (id, i) in ssa.ir().instructions() {
            if let ManagedInsn::Opaque {
                family_tag: 0x8002,
                writes,
                ..
            } = i
            {
                println!("CAPTURE {index} {id:?} {i:?}");
                for (reader, insn) in ssa
                    .ir()
                    .instructions()
                    .iter()
                    .filter(|(_, i)| reads(i, writes[0]))
                {
                    println!("READER {:?} {:?} {insn:?}", reader, ssa.offset(reader));
                }
                for block in ssa
                    .ir()
                    .cfg()
                    .block_ids()
                    .filter(|b| *ssa.ir().cfg().block(*b).unwrap().payload() == id)
                {
                    let node = ssa.ir().cfg().block(block).unwrap();
                    for edge in node.incoming().iter().chain(node.outgoing()) {
                        let edge = ssa.ir().cfg().edge(*edge).unwrap();
                        let source = *ssa.ir().cfg().block(edge.source()).unwrap().payload();
                        let target = *ssa.ir().cfg().block(edge.target()).unwrap().payload();
                        println!(
                            "EDGE {:?} from {:?} {:?} to {:?} {:?}",
                            edge.kind(),
                            source,
                            ssa.offset(source),
                            target,
                            ssa.offset(target)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn mixed_nominal_capture_and_invocation_are_not_precise_rethrows() {
    struct Returns(bool);
    impl JavaResolver for Returns {
        fn method(&self, _: MethodRef) -> Option<JavaMethod> {
            self.0.then(|| JavaMethod {
                owner: "Fixture".into(),
                name: "wrap".into(),
                is_static: true,
                return_type: JavaType::Class("java.io.IOException".into()),
                parameters: vec![],
                throws: vec![],
            })
        }
        fn field(&self, _: FieldRef) -> Option<JavaField> {
            None
        }
        fn class(&self, r: ClassRef) -> Option<String> {
            Context.class(r)
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
    }
    impl ThrowsHierarchy for Returns {
        fn exception_kind(&self, name: &str) -> Option<ExceptionKind> {
            if name == "java.lang.NullPointerException" {
                Some(ExceptionKind::Unchecked)
            } else {
                Context.exception_kind(name)
            }
        }
    }
    let code = code();
    let ssa = DexPasses::run_ssa_with(&code, &Context).unwrap();
    let proof = HandlerTypes::collect(&ssa, &code, &Context);
    let captured = thrown(&ssa, 5);
    let mut mixed = None;
    let mut opaque = None;
    let mut null_join = None;
    let mut cycle = None;
    let ir = rebuild(&ssa, |insns, values, cfg| {
        let edges: Vec<_> = cfg.edge_ids().take(2).collect();
        let mut returned = None;
        insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ManagedType::Unknown));
            returned = Some(v);
            ManagedInsn::Invoke {
                kind: managed_ir::ops::InvokeKind::Static,
                result: Some(v),
                method: MethodRef::new(2),
                args: vec![],
            }
        });
        insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ManagedType::Unknown));
            mixed = Some(v);
            ManagedInsn::Phi {
                result: v,
                inputs: vec![(edges[0], captured), (edges[1], returned.unwrap())],
            }
        });
        insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
            opaque = Some(v);
            ManagedInsn::Opaque {
                family_tag: 1,
                reads: vec![],
                writes: vec![v],
                effects: EffectFlags::from_bits(0),
            }
        });
        let mut zero = None;
        insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ManagedType::Unknown));
            zero = Some(v);
            ManagedInsn::Const {
                result: v,
                value: managed_ir::insn::Constant::Zero,
            }
        });
        insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ManagedType::Unknown));
            null_join = Some(v);
            ManagedInsn::Phi {
                result: v,
                inputs: vec![(edges[0], mixed.unwrap()), (edges[1], zero.unwrap())],
            }
        });
        insns.insert_with_key(|id| {
            let v = values.insert(Value::new(id, ManagedType::Unknown));
            cycle = Some(v);
            ManagedInsn::Phi {
                result: v,
                inputs: vec![(edges[0], captured), (edges[1], v)],
            }
        });
    });
    let value = mixed.unwrap();
    assert!(proof.origins(&ir, value).is_none());
    assert!(proof.thrown_type(&ir, value).is_none());
    assert_eq!(
        proof.nominal_thrown_types(&ir, value, &Returns(true)),
        Some(BTreeSet::from(["java.io.IOException".into()]))
    );
    assert!(
        proof
            .nominal_thrown_types(&ir, value, &Returns(false))
            .is_none()
    );
    assert!(
        proof
            .nominal_thrown_types(&ir, opaque.unwrap(), &Returns(true))
            .is_none()
    );
    assert!(
        proof
            .nominal_thrown_types(&ir, cycle.unwrap(), &Returns(true))
            .is_none()
    );
    assert_eq!(
        proof.nominal_thrown_types(&ir, null_join.unwrap(), &Returns(true)),
        Some(BTreeSet::from([
            "java.io.IOException".into(),
            "java.lang.NullPointerException".into()
        ]))
    );
}

#[test]
#[ignore = "requires retained original Samsung Pass APK"]
fn mixed_nominal_original_samsung_310_and_42568() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk");
    let files = dex_format::apk::parse_apk_path(&path).unwrap();
    let dex = &files[3];
    struct Original<'a>(&'a dex_format::DexFile);
    impl JavaResolver for Original<'_> {
        fn class(&self, r: ClassRef) -> Option<String> {
            let descriptor = self
                .0
                .string(self.0.type_id(r.index()).ok()?.descriptor_idx)
                .ok()?;
            Some(
                descriptor
                    .strip_prefix('L')?
                    .strip_suffix(';')?
                    .replace('/', "."),
            )
        }
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            let raw = self.0.method_id(r.index()).ok()?;
            let proto = self.0.proto_id(u32::from(raw.proto_idx)).ok()?;
            Some(JavaMethod {
                owner: self.class(ClassRef::new(u32::from(raw.class_idx)))?,
                name: self.0.string(raw.name_idx).ok()?.into(),
                is_static: false,
                return_type: JavaType::Class(self.class(ClassRef::new(proto.return_type_idx))?),
                parameters: vec![],
                throws: vec![],
            })
        }
        fn field(&self, _: FieldRef) -> Option<JavaField> {
            None
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
    }
    impl ThrowsHierarchy for Original<'_> {
        fn exception_kind(&self, name: &str) -> Option<ExceptionKind> {
            match name {
                "java.io.IOException" | "java.lang.Throwable" => Some(ExceptionKind::Checked),
                "java.lang.NullPointerException" => Some(ExceptionKind::Unchecked),
                "wu.n" => {
                    let class = self.0.classes.iter().find(|c| {
                        self.class(ClassRef::new(c.class_idx)).as_deref() == Some(name)
                    })?;
                    (self.class(ClassRef::new(class.superclass_idx)).as_deref()
                        == Some("java.io.IOException"))
                    .then_some(ExceptionKind::Checked)
                }
                _ => None,
            }
        }
    }
    let resolver = Original(dex);
    for (index, offset, expected) in [
        (310, 43, BTreeSet::from(["java.io.IOException".to_owned()])),
        (
            42568,
            184,
            BTreeSet::from([
                "java.io.IOException".to_owned(),
                "wu.n".to_owned(),
                "java.lang.NullPointerException".to_owned(),
            ]),
        ),
    ] {
        let code = dex
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
            .find(|m| m.method_idx == index)
            .unwrap()
            .code
            .as_ref()
            .unwrap();
        let ssa = DexPasses::run_ssa_with(code, &dex_decompiler::DexTypes::new(dex, Some(index)))
            .unwrap();
        let value = thrown(&ssa, offset);
        let proof = HandlerTypes::collect(&ssa, code, &resolver);
        assert!(proof.origins(ssa.ir(), value).is_none());
        assert_eq!(
            proof.nominal_thrown_types(ssa.ir(), value, &resolver),
            Some(expected),
            "method {index}"
        );
    }
}

#[test]
fn mixed_nominal_jvm_domain_preserves_selected_exception_identity() {
    let source = r#"public class MixedDomain {
      static final java.io.IOException captured = new java.io.IOException("caught");
      static final java.io.IOException replacement = new java.io.IOException("replacement");
      static java.io.IOException replace(java.io.IOException e) { return replacement; }
      static void select(int path) throws java.io.IOException {
        java.io.IOException selected;
        try { throw captured; }
        catch (java.io.IOException e) { selected = path == 0 ? e : path == 1 ? replace(e) : null; }
        throw selected;
      }
      public static void main(String[] args) {
        for (int path = 0; path < 3; path++) {
          try { select(path); throw new AssertionError(); }
          catch (java.io.IOException e) { if (e != (path == 0 ? captured : replacement)) throw new AssertionError(); }
          catch (NullPointerException e) { if (path != 2) throw new AssertionError(); }
        }
        System.out.print("caught:replacement:null");
      }
    }"#;
    let dir = std::env::temp_dir().join(format!("decomphify-mixed-domain-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("MixedDomain.java"), source).unwrap();
    let jdk =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let compiled = std::process::Command::new(format!("{jdk}/bin/javac"))
        .args(["--release", "17", "MixedDomain.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let run = std::process::Command::new(format!("{jdk}/bin/java"))
        .args(["-cp", ".", "MixedDomain"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "caught:replacement:null"
    );
}

#[test]
fn broad_typed_capture_rethrow_retains_precise_checked_predecessor() {
    let mut code = code();
    code.handlers[0].handlers[0].type_idx = 2;
    for (reference, expected) in [(0u16, vec!["java.io.IOException".to_owned()]), (1, vec![])] {
        code.insns[2..4].copy_from_slice(&reference.to_le_bytes());
        let ssa = DexPasses::run_ssa_with(&code, &Context).unwrap();
        let value = thrown(&ssa, 5);
        let proof = HandlerTypes::collect(&ssa, &code, &Context);
        assert_eq!(
            proof.thrown_type(ssa.ir(), value),
            Some("java.lang.Exception")
        );
        assert!(proof.origins(ssa.ir(), value).is_some());
        let requirements = inspect_method(identity(), &ssa, &code, 8, vec![], &Context).unwrap();
        let plan = derive_program(&[requirements], &Context).unwrap();
        assert!(plan.unavailable.is_empty());
        assert_eq!(plan.methods[&identity().key].derived, expected);
    }
}

#[path = "java_required_throws_nominal_catchall_tests.rs"]
mod nominal_catchall_tests;
