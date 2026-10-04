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
#[ignore = "original Samsung APK, genuine complete input metadata"]
fn original_private_protobuf_payloads() {
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
    let dex = &inputs[1];
    for (owner, ctor_idx, init_idx) in [
        ("Lcom/google/protobuf/DescriptorProtos$Edition;", 6243, 6242),
        ("Lcom/google/protobuf/Field$Cardinality;", 10388, 10387),
    ] {
        let class = dex
            .classes
            .iter()
            .find(|c| descriptor(dex, c.class_idx) == owner)
            .unwrap();
        let r = Resolver {
            inputs: &inputs,
            dex,
            class,
            cp: &cp,
            hierarchy: &hierarchy,
            super_proof: true,
        };
        let ctor = original_ssa(dex, class, ctor_idx);
        let init = original_ssa(dex, class, init_idx);
        let method = r.method(MethodRef::new(ctor_idx)).unwrap();
        let code = class
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods
            .iter()
            .find(|m| m.method_idx == ctor_idx)
            .unwrap()
            .code
            .as_ref()
            .unwrap();
        let params = bindings(&ctor, &method, code);
        let cs = [EnumConstructorDeclarationInput {
            reference: MethodRef::new(ctor_idx),
            ir: ctor.ir(),
            bindings: &params,
        }];
        let result = e::plan_enum_initialization_family_with_prepared_values(
            &inputs,
            1,
            class,
            &cs,
            MethodRef::new(init_idx),
            init.ir(),
            &r,
            &inventory,
        );
        let plan = result.unwrap();
        let data = class.class_data.as_ref().unwrap();
        let zero = data
            .static_fields
            .iter()
            .find(|f| {
                dex.string(dex.field_id(f.field_idx).unwrap().name_idx)
                    .unwrap()
                    .ends_with("UNKNOWN_VALUE")
            })
            .unwrap();
        assert_eq!(
            plan.encoded_declaration_initializers
                .get(&zero.field_idx)
                .map(String::as_str),
            Some("0")
        );
        assert_eq!(
            data.direct_methods
                .iter()
                .filter(|m| method_key(dex, m.method_idx).name == "<init>")
                .map(|m| m.access_flags)
                .collect::<Vec<_>>(),
            [0x10002]
        );
        let methods = e::emission::plan_enum_methods(dex, class, &plan).unwrap();
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
            let mut source = match backend_java::emit_method_declaration(ssa.ir(), &sig, &bound, &r)
            {
                backend_java::SourceResult::Complete { source, .. } => source,
                other => panic!("method {}: {other:?}", m.method_idx),
            };
            if m.access_flags & 0x10 != 0 {
                source = source.replacen("public ", "public final ", 1);
            }
            members.insert(m.method_idx, source);
        }
        let source =
            e::emission::emit_enum_declaration(class, &plan, &methods, &fields, &members).unwrap();
        let root = PathBuf::from(format!(
            "/tmp/opencode/wave27-private-enum-original-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let simple = source_name(owner).rsplit('.').next().unwrap().to_owned();
        std::fs::write(
            root.join(format!("{simple}.java")),
            format!("package com.google.protobuf;\n{source}\n"),
        )
        .unwrap();
        println!(
            "ORIGINAL_PRIVATE_ENUM {owner} source={} ctor={} zero={} suffix={:?}",
            root.display(),
            ctor_idx,
            zero.field_idx,
            plan.static_initializer_source
        );
    }
    let root = PathBuf::from(format!(
        "/tmp/opencode/wave27-private-enum-original-{}",
        std::process::id()
    ));
    let source_dependencies =
        PathBuf::from("/tmp/opencode/samsung-pass-100-wave26/java25/sources/com/google/protobuf");
    for name in [
        "Internal$EnumLite",
        "Internal$EnumLiteMap",
        "Internal$EnumVerifier",
        "DescriptorProtos$Edition$1",
        "DescriptorProtos$Edition$EditionVerifier",
        "Field$Cardinality$1",
        "Field$Cardinality$CardinalityVerifier",
    ] {
        std::fs::copy(
            source_dependencies.join(format!("{name}.java")),
            root.join(format!("{name}.java")),
        )
        .unwrap();
    }
    std::fs::write(root.join("Check.java"), ORIGINAL_CHECK).unwrap();
    let jdk = PathBuf::from("/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1/bin");
    let mut sources = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "java"))
        .collect::<Vec<_>>();
    sources.sort();
    let output = std::process::Command::new(jdk.join("javac"))
        .args(["--release", "25", "-d"])
        .arg(root.join("classes"))
        .args(&sources)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "javac: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = std::process::Command::new(jdk.join("java"))
        .arg("-cp")
        .arg(root.join("classes"))
        .arg("com.google.protobuf.Check")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "java: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    // Mutated-input negative, not a claim about original Samsung execution:
    // a real GETSTATIC makes a constant-variable source initializer unsound.
    let mut changed = inputs.clone();
    let c = changed[1]
        .classes
        .iter_mut()
        .find(|c| descriptor(dex, c.class_idx) == "Lcom/google/protobuf/Field$Cardinality;")
        .unwrap();
    let class_idx = c.class_idx;
    let get_number = c
        .class_data
        .as_mut()
        .unwrap()
        .virtual_methods
        .iter_mut()
        .find(|m| m.method_idx == 10390)
        .unwrap();
    let code = get_number.code.as_mut().unwrap();
    let instruction = dex_decompiler::decode_method(code)
        .unwrap()
        .into_iter()
        .find(|i| matches!(i.opcode,dex_decompiler::Opcode::Supported(info) if info.name=="iget"))
        .unwrap();
    let at = instruction.offset as usize * 2;
    code.insns[at..at + 2].copy_from_slice(&0x0060u16.to_le_bytes());
    code.insns[at + 2..at + 4].copy_from_slice(&1812u16.to_le_bytes());
    let changed_inventory = WriteInventory::scan_with_metadata(&changed, &cp);
    let changed_dex = &changed[1];
    let class = changed_dex
        .classes
        .iter()
        .find(|c| c.class_idx == class_idx)
        .unwrap();
    let key = decomp::java_class_kinds::initializers::write_inventory::field_key(changed_dex, 1812)
        .unwrap();
    assert!(!changed_inventory.known_reads(&key).unwrap().is_empty());
    let r = Resolver {
        inputs: &changed,
        dex: changed_dex,
        class,
        cp: &cp,
        hierarchy: &hierarchy,
        super_proof: true,
    };
    let ctor = original_ssa(changed_dex, class, 10388);
    let init = original_ssa(changed_dex, class, 10387);
    let sig = r.method(MethodRef::new(10388)).unwrap();
    let ctor_code = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|m| m.method_idx == 10388)
        .unwrap()
        .code
        .as_ref()
        .unwrap();
    let bound = bindings(&ctor, &sig, ctor_code);
    let cs = [EnumConstructorDeclarationInput {
        reference: MethodRef::new(10388),
        ir: ctor.ir(),
        bindings: &bound,
    }];
    assert!(
        e::plan_enum_initialization_family_with_prepared_values(
            &changed,
            1,
            class,
            &cs,
            MethodRef::new(10387),
            init.ir(),
            &r,
            &changed_inventory
        )
        .is_err()
    );
}

const ORIGINAL_CHECK: &str = r#"package com.google.protobuf;
public class Check { public static void main(String[] args) throws Exception {
 int[] en={0,900,998,999,1000,1001,1,2,99997,99998,99999,Integer.MAX_VALUE};
 DescriptorProtos$Edition[] es=DescriptorProtos$Edition.values();
 for(int i=0;i<es.length;i++){if(es[i].ordinal()!=i||es[i].getNumber()!=en[i]||DescriptorProtos$Edition.valueOf(es[i].name())!=es[i]||DescriptorProtos$Edition.internalGetValueMap().findValueByNumber(en[i])!=es[i])throw new AssertionError();}
 int[] cn={0,1,2,3,-1};Field$Cardinality[] cs=Field$Cardinality.values();
 for(int i=0;i<cs.length;i++){if(cs[i].ordinal()!=i||Field$Cardinality.valueOf(cs[i].name())!=cs[i])throw new AssertionError();if(i<4&&cs[i].getNumber()!=cn[i])throw new AssertionError();}
 try{Field$Cardinality.UNRECOGNIZED.getNumber();throw new AssertionError();}catch(IllegalArgumentException expected){}
 if(DescriptorProtos$Edition.internalGetValueMap()!=DescriptorProtos$Edition.internalGetValueMap()||Field$Cardinality.internalGetValueMap()!=Field$Cardinality.internalGetValueMap())throw new AssertionError();
 if(DescriptorProtos$Edition.internalGetValueMap().findValueByNumber(-42)!=null||Field$Cardinality.internalGetValueMap().findValueByNumber(-42)!=null)throw new AssertionError();
 es[0]=null;cs[0]=null;if(DescriptorProtos$Edition.values()[0]!=DescriptorProtos$Edition.EDITION_UNKNOWN||Field$Cardinality.values()[0]!=Field$Cardinality.CARDINALITY_UNKNOWN)throw new AssertionError();
 if(DescriptorProtos$Edition.EDITION_UNKNOWN_VALUE!=0||Field$Cardinality.CARDINALITY_UNKNOWN_VALUE!=0)throw new AssertionError();
 if(!java.lang.reflect.Modifier.isPrivate(DescriptorProtos$Edition.class.getDeclaredConstructor(String.class,int.class,int.class).getModifiers())||!java.lang.reflect.Modifier.isPrivate(Field$Cardinality.class.getDeclaredConstructor(String.class,int.class,int.class).getModifiers()))throw new AssertionError();
 System.out.println("original payloads/order/private flags/mapper identity/values clone/implicit defaults OK");
}}
"#;
