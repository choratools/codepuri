//! Original constructor identity, store and suffix-Phi evidence; bounded ignored corpus probe.
#![allow(clippy::unwrap_used)]
use backend_java::constructors::{
    ConstructorClass, ConstructorResolver, Initializer, TrustedConstructorResolver,
};
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType};
use decomp::java_class_kinds::initializers::{self, write_inventory::WriteInventory};
use decomp::java_external_metadata::{Classpath, ExternalMetadata, MethodMetadata};
use dex_format::{DexFile, class::ClassDef};
use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
use std::collections::{BTreeSet, HashMap};
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

fn bindings(
    ssa: &dex_decompiler::SsaMethod,
    m: &JavaMethod,
    code: &dex_format::code::CodeItem,
) -> backend_java::DeclarationBindings {
    let provenance = ssa
        .initial_registers()
        .iter()
        .map(
            |(value, registers)| decomp::java_initial_parameters::InitialProvenance {
                value: *value,
                registers: registers.clone(),
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
    let bound = |p| match p {
        decomp::java_initial_parameters::InitialParameter::Bound(v) => Some(v),
        _ => None,
    };
    backend_java::DeclarationBindings {
        receiver: p.receiver.and_then(bound),
        parameters: p.parameters.into_iter().map(bound).collect(),
        ..Default::default()
    }
}
#[test]
#[ignore = "retained SamsungPass APK, genuine Java25+Android35 metadata; bounded exact original family"]
fn original_a4_135_final_paths_and_four_edge_phi_retain_original_checked_contract() {
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
    assert!(!inventory.has_structural_unknown());
    assert!(!inventory.unknown_observations().is_empty());
    let dex = &inputs[0];
    let key = method_key(dex, 135);
    assert_eq!(key.owner, "La4/a;");
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == key.owner)
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
    let mut ssas = Vec::new();
    let mut bound = Vec::new();
    let mut methods = Vec::new();
    for index in [133, 134, 135] {
        let m = data
            .direct_methods
            .iter()
            .find(|m| m.method_idx == index)
            .unwrap();
        let code = m.code.as_ref().unwrap();
        assert!(code.tries.is_empty());
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
        let ssa =
            dex_decompiler::DexPasses::run_ssa_with(code, &context.with_initial_registers(seeds))
                .unwrap();
        let signature = r.method(MethodRef::new(index)).unwrap();
        bound.push(bindings(&ssa, &signature, code));
        ssas.push(ssa);
        methods.push(index);
    }
    let constructors = methods
        .iter()
        .enumerate()
        .skip(1)
        .map(
            |(position, index)| backend_java::constructors::ConstructorInput {
                reference: MethodRef::new(*index),
                ir: ssas[position].ir(),
                bindings: &bound[position],
            },
        )
        .collect::<Vec<_>>();
    let proof = initializers::prove_instance_final_explicit_stores(
        &inputs,
        0,
        class,
        &BTreeSet::from([134, 135]),
        &inventory,
    )
    .unwrap();
    assert_eq!(proof.plan.len(), 1);
    assert_eq!(proof.plan[0].reference, FieldRef::new(117));
    assert_eq!(proof.plan[0].writes.get(&MethodRef::new(135)), Some(&1));
    let store_key = initializers::write_inventory::field_key(dex, 117).unwrap();
    let sites = inventory.explicit_store_writes(&store_key).unwrap();
    assert_eq!(sites.len(), 1);
    let site = sites.iter().next().unwrap();
    assert_eq!(
        (site.dex, site.method, site.offset, site.is_static),
        (0, 135, 8, false)
    );
    let cfg = ssas[2].ir().cfg();
    let head = cfg
        .block_ids()
        .find(|block| {
            let node = cfg.block(*block).unwrap();
            matches!(
                ssas[2].ir().instruction(*node.payload()).unwrap(),
                managed_ir::insn::ManagedInsn::Phi { .. }
            ) && node.incoming().len() == 4
        })
        .unwrap();
    let node = cfg.block(head).unwrap();
    let managed_ir::insn::ManagedInsn::Phi {
        inputs: phi_inputs, ..
    } = ssas[2].ir().instruction(*node.payload()).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(phi_inputs.len(), 4);
    assert_eq!(
        phi_inputs
            .iter()
            .map(|(edge, _)| *edge)
            .collect::<std::collections::HashSet<_>>(),
        node.incoming().iter().copied().collect()
    );
    let evidence = backend_java::constructors::constructor_initialization_evidence(
        &constructors[1],
        "java.lang.Object",
        &[FieldRef::new(117)],
        &r,
    )
    .unwrap();
    assert_eq!(evidence.fields[0].normal_return_counts, BTreeSet::from([1]));
    assert!(!evidence.fields[0].read_before_assignment);
    for (id, instruction) in ssas[2].ir().instructions() {
        if let managed_ir::insn::ManagedInsn::Invoke { method, .. } = instruction {
            let target = r.method(*method).unwrap();
            println!(
                "ORIGINAL invoke#{} offset{:?} {:?} throws{:?}",
                method.index(),
                ssas[2].offset(id),
                method_key(dex, method.index()),
                target.throws
            );
        }
    }
    println!(
        "ORIGINAL constructor135 throws{:?}",
        r.method(MethodRef::new(135)).unwrap().throws
    );
    let planned =
        backend_java::constructors::plan_constructor_family("a4.a", &constructors, &proof.plan, &r);
    let plans = match planned {
        Ok(plans) => plans,
        Err(error) => {
            assert_eq!(
                error,
                backend_java::Unsupported::Metadata(
                    "invoke checked exceptions are not declared by caller".into()
                )
            );
            assert!(
                initializers::plan_class_initialization_with_explicit_stores_attested(
                    &inputs,
                    0,
                    class,
                    &constructors,
                    Some((MethodRef::new(133), &ssas[0])),
                    &r,
                    &inventory
                )
                .is_err()
            );
            println!(
                "PROVEN original classes.dex#135 field117 sole store@8 normalcount1 and four original Phi edges; grammar/typing barrier cleared; original-only Throws resolver retains checked-call denial; NO complete body/family claim; unknown={}",
                inventory.unknown_observations().len()
            );
            return;
        }
    };
    assert_eq!(plans.len(), 2);
    let atomic = initializers::plan_class_initialization_with_explicit_stores_attested(
        &inputs,
        0,
        class,
        &constructors,
        Some((MethodRef::new(133), &ssas[0])),
        &r,
        &inventory,
    )
    .unwrap();
    assert_eq!(
        atomic.runtime_capabilities.len(),
        inventory.unknown_observations().len()
    );
    let out = std::env::var_os("DECOMPHIFY_EVIDENCE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/opencode/wave12-constructor-original"));
    std::fs::create_dir_all(&out).unwrap();
    for (index, plan) in [134, 135].into_iter().zip(&plans) {
        std::fs::write(out.join(format!("original-{index}.java.txt")), &plan.source).unwrap();
    }
    println!(
        "COMPLETE original classes.dex#135 family134+135 clinit133 field117 originalstore@8 normalcount1 Phi4 exactedges; retainedunknown={}",
        atomic.runtime_capabilities.len()
    );
}
