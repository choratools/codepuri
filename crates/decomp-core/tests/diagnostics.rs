//! Structured diagnostic value tests.

use decomp_core::diagnostics::{Diagnostic, Severity};

#[test]
fn diagnostic_is_an_owned_value_without_output_side_effects() {
    let diagnostic = Diagnostic::new(Severity::Warning, "W001", "unreachable block");

    assert_eq!(diagnostic.severity(), Severity::Warning);
    assert_eq!(diagnostic.code(), "W001");
    assert_eq!(diagnostic.message(), "unreachable block");
}
