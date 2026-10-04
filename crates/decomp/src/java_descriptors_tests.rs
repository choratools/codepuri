// Fixture construction and successful-result assertions intentionally panic on failure.
#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn external_literal_class_names_and_rank_are_preserved() {
    assert_eq!(
        parse_type_descriptor("Lmissing/Outer$Inner;").unwrap(),
        JavaType::Class("missing.Outer$Inner".into())
    );
    assert_eq!(
        parse_type_descriptor("[[Ljava/util/List;").unwrap(),
        JavaType::Array(Box::new(JavaType::Array(Box::new(JavaType::Class(
            "java.util.List".into()
        )))))
    );
    for (descriptor, ty) in [
        ("V", JavaType::Void),
        ("Z", JavaType::Boolean),
        ("B", JavaType::Byte),
        ("S", JavaType::Short),
        ("C", JavaType::Char),
        ("I", JavaType::Int),
        ("J", JavaType::Long),
        ("F", JavaType::Float),
        ("D", JavaType::Double),
    ] {
        assert_eq!(parse_type_descriptor(descriptor).unwrap(), ty);
    }
}

#[test]
fn malformed_and_unsupported_types_have_typed_errors() {
    for descriptor in [
        "",
        "[",
        "[V",
        "[[V",
        "II",
        "Vx",
        "L;",
        "L/a;",
        "La/;",
        "La//b;",
        "La.b;",
        "La/class;",
        "L1bad;",
        "La-b;",
        "Lé;",
        "La<TT;>;",
        "La",
        "La;;",
        "(I)V",
    ] {
        assert!(
            matches!(
                parse_type_descriptor(descriptor),
                Err(Unsupported::Metadata(_))
            ),
            "{descriptor}"
        );
    }
    assert!(parse_type_descriptor(&format!("{}I", "[".repeat(255))).is_ok());
    assert!(parse_type_descriptor(&format!("{}I", "[".repeat(256))).is_err());
}

#[test]
fn full_method_descriptors_use_logical_arity_and_erased_types() {
    let signature =
        parse_method_descriptor("(J[[DLexternal/Thing$Literal;)[Ljava/util/List;").unwrap();
    assert_eq!(signature.parameters.len(), 3);
    assert_eq!(signature.parameters[0], JavaType::Long);
    assert_eq!(
        signature.parameters[1],
        JavaType::Array(Box::new(JavaType::Array(Box::new(JavaType::Double))))
    );
    assert_eq!(
        signature.parameters[2],
        JavaType::Class("external.Thing$Literal".into())
    );
    assert_eq!(
        signature.return_type,
        JavaType::Array(Box::new(JavaType::Class("java.util.List".into())))
    );
    assert!(signature.validate_arity(3).is_ok());
    assert!(signature.validate_arity(4).is_err());
    assert_eq!(parse_method_descriptor("()V").unwrap().parameters.len(), 0);
    for invalid in [
        "", "I)V", "(V)V", "([V)V", "(I", "(I)", "(I)Vx", "()VV", "()", "((I)V",
    ] {
        assert!(parse_method_descriptor(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn metadata_signatures_do_not_require_class_definitions() {
    let mut dex = DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let expected = (0..dex.method_ids.len())
        .filter_map(|i| {
            method_from_pool(&dex, i as u32, false)
                .ok()
                .map(|m| (i as u32, m))
        })
        .collect::<Vec<_>>();
    assert!(!expected.is_empty());
    dex.classes.clear();
    for (index, original) in expected {
        let actual = method_from_pool(&dex, index, false).unwrap();
        assert_eq!(actual.owner, original.owner);
        assert_eq!(actual.name, original.name);
        assert_eq!(actual.parameters, original.parameters);
        assert_eq!(actual.return_type, original.return_type);
    }
    assert!(type_from_pool(&dex, u32::MAX).is_err());
    assert!(method_from_pool(&dex, u32::MAX, false).is_err());
}

#[test]
fn source_names_are_not_silently_aliased() {
    assert_eq!(source_identifier("Outer$Inner").unwrap(), "Outer$Inner");
    for name in ["", "class", "_", "bad-name", "1Thing", "a.b"] {
        assert!(source_identifier(name).is_err());
    }
}
