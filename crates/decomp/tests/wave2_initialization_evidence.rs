//! Original evidence for the first six wave2 definite-assignment rejections.
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

// Independent diagnostic counts on the original register CFG. This proves
// normal-path field effects only, not Java source emission or runtime immutability.
fn normal_counts<F>(
    ir: &managed_ir::method::MethodIr<F>,
    finals: &[u32],
) -> std::collections::BTreeSet<Vec<u8>> {
    use managed_ir::insn::ManagedInsn;
    use std::collections::BTreeSet;
    let mut seen = BTreeSet::new();
    let mut returns = BTreeSet::new();
    let mut pending = vec![(ir.cfg().entry(), vec![0u8; finals.len()])];
    while let Some((block, mut counts)) = pending.pop() {
        assert!(seen.len() < 65_536, "bounded local path diagnostic");
        if !seen.insert((block, counts.clone())) {
            continue;
        }
        let node = ir.cfg().block(block).unwrap();
        match ir.instruction(*node.payload()).unwrap() {
            ManagedInsn::FieldPut {
                object: None,
                field,
                ..
            } => {
                if let Some(position) = finals.iter().position(|f| *f == field.index()) {
                    counts[position] = counts[position].saturating_add(1).min(2);
                }
            }
            ManagedInsn::Return { value: None } => {
                assert!(node.outgoing().is_empty());
                returns.insert(counts);
                continue;
            }
            ManagedInsn::Throw { .. } if node.outgoing().is_empty() => continue,
            _ => {}
        }
        for edge in node.outgoing() {
            let edge = ir.cfg().edge(*edge).unwrap();
            assert!(!matches!(
                edge.kind(),
                decomp_core::cfg::EdgeKind::Exception(_)
            ));
            pending.push((edge.target(), counts.clone()));
        }
    }
    returns
}

fn raw_counts(units: &[u16]) -> std::collections::BTreeSet<Vec<u8>> {
    let code = dex_format::code::CodeItem {
        registers_size: 1,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: dex_format::RawSpan { offset: 0, size: 0 },
    };
    let raw = dex_decompiler::DexPasses::run(&code).unwrap();
    normal_counts(raw.ir(), &[41])
}

fn static_sites<F>(
    ir: &managed_ir::method::MethodIr<F>,
    finals: &[u32],
    offset: impl Fn(managed_ir::value::InsnId) -> Option<u32>,
) -> std::collections::BTreeSet<(u32, u32)> {
    ir.instructions()
        .iter()
        .filter_map(|(id, instruction)| {
            if let managed_ir::insn::ManagedInsn::FieldPut {
                object: None,
                field,
                ..
            } = instruction
                && finals.contains(&field.index())
            {
                Some((field.index(), offset(id).unwrap()))
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn local_path_diagnostic_retains_missing_double_and_throw_paths() {
    use std::collections::BTreeSet;
    assert_eq!(
        raw_counts(&[0x0012, 0x0038, 5, 0x0069, 41, 0x0328, 0x0069, 41, 0x000e]),
        BTreeSet::from([vec![1]])
    );
    assert_eq!(
        raw_counts(&[
            0x0012, 0x0038, 5, 0x0069, 41, 0x0328, 0x0012, 0x0012, 0x000e
        ]),
        BTreeSet::from([vec![0], vec![1]])
    );
    assert_eq!(
        raw_counts(&[0x0012, 0x0069, 41, 0x0069, 41, 0x000e]),
        BTreeSet::from([vec![2]])
    );
    assert_eq!(
        raw_counts(&[0x0012, 0x0038, 5, 0x0069, 41, 0x0328, 0x0027, 0, 0x000e]),
        BTreeSet::from([vec![1]])
    );
    assert!(raw_counts(&[0x0012, 0x0027]).is_empty());
}

#[test]
#[ignore = "requires retained Samsung APK, Android35 metadata and JDK17"]
fn original_wave2_top_six_have_complete_stores_and_normal_final_paths() {
    use std::collections::BTreeSet;
    let inputs = dex_format::apk::parse_apk_path(
        "../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    let cp = Classpath::from_paths([
        java_home().join("jmods/java.base.jmod"),
        PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
    ])
    .unwrap();
    let legacy = WriteInventory::scan(&inputs);
    let inventory = WriteInventory::scan_with_metadata(&inputs, &cp);
    let hierarchy =
        decomp::java_hierarchy::build_with_metadata(&inputs.iter().collect::<Vec<_>>(), &cp)
            .unwrap();
    assert_eq!(
        hierarchy.assignable(
            &JavaType::Class("java.util.List".into()),
            &JavaType::Class("java.util.Collection".into()),
        ),
        backend_java::type_compatibility::Compatibility::Assignable,
    );
    assert_ne!(
        hierarchy.assignable(
            &JavaType::Class("java.util.Collection".into()),
            &JavaType::Class("java.util.List".into()),
        ),
        backend_java::type_compatibility::Compatibility::Assignable,
    );
    assert!(!inventory.has_structural_unknown());
    assert_eq!(
        inventory.unknown_observations(),
        legacy.unknown_observations()
    );
    assert!(!inventory.unknown_observations().is_empty());
    let dex = &inputs[0];
    for name in ["La2/a$d;", "La3/b;", "La4/a;", "La6/a;", "La6/f$d;"] {
        let class = dex
            .classes
            .iter()
            .find(|c| descriptor(dex, c.class_idx) == name)
            .unwrap();
        assert!(class.static_values.is_none());
        let data = class.class_data.as_ref().unwrap();
        let resolver = Resolver {
            inputs: &inputs,
            dex,
            class,
            cp: &cp,
            hierarchy: &hierarchy,
            super_proof: true,
        };
        for field in data
            .static_fields
            .iter()
            .chain(&data.instance_fields)
            .filter(|f| f.access_flags & 0x10 != 0)
        {
            let key = initializers::write_inventory::field_key(dex, field.field_idx).unwrap();
            let sites = inventory.explicit_store_writes(&key).unwrap();
            assert!(!sites.is_empty(), "{name} field{}", field.field_idx);
            assert!(
                inventory.proven_writes(&key).is_none(),
                "runtime capabilities remain"
            );
            let mut original = BTreeSet::new();
            for method in &data.direct_methods {
                let Some(code) = &method.code else { continue };
                for instruction in dex_decompiler::decode_method(code).unwrap() {
                    if let dex_decompiler::Opcode::Supported(opcode) = instruction.opcode
                        && ((0x59..=0x5f).contains(&opcode.opcode)
                            || (0x67..=0x6d).contains(&opcode.opcode))
                        && instruction.pool_index == Some(field.field_idx)
                    {
                        original.insert(initializers::write_inventory::WriteSite {
                            dex: 0,
                            method: method.method_idx,
                            offset: instruction.offset,
                            is_static: opcode.opcode >= 0x67,
                        });
                    }
                }
            }
            assert_eq!(sites, original, "{name} field{}", field.field_idx);
        }
        let instance_finals = data
            .instance_fields
            .iter()
            .filter(|f| f.access_flags & 0x10 != 0)
            .map(|f| FieldRef::new(f.field_idx))
            .collect::<Vec<_>>();
        for method in data.direct_methods.iter().filter(|m| {
            ["<init>", "<clinit>"].contains(&method_key(dex, m.method_idx).name.as_str())
        }) {
            let code = method.code.as_ref().unwrap();
            let ssa = dex_decompiler::DexPasses::run_ssa_with(
                code,
                &dex_decompiler::DexTypes::new(dex, Some(method.method_idx)),
            )
            .unwrap();
            if method_key(dex, method.method_idx).name == "<clinit>" {
                let finals = data
                    .static_fields
                    .iter()
                    .filter(|f| f.access_flags & 0x10 != 0)
                    .map(|f| f.field_idx)
                    .collect::<Vec<_>>();
                let raw = dex_decompiler::DexPasses::run_with(
                    code,
                    &dex_decompiler::DexTypes::new(dex, Some(method.method_idx)),
                )
                .unwrap();
                assert_eq!(
                    static_sites(raw.ir(), &finals, |id| raw.offset(id)),
                    static_sites(ssa.ir(), &finals, |id| ssa.offset(id)),
                    "exact original field/offset correspondence method{}",
                    method.method_idx,
                );
                assert_eq!(
                    normal_counts(raw.ir(), &finals),
                    BTreeSet::from([vec![1; finals.len()]]),
                    "original register CFG method{}",
                    method.method_idx,
                );
                // This original typed filled-array path is required. A
                // NoTypes lift cannot attest its exact element descriptor.
                if method.method_idx == 133 {
                    assert!(matches!(
                        dex_decompiler::DexPasses::run(code),
                        Err(dex_decompiler::LiftError::InvalidFilledArray {
                            offset: 29,
                            type_index: 11506,
                            ..
                        }),
                    ));
                }
                assert_eq!(
                    normal_counts(ssa.ir(), &finals),
                    BTreeSet::from([vec![1; finals.len()]]),
                    "method{}",
                    method.method_idx
                );
                let context = backend_java::static_initializer::StaticInitializerContext {
                    owner: source_name(name),
                    writable_static_fields: data
                        .static_fields
                        .iter()
                        .filter(|f| f.access_flags & 0x10 == 0)
                        .map(|f| resolver.field(FieldRef::new(f.field_idx)).unwrap())
                        .collect(),
                };
                let emission =
                    backend_java::static_initializer::emit_static_initializer_body_subset(
                        ssa.ir(),
                        &resolver.method(MethodRef::new(method.method_idx)).unwrap(),
                        &backend_java::ParameterBindings::default(),
                        &resolver,
                        &context,
                        &finals.iter().map(|f| FieldRef::new(*f)).collect::<Vec<_>>(),
                    );
                if method.method_idx == 133 {
                    assert!(
                        emission.is_ok(),
                        "original static133 must retain its recovered capture and interface arguments: {emission:?}",
                    );
                }
                // Recovery may advance independently. A retained refusal must
                // remain the evidenced source-grammar failure, not a fake DA proof.
                if let Err(error) = emission {
                    let expected = match method.method_idx {
                        18 => backend_java::Unsupported::Instruction(
                            "allocation may throw OOM before initializer".into(),
                        ),
                        73 => backend_java::Unsupported::ControlFlow(
                            "unsupported static initializer branch tree".into(),
                        ),
                        187 => backend_java::Unsupported::Metadata(
                            "unresolved instruction type".into(),
                        ),
                        _ => panic!("unexpected original static initializer"),
                    };
                    assert_eq!(format!("{error:?}"), format!("{expected:?}"));
                }
                continue;
            }
            let receiver_register = code.registers_size - code.ins_size;
            let receiver = ssa
                .initial_registers()
                .iter()
                .find(|(_, registers)| registers.contains(&receiver_register))
                .map(|(value, _)| *value);
            let evidence =
                backend_java::constructors::constructor_receiver_initialization_evidence(
                    ssa.ir(),
                    MethodRef::new(method.method_idx),
                    receiver,
                    &source_name(descriptor(dex, class.superclass_idx)),
                    &instance_finals,
                    &resolver,
                )
                .unwrap();
            let (kind, target, count) = match method.method_idx {
                19 | 135 => (backend_java::constructors::ChainKind::Super, 46672, 1),
                134 => (backend_java::constructors::ChainKind::This, 135, 0),
                312 | 313 => (backend_java::constructors::ChainKind::Super, 336, 1),
                _ => panic!("unexpected original constructor"),
            };
            assert_eq!(
                evidence.normal_chains,
                std::collections::HashSet::from([(kind, MethodRef::new(target))])
            );
            for field in evidence.fields {
                assert!(field.before_chain.is_empty());
                assert!(!field.read_before_assignment);
                assert_eq!(field.normal_return_counts, BTreeSet::from([count]));
                assert_eq!(field.after_chain.len(), usize::from(count));
            }
        }
    }
}

#[test]
#[ignore = "requires retained Samsung APK, Android35 metadata and JDK17"]
fn original_static73_and8201_phi_joins_have_attested_source() {
    use std::collections::BTreeSet;
    let inputs = dex_format::apk::parse_apk_path(
        "../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    let cp = Classpath::from_paths([
        java_home().join("jmods/java.base.jmod"),
        PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
    ])
    .unwrap();
    let inventory = WriteInventory::scan_with_metadata(&inputs, &cp);
    let hierarchy =
        decomp::java_hierarchy::build_with_metadata(&inputs.iter().collect::<Vec<_>>(), &cp)
            .unwrap();
    let dex = &inputs[0];
    for (owner, index, field, offset, name) in [
        ("La3/b;", 73, 61, 38, "a"),
        ("Landroidx/appcompat/widget/j0;", 8201, 2252, 12, "I"),
    ] {
        let class = dex
            .classes
            .iter()
            .find(|c| descriptor(dex, c.class_idx) == owner)
            .unwrap();
        let method = class
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods
            .iter()
            .find(|m| m.method_idx == index)
            .unwrap();
        let ssa = dex_decompiler::DexPasses::run_ssa_with(
            method.code.as_ref().unwrap(),
            &dex_decompiler::DexTypes::new(dex, Some(index)),
        )
        .unwrap();
        let resolver = Resolver {
            inputs: &inputs,
            dex,
            class,
            cp: &cp,
            hierarchy: &hierarchy,
            super_proof: true,
        };
        let proof = initializers::plan_static_initializer_explicit_stores_attested(
            &inputs,
            0,
            class,
            MethodRef::new(index),
            &ssa,
            &resolver,
            &inventory,
        )
        .unwrap();
        assert_eq!(
            proof.plan.encoded.blank_static_finals,
            BTreeSet::from([field])
        );
        assert!(proof.plan.encoded.prefix_block.is_empty());
        assert_eq!(proof.plan.source.matches(&format!("{name} = ")).count(), 1);
        let key = initializers::write_inventory::field_key(dex, field).unwrap();
        assert_eq!(
            inventory.explicit_store_writes(&key).unwrap(),
            BTreeSet::from([initializers::write_inventory::WriteSite {
                dex: 0,
                method: index,
                offset,
                is_static: true
            }])
        );
        assert!(!proof.runtime_capabilities.is_empty());
        let dir =
            std::env::temp_dir().join(format!("original-static-joins-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{index}.java")), &proof.plan.source).unwrap();
        // Original Android/reflection dependencies remain unchanged. These are
        // attested source subsets, not claims of a compiled Android closure.
        if index == 73 {
            assert!(proof.plan.source.contains("kotlin.jvm.internal.n.a"));
        }
        if index == 8201 {
            assert!(proof.plan.source.contains("e5.a.a()"));
        }
    }
}
