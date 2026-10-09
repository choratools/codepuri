//! Receiver identity is distinct from constructors of argument allocations.
#![allow(clippy::unwrap_used)]
use super::*;
use managed_ir::{
    effects::EffectFlags,
    method::{InsnArena, ValueArena},
    types::ManagedType,
    value::Value,
};
fn fixture(
    use_receiver: bool,
    copy_receiver: bool,
    conditional: bool,
) -> (MethodIr<SsaForm>, managed_ir::value::ValueId) {
    let mut insns = InsnArena::with_key();
    let mut values = ValueArena::with_key();
    let mut receiver = None;
    let entry = insns.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
        receiver = Some(result);
        ManagedInsn::Opaque {
            family_tag: 0xfffe,
            reads: vec![],
            writes: vec![result],
            effects: EffectFlags::NONE,
        }
    });
    let receiver = receiver.unwrap();
    let mut object = None;
    let allocation = insns.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
        object = Some(result);
        ManagedInsn::NewInstance {
            result,
            class: ClassRef::new(0),
        }
    });
    let object = object.unwrap();
    let argument_init = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(7),
        args: vec![object],
    });
    let mut alias = None;
    let copy = insns.insert_with_key(|id| {
        let result = values.insert(Value::new(id, ManagedType::Reference(ClassRef::new(0))));
        alias = Some(result);
        ManagedInsn::Move {
            result,
            value: receiver,
        }
    });
    let target = if use_receiver {
        if copy_receiver {
            alias.unwrap()
        } else {
            receiver
        }
    } else {
        object
    };
    let chain = insns.insert(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        result: None,
        method: MethodRef::new(9),
        args: vec![target],
    });
    let ret = insns.insert(ManagedInsn::Return { value: None });
    let mut cfg = decomp_core::cfg::Cfg::new(entry);
    let mut previous = cfg.entry();
    for id in [allocation, argument_init, copy, chain, ret] {
        let next = cfg.add_block(id);
        cfg.add_edge(previous, next, decomp_core::cfg::EdgeKind::Unconditional)
            .unwrap();
        previous = next;
    }
    if conditional {
        cfg.add_edge(
            cfg.entry(),
            previous,
            decomp_core::cfg::EdgeKind::BranchTrue,
        )
        .unwrap();
    }
    (
        MethodIr::<SsaForm>::from_parts(cfg, insns, values),
        receiver,
    )
}
fn key(_: MethodRef) -> Option<MethodKey> {
    Some(MethodKey {
        owner: "LOwner;".into(),
        name: "<init>".into(),
        parameters: vec![],
        return_type: "V".into(),
    })
}
#[test]
fn receiver_bound_this_target_skips_even_same_class_argument_initializers() {
    for copied in [false, true] {
        let (ir, receiver) = fixture(true, copied, false);
        assert_eq!(
            attested_this_target(&ir, receiver, "LOwner;", key),
            Some(MethodRef::new(9))
        );
    }
}
#[test]
fn absent_receiver_ambiguous_flow_and_other_owner_never_attest_this() {
    for (actual, branch) in [(false, false), (true, true)] {
        let (ir, receiver) = fixture(actual, false, branch);
        assert_eq!(attested_this_target(&ir, receiver, "LOwner;", key), None);
    }
    let (ir, receiver) = fixture(true, false, false);
    assert_eq!(attested_this_target(&ir, receiver, "LOther;", key), None);
    assert_eq!(
        attested_this_target(&ir, receiver, "LOwner;", |_| None),
        None
    );
}

#[test]
#[ignore = "requires retained original SamsungPass APK and Java25/Android metadata"]
fn actual_nullable_mask_constructors_attest_through_production_facade() {
    use crate::java_external_metadata::Classpath;
    use backend_java::constructors::ConstructorResolver;
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
        catalog: &'a BTreeMap<MethodKey, Option<MethodMetadata>>,
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
            r.class_catalog = self.class_catalog.clone().into();
            r.external = Some(self.cp);
            r.hierarchy = self.hierarchy.clone();
            r.original_hierarchy = Some(std::sync::Arc::clone(&self.original_hierarchy));
            r.input_owners = self.input_owners.clone().into();
            r
        }
    }
    let fixture = Fixture {
        inputs: &inputs,
        metadata,
        aliases: &aliases,
        catalog: &catalog,
        cp: &cp,
        writes: &writes,
        publication: &publication,
        class_catalog,
        hierarchy: hierarchy.clone(),
        original_hierarchy: std::sync::Arc::clone(&original_hierarchy),
        input_owners,
    };

    for (pool, index, target) in [
        (1, 14488, 14487),
        (1, 14525, 14524),
        (1, 14544, 14543),
        (1, 14565, 14564),
        (1, 14605, 14604),
        (1, 14670, 14669),
        (1, 15050, 15052),
        (1, 15053, 15052),
    ] {
        let dex = &inputs[pool];
        let key = method_key(dex, index).unwrap();
        let class = dex
            .classes
            .iter()
            .find(|class| descriptor(dex, class.class_idx) == Some(key.owner.as_str()))
            .unwrap();
        let r = fixture.resolver(pool, fixture.catalog);
        assert_eq!(
            r.initializer(MethodRef::new(index)).unwrap().delegates_to,
            Some(MethodRef::new(target)),
            "production resolver mask delegation for #{index}"
        );
        let (candidates, failures, prefix) = initialization_source(class, &r, &mut None).unwrap();
        assert!(
            failures
                .get(&index)
                .is_none_or(|reason| !reason.contains("this delegation mapping mismatch")),
            "#{index} still has mapping mismatch: {:?}",
            failures.get(&index)
        );
        assert!(
            candidates.contains_key(&index),
            "#{index} was not retained by production initialization facade; prefix={prefix}; failures={failures:?}"
        );

        let cyclic = fixture.resolver(pool, fixture.catalog);
        cyclic
            .constructor_delegations
            .lock()
            .unwrap()
            .insert(target, Some(MethodRef::new(index)));
        assert_eq!(
            cyclic
                .initializer(MethodRef::new(index))
                .unwrap()
                .delegates_to,
            Some(MethodRef::new(target)),
            "mask proof must not recurse through target initializer metadata for #{index}"
        );

        let mut bad_catalog = catalog.clone();
        let target_key = method_key(dex, target).unwrap();
        bad_catalog.insert(target_key, None);
        let missing = fixture.resolver(pool, &bad_catalog);
        assert_eq!(
            missing
                .initializer(MethodRef::new(index))
                .unwrap()
                .delegates_to,
            None,
            "missing target metadata must fail closed for #{index}"
        );
    }
}
