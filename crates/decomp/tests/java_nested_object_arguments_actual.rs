//! Exact original nested string allocation witness through the seeded facade.
#![allow(clippy::unwrap_used)]
fn descriptor(dex: &dex_format::DexFile, index: u32) -> &str {
    dex.string(dex.type_id(index).unwrap().descriptor_idx)
        .unwrap()
}
fn method_name(dex: &dex_format::DexFile, index: u32) -> String {
    let m = dex.method_id(index).unwrap();
    let p = dex.proto_id(u32::from(m.proto_idx)).unwrap();
    format!(
        "{}->{}(){}",
        descriptor(dex, u32::from(m.class_idx)),
        dex.string(m.name_idx).unwrap(),
        descriptor(dex, p.return_type_idx)
    )
}
#[test]
#[ignore = "requires Samsung APK and four genuine metadata paths; one bounded original class facade export"]
fn original_nested_string_initializer_facade() {
    use decomp::{
        dex_java::{JavaOptions, emit_java},
        output::GeneratedSource,
    };
    use std::{
        path::{Path, PathBuf},
        time::Instant,
    };
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let owner =
        std::env::var("DECOMPHIFY_OBJECT_NESTED_OWNER").unwrap_or_else(|_| "La2/a$d;".into());
    let indices = match owner.as_str() {
        "La2/a$d;" => vec![18],
        _ => panic!("unscoped owner"),
    };
    let dex = &inputs[0];
    for index in &indices {
        let method = dex
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|c| c.direct_methods.iter().chain(&c.virtual_methods))
            .find(|m| m.method_idx == *index)
            .unwrap();
        let raw = dex.method_id(*index).unwrap();
        assert_eq!(descriptor(dex, u32::from(raw.class_idx)), owner);
        let code = method.code.as_ref().unwrap();
        assert_eq!(method_name(dex, *index), "La2/a$d;-><clinit>()V");
        assert_eq!(
            (code.registers_size, code.ins_size, code.insns_size),
            (3, 0, 15)
        );
        assert_eq!(method.access_flags & 8, 8);
        assert_eq!(
            code.insns
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
            [
                34, 19, 290, 18, 538, 6276, 8304, 16, 33, 8304, 19, 16, 105, 41, 14
            ]
        );
        assert!(code.tries.is_empty());
        eprintln!(
            "original pool=0 index={index} identity={} registers={} incoming={} raw_words={:?}",
            method_name(dex, *index),
            code.registers_size,
            code.ins_size,
            code.insns
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>()
        );
    }
    let classpath = [
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod",
        "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/jdk.unsupported.jmod",
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.naming.jmod",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect::<Vec<_>>();
    for path in &classpath {
        assert!(path.is_file());
    }
    let options = JavaOptions {
        source_release: 25,
        workers: 1,
        class_filter: Some(owner.clone()),
        classpath,
        ..JavaOptions::default()
    };
    let start = Instant::now();
    let mut sources = vec![];
    let report = emit_java(&inputs, &options, &mut |unit: &GeneratedSource| {
        sources.push(unit.clone());
        Ok(())
    })
    .unwrap();
    assert_eq!(sources.len(), 1, "{report:?}");
    let unit = sources.remove(0);
    assert_eq!(unit.provenance.descriptor, owner);
    let root = std::env::var_os("DECOMPHIFY_OBJECT_NESTED_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!(
                "decomphify-nested-node-actual-{}",
                std::process::id()
            ))
        });
    let original = root.join("original-units").join(&unit.relative_path);
    std::fs::create_dir_all(original.parent().unwrap()).unwrap();
    std::fs::write(&original, &unit.text).unwrap();
    std::fs::write(root.join("report.txt"), format!("{report:#?}")).unwrap();
    for index in &indices {
        let identity = unit
            .text
            .lines()
            .find(|line| line.contains(&format!("original method pool index {index};")));
        eprintln!(
            "actual nested-node owner={owner} original={index} admitted={} identity={identity:?}",
            identity.is_some()
        );
    }
    let required = 18;
    let marker = format!("original method pool index {required};");
    let identity = unit
        .text
        .lines()
        .find(|line| line.contains(&marker))
        .unwrap_or_else(|| {
            panic!(
                "required original {required} rejected; retained {}",
                original.display()
            )
        });
    assert!(
        identity.contains("complete_body=true")
            && identity.contains("semantic_complete=true")
            && identity.contains("dependency_complete=true")
    );
    let tail = &unit.text[unit.text.find(&marker).unwrap()..];
    let body = tail
        .split("    // original method pool index")
        .next()
        .unwrap();
    assert!(
        body.lines().any(|line| line.contains("new ")
            && line.contains("Failure occurred while trying to finish a future.")),
        "string must remain within original nested allocation expression: {body}"
    );
    assert_eq!(std::fs::read_to_string(&original).unwrap(), unit.text);
    std::fs::write(root.join(Path::new("scope.txt")),"Original selected class emitted through seeded production facade with four genuine metadata paths. Entire source retained unchanged; this test does not claim whole-class compilation or real APK runtime.\n").unwrap();
    eprintln!(
        "actual nested-node required original={required} complete; elapsed_s={:.3} unchanged unit={}",
        start.elapsed().as_secs_f64(),
        original.display()
    );
}

#[test]
#[ignore = "requires retained original selected initializer, original peer source, and genuine JDK25"]
fn retained_original_initializer_compiles_unchanged_with_attested_peer_boundary() {
    use std::{path::PathBuf, process::Command};
    let root = std::env::var_os("DECOMPHIFY_OBJECT_NESTED_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/opencode/decomphify-nested-string-actual".into());
    let original = root.join("original-units/a2/a$d.java");
    let bytes = std::fs::read(&original).unwrap();
    let source = String::from_utf8(bytes.clone()).unwrap();
    assert!(source.contains("original method pool index 18;") && source.contains("new a2.a$d$a("));
    assert_eq!(
        source
            .matches("Failure occurred while trying to finish a future.")
            .count(),
        1
    );
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let dex = &inputs[0];
    for (index, owner, name, param, ret) in [
        (16, "La2/a$d$a;", "<init>", "Ljava/lang/String;", "V"),
        (
            47,
            "La2/a;",
            "h",
            "Ljava/lang/Object;",
            "Ljava/lang/Object;",
        ),
    ] {
        let m = dex.method_id(index).unwrap();
        let p = dex.proto_id(u32::from(m.proto_idx)).unwrap();
        assert_eq!(descriptor(dex, u32::from(m.class_idx)), owner);
        assert_eq!(dex.string(m.name_idx).unwrap(), name);
        assert_eq!(descriptor(dex, p.return_type_idx), ret);
        assert_eq!(
            p.parameters
                .iter()
                .flat_map(|p| &p.types)
                .map(|t| descriptor(dex, u32::from(*t)))
                .collect::<Vec<_>>(),
            [param]
        );
        let original = dex
            .classes
            .iter()
            .filter_map(|c| c.class_data.as_ref())
            .flat_map(|c| c.direct_methods.iter().chain(&c.virtual_methods))
            .find(|m| m.method_idx == index)
            .unwrap();
        assert_eq!(original.access_flags & 9, if index == 47 { 9 } else { 1 });
        eprintln!(
            "attested peer index={index} flags={} words={:?}",
            original.access_flags,
            original.code.as_ref().unwrap().insns
        );
    }
    let peer = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == "La2/a$d$a;")
        .unwrap();
    assert_eq!(
        descriptor(dex, peer.superclass_idx),
        "Ljava/lang/Throwable;"
    );
    assert!(
        !peer
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods
            .iter()
            .any(|m| dex
                .string(dex.method_id(m.method_idx).unwrap().name_idx)
                .unwrap()
                == "<clinit>")
    );
    let compile = root.join("unchanged-unit-compile");
    std::fs::create_dir_all(compile.join("a2")).unwrap();
    std::fs::write(compile.join("a2/a$d.java"), &bytes).unwrap();
    let peer_path = std::env::var_os("DECOMPHIFY_OBJECT_NESTED_PEER")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            "/tmp/opencode/samsung-pass-99-wave2/java25/sources/a2/a$d$a.java".into()
        });
    let peer_bytes = std::fs::read(peer_path).unwrap();
    std::fs::write(compile.join("a2/a$d$a.java"), &peer_bytes).unwrap();
    // Exact original helper descriptor, with an explicit instrumented API
    // boundary. Its class initialization/body are not original APK execution.
    std::fs::write(compile.join("a2/a$type.java"),"package a2;public class a$type{public static int calls;public static Object seen;public static Object h(Object p0){calls++;seen=p0;p0.getClass();return p0;}}").unwrap();
    std::fs::write(compile.join("Driver.java"),"public class Driver{public static void main(String[]args){a2.a$d value=a2.a$d.b;if(value==null||value.a.getClass()!=a2.a$d$a.class||!value.a.getMessage().equals(\"Failure occurred while trying to finish a future.\")||a2.a$type.calls!=1||a2.a$type.seen!=value.a||value.a.fillInStackTrace()!=value.a)throw new AssertionError();}}").unwrap();
    let home = std::env::var_os("DECOMPHIFY_JDK25_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into());
    let run = |tool: &str, args: &[&str]| {
        let out = Command::new(home.join("bin").join(tool))
            .args(args)
            .current_dir(&compile)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{tool}:{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    run(
        "javac",
        &[
            "--release",
            "25",
            "-d",
            "classes",
            "a2/a$d.java",
            "a2/a$d$a.java",
            "a2/a$type.java",
            "Driver.java",
        ],
    );
    run("java", &["-Xverify:all", "-cp", "classes", "Driver"]);
    let bytecode = run("javap", &["-classpath", "classes", "-c", "-p", "a2.a$d"]);
    let initializer = bytecode.split("static {};").nth(1).unwrap();
    let events = initializer
        .lines()
        .filter_map(|line| {
            if line.contains(" new ") && line.ends_with("// class a2/a$d") {
                Some("outer-new")
            } else if line.contains(" new ") && line.ends_with("// class a2/a$d$a") {
                Some("inner-new")
            } else if line.contains(" ldc ") && line.contains("Failure occurred") {
                Some("string")
            } else if line.contains("invokespecial") && line.contains("Method a2/a$d$a.") {
                Some("inner-init")
            } else if line.contains("invokespecial") && line.contains("Method \"<init>\"") {
                Some("outer-init")
            } else if line.contains("putstatic") {
                Some("field-store")
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        events,
        [
            "outer-new",
            "inner-new",
            "string",
            "inner-init",
            "outer-init",
            "field-store"
        ],
        "{initializer}"
    );
    assert_eq!(std::fs::read(&original).unwrap(), bytes);
    assert_eq!(std::fs::read(compile.join("a2/a$d.java")).unwrap(), bytes);
    std::fs::write(root.join("runtime-scope.txt"),"Whole selected generated a2/a$d.java compiled unchanged with unchanged retained original a2/a$d$a peer source and a DEX-attested a2/a$type.h descriptor implemented as an instrumented API boundary. JVM -Xverify:all passed; exact clinit allocation/string/initializer/store bytecode order asserted. Does not execute original APK/helper class initialization, actual heap exhaustion, or device behavior. Global filtered-output peer closure remains missing; no app compilation claim.\n").unwrap();
    std::fs::write(root.join("initializer-javap.txt"), bytecode).unwrap();
}
