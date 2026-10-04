//! Original hidden enum parameters are distinct from qualified field names and strings.
#![allow(clippy::unwrap_used)]
use decomp::{
    dex_java::{JavaOptions, emit_java},
    output::GeneratedSource,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
};
fn checked(command: &mut Command) -> String {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn compile(home: &Path, root: &Path, files: &[PathBuf]) {
    checked(
        Command::new(home.join("bin/javac"))
            .args(["--release", "17", "-d"])
            .arg(root)
            .args(files),
    );
}
#[test]
#[ignore = "local JDK17 and D8; original JVM/source identity and effect-order differential"]
fn original_enum_field_p1_is_not_hidden_ordinal_parameter() {
    let home = std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11"));
    let root = std::env::temp_dir().join(format!(
        "decomphify-enum-hidden-wave21-{}",
        std::process::id()
    ));
    let original = root.join("original");
    let emitted = root.join("emitted");
    let dexdir = root.join("dex");
    for path in [&original, &emitted, &dexdir] {
        std::fs::create_dir_all(path).unwrap();
    }
    let fixture = r#"public enum Fixture {
        A(3), B(5); private final int p1; private final String text;
        private Fixture(int value) { this.p1=value; this.text="p1"; Tracker.observe(value); }
        public int value() { return p1; } public String label() { return text; }
    }"#;
    let tracker = r#"public class Tracker { public static int trace; public static void observe(int value) { trace=trace*10+value; } }"#;
    let check = r#"public class Check { public static void main(String[] args) throws Exception {
        Fixture[] first=Fixture.values(), second=Fixture.values();
        if(first==second || first.length!=2 || first[0]!=Fixture.A || first[1]!=Fixture.B)throw new AssertionError("clone identity/order");
        first[0]=null; if(Fixture.values()[0]!=Fixture.A)throw new AssertionError("backing alias");
        if(Fixture.A.value()!=3 || Fixture.B.value()!=5 || !Fixture.A.label().equals("p1") || !Fixture.B.label().equals("p1"))throw new AssertionError("payload");
        if(Fixture.valueOf("B")!=Fixture.B || Fixture.A.ordinal()!=0 || Fixture.B.ordinal()!=1 || Tracker.trace!=35)throw new AssertionError("order");
        boolean rejected=false; try {Fixture.valueOf("missing");}catch(IllegalArgumentException expected){rejected=true;}
        if(!rejected || !java.lang.reflect.Modifier.isFinal(Fixture.class.getDeclaredField("p1").getModifiers()))throw new AssertionError("contract");
        System.out.print("35:A=3:B=5:clone:final:p1");
    }}"#;
    for (name, text) in [("Fixture", fixture), ("Tracker", tracker), ("Check", check)] {
        std::fs::write(original.join(format!("{name}.java")), text).unwrap();
    }
    compile(
        &home,
        &original,
        &[
            original.join("Fixture.java"),
            original.join("Tracker.java"),
            original.join("Check.java"),
        ],
    );
    let expected = checked(
        Command::new(home.join("bin/java"))
            .arg("-cp")
            .arg(&original)
            .arg("Check"),
    );
    let d8 = std::env::var_os("DECOMPHIFY_TEST_D8")
        .unwrap_or_else(|| "/home/jtjisgod/Android/Sdk/build-tools/35.0.0/d8".into());
    checked(
        Command::new(d8)
            .env("JAVA_HOME", &home)
            .args(["--min-api", "26", "--output"])
            .arg(&dexdir)
            .arg(original.join("Fixture.class"))
            .arg(original.join("Tracker.class")),
    );
    let dex = dex_format::DexFile::from_bytes(&std::fs::read(dexdir.join("classes.dex")).unwrap())
        .unwrap();
    let mut sources = vec![];
    let report = emit_java(
        &[dex],
        &JavaOptions {
            classpath: vec![home.join("jmods/java.base.jmod")],
            ..Default::default()
        },
        &mut |s: &GeneratedSource| {
            sources.push(s.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(report.unsupported_methods, 0, "{report:?}");
    let fixture = sources
        .iter()
        .find(|s| s.text.contains("enum Fixture"))
        .unwrap();
    assert!(fixture.text.contains("private final int p1;"));
    assert!(fixture.text.contains("this.p1 ="));
    let mut files = vec![];
    for source in &sources {
        let path = emitted.join(&source.relative_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &source.text).unwrap();
        files.push(path);
    }
    std::fs::write(emitted.join("Check.java"), check).unwrap();
    files.push(emitted.join("Check.java"));
    compile(&home, &emitted, &files);
    let actual = checked(
        Command::new(home.join("bin/java"))
            .arg("-cp")
            .arg(&emitted)
            .arg("Check"),
    );
    assert_eq!(actual, expected);
    println!(
        "ORIGINAL_ENUM_HIDDEN_EQUIVALENCE {} {}",
        root.display(),
        actual
    );
}
