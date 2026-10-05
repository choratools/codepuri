//! Actual private interface enums and ordinary reference payloads.
#![allow(clippy::unwrap_used)]
use decomp::{
    dex_java::{JavaOptions, emit_java},
    output::GeneratedSource,
};
use dex_format::DexFile;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};
fn checked(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn environment(label: &str) -> (String, PathBuf, Vec<PathBuf>) {
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let dir = std::env::temp_dir().join(format!("decomphify-enum-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut jmod = zip::ZipArchive::new(
        std::fs::File::open(Path::new(&home).join("jmods/java.base.jmod")).unwrap(),
    )
    .unwrap();
    let mut classpath = vec![];
    for name in [
        "java/lang/Enum",
        "java/lang/Object",
        "java/lang/Class",
        "java/lang/String",
        "java/lang/Iterable",
        "java/util/EnumSet",
        "java/util/AbstractSet",
        "java/util/AbstractCollection",
        "java/util/Set",
        "java/util/Collection",
        "java/util/Iterator",
    ] {
        let mut bytes = vec![];
        jmod.by_name(&format!("classes/{name}.class"))
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        let path = dir.join(format!("{}.class", name.rsplit('/').next().unwrap()));
        std::fs::write(&path, bytes).unwrap();
        classpath.push(path);
    }
    (home, dir, classpath)
}
fn emit(
    dex: &DexFile,
    classpath: &[PathBuf],
    descriptor: &str,
) -> (decomp::dex_java::JavaReport, Vec<GeneratedSource>) {
    let mut sources = vec![];
    let report = emit_java(
        std::slice::from_ref(dex),
        &JavaOptions {
            class_filter: Some(descriptor.into()),
            classpath: classpath.to_vec(),
            ..Default::default()
        },
        &mut |s: &GeneratedSource| {
            sources.push(s.clone());
            Ok(())
        },
    )
    .unwrap();
    (report, sources)
}
fn write_sources(dir: &Path, sources: &[GeneratedSource]) -> Vec<PathBuf> {
    sources
        .iter()
        .map(|s| {
            let path = dir.join(&s.relative_path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &s.text).unwrap();
            path
        })
        .collect()
}
#[test]
#[ignore = "local Samsung APK and JDK17; two original enum owners only"]
fn samsung_private_interface_payload_original_bodies_compile_and_run() {
    let (home, dir, classpath) = environment("samsung-interface");
    let apk = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    );
    let bytes = Command::new("unzip")
        .args(["-p", apk, "classes3.dex"])
        .output()
        .unwrap();
    assert!(bytes.status.success());
    let mut dex = DexFile::from_bytes(&bytes.stdout).unwrap();
    // Retain unmodified original pool identities and code, with a bounded
    // declaration closure; this is not an APK-wide emission benchmark.
    let owners = [
        "Lcom/samsung/android/sdk/smp/task/STask$Action;",
        "Lcom/samsung/android/sdk/smp/task/STask$CommonAction;",
        "Lcom/samsung/android/sdk/smp/task/STask$MarketingAction;",
    ];
    let retained: Vec<_> = dex
        .classes
        .iter()
        .filter(|class| {
            owners.contains(
                &dex.string(dex.type_id(class.class_idx).unwrap().descriptor_idx)
                    .unwrap(),
            )
        })
        .cloned()
        .collect();
    assert_eq!(retained.len(), owners.len());
    dex.classes = retained;
    let (_, interface) = emit(
        &dex,
        &classpath,
        "Lcom/samsung/android/sdk/smp/task/STask$Action;",
    );
    let mut files = write_sources(&dir, &interface);
    for (simple, ctor) in [("CommonAction", 36622), ("MarketingAction", 36630)] {
        let owner = format!("Lcom/samsung/android/sdk/smp/task/STask${simple};");
        let (report, sources) = emit(&dex, &classpath, &owner);
        let source = sources
            .iter()
            .find(|s| {
                s.text.contains(&format!(
                    "enum STask${simple} implements com.samsung.android.sdk.smp.task.STask$Action"
                ))
            })
            .unwrap_or_else(|| panic!("{owner}: {report:?}; {sources:?}"));
        let class = dex
            .classes
            .iter()
            .find(|c| {
                dex.string(dex.type_id(c.class_idx).unwrap().descriptor_idx)
                    .unwrap()
                    == owner
            })
            .unwrap();
        let data = class.class_data.as_ref().unwrap();
        assert_eq!(report.complete_constructors, 1);
        assert_eq!(report.complete_static_initializers, 1);
        assert_eq!(report.complete_methods, 6);
        assert_eq!(report.unsupported_methods, 0);
        assert_eq!(
            source
                .text
                .lines()
                .filter(|l| l.starts_with("// decomp-enum-method "))
                .count(),
            data.direct_methods.len() + data.virtual_methods.len()
        );
        assert!(source.text.contains(&format!(
            "// decomp-enum-method {ctor} complete_body constructor"
        )));
        for method in data.direct_methods.iter().chain(&data.virtual_methods) {
            let name = dex
                .string(dex.method_id(method.method_idx).unwrap().name_idx)
                .unwrap();
            let status = match name {
                "<init>" => "complete_body constructor",
                "<clinit>" => "proven_enum_equivalence static_initializer",
                "$values" | "values" | "valueOf" => "proven_enum_equivalence ordinary",
                _ => "complete_body ordinary",
            };
            assert!(
                source
                    .text
                    .lines()
                    .any(|line| line
                        == format!("// decomp-enum-method {} {status}", method.method_idx)),
                "missing original identity {} {name}",
                method.method_idx
            );
        }
        assert!(source.text.contains("int value("));
        assert!(source.text.contains("getAction("));
        files.extend(write_sources(&dir, &sources));
    }
    std::fs::write(dir.join("Check.java"),r#"import com.samsung.android.sdk.smp.task.*;
public class Check { public static void main(String[] args) throws Exception {
    for (Class c : new Class[]{STask$CommonAction.class, STask$MarketingAction.class}) {
        java.lang.reflect.Method lookup=c.getDeclaredMethod("getAction", int.class); lookup.setAccessible(true);
        java.lang.reflect.Method values=c.getMethod("values"); Object[] all=(Object[])values.invoke(null);
        for(int i=0;i<all.length;i++) { Enum e=(Enum)all[i];
            if(e.ordinal()!=i || !(e instanceof STask$Action) || Enum.valueOf(c,e.name())!=e || lookup.invoke(null,((STask$Action)e).value())!=e) throw new AssertionError();
        }
        if(lookup.invoke(null,Integer.MIN_VALUE)!=null) throw new AssertionError();
        for(java.lang.reflect.Constructor ctor:(java.lang.reflect.Constructor[])c.getDeclaredConstructors()) if(!java.lang.reflect.Modifier.isPrivate(ctor.getModifiers())) throw new AssertionError();
    }
} }"#).unwrap();
    files.push(dir.join("Check.java"));
    checked(
        Command::new(format!("{home}/bin/javac"))
            .args(["--release", "17", "-d"])
            .arg(&dir)
            .args(&files),
    );
    checked(
        Command::new(format!("{home}/bin/java"))
            .arg("-cp")
            .arg(&dir)
            .arg("Check"),
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
#[test]
#[ignore = "local JDK17 and D8; one synthetic interface payload enum"]
fn object_string_and_null_payload_production_roundtrip() {
    let (home, dir, classpath) = environment("reference-payload");
    let original = dir.join("original");
    let dexdir = dir.join("dex");
    let emitted = dir.join("emitted");
    for p in [&original, &dexdir, &emitted] {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(
        original.join("Token.java"),
        "public interface Token { Object payload(); }",
    )
    .unwrap();
    std::fs::write(
        original.join("Fixture.java"),
        r#"public enum Fixture implements Token {
        A("first", "value"), B("second", null), C(null, "array");
        private final String label; private final Object data;
        private Fixture(String label, Object data) { this.label=label; this.data=data; }
        public Object payload() { return data; }
        public String label() { return label; }
    }"#,
    )
    .unwrap();
    checked(
        Command::new(format!("{home}/bin/javac"))
            .args(["--release", "17", "Token.java", "Fixture.java"])
            .current_dir(&original),
    );
    let d8 = std::env::var("DECOMPHIFY_TEST_D8")
        .unwrap_or_else(|_| "/home/jtjisgod/Android/Sdk/build-tools/35.0.0/d8".into());
    checked(
        Command::new(d8)
            .env("JAVA_HOME", &home)
            .args(["--min-api", "26", "--output"])
            .arg(&dexdir)
            .arg(original.join("Token.class"))
            .arg(original.join("Fixture.class")),
    );
    let dex = DexFile::from_bytes(&std::fs::read(dexdir.join("classes.dex")).unwrap()).unwrap();
    let (report, sources) = emit(&dex, &classpath, "LFixture;");
    assert_eq!(report.complete_constructors, 1);
    assert_eq!(report.complete_static_initializers, 1);
    assert_eq!(report.complete_methods, 5);
    assert_eq!(report.unsupported_methods, 0);
    let source = sources
        .iter()
        .find(|s| s.text.contains("enum Fixture implements Token"))
        .unwrap_or_else(|| panic!("{report:?}: {sources:?}"));
    assert!(source.text.contains("(java.lang.Object) (\"value\")"));
    assert!(source.text.contains("private final java.lang.Object data;"));
    let mut files = write_sources(&emitted, &sources);
    std::fs::copy(original.join("Token.java"), emitted.join("Token.java")).unwrap();
    files.push(emitted.join("Token.java"));
    std::fs::write(emitted.join("Check.java"),r#"public class Check { public static void main(String[] args) {
        if(!Fixture.A.label().equals("first") || !Fixture.B.label().equals("second") || Fixture.C.label()!=null || !Fixture.A.payload().equals("value") || Fixture.B.payload()!=null || !Fixture.C.payload().equals("array")) throw new AssertionError();
        if(Fixture.C.ordinal()!=2 || Fixture.valueOf("A")!=Fixture.A || !(Fixture.B instanceof Token)) throw new AssertionError();
    }}"#).unwrap();
    files.push(emitted.join("Check.java"));
    checked(
        Command::new(format!("{home}/bin/javac"))
            .args(["--release", "17", "-d"])
            .arg(&emitted)
            .args(&files),
    );
    checked(
        Command::new(format!("{home}/bin/java"))
            .arg("-cp")
            .arg(&emitted)
            .arg("Check"),
    );
    // The same successful input must fail closed if interface declarations,
    // visibility or reference-payload provenance no longer meet the proof.
    let rejected = |input: &DexFile| {
        let (report, sources) = emit(input, &classpath, "LFixture;");
        assert!(sources.iter().all(|s| !s.text.contains("enum Fixture")));
        assert_eq!(report.complete_constructors, 0);
        assert!(
            report
                .reasons
                .keys()
                .any(|reason| reason.contains("EnumConstructorsUnavailable")
                    || reason.contains("EnumConstructorAccessUnrepresentable")),
            "{report:?}"
        );
    };
    let index = dex
        .classes
        .iter()
        .position(|c| {
            dex.string(dex.type_id(c.class_idx).unwrap().descriptor_idx)
                .unwrap()
                == "LFixture;"
        })
        .unwrap();
    let interface_index = dex.classes[index].interfaces.as_ref().unwrap().types[0];
    let mut duplicate = dex.clone();
    duplicate.classes[index]
        .interfaces
        .as_mut()
        .unwrap()
        .types
        .push(interface_index);
    rejected(&duplicate);
    let mut missing = dex.clone();
    missing
        .classes
        .retain(|c| c.class_idx != u32::from(interface_index));
    rejected(&missing);
    let mut wrong_kind = dex.clone();
    wrong_kind
        .classes
        .iter_mut()
        .find(|c| c.class_idx == u32::from(interface_index))
        .unwrap()
        .access_flags = 1;
    rejected(&wrong_kind);
    let mut public = dex.clone();
    public.classes[index]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .iter_mut()
        .find(|m| {
            dex.string(dex.method_id(m.method_idx).unwrap().name_idx)
                .unwrap()
                == "<init>"
        })
        .unwrap()
        .access_flags = 0x10001;
    rejected(&public);
    let clinit_index = dex.classes[index]
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .position(|m| {
            dex.string(dex.method_id(m.method_idx).unwrap().name_idx)
                .unwrap()
                == "<clinit>"
        })
        .unwrap();
    let code = dex.classes[index]
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods[clinit_index]
        .code
        .as_ref()
        .unwrap();
    let string_type = dex
        .type_ids
        .iter()
        .position(|t| dex.string(t.descriptor_idx).unwrap() == "Ljava/lang/String;")
        .unwrap() as u16;
    let payload = dex_decompiler::decode_method(code)
        .unwrap()
        .into_iter()
        .find(|i| {
            matches!(i.opcode,dex_decompiler::Opcode::Supported(info) if info.opcode==0x1a)
                && i.pool_index
                    .is_some_and(|p| dex.string(p).unwrap() == "value")
        })
        .unwrap();
    // A legal Object payload can be a class literal in DEX, but the original
    // resolution order is not yet expressible by this literal-only enum plan.
    let mut class_literal = dex.clone();
    let bytes = &mut class_literal.classes[index]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods[clinit_index]
        .code
        .as_mut()
        .unwrap()
        .insns;
    let offset = payload.offset as usize * 2;
    bytes[offset..offset + 2]
        .copy_from_slice(&(0x1cu16 | (payload.registers[0] << 8)).to_le_bytes());
    bytes[offset + 2..offset + 4].copy_from_slice(&string_type.to_le_bytes());
    rejected(&class_literal);
    std::fs::remove_dir_all(&dir).unwrap();
}
