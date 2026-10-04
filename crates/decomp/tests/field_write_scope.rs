//! Descriptor-scoped evidence regressions independent of the shared facade.
#![allow(clippy::unwrap_used)]
use decomp::java_class_kinds::initializers::write_inventory::{WriteInventory, field_key};
use dex_format::{
    DexFile, RawSpan,
    class::{ClassData, EncodedField, EncodedMethod},
    ids::{FieldId, MethodId, ProtoId, TypeId},
};
const SPAN: RawSpan = RawSpan { offset: 0, size: 0 };
fn string(d: &mut DexFile, s: &str) -> u32 {
    let mut value = d.strings[0].clone();
    value.value = s.into();
    let i = d.strings.len() as u32;
    d.strings.push(value);
    i
}
fn ty(d: &mut DexFile, s: &str) -> u32 {
    let descriptor_idx = string(d, s);
    let i = d.type_ids.len() as u32;
    d.type_ids.push(TypeId {
        descriptor_idx,
        span: SPAN,
    });
    i
}
fn fixture() -> (DexFile, u32, u32) {
    let mut d = DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let mut c = d.classes[0].clone();
    c.class_idx = ty(&mut d, "LScope;");
    c.static_values = None;
    c.static_values_off = 0;
    c.annotations = None;
    let type_idx = ty(&mut d, "I") as u16;
    let mut fields = vec![];
    let start = d.field_ids.len() as u32;
    for name in ["encodedFinal", "unresolved"] {
        let name_idx = string(&mut d, name);
        d.field_ids.push(FieldId {
            class_idx: c.class_idx as u16,
            type_idx,
            name_idx,
            span: SPAN,
        });
    }
    fields.push(EncodedField {
        field_idx: start,
        access_flags: 0x19,
    });
    let return_type_idx = ty(&mut d, "V");
    let proto_idx = d.proto_ids.len() as u16;
    d.proto_ids.push(ProtoId {
        shorty_idx: 0,
        return_type_idx,
        parameters_off: 0,
        parameters: None,
        span: SPAN,
    });
    let name_idx = string(&mut d, "writer");
    let method_idx = d.method_ids.len() as u32;
    d.method_ids.push(MethodId {
        class_idx: c.class_idx as u16,
        proto_idx,
        name_idx,
        span: SPAN,
    });
    let units = [0x0067u16, (start + 1) as u16, 0x000e];
    let code = dex_format::code::CodeItem {
        registers_size: 1,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: 3,
        insns: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: SPAN,
    };
    c.class_data = Some(ClassData {
        static_fields: fields,
        instance_fields: vec![],
        direct_methods: vec![EncodedMethod {
            method_idx,
            access_flags: 9,
            code_off: 1,
            code: Some(code),
        }],
        virtual_methods: vec![],
        span: SPAN,
    });
    d.classes = vec![c];
    (d, start, start + 1)
}
#[test]
fn symbolic_unknown_is_exact_and_pool_references_are_not_declarations() {
    let (d, declared, unresolved) = fixture();
    let inventory = WriteInventory::scan(std::slice::from_ref(&d));
    let known = field_key(&d, declared).unwrap();
    let missing = field_key(&d, unresolved).unwrap();
    assert!(inventory.declares_field(&known));
    assert!(!inventory.declares_field(&missing));
    assert_eq!(inventory.proven_writes(&known).unwrap().len(), 0);
    assert!(inventory.proven_writes(&missing).is_none());
    assert!(inventory.field_unknown_reasons(&known).is_empty());
    assert!(inventory.field_unknown_reasons(&missing)[0].contains("at 0"));
    assert!(inventory.class_writes_known("LScope;"));
}
#[test]
fn native_capability_still_blocks_every_field() {
    let (mut d, declared, _) = fixture();
    let m = &mut d.classes[0].class_data.as_mut().unwrap().direct_methods[0];
    m.access_flags = 0x109;
    m.code_off = 0;
    m.code = None;
    let inventory = WriteInventory::scan(std::slice::from_ref(&d));
    assert!(
        inventory
            .proven_writes(&field_key(&d, declared).unwrap())
            .is_none()
    );
    assert!(!inventory.class_writes_known("LScope;"));
}

#[test]
fn reflection_capability_is_not_treated_as_a_symbolic_write() {
    let (mut d, declared, _) = fixture();
    let target = d.method_ids.len() as u16;
    let mut raw = d.method_ids.last().unwrap().clone();
    raw.class_idx = ty(&mut d, "Ljava/lang/reflect/Field;") as u16;
    raw.name_idx = string(&mut d, "set");
    d.method_ids.push(raw);
    let code = d.classes[0].class_data.as_mut().unwrap().direct_methods[0]
        .code
        .as_mut()
        .unwrap();
    code.insns = [0x006eu16, target, 0, 0x000e]
        .iter()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    code.insns_size = 4;
    let inventory = WriteInventory::scan(std::slice::from_ref(&d));
    let key = field_key(&d, declared).unwrap();
    assert!(inventory.proven_writes(&key).is_none());
    assert!(
        inventory
            .field_unknown_reasons(&key)
            .iter()
            .any(|r| r.contains("invocation at 0"))
    );
}
#[test]
fn declaration_availability_does_not_certify_initialization() {
    let (d, declared, _) = fixture();
    let resolver = decomp::java_members::MemberResolver { dex: &d };
    assert!(resolver.field_declaration(declared).is_err());
    assert_eq!(
        resolver
            .field_declaration_without_initialization(declared)
            .unwrap(),
        "public static final int encodedFinal;"
    );
}

#[test]
fn unresolved_inherited_symbol_poison_is_signature_scoped() {
    let (mut d, declared, unresolved) = fixture();
    d.field_ids[unresolved as usize].name_idx = d.field_ids[declared as usize].name_idx;
    d.field_ids[unresolved as usize].class_idx = ty(&mut d, "LUnknownSubclass;") as u16;
    let inventory = WriteInventory::scan(std::slice::from_ref(&d));
    let key = field_key(&d, declared).unwrap();
    assert!(inventory.proven_writes(&key).is_none());
    assert!(!inventory.field_unknown_reasons(&key).is_empty());
    assert!(!inventory.class_writes_known("LScope;"));
}

#[test]
fn cross_dex_read_operand_resolves_only_real_declaration() {
    let (mut d, declared, _) = fixture();
    d.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .clear();
    let (mut caller, read_field, _) = fixture();
    caller.classes[0].class_idx = ty(&mut caller, "LCaller;");
    caller.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .static_fields
        .clear();
    let method = caller.classes[0]
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods[0]
        .method_idx;
    caller.method_ids[method as usize].class_idx = caller.classes[0].class_idx as u16;
    let m = &mut caller.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods[0];
    m.code.as_mut().unwrap().insns = [0x0060u16, read_field as u16, 0x000e]
        .iter()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let key = field_key(&d, declared).unwrap();
    let inventory = WriteInventory::scan(&[d, caller]);
    assert_eq!(inventory.field_declaration(&key).unwrap().dex, 0);
    let read = inventory.known_reads(&key).unwrap().first().unwrap();
    assert_eq!((read.dex, read.offset, read.is_static), (1, 0, true));
    assert!(inventory.proven_writes(&key).unwrap().is_empty());
}

#[test]
fn public_declaration_and_encoded_prefix_compile_and_preserve_active_use() {
    use decomp::java_class_kinds::initializers::plan_encoded_initialization;
    let (mut d, declared, _) = fixture();
    d.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .clear();
    d.classes[0].static_values = Some(vec![dex_format::encoded_value::EncodedValue {
        value: dex_format::encoded_value::EncodedValueKind::Int(7),
        span: SPAN,
    }]);
    let inventory = WriteInventory::scan(std::slice::from_ref(&d));
    let plan = plan_encoded_initialization(&d, &d.classes[0], &inventory).unwrap();
    let declaration = decomp::java_members::MemberResolver { dex: &d }
        .field_declaration_without_initialization(declared)
        .unwrap();
    let source = format!(
        "class Effects {{ static int count; }} class Scope {{ {declaration} {} static {{ Effects.count++; }} }} public class FieldScopeSmoke {{ public static void main(String[] a) {{ if (Effects.count != 0) throw new AssertionError(); if (Scope.encodedFinal != 7 || Effects.count != 1) throw new AssertionError(); }} }}",
        plan.prefix_block
    );
    let dir = std::env::temp_dir().join(format!("field-scope-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("FieldScopeSmoke.java"), source).unwrap();
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    for (tool, args) in [
        ("javac", vec!["--release", "17", "FieldScopeSmoke.java"]),
        ("java", vec!["-cp", ".", "FieldScopeSmoke"]),
    ] {
        let result = std::process::Command::new(format!("{home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn three_real_okcashbag_classes_expose_fields_without_certifying_unknown_initialization() {
    use decomp::java_class_kinds::initializers::{
        check_class_initializer_coverage, plan_encoded_initialization,
    };
    let apk = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk"
    );
    let bytes = std::process::Command::new("unzip")
        .args(["-p", apk, "classes.dex"])
        .output()
        .unwrap();
    assert!(bytes.status.success());
    let dex = DexFile::from_bytes(&bytes.stdout).unwrap();
    // One real DEX fixture inventory; these are declaration/unknown-status
    // probes, not whole-APK absence proofs or application emission.
    let inventory = WriteInventory::scan(std::slice::from_ref(&dex));
    let mut checked = 0;
    for class in &dex.classes {
        if class.access_flags & (0x200 | 0x2000 | 0x4000) != 0 {
            continue;
        }
        let Some(data) = &class.class_data else {
            continue;
        };
        let Some(field) = data
            .static_fields
            .iter()
            .find(|f| f.access_flags & 0x18 == 0x18)
        else {
            continue;
        };
        let resolver = decomp::java_members::MemberResolver { dex: &dex };
        let Ok(source) = resolver.field_declaration_without_initialization(field.field_idx) else {
            continue;
        };
        let key = field_key(&dex, field.field_idx).unwrap();
        assert!(inventory.field_declaration(&key).is_some());
        assert!(inventory.proven_writes(&key).is_none());
        assert!(plan_encoded_initialization(&dex, class, &inventory).is_err());
        let constructors = data
            .direct_methods
            .iter()
            .filter(|m| {
                dex.string(dex.method_id(m.method_idx).unwrap().name_idx)
                    .unwrap()
                    == "<init>"
            })
            .map(|m| m.method_idx)
            .collect();
        let clinit = data.direct_methods.iter().find(|m| {
            dex.string(dex.method_id(m.method_idx).unwrap().name_idx)
                .unwrap()
                == "<clinit>"
        });
        if let Some(m) = clinit {
            assert!(check_class_initializer_coverage(&dex, class, &constructors, None).is_err());
            println!(
                "real unavailable {} field={} clinit={} code_offset={} declaration={source}",
                key.owner, field.field_idx, m.method_idx, m.code_off
            );
        } else {
            println!(
                "real unavailable {} field={} class_data_offset={} declaration={source}",
                key.owner, field.field_idx, class.class_data_off
            );
        }
        checked += 1;
        if checked == 3 {
            break;
        }
    }
    assert_eq!(checked, 3);
}

#[test]
fn complete_initializer_coverage_rejects_missing_clinit_and_constructor() {
    use decomp::java_class_kinds::{
        DeclarationUnsupported as Error, initializers::check_class_initializer_coverage,
    };
    let (mut d, _, _) = fixture();
    let index = d.classes[0].class_data.as_ref().unwrap().direct_methods[0].method_idx;
    d.method_ids[index as usize].name_idx = string(&mut d, "<clinit>");
    assert_eq!(
        check_class_initializer_coverage(&d, &d.classes[0], &Default::default(), None),
        Err(Error::FieldInitializationUnavailable)
    );
    assert!(
        check_class_initializer_coverage(&d, &d.classes[0], &Default::default(), Some(index))
            .is_ok()
    );
    d.method_ids[index as usize].name_idx = string(&mut d, "<init>");
    d.classes[0].class_data.as_mut().unwrap().direct_methods[0].access_flags = 1;
    assert_eq!(
        check_class_initializer_coverage(&d, &d.classes[0], &Default::default(), None),
        Err(Error::FieldInitializationUnavailable)
    );
    assert!(check_class_initializer_coverage(&d, &d.classes[0], &[index].into(), None).is_ok());
}
