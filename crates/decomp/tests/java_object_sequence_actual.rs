//! Original Welcome allocation followed by a genuine stack-sensitive void check.
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
#[ignore = "requires actual Samsung APK, genuine JDK25/Android35; selected class only"]
fn original_welcome_37968_sequence_source() {
    let start = Instant::now();
    let inputs = dex_format::apk::parse_apk_path(APK).unwrap();
    let dex = &inputs[1];
    let raw = dex.method_id(37968).unwrap();
    let owner = "Lcom/samsung/android/samsungpass/intro/WelcomeFragment;";
    assert_eq!(descriptor(dex, u32::from(raw.class_idx)), owner);
    assert_eq!(dex.string(raw.name_idx).unwrap(), "getViewHolder");
    let proto = dex.proto_id(u32::from(raw.proto_idx)).unwrap();
    assert_eq!(
        descriptor(dex, proto.return_type_idx),
        "Lcom/samsung/android/samsungpass/intro/WelcomeFragmentViewHolder;"
    );
    assert!(proto.parameters.as_ref().is_none_or(|p| p.types.is_empty()));
    let root = std::env::var_os("DECOMPHIFY_OBJECT_SEQUENCE_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/opencode/decomphify-object-sequence-actual".into());
    std::fs::create_dir_all(&root).unwrap();
    let jdk = std::env::var_os("DECOMPHIFY_JDK25_CLASSPATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/decomphify-source25-java-base-649174/java.base.jar".into());
    assert!(jdk.is_file());
    let options = JavaOptions {
        source_release: 25,
        class_filter: Some(owner.into()),
        workers: 1,
        classpath: vec![
            jdk,
            "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar".into(),
        ],
        ..JavaOptions::default()
    };
    let mut units = Vec::new();
    let report = emit_java(&inputs, &options, &mut |unit: &GeneratedSource| {
        units.push(unit.clone());
        Ok(())
    })
    .unwrap();
    assert_eq!(units.len(), 1);
    let unit = units.remove(0);
    assert_eq!(unit.provenance.descriptor, owner);
    let file = root.join("original-units").join(&unit.relative_path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, &unit.text).unwrap();
    let marker = "    // original method pool index 37968;";
    let start_method = unit.text.find(marker).unwrap();
    let tail = &unit.text[start_method..];
    let end = tail[marker.len()..]
        .find("    // original method pool index ")
        .map(|n| n + marker.len())
        .unwrap_or(tail.len());
    let body = &tail[..end];
    std::fs::write(root.join("original-method-37968.java.txt"), body).unwrap();
    let identity = body.lines().next().unwrap();
    eprintln!(
        "original pool1 method37968 elapsed_s={:.3} source={} identity={} report={report:?}",
        start.elapsed().as_secs_f64(),
        file.display(),
        identity
    );
    std::fs::write(
        root.join("source-proof.txt"),
        format!(
            "pool=1\nmethod=37968\nsource_release=25\nelapsed_s={:.3}\nsource={}\nidentity={}\n",
            start.elapsed().as_secs_f64(),
            file.display(),
            identity
        ),
    )
    .unwrap();
    assert!(
        identity.contains("complete_body=true; semantic_complete=true; dependency_complete=true"),
        "{body}"
    );
    assert!(body.contains("switch (0)"), "{body}");
    assert!(body.contains("compiler scaffold: original allocation argument sequence"));
    assert!(
        body.contains("new com.samsung.android.samsungpass.intro.WelcomeFragmentViewHolderImpl(")
    );
}

fn write(dir: &std::path::Path, path: &str, source: &str) {
    let file = dir.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, source).unwrap();
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
#[test]
#[ignore = "requires retained actual Welcome method, genuine DEX and JDK25; explicit API boundaries"]
fn retained_original_welcome_37968_compile_and_runtime() {
    let root = std::env::var_os("DECOMPHIFY_OBJECT_SEQUENCE_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/opencode/decomphify-object-sequence-actual".into());
    let body = std::fs::read_to_string(root.join("original-method-37968.java.txt")).unwrap();
    assert!(
        body.lines()
            .next()
            .unwrap()
            .contains("complete_body=true; semantic_complete=true; dependency_complete=true")
    );
    assert!(
        body.contains("new com.samsung.android.samsungpass.intro.WelcomeFragmentViewHolderImpl(")
    );
    assert_eq!(body.matches("switch (0)").count(), 1);
    assert_eq!(body.matches("kotlin.jvm.internal.n$type.e(").count(), 1);
    let inputs = dex_format::apk::parse_apk_path(APK).unwrap();
    let dex = &inputs[1];
    for (index, owner, name, params, ret) in [
        (
            38042,
            "Lcom/samsung/android/samsungpass/intro/WelcomeFragmentViewHolderImpl;",
            "<init>",
            vec![
                "Lcom/samsung/android/samsungpass/intro/IntroViewModel;",
                "Landroidx/fragment/app/FragmentActivity;",
            ],
            "V",
        ),
        (
            37969,
            "Lcom/samsung/android/samsungpass/intro/WelcomeFragment;",
            "getViewModel$SamsungPass_prodRelease",
            vec![],
            "Lcom/samsung/android/samsungpass/intro/IntroViewModel;",
        ),
        (
            1656,
            "Landroidx/fragment/app/Fragment;",
            "requireActivity",
            vec![],
            "Landroidx/fragment/app/FragmentActivity;",
        ),
        (
            47446,
            "Lkotlin/jvm/internal/n;",
            "e",
            vec!["Ljava/lang/Object;", "Ljava/lang/String;"],
            "V",
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
            params
        );
    }
    let field = dex.field_id(58055).unwrap();
    assert_eq!(
        descriptor(dex, u32::from(field.class_idx)),
        "Lcom/samsung/android/samsungpass/intro/WelcomeFragment;"
    );
    assert_eq!(dex.string(field.name_idx).unwrap(), "_viewHolder");
    assert_eq!(
        descriptor(dex, u32::from(field.type_idx)),
        "Lcom/samsung/android/samsungpass/intro/WelcomeFragmentViewHolder;"
    );
    assert_eq!(dex.string(55338).unwrap(), "requireActivity(...)");
    let dir = root.join("runtime-37968");
    write(
        &dir,
        "com/samsung/android/samsungpass/intro/WelcomeFragment.java",
        &format!(
            r#"package com.samsung.android.samsungpass.intro;public class WelcomeFragment extends androidx.fragment.app.Fragment {{private WelcomeFragmentViewHolder _viewHolder;public final IntroViewModel getViewModel$SamsungPass_prodRelease() {{Driver.events+="M";if(Driver.mode==3)throw Driver.failure;if(Driver.mode==8||Driver.mode==9)_viewHolder=Driver.old;return Driver.model;}}{body}}}"#
        ),
    );
    write(
        &dir,
        "com/samsung/android/samsungpass/intro/WelcomeFragmentViewHolder.java",
        "package com.samsung.android.samsungpass.intro;public interface WelcomeFragmentViewHolder {}\n",
    );
    write(
        &dir,
        "com/samsung/android/samsungpass/intro/IntroViewModel.java",
        "package com.samsung.android.samsungpass.intro;public class IntroViewModel {}\n",
    );
    write(
        &dir,
        "androidx/fragment/app/FragmentActivity.java",
        "package androidx.fragment.app;public class FragmentActivity {}\n",
    );
    write(
        &dir,
        "androidx/fragment/app/Fragment.java",
        r#"package androidx.fragment.app;public class Fragment {public final FragmentActivity requireActivity(){com.samsung.android.samsungpass.intro.Driver.events+="R";if(com.samsung.android.samsungpass.intro.Driver.mode==4)throw com.samsung.android.samsungpass.intro.Driver.failure;return com.samsung.android.samsungpass.intro.Driver.mode==2||com.samsung.android.samsungpass.intro.Driver.mode==9?null:com.samsung.android.samsungpass.intro.Driver.activity;}}
"#,
    );
    write(
        &dir,
        "kotlin/jvm/internal/n$type.java",
        r#"package kotlin.jvm.internal;public class n$type {public static void e(Object o,String name){com.samsung.android.samsungpass.intro.Driver.events+="K";StackTraceElement caller=new Throwable().getStackTrace()[1];if(!caller.getMethodName().equals("getViewHolder")||!caller.getClassName().equals("com.samsung.android.samsungpass.intro.WelcomeFragment"))throw new AssertionError("check caller frame changed");if(!name.equals("requireActivity(...)"))throw new AssertionError();if(com.samsung.android.samsungpass.intro.Driver.mode==5)throw com.samsung.android.samsungpass.intro.Driver.failure;if(o==null)throw new NullPointerException(name);}}
"#,
    );
    write(
        &dir,
        "com/samsung/android/samsungpass/intro/WelcomeFragmentViewHolderImpl.java",
        r#"package com.samsung.android.samsungpass.intro;public class WelcomeFragmentViewHolderImpl implements WelcomeFragmentViewHolder {static {Driver.events+="A";if(Driver.mode==7)throw new OutOfMemoryError("explicit allocation-site class-init fixture boundary");}public final IntroViewModel model;public final androidx.fragment.app.FragmentActivity activity;public WelcomeFragmentViewHolderImpl(IntroViewModel m,androidx.fragment.app.FragmentActivity a){Driver.events+="C";if(Driver.mode==6)throw Driver.failure;model=m;activity=a;}}
"#,
    );
    write(
        &dir,
        "com/samsung/android/samsungpass/intro/Driver.java",
        r#"package com.samsung.android.samsungpass.intro;import java.lang.reflect.*;public class Driver {public static int mode;public static String events="";public static final RuntimeException failure=new IllegalArgumentException("original-boundary");public static final IntroViewModel model=new IntroViewModel();public static final androidx.fragment.app.FragmentActivity activity=new androidx.fragment.app.FragmentActivity();public static final WelcomeFragmentViewHolder old=new WelcomeFragmentViewHolder(){};
public static void main(String[]args)throws Exception{mode=Integer.parseInt(args[0]);WelcomeFragment target=new WelcomeFragment();Field field=WelcomeFragment.class.getDeclaredField("_viewHolder");field.setAccessible(true);if(mode==1)field.set(target,old);Method getter=WelcomeFragment.class.getDeclaredMethod("getViewHolder");getter.setAccessible(true);Throwable thrown=null;Object returned=null;try{returned=getter.invoke(target);if(returned!=field.get(target))throw new AssertionError("capture identity");if(mode!=1){WelcomeFragmentViewHolderImpl impl=(WelcomeFragmentViewHolderImpl)returned;if(impl.model!=model||impl.activity!=activity)throw new AssertionError("exact ctor args");}}catch(InvocationTargetException e){thrown=e.getCause();}System.out.println(events+":"+(thrown==null?"ok":thrown.getClass().getSimpleName())+":"+(thrown==failure)+":"+(field.get(target)==old)+":"+(field.get(target)==null));}}
"#,
    );
    let files = [
        "--release",
        "25",
        "com/samsung/android/samsungpass/intro/WelcomeFragment.java",
        "com/samsung/android/samsungpass/intro/WelcomeFragmentViewHolder.java",
        "com/samsung/android/samsungpass/intro/IntroViewModel.java",
        "com/samsung/android/samsungpass/intro/WelcomeFragmentViewHolderImpl.java",
        "com/samsung/android/samsungpass/intro/Driver.java",
        "androidx/fragment/app/Fragment.java",
        "androidx/fragment/app/FragmentActivity.java",
        "kotlin/jvm/internal/n$type.java",
    ];
    let compile = tool("javac", &files, &dir);
    assert!(
        compile.status.success(),
        "{}\n{body}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let mut after = Vec::new();
    for mode in 0..10 {
        let out = tool(
            "java",
            &[
                "-Xverify:all",
                "com.samsung.android.samsungpass.intro.Driver",
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
public class Original {public static void main(String[]args)throws Exception{ClassFile cf=ClassFile.of();ClassDesc self=ClassDesc.of("com.samsung.android.samsungpass.intro.WelcomeFragment"),holder=ClassDesc.of("com.samsung.android.samsungpass.intro.WelcomeFragmentViewHolder"),impl=ClassDesc.of("com.samsung.android.samsungpass.intro.WelcomeFragmentViewHolderImpl"),model=ClassDesc.of("com.samsung.android.samsungpass.intro.IntroViewModel"),fragment=ClassDesc.of("androidx.fragment.app.Fragment"),activity=ClassDesc.of("androidx.fragment.app.FragmentActivity");Path path=Path.of("com/samsung/android/samsungpass/intro/WelcomeFragment.class");Files.write(path,cf.transformClass(cf.parse(path),(cb,element)->{if(element instanceof MethodModel mm&&mm.methodName().equalsString("getViewHolder"))cb.transformMethod(mm,(mb,me)->{if(me instanceof CodeModel)mb.withCode(c->{Label join=c.newLabel();
c.aload(0).getfield(self,"_viewHolder",holder).astore(1);c.aload(1).ifnonnull(join);c.new_(impl).astore(2);c.aload(0).invokevirtual(self,"getViewModel$SamsungPass_prodRelease",MethodTypeDesc.ofDescriptor("()Lcom/samsung/android/samsungpass/intro/IntroViewModel;")).astore(3);c.aload(0).invokevirtual(fragment,"requireActivity",MethodTypeDesc.ofDescriptor("()Landroidx/fragment/app/FragmentActivity;")).astore(4);c.ldc("requireActivity(...)").astore(5);c.aload(4).aload(5).invokestatic(ClassDesc.of("kotlin.jvm.internal.n$type"),"e",MethodTypeDesc.ofDescriptor("(Ljava/lang/Object;Ljava/lang/String;)V"));c.aload(2).aload(3).aload(4).invokespecial(impl,"<init>",MethodTypeDesc.ofDescriptor("(Lcom/samsung/android/samsungpass/intro/IntroViewModel;Landroidx/fragment/app/FragmentActivity;)V"));c.aload(0).aload(2).putfield(self,"_viewHolder",holder);c.aload(2).astore(1);c.labelBinding(join).aload(1).areturn();
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
    for mode in 0..10 {
        let out = tool(
            "java",
            &[
                "-Xverify:all",
                "com.samsung.android.samsungpass.intro.Driver",
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
    assert_eq!(
        String::from_utf8(after).unwrap(),
        "AMRKC:ok:false:false:false\n:ok:false:true:false\nAMRK:NullPointerException:false:false:true\nAM:IllegalArgumentException:true:false:true\nAMR:IllegalArgumentException:true:false:true\nAMRK:IllegalArgumentException:true:false:true\nAMRKC:IllegalArgumentException:true:false:true\nA:OutOfMemoryError:false:false:true\nAMRKC:ok:false:false:false\nAMRK:NullPointerException:false:true:false\n"
    );
    write(
        &dir,
        "scope.txt",
        "Original pool1 method37968 source body unchanged, including genuine checkmethod n.e and getViewHolder caller frame; surrounding classes/methods are explicit API/runtime boundary fixtures whose descriptors match original DEX, not recovered app bodies. Public ClassFile reference retains original NEW at4 before getters6/10, string14, voidcheck16, exact ctor19 and ownstore22. Ten modes compare normal/cached identity, every call failure, class-init OOM boundary, and getter-mutated field state on normal/failing paths. Not whole-unit closure, Android/Kotlin original runtime, or heap-exhaustion OOM.\n",
    );
    let source =
        root.join("original-units/com/samsung/android/samsungpass/intro/WelcomeFragment.java");
    let full = tool(
        "javac",
        &[
            "--release",
            "25",
            "-Xmaxerrs",
            "8",
            "-cp",
            "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
            source.to_str().unwrap(),
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
        "retained original37968 method subset compile and ten-mode original JVM differential PASS artifacts={}",
        dir.display()
    );
}
