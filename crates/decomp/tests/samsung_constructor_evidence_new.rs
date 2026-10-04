//! Original Samsung constructors, genuine metadata, and exact runtime fixtures.
#![allow(clippy::unwrap_used)]
use backend_java::constructors::{
    ConstructorClass, ConstructorInput, ConstructorResolver, Initializer,
    TrustedConstructorResolver,
};
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType};
use decomp::java_class_kinds::initializers::{self, write_inventory::WriteInventory};
use decomp::java_external_metadata::{Classpath, ExternalMetadata, MethodMetadata};
use dex_format::{DexFile, class::ClassDef};
use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};

const GUIDELINE: &str = "Landroidx/constraintlayout/widget/Guideline;";
const CRYPTO: &str = "Lcom/samsung/android/samsungpass/passservice/mdl/CryptoLibException;";
const EXPIRED: &str =
    "Lcom/samsung/android/samsungpass/passservice/mdl/BiometricTokenExpiredException;";

fn descriptor(dex: &DexFile, index: u32) -> &str {
    dex.string(dex.type_id(index).unwrap().descriptor_idx)
        .unwrap()
}
fn source_name(descriptor: &str) -> String {
    descriptor
        .trim_start_matches('L')
        .trim_end_matches(';')
        .replace('/', ".")
}
fn method_key(dex: &DexFile, index: u32) -> decomp::java_aliases::MethodKey {
    let raw = dex.method_id(index).unwrap();
    let proto = dex.proto_id(u32::from(raw.proto_idx)).unwrap();
    decomp::java_aliases::MethodKey {
        owner: descriptor(dex, u32::from(raw.class_idx)).into(),
        name: dex.string(raw.name_idx).unwrap().into(),
        parameters: proto
            .parameters
            .as_ref()
            .into_iter()
            .flat_map(|p| &p.types)
            .map(|i| descriptor(dex, u32::from(*i)).into())
            .collect(),
        return_type: descriptor(dex, proto.return_type_idx).into(),
    }
}
struct Resolver<'a> {
    inputs: &'a [DexFile],
    dex: &'a DexFile,
    class: &'a ClassDef,
    cp: &'a Classpath,
    super_proof: bool,
}
impl Resolver<'_> {
    fn declaration(&self, key: &decomp::java_aliases::MethodKey) -> Option<MethodMetadata> {
        for dex in self.inputs {
            if let Some(class) = dex
                .classes
                .iter()
                .find(|c| descriptor(dex, c.class_idx) == key.owner)
            {
                let data = class.class_data.as_ref()?;
                let m = data
                    .direct_methods
                    .iter()
                    .chain(&data.virtual_methods)
                    .find(|m| method_key(dex, m.method_idx) == *key)?;
                let exceptions = decomp::java_members::MemberResolver { dex }
                    .declared_throws_for_class(class, m.method_idx)?
                    .iter()
                    .map(|name| format!("L{};", name.replace('.', "/")))
                    .collect();
                return Some(MethodMetadata {
                    access_flags: m.access_flags,
                    exceptions: Some(exceptions),
                });
            }
        }
        self.cp.method(key)
    }
}
impl JavaResolver for Resolver<'_> {
    fn super_invocation(
        &self,
        caller: &str,
        reference: MethodRef,
    ) -> Option<backend_java::SuperInvocation> {
        if !self.constructor_super_target(caller, reference) {
            return None;
        }
        Some(backend_java::SuperInvocation {
            method: self.method(reference)?,
            access_flags: self.method_access(reference)?,
        })
    }
    fn method(&self, reference: MethodRef) -> Option<JavaMethod> {
        let key = method_key(self.dex, reference.index());
        let metadata = self.declaration(&key)?;
        Some(JavaMethod {
            owner: source_name(&key.owner),
            name: key.name,
            is_static: metadata.access_flags & 8 != 0,
            return_type: decomp::dex_java::descriptor_type(&key.return_type)?,
            parameters: key
                .parameters
                .iter()
                .map(|p| decomp::dex_java::descriptor_type(p))
                .collect::<Option<_>>()?,
            throws: metadata
                .exceptions?
                .iter()
                .map(|d| source_name(d))
                .collect(),
        })
    }
    fn method_access(&self, reference: MethodRef) -> Option<u32> {
        Some(
            self.declaration(&method_key(self.dex, reference.index()))?
                .access_flags,
        )
    }
    fn field(&self, reference: FieldRef) -> Option<JavaField> {
        let raw = self.dex.field_id(reference.index()).ok()?;
        let data = self.class.class_data.as_ref()?;
        let declared = data
            .instance_fields
            .iter()
            .chain(&data.static_fields)
            .find(|f| f.field_idx == reference.index())?;
        Some(JavaField {
            owner: self.class(ClassRef::new(u32::from(raw.class_idx)))?,
            name: self.dex.string(raw.name_idx).ok()?.into(),
            ty: decomp::dex_java::descriptor_type(descriptor(self.dex, u32::from(raw.type_idx)))?,
            is_static: declared.access_flags & 8 != 0,
        })
    }
    fn class(&self, reference: ClassRef) -> Option<String> {
        match decomp::dex_java::descriptor_type(descriptor(self.dex, reference.index()))? {
            JavaType::Class(name) => Some(name),
            _ => None,
        }
    }
    fn string(&self, reference: StringRef) -> Option<Vec<u16>> {
        Some(
            self.dex
                .string(reference.index())
                .ok()?
                .encode_utf16()
                .collect(),
        )
    }
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        if actual == expected {
            return true;
        }
        let (JavaType::Class(actual), JavaType::Class(expected)) = (actual, expected) else {
            return false;
        };
        let mut next = Some(actual.clone());
        let mut seen = std::collections::BTreeSet::new();
        while let Some(name) = next {
            if name == *expected {
                return true;
            }
            if !seen.insert(name.clone()) {
                return false;
            }
            next = self.constructor_class(&name).and_then(|c| c.superclass);
        }
        false
    }
}
impl ConstructorResolver for Resolver<'_> {
    fn initializer(&self, reference: MethodRef) -> Option<Initializer> {
        let method = self.method(reference)?;
        if method.name != "<init>" {
            return None;
        }
        let delegates_to = self
            .class
            .class_data
            .as_ref()?
            .direct_methods
            .iter()
            .find(|m| m.method_idx == reference.index())
            .and_then(|m| m.code.as_ref())
            .and_then(|code| dex_decompiler::decode_method(code).ok())
            .and_then(|instructions| {
                instructions.into_iter().find_map(|i| {
                    let dex_decompiler::Opcode::Supported(info) = i.opcode else {
                        return None;
                    };
                    if ![0x70, 0x76].contains(&info.opcode) {
                        return None;
                    }
                    let target = i.pool_index?;
                    let raw = self.dex.method_id(target).ok()?;
                    (u32::from(raw.class_idx) == self.class.class_idx
                        && self.dex.string(raw.name_idx).ok()? == "<init>")
                        .then_some(MethodRef::new(target))
                })
            });
        Some(Initializer {
            method,
            delegates_to,
        })
    }
    fn constructor_super_target(&self, caller: &str, reference: MethodRef) -> bool {
        if !self.super_proof || caller != source_name(descriptor(self.dex, self.class.class_idx)) {
            return false;
        }
        let key = method_key(self.dex, reference.index());
        if key.owner != descriptor(self.dex, self.class.superclass_idx) {
            return false;
        }
        let Some(parent) = self.cp.class(&key.owner) else {
            return false;
        };
        let mut witnessed = false;
        for method in &self.class.class_data.as_ref().unwrap().direct_methods {
            let Some(code) = &method.code else {
                continue;
            };
            for insn in dex_decompiler::decode_method(code).unwrap() {
                let dex_decompiler::Opcode::Supported(info) = insn.opcode else {
                    continue;
                };
                if insn.pool_index == Some(reference.index())
                    && [0x6f, 0x70, 0x75, 0x76].contains(&info.opcode)
                {
                    if ![0x6f, 0x75].contains(&info.opcode) {
                        return false;
                    }
                    witnessed = true;
                }
            }
        }
        // Exact direct-parent original declaration, no source alias or generic/
        // varargs competitor. Descriptor casts pin Java overload resolution.
        witnessed
            && parent.methods.iter().any(|m| {
                m.key == key
                    && m.signature.is_none()
                    && m.metadata.access_flags & (2 | 8 | 0x80 | 0x400) == 0
            })
            && parent
                .methods
                .iter()
                .filter(|m| m.key.name == key.name)
                .all(|m| m.signature.is_none() && m.metadata.access_flags & 0x80 == 0)
    }
}
impl TrustedConstructorResolver for Resolver<'_> {
    fn constructor_class(&self, name: &str) -> Option<ConstructorClass> {
        for dex in self.inputs {
            if let Some(class) = dex
                .classes
                .iter()
                .find(|c| source_name(descriptor(dex, c.class_idx)) == name)
            {
                return Some(ConstructorClass {
                    name: name.into(),
                    superclass: (class.superclass_idx != u32::MAX)
                        .then(|| source_name(descriptor(dex, class.superclass_idx))),
                    access_flags: class.access_flags,
                });
            }
        }
        let class = self.cp.class(&format!("L{};", name.replace('.', "/")))?;
        Some(ConstructorClass {
            name: name.into(),
            superclass: class.declaration.superclass.clone(),
            access_flags: class.access_flags,
        })
    }
    fn constructor_access(&self, reference: MethodRef) -> Option<u32> {
        let key = method_key(self.dex, reference.index());
        (key.name == "<init>")
            .then(|| self.declaration(&key).map(|m| m.access_flags))
            .flatten()
    }
    fn declared_constructors(&self, owner: &str) -> Option<Vec<MethodRef>> {
        if source_name(descriptor(self.dex, self.class.class_idx)) != owner {
            return None;
        }
        Some(
            self.class
                .class_data
                .as_ref()?
                .direct_methods
                .iter()
                .filter(|m| method_key(self.dex, m.method_idx).name == "<init>")
                .map(|m| MethodRef::new(m.method_idx))
                .collect(),
        )
    }
}

fn java_home() -> PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/home/jtjisgod/tools/jdk-17.0.13+11".into())
}
fn javac(dir: &Path, args: &[String]) {
    let result = Command::new(java_home().join("bin/javac"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "javac {args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}
fn write_java(dir: &Path, relative: &str, source: &str) -> String {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, source).unwrap();
    path.to_string_lossy().into()
}

#[test]
fn original_samsung_families_exact_signatures_compile_and_runtime() {
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let cp = Classpath::from_paths([
        java_home().join("jmods/java.base.jmod"),
        PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
    ])
    .unwrap();
    let inventory = WriteInventory::scan(&inputs);
    assert!(
        !inventory.unknown_reasons().is_empty(),
        "native capabilities must not disappear"
    );
    let native = inventory
        .unknown_reasons()
        .iter()
        .filter(|r| r.contains("unavailable/native body"))
        .count();
    eprintln!(
        "whole-input scan: {} methods, {} arbitrary unknowns ({} native/unavailable), {} field-scoped identities",
        inventory.scanned_methods(),
        inventory.unknown_reasons().len(),
        native,
        inventory.scoped_unknowns().len()
    );
    let mut sources = BTreeMap::new();
    for target in [GUIDELINE, CRYPTO, EXPIRED] {
        let (dex_index, dex, class) = inputs
            .iter()
            .enumerate()
            .find_map(|(index, dex)| {
                dex.classes
                    .iter()
                    .find(|c| descriptor(dex, c.class_idx) == target)
                    .map(|c| (index, dex, c))
            })
            .unwrap();
        let resolver = Resolver {
            inputs: &inputs,
            dex,
            class,
            cp: &cp,
            super_proof: true,
        };
        let methods = class
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods
            .iter()
            .filter(|m| method_key(dex, m.method_idx).name == "<init>")
            .collect::<Vec<_>>();
        let mut ssas = Vec::new();
        let mut bindings = Vec::new();
        for method in &methods {
            let code = method.code.as_ref().unwrap();
            let ssa = dex_decompiler::DexPasses::run_ssa_with(
                code,
                &dex_decompiler::DexTypes::new(dex, Some(method.method_idx)),
            )
            .unwrap();
            let signature = resolver.method(MethodRef::new(method.method_idx)).unwrap();
            bindings.push(
                decomp::dex_java::parameter_bindings(
                    ssa.ir(),
                    &signature,
                    code.registers_size,
                    code.ins_size,
                )
                .unwrap(),
            );
            ssas.push(ssa);
        }
        let constructors = methods
            .iter()
            .enumerate()
            .map(|(index, m)| ConstructorInput {
                reference: MethodRef::new(m.method_idx),
                ir: ssas[index].ir(),
                bindings: &bindings[index],
            })
            .collect::<Vec<_>>();
        let evidence = initializers::class_constructor_evidence(
            &inputs,
            dex_index,
            class,
            &constructors,
            None,
            &resolver,
            &inventory,
        )
        .unwrap();
        assert!(evidence.complete_initializer_coverage);
        assert!(evidence.final_writes_known.is_empty());
        let plan = initializers::plan_class_initialization_with_inventory(
            &inputs,
            dex_index,
            class,
            &constructors,
            None,
            &resolver,
            &inventory,
        )
        .unwrap();
        assert_eq!(plan.constructors.len(), methods.len());
        assert!(plan.encoded.prefix_block.is_empty());
        if target == GUIDELINE {
            assert_eq!(
                methods.iter().map(|m| m.method_idx).collect::<Vec<_>>(),
                [10300, 10301, 10302]
            );
            assert_eq!(methods.len(), 3);
            assert_eq!(
                class.class_data.as_ref().unwrap().instance_fields[0].access_flags,
                1
            );
            assert_eq!(evidence.required_declarations.len(), 1);
            for (index, ctor) in plan.constructors.iter().enumerate() {
                assert_eq!(
                    ctor.first_statement,
                    format!(
                        "super({});",
                        (0..index + 1)
                            .map(|i| format!("p{i}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                );
                assert!(ctor.source.contains("this.a ="));
                assert!(ctor.source.contains("super.setVisibility((int) ("));
                let effects = evidence.constructors[&methods[index].method_idx]
                    .as_ref()
                    .unwrap();
                assert!(effects.fields[0].before_chain.is_empty());
                assert_eq!(effects.fields[0].after_chain.len(), 1);
                assert_eq!(effects.fields[0].normal_return_counts, [1].into());
            }
            let denied = Resolver {
                super_proof: false,
                ..resolver
            };
            assert!(
                initializers::plan_class_initialization_with_inventory(
                    &inputs,
                    dex_index,
                    class,
                    &constructors,
                    None,
                    &denied,
                    &inventory
                )
                .is_err()
            );
            let partial = initializers::class_constructor_evidence(
                &inputs,
                dex_index,
                class,
                &constructors[..1],
                None,
                &resolver,
                &inventory,
            )
            .unwrap();
            assert!(!partial.complete_initializer_coverage);
            assert_eq!(partial.required_declarations.len(), 1);
            assert_eq!(partial.constructors.len(), 1);
            assert!(
                initializers::plan_class_initialization_with_inventory(
                    &inputs,
                    dex_index,
                    class,
                    &constructors[..1],
                    None,
                    &resolver,
                    &inventory
                )
                .is_err()
            );
        } else {
            assert_eq!(methods.len(), 2);
            assert!(
                plan.constructors
                    .iter()
                    .all(|c| !c.parameter_names.is_empty()),
                "never invent a noarg constructor"
            );
        }
        let owner = source_name(target);
        let (package, name) = owner.rsplit_once('.').unwrap();
        let parent = resolver
            .constructor_class(&owner)
            .unwrap()
            .superclass
            .unwrap();
        let fields = if target == GUIDELINE {
            "public boolean a;"
        } else {
            ""
        };
        let source = format!(
            "package {package}; public class {name} extends {parent} {{ {fields}\n{} }}",
            plan.constructors
                .iter()
                .map(|c| c.source.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
        eprintln!(
            "{owner}: {} original constructors; no instance finals",
            methods.len()
        );
        sources.insert(owner.replace('.', "/") + ".java", source);
    }
    // Actual final-bearing residual: do not relabel its local evidence as a
    // whole-input exclusivity proof or fabricate a body for its Kotlin mask ctor.
    let residual = "Lcom/samsung/android/samsungpass/passservice/smartswitch/SmartSwitchException;";
    let (dex, class) = inputs
        .iter()
        .find_map(|dex| {
            dex.classes
                .iter()
                .find(|c| descriptor(dex, c.class_idx) == residual)
                .map(|c| (dex, c))
        })
        .unwrap();
    let resolver = Resolver {
        inputs: &inputs,
        dex,
        class,
        cp: &cp,
        super_proof: false,
    };
    let final_field = &class.class_data.as_ref().unwrap().instance_fields[0];
    assert_eq!(final_field.access_flags, 0x12);
    let key = initializers::write_inventory::field_key(dex, final_field.field_idx).unwrap();
    assert!(inventory.proven_writes(&key).is_none());
    assert!(inventory.field_declaration(&key).is_some());
    let mut inspected = 0;
    for method in &class.class_data.as_ref().unwrap().direct_methods {
        let key = method_key(dex, method.method_idx);
        if key.name != "<init>" {
            continue;
        }
        let code = method.code.as_ref().unwrap();
        let ssa = dex_decompiler::DexPasses::run_ssa_with(
            code,
            &dex_decompiler::DexTypes::new(dex, Some(method.method_idx)),
        )
        .unwrap();
        let signature = resolver.method(MethodRef::new(method.method_idx)).unwrap();
        let incoming = decomp::java_initial_parameters::parameter_bindings(
            ssa.ir(),
            &signature,
            code.registers_size,
            code.ins_size,
        )
        .unwrap();
        let Some(decomp::java_initial_parameters::InitialParameter::Bound(receiver)) =
            incoming.receiver
        else {
            panic!("unproven original receiver")
        };
        let evidence = backend_java::constructors::constructor_receiver_initialization_evidence(
            ssa.ir(),
            MethodRef::new(method.method_idx),
            Some(receiver),
            "java.lang.Exception",
            &[FieldRef::new(final_field.field_idx)],
            &resolver,
        )
        .unwrap();
        assert!(evidence.fields[0].before_chain.is_empty());
        let count = if key.parameters.len() == 2 { 1 } else { 0 };
        assert_eq!(evidence.fields[0].normal_return_counts, [count].into());
        assert_eq!(evidence.fields[0].after_chain.len(), usize::from(count));
        if count == 0 {
            assert_eq!(key.parameters.len(), 4);
            assert_eq!(
                evidence.normal_chains,
                [(
                    backend_java::constructors::ChainKind::This,
                    MethodRef::new(13204)
                )]
                .into()
            );
        }
        eprintln!(
            "residual {} {} {:?}: local code writes={count}; whole-input final exclusivity unavailable",
            source_name(residual),
            method.method_idx,
            key.parameters
        );
        inspected += 1;
    }
    assert_eq!(inspected, 2);
    let dir = std::env::temp_dir().join("opencode").join(format!(
        "samsung-constructor-evidence-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let generated = sources
        .iter()
        .map(|(path, source)| write_java(&dir, path, source))
        .collect::<Vec<_>>();
    // Exact same generated sources first compile against genuine Android35.
    let mut args = vec![
        "--release".into(),
        "17".into(),
        "-sourcepath".into(),
        "".into(),
        "-classpath".into(),
        "/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar".into(),
        "-d".into(),
        dir.join("android-compiled").to_string_lossy().into(),
    ];
    args.extend(generated.clone());
    javac(&dir, &args);
    // Android's SDK jar throws Stub! at runtime. A separately labelled fixture
    // classpath supplies executable bodies for exactly the three real View APIs,
    // with no noarg View constructor and with observable virtual/super dispatch.
    let mut fixture_sources = vec![
        write_java(
            &dir,
            "fixture/android/content/Context.java",
            "package android.content; public class Context {}",
        ),
        write_java(
            &dir,
            "fixture/android/util/AttributeSet.java",
            "package android.util; public interface AttributeSet {}",
        ),
        write_java(
            &dir,
            "fixture/android/view/View.java",
            "package android.view; public class View { public final android.content.Context context; public final android.util.AttributeSet attrs; public final int style; public int visibility; public View(android.content.Context c) { this(c,null,0); } public View(android.content.Context c,android.util.AttributeSet a) { this(c,a,0); } public View(android.content.Context c,android.util.AttributeSet a,int s) { if(c==null) throw new IllegalArgumentException(); context=c;attrs=a;style=s; } public void setVisibility(int v) { visibility=v; } }",
        ),
    ];
    let runtime_cp = dir.join("fixture-cp");
    let mut fixture_args = vec![
        "--release".into(),
        "17".into(),
        "-d".into(),
        runtime_cp.to_string_lossy().into(),
    ];
    fixture_args.append(&mut fixture_sources);
    javac(&dir, &fixture_args);
    let probe = write_java(
        &dir,
        "Probe.java",
        "import androidx.constraintlayout.widget.Guideline; import com.samsung.android.samsungpass.passservice.mdl.*; public class Probe { static class Child extends Guideline { int calls; Child(android.content.Context c) { super(c); } public void setVisibility(int n) { calls++; } } public static void main(String[] a) { android.content.Context c=new android.content.Context(); android.util.AttributeSet attrs=new android.util.AttributeSet() {}; Guideline[] gs={new Guideline(c),new Guideline(c,attrs),new Guideline(c,attrs,17)}; for(Guideline g:gs) if(!g.a || g.context!=c || g.visibility!=8) throw new AssertionError(); if(gs[1].attrs!=attrs || gs[2].style!=17) throw new AssertionError(); Child child=new Child(c); if(child.calls!=0 || child.visibility!=8) throw new AssertionError(\"super dispatch\"); try {new Guideline(null);throw new AssertionError();} catch(IllegalArgumentException expected) {} Throwable cause=new Throwable(\"cause\"); CryptoLibException[] es={new CryptoLibException(\"msg\"),new CryptoLibException(\"msg\",cause),new BiometricTokenExpiredException(\"msg\"),new BiometricTokenExpiredException(\"msg\",cause)}; for(CryptoLibException e:es) if(!e.getMessage().equals(\"msg\")) throw new AssertionError(); if(es[1].getCause()!=cause || es[3].getCause()!=cause) throw new AssertionError(); if(Guideline.class.getDeclaredConstructors().length!=3 || CryptoLibException.class.getDeclaredConstructors().length!=2 || BiometricTokenExpiredException.class.getDeclaredConstructors().length!=2) throw new AssertionError(); } }",
    );
    let mut runtime_args = vec![
        "--release".into(),
        "17".into(),
        "-sourcepath".into(),
        "".into(),
        "-classpath".into(),
        runtime_cp.to_string_lossy().into(),
        "-d".into(),
        runtime_cp.to_string_lossy().into(),
    ];
    runtime_args.extend(generated);
    runtime_args.push(probe);
    javac(&dir, &runtime_args);
    let output = Command::new(java_home().join("bin/java"))
        .args(["-cp", runtime_cp.to_str().unwrap(), "Probe"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}
