//! Scratch evidence for the original ExportFailureState public prepared constants.
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
    nonconstant_fields: std::collections::HashSet<u32>,
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
    fn field_source_nonconstant(&self, reference: FieldRef) -> bool {
        self.nonconstant_fields.contains(&reference.index())
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

#[allow(dead_code)]
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

fn original_ssa(dex: &DexFile, class: &ClassDef, index: u32) -> dex_decompiler::SsaMethod {
    use dex_decompiler::infer::TypeContext;
    let method = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .chain(&class.class_data.as_ref().unwrap().virtual_methods)
        .find(|method| method.method_idx == index)
        .unwrap();
    let code = method.code.as_ref().unwrap();
    let context = dex_decompiler::DexTypes::new(dex, Some(index));
    let mut seeds = HashMap::new();
    let mut word = code.registers_size - code.ins_size;
    if method.access_flags & 8 == 0 {
        seeds.insert((word, false), context.type_at(class.class_idx).unwrap());
        word += 1;
    }
    let mut params = Vec::new();
    assert!(context.method_params(index, &mut params));
    for ty in params {
        let wide = matches!(
            ty,
            managed_ir::types::ManagedType::Long | managed_ir::types::ManagedType::Double
        );
        seeds.insert((word, wide), ty);
        word += if wide { 2 } else { 1 };
    }
    dex_decompiler::DexPasses::run_ssa_with(code, &context.with_initial_registers(seeds)).unwrap()
}

#[test]
#[ignore = "original Samsung APK and exact frozen dependencies, JDK25"]
fn original_escape_mode_nonconstant_string_payloads() {
    use decomp::java_class_kinds::enum_initialization::{
        self as e, EnumConstructorDeclarationInput,
    };
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
    let dex = &inputs[3];
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == "Lorg/jsoup/nodes/Entities$EscapeMode;")
        .unwrap();
    let root = PathBuf::from(format!(
        "/tmp/opencode/wave28-escape-original-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let jdk = PathBuf::from("/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1/bin");
    let genuine = PathBuf::from(
        "/tmp/opencode/samsung-pass-100-wave27/java25/sources/org/jsoup/nodes/EntitiesData.java",
    );
    std::fs::copy(&genuine, root.join("EntitiesData.java")).unwrap();
    let out = std::process::Command::new(jdk.join("javac"))
        .args(["--release", "25", "-d"])
        .arg(root.join("classes"))
        .arg(root.join("EntitiesData.java"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let metadata = decomp::java_external_metadata::ClassMetadata::parse(
        &std::fs::read(root.join("classes/org/jsoup/nodes/EntitiesData.class")).unwrap(),
    )
    .unwrap();
    let mut fields = std::collections::HashSet::new();
    for i in [14123, 14124, 14125] {
        let f = dex.field_id(i).unwrap();
        let name = dex.string(f.name_idx).unwrap();
        let original = metadata.fields.iter().find(|f| f.name == name).unwrap();
        assert!(original.constant_value.is_none());
        assert_eq!(original.descriptor, "Ljava/lang/String;");
        assert_eq!(original.access_flags, 0x18);
        fields.insert(i);
    }
    let mut r = Resolver {
        inputs: &inputs,
        dex,
        class,
        cp: &cp,
        hierarchy: &hierarchy,
        super_proof: true,
        nonconstant_fields: fields,
    };
    let ctor = original_ssa(dex, class, 31300);
    let init = original_ssa(dex, class, 31299);
    let sig = r.method(MethodRef::new(31300)).unwrap();
    let code = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|m| m.method_idx == 31300)
        .unwrap()
        .code
        .as_ref()
        .unwrap();
    let bound = bindings(&ctor, &sig, code);
    let cs = [EnumConstructorDeclarationInput {
        reference: MethodRef::new(31300),
        ir: ctor.ir(),
        bindings: &bound,
    }];
    let plan = e::plan_enum_initialization_family_with_prepared_values(
        &inputs,
        3,
        class,
        &cs,
        MethodRef::new(31299),
        init.ir(),
        &r,
        &inventory,
    )
    .unwrap();
    assert_eq!(
        plan.constants
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["xhtml", "base", "extended"]
    );
    assert_eq!(plan.constructor_sources.len(), 1);
    assert!(plan.constructor_sources[&31300].contains("Entities.access$000(this, p2, p3)"));
    let methods = e::emission::plan_enum_methods(dex, class, &plan).unwrap();
    let data = class.class_data.as_ref().unwrap();
    let fields = data
        .static_fields
        .iter()
        .chain(&data.instance_fields)
        .filter(|f| {
            f.field_idx != plan.absorbed_values_field
                && !plan.constants.iter().any(|c| c.field_idx == f.field_idx)
        })
        .map(|f| {
            (
                f.field_idx,
                decomp::java_members::MemberResolver { dex }
                    .field_declaration_without_initialization(f.field_idx)
                    .unwrap(),
            )
        })
        .collect();
    let mut members = std::collections::BTreeMap::new();
    for m in data.direct_methods.iter().chain(&data.virtual_methods) {
        if plan.absorbed_constructors.contains(&m.method_idx)
            || [plan.absorbed_clinit, methods.values, methods.value_of].contains(&m.method_idx)
            || Some(m.method_idx) == plan.absorbed_values_helper
        {
            continue;
        }
        let ssa = original_ssa(dex, class, m.method_idx);
        e::emission::validate_enum_member_ir(class, &plan, ssa.ir()).unwrap();
        let sig = r.method(MethodRef::new(m.method_idx)).unwrap();
        let bound = bindings(&ssa, &sig, m.code.as_ref().unwrap());
        let mut source = match backend_java::emit_method_declaration(ssa.ir(), &sig, &bound, &r) {
            backend_java::SourceResult::Complete { source, .. } => source,
            other => panic!("original member {} {other:?}", m.method_idx),
        };
        if m.access_flags & 7 == 2 {
            source = source.replacen("public ", "private ", 1);
        }
        members.insert(m.method_idx, source);
    }
    let source =
        e::emission::emit_enum_declaration(class, &plan, &methods, &fields, &members).unwrap();
    std::fs::write(
        root.join("Entities$EscapeMode.java"),
        format!("package org.jsoup.nodes;\n{source}\n"),
    )
    .unwrap();
    assert_eq!(data.direct_methods.len() + data.virtual_methods.len(), 16);
    println!(
        "ORIGINAL_ESCAPE_SOURCE {} original_identities=16 missing_loader=31326",
        root.display()
    );
    // Genuine ConstantValue negative: no arbitrary false attestation is needed.
    let negative = root.join("constant-source");
    std::fs::create_dir_all(&negative).unwrap();
    std::fs::write(negative.join("EntitiesData.java"),"package org.jsoup.nodes; class EntitiesData { static final String xmlPoints=\"x\", basePoints=\"b\", fullPoints=\"f\"; }").unwrap();
    let out = std::process::Command::new(jdk.join("javac"))
        .args(["--release", "25", "-d"])
        .arg(root.join("constant-classes"))
        .arg(negative.join("EntitiesData.java"))
        .output()
        .unwrap();
    assert!(out.status.success());
    let constants = decomp::java_external_metadata::ClassMetadata::parse(
        &std::fs::read(root.join("constant-classes/org/jsoup/nodes/EntitiesData.class")).unwrap(),
    )
    .unwrap();
    assert_eq!(constants.fields.len(), 3);
    assert!(constants.fields.iter().all(|f| f.constant_value.is_some()));
    r.nonconstant_fields.clear();
    assert!(
        e::plan_enum_initialization_family_with_prepared_values(
            &inputs,
            3,
            class,
            &cs,
            MethodRef::new(31299),
            init.ir(),
            &r,
            &inventory
        )
        .is_err()
    );
}

#[test]
fn nonconstant_string_keeps_initialization_and_reference_identity() {
    let jdk = PathBuf::from("/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1/bin");
    let root = PathBuf::from(format!(
        "/tmp/opencode/wave28-string-runtime-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Proof.java"),r#"public class Proof {
 static final StringBuilder trace=new StringBuilder();
 static class Data { static final String VALUE; static { trace.append("D"); VALUE=new String("payload"); } }
 static class ConstantData { static final String VALUE="payload"; static { trace.append("X"); } }
 enum Retained { A(Data.VALUE),B(Data.VALUE); final String value; Retained(String v){value=v;trace.append(name());} }
 enum Folded { A(ConstantData.VALUE); Folded(String v){trace.append("F");} }
 public static void main(String[] args) {
 if(Retained.A.value!=Data.VALUE||Retained.B.value!=Data.VALUE||!trace.toString().equals("DAB"))throw new AssertionError(trace);
 Retained[] copy=Retained.values();copy[0]=null;if(Retained.values()[0]!=Retained.A)throw new AssertionError();
 if(Retained.valueOf("B")!=Retained.B)throw new AssertionError();
 if(Folded.A==null||!trace.toString().equals("DABF"))throw new AssertionError(trace);
 System.out.println("GETSTATIC preserves DAB identity; ConstantValue suppresses X");
 }
}"#).unwrap();
    let output = std::process::Command::new(jdk.join("javac"))
        .args(["--release", "25", "-d"])
        .arg(&root)
        .arg(root.join("Proof.java"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = std::process::Command::new(jdk.join("java"))
        .arg("-cp")
        .arg(&root)
        .arg("Proof")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
}
