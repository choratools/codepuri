//! Genuine input inheritance: symbolic child references retain protected final selection.
#![allow(clippy::unwrap_used)]
use decomp::{
    dex_java::JavaOptions,
    java_external_metadata::{Classpath, emit_with_metadata},
    output::{GeneratedSource, OutputError},
};
use std::{fs, io::Write, path::Path, process::Command};
use zip::{ZipWriter, write::SimpleFileOptions};

#[test]
fn protected_final_getter_through_multiple_input_ancestors_compiles_and_runs() {
    let home = "/home/jtjisgod/tools/jdk-17.0.13+11";
    let android = "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar";
    let root =
        std::env::temp_dir().join(format!("codepuri-inherited-final-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("Parent.java"), "package p; public class Parent { protected final int inherited(int value) { return value*3+2; } }").unwrap();
    fs::write(
        root.join("Middle.java"),
        "package q; public class Middle extends p.Parent { }",
    )
    .unwrap();
    fs::write(root.join("Child.java"), "package q; public class Child extends Middle { public int compute(int value) { return inherited(value); } }").unwrap();
    let run = |program: &Path, args: &[&str]| {
        let result = Command::new(program)
            .args(args)
            .env("JAVA_HOME", home)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    run(
        &Path::new(home).join("bin/javac"),
        &[
            "--release",
            "8",
            "-d",
            ".",
            "Parent.java",
            "Middle.java",
            "Child.java",
        ],
    );
    let mut jar = ZipWriter::new(fs::File::create(root.join("classes.jar")).unwrap());
    for name in ["p/Parent.class", "q/Middle.class", "q/Child.class"] {
        jar.start_file(name, SimpleFileOptions::default()).unwrap();
        jar.write_all(&fs::read(root.join(name)).unwrap()).unwrap();
    }
    jar.finish().unwrap();
    fs::create_dir(root.join("dex")).unwrap();
    run(
        Path::new("/home/jtjisgod/Android/Sdk/build-tools/35.0.0/d8"),
        &[
            "--min-api",
            "24",
            "--lib",
            android,
            "--output",
            "dex",
            "classes.jar",
        ],
    );
    let dex =
        dex_format::DexFile::from_bytes(&fs::read(root.join("dex/classes.dex")).unwrap()).unwrap();
    assert!(
        dex.method_ids.iter().any(|method| {
            dex.string(
                dex.type_id(u32::from(method.class_idx))
                    .unwrap()
                    .descriptor_idx,
            )
            .unwrap()
                == "Lq/Child;"
                && dex.string(method.name_idx).unwrap() == "inherited"
        }),
        "fixture must exercise the symbolic child-owner reference"
    );
    let metadata = Classpath::from_paths([android]).unwrap();
    let mut sources = Vec::<GeneratedSource>::new();
    emit_with_metadata(
        &[dex],
        &JavaOptions {
            source_prefixes: vec!["Lq/Child;".into()],
            ..JavaOptions::default()
        },
        &mut |source: &GeneratedSource| -> Result<(), OutputError> {
            sources.push(source.clone());
            Ok(())
        },
        None,
        &metadata,
    )
    .unwrap();
    let source = &sources
        .iter()
        .find(|source| source.provenance.descriptor == "Lq/Child;")
        .unwrap()
        .text;
    assert!(source.contains("public int compute("), "{source}");
    assert!(!source.contains("((p.Parent)"), "{source}");
    fs::write(root.join("Child.java"), source).unwrap();
    fs::write(root.join("Runner.java"), "public class Runner { public static void main(String[] args) { q.Child child=new q.Child(); for(int value:new int[]{0,7,-19,Integer.MAX_VALUE}) if(child.compute(value)!=value*3+2)throw new AssertionError(); } }").unwrap();
    run(
        &Path::new(home).join("bin/javac"),
        &["--release", "17", "-d", ".", "Child.java", "Runner.java"],
    );
    run(&Path::new(home).join("bin/java"), &["-cp", ".", "Runner"]);
    fs::remove_dir_all(root).unwrap();
}
