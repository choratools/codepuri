//! Original evidence for the first six wave2 definite-assignment rejections.
#![allow(clippy::unwrap_used)]
use backend_java::constructors::{
    ConstructorClass, ConstructorResolver, Initializer, TrustedConstructorResolver,
};
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType};
use decomp::java_class_kinds::initializers::write_inventory::WriteInventory;
use decomp::java_external_metadata::{Classpath, ExternalMetadata, MethodMetadata};
use dex_format::{DexFile, class::ClassDef};
use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
use std::collections::HashMap;
use std::path::PathBuf;

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
    hierarchy: &'a backend_java::type_compatibility::Hierarchy,
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
        let key = decomp::java_aliases::FieldKey {
            owner: descriptor(self.dex, u32::from(raw.class_idx)).into(),
            name: self.dex.string(raw.name_idx).ok()?.into(),
            descriptor: descriptor(self.dex, u32::from(raw.type_idx)).into(),
        };
        let access = if let Some(c) = self
            .dex
            .classes
            .iter()
            .find(|c| c.class_idx == u32::from(raw.class_idx))
        {
            c.class_data
                .as_ref()?
                .instance_fields
                .iter()
                .chain(&c.class_data.as_ref()?.static_fields)
                .find(|f| f.field_idx == reference.index())?
                .access_flags
        } else {
            self.cp.field(&key)?.access_flags
        };
        Some(JavaField {
            owner: source_name(&key.owner),
            name: key.name,
            ty: decomp::dex_java::descriptor_type(&key.descriptor)?,
            is_static: access & 8 != 0,
        })
    }
    fn class(&self, reference: ClassRef) -> Option<String> {
        match decomp::dex_java::descriptor_type(descriptor(self.dex, reference.index()))? {
            JavaType::Class(name) => Some(name),
            _ => None,
        }
    }
    fn instruction_type(&self, reference: managed_ir::refs::TypeRef) -> Option<JavaType> {
        decomp::dex_java::descriptor_type(descriptor(self.dex, reference.index()))
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
        if let (JavaType::Array(actual), JavaType::Array(expected)) = (actual, expected) {
            return self.assignable(actual, expected);
        }
        if matches!(actual, JavaType::Array(_))
            && matches!(expected, JavaType::Class(name) if matches!(name.as_str(), "java.lang.Object" | "java.lang.Cloneable" | "java.io.Serializable"))
        {
            return true;
        }
        if !matches!((actual, expected), (JavaType::Class(_), JavaType::Class(_))) {
            return false;
        }
        self.hierarchy.assignable(actual, expected)
            == backend_java::type_compatibility::Compatibility::Assignable
    }
}
impl ConstructorResolver for Resolver<'_> {
    fn constructor_source_level(&self) -> u16 {
        25
    }

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

fn bindings(
    ssa: &dex_decompiler::SsaMethod,
    m: &JavaMethod,
    code: &dex_format::code::CodeItem,
) -> backend_java::DeclarationBindings {
    let provenance = ssa
        .initial_registers()
        .iter()
        .map(
            |(v, r)| decomp::java_initial_parameters::InitialProvenance {
                value: *v,
                registers: r.clone(),
            },
        )
        .collect::<Vec<_>>();
    let p = decomp::java_initial_parameters::parameter_bindings_with_provenance(
        ssa.ir(),
        m,
        code.registers_size,
        code.ins_size,
        &provenance,
    )
    .unwrap();
    let v = |p| match p {
        decomp::java_initial_parameters::InitialParameter::Bound(v) => Some(v),
        _ => None,
    };
    backend_java::DeclarationBindings {
        receiver: p.receiver.and_then(v),
        parameters: p.parameters.into_iter().map(v).collect(),
        ..Default::default()
    }
}
use decomp::java_class_kinds;
fn owner_ref(inputs: &[DexFile], index: u32) -> &ClassDef {
    inputs[2]
        .classes
        .iter()
        .find(|c| c.class_idx == index)
        .unwrap()
}
#[test]
#[ignore = "local original Samsung APK, explicit Java metadata, JDK17; one original enum plus negative evidence"]
fn original_sdklog_prepared_values_preserve_source_and_runtime() {
    use dex_decompiler::infer::TypeContext;
    let inputs = dex_format::apk::parse_apk_path(
        "/storage_0/decomphify/tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    let cp = Classpath::from_paths([
        PathBuf::from("/tmp/decomphify-source25-java-base-649174/java.base.jar"),
        PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
    ])
    .unwrap();
    let hierarchy =
        decomp::java_hierarchy::build_with_metadata(&inputs.iter().collect::<Vec<_>>(), &cp)
            .unwrap();
    let inventory = WriteInventory::scan_with_metadata(&inputs, &cp);
    let dex = &inputs[2];
    let class = dex
        .classes
        .iter()
        .find(|c| {
            descriptor(dex, c.class_idx) == "Lcom/samsung/android/sdk/mobileservice/util/SdkLog;"
        })
        .unwrap();
    let data = class.class_data.as_ref().unwrap();
    let r = Resolver {
        inputs: &inputs,
        dex,
        class,
        cp: &cp,
        hierarchy: &hierarchy,
        super_proof: true,
    };
    let mut ssas = vec![];
    let mut bs = vec![];
    for index in [34624, 34623] {
        let m = data
            .direct_methods
            .iter()
            .find(|m| m.method_idx == index)
            .unwrap();
        let code = m.code.as_ref().unwrap();
        let context = dex_decompiler::DexTypes::new(dex, Some(index));
        let mut seeds = HashMap::new();
        let mut word = code.registers_size - code.ins_size;
        if m.access_flags & 8 == 0 {
            seeds.insert(
                (word, false),
                context
                    .type_at(u32::from(dex.method_id(index).unwrap().class_idx))
                    .unwrap(),
            );
            word += 1;
        }
        let mut ps = vec![];
        assert!(context.method_params(index, &mut ps));
        for t in ps {
            let wide = matches!(
                t,
                managed_ir::types::ManagedType::Long | managed_ir::types::ManagedType::Double
            );
            seeds.insert((word, wide), t);
            word += if wide { 2 } else { 1 };
        }
        let ssa =
            dex_decompiler::DexPasses::run_ssa_with(code, &context.with_initial_registers(seeds))
                .unwrap();
        let b = bindings(&ssa, &r.method(MethodRef::new(index)).unwrap(), code);
        ssas.push(ssa);
        bs.push(b);
    }
    let old =
        decomp::java_class_kinds::enum_initialization::plan_enum_initialization_family_with_writes(
            dex,
            class,
            &[
                decomp::java_class_kinds::enum_initialization::EnumConstructorDeclarationInput {
                    reference: MethodRef::new(34624),
                    ir: ssas[0].ir(),
                    bindings: &bs[0],
                },
            ],
            MethodRef::new(34623),
            ssas[1].ir(),
            &r,
            2,
            &inventory,
        );
    println!("BASELINE {old:?}");
    let ci = [
        java_class_kinds::enum_initialization::EnumConstructorDeclarationInput {
            reference: MethodRef::new(34624),
            ir: ssas[0].ir(),
            bindings: &bs[0],
        },
    ];
    let p =
        java_class_kinds::enum_initialization::plan_enum_initialization_family_with_prepared_values(
            &inputs,
            2,
            class,
            &ci,
            MethodRef::new(34623),
            ssas[1].ir(),
            &r,
            &inventory,
        );
    println!(
        "PATCHED {}",
        match &p {
            Ok(plan) => format!(
                "constants={:?}; ctor={:?}; prepared={:?}; suffix={:?}; capabilities={}",
                plan.constants,
                plan.constructor_sources,
                plan.encoded_declaration_initializers,
                plan.static_initializer_source,
                plan.suffix_runtime_capabilities
            ),
            Err(error) => format!("ERR {error:?}"),
        }
    );
    assert!(p.is_ok(), "actual original plan must pass");
    if let Ok(p) = p {
        assert_eq!(p.encoded_declaration_initializers.len(), 3);
        let methods =
            java_class_kinds::enum_initialization::emission::plan_enum_methods(dex, class, &p)
                .unwrap();
        println!("COMPILER METHODS {methods:?}");
        let mut source = format!(
            "package com.samsung.android.sdk.mobileservice.util;\n{} {{\n{}\n",
            p.header, p.constants_source
        );
        for field in &data.static_fields {
            if [49506, 49508].contains(&field.field_idx) {
                continue;
            }
            let f = r.field(FieldRef::new(field.field_idx)).unwrap();
            source.push_str(&format!(
                "private static final {} {}{};\n",
                match &f.ty {
                    JavaType::Boolean => "boolean".into(),
                    JavaType::Int => "int".into(),
                    JavaType::Class(name) => name.clone(),
                    _ => panic!("unexpected"),
                },
                f.name,
                p.encoded_declaration_initializers
                    .get(&field.field_idx)
                    .map_or(String::new(), |e| format!(" = {e}"))
            ));
        }

        for ctor in p.constructor_sources.values() {
            source.push_str(ctor);
            source.push('\n');
        }
        source.push_str(p.static_initializer_source.as_ref().unwrap());
        source.push_str("}\n");
        sdklog_jvm_oracle(&source);
    }

    let mut negatives = 0;
    for (label, position, value) in [
        (
            "nondefault plus original final store",
            1,
            dex_format::encoded_value::EncodedValueKind::Boolean(true),
        ),
        (
            "wrong encoded default kind",
            1,
            dex_format::encoded_value::EncodedValueKind::Int(0),
        ),
        (
            "compiler enum field nondefault",
            2,
            dex_format::encoded_value::EncodedValueKind::String(5080),
        ),
        (
            "literal wrong kind",
            3,
            dex_format::encoded_value::EncodedValueKind::Long(512),
        ),
    ] {
        let mut changed = inputs.clone();
        let owner = changed[2]
            .classes
            .iter_mut()
            .find(|c| c.class_idx == class.class_idx)
            .unwrap();
        owner.static_values.as_mut().unwrap()[position].value = value;
        let result=java_class_kinds::enum_initialization::plan_enum_initialization_family_with_prepared_values(&changed,2,owner_ref(&changed,class.class_idx),&ci,MethodRef::new(34623),ssas[1].ir(),&r,&inventory);
        assert!(result.is_err(), "{label}");
        println!("NEGATIVE {label}: {:?}", result.err());
        negatives += 1;
    }
    for (label, flags) in [
        ("public enum constructor", 0x10001),
        ("protected enum constructor", 0x10004),
    ] {
        let mut changed = inputs.clone();
        let owner = changed[2]
            .classes
            .iter_mut()
            .find(|c| c.class_idx == class.class_idx)
            .unwrap();
        owner
            .class_data
            .as_mut()
            .unwrap()
            .direct_methods
            .iter_mut()
            .find(|m| m.method_idx == 34624)
            .unwrap()
            .access_flags = flags;
        let result=java_class_kinds::enum_initialization::plan_enum_initialization_family_with_prepared_values(&changed,2,owner_ref(&changed,class.class_idx),&ci,MethodRef::new(34623),ssas[1].ir(),&r,&inventory);
        assert!(result.is_err(), "{label}");
        println!("NEGATIVE {label}: {:?}", result.err());
        negatives += 1;
    }
    let mut changed = inputs.clone();
    let owner = changed[2]
        .classes
        .iter_mut()
        .find(|c| c.class_idx == class.class_idx)
        .unwrap();
    let code = owner
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .iter_mut()
        .find(|m| m.method_idx == 34629)
        .unwrap()
        .code
        .as_mut()
        .unwrap();
    let read = dex_decompiler::decode_method(code)
        .unwrap()
        .into_iter()
        .find(|i| i.pool_index == Some(49507))
        .unwrap();
    let at = read.offset as usize * 2 + 2;
    code.insns[at..at + 2].copy_from_slice(&49509u16.to_le_bytes());
    code.insns[read.offset as usize * 2] = 0x60;
    let result =
        java_class_kinds::enum_initialization::plan_enum_initialization_family_with_prepared_values(
            &changed,
            2,
            owner_ref(&changed, class.class_idx),
            &ci,
            MethodRef::new(34623),
            ssas[1].ir(),
            &r,
            &inventory,
        );
    assert!(
        result.is_err(),
        "raw read absent from supplied positive inventory must deny"
    );
    println!(
        "NEGATIVE raw read absent from inventory: {:?}",
        result.err()
    );
    negatives += 1;

    for label in [
        "missing original final store",
        "constant runtime name mismatch",
    ] {
        let mut changed = inputs.clone();
        let owner = changed[2]
            .classes
            .iter_mut()
            .find(|c| c.class_idx == class.class_idx)
            .unwrap();
        let code = owner
            .class_data
            .as_mut()
            .unwrap()
            .direct_methods
            .iter_mut()
            .find(|m| m.method_idx == 34623)
            .unwrap()
            .code
            .as_mut()
            .unwrap();
        if label == "missing original final store" {
            code.insns[48..52].fill(0);
        } else {
            code.insns[6..8].copy_from_slice(&5080u16.to_le_bytes());
        }
        let owner = owner_ref(&changed, class.class_idx);
        let code = owner
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods
            .iter()
            .find(|m| m.method_idx == 34623)
            .unwrap()
            .code
            .as_ref()
            .unwrap();
        let context = dex_decompiler::DexTypes::new(&changed[2], Some(34623));
        let ssa = dex_decompiler::DexPasses::run_ssa_with(code, &context).unwrap();
        let result=java_class_kinds::enum_initialization::plan_enum_initialization_family_with_prepared_values(&changed,2,owner,&ci,MethodRef::new(34623),ssa.ir(),&r,&inventory);
        assert!(result.is_err(), "{label}");
        println!("NEGATIVE {label}: {:?}", result.err());
        negatives += 1;
    }

    let mut changed = inputs.clone();
    let owner = changed[2]
        .classes
        .iter_mut()
        .find(|c| c.class_idx == class.class_idx)
        .unwrap();
    let code = owner
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .iter_mut()
        .find(|m| m.method_idx == 34626)
        .unwrap()
        .code
        .as_mut()
        .unwrap();
    code.insns = [0x0012u16, 0x006au16, 49507u16, 0x000eu16]
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect();
    code.insns_size = 4;
    assert!(code.tries.is_empty());
    let external_inventory = WriteInventory::scan_with_metadata(&changed, &cp);
    let result =
        java_class_kinds::enum_initialization::plan_enum_initialization_family_with_prepared_values(
            &changed,
            2,
            owner_ref(&changed, class.class_idx),
            &ci,
            MethodRef::new(34623),
            ssas[1].ir(),
            &r,
            &external_inventory,
        );
    assert!(result.is_err(), "foreign final writer");
    println!("NEGATIVE foreign final writer: {:?}", result.err());
    negatives += 1;
    println!("NEGATIVES PASSED {negatives}");
}

fn sdklog_jvm_oracle(source: &str) {
    use std::process::Command;
    let root =
        std::env::temp_dir().join(format!("decomphify-sdklog-prepared-{}", std::process::id()));
    let oracle = root.join("oracle");
    let original = root.join("original");
    let emitted = root.join("emitted");
    let common = root.join("common");
    let builder = root.join("builder");
    for directory in [&oracle, &original, &emitted, &common, &builder] {
        std::fs::create_dir_all(directory).unwrap();
    }
    std::fs::create_dir_all(oracle.join("android/os")).unwrap();
    std::fs::write(
        oracle.join("Builder.java"),
        include_str!("fixtures/enum_prepared/Builder.java"),
    )
    .unwrap();
    std::fs::write(
        oracle.join("Check.java"),
        include_str!("fixtures/enum_prepared/Check.java"),
    )
    .unwrap();
    std::fs::write(
        oracle.join("android/os/Build.java"),
        include_str!("fixtures/enum_prepared/Build.java"),
    )
    .unwrap();
    std::fs::write(root.join("SdkLog.java"), source).unwrap();
    let home = java_home();
    let javac = home.join("bin/javac");
    let java = home.join("bin/java");
    let checked = |command: &mut Command| {
        let result = command.output().unwrap();
        assert!(
            result.status.success(),
            "{command:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    };
    checked(
        Command::new(&javac)
            .args([
                "--add-exports",
                "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
                "-d",
            ])
            .arg(&builder)
            .arg(oracle.join("Builder.java")),
    );
    checked(
        Command::new(&java)
            .args([
                "--add-exports",
                "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
                "-cp",
            ])
            .arg(&builder)
            .arg("Builder")
            .arg(&original),
    );
    checked(
        Command::new(&javac)
            .arg("-d")
            .arg(&common)
            .arg(oracle.join("Check.java"))
            .arg(oracle.join("android/os/Build.java")),
    );
    checked(
        Command::new(&javac)
            .arg("-cp")
            .arg(&common)
            .arg("-d")
            .arg(&emitted)
            .arg(root.join("SdkLog.java")),
    );
    for mode in ["eng", "user", "fail", "mutate"] {
        let expected = checked(
            Command::new(&java)
                .arg("-cp")
                .arg(format!("{}:{}", original.display(), common.display()))
                .args(["oracle.Check", mode]),
        );
        let actual = checked(
            Command::new(&java)
                .arg("-cp")
                .arg(format!("{}:{}", emitted.display(), common.display()))
                .args(["oracle.Check", mode]),
        );
        assert_eq!(
            expected, actual,
            "original opcode codec vs emitted initialization, mode {mode}"
        );
    }
}
