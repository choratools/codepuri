//! Global original-declaration closure and real Java compiler regressions.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use backend_java::{JavaResolver, SourceResult};
use decomp::java_aliases::MethodKey;
use decomp::java_members::{MemberResolver, source_closure::*};
use dex_format::DexFile;
use managed_ir::refs::MethodRef;
use std::collections::{BTreeMap, BTreeSet};

fn fixture() -> DexFile {
    DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap()
}
fn key(owner: &str, name: &str, parameters: &[&str], ret: &str) -> MethodKey {
    MethodKey {
        owner: owner.into(),
        name: name.into(),
        parameters: parameters.iter().map(|s| (*s).into()).collect(),
        return_type: ret.into(),
    }
}

#[test]
fn declared_bytecode_mutual_recursion_cross_dex_single_root() {
    // A synthetic declared-function fixture, not fallback production bodies:
    // install actual invoke/return bytecode and matching class-data definitions.
    let mut dex = fixture();
    let mut definitions: Vec<_> = dex
        .classes
        .iter()
        .filter_map(|c| c.class_data.as_ref())
        .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
        .filter(|m| {
            !method_key(&dex, m.method_idx)
                .unwrap()
                .name
                .starts_with('<')
        })
        .take(2)
        .cloned()
        .collect();
    assert_eq!(definitions.len(), 2);
    let a = definitions[0].method_idx;
    let b = definitions[1].method_idx;
    let owner = dex.method_ids[a as usize].class_idx;
    dex.method_ids[b as usize].class_idx = owner;
    let proto = dex
        .proto_ids
        .iter()
        .position(|p| {
            p.parameters.as_ref().is_none_or(|p| p.types.is_empty())
                && dex
                    .string(dex.type_id(p.return_type_idx).unwrap().descriptor_idx)
                    .unwrap()
                    == "V"
        })
        .unwrap() as u16;
    dex.method_ids[a as usize].proto_idx = proto;
    dex.method_ids[b as usize].proto_idx = proto;
    assert_ne!(method_key(&dex, a), method_key(&dex, b));
    for (method, target) in definitions.iter_mut().zip([b, a]) {
        method.access_flags = 9;
        method.code = Some(dex_format::code::CodeItem {
            registers_size: 0,
            ins_size: 0,
            outs_size: 0,
            debug_info_off: 0,
            insns_size: 4,
            insns: [0x0071u16, target as u16, 0, 0x000e]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect(),
            tries: vec![],
            handlers: vec![],
            span: dex_format::RawSpan { offset: 0, size: 8 },
        });
    }
    for class in &mut dex.classes {
        class.annotations = None;
        if let Some(data) = &mut class.class_data {
            data.direct_methods.clear();
            data.virtual_methods.clear();
        }
    }
    let owner_class = dex
        .classes
        .iter()
        .position(|c| c.class_idx == u32::from(owner))
        .unwrap();
    dex.classes[owner_class].access_flags = 1;
    let mut second = dex.clone();
    dex.classes[owner_class]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(definitions[0].clone());
    second.classes[owner_class]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(definitions[1].clone());
    let files = [dex, second];
    let global = GlobalDeclarations::new(&files).unwrap();
    struct Resolver<'a, 'b> {
        dex: &'a DexFile,
        global: &'a GlobalDeclarations<'b>,
    }
    impl JavaResolver for Resolver<'_, '_> {
        fn class(&self, r: managed_ir::refs::ClassRef) -> Option<String> {
            MemberResolver { dex: self.dex }.class(r)
        }
        fn string(&self, r: managed_ir::refs::StringRef) -> Option<Vec<u16>> {
            MemberResolver { dex: self.dex }.string(r)
        }
        fn field(&self, r: managed_ir::refs::FieldRef) -> Option<backend_java::JavaField> {
            MemberResolver { dex: self.dex }.field(r)
        }
        fn method(&self, r: MethodRef) -> Option<backend_java::JavaMethod> {
            self.global.metadata(&method_key(self.dex, r.index())?)
        }
    }
    let root = method_key(&files[0], a).unwrap();
    let other = method_key(&files[1], b).unwrap();
    let mut visits = BTreeSet::new();
    let report = global.close([root.clone()], &BTreeMap::new(), |key, location| {
        assert!(visits.insert(key.clone()));
        let dex = &files[location.dex];
        let code = dex.classes[owner_class]
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods[0]
            .code
            .as_ref()
            .unwrap();
        let ssa = dex_decompiler::DexPasses::run_ssa_with(
            code,
            &dex_decompiler::DexTypes::new(dex, Some(location.index)),
        )
        .unwrap();
        let signature = global.metadata(key).unwrap();
        let bindings = decomp::dex_java::parameter_bindings(ssa.ir(), &signature, 0, 0).unwrap();
        let SourceResult::Complete { source, .. } = backend_java::emit_method(
            ssa.ir(),
            &signature,
            &bindings,
            &Resolver {
                dex,
                global: &global,
            },
        ) else {
            panic!("declared recursive body must emit")
        };
        Ok(EmittedMethod {
            source,
            dependencies: body_references(ssa.ir(), dex).unwrap(),
        })
    });
    assert_eq!(visits, BTreeSet::from([root.clone(), other.clone()]));
    assert_eq!(
        report.bodies[&root].dependencies,
        BTreeSet::from([other.clone()])
    );
    assert_eq!(
        report.bodies[&other].dependencies,
        BTreeSet::from([root.clone()])
    );
    assert!(report.unresolved.is_empty());
    let owner_name = global.metadata(&other).unwrap().owner;
    let (package, class) = owner_name.rsplit_once('.').unwrap_or(("", &owner_name));
    let dir = std::env::temp_dir().join(format!("decomp-dex-closure-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{class}.java"));
    let package = if package.is_empty() {
        String::new()
    } else {
        format!("package {package};\n")
    };
    std::fs::write(
        &file,
        format!(
            "{package}public class {class} {{\n{}\n}}",
            report
                .bodies
                .values()
                .map(|b| b.source.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .unwrap();
    let compiled = std::process::Command::new("javac")
        .arg(&file)
        .output()
        .expect("javac required");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    for (class_flags, method_flags, modifier) in [(1, 0x20109, ""), (0x401, 0x401, "abstract ")] {
        let mut bodyless_files = files.clone();
        let class_data = &mut bodyless_files[0].classes[owner_class];
        class_data.access_flags = class_flags;
        let method = &mut class_data.class_data.as_mut().unwrap().direct_methods[0];
        method.code = None;
        method.access_flags = method_flags;
        let declarations = GlobalDeclarations::new(&bodyless_files).unwrap().close(
            [root.clone()],
            &BTreeMap::new(),
            |_, _| panic!("genuine bodyless metadata"),
        );
        assert!(declarations.unresolved.is_empty());
        let declaration = &declarations.bodyless[&root];
        if method_flags & 0x100 != 0 {
            assert!(declaration.contains("synchronized native "));
        }
        std::fs::write(
            &file,
            format!("{package}public {modifier}class {class} {{ {declaration} }}"),
        )
        .unwrap();
        let compiled = std::process::Command::new("javac")
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{declaration}: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn refreshed_global_declarations_and_failed_class_fixed_point() {
    use decomp::java_members::dependency_publication::*;
    let a = key("LA;", "a", &[], "V");
    let b = key("LB;", "b", &[], "V");
    let mut publication = DependencyPublication::default();
    let mut semantic = BodyPublicationReport::default();
    publication.record_emitted_body(&mut semantic, a.clone(), [b.clone()]);
    assert!(!semantic.declaration_closed());
    publication
        .declarations
        .insert(b.clone(), DeclarationStatus::CompleteBody);
    publication
        .declarations
        .insert(a.clone(), DeclarationStatus::CompleteBody);
    publication.record_emitted_body(&mut semantic, b.clone(), [a.clone()]);
    publication.refresh(&mut semantic);
    assert!(semantic.declaration_closed());
    assert_eq!(semantic.complete_bodies.len(), 2);
    assert_eq!(
        publication.body_closure,
        BTreeSet::from([a.clone(), b.clone()])
    );
    let report = SourceClosure {
        bodies: BTreeMap::from([
            (
                a.clone(),
                EmittedMethod {
                    source: "static void a() { B.b(); }".into(),
                    dependencies: BTreeSet::from([b.clone()]),
                },
            ),
            (
                b.clone(),
                EmittedMethod {
                    source: "static void b() { A.a(); }".into(),
                    dependencies: BTreeSet::from([a.clone()]),
                },
            ),
        ]),
        ..Default::default()
    };
    let classes = BTreeSet::from([a.owner.clone(), b.owner.clone()]);
    let mut validation = BTreeMap::from([
        (
            a.owner.clone(),
            ClassValidation {
                methods: BTreeSet::from([a.clone()]),
                ..Default::default()
            },
        ),
        (
            b.owner.clone(),
            ClassValidation {
                methods: BTreeSet::from([b.clone()]),
                ..Default::default()
            },
        ),
    ]);
    assert_eq!(
        report
            .select_classes(&classes, &validation, SourceSelection::StrictValidClasses)
            .selected,
        classes
    );
    validation
        .get_mut(&b.owner)
        .unwrap()
        .failures
        .push("original concrete class does not implement required interface method".into());
    let failed = report.select_classes(&classes, &validation, SourceSelection::StrictValidClasses);
    assert!(failed.selected.is_empty());
    assert!(failed.failed[&a.owner][0].contains("source class dependency unavailable"));
    assert!(failed.failed[&b.owner][0].contains("original concrete class"));
    assert_eq!(report.bodies.len(), 2);
}

#[test]
fn real_declared_bodies_global_cross_dex_and_exact_overloads() {
    let original = fixture();
    let mut first = fixture();
    let mut second = fixture();
    // Partition actual definitions, retaining each pool's original reference IDs.
    for class in &mut first.classes {
        if let Some(data) = &mut class.class_data {
            data.direct_methods.retain(|m| m.method_idx % 2 == 0);
            data.virtual_methods.retain(|m| m.method_idx % 2 == 0);
        }
    }
    for class in &mut second.classes {
        if let Some(data) = &mut class.class_data {
            data.direct_methods.retain(|m| m.method_idx % 2 == 1);
            data.virtual_methods.retain(|m| m.method_idx % 2 == 1);
        }
    }
    let files = [first, second];
    let index = GlobalDeclarations::new(&files).unwrap();
    let resolver = MemberResolver { dex: &original };
    let mut publications = BTreeMap::new();
    for (key, locations) in &index.methods {
        assert_eq!(locations.len(), 1);
        let location = locations[0];
        assert_eq!(location.dex, (location.index % 2) as usize);
        let global = index.metadata(key);
        let local = resolver.method(MethodRef::new(location.index));
        assert_eq!(global.is_some(), local.is_some());
        if let (Some(global), Some(local)) = (global, local) {
            assert_eq!(global.name, local.name);
            assert_eq!(global.parameters, local.parameters);
            assert_eq!(global.return_type, local.return_type);
            assert_eq!(global.is_static, local.is_static);
            assert_eq!(global.throws, local.throws);
        }
        let Some(code) = original
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|d| d.direct_methods.iter().chain(&d.virtual_methods))
            .find(|m| m.method_idx == location.index)
            .and_then(|m| m.code.as_ref())
        else {
            continue;
        };
        let Some(signature) = resolver.method(MethodRef::new(location.index)) else {
            continue;
        };
        let Ok(ssa) = dex_decompiler::DexPasses::run_ssa_with(
            code,
            &dex_decompiler::DexTypes::new(&original, Some(location.index)),
        ) else {
            continue;
        };
        let Ok(bindings) = decomp::dex_java::parameter_bindings(
            ssa.ir(),
            &signature,
            code.registers_size,
            code.ins_size,
        ) else {
            continue;
        };
        if let SourceResult::Complete { source, .. } =
            backend_java::emit_method(ssa.ir(), &signature, &bindings, &resolver)
        {
            publications.insert(
                key.clone(),
                EmittedMethod {
                    source,
                    dependencies: body_references(ssa.ir(), &original).unwrap(),
                },
            );
        }
    }
    assert!(
        !publications.is_empty(),
        "fixture must contain actual successfully emitted bodies"
    );
    let mut visited = BTreeSet::new();
    let report = index.close(
        publications.keys().cloned(),
        &BTreeMap::new(),
        |key, location| {
            assert!(
                visited.insert(key.clone()),
                "one global visit per definition"
            );
            assert_eq!(
                method_key(&files[location.dex], location.index).as_ref(),
                Some(key)
            );
            publications
                .get(key)
                .cloned()
                .ok_or_else(|| "original body emission unsupported".into())
        },
    );
    for (key, body) in publications {
        assert_eq!(report.bodies[&key], body);
    }
    let a = key("LF;", "f", &["I"], "I");
    let b = key("LF;", "f", &["J"], "I");
    let c = key("LF;", "f", &["I"], "J");
    assert_eq!(BTreeSet::from([a, b, c]).len(), 3);
}

#[test]
fn unavailable_original_body_native_abstract_and_ambiguity() {
    let mut dex = fixture();
    let (ci, mi, method_index) = dex
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
                    (!method_key(&dex, m.method_idx)?.name.starts_with('<')).then_some((
                        ci,
                        mi,
                        m.method_idx,
                    ))
                })
        })
        .unwrap();
    let root = method_key(&dex, method_index).unwrap();
    let method = &mut dex.classes[ci].class_data.as_mut().unwrap().direct_methods[mi];
    method.code = None;
    method.access_flags = 9;
    let absent = GlobalDeclarations::new(std::slice::from_ref(&dex))
        .unwrap()
        .close([root.clone()], &BTreeMap::new(), |_, _| {
            panic!("missing code cannot be emitted")
        });
    assert!(absent.unresolved[&root].contains("original concrete method body unavailable"));
    assert!(absent.bodies.is_empty() && absent.bodyless.is_empty());
    dex.classes[ci].class_data.as_mut().unwrap().direct_methods[mi].access_flags = 0x109;
    let native = GlobalDeclarations::new(std::slice::from_ref(&dex))
        .unwrap()
        .close([root.clone()], &BTreeMap::new(), |_, _| {
            panic!("native must remain bodyless")
        });
    assert!(native.bodyless[&root].contains("native "));
    dex.classes[ci].access_flags = 0x401;
    dex.classes[ci].class_data.as_mut().unwrap().direct_methods[mi].access_flags = 0x401;
    let abstract_report = GlobalDeclarations::new(std::slice::from_ref(&dex))
        .unwrap()
        .close([root.clone()], &BTreeMap::new(), |_, _| {
            panic!("abstract must remain bodyless")
        });
    assert!(abstract_report.bodyless[&root].contains("abstract "));
    let duplicated = [fixture(), fixture()];
    let ambiguous = GlobalDeclarations::new(&duplicated).unwrap().close(
        [root.clone()],
        &BTreeMap::new(),
        |_, _| panic!("ambiguous definition"),
    );
    assert_eq!(ambiguous.unresolved[&root], "ambiguous global declaration");
}

#[test]
fn exact_external_declarations_and_explicit_class_selection() {
    let files = [];
    let index = GlobalDeclarations::new(&files).unwrap();
    let known = key("LApi;", "call", &["I"], "V");
    let unknown = key("LApi;", "call", &["J"], "V");
    let external = BTreeMap::from([(
        known.clone(),
        decomp::java_external_metadata::MethodMetadata {
            access_flags: 9,
            exceptions: Some(vec![]),
        },
    )]);
    let mut report = index.close([known.clone(), unknown.clone()], &external, |_, _| panic!());
    assert!(report.external.contains(&known));
    assert!(report.unresolved.contains_key(&unknown));
    let caller = key("LCaller;", "run", &[], "V");
    report.bodies.insert(
        caller.clone(),
        EmittedMethod {
            source: "public static void run() { Api.call(1L); }".into(),
            dependencies: BTreeSet::from([unknown.clone()]),
        },
    );
    assert_eq!(report.direct_missing()[&caller], BTreeSet::from([unknown]));
    let classes = BTreeSet::from(["LCaller;".into(), "LUnvalidated;".into()]);
    let validations = BTreeMap::from([(
        "LCaller;".into(),
        ClassValidation {
            methods: BTreeSet::from([caller]),
            ..Default::default()
        },
    )]);
    let strict = report.select_classes(&classes, &validations, SourceSelection::StrictValidClasses);
    assert!(strict.selected.is_empty());
    assert_eq!(strict.failed.len(), 2);
    let analyst = report.select_classes(&classes, &validations, SourceSelection::AnalystAll);
    assert_eq!(analyst.selected, classes);
    assert_eq!(analyst.failed, strict.failed);
}

#[test]
fn actual_javac_failure_metadata_and_recursive_overloaded_runtime() {
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!("decomp-closure-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("Fixture.java");
    // Actual declared recursive and overloaded functions with an observable oracle.
    std::fs::write(&source, "public class Fixture { static int even(int n) { return n == 0 ? 1 : odd(n-1); } static int odd(int n) { return n == 0 ? 0 : even(n-1); } static long even(long n) { return n + 10; } public static void main(String[] args) { if (even(8) != 1 || odd(8) != 0 || even(2L) != 12) throw new AssertionError(); System.out.print(\"closure-ok\"); } }").unwrap();
    let compiled = Command::new("javac")
        .arg(&source)
        .output()
        .expect("javac required");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = Command::new("java")
        .arg("-cp")
        .arg(&dir)
        .arg("Fixture")
        .output()
        .unwrap();
    assert!(executed.status.success());
    assert_eq!(executed.stdout, b"closure-ok");
    std::fs::write(&source, "interface Required { int run(); } public class Fixture implements Required { public static int caller() { return missing(1); } }").unwrap();
    let failed = Command::new("javac").arg(&source).output().unwrap();
    assert!(!failed.status.success());
    let diagnostics = String::from_utf8(failed.stderr).unwrap();
    assert!(
        diagnostics.contains("missing") && diagnostics.contains("run()"),
        "{diagnostics}"
    );
    let caller = key("LFixture;", "caller", &[], "I");
    let missing = key("LFixture;", "missing", &["I"], "I");
    let required = key("LFixture;", "run", &[], "I");
    let report = SourceClosure {
        bodies: BTreeMap::from([(
            caller.clone(),
            EmittedMethod {
                source: "public static int caller() { return missing(1); }".into(),
                dependencies: BTreeSet::from([missing.clone()]),
            },
        )]),
        unresolved: BTreeMap::from([(missing, "original body unavailable".into())]),
        ..Default::default()
    };
    let validations = BTreeMap::from([(
        "LFixture;".into(),
        ClassValidation {
            methods: BTreeSet::from([caller]),
            required_methods: BTreeSet::from([required]),
            failures: vec![diagnostics],
            ..Default::default()
        },
    )]);
    let selected = report.select_classes(
        &BTreeSet::from(["LFixture;".into()]),
        &validations,
        SourceSelection::StrictValidClasses,
    );
    assert!(selected.selected.is_empty());
    assert!(
        selected.failed["LFixture;"]
            .iter()
            .any(|s| s.contains("required original declaration"))
    );
    assert_eq!(
        report.bodies.len(),
        1,
        "compiler failure does not erase semantic identity"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
