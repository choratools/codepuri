//! Annotation defaults retain whole-wrapper validation and element-local closure.
#![allow(clippy::unwrap_used)]
use super::*;
use dex_format::{
    RawSpan,
    annotations::{AnnotationItem, AnnotationSet, AnnotationsDirectory, Visibility},
    class::{ClassData, EncodedField},
    encoded_value::{
        AnnotationElement, EncodedAnnotation, EncodedValue, EncodedValueKind as Value,
    },
    ids::{FieldId, MethodId, ProtoId, TypeId, TypeList},
    strings::StringData,
};

const SPAN: RawSpan = RawSpan { offset: 0, size: 0 };

fn string(dex: &mut DexFile, value: &str) -> u32 {
    let index = dex.strings.len() as u32;
    dex.strings.push(StringData {
        value: value.into(),
        utf16_size: value.encode_utf16().count() as u32,
        span: SPAN,
    });
    index
}

fn ty(dex: &mut DexFile, name: &str) -> u32 {
    let descriptor_idx = string(dex, name);
    let index = dex.type_ids.len() as u32;
    dex.type_ids.push(TypeId {
        descriptor_idx,
        span: SPAN,
    });
    index
}

fn value(value: Value) -> EncodedValue {
    EncodedValue { value, span: SPAN }
}

fn mixed_defaults() -> DexFile {
    let mut dex = DexFile::from_bytes(&super::index_tests::fixture::minimal_dex()).unwrap();
    dex.strings.clear();
    dex.type_ids.clear();
    dex.proto_ids.clear();
    dex.method_ids.clear();
    dex.field_ids.clear();
    let mut annotation = dex.classes[0].clone();
    let owner = ty(&mut dex, "Lregression/Mixed;");
    let object = ty(&mut dex, "Ljava/lang/Object;");
    let marker = ty(&mut dex, "Ljava/lang/annotation/Annotation;");
    let wrapper = ty(&mut dex, "Ldalvik/annotation/AnnotationDefault;");
    let mode = ty(&mut dex, "Lregression/Mode;");
    let enum_base = ty(&mut dex, "Ljava/lang/Enum;");
    let modes = ty(&mut dex, "[Lregression/Mode;");
    let int = ty(&mut dex, "I");
    let text = ty(&mut dex, "Ljava/lang/String;");
    let class = ty(&mut dex, "Ljava/lang/Class;");
    let missing = ty(&mut dex, "Lexternal/Missing;");
    let constant_name = string(&mut dex, "A");
    dex.field_ids.push(FieldId {
        class_idx: mode as u16,
        type_idx: mode as u16,
        name_idx: constant_name,
        span: SPAN,
    });
    let label = string(&mut dex, "mixed");
    let specifications = [
        ("mode", mode, Some(Value::Enum(0))),
        ("number", int, Some(Value::Int(7))),
        ("label", text, Some(Value::String(label))),
        ("type", class, Some(Value::Type(text))),
        ("required", int, None),
        (
            "modes",
            modes,
            Some(Value::Array(vec![value(Value::Enum(0))])),
        ),
        ("missing", class, Some(Value::Type(missing))),
    ];
    let mut methods = Vec::new();
    let mut defaults = Vec::new();
    for (name, return_type_idx, default) in specifications {
        let name_idx = string(&mut dex, name);
        let shorty_idx = string(&mut dex, if return_type_idx == int { "I" } else { "L" });
        let proto_idx = dex.proto_ids.len() as u16;
        dex.proto_ids.push(ProtoId {
            shorty_idx,
            return_type_idx,
            parameters_off: 0,
            parameters: None,
            span: SPAN,
        });
        let method_idx = dex.method_ids.len() as u32;
        dex.method_ids.push(MethodId {
            class_idx: owner as u16,
            proto_idx,
            name_idx,
            span: SPAN,
        });
        methods.push(EncodedMethod {
            method_idx,
            access_flags: 0x401,
            code_off: 0,
            code: None,
        });
        if let Some(default) = default {
            defaults.push(AnnotationElement {
                name_idx,
                value: value(default),
            });
        }
    }
    let wrapper_name = string(&mut dex, "value");
    annotation.class_idx = owner;
    annotation.access_flags = 0x2601;
    annotation.superclass_idx = object;
    annotation.interfaces = Some(TypeList {
        types: vec![marker as u16],
        span: SPAN,
    });
    annotation.static_values = None;
    annotation.static_values_off = 0;
    annotation.annotations = Some(AnnotationsDirectory {
        class_annotations: Some(AnnotationSet {
            entries: vec![AnnotationItem {
                visibility: Visibility::System,
                annotation: EncodedAnnotation {
                    type_idx: wrapper,
                    elements: vec![AnnotationElement {
                        name_idx: wrapper_name,
                        value: value(Value::Annotation(EncodedAnnotation {
                            type_idx: owner,
                            elements: defaults,
                            span: SPAN,
                        })),
                    }],
                    span: SPAN,
                },
                span: SPAN,
            }],
            span: SPAN,
        }),
        field_annotations: Vec::new(),
        method_annotations: Vec::new(),
        parameter_annotations: Vec::new(),
        span: SPAN,
    });
    annotation.class_data = Some(ClassData {
        static_fields: Vec::new(),
        instance_fields: Vec::new(),
        direct_methods: Vec::new(),
        virtual_methods: methods,
        span: SPAN,
    });
    let mut enumeration = annotation.clone();
    enumeration.class_idx = mode;
    enumeration.access_flags = 0x4011;
    enumeration.superclass_idx = enum_base;
    enumeration.interfaces = None;
    enumeration.annotations = None;
    enumeration.class_data = Some(ClassData {
        static_fields: vec![EncodedField {
            field_idx: 0,
            access_flags: 0x4019,
        }],
        instance_fields: Vec::new(),
        direct_methods: Vec::new(),
        virtual_methods: Vec::new(),
        span: SPAN,
    });
    dex.classes = vec![annotation, enumeration];
    dex
}

fn aliases(dex: &DexFile) -> JavaAliases {
    aliases_for(std::slice::from_ref(dex))
}

fn aliases_for(files: &[DexFile]) -> JavaAliases {
    let mut inventory = AliasInventory::default();
    for dex in files {
        for index in 0..dex.type_ids.len() as u32 {
            inventory.add_type(descriptor(dex, index).unwrap()).unwrap();
        }
        for index in 0..dex.method_ids.len() as u32 {
            inventory
                .add_method(method_key(dex, index).unwrap())
                .unwrap();
        }
        for index in 0..dex.field_ids.len() as u32 {
            inventory.add_field(field_key(dex, index).unwrap()).unwrap();
        }
    }
    inventory.build(AliasMode::Ascii)
}

fn sources(dex: &DexFile) -> BTreeMap<String, Result<String, String>> {
    let names = aliases(dex);
    let metadata = dex_decompiler::infer::DexMetadata::new(dex);
    let mut resolver = Resolver::new(dex, &names, &metadata);
    resolver.input_owners = dex
        .classes
        .iter()
        .map(|class| descriptor(dex, class.class_idx).unwrap().to_owned())
        .collect();
    let class = &dex.classes[0];
    class
        .class_data
        .as_ref()
        .unwrap()
        .virtual_methods
        .iter()
        .map(|method| {
            let name = method_key(dex, method.method_idx).unwrap().name;
            let source = method_source(class, method, &resolver, ClassKind::Annotation, &mut None)
                .map(|candidate| {
                    assert!(!candidate.body);
                    assert!(candidate.dependencies.is_empty());
                    candidate.source
                });
            (name, source)
        })
        .collect()
}

#[test]
fn mixed_annotation_defaults_keep_independent_elements() {
    let dex = mixed_defaults();
    let sources = sources(&dex);
    for (name, expected) in [
        ("number", "int number() default 7;\n"),
        ("label", "java.lang.String label() default \"mixed\";\n"),
        (
            "type",
            "java.lang.Class type() default java.lang.String.class;\n",
        ),
        ("required", "int required();\n"),
    ] {
        assert_eq!(sources[name].as_ref().unwrap(), expected);
    }
    for name in ["mode", "modes"] {
        assert_eq!(
            sources[name].as_ref().unwrap_err(),
            "annotation default dependency: enum constant source closure unproven"
        );
    }
    assert_eq!(
        sources["missing"].as_ref().unwrap_err(),
        "annotation default dependency: class declaration unavailable"
    );
    // A static enum field in the fixture does not establish an emitted enum body.
    assert_eq!(
        class_declaration(&dex, &dex.classes[1]),
        Err(DeclarationUnsupported::EnumConstructorsUnavailable)
    );
}

#[test]
fn malformed_sibling_default_still_rejects_the_whole_wrapper() {
    for replacement in [Value::Enum(u32::MAX), Value::Int(7)] {
        let mut dex = mixed_defaults();
        let annotations = dex.classes[0]
            .annotations
            .as_mut()
            .unwrap()
            .class_annotations
            .as_mut()
            .unwrap();
        let Value::Annotation(defaults) =
            &mut annotations.entries[0].annotation.elements[0].value.value
        else {
            unreachable!()
        };
        defaults.elements[0].value.value = replacement;
        for (name, source) in sources(&dex) {
            assert!(
                source.unwrap_err().starts_with("annotation declaration:"),
                "{name}"
            );
        }
    }
}

#[test]
fn default_identity_uses_the_original_element_name_after_aliasing() {
    let mut dex = mixed_defaults();
    // A pool-only return overload reserves the base alias before the actual
    // int element. The default wrapper still names the original element.
    let byte = ty(&mut dex, "B");
    let shorty_idx = string(&mut dex, "B");
    let proto_idx = dex.proto_ids.len() as u16;
    dex.proto_ids.push(ProtoId {
        shorty_idx,
        return_type_idx: byte,
        parameters_off: 0,
        parameters: None,
        span: SPAN,
    });
    let mut reference = dex.method_ids[1].clone();
    reference.proto_idx = proto_idx;
    dex.method_ids.push(reference);
    let sources = sources(&dex);
    let source = sources["number"].as_ref().unwrap();
    assert!(!source.starts_with("int number()"), "{source}");
    assert!(source.ends_with("() default 7;\n"), "{source}");
}

#[test]
fn independent_annotation_elements_compile_with_their_original_defaults() {
    let sources = sources(&mixed_defaults());
    let root = std::env::temp_dir().join(format!(
        "decomphify-annotation-defaults-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let members = ["number", "label", "type", "required"]
        .iter()
        .map(|name| sources[*name].as_ref().unwrap().as_str())
        .collect::<String>();
    let source = format!("public @interface Mixed {{\n{members}}}\n");
    let path = root.join("Mixed.java");
    std::fs::write(&path, source).unwrap();
    let javac = std::env::var_os("JAVA_HOME")
        .map(|home| std::path::PathBuf::from(home).join("bin/javac"))
        .unwrap_or_else(|| "javac".into());
    let output = std::process::Command::new(javac)
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires local Samsung Pass APK and JAVA_HOME; exact annotation identities only"]
fn samsung_original_annotation_elements_have_local_default_dependencies() {
    let files = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    // Freeze the complete pool naming inventory without scanning unrelated
    // method bodies for field publication. This witness emits no field/body.
    let names = aliases_for(&files);
    let jdk = std::path::PathBuf::from(std::env::var_os("JAVA_HOME").unwrap());
    let external =
        crate::java_external_metadata::Classpath::from_paths(&[jdk.join("jmods/java.base.jmod")])
            .unwrap();
    let owners: BTreeSet<String> = files
        .iter()
        .flat_map(|dex| {
            dex.classes
                .iter()
                .map(|class| descriptor(dex, class.class_idx).unwrap().to_owned())
        })
        .collect();
    let mut accepted = BTreeSet::new();
    let mut enum_rejected = 0;
    for (pool, dex) in files.iter().enumerate() {
        let metadata = dex_decompiler::infer::DexMetadata::new(dex);
        let mut resolver = Resolver::new(dex, &names, &metadata);
        resolver.inputs = &files;
        resolver.dex_index = pool;
        resolver.input_owners = owners.clone();
        resolver.external = Some(&external);
        for class in &dex.classes {
            if class_kind(class.access_flags) != Ok(ClassKind::Annotation) {
                continue;
            }
            let Ok(defaults) = initializers::plan_annotation_defaults_with(dex, class, &|index| {
                resolver
                    .class(ClassRef::new(index))
                    .ok_or(DeclarationUnsupported::InvalidMetadata)
            }) else {
                continue;
            };
            if defaults.enum_fields.is_empty() {
                continue;
            }
            let methods = class.class_data.as_ref().unwrap();
            for method in methods
                .direct_methods
                .iter()
                .chain(&methods.virtual_methods)
            {
                assert_eq!(method.code_off, 0);
                assert!(method.code.is_none());
                let key = method_key(dex, method.method_idx).unwrap();
                let identity = format!("{}->{}(){}", key.owner, key.name, key.return_type);
                match method_source(class, method, &resolver, ClassKind::Annotation, &mut None) {
                    Ok(candidate) => {
                        assert!(!candidate.body);
                        let has_default = candidate.source.contains(" default ");
                        assert_eq!(
                            has_default,
                            defaults.by_method.contains_key(&method.method_idx)
                        );
                        println!(
                            "pool={pool} method={} {identity}: {}",
                            method.method_idx,
                            candidate.source.trim()
                        );
                        accepted.insert(identity);
                    }
                    Err(reason) => {
                        assert_eq!(
                            reason,
                            "annotation default dependency: enum constant source closure unproven",
                            "{identity}"
                        );
                        enum_rejected += 1;
                    }
                }
            }
        }
    }
    for identity in [
        "Lfe/d;->tag()I",
        "Lp8/m;->pattern()Ljava/lang/String;",
        "Lp8/m;->timezone()Ljava/lang/String;",
        "Lp8/w;->required()Z",
    ] {
        assert!(accepted.contains(identity), "{identity}");
    }
    println!(
        "original annotation elements recovered={} enum-dependent rejected={enum_rejected}",
        accepted.len()
    );
    assert_eq!(accepted.len(), 37);
    assert_eq!(enum_rejected, 22);
}
