//! Effect-ordered reference payload recovery, with unchanged source runtime oracles.
#![allow(clippy::unwrap_used)]
use decomp::{
    dex_java::{JavaOptions, emit_java},
    output::GeneratedSource,
};
use dex_format::DexFile;
use std::{
    path::{Path, PathBuf},
    process::Command,
};
fn command(command: &mut Command) -> String {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn home() -> String {
    std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into())
}
fn emit(dex: &DexFile, owner: &str) -> (decomp::dex_java::JavaReport, Vec<GeneratedSource>) {
    let mut sources = vec![];
    let report = emit_java(
        std::slice::from_ref(dex),
        &JavaOptions {
            class_filter: Some(owner.into()),
            classpath: vec![PathBuf::from(home()).join("jmods/java.base.jmod")],
            ..Default::default()
        },
        &mut |source: &GeneratedSource| {
            sources.push(source.clone());
            Ok(())
        },
    )
    .unwrap();
    (report, sources)
}
fn desc(dex: &DexFile, index: u32) -> &str {
    dex.string(dex.type_id(index).unwrap().descriptor_idx)
        .unwrap()
}
fn write_source(root: &Path, source: &GeneratedSource) -> PathBuf {
    let p = root.join(&source.relative_path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, &source.text).unwrap();
    p
}
#[test]
#[ignore = "local JDK17 and D8; unchanged enum source with effect-order and throw-identity oracle"]
fn reference_payload_original_effect_order_and_single_evaluation() {
    let home = home();
    let d8 = std::env::var("DECOMPHIFY_TEST_D8")
        .unwrap_or_else(|_| "/home/jtjisgod/Android/Sdk/build-tools/35.0.0/d8".into());
    let root = std::env::temp_dir().join(format!(
        "decomphify-wave9-enum-effects-{}",
        std::process::id()
    ));
    let original = root.join("original");
    let emitted = root.join("emitted");
    let dexdir = root.join("dex");
    for p in [&original, &emitted, &dexdir] {
        std::fs::create_dir_all(p).unwrap();
    }
    let tracker = r#"public class Tracker { public static Object ref = new Object(); public static int trace; public static int mode; public static final Error failure = new Error("same"); public static Object make(int n) { trace = trace*10+n; if(mode==n)throw failure; return new Object(); } public static Object wrap(Object x) { trace=trace*10+8; return x; } public static void observe(Object x) { trace=trace*10+9; if(mode==9)throw failure; } }"#;
    let fixture = r#"public enum Fixture { A(Tracker.make(1)), B(Tracker.wrap(Tracker.make(2))), C(Tracker.ref), D(Double.valueOf(-0.0d)); public final Object payload; private Fixture(Object x) { payload=x; Tracker.observe(x); } }"#;
    let check = r#"public class Check { public static void main(String[] a) { Tracker.mode=Integer.parseInt(a[0]); try { Fixture[] values=Fixture.values(); if(values.length!=4 || Fixture.A.ordinal()!=0 || Fixture.B.ordinal()!=1 || !Fixture.C.name().equals("C") || Fixture.valueOf("B")!=Fixture.B || Fixture.C.payload!=Tracker.ref || Double.doubleToRawLongBits((Double)Fixture.D.payload)!=0x8000000000000000L)throw new AssertionError(); values[0]=null; if(Fixture.values()[0]!=Fixture.A)throw new AssertionError(); System.out.print("ok:"+Tracker.trace); } catch(Error e) { if(e!=Tracker.failure)throw e; System.out.print("same:"+Tracker.trace); } } }"#;
    for (name, text) in [
        ("Tracker.java", tracker),
        ("Fixture.java", fixture),
        ("Check.java", check),
    ] {
        std::fs::write(original.join(name), text).unwrap();
    }
    command(
        Command::new(format!("{home}/bin/javac"))
            .args([
                "--release",
                "17",
                "Tracker.java",
                "Fixture.java",
                "Check.java",
            ])
            .current_dir(&original),
    );
    command(
        Command::new(d8)
            .env("JAVA_HOME", &home)
            .args(["--min-api", "26", "--output"])
            .arg(&dexdir)
            .arg(original.join("Fixture.class"))
            .arg(original.join("Tracker.class")),
    );
    let dex = DexFile::from_bytes(&std::fs::read(dexdir.join("classes.dex")).unwrap()).unwrap();
    let (report, sources) = emit(&dex, "LFixture;");
    let source = sources
        .iter()
        .find(|s| s.text.contains("enum Fixture"))
        .unwrap_or_else(|| panic!("{report:?}"));
    assert!(source.text.contains("Tracker.make"), "{}", source.text);
    assert!(source.text.contains("Tracker.wrap"));
    assert!(source.text.contains("Tracker.ref"));
    let file = write_source(&emitted, source);
    command(
        Command::new(format!("{home}/bin/javac"))
            .args(["--release", "17", "-cp"])
            .arg(&original)
            .arg("-d")
            .arg(&emitted)
            .arg(file),
    );
    for mode in [0, 1, 2, 9] {
        let mode = mode.to_string();
        let expected = command(
            Command::new(format!("{home}/bin/java"))
                .arg("-cp")
                .arg(&original)
                .args(["Check", &mode]),
        );
        let actual = command(
            Command::new(format!("{home}/bin/java"))
                .arg("-cp")
                .arg(format!("{}:{}", emitted.display(), original.display()))
                .args(["Check", &mode]),
        );
        assert_eq!(actual, expected, "mode {mode}");
    }
    // Every exact original enum identity is retained, including values/valueOf.
    let class = dex
        .classes
        .iter()
        .find(|c| desc(&dex, c.class_idx) == "LFixture;")
        .unwrap();
    let cd = class.class_data.as_ref().unwrap();
    assert_eq!(
        source
            .text
            .lines()
            .filter(|l| l.starts_with("// decomp-enum-method "))
            .count(),
        cd.direct_methods.len() + cd.virtual_methods.len()
    );
    // The original factory still executes, but its result is no longer passed
    // to the initializer. It must not disappear from the enum declaration.
    let clinit = cd
        .direct_methods
        .iter()
        .find(|method| {
            dex.string(dex.method_id(method.method_idx).unwrap().name_idx)
                .unwrap()
                == "<clinit>"
        })
        .unwrap();
    let decoded = dex_decompiler::decode_method(clinit.code.as_ref().unwrap()).unwrap();
    let call = decoded.iter().find(|instruction| matches!(instruction.opcode, dex_decompiler::Opcode::Supported(info) if info.name == "invoke-direct") && instruction.registers.len() == 4).unwrap();
    let mut dropped = dex.clone();
    let changed = dropped
        .classes
        .iter_mut()
        .find(|candidate| candidate.class_idx == class.class_idx)
        .unwrap()
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .iter_mut()
        .find(|method| method.method_idx == clinit.method_idx)
        .unwrap()
        .code
        .as_mut()
        .unwrap();
    let at = call.offset as usize * 2 + 4;
    let original_word = u16::from_le_bytes(changed.insns[at..at + 2].try_into().unwrap());
    let word = (original_word & 0x0fff) | (call.registers[2] << 12);
    changed.insns[at..at + 2].copy_from_slice(&word.to_le_bytes());
    let (_, rejected) = emit(&dropped, "LFixture;");
    assert!(
        rejected
            .iter()
            .all(|unit| !unit.text.contains("enum Fixture"))
    );
}
#[test]
#[ignore = "local Samsung APK and JDK17; exactly one original private enum owner"]
fn samsung_wireformat_original_reference_payload_source() {
    let output = Command::new("unzip")
        .args([
            "-p",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
            ),
            "classes2.dex",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let dex = DexFile::from_bytes(&output.stdout).unwrap();
    let (report, sources) = emit(&dex, "Lcom/google/protobuf/WireFormat$JavaType;");
    let source = sources
        .iter()
        .find(|s| s.text.contains("enum WireFormat$JavaType"))
        .unwrap_or_else(|| panic!("{report:?}"));
    assert!(
        source.text.contains("java.lang.Integer.valueOf"),
        "{}",
        source.text
    );
    assert!(source.text.contains("java.lang.Long.valueOf"));
    assert!(source.text.contains("java.lang.Float.valueOf"));
    assert!(source.text.contains("java.lang.Double.valueOf"));
    assert!(source.text.contains("java.lang.Boolean.FALSE"));
    assert!(source.text.contains("com.google.protobuf.ByteString.EMPTY"));
    assert!(
        !source.text.contains("unavailable original method"),
        "{}",
        source.text
    );
    let target = PathBuf::from("/tmp/opencode/wave9-original-wireformat-java-type.java");
    std::fs::write(&target, &source.text).unwrap();
    eprintln!(
        "original WireFormat JavaType source {} bytes; report ordinary={} ctor={} clinit={}",
        source.text.len(),
        report.complete_methods,
        report.complete_constructors,
        report.complete_static_initializers
    );
    assert_eq!(
        source
            .text
            .lines()
            .filter(|l| l.starts_with("// decomp-enum-method "))
            .count(),
        5
    );
    // This genuine library is a declaration/runtime dependency, not an oracle
    // for the APK enum body. Assertions below come from its original DEX table,
    // literals, boxing invokes and field references above.
    let protobuf = PathBuf::from(
        "/home/jtjisgod/.gradle/caches/modules-2/files-2.1/com.google.protobuf/protobuf-java/3.22.3/fdee98b8f6abab73f146a4edb4c09e56f8278d03/protobuf-java-3.22.3.jar",
    );
    assert!(protobuf.is_file());
    let root = PathBuf::from(format!(
        "/tmp/opencode/wave9-wireformat-original-{}",
        std::process::id()
    ));
    let path = write_source(&root, source);
    let classes = root.join("classes");
    std::fs::create_dir_all(&classes).unwrap();
    command(
        Command::new(format!("{}/bin/javac", home()))
            .args(["--release", "17", "-cp"])
            .arg(&protobuf)
            .arg("-d")
            .arg(&classes)
            .arg(path),
    );
    let harness = root.join("Check.java");
    std::fs::write(&harness,r#"public class Check {
      public static void main(String[] ignored) throws Exception {
        Class<?> type=Class.forName("com.google.protobuf.WireFormat$JavaType");
        String[] names={"INT","LONG","FLOAT","DOUBLE","BOOLEAN","STRING","BYTE_STRING","ENUM","MESSAGE"};
        Object[] values=(Object[])type.getMethod("values").invoke(null);
        if(!type.isEnum() || values.length!=names.length)throw new AssertionError();
        for(int n=0;n<names.length;n++) { Enum<?> e=(Enum<?>)values[n]; if(e.ordinal()!=n || !e.name().equals(names[n]) || type.getField(names[n]).get(null)!=e || type.getMethod("valueOf",String.class).invoke(null,names[n])!=e)throw new AssertionError(names[n]); }
        java.lang.reflect.Method get=type.getMethod("getDefaultDefault");
        if(get.invoke(values[0])!=Integer.valueOf(0) || get.invoke(values[1])!=Long.valueOf(0) || Float.floatToRawIntBits((Float)get.invoke(values[2]))!=0 || Double.doubleToRawLongBits((Double)get.invoke(values[3]))!=0 || get.invoke(values[4])!=Boolean.FALSE || get.invoke(values[5])!="" || get.invoke(values[6])!=com.google.protobuf.ByteString.EMPTY || get.invoke(values[7])!=null || get.invoke(values[8])!=null)throw new AssertionError("payload");
        values[0]=null;if(((Object[])type.getMethod("values").invoke(null))[0]!=type.getField("INT").get(null))throw new AssertionError("clone");
        for(java.lang.reflect.Constructor<?> c:type.getDeclaredConstructors())if(!java.lang.reflect.Modifier.isPrivate(c.getModifiers()))throw new AssertionError("ctor");
        try {type.getMethod("valueOf",String.class).invoke(null,"absent");throw new AssertionError();} catch(java.lang.reflect.InvocationTargetException e){if(!(e.getCause() instanceof IllegalArgumentException))throw e;}
        System.out.print("original-name-order-payload-identity-ok");
      }
    }"#).unwrap();
    let cp = format!("{}:{}", classes.display(), protobuf.display());
    command(
        Command::new(format!("{}/bin/javac", home()))
            .args(["--release", "17", "-cp", &cp, "-d"])
            .arg(&classes)
            .arg(harness),
    );
    assert_eq!(
        command(Command::new(format!("{}/bin/java", home())).args(["-cp", &cp, "Check"])),
        "original-name-order-payload-identity-ok"
    );
    let bytecode = command(Command::new(format!("{}/bin/javap", home())).args([
        "-c",
        "-p",
        "-classpath",
        &cp,
        "com.google.protobuf.WireFormat$JavaType",
    ]));
    std::fs::write(root.join("original-emitted-bytecode.txt"), &bytecode).unwrap();
    let initializer = bytecode.split("static {};").nth(1).unwrap();
    assert!(
        initializer.find("new").unwrap() < initializer.find("java/lang/Integer.valueOf").unwrap()
    );
    assert!(
        initializer.find("java/lang/Integer.valueOf").unwrap()
            < initializer.find("invokespecial").unwrap()
    );
}
