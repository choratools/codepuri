// Fixture construction and successful-result assertions intentionally panic on failure.
#![allow(clippy::unwrap_used)]

use crate::output::*;
use std::fs;
use std::path::PathBuf;

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        // Atomic directory creation avoids shared mutable globals and collisions.
        for index in 0..1000 {
            let path = std::env::temp_dir().join(format!(
                "decomp-output-{}-{}-{index}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("temporary directory: {e}"),
            }
        }
        panic!("could not allocate temporary directory");
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn source(descriptor: &str, status: SourceStatus) -> GeneratedSource {
    GeneratedSource::new(
        SourceProvenance {
            descriptor: descriptor.into(),
            origin: Some("classes.dex".into()),
            status,
        },
        "class Example {}\n".into(),
        vec![],
    )
    .unwrap()
}

#[test]
fn deterministic_packages_and_literal_nested_names() {
    for (descriptor, expected) in [
        ("LExample;", "Example.java"),
        ("Lorg/example/Outer$Inner;", "org/example/Outer$Inner.java"),
        ("Lorg/example/Outer$1;", "org/example/Outer$1.java"),
        ("L_pkg/$generated;", "_pkg/$generated.java"),
    ] {
        assert_eq!(
            source_path_from_descriptor(descriptor).unwrap(),
            PathBuf::from(expected)
        );
        assert_eq!(
            source_path_from_descriptor(descriptor).unwrap(),
            source_path_from_descriptor(descriptor).unwrap()
        );
    }
}

#[test]
fn rejects_traversal_reserved_and_unsupported_descriptors() {
    for descriptor in [
        "L../Escape;",
        "Lpkg/../Escape;",
        "L/Absolute;",
        "Lpkg//Name;",
        "Lpkg\\Escape;",
        "LC:Escape;",
        "Lpkg/Name.java;",
        "Lpkg/Name\0;",
        "Lpkg/Name;;",
        "[Lpkg/Name;",
        "I",
        "L;",
        "Lpkg/;",
        "Lclass/Name;",
        "LCON;",
        "Laux/Name;",
        "Lpkg/Lpt1;",
        "Lpkg/9Name;",
        "Lpkg/Ünicode;",
    ] {
        assert!(
            matches!(
                source_path_from_descriptor(descriptor),
                Err(OutputError::InvalidDescriptor(_))
            ),
            "{descriptor:?}"
        );
    }
}

#[test]
fn callback_and_explicit_status_are_independent_of_diagnostics() {
    let mut counts = OutputCounts::default();
    let mut received = Vec::new();
    let mut callback = |s: &GeneratedSource| {
        received.push(s.relative_path.clone());
        counts.record(s.provenance.status);
        Ok(())
    };
    let mut complete = source("LComplete;", SourceStatus::Complete);
    complete.diagnostics.push(SourceDiagnostic {
        severity: DiagnosticSeverity::Warning,
        message: "warning".into(),
    });
    callback.emit(&complete).unwrap();
    callback
        .emit(&source("LFallback;", SourceStatus::Fallback))
        .unwrap();
    assert_eq!(
        counts,
        OutputCounts {
            complete: 1,
            fallback: 1
        }
    );
    assert_eq!(counts.total(), 2);
    assert_eq!(received.len(), 2);
}

#[test]
fn writes_packages_counts_success_and_never_overwrites() {
    let temp = Temp::new();
    let root = temp.0.join("sources");
    let mut sink = DirectorySink::new(&root).unwrap();
    let original = source("Lp/q/Example;", SourceStatus::Complete);
    sink.emit(&original).unwrap();
    let mut duplicate = original.clone();
    duplicate.text = "replacement".into();
    assert!(matches!(
        sink.emit(&duplicate),
        Err(OutputError::DuplicatePath(_))
    ));
    sink.flush_pending().unwrap();
    assert_eq!(
        fs::read_to_string(root.join("p/q/Example.java")).unwrap(),
        original.text
    );
    assert!(matches!(
        sink.emit(&source("Lp/q/example;", SourceStatus::Fallback)),
        Err(OutputError::DuplicatePath(_))
    ));
    sink.emit(&source("Lp/q/Other;", SourceStatus::Fallback))
        .unwrap();
    sink.flush_pending().unwrap();
    assert_eq!(
        sink.counts(),
        OutputCounts {
            complete: 1,
            fallback: 1
        }
    );
    let mut reopened = DirectorySink::new(&root).unwrap();
    assert!(matches!(
        reopened.emit(&duplicate),
        Err(OutputError::DuplicatePath(_))
    ));
    assert!(matches!(
        reopened.emit(&source("LP/q/New;", SourceStatus::Complete)),
        Err(OutputError::DuplicatePath(_))
    ));
    assert_eq!(reopened.counts().total(), 0);
}

#[test]
fn caller_supplied_paths_cannot_escape_or_alias_descriptor() {
    let temp = Temp::new();
    let mut sink = DirectorySink::new(temp.0.join("sources")).unwrap();
    for path in [
        "../Escape.java",
        "/Escape.java",
        "p/../../Escape.java",
        "p/./Example.java",
        "Wrong.java",
        "p\\Example.java",
    ] {
        let mut generated = source("Lp/Example;", SourceStatus::Complete);
        generated.relative_path = PathBuf::from(path);
        assert!(
            matches!(sink.emit(&generated), Err(OutputError::InvalidPath(_))),
            "{path}"
        );
    }
    assert_eq!(sink.counts().total(), 0);
    assert!(!temp.0.join("Escape.java").exists());
    assert_eq!(fs::read_dir(sink.root()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn rejects_symlink_roots_ancestors_directories_and_files() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let outside = temp.0.join("outside");
    fs::create_dir(&outside).unwrap();
    let link = temp.0.join("link");
    symlink(&outside, &link).unwrap();
    assert!(matches!(
        DirectorySink::new(&link),
        Err(OutputError::UnsafePath(_))
    ));
    assert!(matches!(
        DirectorySink::new(link.join("child")),
        Err(OutputError::UnsafePath(_))
    ));
    let mut sink = DirectorySink::new(temp.0.join("sources")).unwrap();
    symlink(&outside, sink.root().join("pkg")).unwrap();
    assert!(matches!(
        sink.emit(&source("Lpkg/Example;", SourceStatus::Complete)),
        Err(OutputError::UnsafePath(_))
    ));
    fs::write(outside.join("Existing.java"), "keep").unwrap();
    symlink(
        outside.join("Existing.java"),
        sink.root().join("Example.java"),
    )
    .unwrap();
    assert!(matches!(
        sink.emit(&source("LExample;", SourceStatus::Complete)),
        Err(OutputError::DuplicatePath(_))
    ));
    assert_eq!(
        fs::read_to_string(outside.join("Existing.java")).unwrap(),
        "keep"
    );
    assert!(!outside.join("Example.java").exists());
    assert_eq!(sink.counts().total(), 0);
}

#[test]
fn shared_parents_are_validated_and_scanned_once() {
    let temp = Temp::new();
    let mut sink = DirectorySink::new(temp.0.join("sources")).unwrap();
    for index in 0..1000 {
        let package = if index % 2 == 0 { "p/q" } else { "p/r" };
        let unit = source(&format!("L{package}/Class{index};"), SourceStatus::Complete);
        sink.emit(&unit).unwrap();
    }
    sink.flush_pending().unwrap();
    for index in 0..1000 {
        let package = if index % 2 == 0 { "p/q" } else { "p/r" };
        let unit = source(&format!("L{package}/Class{index};"), SourceStatus::Complete);
        assert_eq!(
            fs::read_to_string(sink.root().join(&unit.relative_path)).unwrap(),
            unit.text
        );
    }
    // root, p, p/q, p/r: metadata + enumeration scale with distinct parents.
    assert_eq!(sink.cache_stats(), ((4, 4, 3), 4, 1003));
    assert_eq!(sink.counts().total(), 1000);
    println!(
        "1000 files: directory cache name/path payload={} bytes (excluding hash table/allocation overhead and existing duplicate-path set)",
        sink.cache_payload_bytes()
    );
    assert!(matches!(
        sink.emit(&source("LP/q/New;", SourceStatus::Complete)),
        Err(OutputError::DuplicatePath(_))
    ));
    assert!(matches!(
        sink.emit(&source("Lp/Q/New;", SourceStatus::Complete)),
        Err(OutputError::DuplicatePath(_))
    ));
    assert_eq!(sink.cache_stats(), ((4, 4, 3), 4, 1003));
}

#[test]
fn existing_directories_and_case_aliases_are_indexed() {
    let temp = Temp::new();
    fs::create_dir_all(temp.0.join("p/q")).unwrap();
    fs::write(temp.0.join("p/q/Existing.java"), "keep").unwrap();
    fs::write(temp.0.join("p/blocked"), "keep").unwrap();
    let mut sink = DirectorySink::new(&temp.0).unwrap();
    sink.emit(&source("Lp/q/New;", SourceStatus::Complete))
        .unwrap();
    for descriptor in ["Lp/q/existing;", "Lp/q/Existing;", "Lp/Q/Other;"] {
        assert!(matches!(
            sink.emit(&source(descriptor, SourceStatus::Fallback)),
            Err(OutputError::DuplicatePath(_))
        ));
    }
    assert!(matches!(
        sink.emit(&source("Lp/blocked/New;", SourceStatus::Complete)),
        Err(OutputError::UnsafePath(_))
    ));
    sink.flush_pending().unwrap();
    assert_eq!(sink.counts().total(), 1);
    assert_eq!(
        fs::read_to_string(temp.0.join("p/q/Existing.java")).unwrap(),
        "keep"
    );
}

#[test]
fn write_and_flush_errors_do_not_increment_counts() {
    struct FailingWriter {
        fail_write: bool,
        flushed: bool,
    }
    impl std::io::Write for FailingWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.fail_write {
                Err(std::io::Error::other("write failed"))
            } else {
                Ok(bytes.len())
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.flushed = true;
            Err(std::io::Error::other("flush failed"))
        }
    }
    let temp = Temp::new();
    let mut sink = DirectorySink::new(&temp.0).unwrap();
    for fail_write in [true, false] {
        let mut writer = FailingWriter {
            fail_write,
            flushed: false,
        };
        assert!(matches!(
            sink.write_source(&mut writer, &source("LExample;", SourceStatus::Complete)),
            Err(OutputError::Io(_))
        ));
        assert_eq!(writer.flushed, !fail_write);
        assert_eq!(sink.counts().total(), 0);
    }
}

#[cfg(unix)]
#[test]
fn unvalidated_symlink_is_rejected_below_cached_parent() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    fs::create_dir_all(temp.0.join("sources/p")).unwrap();
    fs::create_dir(temp.0.join("outside")).unwrap();
    symlink(temp.0.join("outside"), temp.0.join("sources/p/link")).unwrap();
    let mut sink = DirectorySink::new(temp.0.join("sources")).unwrap();
    sink.emit(&source("Lp/Good;", SourceStatus::Complete))
        .unwrap();
    assert!(matches!(
        sink.emit(&source("Lp/link/Bad;", SourceStatus::Fallback)),
        Err(OutputError::UnsafePath(_))
    ));
    sink.flush_pending().unwrap();
    assert_eq!(sink.counts().total(), 1);
    assert_eq!(fs::read_dir(temp.0.join("outside")).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn multiple_existing_case_spellings_reject_even_exact_match() {
    let temp = Temp::new();
    fs::write(temp.0.join("Example.java"), "first").unwrap();
    fs::write(temp.0.join("example.java"), "second").unwrap();
    let mut sink = DirectorySink::new(&temp.0).unwrap();
    for descriptor in ["LExample;", "Lexample;"] {
        assert!(matches!(
            sink.emit(&source(descriptor, SourceStatus::Complete)),
            Err(OutputError::DuplicatePath(_))
        ));
    }
    assert_eq!(sink.counts().total(), 0);
}
