//! Backend-independent generated sources and conservative filesystem output.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

const COMPACT_COMMENT_DIAGNOSTIC_TAG: &str = "decomp.compact-source-comment/v1";
const DIAGNOSTIC_SIDECAR_HEADER: &str =
    "source_path\tdescriptor\torigin_present\torigin\tstatus\tline\tseverity\tmessage\n";

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
    /// Human-readable diagnostic text. Compact-source comment records use the
    /// versioned encoding decoded by [`decode_compact_source_comment`].
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

/// Remove generated standalone Java diagnostic comments while retaining their
/// original source line and exact UTF-8 line contents in `diagnostics`.
///
/// This operates only at final source delivery, after deferred units and global
/// closure evidence have been attached. Source statements and inline comments
/// are copied byte-for-byte.
pub(crate) fn compact_source_diagnostic_comments(source: &mut GeneratedSource) {
    let mut compacted = String::with_capacity(source.text.len());
    for (index, line) in source.text.split_inclusive('\n').enumerate() {
        if line.trim_start().starts_with("//") {
            source.diagnostics.push(SourceDiagnostic {
                severity: DiagnosticSeverity::Info,
                message: encode_compact_comment(index + 1, line),
            });
        } else {
            compacted.push_str(line);
        }
    }
    source.text = compacted;
}

fn encode_compact_comment(line_number: usize, comment: &str) -> String {
    format!(
        "{COMPACT_COMMENT_DIAGNOSTIC_TAG}\t{line_number}\t{}",
        escape_compact_payload(comment)
    )
}

fn escape_compact_payload(payload: &str) -> String {
    let mut escaped = String::with_capacity(payload.len());
    for ch in payload.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{{{:x}}};", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped
}

/// Decode a compact-source comment diagnostic message.
///
/// Returns the 1-based original line number and exact removed source line,
/// including its original line terminator when present.
pub fn decode_compact_source_comment(message: &str) -> Option<(usize, String)> {
    let (tag, rest) = message.split_once('\t')?;
    if tag != COMPACT_COMMENT_DIAGNOSTIC_TAG {
        return None;
    }
    let (line, payload) = rest.split_once('\t')?;
    let line = line.parse::<usize>().ok().filter(|line| *line > 0)?;
    let mut decoded = String::with_capacity(payload.len());
    let mut chars = payload.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            decoded.push(ch);
            continue;
        }
        match chars.next()? {
            '\\' => decoded.push('\\'),
            't' => decoded.push('\t'),
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut digits = String::new();
                loop {
                    match chars.next()? {
                        '}' => break,
                        digit => digits.push(digit),
                    }
                }
                if chars.next()? != ';' {
                    return None;
                }
                decoded.push(char::from_u32(u32::from_str_radix(&digits, 16).ok()?)?);
            }
            _ => return None,
        }
    }
    decoded
        .trim_start()
        .starts_with("//")
        .then_some((line, decoded))
}

fn escape_tsv_field(field: &str, output: &mut String) {
    for ch in field.chars() {
        match ch {
            '\\' => output.push_str("\\\\"),
            '\t' => output.push_str("\\t"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            ch if ch.is_control() => output.push_str(&format!("\\u{{{:x}}};", ch as u32)),
            ch => output.push(ch),
        }
    }
}

fn write_diagnostic_sidecar_row(
    sidecar: &mut impl Write,
    source: &GeneratedSource,
    diagnostic: &SourceDiagnostic,
) -> Result<(), OutputError> {
    let line = decode_compact_source_comment(&diagnostic.message).map(|(line, _)| line);
    let line = line.map_or_else(String::new, |line| line.to_string());
    let path = source.relative_path.to_string_lossy();
    let origin = source.provenance.origin.as_deref().unwrap_or_default();
    let status = match source.provenance.status {
        SourceStatus::Complete => "complete",
        SourceStatus::Fallback => "fallback",
    };
    let severity = match diagnostic.severity {
        DiagnosticSeverity::Info => "info",
        DiagnosticSeverity::Warning => "warning",
        DiagnosticSeverity::Error => "error",
    };
    let mut row = String::with_capacity(
        source.relative_path.as_os_str().len()
            + source.provenance.descriptor.len()
            + origin.len()
            + diagnostic.message.len()
            + 64,
    );
    for (index, field) in [
        path.as_ref(),
        source.provenance.descriptor.as_str(),
        if source.provenance.origin.is_some() {
            "true"
        } else {
            "false"
        },
        origin,
        status,
        line.as_str(),
        severity,
        diagnostic.message.as_str(),
    ]
    .into_iter()
    .enumerate()
    {
        if index != 0 {
            row.push('\t');
        }
        escape_tsv_field(field, &mut row);
    }
    row.push('\n');
    sidecar.write_all(row.as_bytes())?;
    Ok(())
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

    /// Drain any deferred sink work and report its first failure. Sinks that
    /// write synchronously are already complete when `emit` returns and use
    /// this no-op default; `DirectorySink` overlaps writes on background
    /// threads, so its `emit` success means the unit was validated, created
    /// and queued, and only `flush_pending` returning success proves every
    /// accepted unit reached the filesystem.
    fn flush_pending(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

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

/// One deferred file write: an already-created, exclusively reserved file plus
/// the exact bytes to store. Creation stays synchronous so duplicate-path
/// collisions keep failing at `emit`; byte/flush failures surface from
/// `flush_pending` like every other deferred failure.
struct WriteJob {
    file: File,
    text: String,
    status: SourceStatus,
}

#[derive(Default)]
struct PoolState {
    queue: VecDeque<WriteJob>,
    /// Jobs popped by a writer but not finished yet.
    inflight: usize,
    shutdown: bool,
}

/// Shared between the sink and its background writers. All counters and the
/// first write error are observable only after the queue drains.
struct PoolShared {
    state: Mutex<PoolState>,
    signal: Condvar,
    error: Mutex<Option<io::Error>>,
    counts: Mutex<OutputCounts>,
}

struct PoolHandle {
    shared: Arc<PoolShared>,
    writers: Vec<std::thread::JoinHandle<()>>,
}

impl PoolHandle {
    fn spawn() -> Result<Self, io::Error> {
        let shared = Arc::new(PoolShared {
            state: Mutex::new(PoolState::default()),
            signal: Condvar::new(),
            error: Mutex::new(None),
            counts: Mutex::new(OutputCounts::default()),
        });
        let writers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(1, 4);
        let mut handles = Vec::new();
        for _ in 0..writers {
            let shared = Arc::clone(&shared);
            handles.push(
                std::thread::Builder::new()
                    .name("decomp-sink-writer".into())
                    .spawn(move || write_loop(&shared))?,
            );
        }
        Ok(Self {
            shared,
            writers: handles,
        })
    }

    fn enqueue(&self, file: File, text: String, status: SourceStatus) -> Result<(), OutputError> {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.shutdown {
            return Err(OutputError::Io(io::Error::other(
                "background sink writers stopped",
            )));
        }
        state.queue.push_back(WriteJob { file, text, status });
        self.shared.signal.notify_one();
        Ok(())
    }

    /// Wait until every queued and in-flight write finished; report the first
    /// failure. The queue is left empty and the error slot cleared, so a later
    /// flush observes only newer writes.
    fn flush(&self) -> Result<(), OutputError> {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        while !state.queue.is_empty() || state.inflight > 0 {
            state = self
                .shared
                .signal
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
        let error = self
            .shared
            .error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        match error {
            Some(error) => Err(OutputError::Io(error)),
            None => Ok(()),
        }
    }
}

impl Drop for PoolHandle {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.shutdown = true;
        self.shared.signal.notify_all();
        drop(state);
        for writer in self.writers.drain(..) {
            let _ = writer.join();
        }
    }
}

/// Consumes reserved files until shutdown; never panics on the write path.
fn write_loop(shared: &PoolShared) {
    loop {
        let mut job = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(job) = state.queue.pop_front() {
                    state.inflight += 1;
                    break job;
                }
                if state.shutdown {
                    return;
                }
                state = shared.signal.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        };
        let outcome = write_all_and_flush(&mut job.file, &job.text);
        {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.inflight = state.inflight.saturating_sub(1);
            match outcome {
                Ok(()) => {
                    shared
                        .counts
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .record(job.status);
                }
                Err(error) => {
                    let mut error_slot = shared.error.lock().unwrap_or_else(|e| e.into_inner());
                    if error_slot.is_none() {
                        *error_slot = Some(error);
                    }
                }
            }
            shared.signal.notify_all();
        }
    }
}

fn write_all_and_flush(writer: &mut impl Write, text: &str) -> io::Result<()> {
    writer.write_all(text.as_bytes())?;
    writer.flush()
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
///
/// File writes run on a small background pool: `emit` success proves the source
/// was validated, its file created exclusively, and its bytes queued; a
/// successful `flush_pending` (or `Drop`, which waits) proves every accepted
/// unit reached the filesystem with its exact bytes. Write failures surface
/// from `flush_pending`, not `emit`.
pub struct DirectorySink {
    root: PathBuf,
    paths: HashSet<String>,
    directories: HashMap<PathBuf, CachedDirectory>,
    pool: PoolHandle,
    diagnostic_sidecar: Option<BufWriter<File>>,
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
        Self::new_with_sidecar(root, false)
    }

    /// Open a directory sink that writes every source diagnostic to a
    /// deterministic `diagnostics.tsv` sidecar beneath the output root.
    pub fn with_diagnostic_sidecar(root: impl AsRef<Path>) -> Result<Self, OutputError> {
        Self::new_with_sidecar(root, true)
    }

    fn new_with_sidecar(root: impl AsRef<Path>, write_sidecar: bool) -> Result<Self, OutputError> {
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
        let root = fs::canonicalize(root)?;
        let diagnostic_sidecar = if write_sidecar {
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(root.join("diagnostics.tsv"))?;
            let mut sidecar = BufWriter::new(file);
            sidecar.write_all(DIAGNOSTIC_SIDECAR_HEADER.as_bytes())?;
            Some(sidecar)
        } else {
            None
        };
        Ok(Self {
            root,
            paths: HashSet::new(),
            directories: HashMap::new(),
            pool: PoolHandle::spawn()?,
            diagnostic_sidecar,
            #[cfg(test)]
            directory_operations: (0, 0, 0),
        })
    }

    /// Canonical output root.
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Counts of sources successfully written and flushed by this instance.
    /// Any still-queued writes drain first, so this observes a stable state.
    pub fn counts(&self) -> OutputCounts {
        let _ = self.pool.flush();
        *self
            .pool
            .shared
            .counts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Drain all deferred writes and report the first failure. Only a success
    /// from this method proves every accepted source reached the filesystem.
    pub fn flush_pending(&mut self) -> Result<(), OutputError> {
        self.pool.flush()?;
        if let Some(sidecar) = self.diagnostic_sidecar.as_mut() {
            sidecar.flush()?;
        }
        Ok(())
    }

    /// Synchronous escape hatch used by unit tests; production writes are queued.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn write_source(
        &mut self,
        writer: &mut impl Write,
        source: &GeneratedSource,
    ) -> Result<(), OutputError> {
        write_all_and_flush(writer, &source.text)?;
        self.pool
            .shared
            .counts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .record(source.provenance.status);
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
        let file = OpenOptions::new()
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
        if let Some(sidecar) = self.diagnostic_sidecar.as_mut() {
            for diagnostic in &source.diagnostics {
                write_diagnostic_sidecar_row(sidecar, source, diagnostic)?;
            }
        }
        self.pool
            .enqueue(file, source.text.clone(), source.provenance.status)
    }

    fn flush_pending(&mut self) -> Result<(), OutputError> {
        self.pool.flush()
    }
}
