//! Read-only actual Samsung callee-throws evidence probe.
#![allow(clippy::unwrap_used)]
use backend_java::type_compatibility::{Compatibility, Hierarchy};
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType};
use decomp::java_aliases::MethodKey;
use decomp::java_external_metadata::{Classpath, ExternalMetadata};
use dex_format::DexFile;
use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
use std::collections::{BTreeMap, BTreeSet};
struct Resolver(Hierarchy);
impl JavaResolver for Resolver {
    fn assignable(&self, a: &JavaType, e: &JavaType) -> bool {
        self.0.assignable(a, e) == Compatibility::Assignable
    }
    fn method(&self, _: MethodRef) -> Option<JavaMethod> {
        None
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
fn desc(d: &DexFile, i: u32) -> String {
    d.string(d.type_id(i).unwrap().descriptor_idx)
        .unwrap()
        .into()
}
fn key(d: &DexFile, i: u32) -> MethodKey {
    let m = d.method_id(i).unwrap();
    let p = d.proto_id(u32::from(m.proto_idx)).unwrap();
    MethodKey {
        owner: desc(d, u32::from(m.class_idx)),
        name: d.string(m.name_idx).unwrap().into(),
        parameters: p
            .parameters
            .as_ref()
            .map(|p| p.types.iter().map(|x| desc(d, u32::from(*x))).collect())
            .unwrap_or_default(),
        return_type: desc(d, p.return_type_idx),
    }
}
#[test]
#[ignore = "Requires original Samsung Pass APK and genuine local JDK/Android SDK"]
fn actual_samsung_throws_evidence() {
    let dexes = dex_format::apk::parse_apk_path(
        "../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    let cp = Classpath::from_paths([
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod",
        "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
    ])
    .unwrap();
    let refs: Vec<_> = dexes.iter().collect();
    let r = Resolver(decomp::java_hierarchy::build_with_metadata(&refs, &cp).unwrap());
    let mut declarations = BTreeMap::new();
    for d in &dexes {
        let mr = decomp::java_members::MemberResolver { dex: d };
        for c in &d.classes {
            if let Some(data) = &c.class_data {
                for m in data.direct_methods.iter().chain(&data.virtual_methods) {
                    declarations.insert(
                        key(d, m.method_idx),
                        mr.declared_throws_for_class(c, m.method_idx),
                    );
                }
            }
        }
    }
    let selected: BTreeSet<(usize, u32)> =
        [(0, 87), (0, 92), (0, 93), (0, 119), (0, 120), (0, 584)]
            .into_iter()
            .collect();
    let mut gained = 0;
    let mut negatives = 0;
    for (di, d) in dexes.iter().enumerate() {
        for c in &d.classes {
            if let Some(data) = &c.class_data {
                for m in data.direct_methods.iter().chain(&data.virtual_methods) {
                    if !selected.contains(&(di, m.method_idx)) {
                        continue;
                    }
                    let caller = key(d, m.method_idx);
                    let Some(Some(declared)) = declarations.get(&caller) else {
                        continue;
                    };
                    let Some(code) = &m.code else { continue };
                    for ins in dex_decompiler::decode_method_recovering(code, 2).insns {
                        let dex_decompiler::decode::Opcode::Supported(op) = ins.opcode else {
                            continue;
                        };
                        if !(0x6e..=0x78).contains(&op.opcode) {
                            continue;
                        }
                        let Some(target) = ins.pool_index else {
                            continue;
                        };
                        let callee = key(d, target);
                        let throws = declarations.get(&callee).cloned().flatten().or_else(|| {
                            cp.method(&callee).and_then(|m| m.exceptions).map(|v| {
                                v.into_iter()
                                    .map(|s| {
                                        s.trim_start_matches('L')
                                            .trim_end_matches(';')
                                            .replace('/', ".")
                                    })
                                    .collect()
                            })
                        });
                        let Some(throws) = throws else { continue };
                        if !throws.iter().any(|t| !declared.contains(t)) {
                            continue;
                        }
                        let accepted =
                            backend_java::checked_exceptions::check(&r, &throws, declared).is_ok();
                        if accepted {
                            gained += 1;
                            if gained <= 12 {
                                println!(
                                    "COVERED dex={di} method={} code={} invoke={} caller={caller:?} declared={declared:?} target={callee:?} throws={throws:?}",
                                    m.method_idx, code.span.offset, ins.offset
                                );
                            }
                        } else {
                            negatives += 1;
                            if negatives <= 3 {
                                println!(
                                    "UNDECLARED dex={di} method={} code={} invoke={} caller={caller:?} declared={declared:?} target={callee:?} throws={throws:?}",
                                    m.method_idx, code.span.offset, ins.offset
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    println!("covered_edges={gained} genuinely_uncovered_edges={negatives}");
    assert_eq!(gained, 3);
    assert_eq!(negatives, 3);
}
