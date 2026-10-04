//! Actual wave17 initializer rejection attribution without relaxing source proofs.
#![allow(clippy::unwrap_used)]
use backend_java::constructors::{
    ConstructorClass, ConstructorResolver, Initializer, TrustedConstructorResolver,
};
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType};
use decomp::java_class_kinds::initializers::{self, write_inventory::WriteInventory};
use decomp::java_external_metadata::{Classpath, ExternalMetadata, MethodMetadata};
use dex_format::{DexFile, class::ClassDef};
use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
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
        match match reference.index() {
            0x8000_0081 | 0x8000_0088 | 0x8000_008b => Some(JavaType::Long),
            0x8000_0082 | 0x8000_0085 | 0x8000_008c => Some(JavaType::Float),
            0x8000_0083 | 0x8000_0086 | 0x8000_0089 => Some(JavaType::Double),
            0x8000_0080 | 0x8000_0084 | 0x8000_0087 | 0x8000_008a => Some(JavaType::Int),
            0x8000_008d => Some(JavaType::Byte),
            0x8000_008e => Some(JavaType::Char),
            0x8000_008f => Some(JavaType::Short),
            index if index < 0x8000_0000 => {
                decomp::dex_java::descriptor_type(descriptor(self.dex, index))
            }
            _ => None,
        }? {
            JavaType::Class(name) => Some(name),
            _ => None,
        }
    }
    fn instruction_type(&self, reference: managed_ir::refs::TypeRef) -> Option<JavaType> {
        match reference.index() {
            0x8000_0081 | 0x8000_0088 | 0x8000_008b => Some(JavaType::Long),
            0x8000_0082 | 0x8000_0085 | 0x8000_008c => Some(JavaType::Float),
            0x8000_0083 | 0x8000_0086 | 0x8000_0089 => Some(JavaType::Double),
            0x8000_0080 | 0x8000_0084 | 0x8000_0087 | 0x8000_008a => Some(JavaType::Int),
            0x8000_008d => Some(JavaType::Byte),
            0x8000_008e => Some(JavaType::Char),
            0x8000_008f => Some(JavaType::Short),
            index if index < 0x8000_0000 => {
                decomp::dex_java::descriptor_type(descriptor(self.dex, index))
            }
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
            .and_then(|code| {
                let receiver = code.registers_size - code.ins_size;
                dex_decompiler::decode_method(code)
                    .ok()?
                    .into_iter()
                    .filter(|i| i.registers.first() == Some(&receiver))
                    .find_map(|i| {
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

#[test]
#[ignore = "requires retained original Samsung APK and genuine Android35 metadata"]
fn actual_constructor_failure_attribution() {
    use dex_decompiler::infer::TypeContext;
    use std::collections::{BTreeSet, HashMap};
    let inputs = dex_format::apk::parse_apk_path(
        "../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    let cp = Classpath::from_paths([
        java_home().join("jmods/java.base.jmod"),
        PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
    ])
    .unwrap();
    let hierarchy =
        decomp::java_hierarchy::build_with_metadata(&inputs.iter().collect::<Vec<_>>(), &cp)
            .unwrap();
    let inventory = WriteInventory::scan_with_metadata(&inputs, &cp);
    let dex = &inputs[0];
    for index in [313, 2803, 6866] {
        let key = method_key(dex, index);
        let class = dex
            .classes
            .iter()
            .find(|c| descriptor(dex, c.class_idx) == key.owner)
            .unwrap();
        let r = Resolver {
            inputs: &inputs,
            dex,
            class,
            cp: &cp,
            hierarchy: &hierarchy,
            super_proof: true,
        };
        let data = class.class_data.as_ref().unwrap();
        let constructors = data
            .direct_methods
            .iter()
            .filter(|m| method_key(dex, m.method_idx).name == "<init>")
            .map(|m| m.method_idx)
            .collect::<BTreeSet<_>>();
        let store = initializers::prove_instance_final_explicit_stores(
            &inputs,
            0,
            class,
            &constructors,
            &inventory,
        );
        assert!(
            store.is_ok(),
            "original stores must not be misclassified as unavailable"
        );
        println!(
            "CASE#{index} {key:?} STORE {:?}",
            store.as_ref().map(|p| p.plan.len())
        );
        for original in data
            .direct_methods
            .iter()
            .filter(|m| constructors.contains(&m.method_idx))
        {
            let code = original.code.as_ref().unwrap();
            let context = dex_decompiler::DexTypes::new(dex, Some(original.method_idx));
            let mut seeds = HashMap::new();
            let mut word = code.registers_size - code.ins_size;
            seeds.insert((word, false), context.type_at(class.class_idx).unwrap());
            word += 1;
            let mut params = vec![];
            assert!(context.method_params(original.method_idx, &mut params));
            for ty in params {
                let wide = matches!(
                    ty,
                    managed_ir::types::ManagedType::Long | managed_ir::types::ManagedType::Double
                );
                seeds.insert((word, wide), ty);
                word += if wide { 2 } else { 1 };
            }
            let ssa = dex_decompiler::DexPasses::run_ssa_with(
                code,
                &context.with_initial_registers(seeds),
            )
            .unwrap();
            println!(
                "ORIGINAL#{} FLOW {:?}",
                original.method_idx,
                decomp_structure::extended::structure_flow(ssa.ir().cfg())
                    .as_ref()
                    .map(|_| "supported")
            );
            let finals = store
                .as_ref()
                .unwrap()
                .plan
                .iter()
                .map(|field| field.reference)
                .collect::<Vec<_>>();
            let evidence =
                backend_java::constructors::constructor_receiver_initialization_evidence(
                    ssa.ir(),
                    MethodRef::new(original.method_idx),
                    Some(
                        *ssa.initial_registers()
                            .iter()
                            .find(|(_, words)| {
                                words.contains(&(code.registers_size - code.ins_size))
                            })
                            .unwrap()
                            .0,
                    ),
                    &source_name(descriptor(dex, class.superclass_idx)),
                    &finals,
                    &r,
                )
                .unwrap();
            for field in &evidence.fields {
                assert_eq!(field.normal_return_counts, BTreeSet::from([1]));
                assert!(!field.read_before_assignment);
            }
            let reference = MethodRef::new(original.method_idx);
            let method = r.method(reference).unwrap();
            let b = bindings(&ssa, &method, code);
            let result = backend_java::constructors::plan_constructor_declaration(
                ssa.ir(),
                reference,
                &source_name(descriptor(dex, class.superclass_idx)),
                &b,
                &r,
            );
            if original.method_idx == 6866 {
                assert!(
                    result.is_ok(),
                    "original private-direct suffix must emit: {result:?}"
                );
                assert_eq!(constructors, BTreeSet::from([6866]));
                let family = [backend_java::constructors::ConstructorInput {
                    reference,
                    ir: ssa.ir(),
                    bindings: &b,
                }];
                let atomic = initializers::plan_class_initialization_with_explicit_stores(
                    &inputs, 0, class, &family, None, &r, &inventory,
                )
                .unwrap();
                assert_eq!(atomic.plan.constructors.len(), 1);
                assert!(
                    !atomic.runtime_capabilities.is_empty(),
                    "runtime unknowns remain explicit"
                );
            }
            if original.method_idx == 313 {
                assert!(
                    matches!(&result,Err(backend_java::Unsupported::ControlFlow(reason)) if reason.contains("OverlappingRegion"))
                );
            }
            if original.method_idx == 2803 {
                assert!(
                    matches!(&result,Err(backend_java::Unsupported::Instruction(reason)) if reason.contains("before initializer"))
                );
            }
            if let Ok(plan) = &result {
                std::fs::write(
                    format!(
                        "/tmp/opencode/wave17-constructor-{}.java",
                        original.method_idx
                    ),
                    &plan.source,
                )
                .unwrap();
            }
            println!(
                "ORIGINAL#{} LOWER {:?}",
                original.method_idx,
                result.as_ref().map(|_| "accepted")
            );
        }
    }
}
