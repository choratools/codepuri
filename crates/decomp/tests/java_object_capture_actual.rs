//! Selected original Samsung object arguments through the seeded production
//! facade. Compilation fixtures are explicit API boundaries, not APK bodies.
#![allow(clippy::unwrap_used)]

use decomp::{
    dex_java::{JavaOptions, emit_java},
    output::GeneratedSource,
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
const L0_WORDS: &[u16] = &[
    0x001a, 0x9546, 0x2071, 0xbe19, 0x0009, 0x1070, 0xb650, 0x0008, 0x895b, 0x5ea4, 0x0022, 0x1a22,
    0x0162, 0x5f28, 0x106e, 0xae62, 0x0001, 0x020a, 0x2070, 0xac8a, 0x0020, 0x805b, 0x5ea5, 0x0222,
    0x1a57, 0x1070, 0xae3f, 0x0002, 0x825b, 0x5ea7, 0x0222, 0x1fa5, 0x0313, 0x0010, 0x3423, 0x2cdf,
    0x0512, 0x3070, 0xc5aa, 0x0542, 0x825b, 0x5ea8, 0x0616, 0x0001, 0x865a, 0x5ea9, 0x0222, 0x1fa5,
    0x3323, 0x2cde, 0x3070, 0xc5aa, 0x0532, 0x825b, 0x5eaa, 0x106e, 0xae62, 0x0001, 0x010a, 0x0138,
    0x000c, 0x0122, 0x1a20, 0x106e, 0xc5b3, 0x0002, 0x020c, 0x4070, 0xac80, 0x2091, 0x0228, 0x0112,
    0x815b, 0x5eac, 0x000e,
];
const H_WORDS: &[u16] = &[
    0x001a, 0x7089, 0x2071, 0xbe19, 0x0003, 0x001a, 0x724d, 0x2071, 0xbe19, 0x0004, 0x1070, 0xb650,
    0x0002, 0x0022, 0x1fbd, 0x1070, 0xc600, 0x0000, 0x205b, 0x6794, 0x0022, 0x1c1c, 0x1070, 0xb6ea,
    0x0000, 0x205b, 0x6795, 0x0062, 0x9561, 0x0013, 0x001e, 0x0162, 0x956b, 0x2071, 0xfb7b, 0x0010,
    0x000b, 0x205a, 0x6797, 0x2012, 0x2059, 0x6798, 0x235b, 0x6791, 0x0022, 0x1fde, 0x0122, 0x1fc2,
    0x3070, 0xc609, 0x0431, 0x1312, 0x4070, 0xc68a, 0x5130, 0x205b, 0x6792, 0x205b, 0x6793, 0x000e,
];

fn descriptor(dex: &DexFile, index: u32) -> &str {
    dex.string(dex.type_id(index).unwrap().descriptor_idx)
        .unwrap()
}
fn write(root: &Path, relative: &Path, text: &str) -> PathBuf {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}
fn artifact_dir() -> PathBuf {
    std::env::var_os("DECOMPHIFY_OBJECT_CAPTURE_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!(
                "decomphify-object-capture-actual-{}",
                std::process::id()
            ))
        })
}
fn original_constructor(source: &str, index: u32) -> &str {
    let marker = format!("    // original method pool index {index};");
    let tail = &source[source.find(&marker).unwrap_or_else(|| {
        panic!(
            "original {index} unavailable: {}",
            source
                .lines()
                .filter(|line| line.contains(&index.to_string()))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })..];
    let identity = tail.lines().next().unwrap();
    assert!(
        identity.contains("name: \"<init>\"")
            && identity.contains("complete_body=true")
            && identity.contains("semantic_complete=true")
            && identity.contains("dependency_complete=true")
    );
    let start = tail.find('\n').unwrap() + 1;
    let open = tail[start..].find('{').unwrap() + start;
    let mut depth = 0;
    for (offset, ch) in tail[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &tail[start..open + offset + 1];
                }
            }
            _ => {}
        }
    }
    panic!("unclosed constructor {index}")
}

#[test]
#[ignore = "bounded retained original seeded provenance; no facade export"]
fn original_seeded_shared_zero_provenance() {
    use backend_java::{DeclarationBindings, JavaResolver};
    use decomp::{
        dex_java::DexJavaResolver,
        java_initial_parameters::{InitialParameter, InitialProvenance},
    };
    use dex_decompiler::{DexPasses, DexTypes, infer::TypeContext};
    use managed_ir::refs::MethodRef;
    let inputs = dex_format::apk::parse_apk_path(APK).unwrap();
    let dex = &inputs[0];
    let raw = dex.method_id(44239).unwrap();
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == "Li1/l0;")
        .unwrap();
    let method = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|m| m.method_idx == 44239)
        .unwrap();
    let code = method.code.as_ref().unwrap();
    let ctx = DexTypes::new(dex, Some(44239));
    let layout = decomp::java_incoming_registers::IncomingLayout::from_proto(
        dex,
        dex.proto_id(u32::from(raw.proto_idx)).unwrap(),
        false,
        code.registers_size,
        code.ins_size,
    )
    .unwrap();
    let mut seeds = std::collections::HashMap::new();
    seeds.insert(
        (layout.receiver.unwrap().register, false),
        ctx.type_at(u32::from(raw.class_idx)).unwrap(),
    );
    let mut types = vec![];
    assert!(ctx.method_params(44239, &mut types));
    for (slot, ty) in layout.parameters.iter().zip(types) {
        seeds.insert((slot.register, slot.high_register.is_some()), ty);
    }
    let ssa = DexPasses::run_ssa_with(code, &ctx.with_initial_registers(seeds)).unwrap();
    let resolver = DexJavaResolver { dex };
    let signature = resolver.method(MethodRef::new(44239)).unwrap();
    let provenance = ssa
        .initial_registers()
        .iter()
        .map(|(value, registers)| InitialProvenance {
            value: *value,
            registers: registers.clone(),
        })
        .collect::<Vec<_>>();
    let plan = decomp::java_initial_parameters::parameter_bindings_with_provenance(
        ssa.ir(),
        &signature,
        code.registers_size,
        code.ins_size,
        &provenance,
    )
    .unwrap();
    let bound = |value| match value {
        InitialParameter::Bound(value) => Some(value),
        InitialParameter::Unused { .. } => None,
    };
    let bindings = DeclarationBindings {
        receiver: plan.receiver.and_then(bound),
        parameters: plan.parameters.into_iter().map(bound).collect(),
        names: vec![],
    };
    assert!(bindings.receiver.is_some());
    assert_eq!(bindings.parameters.len(), 1);
    assert!(bindings.parameters[0].is_some());
    for (id, insn) in ssa.ir().instructions() {
        eprintln!("seeded {id:?} off={:?} {insn:?}", ssa.offset(id));
    }
    let at = |offset| {
        ssa.ir()
            .instructions()
            .iter()
            .find(|(id, _)| ssa.offset(*id) == Some(offset))
            .unwrap()
            .1
    };
    let managed_ir::insn::ManagedInsn::Const {
        result: zero,
        value: managed_ir::ops::Constant::Zero,
    } = at(36)
    else {
        panic!("zero definition changed")
    };
    for (offset, reference) in [(37, 50602), (50, 50602)] {
        let managed_ir::insn::ManagedInsn::Invoke { method, args, .. } = at(offset) else {
            panic!("initializer changed")
        };
        assert_eq!(method.index(), reference);
        assert_eq!(args[2], *zero);
    }
    eprintln!(
        "original seeded zero36 has the two genuine init37/init50 consumers; fixed metadata and incoming binding proven"
    );
}

#[test]
#[ignore = "requires retained Samsung APK and four genuine JDK17/Android35 metadata paths; two bounded class exports"]
fn original_samsung_object_arguments_seeded_facade() {
    let inputs = dex_format::apk::parse_apk_path(APK).unwrap();
    let root = artifact_dir();
    let classpath: Vec<PathBuf> = [
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod",
        "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/jdk.unsupported.jmod",
        "/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.naming.jmod",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect();
    for path in &classpath {
        assert!(
            path.is_file(),
            "missing genuine metadata {}",
            path.display()
        );
    }
    let hashes = Command::new("sha256sum")
        .arg(APK)
        .args(&classpath)
        .output()
        .unwrap();
    assert!(hashes.status.success());
    write(
        &root,
        Path::new("input-metadata.sha256"),
        &String::from_utf8(hashes.stdout).unwrap(),
    );
    let mut units = Vec::new();
    let selected_owner = std::env::var("DECOMPHIFY_OBJECT_CAPTURE_OWNER").ok();
    if let Some(owner) = &selected_owner {
        assert!(["Li1/l0;", "Ln5/h;"].contains(&owner.as_str()));
    }
    for (owner, index, words, registers, incoming, parameters) in [
        ("Li1/l0;", 44239, L0_WORDS, 10, 2, vec!["Li1/b0;"]),
        (
            "Ln5/h;",
            50711,
            H_WORDS,
            6,
            4,
            vec!["Ls5/d;", "Ljava/lang/String;", "I"],
        ),
    ]
    .into_iter()
    .filter(|(owner, ..)| {
        selected_owner
            .as_ref()
            .is_none_or(|selected| selected == owner)
    }) {
        let (pool, dex, class) = inputs
            .iter()
            .enumerate()
            .find_map(|(pool, dex)| {
                dex.classes
                    .iter()
                    .find(|c| descriptor(dex, c.class_idx) == owner)
                    .map(|c| (pool, dex, c))
            })
            .unwrap();
        assert_eq!(pool, 0);
        let method = class
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods
            .iter()
            .find(|m| m.method_idx == index)
            .unwrap();
        assert_eq!(method.access_flags, 0x10001);
        let raw = dex.method_id(index).unwrap();
        assert_eq!(descriptor(dex, u32::from(raw.class_idx)), owner);
        assert_eq!(dex.string(raw.name_idx).unwrap(), "<init>");
        let proto = dex.proto_id(u32::from(raw.proto_idx)).unwrap();
        assert_eq!(descriptor(dex, proto.return_type_idx), "V");
        assert_eq!(
            proto
                .parameters
                .as_ref()
                .unwrap()
                .types
                .iter()
                .map(|t| descriptor(dex, u32::from(*t)))
                .collect::<Vec<_>>(),
            parameters
        );
        let code = method.code.as_ref().unwrap();
        assert_eq!(code.registers_size, registers);
        assert_eq!(code.ins_size, incoming);
        assert!(code.tries.is_empty());
        assert_eq!(
            code.insns
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
            words
        );
        // emit_java owns IncomingLayout/DexTypes initial seeds and source-throws
        // seeds. There is no unseeded probe or synthetic replacement method.
        let options = JavaOptions {
            source_release: 25,
            class_filter: Some(owner.into()),
            workers: 1,
            classpath: classpath.clone(),
            ..JavaOptions::default()
        };
        let start = Instant::now();
        let mut selected = Vec::new();
        let report = emit_java(&inputs, &options, &mut |unit: &GeneratedSource| {
            selected.push(unit.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(selected.len(), 1, "{report:?}");
        let unit = selected.remove(0);
        assert_eq!(unit.provenance.descriptor, owner);
        let path = write(
            &root.join("original-units"),
            &unit.relative_path,
            &unit.text,
        );
        write(
            &root,
            Path::new(&format!("report-{index}.txt")),
            &format!("{report:#?}\n"),
        );
        eprintln!(
            "original capture pool={pool} owner={owner} index={index} raw_words={} elapsed_s={:.3} complete_ctors={} unchanged_source={}",
            words.len(),
            start.elapsed().as_secs_f64(),
            report.complete_constructors,
            path.display()
        );
        units.push((index, unit));
    }
    for (index, unit) in &units {
        let body = original_constructor(&unit.text, *index);
        if *index == 44239 {
            let first = body;
            assert_eq!(first.matches("i1.x0$type.I2").count(), 1, "{first}");
            let allocation = first.find("new i1.h$type(").unwrap();
            let field = first.find("i1.x0$type.I2").unwrap();
            assert!(
                allocation < field,
                "static read moved before allocation: {first}"
            );
            assert!(
                first[field..].contains(".a()"),
                "missing original virtual producer: {first}"
            );
        } else {
            let second = body;
            assert_eq!(second.matches("new n5.o$type(").count(), 1, "{second}");
            assert_eq!(second.matches("new n5.g$type(").count(), 1, "{second}");
            assert!(
                second.find("new n5.o$type(").unwrap() < second.find("new n5.g$type(").unwrap()
            );
            assert!(
                second.contains("(int) p2"),
                "incoming third int argument must be seeded: {second}"
            );
        }
    }
    eprintln!(
        "selected original identities accepted; retained entire units in {}",
        root.display()
    );
}

fn jdk25() -> PathBuf {
    std::env::var_os("DECOMPHIFY_JDK25_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into())
}
fn java_tool(tool: &str, args: &[&str], dir: &Path) -> Vec<u8> {
    let result = Command::new(jdk25().join("bin").join(tool))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{tool}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}
fn constructor_subset(unit: &str, index: u32, package: &str, name: &str) -> String {
    let fields = unit
        .lines()
        .take_while(|line| !line.contains("original method pool index"))
        .filter(|line| line.trim_start().starts_with("public ") && line.ends_with(';'))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "// Exact emitted constructor and field declarations; peer APIs below are test boundaries.\npackage {package};\npublic final class {name} {{\n{fields}\n{}\n}}\n",
        original_constructor(unit, index)
    )
}
fn attest_method(
    dex: &DexFile,
    index: u32,
    owner: &str,
    name: &str,
    parameters: &[&str],
    return_type: &str,
) {
    let raw = dex.method_id(index).unwrap();
    assert_eq!(descriptor(dex, u32::from(raw.class_idx)), owner);
    assert_eq!(dex.string(raw.name_idx).unwrap(), name);
    let proto = dex.proto_id(u32::from(raw.proto_idx)).unwrap();
    assert_eq!(descriptor(dex, proto.return_type_idx), return_type);
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

fn attest_field(dex: &DexFile, index: u32, owner: &str, name: &str, ty: &str, flags: u32) {
    let raw = dex.field_id(index).unwrap();
    assert_eq!(descriptor(dex, u32::from(raw.class_idx)), owner);
    assert_eq!(dex.string(raw.name_idx).unwrap(), name);
    assert_eq!(descriptor(dex, u32::from(raw.type_idx)), ty);
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == owner)
        .unwrap();
    let field = class
        .class_data
        .as_ref()
        .unwrap()
        .static_fields
        .iter()
        .find(|f| f.field_idx == index)
        .unwrap();
    assert_eq!(field.access_flags, flags);
}

#[test]
#[ignore = "requires retained seeded facade artifacts and JDK25; exact constructor subsets against attested API boundaries"]
fn retained_original_constructor_subsets_compile_and_runtime() {
    let root = artifact_dir();
    let inputs = dex_format::apk::parse_apk_path(APK).unwrap();
    let dex = &inputs[0];
    let selected_owner = std::env::var("DECOMPHIFY_OBJECT_CAPTURE_OWNER").ok();
    for (owner, index, relative, package, name) in [
        ("Li1/l0;", 44239, "i1/l0$type.java", "i1", "l0$type"),
        ("Ln5/h;", 50711, "n5/h$type.java", "n5", "h$type"),
    ]
    .into_iter()
    .filter(|(owner, ..)| {
        selected_owner
            .as_ref()
            .is_none_or(|selected| selected == owner)
    }) {
        let original = root.join("original-units").join(relative);
        let before = std::fs::read(&original).unwrap();
        let unit = String::from_utf8(before.clone()).unwrap();
        let dir = root.join(format!("constructor-subset-{index}"));
        let subset = constructor_subset(&unit, index, package, name);
        write(&dir, Path::new(relative), &subset);
        let whole = Command::new(jdk25().join("bin/javac"))
            .args([
                "--release",
                "25",
                "-Xmaxerrs",
                "8",
                "-classpath",
                "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar",
                "-d",
            ])
            .arg(dir.join("whole-unit-classes"))
            .arg(&original)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            !whole.status.success(),
            "unexpected complete dependency closure; update this bounded subset harness"
        );
        let whole_errors = String::from_utf8(whole.stderr).unwrap();
        assert!(
            whole_errors.contains("does not exist") || whole_errors.contains("cannot find symbol"),
            "whole unit failure lacks missing dependency evidence: {whole_errors}"
        );
        write(&dir, Path::new("whole-unit-javac.stderr"), &whole_errors);
        let mut files = vec![relative.to_owned()];
        let mut fixture = |path: &str, body: &str| {
            write(&dir, Path::new(path), body);
            files.push(path.to_owned());
        };
        fixture(
            "evidence/Trace.java",
            "package evidence;public class Trace {public static String log=\"\";public static int mode;public static void mark(String s){log+=s;}}",
        );
        attest_method(
            dex,
            48665,
            "Lkotlin/jvm/internal/n;",
            "f",
            &["Ljava/lang/Object;", "Ljava/lang/String;"],
            "V",
        );
        if index == 50711 {
            attest_field(dex, 38241, "Lyt/a;", "b", "Lyt/a$a;", 0x19);
            attest_field(dex, 38251, "Lyt/d;", "e", "Lyt/d;", 0x4019);
            for (reference, owner, params, ret) in [
                (50688, "Ln5/b;", vec![], "V"),
                (50697, "Ln5/g;", vec!["Ls5/d;", "Ljava/lang/String;"], "V"),
                (50826, "Ln5/o;", vec!["I", "Lxq/a;", "I"], "V"),
                (64379, "Lyt/c;", vec!["I", "Lyt/d;"], "J"),
            ] {
                attest_method(
                    dex,
                    reference,
                    owner,
                    if reference == 64379 { "o" } else { "<init>" },
                    &params,
                    ret,
                );
            }
            for (path, body) in [
                (
                    "kotlin/jvm/internal/n$type.java",
                    "package kotlin.jvm.internal;public class n$type {public static void f(Object v,String n){evidence.Trace.mark(\"K\"+n);if(v==null)throw new NullPointerException(n);}}",
                ),
                ("s5/d$type.java", "package s5;public class d$type {}"),
                ("xq/a$type.java", "package xq;public interface a$type {}"),
                (
                    "n5/b$type.java",
                    "package n5;public class b$type {public b$type(){evidence.Trace.mark(\"B\");}}",
                ),
                ("yt/a$a.java", "package yt;public class a$a {}"),
                (
                    "yt/a$type.java",
                    "package yt;public class a$type {static{evidence.Trace.mark(\"A\");}public static final a$a b=new a$a();}",
                ),
                (
                    "yt/d$type.java",
                    "package yt;public enum d$type {e;static{evidence.Trace.mark(\"D\");}}",
                ),
                (
                    "yt/c$type.java",
                    "package yt;public class c$type {public static long o(int n,d$type d){evidence.Trace.mark(\"T\");if(n!=30||d!=d$type.e)throw new AssertionError();return n;}}",
                ),
                (
                    "n5/g$type.java",
                    "package n5;public class g$type implements xq.a$type {static{evidence.Trace.mark(\"G\");}public final s5.d$type driver;public final String name;public g$type(s5.d$type d,String n){evidence.Trace.mark(\"i\");if(evidence.Trace.mode==1)throw new IllegalArgumentException();driver=d;name=n;}}",
                ),
                (
                    "n5/o$type.java",
                    "package n5;public class o$type {static{evidence.Trace.mark(\"O\");if(evidence.Trace.mode==3)throw new OutOfMemoryError();}public final int n;public final n5.g$type source;public o$type(int count,xq.a$type source,int n){evidence.Trace.mark(\"C\");if(evidence.Trace.mode==2)throw new IllegalStateException();if(count!=1)throw new AssertionError();this.source=(n5.g$type)source;this.n=n;}}",
                ),
                (
                    "Driver.java",
                    "public class Driver {public static void main(String[] a){evidence.Trace.mode=Integer.parseInt(a[0]);s5.d$type d=new s5.d$type();String prefix=\"KdriverKfileNameBADT\";if(evidence.Trace.mode==0){n5.h$type h=new n5.h$type(d,\"file\",17);if(h.a!=d||h.b!=h.c||h.b.source.driver!=d||!h.b.source.name.equals(\"file\")||h.b.n!=17||h.g!=30||h.h!=2||h.d==null||h.e==null)throw new AssertionError(\"original fields/aliases\");if(!evidence.Trace.log.equals(prefix+\"OGiC\"))throw new AssertionError(evidence.Trace.log);}else{try{new n5.h$type(d,\"file\",17);throw new AssertionError();}catch(IllegalArgumentException|IllegalStateException|OutOfMemoryError expected){}String suffix=evidence.Trace.mode==1?\"OGi\":evidence.Trace.mode==2?\"OGiC\":\"O\";if(!evidence.Trace.log.equals(prefix+suffix))throw new AssertionError(evidence.Trace.log);}System.out.println(evidence.Trace.log);}}",
                ),
            ] {
                fixture(path, body);
            }
        } else {
            attest_field(dex, 24360, "Li1/x0;", "I2", "Li1/x0$a;", 0x19);
            for (reference, owner, name, params, ret) in [
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
                attest_method(dex, reference, owner, name, &params, ret);
            }
            for (path, body) in [
                (
                    "kotlin/jvm/internal/n$type.java",
                    "package kotlin.jvm.internal;public class n$type {public static void f(Object v,String n){evidence.Trace.mark(\"K\"+n);if(v==null)throw new NullPointerException(n);}}",
                ),
                ("i1/b0$type.java", "package i1;public class b0$type {}"),
                (
                    "i1/x0$a.java",
                    "package i1;public class x0$a {private final boolean value;public x0$a(boolean value){this.value=value;}public boolean a(){evidence.Trace.mark(\"F\");if(evidence.Trace.mode==1)throw new IllegalArgumentException();return value;}}",
                ),
                (
                    "i1/x0$type.java",
                    "package i1;public class x0$type {static{evidence.Trace.mark(\"X\");}public static final x0$a I2=new x0$a(true);}",
                ),
                ("i1/x0$b.java", "package i1;public class x0$b {}"),
                ("i1/l0$a.java", "package i1;public class l0$a {}"),
                (
                    "i1/h$type.java",
                    "package i1;public class h$type {static{evidence.Trace.mark(\"O\");if(evidence.Trace.mode==3)throw new OutOfMemoryError();}public h$type(boolean value){evidence.Trace.mark(\"C\");if(evidence.Trace.mode==2)throw new IllegalStateException();if(!value)throw new AssertionError();}}",
                ),
                (
                    "i1/v0$type.java",
                    "package i1;public class v0$type {public v0$type(){evidence.Trace.mark(\"V\");}}",
                ),
                (
                    "n0/e$type.java",
                    "package n0;public class e$type {static{evidence.Trace.mark(\"E\");}public final Object[] storage;public e$type(Object[] storage,int z){evidence.Trace.mark(\"Q\");if(storage.length!=16||z!=0)throw new AssertionError();this.storage=storage;}public java.util.List i(){evidence.Trace.mark(\"L\");return java.util.Collections.emptyList();}}",
                ),
                (
                    "i1/h0$type.java",
                    "package i1;public class h0$type {static{evidence.Trace.mark(\"H\");}public final b0$type layout;public final h$type value;public h0$type(b0$type layout,h$type value,java.util.List list){evidence.Trace.mark(\"R\");if(list==null||!list.isEmpty())throw new AssertionError();this.layout=layout;this.value=value;}}",
                ),
                ("z1/b$type.java", "package z1;public class b$type {}"),
                (
                    "Driver.java",
                    "public class Driver {public static void main(String[] a){evidence.Trace.mode=Integer.parseInt(a[0]);i1.b0$type layout=new i1.b0$type();String prefix=\"Kroot\";if(evidence.Trace.mode==0){i1.l0$type h=new i1.l0$type(layout);if(h.a!=layout||h.b==null||h.d==null||h.e.storage.length!=16||h.g.storage.length!=16||h.e==h.g||h.f!=1L||h.i==null||h.i.layout!=layout||h.i.value!=h.b)throw new AssertionError(\"original stores/capture identity\");if(!evidence.Trace.log.equals(prefix+\"OXFCVEQQFHLR\"))throw new AssertionError(evidence.Trace.log);}else{try{new i1.l0$type(layout);throw new AssertionError();}catch(IllegalArgumentException|IllegalStateException|OutOfMemoryError expected){}String suffix=evidence.Trace.mode==1?\"OXF\":evidence.Trace.mode==2?\"OXFC\":\"O\";if(!evidence.Trace.log.equals(prefix+suffix))throw new AssertionError(evidence.Trace.log);}System.out.println(evidence.Trace.log);}}",
                ),
            ] {
                fixture(path, body);
            }
        }
        let mut args = vec!["--release", "25"];
        args.extend(files.iter().map(String::as_str));
        java_tool("javac", &args, &dir);
        let mut traces = String::new();
        for mode in ["0", "1", "2", "3"] {
            let out = java_tool("java", &["-Xverify:all", "Driver", mode], &dir);
            traces.push_str(&format!("mode={mode}: {}", String::from_utf8(out).unwrap()));
        }
        write(&dir, Path::new("runtime-traces.txt"), &traces);
        write(
            &dir,
            Path::new("scope.txt"),
            "Compiled the exact emitted constructor and unchanged emitted own-field declarations against attested API signatures with instrumented replacement dependency bodies. The complete original emitted unit is retained unchanged; javac against genuine JDK25/Android35 failed with missing peer dependency diagnostics retained in whole-unit-javac.stderr (maxerrs8, not an exhaustive diagnostic count). These fixture executions are not real APK/framework runtime verification or actual heap exhaustion.\n",
        );
        assert_eq!(
            std::fs::read(&original).unwrap(),
            before,
            "retained original unit mutated"
        );
        eprintln!(
            "original {owner} method{index} unchanged constructor subset javac25/runtime modes4 passed; complete unit retained uncompiled at {}\n{traces}",
            original.display()
        );
    }
}
