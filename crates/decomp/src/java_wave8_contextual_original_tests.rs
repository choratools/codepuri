//! Bounded original Samsung methods, with genuine original alias and classpath
//! metadata, exercising production contextual and handler-predicate recovery.
#![allow(clippy::unwrap_used)]
use super::*;

fn java_type(ty: &JavaType) -> String {
    match ty {
        JavaType::Void => "void".into(),
        JavaType::Boolean => "boolean".into(),
        JavaType::Byte => "byte".into(),
        JavaType::Short => "short".into(),
        JavaType::Char => "char".into(),
        JavaType::Int => "int".into(),
        JavaType::Long => "long".into(),
        JavaType::Float => "float".into(),
        JavaType::Double => "double".into(),
        JavaType::Class(name) => name.clone(),
        JavaType::Array(element) => format!("{}[]", java_type(element)),
    }
}

#[test]
#[ignore = "original Samsung APK with local genuine Android/JDK17 metadata"]
fn samsung_wave8_original_boolean_family_witness() {
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
    let dex = &files[0];
    let metadata = dex_decompiler::infer::DexMetadata::new(dex);
    let mut resolver = Resolver::with_catalog(dex, &aliases, &metadata, Some(&catalog));
    resolver.inputs = &files;
    resolver.dex_index = 0;
    resolver.external = Some(&external);
    resolver.original_hierarchy = Some(original);
    resolver.hierarchy = hierarchy;
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
            if let Some(name) = descriptor(dex, c.class_idx).and_then(|d| aliases.class_name(d)) {
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
    for (method_index, expected_owner, expected_name) in [(39055, "Le2/e;", "g")] {
        let key = method_key(dex, method_index).unwrap();
        assert_eq!(key.owner, expected_owner);
        assert_eq!(key.name, expected_name);
        assert_eq!(key.parameters, ["Lb2/d;", "Z"]);
        assert_eq!(key.return_type, "V");
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
        let d = class.class_data.as_ref().unwrap();
        let method = d
            .direct_methods
            .iter()
            .chain(&d.virtual_methods)
            .find(|m| m.method_idx == method_index)
            .unwrap();
        let code = method.code.as_ref().unwrap();
        let (ctx, seeds) = initial_context_with_seeds(&resolver, method).unwrap();
        let ssa = cached_frontend(&resolver, method, code, &ctx, seeds, None).unwrap();
        let ir = ssa.ir();
        let declarations =
            backend_java::contextual_locals::declaration_plan(ir, &JavaType::Void, &resolver)
                .unwrap();
        let index = ir
            .instructions()
            .iter()
            .find_map(|(id, instruction)| match instruction {
                ManagedInsn::ArrayGet { index, .. } if ssa.offset(id) == Some(533) => Some(*index),
                _ => None,
            })
            .unwrap();
        assert_eq!(declarations.get(&index), Some(&JavaType::Int));
        let mut dump = format!(
            "key={key:?}\nregisters={} incoming={}\ndeclarations={declarations:?}\n",
            code.registers_size, code.ins_size
        );
        for (id, insn) in ir.instructions() {
            dump.push_str(&format!("{id:?} offset={:?} {insn:?}\n", ssa.offset(id)));
        }
        for (id, value) in ir.values() {
            dump.push_str(&format!(
                "{id:?} {value:?} declaration={:?}\n",
                declarations.get(&id)
            ));
        }
        std::fs::write("/tmp/opencode/wave8-original-39055-ir.txt", dump).unwrap();
        let mut members = BTreeSet::new();
        for (_, insn) in ir.instructions() {
            match insn {
                ManagedInsn::FieldGet { field, .. } | ManagedInsn::FieldPut { field, .. } => {
                    let field = resolver.field(*field).unwrap();
                    members.insert(format!(
                        "F|{}|{}|{}|{}",
                        field.owner,
                        field.name,
                        field.is_static,
                        java_type(&field.ty)
                    ));
                }
                ManagedInsn::Invoke { method, .. } => {
                    let method = resolver.method(*method).unwrap();
                    members.insert(format!(
                        "M|{}|{}|{}|{}|{}",
                        method.owner,
                        method.name,
                        method.is_static,
                        java_type(&method.return_type),
                        method
                            .parameters
                            .iter()
                            .map(java_type)
                            .collect::<Vec<_>>()
                            .join(",")
                    ));
                }
                _ => {}
            }
        }
        let required: BTreeSet<_> = members
            .iter()
            .flat_map(|line| {
                line.split('|')
                    .skip(1)
                    .flat_map(|part| part.split(','))
                    .map(|part| part.trim_end_matches("[]").to_owned())
            })
            .filter(|name| name.contains('.'))
            .collect();
        let mut pending: Vec<_> = required.into_iter().collect();
        let mut seen = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            if let Some(Some(class)) = resolver.class_catalog.get(&name) {
                members.insert(format!(
                    "C|{}|{}",
                    class.name,
                    class.superclass.as_deref().unwrap_or("java.lang.Object")
                ));
                if let Some(parent) = &class.superclass {
                    pending.push(parent.clone());
                }
            }
        }
        std::fs::write(
            "/tmp/opencode/wave8-original-39055-boundaries.txt",
            members.into_iter().collect::<Vec<_>>().join("\n"),
        )
        .unwrap();
        let source = method_source(class, method, &resolver, ClassKind::Class, &mut None)
            .unwrap_or_else(|error| {
                panic!("original {expected_owner}->{expected_name} rejected: {error}")
            });
        assert!(source.body);
        let path = format!("/tmp/opencode/wave8-original-{method_index}.java");
        std::fs::write(&path, &source.source).unwrap();
        println!(
            "ORIGINAL ACCEPTED classes.dex:{method_index} owner={expected_owner} name={expected_name} bytes={} path={path}",
            source.source.len()
        );
    }
}

#[test]
#[ignore = "retained actual39055 source/member manifest and frozen wave6 source, JDK25; explicit API boundaries"]
fn unchanged_original_39055_body_boundary_compile_and_runtime() {
    use std::process::Command;
    let manifest =
        std::fs::read_to_string("/tmp/opencode/wave8-original-39055-boundaries.txt").unwrap();
    let fixed = std::fs::read_to_string("/tmp/opencode/wave8-original-39055.java").unwrap();
    let baseline_unit = std::fs::read_to_string(
        "/tmp/opencode/samsung-pass-cache-wave6/replay/cold/sources/e2/e$type.java",
    )
    .unwrap();
    let baseline = &baseline_unit[baseline_unit
        .find("    public void g(b2.d$type p0, boolean p1) {")
        .unwrap()
        ..baseline_unit
            .find("    // original method pool index 39056;")
            .unwrap()];
    let root = std::path::Path::new("/tmp/opencode/wave8-contextual-original-runtime");
    std::fs::create_dir_all(root).unwrap();
    let mut parents = BTreeMap::new();
    let mut fields: BTreeMap<String, Vec<(String, bool, String)>> = BTreeMap::new();
    let mut methods: BTreeMap<String, Vec<(String, bool, String, String)>> = BTreeMap::new();
    for line in manifest.lines() {
        let row: Vec<_> = line.split('|').collect();
        match row[0] {
            "C" if !row[1].starts_with("java.") => {
                parents.insert(row[1].to_owned(), row[2].to_owned());
            }
            "F" => {
                fields.entry(row[1].into()).or_default().push((
                    row[2].into(),
                    row[3] == "true",
                    row[4].into(),
                ));
            }
            "M" if !row[1].starts_with("java.") => {
                methods.entry(row[1].into()).or_default().push((
                    row[2].into(),
                    row[3] == "true",
                    row[4].into(),
                    row[5].into(),
                ));
            }
            _ => {}
        }
    }
    assert_eq!(
        parents.get("e2.n$type").map(String::as_str),
        Some("e2.e$type")
    );
    let home = std::env::var("DECOMPHIFY_JDK25_HOME")
        .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into());
    let mut traces = Vec::new();
    for (profile, body) in [("frozen-wave6", baseline), ("fixed-wave8", fixed.as_str())] {
        let dir = root.join(profile);
        std::fs::create_dir_all(dir.join("probe")).unwrap();
        std::fs::write(dir.join("probe/Fixture.java"), BOUNDARY_DRIVER).unwrap();
        for (owner, parent) in &parents {
            let (package, name) = owner.rsplit_once('.').unwrap();
            let mut source = format!(
                "package {package};\n// Explicit test API boundary; descriptors from actual39055 references.\n"
            );
            if parent == "java.lang.Enum" {
                let names: Vec<_> = fields
                    .get(owner)
                    .unwrap()
                    .iter()
                    .map(|f| f.0.clone())
                    .collect();
                source.push_str(&format!("public enum {name} {{ {} }}", names.join(",")));
            } else {
                source.push_str(&format!(
                    "public class {name}{} {{\n",
                    if parent == "java.lang.Object" {
                        String::new()
                    } else {
                        format!(" extends {parent}")
                    }
                ));
                for (field, static_field, ty) in fields.get(owner).into_iter().flatten() {
                    source.push_str(&format!(
                        "public {}{ty} {field};\n",
                        if *static_field { "static " } else { "" }
                    ));
                }
                for (method, static_method, ret, params) in methods.get(owner).into_iter().flatten()
                {
                    let params: Vec<_> = if params.is_empty() {
                        vec![]
                    } else {
                        params.split(',').collect()
                    };
                    let declarations: Vec<_> = params
                        .iter()
                        .enumerate()
                        .map(|(i, ty)| format!("{ty} p{i}"))
                        .collect();
                    let args: Vec<_> = (0..params.len()).map(|i| format!("p{i}")).collect();
                    let value = match ret.as_str() {
                        "void" => String::new(),
                        "boolean" => "return probe.Fixture.flag;".into(),
                        "int" => "return probe.Fixture.number;".into(),
                        "float" => "return (float) probe.Fixture.number;".into(),
                        "double" => "return (double) probe.Fixture.number;".into(),
                        _ => format!("return ({ret}) probe.Fixture.make(\"{ret}\",0);"),
                    };
                    source.push_str(&format!("public {}{ret} {method}({}) {{probe.Fixture.hit(\"{owner}.{method}\",new Object[]{{{}}}); {value}}}\n",if *static_method {"static "} else {""},declarations.join(","),args.join(",")));
                }
                if owner == "e2.e$type" {
                    source.push_str(body);
                }
                source.push_str("}\n");
            }
            let file = dir.join(owner.replace('.', "/")).with_extension("java");
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, source).unwrap();
        }
        let source_list: Vec<_> = parents
            .keys()
            .map(|n| format!("{}.java", n.replace('.', "/")))
            .chain(std::iter::once("probe/Fixture.java".into()))
            .collect();
        let compiled = Command::new(format!("{home}/bin/javac"))
            .args(["-J-Xss16m", "--release", "25"])
            .args(&source_list)
            .current_dir(&dir)
            .output()
            .unwrap();
        std::fs::write(dir.join("javac.stderr"), &compiled.stderr).unwrap();
        assert!(
            compiled.status.success(),
            "{profile}: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let mut runs = Vec::new();
        for mode in 0..28 {
            let run = Command::new(format!("{home}/bin/java"))
                .args(["-cp", ".", "probe.Fixture", &mode.to_string()])
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                run.status.success(),
                "{profile}/{mode}: {}",
                String::from_utf8_lossy(&run.stderr)
            );
            runs.push(String::from_utf8(run.stdout).unwrap());
        }
        std::fs::write(dir.join("runtime.txt"), runs.join("\n")).unwrap();
        traces.push(runs);
    }
    assert_eq!(
        traces[0], traces[1],
        "complete unchanged original method body with explicit exact-descriptor boundaries"
    );
    assert!(
        traces[1].iter().any(|t| t.contains("e2.e$type.k0")),
        "exercise genuine Boolean-call family"
    );
    assert!(traces[1].iter().any(|t| t.contains("e2.e$type.m0")));
    assert!(traces[1][24].starts_with("java.lang.IllegalStateException:true|"));
    std::fs::write(root.join("scope.txt"),"Both frozen-wave6 and fixed-wave8 complete original39055 bodies compile unchanged. All other classes/methods are explicit descriptor-attested API fixtures, not recovered original bodies. 28 fresh-VM cases compare boundary call/argument order, returned Boolean bits, exception identity and receiver primitive state. This is a source-regression differential against the frozen prior body, not execution of original Android DEX or whole-class closure.\n").unwrap();
    println!(
        "unchanged original39055 full body compile + 28-mode frozen-source boundary differential PASS {}",
        root.display()
    );
}

const BOUNDARY_DRIVER: &str = r##"package probe;
import java.lang.reflect.*;
import java.util.*;
public class Fixture {
public static boolean flag; public static int number; public static int mode;
static String trace=""; static final RuntimeException fault=new IllegalStateException("boundary failure");
static IdentityHashMap<Object,Integer> ids=new IdentityHashMap<>();
static String show(Object value) {if(value==null)return "null";if(value instanceof Number||value instanceof Boolean||value instanceof Character)return value.toString(); if(value instanceof Enum<?> e)return e.name();return value.getClass().getName()+"#"+ids.computeIfAbsent(value,x->ids.size());}
public static void hit(String name,Object[] args) {trace+=name+"(";for(Object a:args)trace+=show(a)+",";trace+=");";if((mode==24&&name.endsWith(".k0"))||(mode==25&&name.endsWith(".q"))||(mode==26&&name.endsWith(".i")))throw fault;}
public static Object make(String name,int depth) {try {
if(name.endsWith("[]")) {String e=name.substring(0,name.length()-2);if(e.equals("int"))return new int[]{0,0,0,0};if(e.equals("boolean"))return new boolean[]{flag,!flag,flag,!flag};Class<?> c=Class.forName(e);Object a=Array.newInstance(c,4);for(int i=0;i<4;i++)Array.set(a,i,c.isEnum()?c.getEnumConstants()[Math.floorMod(mode+i,c.getEnumConstants().length)]:depth<2?make(e,depth+1):null);return a;}
Class<?> c=Class.forName(name);if(c.isEnum())return c.getEnumConstants()[Math.floorMod(mode,c.getEnumConstants().length)];Object value=c.getConstructor().newInstance();if(depth>2)return value;
Field[] fs=c.getFields();Arrays.sort(fs,Comparator.comparing(Field::getName));for(Field f:fs)if(!Modifier.isStatic(f.getModifiers())) {Class<?> t=f.getType();if(t==int.class)f.setInt(value,0);else if(t==float.class)f.setFloat(value,0.5f);else if(t==boolean.class)f.setBoolean(value,flag);else if(t.isArray())f.set(value,make(t.getComponentType().getName()+"[]",depth+1));else f.set(value,make(t.getName(),depth+1));}return value;
}catch(ReflectiveOperationException e){throw new AssertionError(e);}}
public static void main(String[] args) throws Exception {mode=Integer.parseInt(args[0]);flag=(mode&1)!=0;number=0;
e2.e$type root=(e2.e$type)make(mode>=12&&mode<24?"e2.f$type":"e2.e$type",0);root.c0=(mode%3==0)?null:root;root.k=false;root.p=false;root.q=false;root.h=false;
if(mode==24)root.c0=root;
root.b0=new e2.e$b[]{e2.e$b.b,e2.e$b.c};if(mode%4==1)root.b0[1]=e2.e$b.b;if(mode%4==2)root.b0[0]=e2.e$b.c;
if(mode==20)root.b0=null;if(mode==21)root.a0=null;if(mode==22)root.a0=new boolean[1];if(mode==23)root.b0=new e2.e$b[0];
b2.d$type solver=new b2.d$type();String result="ok";try{root.g(solver,flag);}catch(Throwable e){result=e.getClass().getName()+":"+(e==fault);}
String state="";Field[] fields=root.getClass().getFields();Arrays.sort(fields,Comparator.comparing(Field::getName));for(Field f:fields)if(!Modifier.isStatic(f.getModifiers())&&f.getType().isPrimitive())state+=f.getName()+"="+f.get(root)+";";
System.out.println(result+"|"+trace+"|"+state);
}
}"##;
