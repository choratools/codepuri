//! Narrow declaration-only uncertainty without discarding arbitrary writers.
#![allow(clippy::unwrap_used)]
use decomp::java_class_kinds::initializers::write_inventory::{WriteInventory, field_key};
use dex_format::{
    DexFile, RawSpan,
    class::{ClassData, EncodedField, EncodedMethod},
    ids::{FieldId, MethodId, ProtoId, TypeId},
};
const SPAN: RawSpan = RawSpan { offset: 0, size: 0 };
fn string(d: &mut DexFile, text: &str) -> u32 {
    let mut s = d.strings[0].clone();
    s.value = text.into();
    let index = d.strings.len() as u32;
    d.strings.push(s);
    index
}
fn ty(d: &mut DexFile, text: &str) -> u32 {
    let descriptor_idx = string(d, text);
    let index = d.type_ids.len() as u32;
    d.type_ids.push(TypeId {
        descriptor_idx,
        span: SPAN,
    });
    index
}
fn fixture() -> (DexFile, [u32; 3]) {
    let mut d = DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let prototype = d.classes[0].clone();
    d.classes.clear();
    let int = ty(&mut d, "I");
    let void = ty(&mut d, "V");
    let name = string(&mut d, "value");
    let mut fields = [0; 3];
    let mut owners = vec![];
    for (index, owner) in ["LBase;", "LUnrelated;", "LChild;"].iter().enumerate() {
        let class_idx = ty(&mut d, owner);
        owners.push(class_idx);
        let field = d.field_ids.len() as u32;
        fields[index] = field;
        d.field_ids.push(FieldId {
            class_idx: class_idx as u16,
            type_idx: int as u16,
            name_idx: name,
            span: SPAN,
        });
        let mut class = prototype.clone();
        class.class_idx = class_idx;
        class.superclass_idx = if index == 2 { owners[0] } else { u32::MAX };
        class.interfaces = None;
        class.interfaces_off = 0;
        class.static_values = None;
        class.static_values_off = 0;
        class.annotations = None;
        class.class_data = Some(ClassData {
            instance_fields: if index == 2 {
                vec![]
            } else {
                vec![EncodedField {
                    field_idx: field,
                    access_flags: 0x11,
                }]
            },
            static_fields: vec![],
            direct_methods: vec![],
            virtual_methods: vec![],
            span: SPAN,
        });
        d.classes.push(class);
    }
    let proto_idx = d.proto_ids.len() as u16;
    d.proto_ids.push(ProtoId {
        shorty_idx: 0,
        return_type_idx: void,
        parameters: None,
        parameters_off: 0,
        span: SPAN,
    });
    let name_idx = string(&mut d, "writer");
    let method_idx = d.method_ids.len() as u32;
    d.method_ids.push(MethodId {
        class_idx: owners[2] as u16,
        proto_idx,
        name_idx,
        span: SPAN,
    });
    let units = [0x1059u16, fields[2] as u16, 0x000e];
    d.classes[2]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(EncodedMethod {
            method_idx,
            access_flags: 1,
            code_off: 1,
            code: Some(dex_format::code::CodeItem {
                registers_size: 2,
                ins_size: 0,
                outs_size: 0,
                debug_info_off: 0,
                insns_size: units.len() as u32,
                insns: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
                tries: vec![],
                handlers: vec![],
                span: SPAN,
            }),
        });
    (d, fields)
}

#[test]
fn genuine_inherited_operand_resolves_one_owner_not_all_same_signature_fields() {
    let (d, fields) = fixture();
    let inventory = WriteInventory::scan(std::slice::from_ref(&d));
    assert!(inventory.unknown_reasons().is_empty());
    let sites = inventory
        .proven_writes(&field_key(&d, fields[0]).unwrap())
        .unwrap();
    assert_eq!(sites.len(), 1);
    assert_eq!(sites.first().unwrap().offset, 0);
    assert!(
        inventory
            .proven_writes(&field_key(&d, fields[1]).unwrap())
            .unwrap()
            .is_empty()
    );
    assert!(!inventory.declares_field(&field_key(&d, fields[2]).unwrap()));
    assert!(inventory.scoped_unknowns().is_empty());
}

#[test]
fn missing_hierarchy_keeps_signature_uncertainty() {
    let (mut d, fields) = fixture();
    d.classes[2].superclass_idx = ty(&mut d, "LMissing;");
    let inventory = WriteInventory::scan(std::slice::from_ref(&d));
    for field in &fields[..2] {
        assert!(
            inventory
                .proven_writes(&field_key(&d, *field).unwrap())
                .is_none()
        );
    }
}

#[test]
fn bad_field_flags_are_local_but_native_body_still_is_arbitrary() {
    let (mut d, fields) = fixture();
    d.classes[0].class_data.as_mut().unwrap().instance_fields[0].access_flags |= 0x40;
    let inventory = WriteInventory::scan(std::slice::from_ref(&d));
    assert!(inventory.unknown_reasons().is_empty());
    assert!(
        inventory
            .proven_writes(&field_key(&d, fields[0]).unwrap())
            .is_none()
    );
    assert!(
        inventory
            .proven_writes(&field_key(&d, fields[1]).unwrap())
            .is_some()
    );
    let method = &mut d.classes[2].class_data.as_mut().unwrap().direct_methods[0];
    method.code = None;
    method.code_off = 0;
    method.access_flags = 0x101;
    let native = WriteInventory::scan(std::slice::from_ref(&d));
    assert!(
        native
            .unknown_reasons()
            .iter()
            .any(|r| r.contains("native"))
    );
    for field in &fields[..2] {
        assert!(
            native
                .proven_writes(&field_key(&d, *field).unwrap())
                .is_none()
        );
    }
}

#[test]
fn same_owner_staticness_mismatch_must_not_fall_through_to_ancestor() {
    let (mut d, fields) = fixture();
    d.classes[2]
        .class_data
        .as_mut()
        .unwrap()
        .static_fields
        .push(EncodedField {
            field_idx: fields[2],
            access_flags: 0x19,
        });
    let inventory = WriteInventory::scan(std::slice::from_ref(&d));
    assert!(
        inventory
            .known_writes(&field_key(&d, fields[0]).unwrap())
            .is_none()
    );
    assert!(
        inventory
            .proven_writes(&field_key(&d, fields[0]).unwrap())
            .is_none()
    );
}
