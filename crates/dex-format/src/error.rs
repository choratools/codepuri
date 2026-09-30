//! Parser error types.

use std::io;

/// Errors produced while reading DEX files or APK containers.
#[derive(Debug, thiserror::Error)]
pub enum DexError {
    /// An underlying I/O operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// A ZIP container operation failed.
    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// A byte range falls outside the input.
    #[error("out of bounds at offset {offset}: requested {size} bytes from {len}-byte input")]
    OutOfBounds {
        /// Start of the invalid range.
        offset: usize,
        /// Requested byte count.
        size: usize,
        /// Available input length.
        len: usize,
    },
    /// Input bytes violate the DEX format.
    #[error("malformed DEX at offset {offset}: {what}")]
    Malformed {
        /// Byte offset where the violation was observed.
        offset: usize,
        /// Description of the violated format rule.
        what: String,
    },
    /// The magic contains a DEX version this crate does not support.
    #[error("unsupported DEX version {0:03}")]
    UnsupportedVersion(u32),
    /// The APK does not contain `AndroidManifest.xml`.
    #[error("APK is missing AndroidManifest.xml")]
    MissingManifest,
    /// The APK contains no conventional `classes*.dex` entries.
    #[error("APK contains no classes*.dex entries")]
    MissingDex,
}

impl DexError {
    pub(crate) fn malformed(offset: usize, what: impl Into<String>) -> Self {
        Self::Malformed {
            offset,
            what: what.into(),
        }
    }
}
