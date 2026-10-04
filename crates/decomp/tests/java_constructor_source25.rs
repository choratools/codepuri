//! Genuine selected Samsung constructors through emit_java; dependency fixtures
//! are explicit API/runtime boundaries, not recovered Android/Kotlin bodies.
#![allow(clippy::unwrap_used)]
use decomp::{
    dex_java::{JavaOptions, JavaReport, emit_java},
    output::{GeneratedSource, OutputError},
};
use dex_format::DexFile;
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

const APK: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
);
const ANDROID: &str = "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar";
fn jdk25() -> PathBuf {
    std::env::var_os("DECOMPHIFY_JDK25_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into())
}
fn descriptor(dex: &DexFile, index: u32) -> &str {
    dex.string(dex.type_id(index).unwrap().descriptor_idx)
        .unwrap()
}
fn java25_classpath() -> PathBuf {
    static CLASSPATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    CLASSPATH
        .get_or_init(|| {
            let dir = std::env::temp_dir().join(format!(
                "decomphify-source25-java-base-{}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let extracted = dir.join("modules");
            successful(tool(
                &jdk25(),
                "jimage",
                &[
                    "extract".into(),
                    "--dir".into(),
                    extracted.to_string_lossy().into_owned(),
                    "--include".into(),
                    "glob:/java.base/**".into(),
                    jdk25().join("lib/modules").to_string_lossy().into_owned(),
                ],
                &dir,
            ));
            assert!(extracted.join("java.base/java/lang/Object.class").is_file());
            let archive = dir.join("java.base.jar");
            successful(tool(
                &jdk25(),
                "jar",
                &[
                    "--create".into(),
                    "--file".into(),
                    archive.to_string_lossy().into_owned(),
                    "--no-manifest".into(),
                    "-C".into(),
                    extracted.join("java.base").to_string_lossy().into_owned(),
                    ".".into(),
                ],
                &dir,
            ));
            let hash = Command::new("sha256sum").arg(&archive).output().unwrap();
            assert!(hash.status.success());
            eprintln!(
                "genuine extracted JDK25 classpath {}",
                String::from_utf8(hash.stdout).unwrap().trim()
            );
            archive
        })
        .clone()
}
fn export(inputs: &[DexFile], target: &str, release: u16) -> (JavaReport, GeneratedSource) {
    let options = JavaOptions {
        source_release: release,
        class_filter: Some(target.into()),
        workers: 1,
        classpath: vec![java25_classpath(), ANDROID.into()],
        ..JavaOptions::default()
    };
    let mut units = Vec::new();
    let start = Instant::now();
    let report = emit_java(inputs, &options, &mut |unit: &GeneratedSource| {
        units.push(unit.clone());
        Ok(())
    })
    .unwrap();
    eprintln!(
        "selected facade release={release} target={target} elapsed_s={:.3} units={} constructors={}",
        start.elapsed().as_secs_f64(),
        units.len(),
        report.complete_constructors
    );
    assert_eq!(units.len(), 1, "{report:?}");
    assert_eq!(units[0].provenance.descriptor, target);
    // Retain actual facade evidence even if a later compile/runtime assertion
    // fails, so diagnosis does not require another full metadata preparation.
    let retained = std::env::temp_dir()
        .join(format!("decomphify-source25-units-{}", std::process::id()))
        .join(format!("release{release}"));
    let path = write(&retained, &units[0].relative_path, &units[0].text);
    eprintln!("selected unchanged source {}", path.display());
    (report, units.remove(0))
}
fn write(dir: &Path, relative: &Path, source: &str) -> PathBuf {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, source).unwrap();
    path
}
fn tool(home: &Path, tool: &str, args: &[String], cwd: &Path) -> std::process::Output {
    Command::new(home.join("bin").join(tool))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap()
}
fn successful(result: std::process::Output) -> Vec<u8> {
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}
fn copy_tree(source: &Path, target: &Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let destination = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            std::fs::copy(entry.path(), destination).unwrap();
        }
    }
}

#[test]
fn source_release_validation_and_default17_parity() {
    assert_eq!(JavaOptions::default().source_release, 17);
    let mut calls = 0;
    for release in [0, 8, 21, 24, 26] {
        let options = JavaOptions {
            source_release: release,
            ..JavaOptions::default()
        };
        let error = emit_java(&[], &options, &mut |_: &GeneratedSource| {
            calls += 1;
            Ok(())
        })
        .unwrap_err();
        assert!(matches!(error, OutputError::InvalidOptions(_)));
    }
    assert_eq!(calls, 0);
    let default = emit_java(&[], &JavaOptions::default(), &mut |_: &GeneratedSource| {
        Ok(())
    })
    .unwrap();
    let explicit = emit_java(
        &[],
        &JavaOptions {
            source_release: 17,
            ..JavaOptions::default()
        },
        &mut |_: &GeneratedSource| Ok(()),
    )
    .unwrap();
    assert_eq!(default, explicit);
}

#[test]
#[ignore = "requires retained Samsung APK, Android35, explicit JDK25; selected classes only"]
fn actual_samsung_ctor6_and_ctor78_source25_compile_and_runtime() {
    let start = Instant::now();
    let inputs = dex_format::apk::parse_apk_path(APK).unwrap();
    let dex = &inputs[0];
    // Pin the complete actual straight-line bodies, not just a retrieval label.
    for (owner, index, expected) in [
        (
            "La1/b;",
            6,
            vec![
                0x001a, 0xab08, 0x2071, 0xbe19, 0x0002, 0x1070, 0xb650, 0x0001, 0x125b, 0x0023,
                0x000e,
            ],
        ),
        (
            "La4/a$a;",
            78,
            vec![
                0x015b, 0x0040, 0x025b, 0x003f, 0x1070, 0x0954, 0x0000, 0x000e,
            ],
        ),
    ] {
        let class = dex
            .classes
            .iter()
            .find(|c| descriptor(dex, c.class_idx) == owner)
            .unwrap();
        let method = class
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods
            .iter()
            .find(|m| m.method_idx == index)
            .unwrap();
        let raw = dex.method_id(index).unwrap();
        assert_eq!(dex.string(raw.name_idx).unwrap(), "<init>");
        assert_eq!(descriptor(dex, u32::from(raw.class_idx)), owner);
        let code = method.code.as_ref().unwrap();
        assert!(code.tries.is_empty());
        let words = code
            .insns
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect::<Vec<_>>();
        assert_eq!(words, expected);
    }
    // The implemented marker interface has no methods; its fixture is an exact
    // empty API boundary, not an omission of abstract implementation duties.
    let marker = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == "La1/a;")
        .unwrap();
    assert!(
        marker
            .class_data
            .as_ref()
            .is_none_or(|d| d.direct_methods.is_empty() && d.virtual_methods.is_empty())
    );
    let (view_report, view) = export(&inputs, "La1/b;", 25);
    let (capture_report, capture) = export(&inputs, "La4/a$a;", 25);
    for (index, owner, report, unit) in [
        (6, "La1/b;", &view_report, &view),
        (78, "La4/a$a;", &capture_report, &capture),
    ] {
        assert_eq!(report.complete_constructors, 1, "{}", unit.text);
        assert!(unit.text.contains("// Required Java source release: 25"));
        let identity = unit
            .text
            .lines()
            .find(|line| line.contains(&format!("original method pool index {index};")))
            .unwrap();
        assert!(
            identity.contains(owner)
                && identity.contains("name: \"<init>\"")
                && identity.contains("complete_body=true")
        );
    }
    assert!(
        view.text.find("kotlin.jvm.internal.n$type.f").unwrap()
            < view.text.find("super();").unwrap()
    );
    assert!(view.text.find("super();").unwrap() < view.text.find("this.a =").unwrap());
    assert!(capture.text.find("this.c =").unwrap() < capture.text.find("this.b =").unwrap());
    assert!(capture.text.find("this.b =").unwrap() < capture.text.find("super();").unwrap());
    let (legacy_report, legacy) = export(&inputs, "La4/a$a;", 17);
    assert_eq!(legacy_report.complete_constructors, 0);
    assert!(legacy.text.contains("unavailable original method index 78"));
    let dir =
        std::env::temp_dir().join(format!("decomphify-facade-source25-{}", std::process::id()));
    let source = dir.join("source");
    let classes = dir.join("classes");
    std::fs::create_dir_all(&classes).unwrap();
    let mut java = vec![
        write(&source, &view.relative_path, &view.text),
        write(&source, &capture.relative_path, &capture.text),
    ];
    // Ordinary dependency APIs used by the unchanged emitted unit. These are
    // fixtures; readAt/stream/real Kotlin implementation behavior is not claimed.
    for (path, body) in [
        ("a1/a$type.java", "package a1; public interface a$type {}"),
        ("a4/a$type.java", "package a4; public class a$type {}"),
        (
            "a4/a$b.java",
            "package a4; public class a$b extends java.io.InputStream {public int read(){throw new AssertionError(\"stream boundary\");}}",
        ),
        (
            "a4/a$h.java",
            "package a4; public class a$h extends a$b {public void g(long n)throws java.io.IOException{throw new AssertionError(\"stream boundary\");}}",
        ),
        (
            "runtime/Probe.java",
            "package runtime; public class Probe {public static String trace=\"\";public static Object c,b;}",
        ),
        (
            "kotlin/jvm/internal/n$type.java",
            r#"package kotlin.jvm.internal;
public class n$type {public static void f(Object value,String name){
 runtime.Probe.trace+=name;
 if(value==null){StackTraceElement caller=new Throwable().getStackTrace()[1];throw new IllegalArgumentException(caller.getClassName()+"."+caller.getMethodName()+":"+name);}
}}"#,
        ),
    ] {
        java.push(write(&source, Path::new(path), body));
    }
    let mut args = vec![
        "--release".into(),
        "25".into(),
        "-cp".into(),
        ANDROID.into(),
        "-d".into(),
        classes.to_string_lossy().into_owned(),
    ];
    args.extend(java.iter().map(|p| p.to_string_lossy().into_owned()));
    successful(tool(&jdk25(), "javac", &args, &dir));
    let mut legacy_args = args.clone();
    legacy_args[1] = "17".into();
    let denied = tool(
        Path::new("/home/jtjisgod/tools/jdk-17.0.13+11"),
        "javac",
        &legacy_args,
        &dir,
    );
    assert!(!denied.status.success());
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("call to super must be first statement")
    );
    // Compile-time API evidence above comes from genuine Android35. Runtime
    // replaces only these Android boundary implementations, because android.jar
    // constructors are SDK stubs. The generated constructor units stay unchanged.
    let boundary = dir.join("runtime-boundary");
    let view_boundary = write(
        &boundary,
        Path::new("android/view/View.java"),
        "package android.view; public class View {}",
    );
    let parent_boundary = write(
        &boundary,
        Path::new("android/media/MediaDataSource.java"),
        r#"package android.media;
public abstract class MediaDataSource implements java.io.Closeable {
 public MediaDataSource(){runtime.Probe.trace+="P";try {runtime.Probe.c=getClass().getField("c").get(this);runtime.Probe.b=getClass().getField("b").get(this);}catch(ReflectiveOperationException e){throw new AssertionError(e);}}
 public abstract int readAt(long p,byte[] b,int o,int n)throws java.io.IOException;
 public abstract long getSize()throws java.io.IOException;
 public abstract void close()throws java.io.IOException;
}"#,
    );
    let driver = write(
        &boundary,
        Path::new("Driver.java"),
        r#"public class Driver {
 public static void main(String[] args)throws Exception {
  android.view.View view=new android.view.View();runtime.Probe.trace="";
  a1.b$type a=new a1.b$type(view);if(a.a!=view)throw new AssertionError("view identity");System.out.println(runtime.Probe.trace);
  runtime.Probe.trace="";try {new a1.b$type(null);throw new AssertionError("missing check");}catch(IllegalArgumentException e){System.out.println(runtime.Probe.trace+"|"+e.getMessage());}
  a4.a$type outer=new a4.a$type();a4.a$h h=new a4.a$h();runtime.Probe.trace="";
  a4.a$a captured=new a4.a$a(outer,h);
  if(captured.c!=outer||captured.b!=h||runtime.Probe.c!=outer||runtime.Probe.b!=h||captured.getSize()!=-1L)throw new AssertionError("capture visibility");
  System.out.println(runtime.Probe.trace+":captures=true");runtime.Probe.trace="";
  a4.a$a nullable=new a4.a$a(null,null);if(nullable.c!=null||nullable.b!=null||runtime.Probe.c!=null||runtime.Probe.b!=null)throw new AssertionError("invented null guard");System.out.println(runtime.Probe.trace+":nullable=true");
 }
}"#,
    );
    successful(tool(
        &jdk25(),
        "javac",
        &[
            "--release".into(),
            "25".into(),
            "-cp".into(),
            classes.to_string_lossy().into_owned(),
            "-d".into(),
            classes.to_string_lossy().into_owned(),
            view_boundary.to_string_lossy().into_owned(),
            parent_boundary.to_string_lossy().into_owned(),
            driver.to_string_lossy().into_owned(),
        ],
        &dir,
    ));
    // Independently assemble only the selected original constructor instruction
    // sequences from the pinned DEX evidence. All other classfile methods and
    // dependencies remain equal; constructor names and check frames are preserved.
    let asm = write(
        &dir,
        Path::new("OriginalConstructors.java"),
        r#"import java.nio.file.*;import java.lang.classfile.*;import java.lang.constant.*;
public class OriginalConstructors {
 public static void main(String[] args)throws Exception {
  Path path=Path.of(args[0]);ClassFile cf=ClassFile.of();ClassModel model=cf.parse(path);
  ClassDesc owner=model.thisClass().asSymbol();int[] replaced={0};
  byte[] bytes=cf.transformClass(model,(builder,element)->{
   if(element instanceof MethodModel method && method.methodName().equalsString("<init>")) {
    replaced[0]++;
    String descriptor=method.methodType().stringValue();
    if(!descriptor.equals(args[1].equals("view")?"(Landroid/view/View;)V":"(La4/a$type;La4/a$h;)V"))throw new AssertionError(descriptor);
    builder.transformMethod(method,(mb,me)->{
     if(me instanceof CodeModel)mb.withCode(code->{
      if(args[1].equals("view")) {
       code.aload(1).ldc("view").invokestatic(ClassDesc.of("kotlin.jvm.internal.n$type"),"f",MethodTypeDesc.ofDescriptor("(Ljava/lang/Object;Ljava/lang/String;)V"));
       code.aload(0).invokespecial(ClassDesc.of("java.lang.Object"),"<init>",MethodTypeDesc.ofDescriptor("()V"));
       code.aload(0).aload(1).putfield(owner,"a",ClassDesc.of("android.view.View"));
      } else {
       code.aload(0).aload(1).putfield(owner,"c",ClassDesc.of("a4.a$type"));
       code.aload(0).aload(2).putfield(owner,"b",ClassDesc.of("a4.a$h"));
       code.aload(0).invokespecial(ClassDesc.of("android.media.MediaDataSource"),"<init>",MethodTypeDesc.ofDescriptor("()V"));
      }
      code.return_();
     });else mb.with(me);
    });
   } else builder.with(element);
  });
  if(replaced[0]!=1)throw new AssertionError("constructor identities: "+replaced[0]);
  Files.write(path,bytes);
 }
}
"#,
    );
    // JDK25 exposes the public class-file API; no internal ASM dependency or
    // module exports are needed to build the independent instruction fixture.
    successful(tool(
        &jdk25(),
        "javac",
        &[
            "--release".into(),
            "25".into(),
            asm.to_string_lossy().into_owned(),
        ],
        &dir,
    ));
    let original = dir.join("original-bytecode");
    copy_tree(&classes, &original);
    for (unit, case) in [(&view, "view"), (&capture, "captures")] {
        let invoke = vec![
            "OriginalConstructors".into(),
            original
                .join(unit.relative_path.with_extension("class"))
                .to_string_lossy()
                .into_owned(),
            case.into(),
        ];
        successful(tool(&jdk25(), "java", &invoke, &dir));
    }
    let run = |classpath: &Path| {
        successful(tool(
            &jdk25(),
            "java",
            &[
                "-Xverify:all".into(),
                "-cp".into(),
                classpath.to_string_lossy().into_owned(),
                "Driver".into(),
            ],
            &dir,
        ))
    };
    let before = run(&original);
    let after = run(&classes);
    assert_eq!(before, after);
    assert_eq!(
        String::from_utf8(after).unwrap(),
        "view\nview|a1.b$type.<init>:view\nP:captures=true\nP:nullable=true\n"
    );
    eprintln!(
        "actual Samsung source25 selected constructors=2 unchanged_units=2 runtime_boundary_fixtures=true total_s={:.3} artifacts={}",
        start.elapsed().as_secs_f64(),
        dir.display()
    );
}
