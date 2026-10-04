//! Actual Samsung prefix effects through the seeded production facade. Runtime
//! dependencies below are declared API boundaries, not real framework bodies.
#![allow(clippy::unwrap_used)]
use decomp::{
    dex_java::{JavaOptions, emit_java},
    output::GeneratedSource,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};
fn jdk() -> PathBuf {
    std::env::var_os("DECOMPHIFY_JDK25_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into())
}
fn run(tool: &str, args: &[&str], dir: &Path) -> Vec<u8> {
    let out = Command::new(jdk().join("bin").join(tool))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
fn write(dir: &Path, path: &str, text: &str) -> PathBuf {
    let p = dir.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, text).unwrap();
    p
}
#[test]
#[ignore = "requires retained Samsung APK, genuine extracted JDK25 jar and JDK25; two selected units only"]
fn samsung_eager_literal_resolution_and_shared_static_value_before_chain() {
    let start = Instant::now();
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let Some(cp) = std::env::var_os("DECOMPHIFY_JAVA_BASE25_JAR") else {
        panic!("provide genuine extracted Java25 java.base.jar");
    };
    let cp = PathBuf::from(cp);
    assert!(cp.is_file());
    let hash = Command::new("sha256sum").arg(&cp).output().unwrap();
    assert!(hash.status.success());
    eprintln!(
        "actual prefix genuine classpath {}",
        String::from_utf8(hash.stdout).unwrap().trim()
    );
    let dir = std::env::temp_dir().join(format!(
        "decomphify-source25-actual-effects-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut units = vec![];
    for (owner, index) in [("Lfu/v$a;", 1719), ("Lrt/e;", 59430)] {
        let (pool, dex, class) = inputs
            .iter()
            .enumerate()
            .find_map(|(pool, dex)| {
                dex.classes
                    .iter()
                    .find(|c| {
                        dex.string(dex.type_id(c.class_idx).unwrap().descriptor_idx)
                            .unwrap()
                            == owner
                    })
                    .map(|class| (pool, dex, class))
            })
            .unwrap();
        let original = class
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods
            .iter()
            .find(|m| m.method_idx == index)
            .unwrap();
        let code = original.code.as_ref().unwrap();
        assert!(code.tries.is_empty());
        let words = code
            .insns
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect::<Vec<_>>();
        let raw = dex.method_id(index).unwrap();
        assert_eq!(dex.string(raw.name_idx).unwrap(), "<init>");
        if index == 1719 {
            assert_eq!(words.len(), 13);
            for (offset, opcode) in [
                (0, 0x1a),
                (2, 0x12),
                (3, 0x12),
                (4, 0x1c),
                (6, 0x1a),
                (8, 0x07),
                (9, 0x76),
                (12, 0x0e),
            ] {
                assert_eq!(words[offset] & 255, opcode);
            }
            assert_eq!(
                dex.string(u32::from(words[1])).unwrap(),
                "emit(Ljava/lang/Object;Lkotlin/coroutines/Continuation;)Ljava/lang/Object;"
            );
            assert_eq!(dex.string(u32::from(words[7])).unwrap(), "emit");
            assert_eq!(
                dex.string(dex.type_id(u32::from(words[5])).unwrap().descriptor_idx)
                    .unwrap(),
                "Leu/g;"
            );
        } else {
            assert_eq!(words.len(), 11);
            for (offset, opcode) in [(0, 0x62), (2, 0x1a), (4, 0x71), (7, 0x70), (10, 0x0e)] {
                assert_eq!(words[offset] & 255, opcode);
            }
            let check = dex.method_id(u32::from(words[5])).unwrap();
            assert_eq!(dex.string(check.name_idx).unwrap(), "d");
            assert_eq!(u32::from(words[8]), 59431);
        }
        eprintln!(
            "pinned original prefix pool={pool} owner={owner} method={index} raw_words={words:?}"
        );
        let options = JavaOptions {
            source_release: 25,
            class_filter: Some(owner.into()),
            workers: 1,
            classpath: vec![
                cp.clone(),
                "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar".into(),
            ],
            ..JavaOptions::default()
        };
        let phase = Instant::now();
        let mut selected = vec![];
        let report = emit_java(&inputs, &options, &mut |unit: &GeneratedSource| {
            selected.push(unit.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(selected.len(), 1);
        let unit = selected.remove(0);
        assert_eq!(unit.provenance.descriptor, owner);
        write(&dir, &unit.relative_path.to_string_lossy(), &unit.text);
        let identity = unit
            .text
            .lines()
            .find(|line| line.contains(&format!("original method pool index {index};")))
            .unwrap_or_else(|| panic!("{}", unit.text));
        assert!(
            identity.contains(owner)
                && identity.contains("name: \"<init>\"")
                && identity.contains("complete_body=true")
        );
        eprintln!(
            "actual eager prefix owner={owner} original_index={index} complete_ctors={} elapsed_s={:.3}",
            report.complete_constructors,
            phase.elapsed().as_secs_f64()
        );
        units.push(unit);
    }
    let fu = &units[0];
    let rt = &units[1];
    // Original fu/v$a constructor: signature string at0, class literal at4,
    // name string at6, then parent invocation at9. Operand order differs from
    // those original resolution sites; eager locals retain each original site.
    let name = fu.text.find("\"emit\"").unwrap();
    let signature = fu
        .text
        .find("\"emit(Ljava/lang/Object;Lkotlin/coroutines/Continuation;)Ljava/lang/Object;\"")
        .unwrap();
    let class = fu.text.find("eu.g$type.class").unwrap();
    let chain = fu.text.find("super(").unwrap();
    assert!(signature < class && class < name && name < chain);
    // Original rt/e() at pool59430: sget singleton, const-string, static check,
    // then this(singleton). The same sget value has two original uses.
    let ctor = rt.text[rt.text.find("original method pool index 59430;").unwrap()..]
        .split("original method pool index 59431;")
        .next()
        .unwrap();
    assert_eq!(ctor.matches("rt.i$type.a").count(), 1);
    assert!(ctor.find("kotlin.jvm.internal.n$type.d(").unwrap() < ctor.find("this(").unwrap());
    // Compile the entire unchanged fu unit against explicit API boundaries. The
    // rt unit's larger ordinary-method closure is retained for inspection only.
    for (path, body) in [
        ("jq/e$type.java", "package jq;public interface e$type {}"),
        (
            "eu/g$type.java",
            "package eu;public interface g$type {Object emit(Object p,jq.e$type c);}",
        ),
        (
            "xq/q$type.java",
            "package xq;public interface q$type {Object invoke(Object a,Object b,Object c);}",
        ),
        (
            "kotlin/jvm/internal/l$type.java",
            r#"package kotlin.jvm.internal;public class l$type {public l$type(int arity,Class<?> owner,String name,String signature,int flags){if(arity!=3||owner!=eu.g$type.class||!name.equals("emit")||!signature.equals("emit(Ljava/lang/Object;Lkotlin/coroutines/Continuation;)Ljava/lang/Object;")||flags!=0)throw new AssertionError("parent arguments");System.out.println("parent:arguments=true");}}"#,
        ),
        (
            "Driver.java",
            "public class Driver {public static void main(String[] a){if(fu.v$a.a==null)throw new AssertionError(\"original singleton\");}}",
        ),
    ] {
        write(&dir, path, body);
    }
    run(
        "javac",
        &[
            "--release",
            "25",
            "fu/v$a.java",
            "jq/e$type.java",
            "eu/g$type.java",
            "xq/q$type.java",
            "kotlin/jvm/internal/l$type.java",
            "Driver.java",
        ],
        &dir,
    );
    let after = run("java", &["-Xverify:all", "Driver"], &dir);
    assert_eq!(after, b"parent:arguments=true\n");
    // Restore only the original straight-line fu constructor instructions.
    // The clinit and ordinary methods, aliases and dependency fixtures remain
    // byte-for-byte shared by the before/after constructor comparison.
    write(
        &dir,
        "Original.java",
        r#"import java.nio.file.*;import java.lang.classfile.*;import java.lang.constant.*;
public class Original {
 public static void main(String[] args)throws Exception {
  ClassFile cf=ClassFile.of();Path path=Path.of("fu/v$a.class");ClassModel m=cf.parse(path);
  byte[] out=cf.transformClass(m,(cb,e)->{
   if(e instanceof MethodModel mm&&mm.methodName().equalsString("<init>"))cb.transformMethod(mm,(mb,me)->{
    if(me instanceof CodeModel)mb.withCode(c->{
     c.ldc("emit(Ljava/lang/Object;Lkotlin/coroutines/Continuation;)Ljava/lang/Object;").astore(1);
     c.iconst_0().istore(2);c.iconst_3().istore(3);
     c.ldc(ClassDesc.of("eu.g$type")).astore(4);c.ldc("emit").astore(5);
     c.aload(0).iload(3).aload(4).aload(5).aload(1).iload(2).invokespecial(ClassDesc.of("kotlin.jvm.internal.l$type"),"<init>",MethodTypeDesc.ofDescriptor("(ILjava/lang/Class;Ljava/lang/String;Ljava/lang/String;I)V")).return_();
    });else mb.with(me);
   });else cb.with(e);
  });Files.write(path,out);
 }
}"#,
    );
    run("javac", &["--release", "25", "Original.java"], &dir);
    run("java", &["Original"], &dir);
    assert_eq!(run("java", &["-Xverify:all", "Driver"], &dir), after);
    eprintln!(
        "actual eager source25 two original constructors retained; unchanged fu unit compiled/runtime API boundaries; rt ordinary closure uncompiled; total_s={:.3} artifacts={}",
        start.elapsed().as_secs_f64(),
        dir.display()
    );
}
