//! Standalone member adapter tests; the APK is opened read-only.
// Fixture construction and successful-result assertions intentionally panic on failure.
#![allow(clippy::unwrap_used, clippy::expect_used)]
pub use decomp::dex_java;
use decomp::java_members;

use backend_java::{JavaResolver, JavaType, SourceResult};
use java_members::{MemberRequirements, MemberResolver, requirements};
use managed_ir::refs::{FieldRef, MethodRef};
use std::collections::BTreeSet;

#[test]
fn closure_requires_actual_bodies_and_fields_and_allows_cycles() {
    let requirements = MemberRequirements {
        fields: BTreeSet::from([3]),
        methods: BTreeSet::from([7, 8]),
    };
    assert!(
        requirements
            .check_closure(&BTreeSet::from([7]), &BTreeSet::from([3]))
            .is_err()
    );
    assert!(
        requirements
            .check_closure(&BTreeSet::from([7, 8]), &BTreeSet::new())
            .is_err()
    );
    requirements
        .check_closure(&BTreeSet::from([7, 8]), &BTreeSet::from([3]))
        .unwrap();
    assert_eq!(
        java_members::type_source(&JavaType::Array(Box::new(JavaType::Long))),
        "long[]"
    );
}

#[test]
fn genuine_pool_metadata_and_conservative_fields() {
    let mut dex = dex_format::DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let (class_index, field_index) = dex
        .classes
        .iter()
        .enumerate()
        .find_map(|(i, c)| {
            c.class_data
                .as_ref()?
                .instance_fields
                .first()
                .map(|f| (i, f.field_idx))
        })
        .expect("fixture instance field");
    let resolver = MemberResolver { dex: &dex };
    let field = resolver.field(FieldRef::new(field_index)).unwrap();
    assert_eq!(
        field.name,
        dex.string(dex.field_id(field_index).unwrap().name_idx)
            .unwrap()
    );
    assert!(!field.is_static);
    assert!(resolver.method(MethodRef::new(u32::MAX)).is_none());
    assert!(resolver.field(FieldRef::new(u32::MAX)).is_none());
    // Mutate parsed test metadata only: final and inconsistent storage must fail.
    let encoded = &mut dex.classes[class_index]
        .class_data
        .as_mut()
        .unwrap()
        .instance_fields[0];
    encoded.access_flags = 1;
    assert!(
        MemberResolver { dex: &dex }
            .field_declaration(field_index)
            .unwrap()
            .starts_with("public ")
    );
    dex.classes[class_index]
        .class_data
        .as_mut()
        .unwrap()
        .instance_fields[0]
        .access_flags = 0x11;
    assert!(
        MemberResolver { dex: &dex }
            .field_declaration(field_index)
            .is_err()
    );
    dex.classes[class_index]
        .class_data
        .as_mut()
        .unwrap()
        .instance_fields[0]
        .access_flags = 8;
    assert!(
        MemberResolver { dex: &dex }
            .field(FieldRef::new(field_index))
            .is_none()
    );
}

#[test]
fn okcashbag_real_selected_member_body_emits() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk");
    // Read one pool only; parsing every pool in this large APK is unnecessary.
    let output = std::process::Command::new("unzip")
        .arg("-p")
        .arg(path)
        .arg("classes.dex")
        .output()
        .expect("unzip available for read-only fixture");
    assert!(output.status.success());
    let files = [dex_format::DexFile::from_bytes(&output.stdout).unwrap()];
    for dex in &files {
        let resolver = MemberResolver { dex };
        for class in &dex.classes {
            if resolver
                .class(managed_ir::refs::ClassRef::new(class.class_idx))
                .as_deref()
                != Some("android.support.customtabs.ICustomTabsCallback$Stub$Proxy")
            {
                continue;
            }
            let Some(data) = &class.class_data else {
                continue;
            };
            for method in data.direct_methods.iter().chain(&data.virtual_methods) {
                let Some(code) = &method.code else { continue };
                if code.insns_size > 12 {
                    continue;
                }
                let Some(signature) = resolver.method(MethodRef::new(method.method_idx)) else {
                    continue;
                };
                if signature.name != "asBinder" {
                    continue;
                }
                if method.access_flags & (0x20 | 0x100 | 0x400 | 0x20000) != 0 {
                    continue;
                }
                let Ok(ssa) = dex_decompiler::DexPasses::run_ssa_with(
                    code,
                    &dex_decompiler::DexTypes::new(dex, Some(method.method_idx)),
                ) else {
                    continue;
                };
                let Ok(deps) = requirements(ssa.ir(), method.method_idx, &resolver) else {
                    continue;
                };
                if deps.fields.is_empty() {
                    continue;
                }
                let Ok(bindings) = dex_java::parameter_bindings(
                    ssa.ir(),
                    &signature,
                    code.registers_size,
                    code.ins_size,
                ) else {
                    continue;
                };
                let SourceResult::Complete { source, .. } =
                    backend_java::emit_method(ssa.ir(), &signature, &bindings, &resolver)
                else {
                    continue;
                };
                deps.check_closure(&BTreeSet::from([method.method_idx]), &deps.fields)
                    .unwrap();
                for index in &deps.fields {
                    let declaration = resolver.field_declaration(*index).unwrap();
                    let field = resolver.field(FieldRef::new(*index)).unwrap();
                    assert!(declaration.contains(&field.name));
                    assert!(source.contains(&field.name));
                }
                println!("selected {}.{}: {source}", signature.owner, signature.name);
                return;
            }
        }
    }
    panic!("real OKcashbag member-bearing complete method required");
}

#[test]
fn invocation_metadata_dispatch_and_body_closure() {
    let mut dex = dex_format::DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let (ci, mi, index) = dex
        .classes
        .iter()
        .enumerate()
        .find_map(|(ci, c)| {
            c.class_data
                .as_ref()?
                .direct_methods
                .iter()
                .enumerate()
                .find_map(|(mi, m)| {
                    let name = dex
                        .string(dex.method_id(m.method_idx).ok()?.name_idx)
                        .ok()?;
                    (!name.starts_with('<')).then_some((ci, mi, m.method_idx))
                })
        })
        .expect("ordinary method metadata");
    dex.classes[ci].access_flags = 1;
    dex.classes[ci].class_data.as_mut().unwrap().direct_methods[mi].access_flags = 9;
    let code = dex_format::code::CodeItem {
        registers_size: 0,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: 4,
        insns: [0x0071u16, index as u16, 0, 0x000e]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect(),
        tries: vec![],
        handlers: vec![],
        span: dex_format::RawSpan { offset: 0, size: 8 },
    };
    let ssa = dex_decompiler::DexPasses::run_merged(&code).unwrap();
    let resolver = MemberResolver { dex: &dex };
    assert!(resolver.method(MethodRef::new(index)).unwrap().is_static);
    assert_eq!(resolver.declared_throws(index), Some(vec![]));
    let deps = requirements(ssa.ir(), index, &resolver).unwrap();
    assert_eq!(deps.methods, BTreeSet::from([index]));
    assert!(
        deps.check_closure(&BTreeSet::new(), &BTreeSet::new())
            .is_err()
    );
    deps.check_closure(&BTreeSet::from([index]), &BTreeSet::new())
        .unwrap();
    dex.classes[ci].class_data.as_mut().unwrap().direct_methods[mi].access_flags = 1;
    assert!(
        requirements(ssa.ir(), index, &MemberResolver { dex: &dex })
            .unwrap_err()
            .contains("dispatch")
    );
}
