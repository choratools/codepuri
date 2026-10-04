//! Narrow declaration-only uncertainty without discarding arbitrary writers.
#![allow(clippy::unwrap_used)]
use decomp::java_class_kinds::initializers::write_inventory::{
    ExplicitStoreError, UnknownKind, WriteInventory, field_key,
};
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

fn native(d: &mut DexFile) {
    let original = d.classes[2].class_data.as_ref().unwrap().direct_methods[0].clone();
    let mut id = d.method_ids[original.method_idx as usize].clone();
    id.name_idx = string(d, "nativeMutation");
    let method_idx = d.method_ids.len() as u32;
    d.method_ids.push(id);
    d.classes[2]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(EncodedMethod {
            method_idx,
            access_flags: 0x101,
            code_off: 0,
            code: None,
        });
}
#[test]
fn capability_only_native_retains_strict_failure_and_complete_explicit_writes() {
    let (mut d, fields) = fixture();
    native(&mut d);
    let inv = WriteInventory::scan(std::slice::from_ref(&d));
    let key = field_key(&d, fields[0]).unwrap();
    assert!(inv.proven_writes(&key).is_none());
    let stores = inv.explicit_store_writes(&key).unwrap();
    assert_eq!(stores.len(), 1);
    assert_eq!(stores.first().unwrap().offset, 0);
    assert!(
        inv.unknown_observations()
            .iter()
            .any(|o| matches!(o.kind, UnknownKind::NativeCapability { .. }))
    );
    // Same diagnostic text cannot turn a missing nonnative body into capability.
    let m = d.classes[2]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .last_mut()
        .unwrap();
    m.access_flags = 1;
    let bad = WriteInventory::scan(std::slice::from_ref(&d));
    assert!(matches!(
        bad.explicit_store_writes(&key),
        Err(ExplicitStoreError::StructuralUnknown(_))
    ));
    mangle_native_abstract(&mut d);
    assert!(
        WriteInventory::scan(&[d])
            .explicit_store_writes(&key)
            .is_err()
    );
}
fn mangle_native_abstract(d: &mut DexFile) {
    d.classes[2]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .last_mut()
        .unwrap()
        .access_flags = 0x501;
}
#[test]
fn resolved_reflection_is_capability_but_unknown_target_and_decode_failures_are_structural() {
    let (mut d, fields) = fixture();
    let reflect = ty(&mut d, "Ljava/lang/reflect/Field;");
    let name_idx = string(&mut d, "setInt");
    let proto_idx = d.method_ids.last().unwrap().proto_idx;
    let target = d.method_ids.len() as u16;
    d.method_ids.push(MethodId {
        class_idx: reflect as u16,
        proto_idx,
        name_idx,
        span: SPAN,
    });
    let code = d.classes[2].class_data.as_mut().unwrap().direct_methods[0]
        .code
        .as_mut()
        .unwrap();
    let units = [0x0071u16, target, 0, 0x1059, fields[2] as u16, 0x000e];
    code.insns = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    code.insns_size = units.len() as u32;
    let key = field_key(&d, fields[0]).unwrap();
    let inv = WriteInventory::scan(std::slice::from_ref(&d));
    assert_eq!(inv.explicit_store_writes(&key).unwrap().len(), 1);
    assert!(inv.proven_writes(&key).is_none());
    assert!(matches!(
        inv.unknown_observations()[0].kind,
        UnknownKind::ResolvedReflectiveCapability { offset: 0, .. }
    ));
    let code = d.classes[2].class_data.as_mut().unwrap().direct_methods[0]
        .code
        .as_mut()
        .unwrap();
    code.insns[2] = 255;
    code.insns[3] = 255;
    assert!(matches!(
        WriteInventory::scan(std::slice::from_ref(&d)).explicit_store_writes(&key),
        Err(ExplicitStoreError::StructuralUnknown(_))
    ));
    let code = d.classes[2].class_data.as_mut().unwrap().direct_methods[0]
        .code
        .as_mut()
        .unwrap();
    code.insns = vec![255, 255];
    code.insns_size = 1;
    assert!(
        WriteInventory::scan(&[d])
            .explicit_store_writes(&key)
            .is_err()
    );
}
#[test]
fn cross_dex_unreachable_original_stores_are_not_dropped() {
    let (mut first, fields) = fixture();
    let (mut second, _) = fixture();
    first.classes.truncate(1);
    second.classes = vec![second.classes[2].clone()];
    let code = second.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods[0]
        .code
        .as_mut()
        .unwrap();
    let units = [0x000eu16, 0x1059, fields[2] as u16, 0x000e];
    code.insns = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    code.insns_size = units.len() as u32;
    let key = field_key(&first, fields[0]).unwrap();
    let inv = WriteInventory::scan(&[first, second]);
    let writes = inv.explicit_store_writes(&key).unwrap();
    assert_eq!(writes.len(), 1);
    let site = writes.first().unwrap();
    assert_eq!((site.dex, site.offset), (1, 1));
}
#[test]
fn blank_final_reflection_preserves_actual_field_read() {
    let dir = std::env::temp_dir().join(format!("explicit-final-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"),"public class Fixture { final int value; Fixture(){value=1;} int read(){return value;} public static void main(String[]args)throws Exception { Fixture f=new Fixture();var field=Fixture.class.getDeclaredField(\"value\");field.setAccessible(true);field.setInt(f,2);if(f.read()!=2)throw new AssertionError(); }}").unwrap();
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (tool, args) in [
        ("javac", vec!["--release", "17", "Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let o = std::process::Command::new(format!("{home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    let o = std::process::Command::new(format!("{home}/bin/javap"))
        .args(["-c", "-p", "Fixture"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(o.status.success());
    assert!(String::from_utf8(o.stdout).unwrap().contains("getfield"));
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
#[ignore = "bounded real APK inventory probe"]
fn samsung_final_constructor_explicit_store_probe() {
    let inputs = dex_format::apk::parse_apk_path(std::path::Path::new(
        "../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    ))
    .unwrap();
    let inv = WriteInventory::scan(&inputs);
    let mut native = 0;
    let mut reflective = 0;
    let mut structural = 0;
    for o in inv.unknown_observations() {
        match o.kind {
            UnknownKind::NativeCapability { .. } => native += 1,
            UnknownKind::ResolvedReflectiveCapability { .. } => reflective += 1,
            UnknownKind::Structural => {
                structural += 1;
                println!("STRUCTURAL {}", o.reason);
            }
        }
    }
    println!("UNKNOWN native={native} reflective={reflective} structural={structural}");
    let dex = &inputs[0];
    let class = dex
        .classes
        .iter()
        .find(|c| {
            dex.string(dex.type_ids[c.class_idx as usize].descriptor_idx)
                .unwrap()
                == "La4/a$c;"
        })
        .unwrap();
    for field in &class.class_data.as_ref().unwrap().instance_fields {
        let key = field_key(dex, field.field_idx).unwrap();
        let known = inv.known_writes(&key).unwrap();
        assert!(known.iter().any(|w| w.dex == 0 && w.method == 110));
        assert!(!known.iter().any(|w| w.dex == 0 && w.method == 111));
        println!(
            "FIELD {key:?} known={known:?} explicit={:?}",
            inv.explicit_store_writes(&key)
        );
    }
}

#[test]
fn capability_does_not_erase_scoped_flags_or_special_invocation_uncertainty() {
    let (mut d, fields) = fixture();
    native(&mut d);
    d.classes[0].class_data.as_mut().unwrap().instance_fields[0].access_flags = 0x51;
    let key = field_key(&d, fields[0]).unwrap();
    let inv = WriteInventory::scan(std::slice::from_ref(&d));
    assert!(matches!(
        inv.explicit_store_writes(&key),
        Err(ExplicitStoreError::SymbolicFieldUnknown(_))
    ));
    d.classes[0].class_data.as_mut().unwrap().instance_fields[0].access_flags = 0x11;
    let target = d.classes[2].class_data.as_ref().unwrap().direct_methods[0].method_idx as u16;
    let proto = d.method_ids[target as usize].proto_idx;
    for units in [
        vec![0x00fa, target, 0, proto, 0x000e],
        vec![0x00fc, 0, 0, 0x000e],
    ] {
        let code = d.classes[2].class_data.as_mut().unwrap().direct_methods[0]
            .code
            .as_mut()
            .unwrap();
        code.insns = units.iter().flat_map(|u| u.to_le_bytes()).collect();
        code.insns_size = units.len() as u32;
        assert!(matches!(
            WriteInventory::scan(std::slice::from_ref(&d)).explicit_store_writes(&key),
            Err(ExplicitStoreError::StructuralUnknown(_))
        ));
    }
}
