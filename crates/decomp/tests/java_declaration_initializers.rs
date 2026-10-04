//! Focused declaration tests; no shared facade integration ownership.
#![allow(clippy::unwrap_used)]
use backend_java::type_compatibility::Declaration;
use decomp::java_class_kinds::{self as kinds, DeclarationUnsupported as Error, initializers::*};
use dex_format::{
    DexFile, RawSpan,
    annotations::{AnnotationItem, AnnotationSet, AnnotationsDirectory, Visibility},
    class::{ClassData, ClassDef, EncodedField, EncodedMethod},
    encoded_value::{AnnotationElement, EncodedAnnotation, EncodedValue, EncodedValueKind as V},
    ids::{FieldId, MethodId, ProtoId, TypeId},
};
use std::{collections::HashSet, process::Command};

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

#[test]
fn descriptor_owned_bad_staticness_does_not_taint_unrelated_same_signature() {
    let (mut dex, mut class) = fixture(1);
    let own = field(&mut dex, &mut class, "N", "I", 0x11);
    let own_key = write_inventory::field_key(&dex, own).unwrap();
    // Deliberately use a static-put opcode against the instance declaration.
    let mut method = element(&mut dex, &mut class, "writer", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    method.access_flags = 9;
    method.code_off = 1;
    method.code = Some(code(&[0x0067, own as u16, 0x000e]));
    class
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(method);
    dex.classes = vec![class];
    let (mut other, mut other_class) = fixture(1);
    other_class.class_idx = ty(&mut other, "LOther;");
    let other_field = field(&mut other, &mut other_class, "N", "I", 0x19);
    let other_key = write_inventory::field_key(&other, other_field).unwrap();
    other.classes = vec![other_class];
    let inventory = write_inventory::WriteInventory::scan(&[dex, other]);
    assert!(inventory.proven_writes(&own_key).is_none());
    assert!(inventory.proven_writes(&other_key).unwrap().is_empty());
    assert!(inventory.unknown_reasons().is_empty());
}

#[test]
fn complete_cross_dex_inventory_keeps_dropped_writers_and_denies_unknown() {
    let (mut dex, mut class) = fixture(1);
    let f = field(&mut dex, &mut class, "N", "I", 0x19);
    let key = write_inventory::field_key(&dex, f).unwrap();
    dex.classes = vec![class.clone()];
    let (mut other, mut dropped) = fixture(1);
    dropped.class_idx = ty(&mut other, "LDropped;");
    let owner = ty(&mut other, "LFixture;");
    let type_idx = ty(&mut other, "I") as u16;
    let name_idx = string(&mut other, "N");
    let cross_field = other.field_ids.len() as u32;
    other.field_ids.push(FieldId {
        class_idx: owner as u16,
        type_idx,
        name_idx,
        span: SPAN,
    });
    let mut method = element(&mut other, &mut dropped, "droppedWriter", "V");
    dropped.class_data.as_mut().unwrap().virtual_methods.clear();
    method.access_flags = 9;
    method.code_off = 1;
    // Even a write after an unreachable return is included.
    method.code = Some(code(&[0x000e, 0x0067, cross_field as u16, 0x000e]));
    dropped
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(method.clone());
    other.classes = vec![dropped];
    let inventory = write_inventory::WriteInventory::scan(&[dex.clone(), other.clone()]);
    assert!(
        inventory.unknown_reasons().is_empty(),
        "{:?}",
        inventory.unknown_reasons()
    );
    assert_eq!(inventory.scanned_methods(), 1);
    let sites = inventory.proven_writes(&key).unwrap();
    assert_eq!(sites.len(), 1);
    assert_eq!(sites.first().unwrap().dex, 1);
    assert_eq!(
        plan_encoded_initialization(&dex, &class, &inventory)
            .unwrap()
            .blank_static_finals,
        [f].into()
    );
    class.static_values = Some(vec![value(V::Int(7))]);
    assert_eq!(
        plan_encoded_initialization(&dex, &class, &inventory),
        Err(Error::FieldInitializationUnavailable)
    );
    for malformed in [vec![0x00ff], vec![0x0067, u16::MAX], vec![0x0067]] {
        other.classes[0].class_data.as_mut().unwrap().direct_methods[0].code =
            Some(code(&malformed));
        let inventory = write_inventory::WriteInventory::scan(&[dex.clone(), other.clone()]);
        assert!(inventory.proven_writes(&key).is_none());
        assert_eq!(
            plan_encoded_initialization(&dex, &class, &inventory),
            Err(Error::FieldInitializationUnavailable)
        );
    }
    other.classes[0].class_data.as_mut().unwrap().direct_methods[0].code = None;
    let inventory = write_inventory::WriteInventory::scan(&[dex, other]);
    assert!(inventory.proven_writes(&key).is_none());
}

fn compile_run(label: &str, source: &str) {
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let dir = std::env::temp_dir().join(format!("decomp-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), source).unwrap();
    for (binary, args) in [
        ("javac", vec!["--release", "17", "Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let result = Command::new(format!("{home}/bin/{binary}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{binary}: {}\n{source}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

// Compile the exact returned constructor source under its original identity.
// The runtime oracle lives in a separate class; generated members are untouched.
fn compile_original_class(label: &str, owner: &str, source: &str, checks: &str) {
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let dir = std::env::temp_dir().join(format!("decomp-original-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (package, simple) = owner.rsplit_once('.').unwrap();
    let file = format!("{simple}.java");
    let source = format!("package {package};\n{source}");
    std::fs::write(dir.join(&file), &source).unwrap();
    std::fs::write(dir.join("Fixture.java"), format!(
        "package {package}; public class Fixture {{ public static void main(String[] args) throws Exception {{ {checks} }} }}"
    )).unwrap();
    let runtime = format!("{package}.Fixture");
    for (binary, args) in [
        (
            "javac",
            vec!["--release", "17", "-d", ".", &file, "Fixture.java"],
        ),
        ("java", vec!["-cp", ".", &runtime]),
    ] {
        let result = Command::new(format!("{home}/bin/{binary}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{binary}: {}\n{source}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn encoded_prefix_preserves_defaults_order_and_active_use_effects_on_jdk17() {
    let (mut dex, mut class) = fixture(1);
    let a = field(&mut dex, &mut class, "A", "I", 0x19);
    let b = field(&mut dex, &mut class, "B", "I", 8);
    let c = field(&mut dex, &mut class, "C", "Z", 0x19);
    class.static_values = Some(vec![value(V::Int(7))]);
    dex.classes = vec![class.clone()];
    let inventory = write_inventory::WriteInventory::scan(&[dex.clone()]);
    let plan = plan_encoded_initialization(&dex, &class, &inventory).unwrap();
    assert_eq!(
        plan.assignments
            .iter()
            .map(|f| f.field_idx)
            .collect::<Vec<_>>(),
        [a, b, c]
    );
    assert_eq!(
        plan.prefix_block,
        "static {\n    A = 7;\n    B = 0;\n    C = false;\n}\n"
    );
    compile_run(
        "encoded-prefix",
        &format!(
            "class Effects {{ static int count; }} class Values {{ static final int A; static int B; static final boolean C; {} static {{ if (A != 7 || B != 0 || C) throw new AssertionError(); Effects.count++; B = 9; }} }} public class Fixture {{ public static void main(String[] a) {{ if (Effects.count != 0) throw new AssertionError(); if (Values.A != 7 || Effects.count != 1 || Values.B != 9) throw new AssertionError(); }} }}",
            plan.prefix_block
        ),
    );
}

#[test]
fn decoded_branch_static_initializer_keeps_original_conditions_on_jdk17() {
    let (mut dex, mut class) = fixture(1);
    let f = field(&mut dex, &mut class, "N", "I", 8);
    let mut m = element(&mut dex, &mut class, "<clinit>", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    m.access_flags = 0x10008;
    m.code_off = 1;
    // sget v0,N; if-eqz v0,+6; const/4 v0,7; sput; return;
    // taken: const/4 v0,3; sput; return. Both original return paths survive.
    m.code = Some(code(&[
        0x0060, f as u16, 0x0038, 6, 0x7012, 0x0067, f as u16, 0x000e, 0x3012, 0x0067, f as u16,
        0x000e,
    ]));
    class
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(m.clone());
    dex.classes = vec![class];
    let ssa = dex_decompiler::DexPasses::run_ssa_with(
        m.code.as_ref().unwrap(),
        &dex_decompiler::DexTypes::new(&dex, Some(m.method_idx)),
    )
    .unwrap();
    let resolver = Resolver(&dex);
    let method = resolver.method(MethodRef::new(m.method_idx)).unwrap();
    let context = backend_java::static_initializer::StaticInitializerContext {
        owner: "Fixture".into(),
        writable_static_fields: vec![resolver.field(FieldRef::new(f)).unwrap()],
    };
    let source = backend_java::static_initializer::emit_static_initializer(
        ssa.ir(),
        &method,
        &ParameterBindings::default(),
        &resolver,
        &context,
    )
    .unwrap();
    assert!(source.contains("if ("));
    assert!(!source.contains("return;"));
    compile_run(
        "decoded-static-branch",
        &format!(
            "public class Fixture {{ static int N; {source} public static void main(String[] a) {{ if (N != 3) throw new AssertionError(); }} }}"
        ),
    );
    assert!(
        backend_java::static_initializer::emit_static_initializer_with_finals(
            ssa.ir(),
            &method,
            &ParameterBindings::default(),
            &resolver,
            &context,
            &[FieldRef::new(f)]
        )
        .is_err()
    );
}

#[test]
fn inventory_to_blank_final_backend_wiring_is_complete() {
    let (mut dex, mut class) = fixture(1);
    let f = field(&mut dex, &mut class, "N", "I", 0x19);
    let mut m = element(&mut dex, &mut class, "<clinit>", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    m.access_flags = 0x10008;
    m.code_off = 1;
    m.code = Some(code(&[0x7012, 0x0067, f as u16, 0x000e]));
    class
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(m.clone());
    dex.classes = vec![class.clone()];
    let inventory = write_inventory::WriteInventory::scan(std::slice::from_ref(&dex));
    let encoded = plan_encoded_initialization(&dex, &class, &inventory).unwrap();
    assert!(encoded.prefix_block.is_empty());
    let finals = prove_static_final_writes(&dex, 0, m.method_idx, &encoded, &inventory).unwrap();
    let ssa = dex_decompiler::DexPasses::run_ssa_with(
        m.code.as_ref().unwrap(),
        &dex_decompiler::DexTypes::new(&dex, Some(m.method_idx)),
    )
    .unwrap();
    let resolver = Resolver(&dex);
    let method = resolver.method(MethodRef::new(m.method_idx)).unwrap();
    let block = backend_java::static_initializer::emit_static_initializer_with_finals(
        ssa.ir(),
        &method,
        &ParameterBindings::default(),
        &resolver,
        &backend_java::static_initializer::StaticInitializerContext {
            owner: "Fixture".into(),
            writable_static_fields: vec![],
        },
        &finals,
    )
    .unwrap();
    compile_run(
        "inventory-static",
        &format!(
            "public class Fixture {{ static final int N; {block} public static void main(String[] a) {{ if(N != 7) throw new AssertionError(); }} }}"
        ),
    );
    assert!(prove_static_final_writes(&dex, 1, m.method_idx, &encoded, &inventory).is_err());

    // Instance-final evidence rejects a writer in an ordinary method rather than
    // considering only constructors selected for lowering.
    let (mut dex, mut class) = fixture(1);
    let f = field(&mut dex, &mut class, "n", "I", 0x11);
    let mut ctor = element(&mut dex, &mut class, "<init>", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    ctor.access_flags = 0x10001;
    ctor.code_off = 1;
    ctor.code = Some(code(&[0x1059, f as u16, 0x000e]));
    class
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(ctor.clone());
    dex.classes = vec![class.clone()];
    let inventory = write_inventory::WriteInventory::scan(std::slice::from_ref(&dex));
    let evidence = prove_instance_final_writes(
        std::slice::from_ref(&dex),
        0,
        &class,
        &[ctor.method_idx].into(),
        &inventory,
    )
    .unwrap();
    assert_eq!(evidence[0].writes[&MethodRef::new(ctor.method_idx)], 1);
    let mut writer = element(&mut dex, &mut class, "ordinaryWriter", "V");
    writer.access_flags = 1;
    writer.code_off = 1;
    writer.code = ctor.code;
    *class
        .class_data
        .as_mut()
        .unwrap()
        .virtual_methods
        .last_mut()
        .unwrap() = writer;
    dex.classes = vec![class.clone()];
    let inventory = write_inventory::WriteInventory::scan(std::slice::from_ref(&dex));
    assert!(
        prove_instance_final_writes(
            std::slice::from_ref(&dex),
            0,
            &class,
            &[ctor.method_idx].into(),
            &inventory
        )
        .is_err()
    );
}

const SPAN: RawSpan = RawSpan { offset: 0, size: 0 };
fn value(value: V) -> EncodedValue {
    EncodedValue { value, span: SPAN }
}
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
fn defaults(dex: &mut DexFile, class: &mut ClassDef, entries: &[(&EncodedMethod, V)]) {
    let wrapper_type = ty(dex, "Ldalvik/annotation/AnnotationDefault;");
    let name_idx = string(dex, "value");
    let elements = entries
        .iter()
        .map(|(method, v)| AnnotationElement {
            name_idx: dex.method_ids[method.method_idx as usize].name_idx,
            value: value(v.clone()),
        })
        .collect();
    class.annotations = Some(AnnotationsDirectory {
        class_annotations: Some(AnnotationSet {
            entries: vec![AnnotationItem {
                visibility: Visibility::System,
                span: SPAN,
                annotation: EncodedAnnotation {
                    type_idx: wrapper_type,
                    span: SPAN,
                    elements: vec![AnnotationElement {
                        name_idx,
                        value: value(V::Annotation(EncodedAnnotation {
                            type_idx: class.class_idx,
                            elements,
                            span: SPAN,
                        })),
                    }],
                },
            }],
            span: SPAN,
        }),
        field_annotations: vec![],
        method_annotations: vec![],
        parameter_annotations: vec![],
        span: SPAN,
    });
}

#[test]
fn annotation_defaults_are_typed_and_atomic() {
    let (mut dex, mut class) = fixture(0x2601);
    let m = element(&mut dex, &mut class, "answer", "I");
    defaults(&mut dex, &mut class, &[(&m, V::Int(42))]);
    assert_eq!(
        annotation_defaults(&dex, &class).unwrap()[&m.method_idx],
        "42"
    );
    assert_eq!(
        kinds::abstract_method_declaration(&dex, &class, &m).unwrap(),
        "public int answer() default 42;"
    );
    assert_eq!(
        kinds::class_declaration(&dex, &class).unwrap(),
        "public @interface Fixture"
    );
    defaults(&mut dex, &mut class, &[(&m, V::String(0))]);
    assert_eq!(
        annotation_defaults(&dex, &class),
        Err(Error::InvalidEncodedValue)
    );
    assert_eq!(
        kinds::class_declaration(&dex, &class),
        Err(Error::InvalidEncodedValue)
    );
    defaults(&mut dex, &mut class, &[(&m, V::Int(1)), (&m, V::Int(2))]);
    assert_eq!(
        annotation_defaults(&dex, &class),
        Err(Error::AnnotationDefaultsUnavailable)
    );
    defaults(&mut dex, &mut class, &[(&m, V::Int(1))]);
    let set = class
        .annotations
        .as_mut()
        .unwrap()
        .class_annotations
        .as_mut()
        .unwrap();
    set.entries[0].visibility = Visibility::Runtime;
    assert_eq!(
        annotation_defaults(&dex, &class),
        Err(Error::AnnotationDefaultsUnavailable)
    );
}

#[test]
fn production_facade_annotation_defaults_compile_and_execute() {
    let (mut dex, mut class) = fixture(0x2601);
    let m = element(&mut dex, &mut class, "answer", "I");
    defaults(&mut dex, &mut class, &[(&m, V::Int(42))]);
    dex.classes = vec![class];
    let mut sources = Vec::new();
    let report = decomp::dex_java::emit_java(
        &[dex],
        &Default::default(),
        &mut |s: &decomp::output::GeneratedSource| {
            sources.push(s.text.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(report.declaration_only_methods, 1);
    assert_eq!(report.unsupported_methods, 0);
    assert_eq!(sources.len(), 1);
    assert!(sources[0].contains("int answer() default 42;"));
    let dir = std::env::temp_dir().join(format!(
        "decomphify-facade-annotation-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"), &sources[0]).unwrap();
    std::fs::write(dir.join("Main.java"), "public class Main { public static void main(String[] a) throws Exception { if(!Integer.valueOf(42).equals(Fixture.class.getMethod(\"answer\").getDefaultValue())) throw new AssertionError(); System.out.println(\"facade annotation default passed\"); } }").unwrap();
    let compiled = Command::new("javac")
        .args(["Fixture.java", "Main.java"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&compiled.stderr),
        sources[0]
    );
    let run = Command::new("java")
        .args(["-cp", ".", "Main"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "facade annotation default passed\n"
    );
}

#[test]
fn arrays_class_literals_and_frozen_names() {
    let (mut dex, mut class) = fixture(0x2601);
    let m = element(&mut dex, &mut class, "numbers", "[I");
    let c = element(&mut dex, &mut class, "type", "Ljava/lang/Class;");
    let literal = ty(&mut dex, "LOriginal;");
    defaults(
        &mut dex,
        &mut class,
        &[
            (&m, V::Array(vec![value(V::Int(1)), value(V::Int(-2))])),
            (&c, V::Type(literal)),
        ],
    );
    let result = annotation_defaults_with(&dex, &class, &|_| Ok("Renamed".into())).unwrap();
    assert_eq!(result[&m.method_idx], "{1, -2}");
    assert_eq!(result[&c.method_idx], "Renamed.class");
    let plan = plan_annotation_defaults_with(&dex, &class, &|_| Ok("Renamed".into())).unwrap();
    assert_eq!(plan.by_method, result);
    assert_eq!(
        plan.referenced_types.into_iter().collect::<Vec<_>>(),
        vec![literal]
    );
    assert!(plan.enum_fields.is_empty());
    defaults(
        &mut dex,
        &mut class,
        &[(&m, V::Array(vec![value(V::Null)]))],
    );
    assert_eq!(
        annotation_defaults(&dex, &class),
        Err(Error::InvalidEncodedValue)
    );
}

#[test]
fn annotation_enum_defaults_require_real_constant_metadata() {
    let (mut dex, mut class) = fixture(0x2601);
    let (mut enum_dex, _) = fixture(1);
    // Build the enum in this pool; a mere type-pool entry is insufficient.
    let enum_type = ty(&mut dex, "LE;");
    let mut enumeration = enum_dex.classes.remove(0);
    enumeration.class_idx = enum_type;
    enumeration.access_flags = 0x4011;
    enumeration.class_data = Some(ClassData {
        static_fields: vec![],
        instance_fields: vec![],
        direct_methods: vec![],
        virtual_methods: vec![],
        span: SPAN,
    });
    let constant = field(&mut dex, &mut enumeration, "A", "LE;", 0x4019);
    dex.classes.push(enumeration);
    let m = element(&mut dex, &mut class, "choice", "LE;");
    defaults(&mut dex, &mut class, &[(&m, V::Enum(constant))]);
    let plan = plan_annotation_defaults_with(&dex, &class, &|_| Ok("RenamedEnum".into())).unwrap();
    assert_eq!(
        plan.enum_fields.into_iter().collect::<Vec<_>>(),
        vec![constant]
    );
    assert!(plan.referenced_types.contains(&enum_type));
    assert_eq!(
        annotation_defaults_with(&dex, &class, &|_| Ok("RenamedEnum".into())).unwrap()
            [&m.method_idx],
        "RenamedEnum.A"
    );
    dex.classes
        .last_mut()
        .unwrap()
        .class_data
        .as_mut()
        .unwrap()
        .static_fields[0]
        .access_flags &= !0x4000;
    assert_eq!(
        annotation_defaults(&dex, &class),
        Err(Error::InvalidEncodedValue)
    );
}

#[test]
fn malformed_wrappers_or_unknown_elements_never_produce_partial_defaults() {
    for mutation in 0..5 {
        let (mut dex, mut class) = fixture(0x2601);
        let m = element(&mut dex, &mut class, "answer", "I");
        defaults(&mut dex, &mut class, &[(&m, V::Int(42))]);
        let item = &mut class
            .annotations
            .as_mut()
            .unwrap()
            .class_annotations
            .as_mut()
            .unwrap()
            .entries[0];
        match mutation {
            0 => item.annotation.elements[0].value = value(V::Int(1)),
            1 => {
                let V::Annotation(a) = &mut item.annotation.elements[0].value.value else {
                    unreachable!()
                };
                a.type_idx = u32::MAX;
            }
            2 => item
                .annotation
                .elements
                .push(item.annotation.elements[0].clone()),
            3 => class.class_data.as_mut().unwrap().virtual_methods.clear(),
            4 => class
                .class_data
                .as_mut()
                .unwrap()
                .virtual_methods
                .push(m.clone()),
            _ => unreachable!(),
        }
        assert_eq!(
            annotation_defaults(&dex, &class),
            Err(Error::AnnotationDefaultsUnavailable),
            "mutation {mutation}"
        );
    }
}

#[test]
fn encoded_float_payloads_and_missing_values_fail_closed() {
    let (mut dex, mut class) = fixture(1);
    field(&mut dex, &mut class, "F", "F", 0x19);
    class.static_values = Some(vec![value(V::Float(f32::from_bits(0x7fc00001)))]);
    assert_eq!(
        encoded_field_initializations(&dex, &class, &HashSet::new()),
        Err(Error::InvalidEncodedValue)
    );
    class.static_values = Some(vec![value(V::Float(f32::INFINITY))]);
    assert_eq!(
        encoded_field_initializations(&dex, &class, &HashSet::new()).unwrap()[0].expression,
        "(1.0f / 0.0f)"
    );
    class.static_values = Some(vec![value(V::Float(1.0)), value(V::Float(2.0))]);
    assert_eq!(
        encoded_field_initializations(&dex, &class, &HashSet::new()),
        Err(Error::FieldInitializationUnavailable)
    );
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

#[test]
fn encoded_fields_use_declaration_order_and_final_evidence() {
    let (mut dex, mut class) = fixture(1);
    let a = field(&mut dex, &mut class, "A", "I", 0x19);
    let b = field(&mut dex, &mut class, "B", "I", 8);
    let c = field(&mut dex, &mut class, "C", "J", 8);
    class.class_data.as_mut().unwrap().static_fields.swap(0, 1);
    class.static_values = Some(vec![value(V::Int(11)), value(V::Int(22))]);
    let result = encoded_field_initializations(&dex, &class, &HashSet::from([b])).unwrap();
    assert_eq!(
        result
            .iter()
            .map(|v| (v.field_idx, v.expression.as_str(), v.explicit))
            .collect::<Vec<_>>(),
        vec![(b, "11", true), (a, "22", true), (c, "0L", false)]
    );
    assert_eq!(
        encoded_field_initializations(&dex, &class, &HashSet::from([a])),
        Err(Error::FieldInitializationUnavailable)
    );
    class.static_values.as_mut().unwrap().pop();
    assert_eq!(
        encoded_field_initializations(&dex, &class, &HashSet::new()),
        Err(Error::FieldInitializationUnavailable)
    );
    class.static_values = Some(vec![value(V::Int(11)), value(V::Int(22))]);
    field(&mut dex, &mut class, "blankFinal", "I", 0x10);
    assert_eq!(
        encoded_field_initializations(&dex, &class, &HashSet::new()),
        Err(Error::FieldInitializationUnavailable)
    );
}

#[test]
fn fields_reject_wrong_types_unsupported_values_and_enum_conversion() {
    let (mut dex, mut class) = fixture(1);
    field(&mut dex, &mut class, "N", "I", 8);
    for v in [
        V::Null,
        V::Long(3),
        V::Type(0),
        V::MethodHandle(0),
        V::Array(vec![]),
    ] {
        class.static_values = Some(vec![value(v)]);
        assert_eq!(
            encoded_field_initializations(&dex, &class, &HashSet::new()),
            Err(Error::InvalidEncodedValue)
        );
    }
    class.access_flags = 0x4001;
    assert_eq!(
        encoded_field_initializations(&dex, &class, &HashSet::new()),
        Err(Error::FieldInitializationUnavailable)
    );
    assert_eq!(
        kinds::class_declaration(&dex, &class),
        Err(Error::EnumConstructorsUnavailable)
    );
}

fn declaration(name: &str, superclass: Option<&str>) -> Declaration {
    Declaration {
        name: name.into(),
        superclass: superclass.map(str::to_owned),
        interfaces: vec![],
        is_interface: false,
    }
}
#[test]
fn constructor_hook_requires_exact_complete_acyclic_hierarchy() {
    let (mut dex, mut class) = fixture(1);
    class.superclass_idx = ty(&mut dex, "LBase;");
    let names = |i| {
        if i == class.class_idx {
            Ok("Fixture".into())
        } else {
            Ok("Base".into())
        }
    };
    let owner = declaration("Fixture", Some("Base"));
    let base = declaration("Base", Some("java.lang.Object"));
    let root = declaration("java.lang.Object", None);
    assert_eq!(
        trusted_constructor_superclass(
            &dex,
            &class,
            &[owner.clone(), base.clone(), root.clone()],
            &names
        )
        .unwrap(),
        "Base"
    );
    assert_eq!(
        trusted_constructor_superclass(&dex, &class, &[owner.clone(), root.clone()], &names),
        Err(Error::InvalidSuperclass)
    );
    assert_eq!(
        trusted_constructor_superclass(
            &dex,
            &class,
            &[
                declaration("Fixture", Some("Other")),
                base.clone(),
                root.clone()
            ],
            &names
        ),
        Err(Error::InvalidSuperclass)
    );
    assert_eq!(
        trusted_constructor_superclass(
            &dex,
            &class,
            &[
                owner.clone(),
                declaration("Base", Some("Fixture")),
                root.clone()
            ],
            &names
        ),
        Err(Error::InvalidSuperclass)
    );
    assert_eq!(
        trusted_constructor_superclass(
            &dex,
            &class,
            &[owner, base, root, declaration("Base", Some("Other"))],
            &names
        ),
        Err(Error::InvalidSuperclass)
    );
}

#[test]
fn generated_defaults_and_encoded_finals_compile_and_run_on_jdk17() {
    let (mut dex, mut annotation) = fixture(0x2600);
    let scalar = element(&mut dex, &mut annotation, "answer", "I");
    let array = element(&mut dex, &mut annotation, "numbers", "[I");
    let text = element(&mut dex, &mut annotation, "text", "Ljava/lang/String;");
    let char_m = element(&mut dex, &mut annotation, "unit", "C");
    let float_m = element(&mut dex, &mut annotation, "negativeZero", "F");
    let class_m = element(&mut dex, &mut annotation, "type", "Ljava/lang/Class;");
    let string_index = string(&mut dex, "quote\"slash\\u000a\n\0☃");
    let type_index = ty(&mut dex, "I");
    defaults(
        &mut dex,
        &mut annotation,
        &[
            (&scalar, V::Int(42)),
            (&array, V::Array(vec![value(V::Int(-2)), value(V::Int(7))])),
            (&text, V::String(string_index)),
            (&char_m, V::Char(0xd800)),
            (&float_m, V::Float(-0.0)),
            (&class_m, V::Type(type_index)),
        ],
    );
    let elements = [&scalar, &array, &text, &char_m, &float_m, &class_m]
        .iter()
        .map(|m| kinds::abstract_method_declaration(&dex, &annotation, m).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let (mut fields_dex, mut class) = fixture(1);
    field(&mut fields_dex, &mut class, "MIN", "J", 0x19);
    field(&mut fields_dex, &mut class, "NEG", "D", 0x19);
    class.static_values = Some(vec![value(V::Long(i64::MIN)), value(V::Double(-0.0))]);
    let init = encoded_field_initializations(&fields_dex, &class, &HashSet::new()).unwrap();
    let source = format!(
        "@interface Defaults {{ {elements} }}\npublic class Fixture {{\npublic static final long MIN = {};\npublic static final double NEG = {};\npublic static void main(String[] args) throws Exception {{\nif (!Defaults.class.getMethod(\"answer\").getDefaultValue().equals(42)) throw new AssertionError();\nif (!java.util.Arrays.equals((int[]) Defaults.class.getMethod(\"numbers\").getDefaultValue(), new int[] {{-2,7}})) throw new AssertionError();\nif (((Character)Defaults.class.getMethod(\"unit\").getDefaultValue()) != (char)0xd800) throw new AssertionError();\nif (Float.floatToRawIntBits((Float)Defaults.class.getMethod(\"negativeZero\").getDefaultValue()) != 0x80000000) throw new AssertionError();\nif (Defaults.class.getMethod(\"type\").getDefaultValue() != int.class) throw new AssertionError();\nString text = (String)Defaults.class.getMethod(\"text\").getDefaultValue();\nif (text.charAt(5) != '\"' || text.charAt(11) != (char)92 || text.charAt(17) != (char)10 || text.charAt(18) != (char)0 || text.charAt(19) != (char)0x2603) throw new AssertionError(text);\nif (MIN != Long.MIN_VALUE || Double.doubleToRawLongBits(NEG) != Long.MIN_VALUE) throw new AssertionError();\n}}\n}}",
        init[0].expression, init[1].expression
    );
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let directory =
        std::env::temp_dir().join(format!("decomp-declaration-init-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("Fixture.java"), &source).unwrap();
    for (binary, args) in [
        ("javac", vec!["--release", "17", "Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let output = Command::new(format!("{home}/bin/{binary}"))
            .args(args)
            .current_dir(&directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{binary}: {}\n{source}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}

use backend_java::{
    JavaField, JavaMethod, JavaResolver, JavaType, ParameterBindings,
    constructors::{ConstructorResolver, Initializer},
};
use decomp_core::cfg::{Cfg, EdgeKind};
use managed_ir::{
    effects::EffectFlags,
    insn::{Constant, InvokeKind, ManagedInsn},
    method::{InsnArena, MethodIr, SsaForm, ValueArena},
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
    types::ManagedType,
    value::{InsnId, Value, ValueId},
};

struct Resolver<'a>(&'a DexFile);
impl JavaResolver for Resolver<'_> {
    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        let m = self.0.method_id(reference.index()).ok()?;
        let p = self.0.proto_id(m.proto_idx.into()).ok()?;
        let type_at = |i: u32| {
            decomp::dex_java::descriptor_type(
                self.0.string(self.0.type_id(i).ok()?.descriptor_idx).ok()?,
            )
        };
        let JavaType::Class(owner) = type_at(m.class_idx.into())? else {
            return None;
        };
        Some(JavaMethod {
            owner,
            name: self.0.string(m.name_idx).ok()?.into(),
            is_static: self.0.string(m.name_idx).ok()? == "<clinit>",
            return_type: type_at(p.return_type_idx)?,
            parameters: p
                .parameters
                .as_ref()
                .map(|p| {
                    p.types
                        .iter()
                        .map(|i| type_at(u32::from(*i)))
                        .collect::<Option<Vec<_>>>()
                })
                .unwrap_or(Some(vec![]))?,
            throws: vec![],
        })
    }
    fn field(&self, reference: FieldRef) -> Option<JavaField> {
        let f = self.0.field_id(reference.index()).ok()?;
        let owner = self.class(ClassRef::new(f.class_idx.into()))?;
        Some(JavaField {
            owner,
            name: self.0.string(f.name_idx).ok()?.into(),
            ty: decomp::dex_java::descriptor_type(
                self.0
                    .string(self.0.type_id(f.type_idx.into()).ok()?.descriptor_idx)
                    .ok()?,
            )?,
            is_static: true,
        })
    }
    fn class(&self, reference: ClassRef) -> Option<String> {
        let t = decomp::dex_java::descriptor_type(
            self.0
                .string(self.0.type_id(reference.index()).ok()?.descriptor_idx)
                .ok()?,
        )?;
        if let JavaType::Class(n) = t {
            Some(n)
        } else {
            None
        }
    }
    fn string(&self, reference: StringRef) -> Option<Vec<u16>> {
        Some(
            self.0
                .string(reference.index())
                .ok()?
                .encode_utf16()
                .collect(),
        )
    }
}
impl ConstructorResolver for Resolver<'_> {
    fn initializer(&self, reference: MethodRef) -> Option<Initializer> {
        let method = self.method(reference)?;
        if method.name != "<init>" {
            return None;
        }
        Some(Initializer {
            method,
            delegates_to: None,
        })
    }
}
#[derive(Default)]
struct Builder {
    insns: InsnArena,
    values: ValueArena,
    order: Vec<InsnId>,
}

struct FamilyResolver {
    malformed: u8,
}
impl JavaResolver for FamilyResolver {
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || (self.malformed == 12 && *actual == JavaType::Int && *expected == JavaType::Long)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        self.initializer(r).map(|i| i.method)
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        Some(JavaField {
            owner: "Fixture".into(),
            name: if r.index() == 0 { "number" } else { "STATIC" }.into(),
            ty: JavaType::Int,
            is_static: r.index() != 0,
        })
    }
    fn class(&self, _: ClassRef) -> Option<String> {
        Some("Fixture".into())
    }
    fn string(&self, _: StringRef) -> Option<Vec<u16>> {
        None
    }
}
impl ConstructorResolver for FamilyResolver {
    fn initializer(&self, r: MethodRef) -> Option<Initializer> {
        if r.index() > 2 {
            return None;
        }
        Some(Initializer {
            method: JavaMethod {
                owner: if r.index() == 2 { "Base" } else { "Fixture" }.into(),
                name: "<init>".into(),
                is_static: false,
                return_type: JavaType::Void,
                parameters: if self.malformed == 12 && r.index() == 2 {
                    vec![JavaType::Long]
                } else {
                    vec![JavaType::Int; if r.index() == 1 { 2 } else { 1 }]
                },
                throws: if self.malformed == 7 && r.index() == 2 {
                    vec!["java.io.IOException".into()]
                } else if self.malformed == 8 {
                    vec![
                        if r.index() == 2 {
                            "java.io.FileNotFoundException"
                        } else {
                            "java.io.IOException"
                        }
                        .into(),
                    ]
                } else {
                    vec![]
                },
            },
            delegates_to: (r.index() == 1).then_some(MethodRef::new(0)),
        })
    }
    fn exception_covered(&self, exception: &str, declared: &[String]) -> bool {
        declared.iter().any(|t| {
            t == exception
                || (exception == "java.io.FileNotFoundException" && t == "java.io.IOException")
        })
    }
}
impl backend_java::constructors::TrustedConstructorResolver for FamilyResolver {
    fn constructor_class(
        &self,
        name: &str,
    ) -> Option<backend_java::constructors::ConstructorClass> {
        if self.malformed == 9 && name == "Base" {
            return None;
        }
        Some(backend_java::constructors::ConstructorClass {
            name: name.into(),
            superclass: match name {
                "Fixture" => Some("Base".into()),
                "Base" => Some(
                    if self.malformed == 10 {
                        "Fixture"
                    } else {
                        "java.lang.Object"
                    }
                    .into(),
                ),
                "java.lang.Object" => None,
                _ => return None,
            },
            access_flags: if self.malformed == 5 && name == "Base" {
                0x11
            } else {
                1
            },
        })
    }
    fn constructor_access(&self, r: MethodRef) -> Option<u32> {
        Some(if self.malformed == 6 && r.index() == 2 {
            2
        } else if self.malformed == 11 && r.index() == 0 {
            9
        } else {
            1
        })
    }
    fn declared_constructors(&self, _: &str) -> Option<Vec<MethodRef>> {
        Some(
            (0..if self.malformed == 4 { 1 } else { 2 })
                .map(MethodRef::new)
                .collect(),
        )
    }
}

#[test]
fn constructor_branch_final_assignment_is_proven_on_both_return_paths() {
    use backend_java::constructors::{BlankFinal, ConstructorInput, plan_constructor_family};
    for missing_assignment in [false, true] {
        let mut b = Builder::default();
        let receiver = b.parameter();
        let parameter = b.parameter();
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            method: MethodRef::new(2),
            args: vec![receiver, parameter],
            result: None,
        });
        let zero = b.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(0),
        });
        let condition = b.value(ManagedType::Boolean, |result| ManagedInsn::Compare {
            op: managed_ir::ops::CompareOp::GreaterThan,
            result,
            lhs: parameter,
            rhs: zero,
        });
        // Branch identities are wired after the CFG supplies the exact edges.
        b.push(ManagedInsn::Return { value: None });
        let branch = *b.order.last().unwrap();
        let mut cfg = Cfg::new(b.order[0]);
        let mut last = cfg.entry();
        for id in &b.order[1..] {
            let next = cfg.add_block(*id);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        let mut targets = Vec::new();
        for value in [parameter, zero] {
            let ret = b.insns.insert(ManagedInsn::Return { value: None });
            let ret_block = cfg.add_block(ret);
            if missing_assignment && value == zero {
                targets.push(ret_block);
            } else {
                let store = b.insns.insert(ManagedInsn::FieldPut {
                    object: Some(receiver),
                    field: FieldRef::new(0),
                    value,
                });
                let store_block = cfg.add_block(store);
                cfg.add_edge(store_block, ret_block, EdgeKind::Unconditional)
                    .unwrap();
                targets.push(store_block);
            }
        }
        let yes = cfg
            .add_edge(last, targets[0], EdgeKind::BranchTrue)
            .unwrap();
        let no = cfg
            .add_edge(last, targets[1], EdgeKind::BranchFalse)
            .unwrap();
        b.insns[branch] = ManagedInsn::Branch {
            condition: Some(condition),
            then_edge: yes,
            else_edge: Some(no),
        };
        let ir = MethodIr::<SsaForm>::from_parts(cfg, b.insns, b.values);
        let bindings = ParameterBindings {
            receiver: Some(receiver),
            parameters: vec![parameter],
            names: vec![],
        };
        let input = ConstructorInput {
            reference: MethodRef::new(0),
            ir: &ir,
            bindings: &bindings,
        };
        let finals = [BlankFinal {
            reference: FieldRef::new(0),
            writes: [(MethodRef::new(0), if missing_assignment { 1 } else { 2 })].into(),
        }];
        let plans = plan_constructor_family(
            "Fixture",
            &[input],
            &finals,
            &FamilyResolver { malformed: 4 },
        );
        if missing_assignment {
            assert!(plans.is_err());
        } else {
            let source = &plans.unwrap()[0].source;
            assert!(source.contains("super(p0);"));
            assert!(source.contains("if ("));
            compile_run(
                "branch-final",
                &format!(
                    "class Base {{ final int supplied; Base(int n) {{ supplied = n; }} }} public class Fixture extends Base {{ final int number; {source} public static void main(String[] a) {{ Fixture positive = new Fixture(7), negative = new Fixture(-4); if (positive.number != 7 || negative.number != 0 || negative.supplied != -4) throw new AssertionError(); }} }}"
                ),
            );
        }
    }
}

#[test]
fn strict_constructor_family_finals_superargs_thisdelegation_and_malformed_denial() {
    use backend_java::constructors::{BlankFinal, ConstructorInput, plan_constructor_family};
    for malformed in 0..13 {
        let resolver = FamilyResolver { malformed };
        let mut bodies = Vec::new();
        let mut bindings = Vec::new();
        for index in 0..2 {
            let mut b = Builder::default();
            let receiver = b.parameter();
            let parameters = (0..index + 1).map(|_| b.parameter()).collect::<Vec<_>>();
            let argument = if malformed == 12 && index == 0 {
                let one = b.value(ManagedType::Int, |result| ManagedInsn::Const {
                    result,
                    value: Constant::Int(1),
                });
                b.value(ManagedType::Int, |result| ManagedInsn::Arithmetic {
                    op: managed_ir::insn::ArithOp::Add,
                    result,
                    lhs: parameters[index],
                    rhs: one,
                })
            } else {
                parameters[index]
            };
            b.push(ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                method: MethodRef::new(if index == 0 { 2 } else { 0 }),
                args: vec![receiver, argument],
                result: None,
            });
            let count = if index == 0 {
                match malformed {
                    1 => 0,
                    2 => 2,
                    _ => 1,
                }
            } else {
                usize::from(malformed == 3)
            };
            for _ in 0..count {
                b.push(ManagedInsn::FieldPut {
                    field: FieldRef::new(0),
                    object: Some(receiver),
                    value: parameters[index],
                });
            }
            b.push(ManagedInsn::Return { value: None });
            bodies.push(b.finish());
            bindings.push(ParameterBindings {
                receiver: Some(receiver),
                parameters,
                names: vec![],
            });
        }
        let inputs = (0..2)
            .map(|i| ConstructorInput {
                reference: MethodRef::new(i as u32),
                ir: &bodies[i],
                bindings: &bindings[i],
            })
            .collect::<Vec<_>>();
        let finals = [BlankFinal {
            reference: FieldRef::new(0),
            writes: [
                (
                    MethodRef::new(0),
                    match malformed {
                        1 => 0,
                        2 => 2,
                        _ => 1,
                    },
                ),
                (MethodRef::new(1), usize::from(malformed == 3)),
            ]
            .into(),
        }];
        let result = plan_constructor_family("Fixture", &inputs, &finals, &resolver);
        if ![0, 8, 12].contains(&malformed) {
            assert!(result.is_err(), "malformed {malformed}");
            continue;
        }
        let plans = result.unwrap();
        if malformed == 12 {
            assert!(plans[0].first_statement.starts_with("super((long) ("));
        } else {
            assert_eq!(plans[0].first_statement, "super(p0);");
        }
        assert_eq!(plans[1].first_statement, "this(p1);");
        let base = match malformed {
            8 => "Base(int n) throws java.io.FileNotFoundException { base=n; }",
            12 => "Base(int n) { base=-1; } Base(long n) { base=(int)n; }",
            _ => "Base(int n) { base=n; }",
        };
        let expected_base = if malformed == 12 { 18 } else { 17 };
        compile_run(
            "strict-family",
            &format!(
                "class Base {{ final int base; {base} }} public class Fixture extends Base {{ final int number; {} {} public static void main(String[] a) throws Exception {{ Fixture f = new Fixture(3,17); if(f.base != {expected_base} || f.number != 17 || new Fixture(9).number != 9) throw new AssertionError(); }} }}",
                plans[0].source, plans[1].source
            ),
        );
    }
}

#[test]
fn blank_static_final_stays_in_order_and_is_assigned_once_on_jdk17() {
    use backend_java::static_initializer::{
        StaticInitializerContext, emit_static_initializer_with_finals,
    };
    let resolver = FamilyResolver { malformed: 0 };
    let signature = JavaMethod {
        owner: "Fixture".into(),
        name: "<clinit>".into(),
        is_static: true,
        return_type: JavaType::Void,
        parameters: vec![],
        throws: vec![],
    };
    let context = StaticInitializerContext {
        owner: "Fixture".into(),
        writable_static_fields: vec![],
    };
    for count in 0..4 {
        let mut b = Builder::default();
        let n = b.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(23),
        });
        if count == 3 {
            b.value(ManagedType::Int, |result| ManagedInsn::FieldGet {
                result,
                field: FieldRef::new(1),
                object: None,
            });
        }
        for _ in 0..if count == 3 { 1 } else { count } {
            b.push(ManagedInsn::FieldPut {
                field: FieldRef::new(1),
                object: None,
                value: n,
            });
        }
        b.push(ManagedInsn::Return { value: None });
        let result = emit_static_initializer_with_finals(
            &b.finish(),
            &signature,
            &ParameterBindings::default(),
            &resolver,
            &context,
            &[FieldRef::new(1)],
        );
        if count != 1 {
            if count == 3 {
                assert!(
                    matches!(&result, Err(backend_java::Unsupported::Metadata(reason)) if reason.contains("read before assignment"))
                );
            }
            assert!(result.is_err());
            continue;
        }
        let block = result.unwrap();
        assert!(block.contains("STATIC = v0;"));
        assert!(!block.contains("Fixture.STATIC ="));
        compile_run(
            "blank-static",
            &format!(
                "public class Fixture {{ static final int STATIC; {block} public static void main(String[] a) {{ if(STATIC != 23) throw new AssertionError(); }} }}"
            ),
        );
    }
}

struct RealConstructorResolver<'a> {
    dex: &'a DexFile,
    class: &'a ClassDef,
    constructors: Vec<MethodRef>,
}
impl JavaResolver for RealConstructorResolver<'_> {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        Resolver(self.dex).method(r)
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        Resolver(self.dex).field(r)
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        Resolver(self.dex).class(r)
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        Resolver(self.dex).string(r)
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        actual == expected
            || matches!((actual, expected), (JavaType::Class(a), JavaType::Class(b))
            if ((a == "java.lang.Exception" || a == "java.lang.InterruptedException") && b == "java.lang.Throwable") || b == "java.lang.Object")
    }
}
impl ConstructorResolver for RealConstructorResolver<'_> {
    fn initializer(&self, r: MethodRef) -> Option<Initializer> {
        let method = self.method(r)?;
        if method.name != "<init>" {
            return None;
        }
        let delegates_to = self
            .class
            .class_data
            .as_ref()?
            .direct_methods
            .iter()
            .find(|m| m.method_idx == r.index())
            .and_then(|m| m.code.as_ref())
            .and_then(|code| dex_decompiler::decode_method(code).ok())
            .and_then(|instructions| {
                instructions.into_iter().find_map(|i| {
                    let dex_decompiler::Opcode::Supported(info) = i.opcode else {
                        return None;
                    };
                    if ![0x70, 0x76].contains(&info.opcode) {
                        return None;
                    }
                    let target = i.pool_index?;
                    let raw = self.dex.method_id(target).ok()?;
                    (raw.class_idx as u32 == self.class.class_idx
                        && self.dex.string(raw.name_idx).ok()? == "<init>")
                        .then_some(MethodRef::new(target))
                })
            });
        Some(Initializer {
            method,
            delegates_to,
        })
    }
}
impl backend_java::constructors::TrustedConstructorResolver for RealConstructorResolver<'_> {
    fn constructor_class(
        &self,
        name: &str,
    ) -> Option<backend_java::constructors::ConstructorClass> {
        let owner = self.class(ClassRef::new(self.class.class_idx))?;
        let (parent, flags) = match name {
            n if n == owner => (
                Some(Resolver(self.dex).class(ClassRef::new(self.class.superclass_idx))?),
                self.class.access_flags,
            ),
            "java.lang.RuntimeException" => (Some("java.lang.Exception".into()), 1),
            "java.lang.Exception" => (Some("java.lang.Throwable".into()), 1),
            "java.lang.Throwable" => (Some("java.lang.Object".into()), 1),
            "java.lang.Object" => (None, 1),
            _ => return None,
        };
        Some(backend_java::constructors::ConstructorClass {
            name: name.into(),
            superclass: parent,
            access_flags: flags,
        })
    }
    fn constructor_access(&self, r: MethodRef) -> Option<u32> {
        let m = self.method(r)?;
        if m.owner == self.class(ClassRef::new(self.class.class_idx))? {
            self.class
                .class_data
                .as_ref()?
                .direct_methods
                .iter()
                .find(|m| m.method_idx == r.index())
                .map(|m| m.access_flags)
        } else if m.owner == "java.lang.RuntimeException"
            && [
                vec![],
                vec![JavaType::Class("java.lang.String".into())],
                vec![JavaType::Class("java.lang.Throwable".into())],
                vec![
                    JavaType::Class("java.lang.String".into()),
                    JavaType::Class("java.lang.Throwable".into()),
                ],
            ]
            .contains(&m.parameters)
        {
            Some(1)
        } else {
            None
        }
    }
    fn declared_constructors(&self, owner: &str) -> Option<Vec<MethodRef>> {
        (owner == self.class(ClassRef::new(self.class.class_idx))?)
            .then(|| self.constructors.clone())
    }
}

#[test]
fn samsung_original_exception_families_compile_and_run_on_jdk17() {
    let apk = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    );
    let extracted = Command::new("unzip")
        .args(["-p", apk, "classes.dex"])
        .output()
        .unwrap();
    assert!(extracted.status.success());
    let dex = DexFile::from_bytes(&extracted.stdout).unwrap();
    let mut verified = Vec::new();
    for class in &dex.classes {
        let original_owner = Resolver(&dex)
            .class(ClassRef::new(class.class_idx))
            .unwrap();
        if ![
            "com.bumptech.glide.i$a",
            "com.google.firebase.components.s",
            "com.google.firebase.components.v",
        ]
        .contains(&original_owner.as_str())
        {
            continue;
        }
        if Resolver(&dex)
            .class(ClassRef::new(class.superclass_idx))
            .as_deref()
            != Some("java.lang.RuntimeException")
        {
            continue;
        }
        let Some(data) = &class.class_data else {
            continue;
        };
        if !data.instance_fields.is_empty() || !data.static_fields.is_empty() {
            continue;
        }
        let constructors = data
            .direct_methods
            .iter()
            .filter(|m| {
                dex.string(dex.method_id(m.method_idx).unwrap().name_idx)
                    .unwrap()
                    == "<init>"
            })
            .collect::<Vec<_>>();
        if constructors.is_empty() {
            continue;
        }
        let resolver = RealConstructorResolver {
            dex: &dex,
            class,
            constructors: constructors
                .iter()
                .map(|m| MethodRef::new(m.method_idx))
                .collect(),
        };
        let owner = resolver.class(ClassRef::new(class.class_idx)).unwrap();
        let mut ssa = Vec::new();
        let mut bindings = Vec::new();
        let mut checks = Vec::new();
        let mut supported = true;
        for m in &constructors {
            let method = resolver.method(MethodRef::new(m.method_idx)).unwrap();
            if method.parameters.iter().any(|t| !matches!(t, JavaType::Class(n) if n == "java.lang.String" || n == "java.lang.Throwable")) { supported = false; break; }
            let Some(code) = &m.code else {
                supported = false;
                break;
            };
            let Ok(ir) = dex_decompiler::DexPasses::run_ssa_with(
                code,
                &dex_decompiler::DexTypes::new(&dex, Some(m.method_idx)),
            ) else {
                supported = false;
                break;
            };
            let Ok(bound) = decomp::dex_java::parameter_bindings(
                ir.ir(),
                &method,
                code.registers_size,
                code.ins_size,
            ) else {
                supported = false;
                break;
            };
            let args = method
                .parameters
                .iter()
                .map(|t| {
                    if *t == JavaType::Class("java.lang.String".into()) {
                        "\"probe\""
                    } else {
                        "new Exception(\"cause\")"
                    }
                })
                .collect::<Vec<_>>();
            checks.push(format!(
                "if (new {owner}({}).getClass() != {owner}.class) throw new AssertionError();",
                args.join(", ")
            ));
            let expected = if method.parameters.is_empty() {
                "null"
            } else if method.parameters[0] == JavaType::Class("java.lang.String".into()) {
                "\"probe\""
            } else {
                "\"java.lang.Exception: cause\""
            };
            checks.push(format!("if (!java.util.Objects.equals(new {owner}({}).getMessage(), {expected})) throw new AssertionError();", args.join(", ")));
            ssa.push(ir);
            bindings.push(bound);
        }
        assert!(supported, "pinned original family unsupported: {owner}");
        let inputs = constructors
            .iter()
            .enumerate()
            .map(|(i, m)| backend_java::constructors::ConstructorInput {
                reference: MethodRef::new(m.method_idx),
                ir: ssa[i].ir(),
                bindings: &bindings[i],
            })
            .collect::<Vec<_>>();
        let plans =
            backend_java::constructors::plan_constructor_family(&owner, &inputs, &[], &resolver)
                .unwrap();
        check_class_initializer_coverage(
            &dex,
            class,
            &constructors.iter().map(|m| m.method_idx).collect(),
            None,
        )
        .unwrap();
        assert_eq!(plans.len(), constructors.len());
        let simple = owner.rsplit('.').next().unwrap();
        assert!(plans.iter().all(|p| p.constructor_name == simple));
        compile_original_class(
            simple,
            &owner,
            &format!(
                "public class {simple} extends java.lang.RuntimeException {{ {} }}",
                plans
                    .iter()
                    .map(|p| p.source.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            &checks.join("\n"),
        );
        verified.push(owner);
        if verified.len() == 3 {
            break;
        }
    }
    assert_eq!(
        verified.len(),
        3,
        "verified original Samsung identities: {verified:?}"
    );
    eprintln!("verified original Samsung identities: {verified:?}");
}

#[test]
fn real_okcashbag_exception_constructors_compile_and_run_on_jdk17() {
    // Read one named DEX member, lower only these two representative real classes.
    // This is not an APK emission/sweep and does not filter the inventory input.
    let apk = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk"
    );
    let extracted = Command::new("unzip")
        .args(["-p", apk, "classes.dex"])
        .output()
        .unwrap();
    assert!(extracted.status.success());
    let dex = DexFile::from_bytes(&extracted.stdout).unwrap();
    for descriptor in [
        "Landroidx/camera/core/CameraClosedException;",
        "Landroidx/camera/core/impl/utils/InterruptedRuntimeException;",
    ] {
        let class = dex
            .classes
            .iter()
            .find(|c| {
                dex.string(dex.type_id(c.class_idx).unwrap().descriptor_idx)
                    .unwrap()
                    == descriptor
            })
            .unwrap_or_else(|| panic!("missing real fixture {descriptor}"));
        let constructors = class
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods
            .iter()
            .filter(|m| {
                dex.string(dex.method_id(m.method_idx).unwrap().name_idx)
                    .unwrap()
                    == "<init>"
            })
            .collect::<Vec<_>>();
        let resolver = RealConstructorResolver {
            dex: &dex,
            class,
            constructors: constructors
                .iter()
                .map(|m| MethodRef::new(m.method_idx))
                .collect(),
        };
        let mut ssa = Vec::new();
        let mut bindings = Vec::new();
        for m in &constructors {
            let code = m.code.as_ref().unwrap();
            let ir = dex_decompiler::DexPasses::run_ssa_with(
                code,
                &dex_decompiler::DexTypes::new(&dex, Some(m.method_idx)),
            )
            .unwrap();
            let method = resolver.method(MethodRef::new(m.method_idx)).unwrap();
            bindings.push(
                decomp::dex_java::parameter_bindings(
                    ir.ir(),
                    &method,
                    code.registers_size,
                    code.ins_size,
                )
                .unwrap(),
            );
            ssa.push(ir);
        }
        let inputs = constructors
            .iter()
            .enumerate()
            .map(|(i, m)| backend_java::constructors::ConstructorInput {
                reference: MethodRef::new(m.method_idx),
                ir: ssa[i].ir(),
                bindings: &bindings[i],
            })
            .collect::<Vec<_>>();
        let inventory = write_inventory::WriteInventory::scan(std::slice::from_ref(&dex));
        let missing = class_constructor_evidence(
            std::slice::from_ref(&dex),
            0,
            class,
            &inputs[1..],
            None,
            &resolver,
            &inventory,
        )
        .unwrap();
        assert!(!missing.complete_initializer_coverage);
        assert!(
            missing
                .unavailable_initialization_comments()
                .contains("unavailable original constructor")
        );
        assert!(
            !missing
                .unavailable_initialization_comments()
                .contains("super(")
        );
        assert!(
            missing
                .unavailable_constructor_sources
                .contains_key(&constructors[0].method_idx)
        );
        assert!(
            plan_class_initialization_with_inventory(
                std::slice::from_ref(&dex),
                0,
                class,
                &inputs[1..],
                None,
                &resolver,
                &inventory,
            )
            .is_err()
        );
        let parent = plan_class_initialization_with_inventory(
            std::slice::from_ref(&dex),
            0,
            class,
            &inputs,
            None,
            &resolver,
            &inventory,
        )
        .unwrap();
        assert!(parent.encoded.prefix_block.is_empty());
        assert!(parent.static_initializer.is_none());
        let plans = parent.constructors;
        let owner = resolver.class(ClassRef::new(class.class_idx)).unwrap();
        let mut checks = Vec::new();
        for m in &constructors {
            let method = resolver.method(MethodRef::new(m.method_idx)).unwrap();
            let args = method
                .parameters
                .iter()
                .map(|t| match t {
                    JavaType::Class(n) if n == "java.lang.String" => "\"probe\"",
                    JavaType::Class(n)
                        if n == "java.lang.Throwable" || n == "java.lang.Exception" =>
                    {
                        "new Exception(\"cause\")"
                    }
                    JavaType::Class(n) if n == "java.lang.InterruptedException" => {
                        "new InterruptedException(\"cause\")"
                    }
                    _ => panic!("unexpected actual parameter {t:?}"),
                })
                .collect::<Vec<_>>();
            let expected = if args.is_empty() {
                "null"
            } else if method.parameters.first() == Some(&JavaType::Class("java.lang.String".into()))
            {
                "\"probe\""
            } else if method.parameters.first()
                == Some(&JavaType::Class("java.lang.InterruptedException".into()))
            {
                "\"java.lang.InterruptedException: cause\""
            } else {
                "\"java.lang.Exception: cause\""
            };
            checks.push(format!(
                "if(!java.util.Objects.equals(new {owner}({}).getMessage(), {expected})) throw new AssertionError();",
                args.join(", ")
            ));
        }
        compile_original_class(
            if descriptor.contains("Interrupted") {
                "real-interrupted"
            } else {
                "real-camera-closed"
            },
            &owner,
            &format!(
                "public class {} extends java.lang.RuntimeException {{ {} }}",
                owner.rsplit('.').next().unwrap(),
                plans
                    .iter()
                    .map(|p| p.source.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            &checks.join("\n"),
        );
    }
}
impl Builder {
    fn value(&mut self, ty: ManagedType, make: impl FnOnce(ValueId) -> ManagedInsn) -> ValueId {
        let mut result = None;
        let id = self.insns.insert_with_key(|id| {
            let v = self.values.insert(Value::new(id, ty));
            result = Some(v);
            make(v)
        });
        self.order.push(id);
        result.unwrap()
    }
    fn parameter(&mut self) -> ValueId {
        let v = self.value(ManagedType::Unknown, |v| ManagedInsn::Opaque {
            family_tag: 0,
            reads: vec![],
            writes: vec![v],
            effects: EffectFlags::NONE,
        });
        self.order.pop();
        v
    }
    fn push(&mut self, insn: ManagedInsn) {
        self.order.push(self.insns.insert(insn));
    }
    fn finish(self) -> MethodIr<SsaForm> {
        let mut cfg = Cfg::new(self.order[0]);
        let mut last = cfg.entry();
        for id in &self.order[1..] {
            let next = cfg.add_block(*id);
            cfg.add_edge(last, next, EdgeKind::Unconditional).unwrap();
            last = next;
        }
        MethodIr::<SsaForm>::from_parts(cfg, self.insns, self.values)
    }
}

fn enum_fixture(
    pattern: u8,
) -> (
    DexFile,
    ClassDef,
    MethodIr<SsaForm>,
    ParameterBindings,
    MethodIr<SsaForm>,
    MethodRef,
    MethodRef,
) {
    let (mut dex, mut class) = fixture(0x4011);
    let enum_type = ty(&mut dex, "Ljava/lang/Enum;");
    class.superclass_idx = enum_type;
    let string_type = ty(&mut dex, "Ljava/lang/String;");
    let int_type = ty(&mut dex, "I");
    let ctor = element(&mut dex, &mut class, "<init>", "V");
    let clinit = element(&mut dex, &mut class, "<clinit>", "V");
    let data = class.class_data.as_mut().unwrap();
    data.virtual_methods.clear();
    let mut ctor = ctor;
    ctor.access_flags = 0x10002;
    let mut clinit = clinit;
    clinit.access_flags = 0x10008;
    data.direct_methods = vec![ctor.clone(), clinit.clone()];
    let proto = dex.method_ids[ctor.method_idx as usize].proto_idx as usize;
    dex.proto_ids[proto].parameters = Some(dex_format::ids::TypeList {
        types: vec![string_type as u16, int_type as u16],
        span: SPAN,
    });
    let enum_ctor = dex.method_ids.len() as u32;
    let mut raw = dex.method_ids[ctor.method_idx as usize].clone();
    raw.class_idx = enum_type as u16;
    dex.method_ids.push(raw);
    let a = field(&mut dex, &mut class, "A", "LFixture;", 0x4019);
    let b = field(&mut dex, &mut class, "B", "LFixture;", 0x4019);
    let backing = field(&mut dex, &mut class, "$VALUES", "[LFixture;", 0x101a);
    let array_type = dex.field_ids[backing as usize].type_idx as u32;
    let mut constructor = Builder::default();
    let receiver = constructor.parameter();
    let name = constructor.parameter();
    let ordinal = constructor.parameter();
    constructor.push(ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        method: MethodRef::new(enum_ctor),
        args: if pattern == 3 {
            vec![receiver, name, name]
        } else {
            vec![receiver, name, ordinal]
        },
        result: None,
    });
    if pattern == 4 {
        constructor.push(ManagedInsn::FieldPut {
            object: None,
            field: FieldRef::new(a),
            value: receiver,
        });
    }
    constructor.push(ManagedInsn::Return { value: None });
    let bindings = ParameterBindings {
        receiver: Some(receiver),
        parameters: vec![name, ordinal],
        names: vec![],
    };
    let mut init = Builder::default();
    let mut objects = Vec::new();
    for (index, (field, text)) in [(a, "A"), (b, "B")].into_iter().enumerate() {
        let allocated = if pattern == 5 {
            enum_type
        } else {
            class.class_idx
        };
        let object = init.value(ManagedType::Reference(ClassRef::new(allocated)), |result| {
            ManagedInsn::NewInstance {
                result,
                class: ClassRef::new(allocated),
            }
        });
        let string_index = string(&mut dex, if pattern == 1 { "WRONG" } else { text });
        let name = init.value(
            ManagedType::Reference(ClassRef::new(string_type)),
            |result| ManagedInsn::Const {
                result,
                value: Constant::String(StringRef::new(string_index)),
            },
        );
        let ordinal = init.value(ManagedType::Int, |result| ManagedInsn::Const {
            result,
            value: Constant::Int(if pattern == 2 { 9 } else { index as i32 }),
        });
        init.push(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            method: MethodRef::new(ctor.method_idx),
            args: vec![object, name, ordinal],
            result: None,
        });
        init.push(ManagedInsn::FieldPut {
            object: None,
            field: FieldRef::new(field),
            value: object,
        });
        objects.push(object);
    }
    if pattern == 6 {
        objects.reverse();
    }
    let array = init.value(ManagedType::Unknown, |result| ManagedInsn::NewFilledArray {
        result: Some(result),
        target: TypeRef::new(array_type),
        elements: objects,
    });
    init.push(ManagedInsn::FieldPut {
        object: None,
        field: FieldRef::new(backing),
        value: array,
    });
    if pattern == 7 {
        init.push(ManagedInsn::Invoke {
            kind: InvokeKind::Static,
            method: MethodRef::new(clinit.method_idx),
            args: vec![],
            result: None,
        });
    }
    init.push(ManagedInsn::Return { value: None });
    (
        dex,
        class,
        constructor.finish(),
        bindings,
        init.finish(),
        MethodRef::new(ctor.method_idx),
        MethodRef::new(clinit.method_idx),
    )
}

#[test]
fn enum_plan_requires_exact_name_ordinal_chain_array_and_no_extra_effects() {
    for pattern in 0..8 {
        let (dex, class, ctor, bindings, clinit, ctor_ref, clinit_ref) = enum_fixture(pattern);
        if pattern == 0 {
            let p = backend_java::constructors::plan_constructor(
                &ctor,
                ctor_ref,
                "java.lang.Enum",
                &bindings,
                &Resolver(&dex),
            )
            .unwrap();
            assert_eq!(p.first_statement, "super(p0, p1);");
            assert_eq!(p.body, ["return;"]);
        }
        let result = kinds::enum_initialization::plan_enum_initialization(
            &dex,
            &class,
            kinds::enum_initialization::EnumConstructorInput {
                reference: ctor_ref,
                ir: &ctor,
                bindings: &bindings,
            },
            clinit_ref,
            &clinit,
            &Resolver(&dex),
        );
        if pattern == 0 {
            let plan = result.unwrap();
            assert_eq!(plan.header, "public enum Fixture");
            assert_eq!(plan.constants_source, "A, B;");
            assert_eq!(
                plan.constants.iter().map(|c| c.ordinal).collect::<Vec<_>>(),
                vec![0, 1]
            );
            assert_eq!(plan.absorbed_constructor, ctor_ref.index());
            assert_eq!(plan.absorbed_clinit, clinit_ref.index());
        } else {
            assert!(
                matches!(result, Err(Error::EnumConstructorsUnavailable)),
                "pattern {pattern}"
            );
        }
    }
}

#[test]
fn enum_recovery_plan_compiles_and_preserves_runtime_constants_on_jdk17() {
    let (dex, class, ctor, bindings, clinit, ctor_ref, clinit_ref) = enum_fixture(0);
    let plan = kinds::enum_initialization::plan_enum_initialization(
        &dex,
        &class,
        kinds::enum_initialization::EnumConstructorInput {
            reference: ctor_ref,
            ir: &ctor,
            bindings: &bindings,
        },
        clinit_ref,
        &clinit,
        &Resolver(&dex),
    )
    .unwrap();
    let source = format!(
        "{} {{ {} public static void main(String[] args) {{ if (values().length != 2 || !A.name().equals(\"A\") || A.ordinal() != 0 || B.ordinal() != 1 || valueOf(\"B\") != B) throw new AssertionError(); }} }}",
        plan.header, plan.constants_source
    );
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let directory = std::env::temp_dir().join(format!("decomp-enum-init-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("Fixture.java"), &source).unwrap();
    for (binary, args) in [
        ("javac", vec!["--release", "17", "Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let output = Command::new(format!("{home}/bin/{binary}"))
            .args(args)
            .current_dir(&directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{binary}: {}\n{source}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn enum_plan_rejects_class_conversion_payload_state_and_missing_backing() {
    for mutation in 0..4 {
        let (mut dex, mut class, ctor, bindings, clinit, ctor_ref, clinit_ref) = enum_fixture(0);
        match mutation {
            0 => class.access_flags = 0x11,
            1 => {
                field(&mut dex, &mut class, "payload", "I", 2);
            }
            2 => {
                class.class_data.as_mut().unwrap().static_fields.pop();
            }
            3 => class.static_values = Some(vec![value(V::Null)]),
            _ => unreachable!(),
        }
        assert!(
            matches!(
                kinds::enum_initialization::plan_enum_initialization(
                    &dex,
                    &class,
                    kinds::enum_initialization::EnumConstructorInput {
                        reference: ctor_ref,
                        ir: &ctor,
                        bindings: &bindings
                    },
                    clinit_ref,
                    &clinit,
                    &Resolver(&dex)
                ),
                Err(Error::EnumConstructorsUnavailable)
            ),
            "mutation {mutation}"
        );
    }
}

struct DelegatingResolver<'a> {
    base: Resolver<'a>,
    from: MethodRef,
    to: MethodRef,
}
impl JavaResolver for DelegatingResolver<'_> {
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        self.base.method(r)
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        self.base.field(r)
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        self.base.class(r)
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        self.base.string(r)
    }
}
impl ConstructorResolver for DelegatingResolver<'_> {
    fn initializer(&self, r: MethodRef) -> Option<Initializer> {
        let mut result = self.base.initializer(r)?;
        if r == self.from {
            result.delegates_to = Some(self.to);
        }
        Some(result)
    }
}

#[test]
fn trusted_non_object_super_and_this_arguments_compile_and_run_on_jdk17() {
    let (mut dex, mut class) = fixture(1);
    let base_type = ty(&mut dex, "LBase;");
    class.superclass_idx = base_type;
    let int_type = ty(&mut dex, "I") as u16;
    let two = element(&mut dex, &mut class, "<init>", "V");
    let one = element(&mut dex, &mut class, "<init>", "V");
    for (m, types) in [(&two, vec![int_type, int_type]), (&one, vec![int_type])] {
        let proto = dex.method_ids[m.method_idx as usize].proto_idx as usize;
        dex.proto_ids[proto].parameters = Some(dex_format::ids::TypeList { types, span: SPAN });
    }
    let base_ref = MethodRef::new(dex.method_ids.len() as u32);
    let mut base = dex.method_ids[one.method_idx as usize].clone();
    base.class_idx = base_type as u16;
    dex.method_ids.push(base);
    let one_ref = MethodRef::new(one.method_idx);
    let two_ref = MethodRef::new(two.method_idx);
    let resolver = DelegatingResolver {
        base: Resolver(&dex),
        from: two_ref,
        to: one_ref,
    };
    let names = |i| {
        resolver
            .class(ClassRef::new(i))
            .ok_or(Error::InvalidMetadata)
    };
    let superclass = trusted_constructor_superclass(
        &dex,
        &class,
        &[
            declaration("Fixture", Some("Base")),
            declaration("Base", Some("java.lang.Object")),
            declaration("java.lang.Object", None),
        ],
        &names,
    )
    .unwrap();
    let mut sources = Vec::new();
    for (reference, target, count) in [(one_ref, base_ref, 1), (two_ref, one_ref, 2)] {
        let mut b = Builder::default();
        let receiver = b.parameter();
        let parameters = (0..count).map(|_| b.parameter()).collect::<Vec<_>>();
        b.push(ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            method: target,
            args: vec![receiver, parameters[count - 1]],
            result: None,
        });
        b.push(ManagedInsn::Return { value: None });
        let bindings = ParameterBindings {
            receiver: Some(receiver),
            parameters,
            names: vec![],
        };
        let plan = backend_java::constructors::plan_constructor(
            &b.finish(),
            reference,
            &superclass,
            &bindings,
            &resolver,
        )
        .unwrap();
        assert_eq!(
            plan.first_statement,
            if count == 1 {
                "super(p0);"
            } else {
                "this(p1);"
            }
        );
        sources.push(plan.source);
    }
    let source = format!(
        "class Base {{ final int number; Base(int n) {{ number = n; }} }}\npublic class Fixture extends Base {{ {}\npublic static void main(String[] args) {{ if (new Fixture(3,17).number != 17 || new Fixture(9).number != 9) throw new AssertionError(); }} }}",
        sources.join("\n")
    );
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let directory = std::env::temp_dir().join(format!("decomp-ctor-init-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("Fixture.java"), &source).unwrap();
    for (binary, args) in [
        ("javac", vec!["--release", "17", "Fixture.java"]),
        ("java", vec!["-cp", ".", "Fixture"]),
    ] {
        let output = Command::new(format!("{home}/bin/{binary}"))
            .args(args)
            .current_dir(&directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{binary}: {}\n{source}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}
