//! Hierarchy ingestion and missing-evidence boundaries.
#![allow(clippy::unwrap_used)]
pub use backend_java::JavaType;
use decomp::{java_hierarchy, type_compatibility};
use type_compatibility::*;
#[test]
fn dex_definitions_and_supplied_classpath() {
    let dex = dex_format::DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let h = java_hierarchy::build_hierarchy(
        &[&dex],
        [Declaration {
            name: "external.Child".into(),
            superclass: Some("external.Base".into()),
            interfaces: vec!["external.I".into()],
            is_interface: false,
        }],
    )
    .unwrap();
    assert_eq!(
        h.assignable(
            &java_hierarchy::class_type("external.Child"),
            &java_hierarchy::class_type("external.I")
        ),
        Compatibility::Assignable
    );
    assert!(matches!(
        h.assignable(
            &java_hierarchy::class_type("external.Child"),
            &java_hierarchy::class_type("Unrelated")
        ),
        Compatibility::Indeterminate(_)
    ));
    assert!(
        java_hierarchy::build_hierarchy(
            &[],
            [
                Declaration {
                    name: "A".into(),
                    superclass: None,
                    interfaces: vec![],
                    is_interface: true
                },
                Declaration {
                    name: "A".into(),
                    superclass: None,
                    interfaces: vec![],
                    is_interface: false
                }
            ]
        )
        .is_err()
    );
}
