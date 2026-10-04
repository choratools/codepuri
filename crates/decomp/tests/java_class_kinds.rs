//! Standalone wiring keeps shared facade integration owned by the parent.
// Fixture construction and successful-result assertions intentionally panic on failure.
#![allow(clippy::unwrap_used)]
use decomp::java_class_kinds;
use dex_format::{
    DexFile, RawSpan,
    class::{ClassDef, EncodedMethod},
    ids::TypeList,
};
use java_class_kinds::*;

fn fixture() -> (DexFile, ClassDef, EncodedMethod) {
    let mut dex = DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let mut class = dex.classes[0].clone();
    class.access_flags = 0x601;
    class.interfaces = None;
    class.annotations = None;
    class.superclass_idx = u32::MAX;
    let method = EncodedMethod {
        method_idx: 0,
        access_flags: 0x401,
        code_off: 0,
        code: None,
    };
    dex.method_ids[0].class_idx = class.class_idx as u16;
    let name = dex.method_ids[0].name_idx as usize;
    dex.strings[name].value = "value".into();
    let p = dex.method_ids[0].proto_idx as usize;
    dex.proto_ids[p].parameters = None;
    let int = dex
        .type_ids
        .iter()
        .position(|t| dex.strings[t.descriptor_idx as usize].value == "I")
        .unwrap();
    dex.proto_ids[p].return_type_idx = int as u32;
    (dex, class, method)
}

fn type_index(dex: &DexFile, descriptor: &str) -> u16 {
    dex.type_ids
        .iter()
        .position(|t| dex.strings[t.descriptor_idx as usize].value == descriptor)
        .unwrap() as u16
}

#[test]
fn interface_signature_and_extends_use_real_pools() {
    let (mut dex, mut class, method) = fixture();
    let parent = dex
        .type_ids
        .iter()
        .enumerate()
        .find(|(i, t)| {
            *i != class.class_idx as usize
                && dex.strings[t.descriptor_idx as usize]
                    .value
                    .starts_with('L')
        })
        .unwrap()
        .0 as u16;
    let string = dex.type_ids[parent as usize].descriptor_idx as usize;
    dex.strings[string].value = "Lfixture/Parent;".into();
    class.interfaces = Some(TypeList {
        types: vec![parent],
        span: RawSpan { offset: 0, size: 0 },
    });
    let header = class_declaration(&dex, &class).unwrap();
    assert!(header.starts_with("public interface "));
    assert!(header.ends_with(" extends fixture.Parent"));
    assert_eq!(
        abstract_method_declaration(&dex, &class, &method).unwrap(),
        "public abstract int value();"
    );
    class.access_flags &= !1;
    assert!(
        class_declaration(&dex, &class)
            .unwrap()
            .starts_with("interface ")
    );
}

#[test]
fn annotation_elements_and_implicit_annotation_superinterface() {
    let (mut dex, mut class, method) = fixture();
    class.access_flags = 0x2601;
    let object = type_index(&dex, "Ljava/lang/Object;");
    let string = dex.type_ids[object as usize].descriptor_idx as usize;
    dex.strings[string].value = "Ljava/lang/annotation/Annotation;".into();
    class.interfaces = Some(TypeList {
        types: vec![object],
        span: RawSpan { offset: 0, size: 0 },
    });
    assert!(
        class_declaration(&dex, &class)
            .unwrap()
            .starts_with("public @interface ")
    );
    assert_eq!(
        abstract_method_declaration(&dex, &class, &method).unwrap(),
        "public int value();"
    );
    let p = dex.method_ids[0].proto_idx as usize;
    dex.proto_ids[p].return_type_idx = type_index(&dex, "V") as u32;
    assert_eq!(
        abstract_method_declaration(&dex, &class, &method),
        Err(DeclarationUnsupported::InvalidAnnotationElement)
    );
    dex.proto_ids[p].return_type_idx = type_index(&dex, "I") as u32;
    dex.proto_ids[p].parameters = Some(TypeList {
        types: vec![type_index(&dex, "I")],
        span: RawSpan { offset: 0, size: 0 },
    });
    assert_eq!(
        abstract_method_declaration(&dex, &class, &method),
        Err(DeclarationUnsupported::InvalidAnnotationElement)
    );
    dex.strings[string].value = "Lfixture/Other;".into();
    assert_eq!(
        class_declaration(&dex, &class),
        Err(DeclarationUnsupported::InvalidAnnotationInterfaces)
    );
}

#[test]
fn enum_never_falls_back_to_plain_class() {
    let (dex, mut class, method) = fixture();
    class.access_flags = 0x4001;
    assert_eq!(class_kind(class.access_flags), Ok(ClassKind::Enum));
    assert_eq!(
        class_declaration(&dex, &class),
        Err(DeclarationUnsupported::EnumConstructorsUnavailable)
    );
    assert_eq!(
        abstract_method_declaration(&dex, &class, &method),
        Err(DeclarationUnsupported::EnumConstructorsUnavailable)
    );
    assert_eq!(
        class_kind(0x2000),
        Err(DeclarationUnsupported::InvalidFlags)
    );
    assert_eq!(
        class_kind(0x4200),
        Err(DeclarationUnsupported::InvalidFlags)
    );
}

#[test]
fn rejects_concrete_static_bad_owner_and_invalid_metadata() {
    let (dex, mut class, mut method) = fixture();
    method.access_flags = 1;
    assert_eq!(
        abstract_method_declaration(&dex, &class, &method),
        Err(DeclarationUnsupported::NotAbstractDeclaration)
    );
    method.access_flags = 0x409;
    assert_eq!(
        abstract_method_declaration(&dex, &class, &method),
        Err(DeclarationUnsupported::NotAbstractDeclaration)
    );
    method.access_flags = 0x401;
    method.code_off = 1;
    assert_eq!(
        abstract_method_declaration(&dex, &class, &method),
        Err(DeclarationUnsupported::NotAbstractDeclaration)
    );
    method.code_off = 0;
    class.class_idx = u32::MAX;
    assert_eq!(
        abstract_method_declaration(&dex, &class, &method),
        Err(DeclarationUnsupported::InvalidMetadata)
    );
    assert_eq!(
        class_declaration(&dex, &class),
        Err(DeclarationUnsupported::InvalidMetadata)
    );
}

#[test]
fn ordinary_class_keeps_abstract_access_and_inheritance() {
    let (mut dex, mut class, mut method) = fixture();
    class.access_flags = 0x400;
    let parent = type_index(&dex, "Ljava/lang/Object;");
    let index = dex.type_ids[parent as usize].descriptor_idx as usize;
    dex.strings[index].value = "Lfixture/Base;".into();
    class.superclass_idx = parent as u32;
    let header = class_declaration(&dex, &class).unwrap();
    assert!(header.starts_with("abstract class "));
    assert!(header.ends_with(" extends fixture.Base"));
    method.access_flags = 0x404;
    assert_eq!(
        abstract_method_declaration(&dex, &class, &method).unwrap(),
        "protected abstract int value();"
    );
    class.access_flags = 0x601;
    assert_eq!(
        class_declaration(&dex, &class),
        Err(DeclarationUnsupported::InvalidSuperclass)
    );
    class.access_flags = 2;
    assert_eq!(
        class_declaration(&dex, &class),
        Err(DeclarationUnsupported::InvalidFlags)
    );
}

#[test]
fn annotation_defaults_are_explicitly_unsupported() {
    use dex_format::annotations::{
        AnnotationItem, AnnotationSet, AnnotationsDirectory, Visibility,
    };
    use dex_format::encoded_value::EncodedAnnotation;
    let (mut dex, mut class, _) = fixture();
    class.access_flags = 0x2601;
    let index = type_index(&dex, "Ljava/lang/Object;");
    let string = dex.type_ids[index as usize].descriptor_idx as usize;
    dex.strings[string].value = "Ldalvik/annotation/AnnotationDefault;".into();
    let span = RawSpan { offset: 0, size: 0 };
    class.annotations = Some(AnnotationsDirectory {
        class_annotations: Some(AnnotationSet {
            entries: vec![AnnotationItem {
                visibility: Visibility::System,
                annotation: EncodedAnnotation {
                    type_idx: index as u32,
                    elements: vec![],
                    span,
                },
                span,
            }],
            span,
        }),
        field_annotations: vec![],
        method_annotations: vec![],
        parameter_annotations: vec![],
        span,
    });
    assert_eq!(
        class_declaration(&dex, &class),
        Err(DeclarationUnsupported::AnnotationDefaultsUnavailable)
    );
}

#[test]
fn typed_parameters_and_annotation_return_boundaries() {
    let (mut dex, mut class, method) = fixture();
    let p = dex.method_ids[0].proto_idx as usize;
    let object = type_index(&dex, "Ljava/lang/Object;");
    let s = dex.type_ids[object as usize].descriptor_idx as usize;
    dex.strings[s].value = "J".into();
    dex.proto_ids[p].parameters = Some(TypeList {
        types: vec![type_index(&dex, "J"), type_index(&dex, "I")],
        span: RawSpan { offset: 0, size: 0 },
    });
    assert_eq!(
        abstract_method_declaration(&dex, &class, &method).unwrap(),
        "public abstract int value(long p0, int p1);"
    );
    dex.proto_ids[p].parameters = None;
    class.access_flags = 0x2601;
    dex.strings[s].value = "[I".into();
    dex.proto_ids[p].return_type_idx = object as u32;
    assert_eq!(
        abstract_method_declaration(&dex, &class, &method).unwrap(),
        "public int[] value();"
    );
    dex.strings[s].value = "[[I".into();
    assert_eq!(
        abstract_method_declaration(&dex, &class, &method),
        Err(DeclarationUnsupported::InvalidAnnotationElement)
    );
    dex.strings[s].value = "Lfixture/Unknown;".into();
    assert_eq!(
        abstract_method_declaration(&dex, &class, &method),
        Err(DeclarationUnsupported::InvalidAnnotationElement)
    );
    class.access_flags = 1;
    assert_eq!(
        abstract_method_declaration(&dex, &class, &method),
        Err(DeclarationUnsupported::NotAbstractDeclaration)
    );
}
