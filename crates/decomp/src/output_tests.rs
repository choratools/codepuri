// Fixture construction and successful-result assertions intentionally panic on failure.
#![allow(clippy::unwrap_used)]

use crate::output::*;
use std::fs;
use std::path::{Path, PathBuf};

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
    assert!(!root.join("diagnostics.tsv").exists());
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
fn compact_source_comments_round_trip_through_deterministic_sidecar() {
    let original = "package p;\n\t// first\tcomment\r\nclass Example {\n  int value; // inline comment\n  // last comment\n}\n// eof without terminator";
    let mut generated = source("Lp/Example;", SourceStatus::Complete);
    generated.text = original.into();
    generated.diagnostics.push(SourceDiagnostic {
        severity: DiagnosticSeverity::Warning,
        message: "existing\tmessage\\with\ncontrol".into(),
    });
    compact_source_diagnostic_comments(&mut generated);
    assert_eq!(
        generated.text,
        "package p;\nclass Example {\n  int value; // inline comment\n}\n"
    );
    assert_eq!(generated.diagnostics.len(), 4);

    let emit = |root: &Path| -> String {
        let mut sink = DirectorySink::with_diagnostic_sidecar(root).unwrap();
        sink.emit(&generated).unwrap();
        sink.flush_pending().unwrap();
        fs::read_to_string(root.join("diagnostics.tsv")).unwrap()
    };

    fn unescape_tsv(value: &str) -> String {
        let mut decoded = String::new();
        let mut chars = value.chars();
        while let Some(ch) = chars.next() {
            if ch != '\\' {
                decoded.push(ch);
                continue;
            }
            match chars.next().unwrap() {
                '\\' => decoded.push('\\'),
                't' => decoded.push('\t'),
                'n' => decoded.push('\n'),
                'r' => decoded.push('\r'),
                'u' => {
                    assert_eq!(chars.next(), Some('{'));
                    let digits: String = chars.by_ref().take_while(|ch| *ch != '}').collect();
                    assert_eq!(chars.next(), Some(';'));
                    decoded
                        .push(char::from_u32(u32::from_str_radix(&digits, 16).unwrap()).unwrap());
                }
                other => panic!("unknown TSV escape {other:?}"),
            }
        }
        decoded
    }

    let first_temp = Temp::new();
    let second_temp = Temp::new();
    let first_root = first_temp.0.join("first");
    let second_root = second_temp.0.join("second");
    let sidecar = emit(&first_root);
    assert_eq!(sidecar, emit(&second_root));
    let mut rows = sidecar.lines();
    assert_eq!(
        rows.next(),
        Some("source_path\tdescriptor\torigin_present\torigin\tstatus\tline\tseverity\tmessage")
    );
    let mut removed = std::collections::BTreeMap::<usize, String>::new();
    let mut found_existing_diagnostic = false;
    for row in rows {
        let fields: Vec<_> = row.split('\t').map(unescape_tsv).collect();
        assert_eq!(fields.len(), 8);
        assert_eq!(fields[0], "p/Example.java");
        assert_eq!(fields[1], "Lp/Example;");
        assert_eq!(fields[2], "true");
        assert_eq!(fields[3], "classes.dex");
        assert_eq!(fields[4], "complete");
        if fields[5].is_empty() {
            assert_eq!(fields[6], "warning");
            assert_eq!(fields[7], "existing\tmessage\\with\ncontrol");
            found_existing_diagnostic = true;
        } else {
            let line = fields[5].parse::<usize>().unwrap();
            assert_eq!(fields[6], "info");
            let (encoded_line, text) = decode_compact_source_comment(&fields[7]).unwrap();
            assert_eq!(encoded_line, line);
            assert!(removed.insert(line, text).is_none());
        }
    }
    assert!(found_existing_diagnostic);
    assert_eq!(removed.len(), 3);
    assert_eq!(removed[&2], "\t// first\tcomment\r\n");
    assert_eq!(removed[&5], "  // last comment\n");
    assert_eq!(removed[&7], "// eof without terminator");

    let compacted = fs::read_to_string(first_root.join("p/Example.java")).unwrap();
    let mut retained = compacted.split_inclusive('\n');
    let line_count = compacted.split_inclusive('\n').count() + removed.len();
    let mut reconstructed = String::new();
    for line in 1..=line_count {
        if let Some(comment) = removed.remove(&line) {
            reconstructed.push_str(&comment);
        } else {
            reconstructed.push_str(retained.next().unwrap());
        }
    }
    assert!(retained.next().is_none());
    assert_eq!(reconstructed, original);
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
