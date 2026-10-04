//! Java -> D8 -> production facade -> javac/runtime, limited to one private enum.
#![allow(clippy::unwrap_used)]
use decomp::{
    dex_java::{JavaOptions, emit_java},
    output::GeneratedSource,
};
use dex_format::DexFile;
use std::{io::Read, path::Path, process::Command};

fn checked(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "command {:?}: {}",
        command,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires local JDK17 and Android build-tools D8; one synthetic enum only"]
fn private_payload_family_production_preserves_unused_parameters_and_runtime() {
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let d8 = std::env::var("DECOMPHIFY_TEST_D8")
        .unwrap_or_else(|_| "/home/jtjisgod/Android/Sdk/build-tools/35.0.0/d8".into());
    assert!(Path::new(&d8).is_file(), "D8 missing: {d8}");
    let dir =
        std::env::temp_dir().join(format!("decomphify-enum-production-{}", std::process::id()));
    let original = dir.join("original");
    let dex_dir = dir.join("dex");
    let emitted = dir.join("emitted");
    for p in [&original, &dex_dir, &emitted] {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(original.join("Fixture.java"), r#"public enum Fixture {
        A(-13, "slash\\u000a\"end", false, Long.MIN_VALUE, -0.0f, -0.0d, null), B(99);
        final int n; final String text; final boolean enabled; final long wide; final float single; final double doubleValue;
        private Fixture(int n, String text, boolean enabled, long wide, float single, double doubleValue, Object unused) {
            this.n=n; this.text=text; this.enabled=enabled; this.wide=wide; this.single=single; this.doubleValue=doubleValue;
        }
        private Fixture(int n) { this(n, null, true, -23L, 1.25f, 2.5d, null); }
        public int number() { return n; }
        public native int nativeProbe();
    }"#).unwrap();
    checked(
        Command::new(format!("{home}/bin/javac"))
            .args(["--release", "17", "Fixture.java"])
            .current_dir(&original),
    );
    checked(
        Command::new(&d8)
            .env("JAVA_HOME", &home)
            .args(["--min-api", "26", "--output"])
            .arg(&dex_dir)
            .arg(original.join("Fixture.class")),
    );
    let dex = DexFile::from_bytes(&std::fs::read(dex_dir.join("classes.dex")).unwrap()).unwrap();
    let class = dex
        .classes
        .iter()
        .find(|c| {
            dex.string(dex.type_id(c.class_idx).unwrap().descriptor_idx)
                .unwrap()
                == "LFixture;"
        })
        .unwrap();
    let original_methods = class.class_data.as_ref().unwrap().direct_methods.len()
        + class.class_data.as_ref().unwrap().virtual_methods.len();
    let original_constructors: Vec<_> = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .filter(|m| {
            dex.string(dex.method_id(m.method_idx).unwrap().name_idx)
                .unwrap()
                == "<init>"
        })
        .map(|m| m.method_idx)
        .collect();
    assert_eq!(original_constructors.len(), 2);
    // Explicit bootstrap declarations, rather than ambient/platform guesses.
    let mut jmod = zip::ZipArchive::new(
        std::fs::File::open(Path::new(&home).join("jmods/java.base.jmod")).unwrap(),
    )
    .unwrap();
    let mut classpath = Vec::new();
    for name in ["Enum", "Object"] {
        let mut bytes = Vec::new();
        jmod.by_name(&format!("classes/java/lang/{name}.class"))
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        let path = dir.join(format!("{name}.class"));
        std::fs::write(&path, bytes).unwrap();
        classpath.push(path);
    }
    let mut sources = Vec::<GeneratedSource>::new();
    let report = emit_java(
        std::slice::from_ref(&dex),
        &JavaOptions {
            class_filter: Some("LFixture;".into()),
            classpath: classpath.clone(),
            ..Default::default()
        },
        &mut |s: &GeneratedSource| {
            sources.push(s.clone());
            Ok(())
        },
    )
    .unwrap();
    let source = sources
        .iter()
        .find(|s| s.text.contains("enum Fixture {"))
        .unwrap_or_else(|| {
            panic!("production enum missing: report={report:?}, sources={sources:?}")
        });
    assert!(
        source.text.contains("java.lang.Object p8"),
        "unused original payload position lost: {}",
        source.text
    );
    assert_eq!(
        source.text.matches("private Fixture(").count(),
        2,
        "constructor family lost"
    );
    assert!(
        source.text.contains("int number("),
        "ordinary original body lost"
    );
    assert!(source.text.contains("native int nativeProbe("));
    assert_eq!(report.declaration_only_methods, 1);
    assert_eq!(report.complete_constructors, 2);
    assert_eq!(report.complete_static_initializers, 1);
    assert_eq!(report.complete_methods, 4); // number() and three proven compiler methods.
    assert_eq!(
        report.complete_constructors
            + report.complete_static_initializers
            + report.complete_methods
            + report.declaration_only_methods,
        original_methods
    );
    let markers: std::collections::BTreeMap<_, _> = source
        .text
        .lines()
        .filter_map(|line| line.strip_prefix("// decomp-enum-method "))
        .map(|line| {
            let tokens: Vec<_> = line.split_ascii_whitespace().collect();
            assert_eq!(tokens.len(), 3);
            (tokens[0].parse::<u32>().unwrap(), (tokens[1], tokens[2]))
        })
        .collect();
    assert_eq!(markers.len(), original_methods);
    for method in class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .chain(&class.class_data.as_ref().unwrap().virtual_methods)
    {
        let name = dex
            .string(dex.method_id(method.method_idx).unwrap().name_idx)
            .unwrap();
        let expected = match name {
            "<init>" => ("complete_body", "constructor"),
            "<clinit>" => ("proven_enum_equivalence", "static_initializer"),
            "number" => ("complete_body", "ordinary"),
            "nativeProbe" => ("declaration_only", "ordinary"),
            "$values" | "values" | "valueOf" => ("proven_enum_equivalence", "ordinary"),
            other => panic!("unexpected original fixture method {other}"),
        };
        assert_eq!(
            markers.get(&method.method_idx),
            Some(&expected),
            "original method {} {name}",
            method.method_idx
        );
    }
    assert_eq!(report.unsupported_methods, 0);
    assert_eq!(report.unavailable_constructors, 0);
    std::fs::write(emitted.join("Fixture.java"), &source.text).unwrap();
    std::fs::write(emitted.join("EnumCheck.java"), r#"public class EnumCheck { public static void main(String[] args) {
        if(Fixture.values().length != 2 || Fixture.valueOf("B") != Fixture.B || !Fixture.A.name().equals("A") || Fixture.B.ordinal() != 1) throw new AssertionError();
        if(Fixture.A.number() != -13 || !Fixture.A.text.equals("slash\\u000a\"end") || Fixture.A.enabled || Fixture.A.wide != Long.MIN_VALUE || Float.floatToRawIntBits(Fixture.A.single) != 0x80000000 || Double.doubleToRawLongBits(Fixture.A.doubleValue) != 0x8000000000000000L) throw new AssertionError();
        if(Fixture.B.number() != 99 || Fixture.B.text != null || !Fixture.B.enabled || Fixture.B.wide != -23 || Fixture.B.single != 1.25f || Fixture.B.doubleValue != 2.5d) throw new AssertionError();
        Fixture[] copy=Fixture.values(); copy[0]=Fixture.B; if(Fixture.values()[0] != Fixture.A) throw new AssertionError();
    }}"#).unwrap();
    checked(
        Command::new(format!("{home}/bin/javac"))
            .args(["--release", "17", "Fixture.java", "EnumCheck.java"])
            .current_dir(&emitted),
    );
    checked(
        Command::new(format!("{home}/bin/java"))
            .args(["-cp", ".", "EnumCheck"])
            .current_dir(&emitted),
    );
    // A genuine DEX store before Enum.<init> cannot disappear just because
    // hidden Java enum parameters and the explicit super call are absorbed.
    let mut pre_chain = dex.clone();
    let class = pre_chain.classes.first_mut().unwrap();
    let data = class.class_data.as_mut().unwrap();
    let n = data
        .instance_fields
        .iter_mut()
        .find(|f| {
            dex.string(dex.field_id(f.field_idx).unwrap().name_idx)
                .unwrap()
                == "n"
        })
        .unwrap();
    n.access_flags &= !0x10;
    let full = data
        .direct_methods
        .iter_mut()
        .find(|m| {
            let method = dex.method_id(m.method_idx).unwrap();
            dex.proto_id(u32::from(method.proto_idx))
                .unwrap()
                .parameters
                .as_ref()
                .is_some_and(|p| p.types.len() == 9)
        })
        .unwrap();
    let code = full.code.as_mut().unwrap();
    let words: Vec<_> = code
        .insns
        .chunks_exact(2)
        .map(|w| u16::from_le_bytes([w[0], w[1]]))
        .collect();
    assert!(matches!(words[0] & 255, 0x70 | 0x76));
    assert_eq!(words[3] & 255, 0x59);
    assert_eq!(u32::from(words[4]), n.field_idx);
    code.insns = words[3..5]
        .iter()
        .chain(&words[..3])
        .chain(&words[5..])
        .flat_map(|w| w.to_le_bytes())
        .collect();
    let mut pre_chain_sources = Vec::<GeneratedSource>::new();
    let pre_chain_report = emit_java(
        &[pre_chain],
        &JavaOptions {
            classpath: classpath.clone(),
            ..Default::default()
        },
        &mut |s: &GeneratedSource| {
            pre_chain_sources.push(s.clone());
            Ok(())
        },
    )
    .unwrap();
    assert!(
        pre_chain_sources
            .iter()
            .all(|s| !s.text.contains("enum Fixture {"))
    );
    assert!(pre_chain_report.unavailable_constructors >= 2);
    // Preserve the original public-constructor residual; no access rewrite is
    // permitted merely because its supported body could be rendered privately.
    let mut public = dex.clone();
    for class in &mut public.classes {
        for method in &mut class.class_data.as_mut().unwrap().direct_methods {
            if original_constructors.contains(&method.method_idx) {
                method.access_flags = (method.access_flags & !2) | 1;
            }
        }
    }
    let mut public_sources = Vec::<GeneratedSource>::new();
    let public_report = emit_java(
        &[public],
        &JavaOptions {
            classpath,
            ..Default::default()
        },
        &mut |s: &GeneratedSource| {
            public_sources.push(s.clone());
            Ok(())
        },
    )
    .unwrap();
    assert!(
        public_sources
            .iter()
            .all(|s| !s.text.contains("enum Fixture {"))
    );
    assert!(public_report.unavailable_constructors >= 2);
    eprintln!(
        "production private enum: original_methods={original_methods}, constructors={original_constructors:?}; public access variant retained as residual"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
