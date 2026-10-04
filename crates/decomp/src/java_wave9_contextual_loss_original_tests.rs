//! Bounded original Samsung methods, with genuine original alias and classpath
//! metadata, exercising production contextual and handler-predicate recovery.
#![allow(clippy::unwrap_used)]
use super::*;

#[test]
#[ignore = "original Samsung APK with local genuine Android/JDK17 metadata"]
fn samsung_wave9_contextual_loss_original_witnesses() {
    let files = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let external = crate::java_external_metadata::Classpath::from_paths(&[
        std::path::PathBuf::from("/home/jtjisgod/tools/jdk-17.0.13+11/jmods/java.base.jmod"),
        std::path::PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
    ])
    .unwrap();
    let mut inventory = AliasInventory::default();
    for dex in &files {
        for ty in &dex.type_ids {
            inventory
                .add_type(dex.string(ty.descriptor_idx).unwrap())
                .unwrap();
        }
        for index in 0..dex.method_ids.len() {
            inventory
                .add_method(method_key(dex, index as u32).unwrap())
                .unwrap();
        }
        for index in 0..dex.field_ids.len() {
            inventory
                .add_field(field_key(dex, index as u32).unwrap())
                .unwrap();
        }
    }
    let aliases = inventory.build(AliasMode::Ascii);
    let catalog = method_catalog(&files);
    let original = Arc::new(
        crate::java_hierarchy::build_hierarchy(
            &files.iter().collect::<Vec<_>>(),
            external.hierarchy(),
        )
        .unwrap(),
    );
    let hierarchy = original
        .map_names(|n| {
            aliases
                .class_name(&format!("L{};", n.replace('.', "/")))
                .unwrap_or(n)
                .to_owned()
        })
        .unwrap();
    let root = std::path::Path::new("/tmp/opencode/wave9-contextual-loss");
    std::fs::create_dir_all(root).unwrap();
    for (dex_index, method_index, expected_owner, expected_name) in [
        (
            2,
            43723,
            "Ldagger/hilt/android/internal/managers/ViewComponentManager;",
            "d",
        ),
        (
            2,
            40557,
            "Lcom/yessign/fido/asn1/DERBoolean;",
            "getInstance",
        ),
        (0, 5765, "Landroidx/appcompat/app/s$a;", "a"),
    ] {
        let dex = &files[dex_index];
        let metadata = dex_decompiler::infer::DexMetadata::new(dex);
        let mut resolver = Resolver::with_catalog(dex, &aliases, &metadata, Some(&catalog));
        resolver.inputs = &files;
        resolver.dex_index = dex_index;
        resolver.external = Some(&external);
        resolver.original_hierarchy = Some(original.clone());
        resolver.hierarchy = hierarchy.clone();
        resolver.input_owners = files
            .iter()
            .flat_map(|dex| {
                dex.classes
                    .iter()
                    .filter_map(|c| descriptor(dex, c.class_idx).map(str::to_owned))
            })
            .collect();
        for d in external.hierarchy() {
            let key = format!("L{};", d.name.replace('.', "/"));
            let name = aliases.class_name(&key).unwrap_or(&d.name).to_owned();
            if let Some(access_flags) = external.class_access(&key) {
                resolver.class_catalog.insert(
                    name.clone(),
                    Some(backend_java::constructors::ConstructorClass {
                        name,
                        superclass: d.superclass.map(|s| {
                            aliases
                                .class_name(&format!("L{};", s.replace('.', "/")))
                                .unwrap_or(&s)
                                .to_owned()
                        }),
                        access_flags,
                    }),
                );
            }
        }
        for dex in &files {
            for c in &dex.classes {
                if let Some(name) = descriptor(dex, c.class_idx).and_then(|d| aliases.class_name(d))
                {
                    resolver.class_catalog.insert(
                        name.to_owned(),
                        Some(backend_java::constructors::ConstructorClass {
                            name: name.to_owned(),
                            superclass: descriptor(dex, c.superclass_idx)
                                .and_then(|d| aliases.class_name(d))
                                .map(str::to_owned),
                            access_flags: c.access_flags,
                        }),
                    );
                }
            }
        }
        let key = method_key(dex, method_index).unwrap();
        assert_eq!(key.owner, expected_owner);
        assert_eq!(key.name, expected_name);
        let class = dex
            .classes
            .iter()
            .find(|c| {
                c.class_data.as_ref().is_some_and(|d| {
                    d.direct_methods
                        .iter()
                        .chain(&d.virtual_methods)
                        .any(|m| m.method_idx == method_index)
                })
            })
            .unwrap();
        let data = class.class_data.as_ref().unwrap();
        let method = data
            .direct_methods
            .iter()
            .chain(&data.virtual_methods)
            .find(|m| m.method_idx == method_index)
            .unwrap();
        let code = method.code.as_ref().unwrap();
        if method_index == 43723 {
            assert_eq!(
                code.insns,
                [
                    32, 16, 137, 0, 56, 0, 15, 0, 110, 32, 132, 189, 18, 0, 10, 0, 57, 0, 9, 0, 31,
                    1, 137, 0, 110, 16, 255, 1, 1, 0, 12, 1, 40, 240, 17, 1
                ]
            );
            let getter = resolver.method(MethodRef::new(511)).unwrap();
            assert_eq!(getter.owner, "android.content.ContextWrapper");
            assert_eq!(getter.name, "getBaseContext");
            assert!(!getter.is_static);
            assert!(getter.parameters.is_empty());
            assert_eq!(
                getter.return_type,
                JavaType::Class("android.content.Context".into())
            );
            let check = resolver.method(MethodRef::new(48516)).unwrap();
            assert_eq!(check.owner, "java.lang.Class");
            assert_eq!(check.name, "isInstance");
            assert_eq!(
                check.parameters,
                [JavaType::Class("java.lang.Object".into())]
            );
            assert_eq!(check.return_type, JavaType::Boolean);
        }
        let (context, seeds) = initial_context_with_seeds(&resolver, method).unwrap();
        let ssa = cached_frontend(&resolver, method, code, &context, seeds, None).unwrap();
        let ir = ssa.ir();
        let signature = resolver.method(MethodRef::new(method_index)).unwrap();
        let declarations = backend_java::contextual_locals::declaration_plan(
            ir,
            &signature.return_type,
            &resolver,
        )
        .unwrap();
        let mut dump = format!(
            "key={key:?}\nregisters={} incoming={}\nraw={:?}\ndeclarations={declarations:?}\n",
            code.registers_size, code.ins_size, code.insns
        );
        for (id, instruction) in ir.instructions() {
            dump.push_str(&format!(
                "{id:?} offset={:?} {instruction:?}\n",
                ssa.offset(id)
            ));
            if let ManagedInsn::Phi { result, inputs } = instruction {
                dump.push_str(&format!(
                    "  constraints={:?}\n",
                    backend_java::contextual_locals::consumer_constraints(
                        ir,
                        &[*result],
                        &signature.return_type,
                        &resolver
                    )
                ));
                for (_, value) in inputs {
                    let record = ir.value(*value).unwrap();
                    dump.push_str(&format!(
                        "  INPUT {value:?} type={:?} definition={:?}\n",
                        record.ty(),
                        ir.instruction(record.def()).unwrap()
                    ));
                }
                let mut candidates = vec![
                    signature.return_type.clone(),
                    JavaType::Class("java.lang.Object".into()),
                ];
                if let managed_ir::types::ManagedType::Reference(class) =
                    ir.value(*result).unwrap().ty()
                {
                    candidates.push(JavaType::Class(resolver.class(*class).unwrap()));
                }
                for candidate in candidates {
                    dump.push_str(&format!(
                        "  CANDIDATE {candidate:?}: {:?}\n",
                        backend_java::contextual_locals::checked_phi_plan(
                            ir,
                            *result,
                            &candidate,
                            &signature.return_type,
                            &backend_java::type_compatibility::Hierarchy::default(),
                            &resolver
                        )
                    ));
                }
            }
        }
        for (id, value) in ir.values() {
            dump.push_str(&format!(
                "{id:?} {value:?} declaration={:?}\n",
                declarations.get(&id)
            ));
        }
        std::fs::write(
            root.join(format!("original-{dex_index}-{method_index}-ir.txt")),
            dump,
        )
        .unwrap();
        let source = method_source(class, method, &resolver, ClassKind::Class, &mut None);
        match source {
            Ok(fragment) => {
                assert!(fragment.body, "original body withdrawn: {key:?}");
                std::fs::write(
                    root.join(format!("original-{dex_index}-{method_index}.java")),
                    &fragment.source,
                )
                .unwrap();
                println!(
                    "ORIGINAL {dex_index}:{method_index} {key:?} body={} bytes={}",
                    fragment.body,
                    fragment.source.len()
                );
            }
            Err(error) => {
                std::fs::write(
                    root.join(format!("original-{dex_index}-{method_index}-rejection.txt")),
                    &error,
                )
                .unwrap();
                println!("ORIGINAL REFUSED {dex_index}:{method_index} {key:?} reason={error}");
                panic!("original recovery regression: {key:?}: {error}");
            }
        }
    }
}

/// The entire original #43723 body is unchanged. Only exact Android API
/// boundaries are fixtures; the comparison body encodes its attested physical
/// instruction order directly, independently of contextual SSA declarations.
#[test]
#[ignore = "run genuine witness first; requires explicitly supplied local JDK25"]
#[allow(clippy::expect_used)] // Fail explicitly if the required runtime is absent.
fn unchanged_original_43723_body_matches_original_instruction_runtime() {
    use std::{fs, path::Path, process::Command};
    let root = Path::new("/tmp/opencode/wave9-contextual-loss");
    let body = fs::read_to_string(root.join("original-2-43723.java")).unwrap();
    assert!(body.starts_with(
        "public static android.content.Context d(android.content.Context p0, java.lang.Class p1)"
    ));
    let ir = fs::read_to_string(root.join("original-2-43723-ir.txt")).unwrap();
    assert!(ir.contains("raw=[32, 16, 137, 0, 56, 0, 15, 0, 110, 32, 132, 189, 18, 0, 10, 0, 57, 0, 9, 0, 31, 1, 137, 0, 110, 16, 255, 1, 1, 0, 12, 1, 40, 240, 17, 1]"));
    let home = std::env::var_os("DECOMPHIFY_JDK25_HOME")
        .map(std::path::PathBuf::from)
        .expect("explicit genuine JDK25 home");
    let dir = root.join("runtime-43723");
    fs::create_dir_all(&dir).unwrap();
    for (path,source) in [
        ("android/content/Context.java", "package android.content; public class Context {public final String name;public Context(String n){name=n;}}".to_owned()),
        ("android/content/ContextWrapper.java", r#"package android.content;
public class ContextWrapper extends Context {public Context base;public boolean fail;public static String trace="";public static RuntimeException failure=new RuntimeException("same");
public ContextWrapper(String n,Context b){super(n);base=b;}public Context getBaseContext(){trace+=name+":";if(fail)throw failure;return base;}}
"#.to_owned()),
        ("dagger/hilt/android/internal/managers/ViewComponentManager.java", format!("package dagger.hilt.android.internal.managers; public class ViewComponentManager {{\n{body}\n}}")),
        ("Harness.java", r#"import android.content.*;import dagger.hilt.android.internal.managers.ViewComponentManager;
public class Harness {
public static void main(String[]args){int mode=Integer.parseInt(args[0]);Context base=new Context("base");ContextWrapper inner=new ContextWrapper("inner",base),outer=new ContextWrapper("outer",inner);Context input;Class<?> target;
switch(mode){case 0:input=base;target=null;break;case 1:input=null;target=null;break;case 2:input=outer;target=ContextWrapper.class;break;case 3:input=inner;target=String.class;break;case 4:input=outer;target=String.class;break;case 5:input=outer;target=Context.class;break;case 6:inner.base=null;input=outer;target=String.class;break;case 7:input=outer;target=null;break;case 8:outer.fail=true;input=outer;target=String.class;break;case 9:inner.fail=true;input=outer;target=String.class;break;case 10:input=base;target=ContextWrapper.class;break;case 11:input=outer;target=Harness.class;break;default:throw new AssertionError();}
try{Context result=ViewComponentManager.d(input,target);boolean identity=result==(mode==2||mode==5?outer:mode==1||mode==6?null:base);if(!identity)throw new AssertionError("identity");System.out.println(mode+":"+ContextWrapper.trace+":"+(result==null?"null":result.name)+":identity");}
catch(Throwable t){if(!(t instanceof NullPointerException)&&t!=ContextWrapper.failure)throw new AssertionError(t);boolean frame=false;for(StackTraceElement f:t.getStackTrace())if(f.getClassName().equals("dagger.hilt.android.internal.managers.ViewComponentManager")&&f.getMethodName().equals("d"))frame=true;if(t instanceof NullPointerException&&!frame)throw new AssertionError("original caller frame absent");System.out.println(mode+":"+ContextWrapper.trace+":"+t.getClass().getName()+":"+(t==ContextWrapper.failure));}}
}"#.to_owned()),
        ("Original.java", r#"import java.lang.classfile.*;import java.lang.constant.*;import java.nio.file.*;
public class Original {public static void main(String[]args)throws Exception{ClassFile cf=ClassFile.of();ClassDesc wrapper=ClassDesc.of("android.content.ContextWrapper"),context=ClassDesc.of("android.content.Context"),klass=ClassDesc.of("java.lang.Class");Path path=Path.of("dagger/hilt/android/internal/managers/ViewComponentManager.class");Files.write(path,cf.transformClass(cf.parse(path),(cb,e)->{if(e instanceof MethodModel mm&&mm.methodName().equalsString("d"))cb.transformMethod(mm,(mb,me)->{if(me instanceof CodeModel)mb.withCode(c->{Label loop=c.newLabel(),done=c.newLabel();c.labelBinding(loop).aload(0).instanceOf(wrapper).ifeq(done);c.aload(1).aload(0).invokevirtual(klass,"isInstance",MethodTypeDesc.ofDescriptor("(Ljava/lang/Object;)Z")).ifne(done);c.aload(0).checkcast(wrapper).invokevirtual(wrapper,"getBaseContext",MethodTypeDesc.of(context)).astore(0).goto_(loop);c.labelBinding(done).aload(0).areturn();});else mb.with(me);});else cb.with(e);}));}}
"#.to_owned()),
    ] {let p=dir.join(path);fs::create_dir_all(p.parent().unwrap()).unwrap();fs::write(p,source).unwrap();}
    let run = |tool: &str, args: &[&str]| {
        let out = Command::new(home.join("bin").join(tool))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{tool}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    };
    run(
        "javac",
        &[
            "--release",
            "25",
            "android/content/Context.java",
            "android/content/ContextWrapper.java",
            "dagger/hilt/android/internal/managers/ViewComponentManager.java",
            "Harness.java",
            "Original.java",
        ],
    );
    let mut emitted = Vec::new();
    for mode in 0..12 {
        emitted.extend(run("java", &["-Xverify:all", "Harness", &mode.to_string()]));
    }
    let text = String::from_utf8(emitted.clone()).unwrap();
    assert_eq!(
        text,
        "0::base:identity\n1::null:identity\n2::outer:identity\n3:inner::base:identity\n4:outer:inner::base:identity\n5::outer:identity\n6:outer:inner::null:identity\n7::java.lang.NullPointerException:false\n8:outer::java.lang.RuntimeException:true\n9:outer:inner::java.lang.RuntimeException:true\n10::base:identity\n11:outer:inner::base:identity\n"
    );
    fs::copy(
        dir.join("dagger/hilt/android/internal/managers/ViewComponentManager.class"),
        dir.join("emitted.class"),
    )
    .unwrap();
    run("java", &["Original"]);
    let mut original = Vec::new();
    for mode in 0..12 {
        original.extend(run("java", &["-Xverify:all", "Harness", &mode.to_string()]));
    }
    assert_eq!(emitted, original);
    fs::write(dir.join("runtime.txt"), text).unwrap();
    fs::write(dir.join("scope.txt"),format!("Original pool2 method43723 whole {}-byte generated body unchanged. Genuine Android35/JDK17 declarations in original witness. Exact-descriptor Android API fixtures and original physical opcode-order JVM body;12 fresh-JVM differential modes. Source and JVM runtime verified, not full-class closure/Android DEX execution. No fabricated casts, no heap-OOM claim.\n", body.len())).unwrap();
}
