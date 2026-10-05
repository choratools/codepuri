//! Shared evidence must preserve the existing explicit-store proof boundary.
#![allow(clippy::unwrap_used)]
use super::super::{
    ClassInitializationPlan, Error, plan_class_initialization_shared_explicit_stores,
    plan_class_initialization_with_explicit_stores, plan_static_initializer_explicit_stores,
    plan_static_initializer_shared_explicit_stores,
};
use super::*;
use backend_java::constructors::{
    ConstructorClass, ConstructorInput, ConstructorResolver, Initializer,
    TrustedConstructorResolver,
};
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType};
use dex_format::{
    RawSpan,
    class::{ClassData, EncodedField, EncodedMethod},
    code::CodeItem,
    ids::{FieldId, MethodId, ProtoId, TypeId},
};
use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};

fn native(method: u32, reason: &str) -> UnknownObservation {
    UnknownObservation {
        kind: UnknownKind::NativeCapability { dex: 0, method },
        reason: reason.into(),
    }
}

#[test]
fn shared_capabilities_preserve_exact_kinds_order_and_reasons() {
    let expected = [
        native(7, "structural-looking capability reason"),
        UnknownObservation {
            kind: UnknownKind::ResolvedReflectiveCapability {
                dex: 2,
                method: 4,
                offset: 9,
                target: 3,
            },
            reason: "original reflective observation".into(),
        },
        native(7, "structural-looking capability reason"),
    ];
    let mut inventory = WriteInventory::default();
    inventory.record_unknown(expected[0].kind.clone(), expected[0].reason.clone());
    inventory.record_unknown(
        UnknownKind::Structural,
        "native-looking structural reason".into(),
    );
    for observation in &expected[1..] {
        inventory.record_unknown(observation.kind.clone(), observation.reason.clone());
    }
    let original = inventory.unknown_observations().to_vec();
    let first = inventory.runtime_capabilities_shared();
    let second = inventory.runtime_capabilities_shared();
    assert_eq!(first.as_ref(), expected.as_slice());
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(inventory.unknown_observations(), original);
    assert!(inventory.has_structural_unknown());
    assert_eq!(inventory.unknown_reasons().len(), 4);
}

#[test]
fn capability_snapshots_are_isolated_across_inventory_mutation() {
    let mut inventory = WriteInventory::default();
    let first = native(1, "first");
    inventory.record_unknown(first.kind.clone(), first.reason.clone());
    let original = inventory.runtime_capabilities_shared();
    let mut cloned = inventory.clone();
    assert!(Arc::ptr_eq(
        &original,
        &cloned.runtime_capabilities_shared()
    ));
    let second = native(2, "second");
    inventory.record_unknown(second.kind.clone(), second.reason.clone());
    let updated = inventory.runtime_capabilities_shared();
    assert!(!Arc::ptr_eq(&original, &updated));
    assert_eq!(original.as_ref(), std::slice::from_ref(&first));
    assert_eq!(updated.as_ref(), [first.clone(), second].as_slice());
    cloned.record_unknown(UnknownKind::Structural, "clone-only failure".into());
    let cloned_snapshot = cloned.runtime_capabilities_shared();
    assert!(!Arc::ptr_eq(&original, &cloned_snapshot));
    assert_eq!(cloned_snapshot.as_ref(), original.as_ref());
    assert!(!inventory.has_structural_unknown());
    assert!(cloned.has_structural_unknown());
    let mut independent = WriteInventory::default();
    independent.record_unknown(first.kind, first.reason);
    assert!(!Arc::ptr_eq(
        &original,
        &independent.runtime_capabilities_shared()
    ));
}

#[test]
fn concurrent_readers_share_one_capability_snapshot() {
    let mut inventory = WriteInventory::default();
    inventory.record_unknown(
        UnknownKind::NativeCapability { dex: 0, method: 1 },
        "native".into(),
    );
    let barrier = std::sync::Barrier::new(8);
    let snapshots = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    inventory.runtime_capabilities_shared()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(
        snapshots
            .iter()
            .all(|snapshot| Arc::ptr_eq(snapshot, &snapshots[0]))
    );
}

const SPAN: RawSpan = RawSpan { offset: 0, size: 0 };

fn string(dex: &mut DexFile, value: &str) -> u32 {
    let mut item = dex.strings[0].clone();
    item.value = value.into();
    let index = dex.strings.len() as u32;
    dex.strings.push(item);
    index
}

fn ty(dex: &mut DexFile, descriptor: &str) -> u32 {
    let descriptor_idx = string(dex, descriptor);
    let index = dex.type_ids.len() as u32;
    dex.type_ids.push(TypeId {
        descriptor_idx,
        span: SPAN,
    });
    index
}

fn code(units: &[u16], incoming: u16) -> CodeItem {
    CodeItem {
        registers_size: 1,
        ins_size: incoming,
        outs_size: 1,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|unit| unit.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: SPAN,
    }
}

fn method(dex: &mut DexFile, class: u32, proto_idx: u16, name: &str) -> u32 {
    let name_idx = string(dex, name);
    let index = dex.method_ids.len() as u32;
    dex.method_ids.push(MethodId {
        class_idx: class as u16,
        proto_idx,
        name_idx,
        span: SPAN,
    });
    index
}

/// One real constructor, one static final assigned once in clinit, one native
/// declaration. The native observation prevents runtime-exclusivity claims.
fn fixture() -> (DexFile, u32, u32, u32) {
    let mut dex = DexFile::from_bytes(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/dex/clean/classes.dex"
    )))
    .unwrap();
    let mut class = dex.classes[0].clone();
    class.class_idx = ty(&mut dex, "LFixture;");
    class.superclass_idx = ty(&mut dex, "Ljava/lang/Object;");
    class.access_flags = 1;
    class.interfaces_off = 0;
    class.interfaces = None;
    class.annotations_off = 0;
    class.annotations = None;
    class.static_values_off = 0;
    class.static_values = None;
    let type_idx = ty(&mut dex, "I") as u16;
    let name_idx = string(&mut dex, "FINAL");
    let field = dex.field_ids.len() as u32;
    dex.field_ids.push(FieldId {
        class_idx: class.class_idx as u16,
        type_idx,
        name_idx,
        span: SPAN,
    });
    let return_type_idx = ty(&mut dex, "V");
    let proto_idx = dex.proto_ids.len() as u16;
    dex.proto_ids.push(ProtoId {
        shorty_idx: 0,
        return_type_idx,
        parameters_off: 0,
        parameters: None,
        span: SPAN,
    });
    let ctor = method(&mut dex, class.class_idx, proto_idx, "<init>");
    let parent = method(&mut dex, class.superclass_idx, proto_idx, "<init>");
    let clinit = method(&mut dex, class.class_idx, proto_idx, "<clinit>");
    let native = method(&mut dex, class.class_idx, proto_idx, "nativeMutation");
    class.class_data = Some(ClassData {
        static_fields: vec![EncodedField {
            field_idx: field,
            access_flags: 0x19,
        }],
        instance_fields: vec![],
        direct_methods: vec![
            EncodedMethod {
                method_idx: ctor,
                access_flags: 0x10001,
                code_off: 1,
                code: Some(code(&[0x1070, parent as u16, 0, 0x000e], 1)),
            },
            EncodedMethod {
                method_idx: clinit,
                access_flags: 0x10008,
                code_off: 1,
                code: Some(code(&[0x1012, 0x0067, field as u16, 0x000e], 0)),
            },
            EncodedMethod {
                method_idx: native,
                access_flags: 0x109,
                code_off: 0,
                code: None,
            },
        ],
        virtual_methods: vec![],
        span: SPAN,
    });
    dex.classes = vec![class];
    (dex, ctor, clinit, field)
}

struct Resolver<'a>(&'a DexFile);
impl JavaResolver for Resolver<'_> {
    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        // The pool's Object constructor is external but attested in this fixture.
        let raw = self.0.method_id(reference.index()).ok()?;
        Some(JavaMethod {
            owner: self.class(ClassRef::new(u32::from(raw.class_idx)))?,
            name: self.0.string(raw.name_idx).ok()?.into(),
            is_static: self.flags(reference)? & 8 != 0,
            return_type: JavaType::Void,
            parameters: vec![],
            throws: vec![],
        })
    }
    fn field(&self, reference: FieldRef) -> Option<JavaField> {
        crate::dex_java::DexJavaResolver { dex: self.0 }.field(reference)
    }
    fn class(&self, reference: ClassRef) -> Option<String> {
        crate::dex_java::DexJavaResolver { dex: self.0 }.class(reference)
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
impl Resolver<'_> {
    fn flags(&self, reference: MethodRef) -> Option<u32> {
        let raw = self.0.method_id(reference.index()).ok()?;
        if u32::from(raw.class_idx) == self.0.classes[0].superclass_idx {
            return Some(1);
        }
        self.0.classes[0]
            .class_data
            .as_ref()?
            .direct_methods
            .iter()
            .find(|method| method.method_idx == reference.index())
            .map(|method| method.access_flags)
    }
}
impl ConstructorResolver for Resolver<'_> {
    fn initializer(&self, reference: MethodRef) -> Option<Initializer> {
        let method = self.method(reference)?;
        (method.name == "<init>").then_some(Initializer {
            method,
            delegates_to: None,
        })
    }
}
impl TrustedConstructorResolver for Resolver<'_> {
    fn constructor_class(&self, name: &str) -> Option<ConstructorClass> {
        Some(ConstructorClass {
            name: name.into(),
            access_flags: 1,
            superclass: match name {
                "Fixture" => Some("java.lang.Object".into()),
                "java.lang.Object" => None,
                _ => return None,
            },
        })
    }
    fn constructor_access(&self, reference: MethodRef) -> Option<u32> {
        self.flags(reference)
    }
    fn declared_constructors(&self, owner: &str) -> Option<Vec<MethodRef>> {
        (owner == "Fixture").then(|| {
            self.0.classes[0]
                .class_data
                .as_ref()
                .unwrap()
                .direct_methods
                .iter()
                .filter(|method| {
                    self.0
                        .string(self.0.method_ids[method.method_idx as usize].name_idx)
                        .ok()
                        == Some("<init>")
                })
                .map(|method| MethodRef::new(method.method_idx))
                .collect()
        })
    }
}

fn frontend(dex: &DexFile, index: u32) -> dex_decompiler::ssa::SsaMethod {
    let code = dex.classes[0]
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|method| method.method_idx == index)
        .unwrap()
        .code
        .as_ref()
        .unwrap();
    dex_decompiler::DexPasses::run_ssa_with(code, &dex_decompiler::DexTypes::new(dex, Some(index)))
        .unwrap()
}

fn assert_class_plans_equal(owned: &ClassInitializationPlan, shared: &ClassInitializationPlan) {
    assert_eq!(owned.encoded, shared.encoded);
    assert_eq!(owned.static_initializer, shared.static_initializer);
    assert_eq!(owned.constructor_evidence, shared.constructor_evidence);
    assert_eq!(owned.constructors, shared.constructors);
}

#[test]
fn shared_and_public_plans_preserve_complete_source_and_capability_evidence() {
    let (dex, ctor, clinit, field) = fixture();
    let inputs = [dex];
    let dex = &inputs[0];
    let class = &dex.classes[0];
    let original = class.clone();
    let inventory = WriteInventory::scan(&inputs);
    assert!(
        !inventory.has_structural_unknown(),
        "{:?}",
        inventory.unknown_observations()
    );
    let resolver = Resolver(dex);
    let ctor_ssa = frontend(dex, ctor);
    let clinit_ssa = frontend(dex, clinit);
    let bindings = crate::dex_java::parameter_bindings(
        ctor_ssa.ir(),
        &resolver.method(MethodRef::new(ctor)).unwrap(),
        1,
        1,
    )
    .unwrap();
    let constructors = [ConstructorInput {
        reference: MethodRef::new(ctor),
        ir: ctor_ssa.ir(),
        bindings: &bindings,
    }];
    let clinit_input = Some((MethodRef::new(clinit), clinit_ssa.ir()));
    let owned = plan_class_initialization_with_explicit_stores(
        &inputs,
        0,
        class,
        &constructors,
        clinit_input,
        &resolver,
        &inventory,
    )
    .unwrap();
    let shared = plan_class_initialization_shared_explicit_stores(
        &inputs,
        0,
        class,
        &constructors,
        clinit_input,
        &resolver,
        &inventory,
    )
    .unwrap();
    assert_class_plans_equal(&owned.plan, &shared.plan);
    assert_eq!(
        owned.runtime_capabilities.as_slice(),
        shared.runtime_capabilities.as_ref()
    );
    let static_owned = plan_static_initializer_explicit_stores(
        &inputs,
        0,
        class,
        MethodRef::new(clinit),
        clinit_ssa.ir(),
        &resolver,
        &inventory,
    )
    .unwrap();
    let static_shared = plan_static_initializer_shared_explicit_stores(
        &inputs,
        0,
        class,
        MethodRef::new(clinit),
        clinit_ssa.ir(),
        &resolver,
        &inventory,
    )
    .unwrap();
    assert_eq!(static_owned.plan.encoded, static_shared.plan.encoded);
    assert_eq!(static_owned.plan.source, static_shared.plan.source);
    assert_eq!(
        static_owned.runtime_capabilities.as_slice(),
        static_shared.runtime_capabilities.as_ref()
    );
    assert!(Arc::ptr_eq(
        &shared.runtime_capabilities,
        &static_shared.runtime_capabilities
    ));
    assert!(Arc::ptr_eq(
        &shared.runtime_capabilities,
        &inventory.runtime_capabilities_shared()
    ));
    assert_eq!(shared.runtime_capabilities.len(), 1);
    assert!(shared.plan.encoded.blank_static_finals.contains(&field));
    assert_eq!(class, &original);
    assert_eq!(
        class.class_data.as_ref().unwrap().static_fields[0].access_flags,
        0x19
    );
}

#[test]
fn shared_routes_keep_structural_coverage_and_final_store_rejections() {
    let (dex, ctor, clinit, field) = fixture();
    let inputs = [dex];
    let dex = &inputs[0];
    let class = &dex.classes[0];
    let inventory = WriteInventory::scan(&inputs);
    let resolver = Resolver(dex);
    let ctor_ssa = frontend(dex, ctor);
    let clinit_ssa = frontend(dex, clinit);
    let bindings = crate::dex_java::parameter_bindings(
        ctor_ssa.ir(),
        &resolver.method(MethodRef::new(ctor)).unwrap(),
        1,
        1,
    )
    .unwrap();
    let constructors = [ConstructorInput {
        reference: MethodRef::new(ctor),
        ir: ctor_ssa.ir(),
        bindings: &bindings,
    }];
    let clinit_input = Some((MethodRef::new(clinit), clinit_ssa.ir()));
    for (constructors, clinit) in [
        (&constructors[..0], clinit_input),
        (&constructors[..], None),
    ] {
        let owned = plan_class_initialization_with_explicit_stores(
            &inputs,
            0,
            class,
            constructors,
            clinit,
            &resolver,
            &inventory,
        )
        .err();
        let shared = plan_class_initialization_shared_explicit_stores(
            &inputs,
            0,
            class,
            constructors,
            clinit,
            &resolver,
            &inventory,
        )
        .err();
        assert!(owned.is_some());
        assert_eq!(owned, shared);
    }
    let mut structural = inventory.clone();
    structural.record_unknown(UnknownKind::Structural, "incomplete original input".into());
    let mut conflicting = inventory.clone();
    conflicting
        .writes
        .entry(field_key(dex, field).unwrap())
        .or_default()
        .insert(WriteSite {
            dex: 0,
            method: ctor,
            offset: 99,
            is_static: true,
        });
    for inventory in [&structural, &conflicting] {
        assert_eq!(
            plan_class_initialization_with_explicit_stores(
                &inputs,
                0,
                class,
                &constructors,
                clinit_input,
                &resolver,
                inventory
            )
            .err(),
            Some(Error::FieldInitializationUnavailable)
        );
        assert_eq!(
            plan_class_initialization_shared_explicit_stores(
                &inputs,
                0,
                class,
                &constructors,
                clinit_input,
                &resolver,
                inventory
            )
            .err(),
            Some(Error::FieldInitializationUnavailable)
        );
        assert_eq!(
            plan_static_initializer_explicit_stores(
                &inputs,
                0,
                class,
                MethodRef::new(clinit),
                clinit_ssa.ir(),
                &resolver,
                inventory
            )
            .err(),
            Some(Error::FieldInitializationUnavailable)
        );
        assert_eq!(
            plan_static_initializer_shared_explicit_stores(
                &inputs,
                0,
                class,
                MethodRef::new(clinit),
                clinit_ssa.ir(),
                &resolver,
                inventory
            )
            .err(),
            Some(Error::FieldInitializationUnavailable)
        );
    }
}

#[test]
fn enum_access_evidence_matches_original_scan_and_rescans_other_storage() {
    // Includes the whole body of a class that an exporter could later drop.
    // A return before an access exercises unreachable original bytecode.
    let cases: &[(&str, &[u16], u32, bool)] = &[
        ("clean_with_native", &[0x000e], 0x109, true),
        ("abstract", &[0x000e], 0x409, true),
        ("missing_body", &[0x000e], 9, false),
        ("read", &[0x0060, 0, 0x000e], 0x109, false),
        (
            "unreachable_read",
            &[0x000e, 0x0060, 0, 0x000e],
            0x109,
            false,
        ),
        ("store", &[0x0067, 0, 0x000e], 0x109, false),
        ("unresolved_read", &[0x0060, 0xffff, 0x000e], 0x109, false),
        ("malformed", &[0x0060], 0x109, false),
    ];
    for &(name, units, flags, expected) in cases {
        let (mut dex, _, _, field) = fixture();
        let mut units = units.to_vec();
        for index in 0..units.len().saturating_sub(1) {
            if (units[index] == 0x0060 || units[index] == 0x0067) && units[index + 1] != 0xffff {
                units[index + 1] = field as u16;
            }
        }
        let members = dex.classes[0].class_data.as_mut().unwrap();
        members.direct_methods[1].code = Some(code(&units, 0));
        members.direct_methods[2].access_flags = flags;
        let fields = BTreeSet::from([field_key(&dex, field).unwrap()]);
        let inputs = vec![dex];
        let inventory = WriteInventory::scan(&inputs);
        assert_eq!(
            enum_fields_unaccessed_original(&inputs, &fields),
            expected,
            "{name}"
        );
        assert_eq!(
            inventory.enum_fields_unaccessed(&inputs, &fields),
            expected,
            "{name}"
        );
        assert_eq!(
            inventory.enum_fields_unaccessed(&inputs, &fields),
            expected,
            "repeat {name}"
        );
        let other = inputs.clone();
        assert_eq!(
            inventory.enum_fields_unaccessed(&other, &fields),
            expected,
            "other storage {name}"
        );
    }
    let (mut dex, _, _, field) = fixture();
    dex.classes[0].class_data.as_mut().unwrap().direct_methods[1].code = Some(code(&[0x000e], 0));
    let fields = BTreeSet::from([field_key(&dex, field).unwrap()]);
    let inputs = vec![dex];
    let inventory = WriteInventory::scan(&inputs);
    assert!(inventory.enum_fields_unaccessed(&inputs, &fields));
    let mut changed = inputs.clone();
    changed[0].classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods[1]
        .code = Some(code(&[0x0060, field as u16, 0x000e], 0));
    assert!(!inventory.enum_fields_unaccessed(&changed, &fields));
}
