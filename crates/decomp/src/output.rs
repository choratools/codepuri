//! Backend-independent generated sources and conservative filesystem output.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Quality of generated output, supplied explicitly by the producer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceStatus {
    /// The producer reports fully generated source.
    Complete,
    /// The producer used a partial or fallback representation.
    Fallback,
}

/// Origin of a source. No backend-specific types are required.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceProvenance {
    /// Original DEX class descriptor.
    pub descriptor: String,
    /// Optional origin such as `classes2.dex`.
    pub origin: Option<String>,
    /// Explicit quality; never inferred from diagnostics or text.
    pub status: SourceStatus,
}

/// Importance of a producer diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticSeverity {
    /// Informational context.
    Info,
    /// A nonfatal concern.
    Warning,
    /// A producer error associated with this source.
    Error,
}

/// Backend-independent diagnostic attached to a source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceDiagnostic {
    /// Importance of the message.
    pub severity: DiagnosticSeverity,
    /// Human-readable diagnostic text.
    pub message: String,
}

/// One generated compilation unit delivered to a sink.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedSource {
    source_descriptor: Option<String>,
    /// Relative source path; directory sinks validate against the descriptor.
    pub relative_path: PathBuf,
    /// Generated UTF-8 source text.
    pub text: String,
    /// Original class and explicit completion status.
    pub provenance: SourceProvenance,
    /// Producer diagnostics, without implied completion status.
    pub diagnostics: Vec<SourceDiagnostic>,
}

impl GeneratedSource {
    /// Derive a deterministic path without inferring nesting from `$`.
    pub fn new(
        provenance: SourceProvenance,
        text: String,
        diagnostics: Vec<SourceDiagnostic>,
    ) -> Result<Self, OutputError> {
        let relative_path = source_path_from_descriptor(&provenance.descriptor)?;
        Ok(Self {
            source_descriptor: None,
            relative_path,
            text,
            provenance,
            diagnostics,
        })
    }
    /// Construct a registry-aliased unit while retaining its genuine provenance.
    pub fn with_source_name(
        provenance: SourceProvenance,
        name: &str,
        text: String,
        diagnostics: Vec<SourceDiagnostic>,
    ) -> Result<Self, OutputError> {
        let descriptor = format!("L{};", name.replace('.', "/"));
        let relative_path = source_path_from_descriptor(&descriptor)?;
        Ok(Self {
            source_descriptor: Some(descriptor),
            relative_path,
            text,
            provenance,
            diagnostics,
        })
    }
}

/// Counts of successfully accepted sources by explicit quality.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OutputCounts {
    /// Number of complete sources.
    pub complete: usize,
    /// Number of fallback sources.
    pub fallback: usize,
}

impl OutputCounts {
    /// Number of accepted sources of either quality.
    pub fn total(self) -> usize {
        self.complete + self.fallback
    }

    /// Record a source only after its sink has accepted it.
    pub fn record(&mut self, status: SourceStatus) {
        match status {
            SourceStatus::Complete => self.complete += 1,
            SourceStatus::Fallback => self.fallback += 1,
        }
    }
}

/// Source validation or output failure.
#[derive(Debug)]
pub enum OutputError {
    /// An export configuration is unsupported.
    InvalidOptions(String),
    /// The descriptor cannot be mapped by the supported naming policy.
    InvalidDescriptor(String),
    /// A supplied path does not match the descriptor-derived path.
    InvalidPath(PathBuf),
    /// An existing file or case-insensitive alias would be overwritten.
    DuplicatePath(PathBuf),
    /// A directory chain contains a symlink or non-directory.
    UnsafePath(PathBuf),
    /// Underlying filesystem or callback I/O error.
    Io(io::Error),
}

impl fmt::Display for OutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOptions(message) => write!(f, "invalid Java export options: {message}"),
            Self::InvalidDescriptor(d) => {
                write!(f, "invalid or unsupported class descriptor: {d:?}")
            }
            Self::InvalidPath(p) => write!(f, "invalid source path: {}", p.display()),
            Self::DuplicatePath(p) => write!(f, "source path already exists: {}", p.display()),
            Self::UnsafePath(p) => write!(
                f,
                "symlink or non-directory in output path: {}",
                p.display()
            ),
            Self::Io(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for OutputError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for OutputError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A streaming sink. Closures implementing the same signature are sinks too.
pub trait SourceSink {
    /// Accept one source or return a failure to the producer.
    fn emit(&mut self, source: &GeneratedSource) -> Result<(), OutputError>;

    /// Transfer a final source whose publication needs no intermediate receipt.
    /// Export delivery adapters may queue it; export success still requires all
    /// queued sources to be acknowledged by the underlying sink.
    fn emit_owned(&mut self, source: GeneratedSource) -> Result<(), OutputError> {
        self.emit(&source)
    }
}

impl<F> SourceSink for F
where
    F: FnMut(&GeneratedSource) -> Result<(), OutputError>,
{
    fn emit(&mut self, source: &GeneratedSource) -> Result<(), OutputError> {
        self(source)
    }
}

/// Strict, injective mapping for supported ASCII Java identifiers.
///
/// Unsupported Unicode identifiers, Java keywords, Windows device names,
/// empty segments, separators, and traversal are rejected rather than renamed.
/// `$` is preserved literally, including in nested/anonymous class names.
pub fn source_path_from_descriptor(descriptor: &str) -> Result<PathBuf, OutputError> {
    let body = descriptor
        .strip_prefix('L')
        .and_then(|s| s.strip_suffix(';'))
        .ok_or_else(|| OutputError::InvalidDescriptor(descriptor.to_owned()))?;
    let segments: Vec<_> = body.split('/').collect();
    if segments.iter().any(|s| !valid_identifier(s)) {
        return Err(OutputError::InvalidDescriptor(descriptor.to_owned()));
    }
    let mut path = PathBuf::new();
    for segment in &segments[..segments.len() - 1] {
        path.push(segment);
    }
    path.push(format!("{}.java", segments[segments.len() - 1]));
    Ok(path)
}

fn valid_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == b'_' || first == b'$')
        || !bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'$')
    {
        return false;
    }
    let upper = name.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || (upper.len() == 4
        && (upper.starts_with("COM") || upper.starts_with("LPT"))
        && matches!(upper.as_bytes()[3], b'1'..=b'9'))
    {
        return false;
    }
    !matches!(
        name,
        "abstract"
            | "assert"
            | "boolean"
            | "break"
            | "byte"
            | "case"
            | "catch"
            | "char"
            | "class"
            | "const"
            | "continue"
            | "default"
            | "do"
            | "double"
            | "else"
            | "enum"
            | "extends"
            | "final"
            | "finally"
            | "float"
            | "for"
            | "goto"
            | "if"
            | "implements"
            | "import"
            | "instanceof"
            | "int"
            | "interface"
            | "long"
            | "native"
            | "new"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "short"
            | "static"
            | "strictfp"
            | "super"
            | "switch"
            | "synchronized"
            | "this"
            | "throw"
            | "throws"
            | "transient"
            | "try"
            | "void"
            | "volatile"
            | "while"
            | "true"
            | "false"
            | "null"
            | "_"
    )
}

/// Writes new source files beneath an explicit root, never overwriting files.
///
/// Root ancestors must already exist; the root itself may be created. Symlinks
/// are rejected in the root and source directory chains. Case-insensitive path
/// collisions are rejected even on case-sensitive filesystems for portability.
/// The caller must exclusively control the root and its ancestors during writes:
/// portable `std` checks cannot prevent concurrent directory replacement.
/// Validated directories and child names are cached for this sink's lifetime.
/// Exclusive control includes all descendants: callers must not add, remove, or
/// replace entries behind the sink after their directory has been cached. Root
/// ancestors are validated at construction, and source directories on first use;
/// this is not a defense against concurrent filesystem mutation (TOCTOU).
pub struct DirectorySink {
    root: PathBuf,
    paths: HashSet<String>,
    directories: HashMap<PathBuf, CachedDirectory>,
    counts: OutputCounts,
    #[cfg(test)]
    directory_operations: (usize, usize, usize),
}

#[derive(Default)]
struct CachedDirectory {
    // None means multiple existing spellings share this case-insensitive key.
    names: HashMap<String, Option<OsString>>,
}

impl CachedDirectory {
    fn insert(&mut self, name: OsString) {
        let key = name.to_string_lossy().to_ascii_lowercase();
        self.names
            .entry(key)
            .and_modify(|existing| {
                if existing.as_ref() != Some(&name) {
                    *existing = None;
                }
            })
            .or_insert(Some(name));
    }

    fn rejects(&self, name: &std::ffi::OsStr) -> bool {
        self.names
            .get(&name.to_string_lossy().to_ascii_lowercase())
            .is_some_and(|existing| existing.as_deref() != Some(name))
    }
}

impl DirectorySink {
    /// Open an explicit output root, creating its final directory if absent.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, OutputError> {
        let root = if root.as_ref().is_absolute() {
            root.as_ref().to_owned()
        } else {
            std::env::current_dir()?.join(root)
        };
        if root
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(OutputError::UnsafePath(root));
        }
        let mut current = PathBuf::new();
        let components: Vec<_> = root.components().collect();
        for (index, component) in components.iter().enumerate() {
            current.push(component.as_os_str());
            match fs::symlink_metadata(&current) {
                Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
                Ok(_) => return Err(OutputError::UnsafePath(current)),
                Err(e) if e.kind() == io::ErrorKind::NotFound && index + 1 == components.len() => {
                    fs::create_dir(&current)?;
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(Self {
            root: fs::canonicalize(root)?,
            paths: HashSet::new(),
            directories: HashMap::new(),
            counts: OutputCounts::default(),
            #[cfg(test)]
            directory_operations: (0, 0, 0),
        })
    }

    /// Canonical output root.
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Counts of sources successfully written and flushed by this instance.
    pub fn counts(&self) -> OutputCounts {
        self.counts
    }

    pub(crate) fn write_source(
        &mut self,
        writer: &mut impl Write,
        source: &GeneratedSource,
    ) -> Result<(), OutputError> {
        writer.write_all(source.text.as_bytes())?;
        writer.flush()?;
        self.counts.record(source.provenance.status);
        Ok(())
    }

    fn ensure_directory(&mut self, relative: &Path, output: &Path) -> Result<(), OutputError> {
        if self.directories.contains_key(relative) {
            return Ok(());
        }
        if let Some(parent) = relative.parent() {
            self.ensure_directory(parent, output)?;
            let name = relative
                .file_name()
                .ok_or_else(|| OutputError::UnsafePath(relative.into()))?;
            if self.directories[parent].rejects(name) {
                return Err(OutputError::DuplicatePath(output.into()));
            }
        }
        let directory = self.root.join(relative);
        #[cfg(test)]
        {
            self.directory_operations.0 += 1;
        }
        match fs::symlink_metadata(&directory) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => return Err(OutputError::UnsafePath(directory)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&directory)?;
                #[cfg(test)]
                {
                    self.directory_operations.2 += 1;
                }
            }
            Err(e) => return Err(e.into()),
        }
        let mut cached = CachedDirectory::default();
        #[cfg(test)]
        {
            self.directory_operations.1 += 1;
        }
        for entry in fs::read_dir(&directory)? {
            cached.insert(entry?.file_name());
        }
        if let Some(parent) = relative.parent()
            && let Some(cache) = self.directories.get_mut(parent)
        {
            cache.insert(
                relative
                    .file_name()
                    .ok_or_else(|| OutputError::UnsafePath(relative.into()))?
                    .to_owned(),
            );
        }
        self.directories.insert(relative.into(), cached);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn cache_stats(&self) -> ((usize, usize, usize), usize, usize) {
        (
            self.directory_operations,
            self.directories.len(),
            self.directories.values().map(|d| d.names.len()).sum(),
        )
    }

    #[cfg(test)]
    pub(crate) fn cache_payload_bytes(&self) -> usize {
        self.directories
            .iter()
            .map(|(path, directory)| {
                path.as_os_str().len()
                    + directory
                        .names
                        .iter()
                        .map(|(key, name)| key.len() + name.as_ref().map_or(0, |name| name.len()))
                        .sum::<usize>()
            })
            .sum()
    }
}

impl SourceSink for DirectorySink {
    fn emit(&mut self, source: &GeneratedSource) -> Result<(), OutputError> {
        let expected = source_path_from_descriptor(
            source
                .source_descriptor
                .as_deref()
                .unwrap_or(&source.provenance.descriptor),
        )?;
        // Compare raw spelling as well as Path equality (which normalizes `.`).
        if source.relative_path.as_os_str() != expected.as_os_str() {
            return Err(OutputError::InvalidPath(source.relative_path.clone()));
        }
        let key = expected.to_string_lossy().to_ascii_lowercase();
        if self.paths.contains(&key) {
            return Err(OutputError::DuplicatePath(expected));
        }
        let parent = expected.parent().unwrap_or(Path::new(""));
        self.ensure_directory(parent, &expected)?;
        let name = expected
            .file_name()
            .ok_or_else(|| OutputError::InvalidPath(expected.clone()))?;
        if self.directories[parent].rejects(name) {
            return Err(OutputError::DuplicatePath(expected));
        }
        let destination = self.root.join(&expected);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|e| {
                if e.kind() == io::ErrorKind::AlreadyExists {
                    OutputError::DuplicatePath(expected.clone())
                } else {
                    OutputError::Io(e)
                }
            })?;
        // Reserve immediately: failed writes leave a partial file that must not
        // be silently reused. Counts track only successfully flushed sources.
        self.paths.insert(key);
        if let Some(cache) = self.directories.get_mut(parent) {
            cache.insert(name.to_owned());
        }
        self.write_source(&mut file, source)
    }
}
