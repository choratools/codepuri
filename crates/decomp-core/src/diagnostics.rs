//! Structured diagnostic values produced by decompiler components.

/// The importance of a diagnostic.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Severity {
    /// Informational context that does not indicate a problem.
    Note,
    /// A recoverable condition that may affect output quality.
    Warning,
    /// A condition that prevents the requested operation from succeeding.
    Error,
}

/// A diagnostic value without output or global-state behavior.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    /// The diagnostic importance.
    pub severity: Severity,
    /// The stable machine-readable diagnostic code.
    pub code: &'static str,
    /// The human-readable diagnostic message.
    pub message: String,
}

impl Diagnostic {
    /// Creates a diagnostic from its stable code and human-readable message.
    pub fn new(severity: Severity, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity,
            code,
            message: message.into(),
        }
    }

    /// Returns the diagnostic severity.
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    /// Returns the stable machine-readable diagnostic code.
    pub const fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the human-readable diagnostic message.
    pub fn message(&self) -> &str {
        &self.message
    }
}
