//! Retained original constructor, unchanged source subset, explicit peer APIs.
#![allow(clippy::unwrap_used)]
use dex_format::DexFile;
use std::{
    path::{Path, PathBuf},
    process::Command,
};
const APK: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
);
fn descriptor(d: &DexFile, index: u32) -> &str {
    d.string(d.type_id(index).unwrap().descriptor_idx).unwrap()
}
fn method(d: &DexFile, index: u32, owner: &str, name: &str, args: &[&str], ret: &str) {
    let m = d.method_id(index).unwrap();
    assert_eq!(descriptor(d, u32::from(m.class_idx)), owner);
    assert_eq!(d.string(m.name_idx).unwrap(), name);
    let p = d.proto_id(u32::from(m.proto_idx)).unwrap();
    assert_eq!(descriptor(d, p.return_type_idx), ret);
    assert_eq!(
        p.parameters
            .iter()
            .flat_map(|p| &p.types)
            .map(|t| descriptor(d, u32::from(*t)))
            .collect::<Vec<_>>(),
        args
    );
}
fn write(dir: &Path, path: &str, source: &str) {
    let file = dir.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, source).unwrap();
}
fn run(tool: &str, args: &[&str], dir: &Path) -> Vec<u8> {
    let home = std::env::var("DECOMPHIFY_JDK25_HOME")
        .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into());
    let out = Command::new(format!("{home}/bin/{tool}"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{tool}:{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
#[test]
#[ignore = "requires retained real Samsung single-class facade output and genuine JDK25"]
fn retained_44239_original_jvm_differential() {
    let root = std::env::var_os("DECOMPHIFY_CONSTRUCTOR_SUFFIX_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/opencode/decomphify-constructor-suffix-actual".into());
    let original = root.join("original-units/i1/l0$type.java");
    let bytes = std::fs::read(&original).unwrap();
    let unit = String::from_utf8(bytes.clone()).unwrap();
    let marker = "    // original method pool index 44239;";
    let start = unit.find(marker).unwrap();
    let tail = &unit[start..];
    assert!(
        tail.lines()
            .next()
            .unwrap()
            .contains("complete_body=true; semantic_complete=true; dependency_complete=true")
    );
    let end = tail
        .find("    // original method pool index 44240;")
        .unwrap();
    let ctor = &tail[..end];
    assert!(ctor.contains("\"root\""));
    assert_eq!(ctor.matches("i1.x0$type.I2").count(), 1);
    assert_eq!(ctor.matches("this.i =").count(), 1);
    let fields = unit[..start]
        .lines()
        .filter(|l| l.trim_start().starts_with("public ") && l.ends_with(';'))
        .collect::<Vec<_>>()
        .join("\n");
    let dir = root.join("suffix-runtime-44239");
    write(
        &dir,
        "i1/l0$type.java",
        &format!("package i1;public final class l0$type {{\n{fields}\n{ctor}}}\n"),
    );
    let inputs = dex_format::apk::parse_apk_path(APK).unwrap();
    let d = &inputs[0];
    for (index, owner, name, args, ret) in [
        (44239, "Li1/l0;", "<init>", vec!["Li1/b0;"], "V"),
        (
            48665,
            "Lkotlin/jvm/internal/n;",
            "f",
            vec!["Ljava/lang/Object;", "Ljava/lang/String;"],
            "V",
        ),
        (44170, "Li1/h;", "<init>", vec!["Z"], "V"),
        (44642, "Li1/x0$a;", "a", vec![], "Z"),
        (44607, "Li1/v0;", "<init>", vec![], "V"),
        (
            50602,
            "Ln0/e;",
            "<init>",
            vec!["[Ljava/lang/Object;", "I"],
            "V",
        ),
        (50611, "Ln0/e;", "i", vec![], "Ljava/util/List;"),
        (
            44160,
            "Li1/h0;",
            "<init>",
            vec!["Li1/b0;", "Li1/h;", "Ljava/util/List;"],
            "V",
        ),
    ] {
        method(d, index, owner, name, &args, ret);
    }
    let f = d.field_id(24360).unwrap();
    assert_eq!(descriptor(d, u32::from(f.class_idx)), "Li1/x0;");
    assert_eq!(d.string(f.name_idx).unwrap(), "I2");
    assert_eq!(descriptor(d, u32::from(f.type_idx)), "Li1/x0$a;");
    let cls = d
        .classes
        .iter()
        .find(|c| descriptor(d, c.class_idx) == "Li1/x0;")
        .unwrap();
    assert_eq!(
        cls.class_data
            .as_ref()
            .unwrap()
            .static_fields
            .iter()
            .find(|f| f.field_idx == 24360)
            .unwrap()
            .access_flags,
        0x19
    );
    let mut files = vec!["i1/l0$type.java"];
    for (path, src) in [
        (
            "evidence/Trace.java",
            r#"package evidence;public class Trace {public static int mode,calls;public static String log="";public static void mark(String s){log+=s;}}"#,
        ),
        (
            "kotlin/jvm/internal/n$type.java",
            r#"package kotlin.jvm.internal;public class n$type {public static void f(Object o,String s){evidence.Trace.mark("K"+s);if(o==null)throw new NullPointerException(s);}}"#,
        ),
        ("i1/b0$type.java", "package i1;public class b0$type {}"),
        (
            "i1/x0$a.java",
            r#"package i1;public class x0$a {public boolean a(){evidence.Trace.mark("F");if(++evidence.Trace.calls==2&&evidence.Trace.mode==2)throw new IllegalArgumentException();return evidence.Trace.mode!=1;}}"#,
        ),
        (
            "i1/x0$type.java",
            r#"package i1;public class x0$type {static{evidence.Trace.mark("X");}public static final x0$a I2=new x0$a();}"#,
        ),
        ("i1/x0$b.java", "package i1;public class x0$b {}"),
        ("i1/l0$a.java", "package i1;public class l0$a {}"),
        (
            "i1/h$type.java",
            r#"package i1;public class h$type {static{evidence.Trace.mark("O");if(evidence.Trace.mode==6)throw new OutOfMemoryError("fixture class initialization failure");}public final boolean flag;public h$type(boolean flag){evidence.Trace.mark("C");this.flag=flag;}}"#,
        ),
        (
            "i1/v0$type.java",
            r#"package i1;public class v0$type {public v0$type(){evidence.Trace.mark("V");}}"#,
        ),
        (
            "n0/e$type.java",
            r#"package n0;public class e$type {static{evidence.Trace.mark("E");}public final Object[] storage;public e$type(Object[] a,int n){evidence.Trace.mark("Q");if(a.length!=16||n!=0)throw new AssertionError();storage=a;}public java.util.List i(){evidence.Trace.mark("L");if(evidence.Trace.mode==3)throw new IllegalStateException();return java.util.Collections.emptyList();}}"#,
        ),
        (
            "i1/h0$type.java",
            r#"package i1;public class h0$type {static{evidence.Trace.mark("H");}public final b0$type root;public final h$type value;public h0$type(b0$type root,h$type value,java.util.List list){evidence.Trace.mark("R");if(evidence.Trace.mode==4)throw new UnsupportedOperationException();if(list==null||!list.isEmpty())throw new AssertionError();this.root=root;this.value=value;}}"#,
        ),
        ("z1/b$type.java", "package z1;public class b$type {}"),
        (
            "Driver.java",
            r#"public class Driver {public static void main(String[]args){evidence.Trace.mode=Integer.parseInt(args[0]);i1.b0$type root=evidence.Trace.mode==5?null:new i1.b0$type();try{i1.l0$type o=new i1.l0$type(root);if(o.a!=root||o.b==null||o.d==null||o.e==o.g||o.e.storage.length!=16||o.g.storage.length!=16||o.f!=1L||o.c||o.h!=null)throw new AssertionError("stores");if(evidence.Trace.mode==1){if(o.i!=null)throw new AssertionError("false branch");}else if(o.i==null||o.i.root!=root||o.i.value!=o.b)throw new AssertionError("Phi object identity");System.out.println(evidence.Trace.log+":ok");}catch(Throwable t){if(t instanceof AssertionError)throw t;System.out.println(evidence.Trace.log+":"+t.getClass().getSimpleName());}}}"#,
        ),
    ] {
        write(&dir, path, src);
        files.push(path);
    }
    let mut args = vec!["--release", "25"];
    args.extend(files);
    run("javac", &args, &dir);
    let mut after = Vec::new();
    for mode in ["0", "1", "2", "3", "4", "5", "6"] {
        after.extend(run("java", &["-Xverify:all", "Driver", mode], &dir));
    }
    write(
        &dir,
        "Original.java",
        r#"import java.nio.file.*;import java.lang.classfile.*;import java.lang.constant.*;
public class Original {public static void main(String[]args)throws Exception{ClassFile cf=ClassFile.of();ClassDesc self=ClassDesc.of("i1.l0$type"),b0=ClassDesc.of("i1.b0$type"),h=ClassDesc.of("i1.h$type"),v0=ClassDesc.of("i1.v0$type"),x0=ClassDesc.of("i1.x0$type"),flag=ClassDesc.of("i1.x0$a"),e=ClassDesc.of("n0.e$type"),h0=ClassDesc.of("i1.h0$type");Path path=Path.of("i1/l0$type.class");Files.write(path,cf.transformClass(cf.parse(path),(cb,element)->{if(element instanceof MethodModel mm&&mm.methodName().equalsString("<init>"))cb.transformMethod(mm,(mb,me)->{if(me instanceof CodeModel)mb.withCode(c->{Label no=c.newLabel(),join=c.newLabel();
 c.ldc("root").astore(14);c.aload(1).aload(14).invokestatic(ClassDesc.of("kotlin.jvm.internal.n$type"),"f",MethodTypeDesc.ofDescriptor("(Ljava/lang/Object;Ljava/lang/String;)V"));c.aload(0).invokespecial(ConstantDescs.CD_Object,"<init>",MethodTypeDesc.ofDescriptor("()V"));c.aload(0).aload(1).putfield(self,"a",b0);
 c.new_(h).astore(2);c.getstatic(x0,"I2",flag).astore(3);c.aload(3).invokevirtual(flag,"a",MethodTypeDesc.ofDescriptor("()Z")).istore(4);c.aload(2).iload(4).invokespecial(h,"<init>",MethodTypeDesc.ofDescriptor("(Z)V"));c.aload(0).aload(2).putfield(self,"b",h);
 c.new_(v0).dup().invokespecial(v0,"<init>",MethodTypeDesc.ofDescriptor("()V")).astore(5);c.aload(0).aload(5).putfield(self,"d",v0);
 c.new_(e).astore(6);c.bipush(16).istore(7);c.iload(7).anewarray(ClassDesc.of("i1.x0$b")).astore(15);c.iconst_0().istore(8);c.aload(6).aload(15).iload(8).invokespecial(e,"<init>",MethodTypeDesc.ofDescriptor("([Ljava/lang/Object;I)V"));c.aload(0).aload(6).putfield(self,"e",e);c.lconst_1().lstore(9);c.aload(0).lload(9).putfield(self,"f",ConstantDescs.CD_long);
 c.new_(e).astore(11);c.iload(7).anewarray(ClassDesc.of("i1.l0$a")).astore(15);c.aload(11).aload(15).iload(8).invokespecial(e,"<init>",MethodTypeDesc.ofDescriptor("([Ljava/lang/Object;I)V"));c.aload(0).aload(11).putfield(self,"g",e);
 c.aload(3).invokevirtual(flag,"a",MethodTypeDesc.ofDescriptor("()Z")).ifeq(no);c.new_(h0).astore(12);c.aload(11).invokevirtual(e,"i",MethodTypeDesc.ofDescriptor("()Ljava/util/List;")).astore(13);c.aload(12).aload(1).aload(2).aload(13).invokespecial(h0,"<init>",MethodTypeDesc.ofDescriptor("(Li1/b0$type;Li1/h$type;Ljava/util/List;)V"));c.goto_(join);c.labelBinding(no).aconst_null().astore(12);c.labelBinding(join).aload(0).aload(12).putfield(self,"i",h0).return_();
 });else mb.with(me);});else cb.with(element);}));}}"#,
    );
    run("javac", &["--release", "25", "Original.java"], &dir);
    run("java", &["Original"], &dir);
    let mut before = Vec::new();
    for mode in ["0", "1", "2", "3", "4", "5", "6"] {
        before.extend(run("java", &["-Xverify:all", "Driver", mode], &dir));
    }
    assert_eq!(before, after);
    let traces = String::from_utf8(after).unwrap();
    assert_eq!(
        traces,
        "KrootOXFCVEQQFHLR:ok\nKrootOXFCVEQQF:ok\nKrootOXFCVEQQF:IllegalArgumentException\nKrootOXFCVEQQFHL:IllegalStateException\nKrootOXFCVEQQFHLR:UnsupportedOperationException\nKroot:NullPointerException\nKrootO:OutOfMemoryError\n"
    );
    write(&dir, "runtime-traces.txt", &traces);
    write(
        &dir,
        "scope.txt",
        "Compiled unchanged generated constructor and unchanged own-field declarations with exact DEX-attested peer signatures and instrumented replacement bodies. Compared all emitted results with explicit original instruction-order JVM constructor under -Xverify:all. Complete unchanged unit retained; its large ordinary-method peer closure is not compiled here. OOM is a deliberately thrown class initialization boundary fixture, not actual heap exhaustion or APK/device behavior.\n",
    );
    assert_eq!(std::fs::read(original).unwrap(), bytes);
    eprintln!(
        "Original Samsung Li1/l0; ctor44239 unchanged subset compile and original-instruction JVM differential modes7 passed at {}\n{traces}",
        dir.display()
    );
}
