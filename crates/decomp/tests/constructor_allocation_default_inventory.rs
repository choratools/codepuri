//! Explicit DEX stores and runtime capability evidence are independent contracts.
#![allow(clippy::unwrap_used)]
use decomp::java_class_kinds::initializers::*;
use dex_format::{
    DexFile, RawSpan,
    class::{ClassData, ClassDef, EncodedField, EncodedMethod},
    ids::{FieldId, MethodId, ProtoId, TypeId},
};
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

fn native(dex: &mut DexFile, class: &mut ClassDef) {
    let mut m = element(dex, class, "nativeMutation", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    m.access_flags = 0x109;
    class.class_data.as_mut().unwrap().direct_methods.push(m);
}

#[test]
fn allocation_default_inventory_does_not_erase_native_runtime_uncertainty() {
    let (mut dex, mut class) = fixture(1);
    let field_index = field(&mut dex, &mut class, "defaultFlag", "Z", 0x11);
    let mut constructor = element(&mut dex, &mut class, "<init>", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    let parent_index = dex.method_ids.len() as u32;
    let mut parent = dex.method_ids[constructor.method_idx as usize].clone();
    parent.class_idx = class.superclass_idx as u16;
    dex.method_ids.push(parent);
    constructor.access_flags = 0x10001;
    constructor.code_off = 1;
    let mut body = code(&[0x1070, parent_index as u16, 0, 0x000e]);
    body.registers_size = 1;
    body.ins_size = 1;
    body.outs_size = 1;
    constructor.code = Some(body);
    let reference = constructor.method_idx;
    class
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(constructor);
    native(&mut dex, &mut class);
    dex.classes = vec![class.clone()];
    let inputs = [dex];
    let inventory = write_inventory::WriteInventory::scan(&inputs);
    let explicit =
        prove_instance_final_explicit_stores(&inputs, 0, &class, &[reference].into(), &inventory)
            .unwrap();
    assert_eq!(explicit.plan.len(), 1);
    assert_eq!(explicit.plan[0].reference.index(), field_index);
    assert!(explicit.plan[0].writes.is_empty());
    assert_eq!(explicit.runtime_capabilities.len(), 1);
    assert!(
        prove_instance_final_writes(&inputs, 0, &class, &[reference].into(), &inventory).is_err()
    );
    // This is only inventory evidence. Full constructor-family/backend proof
    // remains separately mandatory; no source or runtime-success token exists.
}
