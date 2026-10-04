//! Complete declaration evidence narrows symbolic stores, never runtime capabilities.
#![allow(clippy::unwrap_used)]
use decomp::java_aliases::{FieldKey as MetadataKey, MethodKey};
use decomp::java_class_kinds::initializers::write_inventory::{
    self, ExplicitStoreError, UnknownKind, WriteInventory,
};
use decomp::java_external_metadata::{ExternalMetadata, FieldMetadata, MethodMetadata};
use decomp::type_compatibility::Declaration;
use dex_format::{
    DexFile, RawSpan,
    class::{ClassData, ClassDef, EncodedField, EncodedMethod},
    ids::{FieldId, MethodId, ProtoId, TypeId},
};
use std::collections::BTreeMap;
const SPAN: RawSpan = RawSpan { offset: 0, size: 0 };
fn string(dex: &mut DexFile, text: &str) -> u32 {
    let mut s = dex.strings[0].clone();
    s.value = text.into();
    let index = dex.strings.len() as u32;
    dex.strings.push(s);
    index
}
fn ty(dex: &mut DexFile, text: &str) -> u32 {
    let descriptor_idx = string(dex, text);
    let index = dex.type_ids.len() as u32;
    dex.type_ids.push(TypeId {
        descriptor_idx,
        span: SPAN,
    });
    index
}
fn fixture(flags: u32) -> (DexFile, ClassDef) {
    let mut dex = DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let mut class = dex.classes[0].clone();
    class.class_idx = ty(&mut dex, "LFixture;");
    class.access_flags = flags;
    class.superclass_idx = ty(&mut dex, "Ljava/lang/Object;");
    class.interfaces = None;
    class.annotations = None;
    class.static_values = None;
    class.class_data = Some(ClassData {
        static_fields: vec![],
        instance_fields: vec![],
        direct_methods: vec![],
        virtual_methods: vec![],
        span: SPAN,
    });
    (dex, class)
}
fn element(dex: &mut DexFile, class: &mut ClassDef, name: &str, descriptor: &str) -> EncodedMethod {
    let return_type_idx = ty(dex, descriptor);
    let proto_idx = dex.proto_ids.len() as u16;
    dex.proto_ids.push(ProtoId {
        shorty_idx: 0,
        return_type_idx,
        parameters_off: 0,
        parameters: None,
        span: SPAN,
    });
    let name_idx = string(dex, name);
    let method_idx = dex.method_ids.len() as u32;
    dex.method_ids.push(MethodId {
        class_idx: class.class_idx as u16,
        proto_idx,
        name_idx,
        span: SPAN,
    });
    let method = EncodedMethod {
        method_idx,
        access_flags: 0x401,
        code_off: 0,
        code: None,
    };
    class
        .class_data
        .as_mut()
        .unwrap()
        .virtual_methods
        .push(method.clone());
    method
}
fn field(dex: &mut DexFile, class: &mut ClassDef, name: &str, descriptor: &str, flags: u32) -> u32 {
    let type_idx = ty(dex, descriptor) as u16;
    let name_idx = string(dex, name);
    let index = dex.field_ids.len() as u32;
    dex.field_ids.push(FieldId {
        class_idx: class.class_idx as u16,
        type_idx,
        name_idx,
        span: SPAN,
    });
    let f = EncodedField {
        field_idx: index,
        access_flags: flags,
    };
    if flags & 8 != 0 {
        class.class_data.as_mut().unwrap().static_fields.push(f);
    } else {
        class.class_data.as_mut().unwrap().instance_fields.push(f);
    }
    index
}
fn code(units: &[u16]) -> dex_format::code::CodeItem {
    dex_format::code::CodeItem {
        registers_size: 4,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: SPAN,
    }
}

#[derive(Default)]
struct Metadata {
    fields: BTreeMap<String, Vec<FieldMetadata>>,
    nodes: BTreeMap<String, Vec<Declaration>>,
}
impl ExternalMetadata for Metadata {
    fn method(&self, _: &MethodKey) -> Option<MethodMetadata> {
        None
    }
    fn field(&self, key: &MetadataKey) -> Option<FieldMetadata> {
        self.fields
            .get(&key.owner)?
            .iter()
            .find(|f| f.name == key.name && f.descriptor == key.descriptor)
            .cloned()
    }
    fn declared_fields(&self, owner: &str) -> Option<Vec<FieldMetadata>> {
        self.fields.get(owner).cloned()
    }
    fn class_hierarchy(&self, owner: &str) -> Vec<Declaration> {
        self.nodes.get(owner).cloned().unwrap_or_default()
    }
    fn class_access(&self, owner: &str) -> Option<u32> {
        self.nodes.contains_key(owner).then_some(1)
    }
}
fn declaration(owner: &str, parent: Option<&str>, interfaces: &[&str]) -> Declaration {
    let name = |n: &str| {
        n.trim_start_matches('L')
            .trim_end_matches(';')
            .replace('/', ".")
    };
    Declaration {
        is_interface: false,
        name: name(owner),
        superclass: parent.map(name),
        interfaces: interfaces.iter().map(|i| name(i)).collect(),
    }
}
fn external_field(owner: &str, flags: u32) -> FieldMetadata {
    FieldMetadata {
        owner: owner.into(),
        name: "value".into(),
        descriptor: "I".into(),
        access_flags: flags,
        signature: None,
        constant_value: None,
        attributes: vec![],
    }
}
fn metadata(owner: &str, flags: u32) -> Metadata {
    Metadata {
        fields: [(owner.into(), vec![external_field(owner, flags)])].into(),
        nodes: [(owner.into(), vec![declaration(owner, None, &[])])].into(),
    }
}
fn input(
    symbolic: &str,
    static_store: bool,
) -> (
    DexFile,
    write_inventory::FieldKey,
    write_inventory::FieldKey,
    u32,
) {
    let (mut dex, mut class) = fixture(1);
    let own = field(&mut dex, &mut class, "value", "I", 0x11);
    let own_key = write_inventory::field_key(&dex, own).unwrap();
    let owner = ty(&mut dex, symbolic);
    let int_type = ty(&mut dex, "I");
    let name_idx = string(&mut dex, "value");
    let external = dex.field_ids.len() as u32;
    dex.field_ids.push(FieldId {
        class_idx: owner as u16,
        type_idx: int_type as u16,
        name_idx,
        span: SPAN,
    });
    let external_key = write_inventory::field_key(&dex, external).unwrap();
    let mut writer = element(&mut dex, &mut class, "writer", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    writer.access_flags = 9;
    writer.code_off = 1;
    writer.code = Some(code(&[
        0x0012,
        if static_store { 0x0067 } else { 0x1059 },
        external as u16,
        0x000e,
    ]));
    let writer_index = writer.method_idx;
    class
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(writer);
    // Both genuine capabilities stay global, independently of symbolic scoping.
    let mut native = element(&mut dex, &mut class, "nativeChange", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    native.access_flags = 0x109;
    class
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(native);
    let reflection_owner = ty(&mut dex, "Ljava/lang/reflect/Field;");
    let target = element(&mut dex, &mut class, "set", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    dex.method_ids[target.method_idx as usize].class_idx = reflection_owner as u16;
    let mut reflect = element(&mut dex, &mut class, "reflect", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    reflect.access_flags = 9;
    reflect.code_off = 1;
    reflect.code = Some(code(&[0x106e, target.method_idx as u16, 0, 0x000e]));
    class
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(reflect);
    dex.classes = vec![class];
    (dex, own_key, external_key, writer_index)
}
#[test]
fn exact_external_declaration_narrows_only_the_symbolic_target() {
    let (dex, own, external, writer) = input("Lplatform/Base;", false);
    let old = WriteInventory::scan(std::slice::from_ref(&dex));
    assert!(matches!(
        old.explicit_store_writes(&own),
        Err(ExplicitStoreError::SymbolicFieldUnknown(_))
    ));
    let proof = WriteInventory::scan_with_metadata(
        std::slice::from_ref(&dex),
        &metadata(&external.owner, 1),
    );
    assert!(proof.explicit_store_writes(&own).unwrap().is_empty());
    let sites = proof.known_writes(&external).unwrap();
    assert_eq!(sites.len(), 1);
    assert_eq!(sites.first().unwrap().method, writer);
    assert!(proof.proven_writes(&own).is_none());
    assert_eq!(old.unknown_observations(), proof.unknown_observations());
    assert_eq!(proof.unknown_observations().len(), 2);
    assert!(
        proof
            .unknown_observations()
            .iter()
            .any(|o| matches!(o.kind, UnknownKind::NativeCapability { .. }))
    );
    assert!(
        proof
            .unknown_observations()
            .iter()
            .any(|o| matches!(o.kind, UnknownKind::ResolvedReflectiveCapability { .. }))
    );
}
#[test]
fn missing_incomplete_staticness_and_ambiguous_external_evidence_remain_unknown() {
    let (dex, own, external, _) = input("Lplatform/Base;", false);
    let complete = metadata(&external.owner, 1);
    let mut incomplete = metadata(&external.owner, 1);
    incomplete.fields.remove(&external.owner);
    let mut missing_hierarchy = metadata(&external.owner, 1);
    missing_hierarchy.nodes.clear();
    let mut duplicate_fields = metadata(&external.owner, 1);
    duplicate_fields
        .fields
        .get_mut(&external.owner)
        .unwrap()
        .push(external_field(&external.owner, 1));
    let mut ambiguous_hierarchy = metadata(&external.owner, 1);
    ambiguous_hierarchy
        .nodes
        .get_mut(&external.owner)
        .unwrap()
        .push(declaration(&external.owner, None, &[]));
    for provider in [
        Metadata::default(),
        incomplete,
        missing_hierarchy,
        duplicate_fields,
        ambiguous_hierarchy,
        metadata(&external.owner, 9),
    ] {
        let proof = WriteInventory::scan_with_metadata(std::slice::from_ref(&dex), &provider);
        assert!(matches!(
            proof.explicit_store_writes(&own),
            Err(ExplicitStoreError::SymbolicFieldUnknown(_))
        ));
    }
    struct ExactOnly(Metadata);
    impl ExternalMetadata for ExactOnly {
        fn method(&self, _: &MethodKey) -> Option<MethodMetadata> {
            None
        }
        fn field(&self, k: &MetadataKey) -> Option<FieldMetadata> {
            self.0.field(k)
        }
        fn class_hierarchy(&self, o: &str) -> Vec<Declaration> {
            self.0.class_hierarchy(o)
        }
        fn class_access(&self, o: &str) -> Option<u32> {
            self.0.class_access(o)
        }
    }
    let proof =
        WriteInventory::scan_with_metadata(std::slice::from_ref(&dex), &ExactOnly(complete));
    assert!(proof.explicit_store_writes(&own).is_err());

    struct Contradictory(Metadata);
    impl ExternalMetadata for Contradictory {
        fn method(&self, _: &MethodKey) -> Option<MethodMetadata> {
            None
        }
        fn field(&self, key: &MetadataKey) -> Option<FieldMetadata> {
            self.0.field(key)
        }
        fn declared_fields(&self, _: &str) -> Option<Vec<FieldMetadata>> {
            Some(vec![])
        }
        fn class_hierarchy(&self, owner: &str) -> Vec<Declaration> {
            self.0.class_hierarchy(owner)
        }
        fn class_access(&self, owner: &str) -> Option<u32> {
            self.0.class_access(owner)
        }
    }
    let proof = WriteInventory::scan_with_metadata(
        std::slice::from_ref(&dex),
        &Contradictory(metadata(&external.owner, 1)),
    );
    assert!(proof.explicit_store_writes(&own).is_err());
}
#[test]
fn inherited_and_interface_targets_are_canonical_or_stay_ambiguous() {
    let (dex, own, symbolic, writer) = input("Lplatform/Child;", false);
    let mut provider = metadata("Lplatform/Base;", 1);
    provider.fields.insert(symbolic.owner.clone(), vec![]);
    provider.nodes.insert(
        symbolic.owner.clone(),
        vec![declaration(&symbolic.owner, Some("Lplatform/Base;"), &[])],
    );
    let proof = WriteInventory::scan_with_metadata(std::slice::from_ref(&dex), &provider);
    assert!(proof.explicit_store_writes(&own).unwrap().is_empty());
    let canonical = write_inventory::FieldKey {
        owner: "Lplatform/Base;".into(),
        ..symbolic.clone()
    };
    assert_eq!(
        proof
            .known_writes(&canonical)
            .unwrap()
            .first()
            .unwrap()
            .method,
        writer
    );
    assert!(proof.known_writes(&symbolic).is_none());
    provider.fields.remove(&symbolic.owner);
    assert!(
        WriteInventory::scan_with_metadata(std::slice::from_ref(&dex), &provider)
            .explicit_store_writes(&own)
            .is_err()
    );

    let (dex, own, symbolic, _) = input("Lplatform/Both;", true);
    let mut provider = metadata("Lplatform/Left;", 9);
    provider.fields.insert(
        "Lplatform/Right;".into(),
        vec![external_field("Lplatform/Right;", 9)],
    );
    provider.nodes.insert(
        "Lplatform/Right;".into(),
        vec![declaration("Lplatform/Right;", None, &[])],
    );
    provider.fields.insert(symbolic.owner.clone(), vec![]);
    provider.nodes.insert(
        symbolic.owner.clone(),
        vec![declaration(
            &symbolic.owner,
            None,
            &["Lplatform/Left;", "Lplatform/Right;"],
        )],
    );
    assert!(
        WriteInventory::scan_with_metadata(std::slice::from_ref(&dex), &provider)
            .explicit_store_writes(&own)
            .is_err()
    );
}
#[test]
fn inherited_input_writes_remain_real_conflicts_and_input_owner_shadows_classpath() {
    let (dex, own, symbolic, writer) = input("Lplatform/Child;", false);
    let mut provider = Metadata::default();
    provider.fields.insert(symbolic.owner.clone(), vec![]);
    provider.nodes.insert(
        symbolic.owner.clone(),
        vec![declaration(&symbolic.owner, Some(&own.owner), &[])],
    );
    let proof = WriteInventory::scan_with_metadata(std::slice::from_ref(&dex), &provider);
    let writes = proof.explicit_store_writes(&own).unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes.first().unwrap().method, writer);
    let mut shadowed = dex.clone();
    let mut class = shadowed.classes[0].clone();
    class.class_idx = ty(&mut shadowed, &symbolic.owner);
    class.class_data = Some(ClassData {
        static_fields: vec![],
        instance_fields: vec![],
        direct_methods: vec![],
        virtual_methods: vec![],
        span: SPAN,
    });
    shadowed.classes.push(class);
    let provider = metadata(&symbolic.owner, 1);
    assert!(
        WriteInventory::scan_with_metadata(std::slice::from_ref(&shadowed), &provider)
            .explicit_store_writes(&own)
            .is_err()
    );
}

#[test]
fn resolving_one_store_keeps_another_uncertain_same_signature_store() {
    let (mut dex, own, external, writer) = input("Lplatform/Base;", false);
    let missing_owner = ty(&mut dex, "Lplatform/Missing;");
    let int_type = ty(&mut dex, "I");
    let name_idx = string(&mut dex, "value");
    let missing = dex.field_ids.len() as u16;
    dex.field_ids.push(FieldId {
        class_idx: missing_owner as u16,
        type_idx: int_type as u16,
        name_idx,
        span: SPAN,
    });
    dex.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .iter_mut()
        .find(|m| m.method_idx == writer)
        .unwrap()
        .code = Some(code(&[
        0x0012,
        0x1059,
        (missing - 1),
        0x1059,
        missing,
        0x000e,
    ]));
    let scoped = WriteInventory::scan_with_metadata(
        std::slice::from_ref(&dex),
        &metadata(&external.owner, 1),
    );
    assert!(scoped.explicit_store_writes(&own).is_err());
    assert_eq!(scoped.known_writes(&external).unwrap().len(), 1);
    assert_eq!(scoped.scoped_unknowns().get(&own).unwrap().len(), 1);
}
