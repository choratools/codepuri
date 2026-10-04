//! Original private enum production-resolver attribution, with complete inputs.
#![allow(clippy::unwrap_used)]
use super::*;
#[test]
#[ignore = "original Samsung APK, exact metadata and production enum planning"]
fn actual_private_enum_production_wave17() {
    use crate::java_external_metadata::Classpath;
    use std::path::PathBuf;

    let inputs = dex_format::apk::parse_apk_path(
        "/storage_0/decomphify/tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    let cp = Classpath::from_paths([
        PathBuf::from("/tmp/decomphify-source25-java-base-649174/java.base.jar"),
        PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
    ])
    .unwrap();
    let writes = initializers::write_inventory::WriteInventory::scan_with_metadata(&inputs, &cp);
    let mut profile = None;
    let (aliases, publication) = export_inventory(&inputs, &mut profile, &writes, None).unwrap();
    let catalog = method_catalog(&inputs);
    let hierarchy =
        crate::java_hierarchy::build_with_metadata(&inputs.iter().collect::<Vec<_>>(), &cp)
            .unwrap();
    let original_hierarchy = std::sync::Arc::new(hierarchy.clone());
    let mut class_catalog = BTreeMap::new();
    for declaration in cp.hierarchy() {
        let original = format!("L{};", declaration.name.replace('.', "/"));
        if let Some(access_flags) = cp.class_access(&original) {
            let name = aliases
                .class_name(&original)
                .unwrap_or(&declaration.name)
                .to_owned();
            let superclass = declaration.superclass.map(|s| {
                aliases
                    .class_name(&format!("L{};", s.replace('.', "/")))
                    .unwrap_or(&s)
                    .to_owned()
            });
            class_catalog.insert(
                name.clone(),
                Some(backend_java::constructors::ConstructorClass {
                    name,
                    superclass,
                    access_flags,
                }),
            );
        }
    }
    for dex in &inputs {
        for class in &dex.classes {
            if let Some(name) = descriptor(dex, class.class_idx).and_then(|d| aliases.class_name(d))
            {
                let declaration = backend_java::constructors::ConstructorClass {
                    name: name.into(),
                    superclass: descriptor(dex, class.superclass_idx)
                        .and_then(|d| aliases.class_name(d))
                        .map(str::to_owned),
                    access_flags: class.access_flags,
                };
                class_catalog.insert(name.into(), Some(declaration));
            }
        }
    }
    let input_owners = inputs
        .iter()
        .flat_map(|dex| {
            dex.classes
                .iter()
                .filter_map(|class| descriptor(dex, class.class_idx).map(str::to_owned))
        })
        .collect::<BTreeSet<_>>();

    let metadata = inputs
        .iter()
        .map(dex_decompiler::infer::DexMetadata::new)
        .collect::<Vec<_>>();

    struct Fixture<'a> {
        inputs: &'a [DexFile],
        metadata: Vec<dex_decompiler::infer::DexMetadata>,
        aliases: &'a JavaAliases,
        cp: &'a Classpath,
        writes: &'a initializers::write_inventory::WriteInventory,
        publication: &'a FieldPublication,
        class_catalog: BTreeMap<String, Option<backend_java::constructors::ConstructorClass>>,
        hierarchy: crate::type_compatibility::Hierarchy,
        original_hierarchy: std::sync::Arc<crate::type_compatibility::Hierarchy>,
        input_owners: BTreeSet<String>,
    }
    impl<'a> Fixture<'a> {
        fn resolver(
            &'a self,
            pool: usize,
            catalog: &'a BTreeMap<MethodKey, Option<MethodMetadata>>,
        ) -> Resolver<'a> {
            let mut r = Resolver::with_catalog(
                &self.inputs[pool],
                self.aliases,
                &self.metadata[pool],
                Some(catalog),
            );
            r.source_release = 25;
            r.cache_inventory = true;
            r.inputs = self.inputs;
            r.dex_index = pool;
            r.writes = Some(self.writes);
            r.field_catalog = Some(self.publication);
            r.class_catalog = self.class_catalog.clone();
            r.external = Some(self.cp);
            r.hierarchy = self.hierarchy.clone();
            r.original_hierarchy = Some(std::sync::Arc::clone(&self.original_hierarchy));
            r.input_owners = self.input_owners.clone();
            r
        }
    }
    let fixture = Fixture {
        inputs: &inputs,
        metadata,
        aliases: &aliases,
        cp: &cp,
        writes: &writes,
        publication: &publication,
        class_catalog,
        hierarchy: hierarchy.clone(),
        original_hierarchy: std::sync::Arc::clone(&original_hierarchy),
        input_owners,
    };

    for (pool, owner) in [
        (3, "Lorg/threeten/bp/DayOfWeek;"),
        (
            1,
            "Lcom/samsung/android/samsungpass/ocr/nfc/cardtagging/Tag/CommandEnum;",
        ),
    ] {
        let r = fixture.resolver(pool, &catalog);
        let class = inputs[pool]
            .classes
            .iter()
            .find(|c| descriptor(&inputs[pool], c.class_idx) == Some(owner))
            .unwrap();
        let result = enum_source(class, &r, &mut None);
        if pool == 3 {
            use crate::java_class_kinds::enum_initialization::{self as enumeration, emission};
            let data = class.class_data.as_ref().unwrap();
            let lift = |m: &EncodedMethod| {
                let (context, seeds) = initial_context_with_seeds(&r, m).unwrap();
                cached_frontend(&r, m, m.code.as_ref().unwrap(), &context, seeds, None).unwrap()
            };
            let ctor = data
                .direct_methods
                .iter()
                .find(|m| m.method_idx == 35588)
                .unwrap();
            let clinit = data
                .direct_methods
                .iter()
                .find(|m| m.method_idx == 35587)
                .unwrap();
            let cs = lift(ctor);
            let ci = lift(clinit);
            let b = declaration_bindings(
                &cs,
                &r.method(MethodRef::new(35588)).unwrap(),
                ctor.code.as_ref().unwrap(),
            )
            .unwrap();
            let inputs = [enumeration::EnumConstructorDeclarationInput {
                reference: MethodRef::new(35588),
                ir: cs.ir(),
                bindings: &b,
            }];
            let init = enumeration::plan_enum_initialization_family_with_prepared_values(
                fixture.inputs,
                pool,
                class,
                &inputs,
                MethodRef::new(35587),
                ci.ir(),
                &r,
                &writes,
            )
            .unwrap();
            let methods = emission::plan_enum_methods(r.dex, class, &init).unwrap();
            for m in data.direct_methods.iter().chain(&data.virtual_methods) {
                if [35587, 35588, methods.values, methods.value_of].contains(&m.method_idx) {
                    continue;
                }
                let body = lift(m);
                if let Err(e) = emission::validate_enum_member_ir(class, &init, body.ir()) {
                    println!(
                        "ENUM_MEMBER_GATE {} {:?} {e:?}",
                        m.method_idx,
                        method_key(r.dex, m.method_idx)
                    );
                    for insn in body.ir().instructions().values() {
                        println!("ENUM_MEMBER_IR {insn:?}");
                    }
                }
            }
        }
        match result {
            Ok(source) => {
                println!("PRODUCTION_ENUM_OK {owner} {}", source.source);
                if pool == 1 {
                    assert_eq!(source.publications.len(), 9);
                    assert!(source.source.contains("this.p1 ="));
                    let dir = std::path::PathBuf::from("/tmp/opencode/wave21-command-original");
                    std::fs::create_dir_all(&dir).unwrap();
                    let name = aliases.class_name(owner).unwrap();
                    let (package, simple) = name.rsplit_once('.').unwrap();
                    std::fs::write(
                        dir.join(format!("{simple}.java")),
                        format!("package {package};\n{}", source.source),
                    )
                    .unwrap();
                }
            }
            Err(error) => {
                println!("PRODUCTION_ENUM_ERROR {owner} {error}");
                assert_ne!(pool, 1, "original CommandEnum remains unavailable: {error}");
            }
        }
    }
}
