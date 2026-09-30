//! Lossless DEX parser and APK container layer.
//!
//! Parses Dalvik Executable formats (versions 035–041) into a typed
//! in-memory model and extracts `classes*.dex` entries from APK (ZIP)
//! files. Unknown structures are represented explicitly, never skipped
//! silently. This crate produces the raw model only — IR construction and
//! decompilation live in later crates.

pub mod annotations;
pub mod apk;
pub mod class;
pub mod code;
mod cursor;
pub mod encoded_value;
pub mod error;
pub mod header;
pub mod ids;
pub mod map;
pub mod model;
mod reader;
mod span;
pub mod strings;
pub mod uleb;

pub use error::DexError;
pub use model::DexFile;
pub use span::RawSpan;
