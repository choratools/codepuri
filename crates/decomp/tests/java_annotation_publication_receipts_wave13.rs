//! Actual outer-sink receipts for unchanged Samsung annotation sources.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use decomp::{
    dex_java::{JavaOptions, JavaReport, emit_java},
    output::{GeneratedSource, OutputError},
};
use dex_format::DexFile;
use std::{path::PathBuf, process::Command};

fn emit(
    files: &[DexFile],
    filter: &str,
    workers: usize,
    reject: Option<&str>,
) -> Result<(JavaReport, Vec<GeneratedSource>), OutputError> {
    let jdk = PathBuf::from(std::env::var_os("JAVA_HOME").expect("explicit local JDK"));
    let options = JavaOptions {
        class_filter: Some(filter.into()),
        workers,
        classpath: vec![jdk.join("jmods/java.base.jmod")],
        ..JavaOptions::default()
    };
    let mut sources = Vec::new();
    let mut sink = |source: &GeneratedSource| {
        if reject == Some(source.provenance.descriptor.as_str()) {
            return Err(OutputError::Io(std::io::Error::other(
                "intentional actual sink denial",
            )));
        }
        sources.push(source.clone());
        Ok(())
    };
    emit_java(files, &options, &mut sink).map(|r| (r, sources))
}

fn failed_delivery(files: &[DexFile], rejected: &str) {
    let jdk = PathBuf::from(std::env::var_os("JAVA_HOME").unwrap());
    let options = JavaOptions {
        class_filter: Some("Lorg/simpleframework/xml/Default".into()),
        workers: 4,
        classpath: vec![jdk.join("jmods/java.base.jmod")],
        ..JavaOptions::default()
    };
    let mut callbacks = Vec::new();
    let mut acknowledged = Vec::new();
    let result = emit_java(files, &options, &mut |source: &GeneratedSource| {
        callbacks.push(source.provenance.descriptor.clone());
        if source.provenance.descriptor == rejected {
            return Err(OutputError::Io(std::io::Error::other("actual sink denial")));
        }
        acknowledged.push(source.provenance.descriptor.clone());
        Ok(())
    });
    assert!(
        result.is_err(),
        "failed actual delivery must not return successful metrics"
    );
    assert!(!acknowledged.iter().any(|d| d == rejected));
    if rejected == "Lorg/simpleframework/xml/DefaultType;" {
        assert!(
            !callbacks
                .iter()
                .any(|d| d == "Lorg/simpleframework/xml/Default;"),
            "dependent annotation must never reach outer sink after failed enum delivery"
        );
    } else {
        assert_eq!(acknowledged, vec!["Lorg/simpleframework/xml/DefaultType;"]);
    }
}

/// Deliberately synthetic normal-caller fixture with real invoke-interface bytes.
/// The referenced annotation/enum remain unchanged original Samsung definitions.
fn add_caller_fixture(dex: &mut DexFile) {
    use dex_format::{
        RawSpan,
        class::{ClassData, EncodedMethod},
        code::CodeItem,
        ids::{MethodId, ProtoId, TypeId, TypeList},
        strings::StringData,
    };
    const SPAN: RawSpan = RawSpan { offset: 0, size: 0 };
    fn string(dex: &mut DexFile, text: &str) -> u32 {
        let index = dex.strings.len() as u32;
        dex.strings.push(StringData {
            value: text.into(),
            utf16_size: text.encode_utf16().count() as u32,
            span: SPAN,
        });
        index
    }
    let annotation = dex
        .classes
        .iter()
        .find(|c| {
            dex.string(dex.type_id(c.class_idx).unwrap().descriptor_idx)
                .unwrap()
                == "Lorg/simpleframework/xml/Default;"
        })
        .unwrap()
        .clone();
    let target = 32593u32;
    let target_id = dex.method_id(target).unwrap();
    assert_eq!(dex.string(target_id.name_idx).unwrap(), "value");
    assert_eq!(u32::from(target_id.class_idx), annotation.class_idx);
    let return_type_idx = dex
        .proto_id(u32::from(target_id.proto_idx))
        .unwrap()
        .return_type_idx;
    let descriptor_idx = string(dex, "Lorg/simpleframework/xml/DefaultReceiptCaller;");
    let class_idx = dex.type_ids.len() as u32;
    dex.type_ids.push(TypeId {
        descriptor_idx,
        span: SPAN,
    });
    let name_idx = string(dex, "call");
    let shorty_idx = string(dex, "LL");
    let proto_idx = dex.proto_ids.len() as u16;
    dex.proto_ids.push(ProtoId {
        shorty_idx,
        return_type_idx,
        parameters_off: 0,
        parameters: Some(TypeList {
            types: vec![annotation.class_idx as u16],
            span: SPAN,
        }),
        span: SPAN,
    });
    let method_idx = dex.method_ids.len() as u32;
    dex.method_ids.push(MethodId {
        class_idx: class_idx as u16,
        proto_idx,
        name_idx,
        span: SPAN,
    });
    let mut caller = annotation;
    caller.class_idx = class_idx;
    caller.access_flags = 1;
    caller.interfaces = None;
    caller.interfaces_off = 0;
    caller.annotations = None;
    caller.annotations_off = 0;
    caller.static_values = None;
    caller.static_values_off = 0;
    // invoke-interface {v1}, exact Default.value; move-result-object v0; return-object v0.
    let units = [0x1072u16, target as u16, 1, 0x000c, 0x0011];
    caller.class_data = Some(ClassData {
        static_fields: vec![],
        instance_fields: vec![],
        direct_methods: vec![EncodedMethod {
            method_idx,
            access_flags: 9,
            code_off: 1,
            code: Some(CodeItem {
                registers_size: 2,
                ins_size: 1,
                outs_size: 1,
                debug_info_off: 0,
                insns_size: units.len() as u32,
                insns: units.into_iter().flat_map(u16::to_le_bytes).collect(),
                tries: vec![],
                handlers: vec![],
                span: SPAN,
            }),
        }],
        virtual_methods: vec![],
        span: SPAN,
    });
    dex.classes.push(caller);
}

#[test]
#[ignore = "read-only original Samsung APK plus explicit local JDK17"]
fn actual_outer_receipts_compile_reflect_and_reject_failed_filtered_delivery() {
    let mut files = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    // Bounded unchanged original-definition slice: retain all four genuine pools,
    // bytes and target metadata/bodies; no synthetic source or full APK export.
    for dex in &mut files {
        let selected = dex
            .classes
            .iter()
            .filter(|c| {
                let raw = dex
                    .string(dex.type_id(c.class_idx).unwrap().descriptor_idx)
                    .unwrap();
                matches!(
                    raw,
                    "Lorg/simpleframework/xml/DefaultType;"
                        | "Lorg/simpleframework/xml/Default;"
                        | "Llq/f;"
                )
            })
            .cloned()
            .collect();
        dex.classes = selected;
    }
    {
        use decomp::java_class_kinds::{
            DeclarationUnsupported,
            initializers::{
                annotation_defaults_with, annotation_publication::declared_defaults_with,
            },
        };
        use dex_format::{
            class::ClassDef,
            encoded_value::{AnnotationElement, EncodedValueKind as Value},
        };
        let dex = &files[2];
        let original = dex
            .classes
            .iter()
            .find(|c| {
                dex.string(dex.type_id(c.class_idx).unwrap().descriptor_idx)
                    .unwrap()
                    == "Llq/f;"
            })
            .unwrap();
        let name = |index| -> Result<String, DeclarationUnsupported> {
            Ok(dex
                .string(
                    dex.type_id(index)
                        .map_err(|_| DeclarationUnsupported::InvalidMetadata)?
                        .descriptor_idx,
                )
                .map_err(|_| DeclarationUnsupported::InvalidMetadata)?
                .trim_start_matches('L')
                .trim_end_matches(';')
                .replace('/', "."))
        };
        let plan = declared_defaults_with(dex, original, &name).unwrap();
        assert_eq!(plan.original_annotations, original.annotations);
        assert_eq!(
            plan.by_method.keys().copied().collect::<Vec<_>>(),
            vec![52940, 52941, 52942, 52943, 52944]
        );
        assert_eq!(
            plan.retained_orphans
                .iter()
                .map(|e| dex.string(e.name_idx).unwrap())
                .collect::<Vec<_>>(),
            vec!["i", "n", "nl", "s"]
        );
        assert!(
            annotation_defaults_with(dex, original, &name).is_err(),
            "legacy strict whole-wrapper equivalence is unchanged"
        );
        fn elements(class: &mut ClassDef) -> &mut Vec<AnnotationElement> {
            let item = class
                .annotations
                .as_mut()
                .unwrap()
                .class_annotations
                .as_mut()
                .unwrap()
                .entries
                .iter_mut()
                .find(|a| {
                    a.annotation
                        .elements
                        .first()
                        .is_some_and(|e| matches!(e.value.value, Value::Annotation(_)))
                })
                .unwrap();
            if let Value::Annotation(defaults) = &mut item.annotation.elements[0].value.value {
                &mut defaults.elements
            } else {
                unreachable!()
            }
        }
        let mut duplicate = original.clone();
        let copy = elements(&mut duplicate)[0].clone();
        elements(&mut duplicate).push(copy);
        assert!(declared_defaults_with(dex, &duplicate, &name).is_err());
        let mut mismatched = original.clone();
        elements(&mut mismatched)[0].value.value = Value::Int(7);
        assert!(declared_defaults_with(dex, &mismatched, &name).is_err());
        for replacement in [Value::Null, Value::String(u32::MAX)] {
            let mut malformed = original.clone();
            elements(&mut malformed)[2].value.value = replacement;
            assert!(declared_defaults_with(dex, &malformed, &name).is_err());
        }
        let mut malformed = original.clone();
        malformed
            .annotations
            .as_mut()
            .unwrap()
            .class_annotations
            .as_mut()
            .unwrap()
            .entries
            .iter_mut()
            .find(|a| {
                a.annotation
                    .elements
                    .first()
                    .is_some_and(|e| matches!(e.value.value, Value::Annotation(_)))
            })
            .unwrap()
            .visibility = dex_format::annotations::Visibility::Runtime;
        assert!(declared_defaults_with(dex, &malformed, &name).is_err());
    }
    let filter = "Lorg/simpleframework/xml/Default";
    let (serial_report, serial) = emit(&files, filter, 1, None).unwrap();
    eprintln!(
        "serial report={serial_report:?}; owners={:?}",
        serial
            .iter()
            .map(|s| &s.provenance.descriptor)
            .collect::<Vec<_>>()
    );
    let (parallel_report, parallel) = emit(&files, filter, 4, None).unwrap();
    assert_eq!(
        serial, parallel,
        "actual sources and receipt order must be deterministic"
    );
    assert_eq!(serial_report, parallel_report);
    let enum_pos = serial
        .iter()
        .position(|s| s.provenance.descriptor == "Lorg/simpleframework/xml/DefaultType;")
        .unwrap();
    let annotation_pos = serial
        .iter()
        .position(|s| s.provenance.descriptor == "Lorg/simpleframework/xml/Default;")
        .unwrap();
    assert!(
        enum_pos < annotation_pos,
        "actual outer enum acknowledgement precedes dependent annotation"
    );
    let enumeration = serial[enum_pos].clone();
    let annotation = serial[annotation_pos].clone();
    assert!(
        annotation
            .text
            .contains("value() default org.simpleframework.xml.DefaultType.FIELD;"),
        "{}",
        annotation.text
    );
    let (orphan_report, orphan_sources) = emit(&files, "Llq/f;", 3, None).unwrap();
    assert_eq!(orphan_sources.len(), 1);
    assert_eq!(orphan_report.declaration_only_methods, 5);
    let orphan = orphan_sources[0].clone();
    assert!(
        orphan
            .text
            .contains("orphan AnnotationDefault entries retained"),
        "{}",
        orphan.text
    );
    assert!(orphan.text.contains("original metadata:"));
    let root = PathBuf::from(format!(
        "/tmp/opencode/annotation-wave13-original-{}",
        std::process::id()
    ));
    let src = root.join("src");
    let cp = root.join("classes");
    std::fs::create_dir_all(&cp).unwrap();
    let mut paths = Vec::new();
    for source in [enumeration, annotation, orphan] {
        let path = src.join(&source.relative_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &source.text).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source.text);
        paths.push(path);
    }
    let check = src.join("Check.java");
    std::fs::write(&check, r#"public class Check { public static void main(String[] args) throws Exception { if(org.simpleframework.xml.Default.class.getMethod("value").getDefaultValue()!=org.simpleframework.xml.DefaultType.FIELD)throw new AssertionError(); if(!Boolean.TRUE.equals(org.simpleframework.xml.Default.class.getMethod("required").getDefaultValue()))throw new AssertionError(); Class<?> x=Class.forName("lq.f$type"); if(x.getDeclaredMethods().length!=5)throw new AssertionError(); for(String s:new String[]{"c","f","m"})if(!"".equals(x.getMethod(s).getDefaultValue()))throw new AssertionError(); if(((int[])x.getMethod("l").getDefaultValue()).length!=0 || !Integer.valueOf(2).equals(x.getMethod("v").getDefaultValue()))throw new AssertionError(); for(String s:new String[]{"i","n","nl","s"}) {try{x.getDeclaredMethod(s);throw new AssertionError();}catch(NoSuchMethodException ok){}} if(!org.simpleframework.xml.DefaultType.FIELD.name().equals("FIELD") || org.simpleframework.xml.DefaultType.FIELD.ordinal()!=0 || org.simpleframework.xml.DefaultType.values().length!=2)throw new AssertionError(); System.out.print("six genuine defaults and acknowledged enum"); } }"#).unwrap();
    paths.push(check);
    let jdk = PathBuf::from(std::env::var_os("JAVA_HOME").unwrap());
    let compile = Command::new(jdk.join("bin/javac"))
        .args(["--release", "17", "-d"])
        .arg(&cp)
        .args(&paths)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new(jdk.join("bin/java"))
        .arg("-cp")
        .arg(&cp)
        .arg("Check")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "six genuine defaults and acknowledged enum"
    );
    eprintln!(
        "unchanged original sources compiled/reflected at {}",
        root.display()
    );
    failed_delivery(&files, "Lorg/simpleframework/xml/DefaultType;");
    failed_delivery(&files, "Lorg/simpleframework/xml/Default;");
    let (filtered_report, filtered) =
        emit(&files, "Lorg/simpleframework/xml/Default;", 2, None).unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered_report.declaration_only_methods, 1);
    assert_eq!(filtered_report.unsupported_methods, 1);
    assert!(!filtered[0].text.contains("value() default"));
    assert!(!filtered[0].text.contains("DefaultType.FIELD"));

    // A real normal-caller bytecode fixture must see the delivered declaration
    // in the final audit, not an obsolete pre-acknowledgment missing-source edge.
    let mut caller_files = files.clone();
    add_caller_fixture(&mut caller_files[3]);
    let (caller_report, caller_units) = emit(&caller_files, filter, 4, None).unwrap();
    assert_eq!(caller_report.declaration_only_methods, 2);
    let caller_pos = caller_units
        .iter()
        .position(|s| s.provenance.descriptor == "Lorg/simpleframework/xml/DefaultReceiptCaller;")
        .unwrap();
    let default_pos = caller_units
        .iter()
        .position(|s| s.provenance.descriptor == "Lorg/simpleframework/xml/Default;")
        .unwrap();
    assert!(
        default_pos < caller_pos,
        "annotation outer acknowledgment must precede ordinary caller audit/delivery"
    );
    let caller = &caller_units[caller_pos];
    assert!(caller.text.contains(" call("), "{}", caller.text);
    assert!(
        !caller.text.contains("Global method source closure:"),
        "acknowledged value() must not retain stale missing-source evidence: {}",
        caller.text
    );
    assert!(
        !caller.text.contains("unavailable original method"),
        "{}",
        caller.text
    );
    let caller_path = src.join(&caller.relative_path);
    std::fs::write(&caller_path, &caller.text).unwrap();
    let caller_check = src.join("CallerCheck.java");
    std::fs::write(&caller_check, r#"public class CallerCheck { public static void main(String[] args) throws Exception { org.simpleframework.xml.Default p = (org.simpleframework.xml.Default) java.lang.reflect.Proxy.newProxyInstance(CallerCheck.class.getClassLoader(), new Class<?>[]{org.simpleframework.xml.Default.class}, (proxy,m,a)->m.getDefaultValue()); if(org.simpleframework.xml.DefaultReceiptCaller.call(p)!=org.simpleframework.xml.DefaultType.FIELD)throw new AssertionError(); System.out.print("acknowledged annotation caller"); } }"#).unwrap();
    let compile = Command::new(jdk.join("bin/javac"))
        .args(["--release", "17", "-cp"])
        .arg(&cp)
        .arg("-d")
        .arg(&cp)
        .args([&caller_path, &caller_check])
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new(jdk.join("bin/java"))
        .arg("-cp")
        .arg(&cp)
        .arg("CallerCheck")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "acknowledged annotation caller"
    );
}
