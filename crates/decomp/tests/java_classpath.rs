//! Actual javac declaration fixtures and malformed-input boundaries.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use decomp::{
    java_aliases::MethodKey,
    java_external_metadata::{ClassMetadata, Classpath, ClasspathError, ExternalMetadata},
    java_hierarchy,
    type_compatibility::{Compatibility, Declaration},
};
use std::{io::Write, path::PathBuf, process::Command};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn javac() -> PathBuf {
    if let Some(home) = std::env::var_os("JAVA_HOME") {
        let p = PathBuf::from(home).join("bin/javac");
        if p.is_file() {
            return p;
        }
    }
    if Command::new("javac").arg("-version").output().is_ok() {
        return "javac".into();
    }
    let local = PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/bin/javac");
    assert!(
        local.is_file(),
        "real class-file tests require local javac or JAVA_HOME"
    );
    local
}
fn fixture() -> Fixture {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let f = Fixture(std::env::temp_dir().join(format!(
        "decomp-classpath-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )));
    std::fs::create_dir(&f.0).unwrap();
    std::fs::write(f.0.join("Sample.java"),r#"
package fixture;
interface Marker {}
class Checked extends java.io.IOException {}
public class Sample<T extends Number> extends Base {
    public T value;
    protected Sample(int n) throws Checked {}
    public Sample() {}
    public <U extends CharSequence> U exact(U value, String... rest) throws Checked { return value; }
    public <E extends Exception> T generic(T value) throws E { return value; }
    public static long wide(double d, long l) { return l; }
    public String text() { return "\u0000\ud83d\ude00"; }
}
"#).unwrap();
    std::fs::write(f.0.join("Base.java"),"package fixture; public class Base implements Marker { public Number inherited() throws java.io.IOException { return null; } }").unwrap();
    let out = Command::new(javac())
        .args(["--release", "17", "-d"])
        .arg(&f.0)
        .arg(f.0.join("Sample.java"))
        .arg(f.0.join("Base.java"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    f
}
fn key(name: &str, parameters: &[&str], ret: &str) -> MethodKey {
    MethodKey {
        owner: "Lfixture/Sample;".into(),
        name: name.into(),
        parameters: parameters.iter().map(|s| s.to_string()).collect(),
        return_type: ret.into(),
    }
}
#[test]
fn attested_member_source_names_compile_and_execute_without_splitting_literal_dollars() {
    use decomp::java_aliases::{AliasInventory, AliasMode};
    let f = fixture();
    std::fs::write(f.0.join("Outer.java"), "package fixture; public class Outer { public static class Inner { public static class Deep { public static int value() { return 37; } } } public static Class<?> local() { class Local { class Nested {} } return Local.class; } public static Runnable anonymous() { return new Runnable() { public void run() {} }; } }").unwrap();
    std::fs::write(
        f.0.join("Literal$Type.java"),
        "package fixture; public class Literal$Type {}",
    )
    .unwrap();
    assert!(
        Command::new(javac())
            .args(["--release", "17", "-d"])
            .arg(&f.0)
            .arg(f.0.join("Outer.java"))
            .arg(f.0.join("Literal$Type.java"))
            .status()
            .unwrap()
            .success()
    );
    let cp = Classpath::from_paths(
        [
            "Outer",
            "Outer$Inner",
            "Outer$Inner$Deep",
            "Literal$Type",
            "Outer$1Local",
            "Outer$1Local$Nested",
            "Outer$1",
        ]
        .map(|n| f.0.join(format!("fixture/{n}.class"))),
    )
    .unwrap();
    let names = cp.member_source_names();
    assert_eq!(
        names["Lfixture/Outer$Inner$Deep;"],
        "fixture.Outer.Inner.Deep"
    );
    assert!(!names.contains_key("Lfixture/Literal$Type;"));
    assert!(!names.contains_key("Lfixture/Outer$1Local;"));
    assert!(!names.contains_key("Lfixture/Outer$1Local$Nested;"));
    assert!(!names.contains_key("Lfixture/Outer$1;"));
    let mut inventory = AliasInventory::default();
    for d in [
        "Lfixture/Outer$Inner$Deep;",
        "Lfixture/Literal$Type;",
        "Lunknown/Outer$Inner;",
    ] {
        inventory.add_type(d).unwrap();
    }
    let mut aliases = inventory.build(AliasMode::Unicode);
    let inputs = std::collections::BTreeSet::new();
    aliases.apply_external_member_names(&names, &inputs);
    assert_eq!(
        aliases.class_name("Lunknown/Outer$Inner;"),
        Some("unknown.Outer$Inner")
    );
    assert_eq!(
        aliases.class_name("Lfixture/Literal$Type;"),
        Some("fixture.Literal$Type")
    );
    let ty = aliases.type_name("Lfixture/Outer$Inner$Deep;").unwrap();
    let array = aliases.type_name("[[Lfixture/Outer$Inner$Deep;").unwrap();
    std::fs::write(f.0.join("Consumer.java"), format!(
        "public class Consumer {{ public static {array} arrays; public static {ty} identity({ty} p) {{ return p; }} public static void main(String[] args) {{ if ({ty}.value() != 37 || identity(new {ty}()).getClass() != {ty}.class) throw new AssertionError(); }} }}"
    )).unwrap();
    let out = Command::new(javac())
        .args(["--release", "17", "-cp"])
        .arg(&f.0)
        .arg("-d")
        .arg(&f.0)
        .arg(f.0.join("Consumer.java"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let java = javac().with_file_name("java");
    assert!(
        Command::new(java)
            .arg("-cp")
            .arg(&f.0)
            .arg("Consumer")
            .status()
            .unwrap()
            .success()
    );
    let mut preserved = inventory.build(AliasMode::Unicode);
    preserved.apply_external_member_names(
        &names,
        &std::collections::BTreeSet::from(["Lfixture/Outer;".into()]),
    );
    assert_eq!(
        preserved.class_name("Lfixture/Outer$Inner$Deep;"),
        Some("fixture.Outer$Inner$Deep")
    );
    let incomplete = Classpath::from_paths([f.0.join("fixture/Outer$Inner$Deep.class")]).unwrap();
    assert!(incomplete.member_source_names().is_empty());
}
#[test]
fn explicit_path_precedence_keeps_whole_first_declaration() {
    let first = fixture();
    let later = fixture();
    std::fs::write(
        later.0.join("Sample.java"),
        "package fixture; public class Sample { public static int onlyLater() { return 1; } }",
    )
    .unwrap();
    assert!(
        Command::new(javac())
            .args(["--release", "17", "-d"])
            .arg(&later.0)
            .arg(later.0.join("Sample.java"))
            .status()
            .unwrap()
            .success()
    );
    let a = first.0.join("fixture/Sample.class");
    let b = later.0.join("fixture/Sample.class");
    let forward = Classpath::from_paths([&a, &b]).unwrap();
    assert!(forward.method(&key("wide", &["D", "J"], "J")).is_some());
    assert!(forward.method(&key("onlyLater", &[], "I")).is_none());
    let reverse = Classpath::from_paths([&b, &a]).unwrap();
    assert!(reverse.method(&key("onlyLater", &[], "I")).is_some());
    assert!(reverse.method(&key("wide", &["D", "J"], "J")).is_none());
}
#[test]
fn real_signatures_flags_exceptions_and_hierarchy() {
    let f = fixture();
    let cp = Classpath::from_paths(
        ["Sample", "Base", "Marker", "Checked"].map(|n| f.0.join(format!("fixture/{n}.class"))),
    )
    .unwrap();
    let class = cp.class("Lfixture/Sample;").unwrap();
    assert_eq!(class.major_version, 61);
    assert_eq!(
        class.signature.as_deref(),
        Some("<T:Ljava/lang/Number;>Lfixture/Base;")
    );
    assert_eq!(class.fields[0].signature.as_deref(), Some("TT;"));
    let exact = key(
        "exact",
        &["Ljava/lang/CharSequence;", "[Ljava/lang/String;"],
        "Ljava/lang/CharSequence;",
    );
    let method = cp.method(&exact).unwrap();
    assert_eq!(method.access_flags & 0x81, 0x81);
    assert_eq!(method.exceptions, Some(vec!["Lfixture/Checked;".into()]));
    assert_eq!(
        class
            .methods
            .iter()
            .find(|m| m.key == exact)
            .unwrap()
            .signature
            .as_deref(),
        Some("<U::Ljava/lang/CharSequence;>(TU;[Ljava/lang/String;)TU;")
    );
    assert_eq!(cp.constructor_access(&key("<init>", &["I"], "V")), Some(4));
    assert_eq!(
        cp.method(&key("<init>", &[], "V")).unwrap().exceptions,
        Some(vec![])
    );
    assert!(
        cp.method(&key(
            "exact",
            &["Ljava/lang/CharSequence;", "[Ljava/lang/String;"],
            "Ljava/lang/Object;"
        ))
        .is_none()
    );
    // Inherited declarations are preserved under their actual declaring owner.
    assert!(
        cp.method(&key("inherited", &[], "Ljava/lang/Number;"))
            .is_none()
    );
    let inherited = MethodKey {
        owner: "Lfixture/Base;".into(),
        ..key("inherited", &[], "Ljava/lang/Number;")
    };
    assert_eq!(
        cp.method(&inherited).unwrap().exceptions,
        Some(vec!["Ljava/io/IOException;".into()])
    );
    let h = cp.to_hierarchy().unwrap();
    let generic = class
        .methods
        .iter()
        .find(|m| m.key.name == "generic")
        .unwrap();
    assert_eq!(
        generic.signature.as_deref(),
        Some("<E:Ljava/lang/Exception;>(TT;)TT;^TE;")
    );
    assert_eq!(
        generic.metadata.exceptions,
        Some(vec!["Ljava/lang/Exception;".into()])
    );
    let adapted = java_hierarchy::build_with_metadata(&[], &cp).unwrap();
    assert_eq!(
        adapted.assignable(
            &java_hierarchy::class_type("fixture.Sample"),
            &java_hierarchy::class_type("fixture.Base")
        ),
        Compatibility::Assignable
    );
    assert_eq!(
        h.assignable(
            &java_hierarchy::class_type("fixture.Sample"),
            &java_hierarchy::class_type("fixture.Marker")
        ),
        Compatibility::Assignable
    );
    assert_eq!(
        h.assignable(
            &java_hierarchy::class_type("fixture.Checked"),
            &java_hierarchy::class_type("java.io.IOException")
        ),
        Compatibility::Assignable
    );
    assert!(matches!(
        h.assignable(
            &java_hierarchy::class_type("fixture.Checked"),
            &java_hierarchy::class_type("java.lang.RuntimeException")
        ),
        Compatibility::Indeterminate(_)
    ));
    assert_eq!(Classpath::default().classes().count(), 0);
}
#[test]
fn all_truncations_and_duplicate_definitions_are_typed() {
    let f = fixture();
    let bytes = std::fs::read(f.0.join("fixture/Sample.class")).unwrap();
    for end in 0..bytes.len() {
        assert!(
            matches!(
                ClassMetadata::parse(&bytes[..end]),
                Err(ClasspathError::Malformed { .. })
            ),
            "truncation {end}"
        );
    }
    let mut invalid = bytes.clone();
    invalid[10] = 255;
    assert!(matches!(
        ClassMetadata::parse(&invalid),
        Err(ClasspathError::Malformed { .. })
    ));
    // Modify genuine constant-pool entries rather than mocking a parser result.
    let descriptor = b"(Ljava/lang/CharSequence;[Ljava/lang/String;)Ljava/lang/CharSequence;";
    let at = bytes
        .windows(descriptor.len())
        .position(|w| w == descriptor)
        .unwrap();
    let mut invalid_descriptor = bytes.clone();
    invalid_descriptor[at + 1] = b'V';
    assert!(matches!(
        ClassMetadata::parse(&invalid_descriptor),
        Err(ClasspathError::Malformed { .. })
    ));
    let mut invalid_length = bytes.clone();
    invalid_length[at - 2..at].copy_from_slice(&u16::MAX.to_be_bytes());
    assert!(matches!(
        ClassMetadata::parse(&invalid_length),
        Err(ClasspathError::Malformed { .. })
    ));
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(matches!(
        ClassMetadata::parse(&trailing),
        Err(ClasspathError::Malformed { .. })
    ));
    let mut cp = Classpath::default();
    cp.add_class(&bytes).unwrap();
    assert!(matches!(
        cp.add_class(&bytes),
        Err(ClasspathError::DuplicateClass(_))
    ));
    assert_eq!(cp.classes().count(), 1);
}
#[test]
fn archive_jmod_and_atomic_duplicates() {
    let f = fixture();
    let bytes = std::fs::read(f.0.join("fixture/Sample.class")).unwrap();
    let make = |path: PathBuf, names: &[&str], jmod: bool| {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for name in names {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&bytes).unwrap();
        }
        let archive = zip.finish().unwrap().into_inner();
        let mut output = if jmod { b"JM\x01\x00".to_vec() } else { vec![] };
        output.extend(archive);
        std::fs::write(path, output).unwrap();
    };
    let jar = f.0.join("sample.jar");
    make(
        jar.clone(),
        &[
            "fixture/Sample.class",
            "META-INF/versions/17/fixture/Sample.class",
        ],
        false,
    );
    let cp = Classpath::from_paths([&jar]).unwrap();
    assert_eq!(cp.classes().count(), 1);
    let jmod = f.0.join("sample.jmod");
    make(jmod.clone(), &["classes/fixture/Sample.class"], true);
    assert_eq!(Classpath::from_paths([&jmod]).unwrap().classes().count(), 1);
    let dup = f.0.join("duplicate.jar");
    make(dup.clone(), &["a.class", "b.class"], false);
    let mut cp = Classpath::default();
    assert!(matches!(
        cp.add_path(&dup),
        Err(ClasspathError::DuplicateClass(_))
    ));
    assert_eq!(cp.classes().count(), 0);
    cp.add_path(&jar).unwrap();
    assert!(matches!(
        cp.add_path(&jmod),
        Err(ClasspathError::DuplicateClass(_))
    ));
    assert_eq!(cp.classes().count(), 1);
}
#[test]
fn dex_precedence_and_duplicate_input_owners() {
    let dex = dex_format::DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let class = &dex.classes[0];
    let descriptor = dex
        .string(dex.type_id(class.class_idx).unwrap().descriptor_idx)
        .unwrap();
    let name = descriptor[1..descriptor.len() - 1].replace('/', ".");
    let h = java_hierarchy::build_hierarchy(
        &[&dex],
        [Declaration {
            name: name.clone(),
            superclass: None,
            interfaces: vec![],
            is_interface: true,
        }],
    )
    .unwrap();
    let _ = h;
    assert!(java_hierarchy::build_hierarchy(&[&dex, &dex], []).is_err());
    let k = MethodKey {
        owner: descriptor.into(),
        name: "missing".into(),
        parameters: vec![],
        return_type: "V".into(),
    };
    let mut external = std::collections::BTreeMap::new();
    external.insert(
        k.clone(),
        decomp::java_external_metadata::MethodMetadata {
            access_flags: 1,
            exceptions: Some(vec![]),
        },
    );
    assert!(
        decomp::java_external_metadata::resolve_method(std::slice::from_ref(&dex), &k, &external)
            .is_err()
    );
    assert!(
        decomp::java_external_metadata::resolve_method(&[dex.clone(), dex], &k, &external).is_err()
    );
}

#[test]
fn local_jdk17_jmod_proves_checked_exception_edges() {
    let compiler = javac();
    let Some(home) = compiler.parent().and_then(|p| p.parent()) else {
        return;
    };
    let jmod = home.join("jmods/java.base.jmod");
    if !jmod.is_file() {
        return;
    }
    let cp = Classpath::from_paths([jmod]).unwrap();
    let h = cp.to_hierarchy().unwrap();
    assert_eq!(
        h.assignable(
            &java_hierarchy::class_type("java.io.FileNotFoundException"),
            &java_hierarchy::class_type("java.lang.Exception")
        ),
        Compatibility::Assignable
    );
    assert_eq!(
        h.assignable(
            &java_hierarchy::class_type("java.io.IOException"),
            &java_hierarchy::class_type("java.lang.RuntimeException")
        ),
        Compatibility::Incompatible
    );
    assert_eq!(cp.class_access("Ljava/io/IOException;").unwrap() & 1, 1);
}
