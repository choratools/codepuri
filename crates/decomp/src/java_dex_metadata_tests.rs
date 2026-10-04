#![allow(clippy::unwrap_used)]
use super::*;
use dex_format::{
    RawSpan,
    annotations::{AnnotationsDirectory, MemberAnnotations},
    class::{ClassData, EncodedField, EncodedMethod},
    encoded_value::{AnnotationElement, EncodedAnnotation, EncodedValue},
    ids::{FieldId, MethodId, ProtoId, TypeId, TypeList},
    strings::StringData,
};
use std::{path::PathBuf, process::Command};
const SPAN: RawSpan = RawSpan { offset: 0, size: 0 };
fn string(dex: &mut DexFile, text: &str) -> u32 {
    let index = dex.strings.len() as u32;
    dex.strings.push(StringData {
        value: text.into(),
        utf16_size: text.encode_utf16().count() as u32,
        span: SPAN,
    });
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
fn sample() -> DexFile {
    let mut dex = DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    dex.classes.truncate(1);
    dex.classes[0].class_idx = 0;
    dex.classes[0].access_flags = 1;
    dex.classes[0].superclass_idx = u32::MAX;
    dex.classes[0].interfaces_off = 0;
    dex.classes[0].interfaces = None;
    dex.classes[0].annotations_off = 0;
    dex.classes[0].annotations = None;
    dex.classes[0].static_values_off = 0;
    dex.classes[0].static_values = None;
    dex.strings.truncate(1);
    dex.type_ids = vec![TypeId {
        descriptor_idx: 0,
        span: SPAN,
    }];
    dex.proto_ids.clear();
    dex.method_ids.clear();
    dex.field_ids.clear();
    dex.strings[0].value = "Lfixture/C;".into();
    let void = ty(&mut dex, "V");
    let int = ty(&mut dex, "I");
    let method = string(&mut dex, "method");
    let field = string(&mut dex, "FIELD");
    dex.proto_ids.push(ProtoId {
        shorty_idx: 0,
        return_type_idx: void,
        parameters_off: 1,
        parameters: Some(TypeList {
            types: vec![int as u16],
            span: SPAN,
        }),
        span: SPAN,
    });
    dex.method_ids.push(MethodId {
        class_idx: 0,
        proto_idx: 0,
        name_idx: method,
        span: SPAN,
    });
    dex.field_ids.push(FieldId {
        class_idx: 0,
        type_idx: int as u16,
        name_idx: field,
        span: SPAN,
    });
    dex.classes[0].class_data_off = 1;
    dex.classes[0].class_data = Some(ClassData {
        static_fields: vec![EncodedField {
            field_idx: 0,
            access_flags: 0x19,
        }],
        instance_fields: vec![],
        direct_methods: vec![EncodedMethod {
            method_idx: 0,
            access_flags: 0x109,
            code_off: 0,
            code: None,
        }],
        virtual_methods: vec![],
        span: SPAN,
    });
    dex
}
fn key(name: &str) -> MethodKey {
    MethodKey {
        owner: "Lfixture/C;".into(),
        name: name.into(),
        parameters: vec!["I".into()],
        return_type: "V".into(),
    }
}
fn annotation_item(
    dex: &mut DexFile,
    kind: &str,
    elements: Vec<(&str, EncodedValueKind)>,
) -> AnnotationItem {
    let type_idx = ty(dex, kind);
    let elements = elements
        .into_iter()
        .map(|(name, value)| AnnotationElement {
            name_idx: string(dex, name),
            value: EncodedValue { value, span: SPAN },
        })
        .collect();
    AnnotationItem {
        visibility: Visibility::System,
        annotation: EncodedAnnotation {
            type_idx,
            elements,
            span: SPAN,
        },
        span: SPAN,
    }
}
fn attach_method(dex: &mut DexFile, annotations: Vec<AnnotationItem>) {
    dex.classes[0].annotations_off = 1;
    dex.classes[0].annotations = Some(AnnotationsDirectory {
        class_annotations: None,
        field_annotations: vec![],
        method_annotations: vec![MemberAnnotations {
            member_idx: 0,
            annotations: AnnotationSet {
                entries: annotations,
                span: SPAN,
            },
        }],
        parameter_annotations: vec![],
        span: SPAN,
    });
}
fn thrown(dex: &mut DexFile) -> AnnotationItem {
    let exception = ty(dex, "Ljava/io/IOException;");
    annotation_item(
        dex,
        "Ldalvik/annotation/Throws;",
        vec![(
            "value",
            EncodedValueKind::Array(vec![EncodedValue {
                value: EncodedValueKind::Type(exception),
                span: SPAN,
            }]),
        )],
    )
}
#[test]
fn dex_metadata_exact_declarations_flags_and_absent_throws() {
    let mut dex = sample();
    let absent = key("referenceOnly");
    let name = string(&mut dex, "referenceOnly");
    dex.method_ids.push(MethodId {
        class_idx: 0,
        proto_idx: 0,
        name_idx: name,
        span: SPAN,
    });
    let metadata = DexMetadata::from_dex_files(&[dex]).unwrap();
    assert_eq!(metadata.owners().collect::<Vec<_>>(), ["Lfixture/C;"]);
    assert_eq!(
        metadata.method(&key("method")),
        Some(MethodMetadata {
            access_flags: 0x109,
            exceptions: Some(vec![])
        })
    );
    assert!(metadata.method(&absent).is_none());
    assert!(
        metadata
            .method(&MethodKey {
                return_type: "I".into(),
                ..key("method")
            })
            .is_none()
    );
    let methods = metadata.declared_methods("Lfixture/C;").unwrap();
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].descriptor, "(I)V");
    assert!(methods[0].code.is_none());
    let field = metadata.declared_fields("Lfixture/C;").unwrap().remove(0);
    assert_eq!(field.access_flags, 0x19);
    assert!(field.constant_value.is_none());
    assert!(field.attributes.is_empty());
}
#[test]
fn dex_metadata_system_throws_and_generic_fragments_are_exact() {
    let mut dex = sample();
    let throws = thrown(&mut dex);
    let a = string(&mut dex, "<T:");
    let b = string(&mut dex, "Ljava/lang/Object;>(I)V");
    let signature = annotation_item(
        &mut dex,
        "Ldalvik/annotation/Signature;",
        vec![(
            "value",
            EncodedValueKind::Array(vec![
                EncodedValue {
                    value: EncodedValueKind::String(a),
                    span: SPAN,
                },
                EncodedValue {
                    value: EncodedValueKind::String(b),
                    span: SPAN,
                },
            ]),
        )],
    );
    attach_method(&mut dex, vec![throws, signature]);
    let metadata = DexMetadata::from_dex_files(&[dex]).unwrap();
    assert_eq!(
        metadata.method(&key("method")).unwrap().exceptions,
        Some(vec!["Ljava/io/IOException;".into()])
    );
    assert_eq!(
        metadata.declared_methods("Lfixture/C;").unwrap()[0]
            .signature
            .as_deref(),
        Some("<T:Ljava/lang/Object;>(I)V")
    );
}
#[test]
fn dex_metadata_malformed_throws_signature_and_incomplete_tables_are_denied() {
    for variant in 0..8 {
        let mut dex = sample();
        let mut throws = thrown(&mut dex);
        match variant {
            0 => throws.visibility = Visibility::Runtime,
            1 => throws.annotation.elements[0].value.value = EncodedValueKind::String(0),
            2 => throws.annotation.elements[0].name_idx = 0,
            3 => {
                throws.annotation.elements[0].value.value =
                    EncodedValueKind::Array(vec![EncodedValue {
                        value: EncodedValueKind::Int(1),
                        span: SPAN,
                    }])
            }
            4 => {
                throws.annotation.elements[0].value.value =
                    EncodedValueKind::Array(vec![EncodedValue {
                        value: EncodedValueKind::Type(999),
                        span: SPAN,
                    }])
            }
            5 => throws.annotation.elements.clear(),
            6 => {
                throws.annotation.elements[0].value.value =
                    EncodedValueKind::Array(vec![EncodedValue {
                        value: EncodedValueKind::Type(2),
                        span: SPAN,
                    }])
            }
            _ => throws
                .annotation
                .elements
                .push(throws.annotation.elements[0].clone()),
        }
        attach_method(&mut dex, vec![throws]);
        assert!(
            DexMetadata::from_dex_files(&[dex]).is_err(),
            "variant {variant}"
        );
    }
    let mut dex = sample();
    let throws = thrown(&mut dex);
    attach_method(&mut dex, vec![throws.clone(), throws]);
    assert!(DexMetadata::from_dex_files(&[dex]).is_err());
    let mut dex = sample();
    let item = annotation_item(
        &mut dex,
        "Ldalvik/annotation/Signature;",
        vec![(
            "value",
            EncodedValueKind::Array(vec![EncodedValue {
                value: EncodedValueKind::Int(1),
                span: SPAN,
            }]),
        )],
    );
    attach_method(&mut dex, vec![item]);
    assert!(DexMetadata::from_dex_files(&[dex]).is_err());
    for table in 0..3 {
        let mut dex = sample();
        match table {
            0 => dex.classes[0].class_data = None,
            1 => dex.classes[0].annotations_off = 1,
            _ => dex.classes[0].interfaces_off = 1,
        };
        assert!(DexMetadata::from_dex_files(&[dex]).is_err());
    }
}
#[test]
fn dex_metadata_duplicate_owners_members_and_foreign_annotations_are_denied() {
    let dex = sample();
    assert!(matches!(
        DexMetadata::from_dex_files(&[dex.clone(), dex]),
        Err(DexMetadataError::DuplicateOwner(_))
    ));
    let mut dex = sample();
    let m = dex.classes[0].class_data.as_ref().unwrap().direct_methods[0].clone();
    dex.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(m);
    assert!(matches!(
        DexMetadata::from_dex_files(&[dex]),
        Err(DexMetadataError::DuplicateMethod(_))
    ));
    let mut dex = sample();
    let f = dex.classes[0].class_data.as_ref().unwrap().static_fields[0].clone();
    dex.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .static_fields
        .push(f);
    assert!(matches!(
        DexMetadata::from_dex_files(&[dex]),
        Err(DexMetadataError::DuplicateField(_))
    ));
    for target in [0, 99] {
        let mut dex = sample();
        let item = thrown(&mut dex);
        attach_method(&mut dex, vec![item]);
        let d = dex.classes[0].annotations.as_mut().unwrap();
        if target == 0 {
            d.method_annotations.push(d.method_annotations[0].clone())
        } else {
            d.method_annotations[0].member_idx = target
        };
        assert!(DexMetadata::from_dex_files(&[dex]).is_err());
    }
    let mut dex = sample();
    dex.method_ids[0].class_idx = 1;
    assert!(DexMetadata::from_dex_files(&[dex]).is_err());
}
#[test]
fn dex_metadata_shared_whole_owner_precedence_never_merges_members() {
    let first = DexMetadata::from_dex_files(&[sample()]).unwrap();
    let mut secondary = sample();
    let index = string(&mut secondary, "second");
    secondary.method_ids[0].name_idx = index;
    secondary.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods[0]
        .access_flags = 0x10a;
    secondary.classes[0].access_flags = 0x11;
    let second = DexMetadata::from_dex_files(&[secondary]).unwrap();
    let chain = MetadataChain::new(vec![
        MetadataProvider::Dex(&first),
        MetadataProvider::Dex(&second),
    ]);
    assert!(chain.method(&key("second")).is_none());
    assert_eq!(chain.method_access(&key("method")), Some(0x109));
    assert_eq!(chain.class_access("Lfixture/C;"), Some(1));
    assert_eq!(chain.declared_methods("Lfixture/C;").unwrap().len(), 1);
    assert_eq!(chain.hierarchy().len(), 1);
    assert!(chain.jvm_class("Lfixture/C;").is_none());
    let reverse = MetadataChain::new(vec![
        MetadataProvider::Dex(&second),
        MetadataProvider::Dex(&first),
    ]);
    assert!(reverse.method(&key("method")).is_none());
    assert_eq!(reverse.method_access(&key("second")), Some(0x10a));
    assert_eq!(reverse.class_access("Lfixture/C;"), Some(0x11));
}
#[test]
fn dex_metadata_member_names_require_genuine_nesting_and_preserve_hierarchy() {
    let mut dex = sample();
    let owner = ty(&mut dex, "Lfixture/C$Inner;");
    let name = string(&mut dex, "Inner");
    let inner = annotation_item(
        &mut dex,
        "Ldalvik/annotation/InnerClass;",
        vec![
            ("name", EncodedValueKind::String(name)),
            ("accessFlags", EncodedValueKind::Int(9)),
        ],
    );
    let enclosing = annotation_item(
        &mut dex,
        "Ldalvik/annotation/EnclosingClass;",
        vec![("value", EncodedValueKind::Type(0))],
    );
    let mut class = dex.classes[0].clone();
    class.class_idx = owner;
    class.class_data_off = 0;
    class.class_data = None;
    class.superclass_idx = 0;
    class.annotations_off = 1;
    class.annotations = Some(AnnotationsDirectory {
        class_annotations: Some(AnnotationSet {
            entries: vec![inner, enclosing],
            span: SPAN,
        }),
        method_annotations: vec![],
        field_annotations: vec![],
        parameter_annotations: vec![],
        span: SPAN,
    });
    dex.classes.push(class);
    let m = DexMetadata::from_dex_files(&[dex.clone()]).unwrap();
    assert_eq!(
        m.member_source_names()
            .get("Lfixture/C$Inner;")
            .map(String::as_str),
        Some("fixture.C.Inner")
    );
    assert_eq!(
        m.class_hierarchy("Lfixture/C$Inner;")[0]
            .superclass
            .as_deref(),
        Some("fixture.C")
    );
    dex.classes[1].annotations_off = 0;
    dex.classes[1].annotations = None;
    assert!(
        DexMetadata::from_dex_files(&[dex])
            .unwrap()
            .member_source_names()
            .is_empty()
    );
}
#[test]
fn dex_metadata_manifest_free_zip_and_raw_dex_loading_is_bounded() {
    use std::io::Write;
    let bytes = include_bytes!("../../../tests/corpus/dex/clean/classes.dex").to_vec();
    let path = std::env::temp_dir().join(format!("decomp-dex-metadata-{}.jar", std::process::id()));
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("classes.dex", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&bytes).unwrap();
    let archive = zip.finish().unwrap().into_inner();
    std::fs::write(&path, archive).unwrap();
    let m = DexMetadata::from_path(&path).unwrap();
    let expected = DexMetadata::from_dex_files(&[DexFile::from_bytes(&bytes).unwrap()]).unwrap();
    assert_eq!(
        m.owners().collect::<Vec<_>>(),
        expected.owners().collect::<Vec<_>>()
    );
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(
        DexMetadata::from_path(&path).unwrap().owners().count(),
        expected.owners().count()
    );
    std::fs::write(&path, b"malformed").unwrap();
    assert!(DexMetadata::from_path(&path).is_err());
    let _ = std::fs::remove_file(path);
}
#[test]
#[ignore = "requires separately acquired authorized Flip5 framework receipt and genuine Java17 JMOD"]
fn dex_metadata_original_flip5_samsung_declarations_and_jdk_object_proof() {
    let root = PathBuf::from("/tmp/opencode/samsung-framework-metadata");
    let path = root.join("framework.jar");
    let receipt = std::fs::read_to_string(root.join("receipt.json")).unwrap();
    assert!(receipt.contains("not_attested"));
    assert!(receipt.contains("F731NKSS6GZI1"));
    let sha = Command::new("sha256sum").arg(&path).output().unwrap();
    assert!(sha.status.success());
    assert!(
        String::from_utf8(sha.stdout)
            .unwrap()
            .starts_with("1d329cf676fc029766bf37780fad61d835881e18ce2da570a3e165ed9f186a0b ")
    );
    let metadata = DexMetadata::from_path(&path).unwrap();
    assert_eq!(metadata.owners().count(), 43973);
    for (owner, name, params, ret, flags) in [
        (
            "Landroid/os/UserHandle;",
            "semGetCallingUserId",
            vec![],
            "I",
            0x19,
        ),
        (
            "Landroid/view/View;",
            "semSetHoverPopupType",
            vec!["I".into()],
            "V",
            1,
        ),
    ] {
        let key = MethodKey {
            owner: owner.into(),
            name: name.into(),
            parameters: params,
            return_type: ret.into(),
        };
        let method = metadata.method(&key).unwrap();
        assert_eq!(method.access_flags, flags);
        assert_eq!(method.exceptions, Some(vec![]));
        let parsed = metadata.declared_methods(owner).unwrap();
        assert_eq!(parsed.iter().filter(|m| m.key == key).count(), 1);
        assert!(parsed.iter().all(|m| m.code.is_none()));
        assert_eq!(
            metadata.class_hierarchy(owner)[0].superclass.as_deref(),
            Some("java.lang.Object")
        );
        println!(
            "actual {key:?} flags={flags:#x} declared={:?}",
            method.exceptions
        );
    }
    assert!(metadata.class_access("Ljava/lang/Object;").is_none());
    let tools = PathBuf::from(std::env::var_os("HOME").unwrap()).join("tools");
    let jdk = std::fs::read_dir(tools)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with("jdk-17"))
        })
        .unwrap();
    let classpath = Classpath::from_paths([jdk.join("jmods/java.base.jmod")]).unwrap();
    let chain = MetadataChain::new(vec![
        MetadataProvider::Jvm(&classpath),
        MetadataProvider::Dex(&metadata),
    ]);
    let object = chain.jvm_class("Ljava/lang/Object;").unwrap();
    assert!(std::ptr::eq(
        object,
        classpath.class("Ljava/lang/Object;").unwrap()
    ));
    assert!(object.object_return_only_evidence().is_some());
    assert!(chain.jvm_class("Landroid/view/View;").is_none());
    assert_eq!(chain.class_hierarchy("Ljava/lang/Object;").len(), 1);
    println!(
        "receipt/current Android16 SDK36, APK build match not attested; DEX is not a javac consumable classpath"
    );
}

#[test]
fn dex_metadata_binary_names_are_preserved_without_source_alias_guesses() {
    let mut dex = sample();
    dex.strings[0].value = "Lfixture/C$-CC;".into();
    let metadata = DexMetadata::from_dex_files(&[dex.clone()]).unwrap();
    let original = MethodKey {
        owner: "Lfixture/C$-CC;".into(),
        ..key("method")
    };
    assert!(metadata.method(&original).is_some());
    assert!(metadata.member_source_names().is_empty());
    assert!(crate::java_descriptors::parse_type_descriptor(&original.owner).is_err());
    dex.strings[0].value = "Lfixture/Renamed;".into();
    assert!(metadata.method(&original).is_some());
    assert!(
        DexMetadata::from_dex_files(&[dex])
            .unwrap()
            .method(&original)
            .is_none()
    );
    for descriptor in ["V", "[V", "II", "[", "Lbad/;", "L;", "Lbad;;"] {
        let mut dex = sample();
        let malformed = ty(&mut dex, descriptor);
        dex.field_ids[0].type_idx = malformed as u16;
        assert!(DexMetadata::from_dex_files(&[dex]).is_err(), "{descriptor}");
    }
}

#[test]
fn class_signature_absence_and_original_fragments_remain_distinct() {
    let mut dex = sample();
    let metadata = DexMetadata::from_dex_files(&[dex.clone()]).unwrap();
    assert_eq!(metadata.class_generic_signature("Lfixture/C;"), Some(None));
    assert_eq!(metadata.class_generic_signature("Lmissing/C;"), None);
    let fragment = string(&mut dex, "<T:Ljava/lang/Object;>Ljava/lang/Object;");
    let item = annotation_item(
        &mut dex,
        "Ldalvik/annotation/Signature;",
        vec![(
            "value",
            EncodedValueKind::Array(vec![EncodedValue {
                value: EncodedValueKind::String(fragment),
                span: SPAN,
            }]),
        )],
    );
    dex.classes[0].annotations_off = 1;
    dex.classes[0].annotations = Some(AnnotationsDirectory {
        class_annotations: Some(AnnotationSet {
            entries: vec![item],
            span: SPAN,
        }),
        method_annotations: vec![],
        field_annotations: vec![],
        parameter_annotations: vec![],
        span: SPAN,
    });
    let metadata = DexMetadata::from_dex_files(&[dex]).unwrap();
    assert_eq!(
        metadata.class_generic_signature("Lfixture/C;"),
        Some(Some("<T:Ljava/lang/Object;>Ljava/lang/Object;".into()))
    );
}

#[test]
fn exact_input_class_index_rejects_duplicate_owner() {
    let files = vec![sample()];
    let index = crate::java_external_metadata::fields::InputIndex::new(&files);
    assert!(index.declared_class("Lfixture/C;").is_some());
    assert!(index.declared_class("Lmissing/C;").is_none());
    let files = vec![sample(), sample()];
    let index = crate::java_external_metadata::fields::InputIndex::new(&files);
    assert!(index.declared_class("Lfixture/C;").is_none());
}
