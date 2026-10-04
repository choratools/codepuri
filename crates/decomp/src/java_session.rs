//! Immutable input and classpath snapshots with reusable generated Java sources.
//!
//! A session amortizes repeated identical exports. Its first render runs the
//! normal decompiler; subsequent exports only replay cached sources. It never
//! upgrades partial output or unresolved dependencies to complete recovery.
use crate::{
    dex_java::{JavaOptions, JavaReport},
    java_external_metadata::{Classpath, ClasspathError},
    output::{GeneratedSource, OutputError, SourceSink},
};
use dex_format::DexFile;

/// Lifetime counters and retained source payload, excluding DEX/metadata and
/// allocator overhead. This is not a process RSS measurement.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SessionStats {
    /// Successfully completed decompiler renders, including after cache clears.
    pub renders: usize,
    /// Successful sink deliveries from an already prepared source snapshot.
    pub replays: usize,
    /// Compilation units currently retained.
    pub sources: usize,
    /// UTF-8 source text bytes currently retained.
    pub source_text_bytes: usize,
    /// Whether even an empty successful export is cached.
    pub ready: bool,
}

struct Snapshot {
    report: JavaReport,
    sources: Vec<GeneratedSource>,
}

/// Owned, fixed-configuration Java export session.
///
/// Inputs and options have no mutable accessors. Explicit classpath files are
/// loaded at construction and never reread, so file changes cannot silently
/// corrupt cache identity. Create a new session to change inputs, selection,
/// workers or classpath. No global cache or cross-process persistence is used.
pub struct JavaSession {
    files: Vec<DexFile>,
    options: JavaOptions,
    classpath: Classpath,
    snapshot: Option<Snapshot>,
    renders: usize,
    replays: usize,
}

impl JavaSession {
    /// Own the supplied immutable inputs and load the explicit classpath now.
    pub fn new(files: Vec<DexFile>, options: JavaOptions) -> Result<Self, ClasspathError> {
        let classpath = Classpath::from_paths(&options.classpath)?;
        Ok(Self {
            files,
            options,
            classpath,
            snapshot: None,
            renders: 0,
            replays: 0,
        })
    }

    /// Complete one render into memory, retaining only a successful export.
    /// Failed rendering leaves no partial snapshot and may be retried.
    pub fn prepare(&mut self) -> Result<&JavaReport, OutputError> {
        if self.snapshot.is_none() {
            let mut sources = Vec::new();
            let mut collect = |source: &GeneratedSource| {
                sources.push(source.clone());
                Ok(())
            };
            let external = (!self.options.classpath.is_empty())
                .then_some(&self.classpath as &dyn crate::java_external_metadata::ExternalMetadata);
            let report = crate::java_integration::emit_with_optional_metadata(
                &self.files,
                &self.options,
                &mut collect,
                None,
                external,
            )?;
            self.snapshot = Some(Snapshot { report, sources });
            self.renders += 1;
        }
        match &self.snapshot {
            Some(snapshot) => Ok(&snapshot.report),
            None => unreachable!("successful preparation installs a snapshot"),
        }
    }

    /// Prepare if necessary, then deliver every original cached unit in order.
    ///
    /// Returns the report only after all units are accepted. A sink failure
    /// stops delivery but keeps the valid render for retry into a fresh sink.
    /// Already accepted sink side effects are not rolled back. Each call replays
    /// from the beginning; DirectorySink callers should use a new output root.
    pub fn emit(&mut self, sink: &mut impl SourceSink) -> Result<JavaReport, OutputError> {
        let warm = self.snapshot.is_some();
        self.prepare()?;
        let Some(snapshot) = &self.snapshot else {
            unreachable!("successful preparation installs a snapshot");
        };
        for source in &snapshot.sources {
            sink.emit(source)?;
        }
        if warm {
            self.replays += 1;
        }
        Ok(snapshot.report.clone())
    }

    /// Read-only access after preparation, without copying source text.
    /// Before preparation this is empty; consult `stats().ready` to distinguish
    /// an unprepared session from a successfully rendered empty selection.
    pub fn sources(&self) -> &[GeneratedSource] {
        self.snapshot.as_ref().map_or(&[], |s| s.sources.as_slice())
    }

    /// Release rendered sources and report, keeping inputs, classpath snapshot
    /// and lifetime counters. The next prepare/emit runs the decompiler again.
    pub fn clear_cache(&mut self) {
        self.snapshot = None;
    }

    /// Inspect exact retained text bytes and completed render/replay counters.
    pub fn stats(&self) -> SessionStats {
        SessionStats {
            renders: self.renders,
            replays: self.replays,
            sources: self.sources().len(),
            source_text_bytes: self.sources().iter().map(|s| s.text.len()).sum(),
            ready: self.snapshot.is_some(),
        }
    }
}
