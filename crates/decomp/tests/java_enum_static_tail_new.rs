//! Original enum suffix evidence; limited to selected owners.
#![allow(clippy::unwrap_used)]
use dex_format::DexFile;
use std::process::Command;
fn desc(dex: &DexFile, index: u32) -> &str {
    dex.string(dex.type_id(index).unwrap().descriptor_idx)
        .unwrap()
}
#[test]
#[ignore = "local Samsung APK; eight original enum owners, no source export"]
fn samsung_original_enum_shape_evidence() {
    let apk = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    );
    for (file, owners) in [
        (
            "classes.dex",
            vec!["Landroidx/appcompat/widget/SearchView$q;"],
        ),
        (
            "classes2.dex",
            vec![
                "Lcom/google/protobuf/NullValue;",
                "Lcom/google/protobuf/DescriptorProtos$FeatureSet$EnumType;",
                "Lcom/google/protobuf/JavaType;",
                "Lcom/samsung/android/samsungpass/base/common/google/i18n/addressinput/common/AddressDataKey;",
                "Lcom/samsung/android/samsungpass/base/common/utils/AppIdUtil;",
                "Lcom/samsung/android/samsungpass/debug/DebugSetupAiAutofillMode;",
                "Lcom/samsung/android/samsungpass/common/util/ValueType;",
            ],
        ),
    ] {
        let bytes = Command::new("unzip")
            .args(["-p", apk, file])
            .output()
            .unwrap();
        assert!(bytes.status.success());
        let dex = DexFile::from_bytes(&bytes.stdout).unwrap();
        for owner in owners {
            let class = dex
                .classes
                .iter()
                .find(|c| desc(&dex, c.class_idx) == owner)
                .unwrap();
            let data = class.class_data.as_ref().unwrap();
            let ctors: Vec<_> = data
                .direct_methods
                .iter()
                .filter(|m| {
                    dex.string(dex.method_id(m.method_idx).unwrap().name_idx)
                        .unwrap()
                        == "<init>"
                })
                .collect();
            eprintln!(
                "{file} {owner} constructors={:?} encoded={:?} methods={}",
                ctors
                    .iter()
                    .map(|m| (m.method_idx, format!("{:x}", m.access_flags)))
                    .collect::<Vec<_>>(),
                class.static_values,
                data.direct_methods.len() + data.virtual_methods.len()
            );
            for f in &data.static_fields {
                let raw = dex.field_id(f.field_idx).unwrap();
                eprintln!(
                    "field {} {} flags={:x} {}",
                    f.field_idx,
                    dex.string(raw.name_idx).unwrap(),
                    f.access_flags,
                    desc(&dex, raw.type_idx.into())
                );
            }
            let clinit = data
                .direct_methods
                .iter()
                .find(|m| {
                    dex.string(dex.method_id(m.method_idx).unwrap().name_idx)
                        .unwrap()
                        == "<clinit>"
                })
                .unwrap();
            let decoded = dex_decompiler::decode_method(clinit.code.as_ref().unwrap()).unwrap();
            for insn in decoded.iter().rev().take(20).rev() {
                let target = match insn.opcode {
                    dex_decompiler::Opcode::Supported(info) if info.name.starts_with("invoke-") => {
                        insn.pool_index.map(|index| {
                            let raw = dex.method_id(index).unwrap();
                            format!(
                                "{}->{}",
                                desc(&dex, raw.class_idx.into()),
                                dex.string(raw.name_idx).unwrap()
                            )
                        })
                    }
                    dex_decompiler::Opcode::Supported(info)
                        if info.name.starts_with("sget") || info.name.starts_with("sput") =>
                    {
                        insn.pool_index.map(|index| {
                            let raw = dex.field_id(index).unwrap();
                            format!(
                                "{}->{}",
                                desc(&dex, raw.class_idx.into()),
                                dex.string(raw.name_idx).unwrap()
                            )
                        })
                    }
                    _ => None,
                };
                eprintln!("{insn:?} {target:?}");
            }
        }
    }
}

fn checked(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
#[ignore = "local JDK17 and D8; one original synthetic private enum and whole-input writer negatives"]
fn private_enum_original_suffix_production_ordering() {
    use decomp::{
        dex_java::{JavaOptions, emit_java},
        output::GeneratedSource,
    };
    use std::{io::Read, path::Path};
    let home =
        std::env::var("JAVA_HOME").unwrap_or_else(|_| "/home/jtjisgod/tools/jdk-17.0.13+11".into());
    let d8 = std::env::var("DECOMPHIFY_TEST_D8")
        .unwrap_or_else(|_| "/home/jtjisgod/Android/Sdk/build-tools/35.0.0/d8".into());
    let dir = std::env::temp_dir().join(format!("decomphify-enum-suffix-{}", std::process::id()));
    let original = dir.join("original");
    let dexdir = dir.join("dex");
    let emitted = dir.join("emitted");
    for path in [&original, &dexdir, &emitted] {
        std::fs::create_dir_all(path).unwrap();
    }
    std::fs::write(
        original.join("Tracker.java"),
        r#"public class Tracker {
        public static int steps; public static String scratch;
        public static int next() { return ++steps; }
        public static String text() { steps=steps*10; return "tail"; }
        public static String use(Object[] input) { steps=steps*100; return "alias"; }
        public static void unusedStore() { scratch=null; }
    }"#,
    )
    .unwrap();
    std::fs::write(
        original.join("Fixture.java"),
        r#"public enum Fixture {
        A(7), B(9); private final int n;
        public static final String state;
        private Fixture(int unused) { n=Tracker.next(); }
        static { state=Tracker.text(); }
        public int number() { return n; }
        public native void nativeProbe();
    }"#,
    )
    .unwrap();
    checked(
        Command::new(format!("{home}/bin/javac"))
            .args(["--release", "17", "Tracker.java", "Fixture.java"])
            .current_dir(&original),
    );
    checked(
        Command::new(d8)
            .env("JAVA_HOME", &home)
            .args(["--min-api", "26", "--output"])
            .arg(&dexdir)
            .arg(original.join("Tracker.class"))
            .arg(original.join("Fixture.class")),
    );
    let dex = DexFile::from_bytes(&std::fs::read(dexdir.join("classes.dex")).unwrap()).unwrap();
    let mut jmod = zip::ZipArchive::new(
        std::fs::File::open(Path::new(&home).join("jmods/java.base.jmod")).unwrap(),
    )
    .unwrap();
    let mut classpath = vec![];
    for name in ["Enum", "Object", "String"] {
        let mut bytes = vec![];
        jmod.by_name(&format!("classes/java/lang/{name}.class"))
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        let path = dir.join(format!("{name}.class"));
        std::fs::write(&path, bytes).unwrap();
        classpath.push(path);
    }
    let emit = |inputs: &[DexFile], filter: Option<&str>| {
        let mut sources = vec![];
        let report = emit_java(
            inputs,
            &JavaOptions {
                class_filter: filter.map(str::to_owned),
                classpath: classpath.clone(),
                ..Default::default()
            },
            &mut |source: &GeneratedSource| {
                sources.push(source.clone());
                Ok(())
            },
        )
        .unwrap();
        (report, sources)
    };
    let (report, sources) = emit(std::slice::from_ref(&dex), None);
    let source = sources
        .iter()
        .find(|s| s.text.contains("enum Fixture {"))
        .unwrap_or_else(|| panic!("{report:?}: {sources:?}"));
    assert_eq!(report.complete_constructors, 2);
    assert_eq!(report.complete_static_initializers, 1);
    assert_eq!(report.complete_methods, 8);
    assert_eq!(report.declaration_only_methods, 1);
    assert_eq!(report.unsupported_methods, 0);
    assert!(
        !source.text.contains("// Global method source closure:"),
        "{}",
        source.text
    );
    assert!(
        !source.text.contains("// Global field source closure:"),
        "{}",
        source.text
    );
    assert!(
        source
            .text
            .contains("public static final java.lang.String state;")
    );
    assert!(source.text.contains("Tracker.text()"));
    assert!(
        source
            .text
            .contains("native/reflection capability observations retained")
    );
    let class = dex
        .classes
        .iter()
        .find(|c| desc(&dex, c.class_idx) == "LFixture;")
        .unwrap();
    let data = class.class_data.as_ref().unwrap();
    assert_eq!(
        source
            .text
            .lines()
            .filter(|l| l.starts_with("// decomp-enum-method "))
            .count(),
        data.direct_methods.len() + data.virtual_methods.len()
    );
    std::fs::write(emitted.join("Fixture.java"), &source.text).unwrap();
    let tracker_source = sources
        .iter()
        .find(|s| s.text.contains("class Tracker {"))
        .unwrap();
    std::fs::write(emitted.join("Tracker.java"), &tracker_source.text).unwrap();
    std::fs::write(emitted.join("Check.java"),r#"public class Check { public static void main(String[] args) throws Exception {
        if(!Fixture.state.equals("tail") || Tracker.steps!=20 || Fixture.A.number()!=1 || Fixture.B.number()!=2 || Fixture.B.ordinal()!=1 || Fixture.valueOf("A")!=Fixture.A) throw new AssertionError();
        if(!java.lang.reflect.Modifier.isFinal(Fixture.class.getDeclaredField("state").getModifiers())) throw new AssertionError();
    }}"#).unwrap();
    checked(
        Command::new(format!("{home}/bin/javac"))
            .args([
                "--release",
                "17",
                "-cp",
                ".",
                "Fixture.java",
                "Tracker.java",
                "Check.java",
            ])
            .current_dir(&emitted),
    );
    checked(
        Command::new(format!("{home}/bin/java"))
            .args(["-cp", ".", "Check"])
            .current_dir(&emitted),
    );
    let rejected = |inputs: &[DexFile]| {
        let (report, sources) = emit(inputs, None);
        assert!(
            sources.iter().all(|s| !s.text.contains("enum Fixture")),
            "{report:?}"
        );
        assert_eq!(report.complete_constructors, 1); // Only original Tracker.<init>.
    };
    let fixture_index = dex
        .classes
        .iter()
        .position(|c| desc(&dex, c.class_idx) == "LFixture;")
        .unwrap();
    let state = data
        .static_fields
        .iter()
        .find(|f| {
            dex.string(dex.field_id(f.field_idx).unwrap().name_idx)
                .unwrap()
                == "state"
        })
        .unwrap()
        .field_idx;
    let mut public = dex.clone();
    public.classes[fixture_index]
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
    rejected(&[public]);
    let clinit_index = data
        .direct_methods
        .iter()
        .position(|m| {
            dex.string(dex.method_id(m.method_idx).unwrap().name_idx)
                .unwrap()
                == "<clinit>"
        })
        .unwrap();
    let code = data.direct_methods[clinit_index].code.as_ref().unwrap();
    let instructions = dex_decompiler::decode_method(code).unwrap();
    let backing = data
        .static_fields
        .iter()
        .find(|field| {
            dex.string(dex.field_id(field.field_idx).unwrap().name_idx)
                .unwrap()
                == "$VALUES"
        })
        .unwrap()
        .field_idx;
    let backing_store = instructions.iter().find(|instruction| {
        instruction.pool_index == Some(backing)
            && matches!(instruction.opcode, dex_decompiler::Opcode::Supported(info) if info.name.starts_with("sput"))
    }).unwrap();
    let array_register = backing_store.registers[0];
    assert!(array_register < 16);
    let text_call = instructions.iter().find(|instruction| {
        matches!(instruction.opcode, dex_decompiler::Opcode::Supported(info) if info.name == "invoke-static")
            && instruction.pool_index.is_some_and(|index| dex.string(dex.method_id(index).unwrap().name_idx).unwrap() == "text")
    }).unwrap();
    let use_method = dex
        .method_ids
        .iter()
        .enumerate()
        .find(|(_, method)| {
            desc(&dex, method.class_idx as u32) == "LTracker;"
                && dex.string(method.name_idx).unwrap() == "use"
        })
        .unwrap()
        .0;
    let mut backing_alias = dex.clone();
    let bytes = &mut backing_alias.classes[fixture_index]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods[clinit_index]
        .code
        .as_mut()
        .unwrap()
        .insns;
    let offset = text_call.offset as usize * 2;
    bytes[offset..offset + 2].copy_from_slice(&0x1071_u16.to_le_bytes());
    bytes[offset + 2..offset + 4].copy_from_slice(&(use_method as u16).to_le_bytes());
    bytes[offset + 4..offset + 6].copy_from_slice(&(array_register as u16).to_le_bytes());
    // This is a genuine dependency and original backing-array alias, not a clone.
    let changed = dex_decompiler::decode_method(
        backing_alias.classes[fixture_index]
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods[clinit_index]
            .code
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let changed_call = changed
        .iter()
        .find(|instruction| instruction.offset == text_call.offset)
        .unwrap();
    assert_eq!(changed_call.pool_index, Some(use_method as u32));
    assert_eq!(changed_call.registers, vec![array_register]);
    rejected(&[backing_alias]);
    let store=dex_decompiler::decode_method(code).unwrap().into_iter().find(|i|i.pool_index==Some(state)&&matches!(i.opcode,dex_decompiler::Opcode::Supported(info)if info.name.starts_with("sput"))).unwrap();
    let mut no_store = dex.clone();
    let bytes = &mut no_store.classes[fixture_index]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods[clinit_index]
        .code
        .as_mut()
        .unwrap()
        .insns;
    let offset = store.offset as usize * 2;
    bytes[offset..offset + 4].fill(0);
    rejected(&[no_store]);
    let tracker_index = dex
        .classes
        .iter()
        .position(|c| desc(&dex, c.class_idx) == "LTracker;")
        .unwrap();
    let mut writer = dex.clone();
    writer.classes = vec![writer.classes[tracker_index].clone()];
    let method = writer.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .iter_mut()
        .find(|m| {
            dex.string(dex.method_id(m.method_idx).unwrap().name_idx)
                .unwrap()
                == "unusedStore"
        })
        .unwrap();
    let code = method.code.as_mut().unwrap();
    let store=dex_decompiler::decode_method(code).unwrap().into_iter().find(|i|matches!(i.opcode,dex_decompiler::Opcode::Supported(info)if info.name.starts_with("sput"))).unwrap();
    let offset = store.offset as usize * 2;
    code.insns[offset + 2..offset + 4].copy_from_slice(&(state as u16).to_le_bytes());
    let mut primary = dex.clone();
    primary.classes = vec![primary.classes[fixture_index].clone()];
    rejected(&[primary, writer]);
    // An input definition excluded from publication cannot be satisfied by
    // metadata alone, even though the enum suffix itself has been proven.
    let (filtered_report, filtered_sources) = emit(std::slice::from_ref(&dex), Some("LFixture;"));
    let filtered_fixture = filtered_sources
        .iter()
        .find(|source| source.text.contains("enum Fixture"))
        .unwrap();
    let closures: Vec<_> = filtered_fixture
        .text
        .lines()
        .filter(|line| line.starts_with("// Global method source closure:"))
        .collect();
    assert!(
        closures
            .iter()
            .any(|line| line.contains("name: \"<clinit>\"")
                && line.contains("owner: \"LTracker;\"")
                && line.contains("name: \"text\"")),
        "{filtered_report:?}: {}",
        filtered_fixture.text
    );
    assert!(
        closures.iter().any(|line| line.contains("name: \"<init>\"")
            && line.contains("owner: \"LTracker;\"")
            && line.contains("name: \"next\"")),
        "{filtered_report:?}: {}",
        filtered_fixture.text
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
