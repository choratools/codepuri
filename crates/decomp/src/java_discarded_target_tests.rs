use super::*;

#[test]
#[allow(clippy::unwrap_used)]
fn discarded_external_call_denies_nonbridge_return_only_overloads() {
    let dir = std::env::temp_dir().join(format!("discarded-target-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Fixture.java"),
        "public class Fixture { public int value(){ return 1; } public long other(){ return 2L; } }").unwrap();
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let output = std::process::Command::new(format!("{home}/bin/javac"))
        .current_dir(&dir)
        .args(["--release", "17", "Fixture.java"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = std::fs::read(dir.join("Fixture.class")).unwrap();
    let key = MethodKey {
        owner: "LFixture;".into(),
        name: "value".into(),
        parameters: vec![],
        return_type: "I".into(),
    };
    let mut original = crate::java_external_metadata::Classpath::default();
    original.add_class(&bytes).unwrap();
    assert!(discarded_external_source_target(&original, &key, "value"));
    assert!(!discarded_external_source_target(
        &original, &key, "renamed"
    ));
    // JVM identity includes return descriptors; replace one UTF8 method name
    // to create a legal class-file family Java source cannot disambiguate.
    let pattern = b"\x01\x00\x05other";
    let positions: Vec<_> = bytes
        .windows(pattern.len())
        .enumerate()
        .filter_map(|(i, value)| (value == pattern).then_some(i))
        .collect();
    assert_eq!(positions.len(), 1);
    let mut overloaded = bytes;
    overloaded[positions[0] + 3..positions[0] + 8].copy_from_slice(b"value");
    let mut ambiguous = crate::java_external_metadata::Classpath::default();
    ambiguous.add_class(&overloaded).unwrap();
    let declarations = ambiguous.declared_methods("LFixture;").unwrap();
    assert_eq!(
        declarations
            .iter()
            .filter(|m| m.key.name == "value")
            .count(),
        2
    );
    assert!(!discarded_external_source_target(&ambiguous, &key, "value"));
    let wide = MethodKey {
        return_type: "J".into(),
        ..key
    };
    assert!(!discarded_external_source_target(
        &ambiguous, &wide, "value"
    ));
    std::fs::remove_dir_all(dir).unwrap();
}
