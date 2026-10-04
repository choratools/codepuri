//! One original protected allocation/operand chain through the production facade.
#![allow(clippy::unwrap_used)]
use decomp::{
    dex_java::{JavaOptions, emit_java},
    output::GeneratedSource,
};
use std::{path::PathBuf, time::Instant};
const APK: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
);
fn descriptor(d: &dex_format::DexFile, index: u32) -> &str {
    d.string(d.type_id(index).unwrap().descriptor_idx).unwrap()
}
#[test]
#[ignore = "requires retained Samsung APK and genuine JDK25/Android35 metadata; one selected class"]
fn original_spass_34682_protected_constructor_operands() {
    let start = Instant::now();
    let inputs = dex_format::apk::parse_apk_path(APK).unwrap();
    let dex = &inputs[2];
    let raw = dex.method_id(34682).unwrap();
    let owner = "Lcom/samsung/android/sdk/pass/SpassFingerprint;";
    assert_eq!(descriptor(dex, u32::from(raw.class_idx)), owner);
    assert_eq!(dex.string(raw.name_idx).unwrap(), "e");
    let proto = dex.proto_id(u32::from(raw.proto_idx)).unwrap();
    assert_eq!(descriptor(dex, proto.return_type_idx), "V");
    assert!(proto.parameters.as_ref().is_none_or(|p| p.types.is_empty()));
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == owner)
        .unwrap();
    let encoded = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|m| m.method_idx == 34682)
        .unwrap();
    assert_eq!(encoded.access_flags & 2, 2);
    // The original uses explicit monitor-enter/exit rather than ACC_SYNCHRONIZED.
    let code = encoded.code.as_ref().unwrap();
    assert!(code.insns.len() >= 49);
    assert!(!code.tries.is_empty());
    let jdk = std::env::var_os("DECOMPHIFY_JDK25_CLASSPATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/decomphify-source25-java-base-649174/java.base.jar".into());
    assert!(jdk.is_file());
    let root = std::env::var_os("DECOMPHIFY_OBJECT_SCOPE_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/opencode/decomphify-object-scope-actual".into());
    std::fs::create_dir_all(&root).unwrap();
    let options = JavaOptions {
        source_release: 25,
        class_filter: Some(owner.into()),
        workers: 1,
        classpath: vec![
            jdk.clone(),
            "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar".into(),
        ],
        ..JavaOptions::default()
    };
    let mut units = Vec::new();
    let report = emit_java(&inputs, &options, &mut |u: &GeneratedSource| {
        units.push(u.clone());
        Ok(())
    })
    .unwrap();
    assert_eq!(units.len(), 1);
    let unit = units.remove(0);
    assert_eq!(unit.provenance.descriptor, owner);
    let path = root.join("original-units").join(&unit.relative_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &unit.text).unwrap();
    let marker = "    // original method pool index 34682;";
    let start_method = unit.text.find(marker).unwrap();
    let tail = &unit.text[start_method..];
    let identity = tail.lines().next().unwrap();
    eprintln!(
        "original pool2 method34682 elapsed_s={:.3} source={} identity={} report={report:?}",
        start.elapsed().as_secs_f64(),
        path.display(),
        identity
    );
    std::fs::write(root.join("source-proof.txt"),format!("owner={owner}\nmethod=34682\npool=2\nsource_release=25\nclasspath={}\nelapsed_s={:.3}\nsource={}\nidentity={}\n",jdk.display(),start.elapsed().as_secs_f64(),path.display(),identity)).unwrap();
    assert!(
        identity.contains("complete_body=true; semantic_complete=true; dependency_complete=true"),
        "{identity}\n{}",
        &tail[..tail.len().min(3000)]
    );
    let end = tail[marker.len()..]
        .find("    // original method pool index ")
        .map(|offset| offset + marker.len())
        .unwrap_or(tail.len());
    let body = &tail[..end];
    assert!(
        body.contains("new java.lang.UnsupportedOperationException("),
        "{body}"
    );
    assert_eq!(
        body.matches("new java.lang.UnsupportedOperationException(")
            .count(),
        2
    );
    assert!(body.contains("synchronized"), "{body}");
    std::fs::write(root.join("original-method-34682.java.txt"), body).unwrap();
}

fn tool(tool: &str, args: &[&str], dir: &std::path::Path) -> std::process::Output {
    let home = std::env::var("DECOMPHIFY_JDK25_HOME")
        .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into());
    std::process::Command::new(format!("{home}/bin/{tool}"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}
fn write(dir: &std::path::Path, path: &str, source: &str) {
    let file = dir.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, source).unwrap();
}
#[test]
#[ignore = "requires retained original Spass facade source and genuine JDK25/Android35; explicit API boundaries"]
fn retained_original_spass_34682_compile_and_bytecode_differential() {
    let root = std::env::var_os("DECOMPHIFY_OBJECT_SCOPE_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/opencode/decomphify-object-scope-actual".into());
    let source_path =
        root.join("original-units/com/samsung/android/sdk/pass/SpassFingerprint.java");
    let unit = std::fs::read_to_string(&source_path).unwrap();
    let start = unit
        .find("    // original method pool index 34682;")
        .unwrap();
    let tail = &unit[start..];
    let end = tail[10..]
        .find("    // original method pool index ")
        .map(|offset| offset + 10)
        .unwrap_or(tail.len());
    let body = &tail[..end];
    assert!(
        body.lines()
            .next()
            .unwrap()
            .contains("complete_body=true; semantic_complete=true; dependency_complete=true")
    );
    assert_eq!(
        body.matches("new java.lang.UnsupportedOperationException(")
            .count(),
        2
    );
    assert!(body.contains("synchronized (this)"));
    let inputs = dex_format::apk::parse_apk_path(APK).unwrap();
    let dex = &inputs[2];
    assert_eq!(
        descriptor(dex, 9116),
        "Ljava/lang/UnsupportedOperationException;"
    );
    for (index, owner, name, parameters, ret) in [
        (
            48801,
            "Ljava/lang/UnsupportedOperationException;",
            "<init>",
            vec!["Ljava/lang/String;"],
            "V",
        ),
        (
            1145,
            "Landroid/util/Log;",
            "i",
            vec!["Ljava/lang/String;", "Ljava/lang/String;"],
            "I",
        ),
        (
            34758,
            "Lcom/samsung/android/sdk/pass/support/v1/FingerprintManagerProxyFactory;",
            "create",
            vec!["Landroid/content/Context;"],
            "Lcom/samsung/android/sdk/pass/support/IFingerprintManagerProxy;",
        ),
    ] {
        let m = dex.method_id(index).unwrap();
        assert_eq!(descriptor(dex, u32::from(m.class_idx)), owner);
        assert_eq!(dex.string(m.name_idx).unwrap(), name);
        let proto = dex.proto_id(u32::from(m.proto_idx)).unwrap();
        assert_eq!(descriptor(dex, proto.return_type_idx), ret);
        assert_eq!(
            proto
                .parameters
                .iter()
                .flat_map(|p| &p.types)
                .map(|t| descriptor(dex, u32::from(*t)))
                .collect::<Vec<_>>(),
            parameters
        );
    }
    for (index, name, ty) in [
        (
            49548,
            "a",
            "Lcom/samsung/android/sdk/pass/support/IFingerprintManagerProxy;",
        ),
        (49549, "b", "Landroid/content/Context;"),
        (49565, "r", "Z"),
    ] {
        let f = dex.field_id(index).unwrap();
        assert_eq!(
            descriptor(dex, u32::from(f.class_idx)),
            "Lcom/samsung/android/sdk/pass/SpassFingerprint;"
        );
        assert_eq!(dex.string(f.name_idx).unwrap(), name);
        assert_eq!(descriptor(dex, u32::from(f.type_idx)), ty);
    }
    let dir = root.join("runtime-34682");
    write(
        &dir,
        "com/samsung/android/sdk/pass/SpassFingerprint.java",
        &format!(
            "package com.samsung.android.sdk.pass;public class SpassFingerprint {{private com.samsung.android.sdk.pass.support.IFingerprintManagerProxy a;private android.content.Context b;private static boolean r;\n{body}\n}}"
        ),
    );
    write(
        &dir,
        "com/samsung/android/sdk/pass/support/IFingerprintManagerProxy.java",
        "package com.samsung.android.sdk.pass.support;public interface IFingerprintManagerProxy {}\n",
    );
    write(
        &dir,
        "com/samsung/android/sdk/pass/support/v1/FingerprintManagerProxyFactory.java",
        r#"package com.samsung.android.sdk.pass.support.v1;
public final class FingerprintManagerProxyFactory {public static final com.samsung.android.sdk.pass.support.IFingerprintManagerProxy proxy=new com.samsung.android.sdk.pass.support.IFingerprintManagerProxy() {};
public static com.samsung.android.sdk.pass.support.IFingerprintManagerProxy create(android.content.Context c) {com.samsung.android.sdk.pass.Driver.events+="F";if(c!=null)throw new AssertionError();if(com.samsung.android.sdk.pass.Driver.mode==5)throw com.samsung.android.sdk.pass.Driver.failure;return com.samsung.android.sdk.pass.Driver.mode==3?null:proxy;}}
"#,
    );
    write(
        &dir,
        "android/util/Log.java",
        r#"package android.util;public final class Log {static {com.samsung.android.sdk.pass.Driver.events+="L";if(com.samsung.android.sdk.pass.Driver.mode==6)throw new OutOfMemoryError("explicit class-init fixture boundary");}
public static int i(String tag,String message) {com.samsung.android.sdk.pass.Driver.events+="I";if(!tag.equals("SpassFingerprintSDK")||!message.equals("ensureServiceSupported : proxy is null, retry to create proxy"))throw new AssertionError();if(com.samsung.android.sdk.pass.Driver.mode==4)throw com.samsung.android.sdk.pass.Driver.failure;return 0;}}
"#,
    );
    write(
        &dir,
        "com/samsung/android/sdk/pass/Driver.java",
        r#"package com.samsung.android.sdk.pass;import java.lang.reflect.*;
public class Driver {public static int mode;public static String events="";public static final RuntimeException failure=new IllegalArgumentException("original-boundary");
public static void main(String[]args)throws Exception {mode=Integer.parseInt(args[0]);SpassFingerprint target=new SpassFingerprint();Field r=SpassFingerprint.class.getDeclaredField("r"),a=SpassFingerprint.class.getDeclaredField("a");r.setAccessible(true);a.setAccessible(true);r.setBoolean(null,mode!=0);if(mode==1)a.set(target,com.samsung.android.sdk.pass.support.v1.FingerprintManagerProxyFactory.proxy);
Method e=SpassFingerprint.class.getDeclaredMethod("e");e.setAccessible(true);Throwable thrown=null;try {e.invoke(target);}catch(InvocationTargetException x){thrown=x.getCause();}
if(Thread.holdsLock(target))throw new AssertionError("monitor remains owned");Thread other=new Thread(()->{synchronized(target){}});other.start();other.join(2000);if(other.isAlive())throw new AssertionError("monitor not released");
System.out.println(events+":"+(thrown==null?"ok":thrown.getClass().getSimpleName()+":"+thrown.getMessage())+":"+(thrown==failure)+":"+(a.get(target)==null));}}
"#,
    );
    let cp = "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar";
    let compile = tool(
        "javac",
        &[
            "--release",
            "25",
            "-cp",
            cp,
            "com/samsung/android/sdk/pass/SpassFingerprint.java",
            "com/samsung/android/sdk/pass/Driver.java",
            "com/samsung/android/sdk/pass/support/IFingerprintManagerProxy.java",
            "com/samsung/android/sdk/pass/support/v1/FingerprintManagerProxyFactory.java",
            "android/util/Log.java",
        ],
        &dir,
    );
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let classpath = format!(".:{cp}");
    let mut after = Vec::new();
    for mode in 0..7 {
        let out = tool(
            "java",
            &[
                "-Xverify:all",
                "-cp",
                &classpath,
                "com.samsung.android.sdk.pass.Driver",
                &mode.to_string(),
            ],
            &dir,
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        after.extend(out.stdout);
    }
    write(
        &dir,
        "Original.java",
        r#"import java.lang.classfile.*;import java.lang.constant.*;import java.nio.file.*;
public class Original {public static void main(String[]args)throws Exception {ClassFile cf=ClassFile.of();ClassDesc self=ClassDesc.of("com.samsung.android.sdk.pass.SpassFingerprint"),proxy=ClassDesc.of("com.samsung.android.sdk.pass.support.IFingerprintManagerProxy"),context=ClassDesc.of("android.content.Context"),unsupported=ClassDesc.of("java.lang.UnsupportedOperationException");Path path=Path.of("com/samsung/android/sdk/pass/SpassFingerprint.class");Files.write(path,cf.transformClass(cf.parse(path),(cb,element)->{if(element instanceof MethodModel mm&&mm.methodName().equalsString("e"))cb.transformMethod(mm,(mb,me)->{if(me instanceof CodeModel)mb.withCode(c->{Label start=c.newLabel(),success=c.newLabel(),platform=c.newLabel(),handler=c.newLabel();
c.aload(0).monitorenter().labelBinding(start);c.getstatic(self,"r",ConstantDescs.CD_boolean).ifeq(platform);c.aload(0).getfield(self,"a",proxy).ifnonnull(success);c.ldc("SpassFingerprintSDK").ldc("ensureServiceSupported : proxy is null, retry to create proxy").invokestatic(ClassDesc.of("android.util.Log"),"i",MethodTypeDesc.ofDescriptor("(Ljava/lang/String;Ljava/lang/String;)I")).pop();c.aload(0).aload(0).getfield(self,"b",context).invokestatic(ClassDesc.of("com.samsung.android.sdk.pass.support.v1.FingerprintManagerProxyFactory"),"create",MethodTypeDesc.ofDescriptor("(Landroid/content/Context;)Lcom/samsung/android/sdk/pass/support/IFingerprintManagerProxy;")).putfield(self,"a",proxy);c.aload(0).getfield(self,"a",proxy).ifnonnull(success);
c.new_(unsupported).astore(1);c.ldc("Fingerprint Service is not running on the device.").astore(2);c.aload(1).aload(2).invokespecial(unsupported,"<init>",MethodTypeDesc.ofDescriptor("(Ljava/lang/String;)V"));c.aload(1).athrow();
c.labelBinding(platform).new_(unsupported).astore(1);c.ldc("Fingerprint Service is not supported in the platform.").astore(2);c.aload(1).aload(2).invokespecial(unsupported,"<init>",MethodTypeDesc.ofDescriptor("(Ljava/lang/String;)V"));c.aload(1).athrow();
c.labelBinding(success).aload(0).monitorexit().return_();c.labelBinding(handler).astore(3).aload(0).monitorexit().aload(3).athrow();c.exceptionCatchAll(start,success,handler);
});else mb.with(me);});else cb.with(element);}));}}
"#,
    );
    for (name, args) in [
        ("javac", vec!["--release", "25", "Original.java"]),
        ("java", vec!["Original"]),
    ] {
        let out = tool(name, &args, &dir);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let mut before = Vec::new();
    for mode in 0..7 {
        let out = tool(
            "java",
            &[
                "-Xverify:all",
                "-cp",
                &classpath,
                "com.samsung.android.sdk.pass.Driver",
                &mode.to_string(),
            ],
            &dir,
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        before.extend(out.stdout);
    }
    assert_eq!(before, after);
    std::fs::write(dir.join("runtime-traces.txt"), &after).unwrap();
    write(
        &dir,
        "scope.txt",
        "Original retained method34682 body unchanged; surrounding fields and peers are explicit API boundary fixtures verified against genuine DEX descriptors. Android Context declaration comes from genuine Android35. Reference JVM e body retains original explicit monitor, branches, strings, new-before-string pairs, ctor descriptor, and catch-all release/rethrow. Seven JVM modes cover both throw messages, cached and factory success, factory-null, log and factory original exception identity, and explicit Log class-init OOM boundary. This is not whole-class compilation or Android original runtime, and not heap-exhaustion OOM.\n",
    );
    let full = tool(
        "javac",
        &[
            "--release",
            "25",
            "-Xmaxerrs",
            "8",
            "-cp",
            cp,
            source_path.to_str().unwrap(),
        ],
        &root,
    );
    std::fs::write(root.join("whole-unit-javac.stderr"), full.stderr).unwrap();
    std::fs::write(
        root.join("whole-unit-javac.status"),
        format!("success={}\n", full.status.success()),
    )
    .unwrap();
    eprintln!(
        "retained original34682 subset compile and seven-mode JVM differential PASS; artifacts={}",
        dir.display()
    );
}
