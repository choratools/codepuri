//! Compile and exercise the standalone module before facade wiring.
#[path = "../src/java_descriptors.rs"]
mod java_descriptors;

#[test]
fn option_resolver_hook_preserves_typed_parser_behavior() {
    assert_eq!(
        java_descriptors::descriptor_type("I"),
        Some(backend_java::JavaType::Int)
    );
    assert_eq!(java_descriptors::descriptor_type("[V"), None);
}
