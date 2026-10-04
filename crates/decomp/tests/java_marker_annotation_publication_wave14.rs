//! Original Samsung marker metadata and unchanged retained-method compilation.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use decomp::java_class_kinds::initializers::marker_annotation_publication as marker;
use decomp::{
    dex_java::{JavaOptions, JavaReport, emit_java},
    java_class_kinds, java_external_metadata,
    output::{GeneratedSource, OutputError},
};
use dex_format::{DexFile, class::ClassDef};
use marker::{MarkerRequirement, plan_marker_annotation};
use std::{path::PathBuf, process::Command};

fn descriptor(dex: &DexFile, index: u32) -> String {
    dex.string(dex.type_id(index).unwrap().descriptor_idx)
        .unwrap()
        .into()
}
fn name(dex: &DexFile, index: u32) -> Result<String, java_class_kinds::DeclarationUnsupported> {
    Ok(descriptor(dex, index)
        .trim_start_matches('L')
        .trim_end_matches(';')
        .replace('/', "."))
}
fn find<'a>(dex: &'a DexFile, owner: &str) -> &'a ClassDef {
    dex.classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == owner)
        .unwrap()
}
fn original_method(source: &str, name: &str) -> String {
    let start = source
        .find(&format!("    public static boolean {name}("))
        .unwrap();
    let open = start + source[start..].find('{').unwrap();
    let mut depth = 0;
    for (i, byte) in source.as_bytes().iter().enumerate().skip(open) {
        match *byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return source[start..=i].to_owned();
                }
            }
            _ => {}
        }
    }
    panic!("unterminated original declaration")
}

fn emit_original(
    files: &[DexFile],
    owner: &str,
    classpath: Vec<PathBuf>,
    deny_delivery: bool,
) -> Result<(JavaReport, Vec<GeneratedSource>), OutputError> {
    let options = JavaOptions {
        class_filter: Some(owner.into()),
        workers: 4,
        classpath,
        ..JavaOptions::default()
    };
    let mut sources = Vec::new();
    let report = emit_java(files, &options, &mut |source: &GeneratedSource| {
        assert_eq!(source.provenance.descriptor, owner);
        if deny_delivery {
            return Err(OutputError::Io(std::io::Error::other(
                "intentional actual marker sink denial",
            )));
        }
        sources.push(source.clone());
        Ok(())
    })?;
    Ok((report, sources))
}

#[test]
#[ignore = "original Samsung APK, wave13 source evidence and explicit local JDK17"]
fn original_empty_markers_restore_runtime_metadata_and_eight_missing_type_diagnostics() {
    use dex_format::{annotations::Visibility, encoded_value::EncodedValueKind as Value};
    let files = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let jdk = PathBuf::from(std::env::var_os("JAVA_HOME").expect("explicit JDK17"));
    let platform =
        java_external_metadata::Classpath::from_paths(&[jdk.join("jmods/java.base.jmod")]).unwrap();
    let input_owners = files
        .iter()
        .flat_map(|d| d.classes.iter().map(|c| descriptor(d, c.class_idx)))
        .collect();
    let dex = &files[1];
    let owners = [
        "Lcom/samsung/android/samsungpass/base/common/annotations/PrivateDisplay;",
        "Lcom/samsung/android/samsungpass/base/common/annotations/SecureScreen;",
    ];
    let root = PathBuf::from(format!(
        "/tmp/opencode/marker-wave14-original-{}",
        std::process::id()
    ));
    let src = root.join("src");
    let before = root.join("before");
    let after = root.join("after");
    let empty = root.join("empty-sourcepath");
    for p in [&src, &before, &after, &empty] {
        std::fs::create_dir_all(p).unwrap();
    }
    let mut marker_paths = Vec::new();
    for owner in owners {
        let class = find(dex, owner);
        eprintln!(
            "original marker {owner}: type={} flags={:#x} class_def={} annotations_directory={} class_data={}",
            class.class_idx,
            class.access_flags,
            class.span.offset,
            class.annotations_off,
            class.class_data_off
        );
        assert_eq!(class.access_flags, 0x2601);
        assert!(
            class
                .class_data
                .as_ref()
                .is_none_or(|d| d.direct_methods.is_empty()
                    && d.virtual_methods.is_empty()
                    && d.static_fields.is_empty()
                    && d.instance_fields.is_empty())
        );
        let plan =
            plan_marker_annotation(dex, class, &files, &platform, &|i| name(dex, i)).unwrap();
        let indexed =
            marker::plan_marker_annotation_indexed(dex, class, &input_owners, &platform, &|i| {
                name(dex, i)
            })
            .unwrap();
        assert_eq!(indexed.header, plan.header);
        assert_eq!(indexed.original_annotations, plan.original_annotations);
        assert_eq!(plan.descriptor, owner);
        assert_eq!(plan.access_flags, class.access_flags);
        assert_eq!(plan.original_annotations, class.annotations);
        assert!(plan.annotation_metadata_restored());
        assert!(plan.retained_annotations.is_empty());
        assert!(plan.header.contains(
            "@java.lang.annotation.Retention(java.lang.annotation.RetentionPolicy.RUNTIME)"
        ));
        assert!(
            plan.header
                .contains("@java.lang.annotation.Target({java.lang.annotation.ElementType.TYPE})")
        );
        let java_owner = owner
            .trim_start_matches('L')
            .trim_end_matches(';')
            .replace('/', ".");
        let package = java_owner.rsplit_once('.').unwrap().0;
        let (report, sources) =
            emit_original(&files, owner, vec![jdk.join("jmods/java.base.jmod")], false).unwrap();
        assert_eq!(report.classes_emitted, 1);
        assert_eq!(report.complete_methods, 0);
        assert_eq!(report.complete_constructors, 0);
        assert_eq!(report.complete_static_initializers, 0);
        assert_eq!(report.declaration_only_methods, 0);
        assert_eq!(report.fields_emitted, 0);
        assert_eq!(sources.len(), 1);
        let text = sources[0].text.clone();
        assert!(text.contains(&format!("package {package};")));
        assert!(text.contains(&plan.header));
        assert!(sources[0].diagnostics.iter().any(|d| d.message.contains(
            "Genuine zero-member annotation declaration; checked platform annotation metadata restored; 0 original entries retained"
        )));
        let path = src.join(format!(
            "{}.java",
            owner.trim_start_matches('L').trim_end_matches(';')
        ));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &text).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        marker_paths.push(path);
    }
    let original = std::fs::read_to_string("/tmp/opencode/samsung-pass-100-wave13/java25/sources/com/samsung/android/samsungpass/base/common/utils/SecurityUtil.java").unwrap();
    let (_, original_sources) = emit_original(
        &files,
        "Lcom/samsung/android/samsungpass/base/common/utils/SecurityUtil;",
        vec![jdk.join("jmods/java.base.jmod")],
        false,
    )
    .unwrap();
    assert_eq!(original_sources.len(), 1);
    let production = &original_sources[0].text;
    let methods = [
        original_method(production, "hasPrivateDisplayStatus"),
        original_method(production, "hasSecureScreenStatus"),
    ];
    assert_eq!(
        methods[0],
        original_method(&original, "hasPrivateDisplayStatus")
    );
    assert_eq!(
        methods[1],
        original_method(&original, "hasSecureScreenStatus")
    );
    let header = production
        .lines()
        .find(|l| l.contains("class SecurityUtil {"))
        .unwrap();
    let security = src.join("com/samsung/android/samsungpass/base/common/utils/SecurityUtil.java");
    std::fs::create_dir_all(security.parent().unwrap()).unwrap();
    let retained_source = format!(
        "package com.samsung.android.samsungpass.base.common.utils;\n{header}\n{}\n{}\n}}\n",
        methods[0], methods[1]
    );
    std::fs::write(&security, &retained_source).unwrap();
    let baseline = Command::new(jdk.join("bin/javac"))
        .args(["--release", "17", "-sourcepath"])
        .arg(&empty)
        .arg("-d")
        .arg(&before)
        .arg(&security)
        .output()
        .unwrap();
    let errors = String::from_utf8_lossy(&baseline.stderr);
    assert!(!baseline.status.success());
    assert_eq!(errors.matches("error: package com.samsung.android.samsungpass.base.common.annotations does not exist").count(), 8, "{errors}");
    std::fs::write(root.join("before.log"), &baseline.stderr).unwrap();
    let harness = src.join("MarkerCheck.java");
    std::fs::write(&harness, r#"@com.samsung.android.samsungpass.base.common.annotations.PrivateDisplay @com.samsung.android.samsungpass.base.common.annotations.SecureScreen class Annotated {} class Derived extends Annotated {} public class MarkerCheck { public static void main(String[] args) throws Exception { for(Class<?> c:new Class<?>[]{com.samsung.android.samsungpass.base.common.annotations.PrivateDisplay.class,com.samsung.android.samsungpass.base.common.annotations.SecureScreen.class}) { if(!c.isAnnotation() || !java.lang.reflect.Modifier.isPublic(c.getModifiers()) || c.getDeclaredMethods().length!=0 || c.getDeclaredFields().length!=0)throw new AssertionError(); if(c.getAnnotation(java.lang.annotation.Retention.class).value()!=java.lang.annotation.RetentionPolicy.RUNTIME)throw new AssertionError(); if(!java.util.Arrays.equals(c.getAnnotation(java.lang.annotation.Target.class).value(),new java.lang.annotation.ElementType[]{java.lang.annotation.ElementType.TYPE}))throw new AssertionError(); } if(!com.samsung.android.samsungpass.base.common.utils.SecurityUtil.hasPrivateDisplayStatus(new Annotated()) || !com.samsung.android.samsungpass.base.common.utils.SecurityUtil.hasSecureScreenStatus(new Annotated()))throw new AssertionError(); if(com.samsung.android.samsungpass.base.common.utils.SecurityUtil.hasPrivateDisplayStatus(new Object()) || com.samsung.android.samsungpass.base.common.utils.SecurityUtil.hasSecureScreenStatus(new Derived()))throw new AssertionError(); System.out.print("original marker metadata and unchanged original consumers"); } }"#).unwrap();
    let compile = Command::new(jdk.join("bin/javac"))
        .args(["--release", "17", "-sourcepath"])
        .arg(&empty)
        .arg("-d")
        .arg(&after)
        .arg(&security)
        .args(&marker_paths)
        .arg(&harness)
        .output()
        .unwrap();
    std::fs::write(root.join("after.log"), &compile.stderr).unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&security).unwrap(),
        retained_source,
        "original consumer bodies must remain unchanged"
    );
    let run = Command::new(jdk.join("bin/java"))
        .arg("-cp")
        .arg(&after)
        .arg("MarkerCheck")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "original marker metadata and unchanged original consumers"
    );
    let illegal = src.join("IllegalTarget.java");
    std::fs::write(&illegal, "class IllegalTarget { @com.samsung.android.samsungpass.base.common.annotations.PrivateDisplay void forbidden() {} }").unwrap();
    let invalid = Command::new(jdk.join("bin/javac"))
        .args(["--release", "17", "-cp"])
        .arg(&after)
        .arg(&illegal)
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("annotation type not applicable"));
    let class = find(dex, owners[0]);
    let (without_platform, without_sources) =
        emit_original(&files, owners[0], vec![], false).unwrap();
    assert_eq!(without_platform.classes_emitted, 0);
    assert!(without_sources.is_empty());
    assert!(
        emit_original(
            &files,
            owners[0],
            vec![jdk.join("jmods/java.base.jmod")],
            true
        )
        .is_err()
    );
    let missing = std::collections::BTreeMap::new();
    assert!(matches!(
        plan_marker_annotation(dex, class, &files, &missing, &|i| name(dex, i)),
        Err(MarkerRequirement::Platform(_))
    ));
    assert!(
        plan_marker_annotation(
            &files[3],
            find(&files[3], "Lorg/simpleframework/xml/Default;"),
            &files,
            &platform,
            &|i| name(&files[3], i)
        )
        .is_err()
    );
    // Make changed malformed metadata an actual selected fixture declaration.
    let mutate = |change: &dyn Fn(&mut ClassDef)| {
        let mut changed = files.clone();
        let i = changed[1]
            .classes
            .iter()
            .position(|c| c.class_idx == class.class_idx)
            .unwrap();
        change(&mut changed[1].classes[i]);
        let d = &changed[1];
        plan_marker_annotation(d, &d.classes[i], &changed, &platform, &|index| {
            name(d, index)
        })
    };
    assert!(
        mutate(&|c| {
            c.interfaces = None;
        })
        .is_err()
    );
    assert!(
        mutate(&|c| {
            c.access_flags = 1;
        })
        .is_err()
    );
    assert!(
        mutate(&|c| {
            let set = c
                .annotations
                .as_mut()
                .unwrap()
                .class_annotations
                .as_mut()
                .unwrap();
            set.entries.push(set.entries[0].clone());
        })
        .is_err()
    );
    assert!(
        mutate(&|c| {
            c.annotations
                .as_mut()
                .unwrap()
                .class_annotations
                .as_mut()
                .unwrap()
                .entries[0]
                .visibility = Visibility::System;
        })
        .is_err()
    );
    assert!(
        mutate(&|c| {
            c.annotations
                .as_mut()
                .unwrap()
                .class_annotations
                .as_mut()
                .unwrap()
                .entries[0]
                .annotation
                .elements[0]
                .value
                .value = Value::Enum(u32::MAX);
        })
        .is_err()
    );
    assert!(
        plan_marker_annotation(dex, class, &files, &platform, &|index| {
            let original = name(dex, index)?;
            if original == "java.lang.annotation.RetentionPolicy" {
                Ok("changed.Policy".into())
            } else {
                Ok(original)
            }
        })
        .is_err()
    );
    let keep_dex = &files[0];
    let keep = find(keep_dex, "Landroidx/annotation/Keep;");
    let partial =
        plan_marker_annotation(keep_dex, keep, &files, &platform, &|i| name(keep_dex, i)).unwrap();
    assert!(!partial.annotation_metadata_restored());
    assert!(!partial.retained_annotations.is_empty());
    assert_eq!(partial.original_annotations, keep.annotations);
    eprintln!(
        "exact 2 marker sources, retained original methods23716/23717, 8->0 matching diagnostics, runtime/negative evidence: {}",
        root.display()
    );
}
