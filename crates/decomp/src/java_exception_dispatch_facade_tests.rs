//! Production method-source routing retains strict metadata and old output.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use dex_format::{
    RawSpan,
    class::ClassData,
    code::{CodeItem, EncodedCatchHandler, TryItem, TypeAddrPair},
    ids::{MethodId, ProtoId, TypeId, TypeList},
    strings::StringData,
};

fn jdk() -> std::path::PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into())
}
pub(super) fn classpath() -> crate::java_external_metadata::Classpath {
    crate::java_external_metadata::Classpath::from_paths(&[jdk().join("jmods/java.base.jmod")])
        .unwrap()
}
pub(super) fn install_jdk(r: &mut Resolver<'_>, external: &impl ExternalMetadata) {
    for d in external.hierarchy() {
        let original = format!("L{};", d.name.replace('.', "/"));
        let name = r
            .aliases
            .class_name(&original)
            .unwrap_or(&d.name)
            .to_owned();
        if let Some(access_flags) = external.class_access(&original) {
            r.class_catalog.insert(
                name.clone(),
                Some(backend_java::constructors::ConstructorClass {
                    name,
                    superclass: d.superclass.clone(),
                    access_flags,
                }),
            );
        }
        r.hierarchy.insert(d).unwrap();
    }
}
pub(super) fn raw_dex() -> DexFile {
    let mut dex = DexFile::from_bytes(&super::index_tests::fixture::minimal_dex()).unwrap();
    let span = RawSpan { offset: 0, size: 0 };
    for text in ["V", "dispatch", "I", "Ljava/lang/ArithmeticException;"] {
        dex.strings.push(StringData {
            value: text.into(),
            utf16_size: text.len() as u32,
            span,
        });
    }
    for descriptor_idx in [1, 3, 4] {
        dex.type_ids.push(TypeId {
            descriptor_idx,
            span,
        });
    }
    dex.proto_ids.push(ProtoId {
        shorty_idx: 3,
        return_type_idx: 2,
        parameters_off: 1,
        parameters: Some(TypeList {
            types: vec![2],
            span,
        }),
        span,
    });
    dex.method_ids.push(MethodId {
        class_idx: 0,
        proto_idx: 0,
        name_idx: 2,
        span,
    });
    let units = [
        0x0012u16, 0x1112, 0x0293, 0x0301, 0x13b1, 0x033c, 0xfffd, 0x020f, 0x000d, 0xf212, 0x020f,
    ];
    dex.classes[0].class_data = Some(ClassData {
        static_fields: vec![],
        instance_fields: vec![],
        virtual_methods: vec![],
        direct_methods: vec![EncodedMethod {
            method_idx: 0,
            access_flags: 9,
            code_off: 1,
            code: Some(CodeItem {
                registers_size: 4,
                ins_size: 1,
                outs_size: 0,
                debug_info_off: 0,
                insns_size: units.len() as u32,
                insns: units.into_iter().flat_map(u16::to_le_bytes).collect(),
                tries: vec![TryItem {
                    start_addr: 2,
                    insn_count: 5,
                    handler_off: 1,
                    span,
                }],
                handlers: vec![EncodedCatchHandler {
                    relative_offset: 1,
                    handlers: vec![TypeAddrPair {
                        type_idx: 3,
                        address: 8,
                    }],
                    catch_all_addr: None,
                    span,
                }],
                span,
            }),
        }],
        span,
    });
    dex
}
pub(super) fn compile_run(label: &str, declaration: &str, body: &str) {
    let dir = std::env::temp_dir().join(format!(
        "decomphify-facade-dispatch-{label}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = format!(
        "public class FacadeDispatch {{\n{declaration}\npublic static void main(String[] args) {{ {body} }}\n}}"
    );
    std::fs::write(dir.join("FacadeDispatch.java"), &source).unwrap();
    let compile = std::process::Command::new(jdk().join("bin/javac"))
        .arg("--release")
        .arg("17")
        .arg("FacadeDispatch.java")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}\n{source}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = std::process::Command::new(jdk().join("bin/java"))
        .arg("-cp")
        .arg(&dir)
        .arg("FacadeDispatch")
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "facade-dispatch-ok");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn production_backend_rejection_falls_back_without_changing_encoded_catch_priority() {
    let mut dex = raw_dex();
    let span = RawSpan { offset: 0, size: 0 };
    let name = "Ljava/lang/RuntimeException;";
    dex.strings.push(StringData {
        value: name.into(),
        utf16_size: name.len() as u32,
        span,
    });
    dex.type_ids.push(TypeId {
        descriptor_idx: 5,
        span,
    });
    let code = dex.classes[0].class_data.as_mut().unwrap().direct_methods[0]
        .code
        .as_mut()
        .unwrap();
    let units = [
        0x0013u16, 10, 0x0193, 0x0300, 0x010f, 0x000d, 0x1112, 0x010f, 0x000d, 0x2112, 0x010f,
    ];
    code.insns = units.into_iter().flat_map(u16::to_le_bytes).collect();
    code.insns_size = 11;
    code.tries[0].insn_count = 2;
    code.handlers[0].handlers = vec![
        TypeAddrPair {
            type_idx: 4,
            address: 5,
        },
        TypeAddrPair {
            type_idx: 3,
            address: 8,
        },
    ];
    let aliases = inventory(std::slice::from_ref(&dex), &mut None).unwrap();
    let external = classpath();
    let metadata = dex_decompiler::infer::DexMetadata::new(&dex);
    let mut r = Resolver::new(&dex, &aliases, &metadata);
    r.external = Some(&external);
    install_jdk(&mut r, &external);
    let class = &dex.classes[0];
    let encoded = &class.class_data.as_ref().unwrap().direct_methods[0];
    let ctx = initial_context(&r, encoded).unwrap();
    let ssa =
        dex_decompiler::DexPasses::run_ssa_with(encoded.code.as_ref().unwrap(), &ctx).unwrap();
    let scopes = crate::java_exception_regions::from_dex(
        &ssa,
        encoded.code.as_ref().unwrap(),
        &r,
        &r.hierarchy,
    )
    .unwrap();
    let signature = r.method(MethodRef::new(0)).unwrap();
    let bound = declaration_bindings(&ssa, &signature, encoded.code.as_ref().unwrap()).unwrap();
    assert!(
        matches!(
            backend_java::exceptions::emit_method_region(ssa.ir(), &signature, &bound, &r, &scopes),
            SourceResult::Unsupported(_)
        ),
        "legacy backend must reject shadowed catch ordering"
    );
    let candidate = method_source(class, encoded, &r, ClassKind::Class, &mut None).unwrap();
    assert!(candidate.source.contains("_pc"));
    compile_run(
        "backend-catch-priority",
        &candidate.source,
        "if(dispatch(0)!=1 || dispatch(2)!=5) throw new AssertionError(\"encoded catch priority\"); System.out.print(\"facade-dispatch-ok\");",
    );
}

#[test]
fn production_exception_fallback_executes_original_loop_and_rejects_bad_ranges_and_monitors() {
    let mut dex = raw_dex();
    let aliases = inventory(std::slice::from_ref(&dex), &mut None).unwrap();
    let external = classpath();
    let metadata = dex_decompiler::infer::DexMetadata::new(&dex);
    let mut r = Resolver::new(&dex, &aliases, &metadata);
    r.external = Some(&external);
    install_jdk(&mut r, &external);
    let class = &dex.classes[0];
    let method = &class.class_data.as_ref().unwrap().direct_methods[0];
    let ctx = initial_context(&r, method).unwrap();
    let ssa = dex_decompiler::DexPasses::run_ssa_with(method.code.as_ref().unwrap(), &ctx).unwrap();
    assert!(
        crate::java_exception_regions::from_dex(
            &ssa,
            method.code.as_ref().unwrap(),
            &r,
            &r.hierarchy
        )
        .is_err()
    );
    let candidate = method_source(class, method, &r, ClassKind::Class, &mut None).unwrap();
    assert!(
        candidate.source.contains("_pc"),
        "fallback schedule must be selected"
    );
    compile_run(
        "raw-loop",
        &candidate.source,
        "if(dispatch(3)!=1 || dispatch(0)!=-1 || dispatch(-1)!=-1) throw new AssertionError(); System.out.print(\"facade-dispatch-ok\");",
    );
    drop(r);
    drop(metadata);
    dex.classes[0].class_data.as_mut().unwrap().direct_methods[0]
        .code
        .as_mut()
        .unwrap()
        .tries[0]
        .insn_count = 65535;
    let metadata = dex_decompiler::infer::DexMetadata::new(&dex);
    let mut r = Resolver::new(&dex, &aliases, &metadata);
    r.external = Some(&external);
    install_jdk(&mut r, &external);
    let class = &dex.classes[0];
    let method = &class.class_data.as_ref().unwrap().direct_methods[0];
    assert!(method_source(class, method, &r, ClassKind::Class, &mut None).is_err());
    drop(r);
    drop(metadata);
    let code = dex.classes[0].class_data.as_mut().unwrap().direct_methods[0]
        .code
        .as_mut()
        .unwrap();
    let monitor = [0x0012u16, 0x001d, 0x030f, 0x000d, 0xf112, 0x010f];
    code.insns = monitor.into_iter().flat_map(u16::to_le_bytes).collect();
    code.insns_size = 6;
    code.tries[0].start_addr = 1;
    code.tries[0].insn_count = 1;
    code.handlers[0].handlers[0].address = 3;
    let metadata = dex_decompiler::infer::DexMetadata::new(&dex);
    let mut r = Resolver::new(&dex, &aliases, &metadata);
    r.external = Some(&external);
    install_jdk(&mut r, &external);
    let class = &dex.classes[0];
    let method = &class.class_data.as_ref().unwrap().direct_methods[0];
    assert!(
        method_source(class, method, &r, ClassKind::Class, &mut None).is_err(),
        "unbalanced monitor must retain rejection"
    );
}

#[test]
#[ignore = "local Samsung APK and genuine JDK17 metadata; bounded actual facade method"]
fn samsung_original_parse_method_through_production_facade_preserves_legacy_source() {
    let files = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let dex = &files[0];
    let class = dex
        .classes
        .iter()
        .find(|c| {
            c.class_data.as_ref().is_some_and(|d| {
                d.direct_methods
                    .iter()
                    .chain(&d.virtual_methods)
                    .any(|m| m.method_idx == 11633)
            })
        })
        .unwrap();
    let encoded = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .chain(&class.class_data.as_ref().unwrap().virtual_methods)
        .find(|m| m.method_idx == 11633)
        .unwrap();
    let code = encoded.code.as_ref().unwrap();
    // One genuine original body plus its genuine referenced method signatures.
    let seed = dex_decompiler::DexPasses::run_ssa_with(
        code,
        &dex_decompiler::DexTypes::new(dex, Some(encoded.method_idx)),
    )
    .unwrap();
    let mut inv = AliasInventory::default();
    for ty in &dex.type_ids {
        inv.add_type(dex.string(ty.descriptor_idx).unwrap())
            .unwrap();
    }
    inv.add_method(method_key(dex, encoded.method_idx).unwrap())
        .unwrap();
    for insn in seed.ir().instructions().values() {
        if let ManagedInsn::Invoke { method, .. } = insn {
            inv.add_method(method_key(dex, method.index()).unwrap())
                .unwrap();
        }
    }
    let aliases = inv.build(AliasMode::Ascii);
    let external = classpath();
    let metadata = dex_decompiler::infer::DexMetadata::new(dex);
    let mut r = Resolver::new(dex, &aliases, &metadata);
    r.external = Some(&external);
    install_jdk(&mut r, &external);
    let candidate = method_source(class, encoded, &r, ClassKind::Class, &mut None).unwrap();
    let signature = r.method(MethodRef::new(11633)).unwrap();
    let ctx = initial_context(&r, encoded).unwrap();
    let ssa = dex_decompiler::DexPasses::run_ssa_with(code, &ctx).unwrap();
    let legacy = crate::java_exception_regions::from_dex(&ssa, code, &r, &r.hierarchy).unwrap();
    let bound = declaration_bindings(&ssa, &signature, code).unwrap();
    let source = match backend_java::exceptions::emit_method_region(
        ssa.ir(),
        &signature,
        &bound,
        &r,
        &legacy,
    ) {
        SourceResult::Complete { source, .. } => source,
        e => panic!("legacy: {e:?}"),
    };
    let prefix = if signature.is_static {
        "public static "
    } else {
        "public "
    };
    let expected = source.replacen(
        prefix,
        &format!("{}static ", visibility(encoded.access_flags).unwrap()),
        1,
    );
    assert_eq!(
        candidate.source, expected,
        "successful legacy output must remain exact"
    );
    assert!(!candidate.source.contains("_pc"));
    assert!(
        candidate
            .dependencies
            .iter()
            .any(|m| m.owner == "Ljava/lang/Integer;" && m.name == "parseInt")
    );
    let name = signature.name;
    compile_run(
        "Samsung11633",
        &candidate.source,
        &format!(
            "if({name}(\"123\",7)!=123 || {name}(null,7)!=7 || {name}(\"bad\",7)!=7 || {name}(\"2147483648\",7)!=7) throw new AssertionError(); System.out.print(\"facade-dispatch-ok\");"
        ),
    );
    println!(
        "Samsung pool0 original method11633 facade metadata/requirements and unchanged legacy source compile/runtime verified"
    );
}
