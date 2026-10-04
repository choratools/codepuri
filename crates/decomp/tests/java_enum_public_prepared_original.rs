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
#[ignore = "authorized original Samsung APK, genuine frozen dependencies, scratch-only enum preparation proof"]
fn original_export_failure_state_public_prepared_literals() {
    use decomp::java_class_kinds::enum_initialization::{
        self as enum_init, EnumConstructorDeclarationInput,
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
    let class = dex
        .classes
        .iter()
        .find(|class| {
            descriptor(dex, class.class_idx)
                == "Lcom/samsung/android/samsungpass/ExportFailureState;"
        })
        .unwrap();
    let data = class.class_data.as_ref().unwrap();
    let resolver = Resolver {
        inputs: &inputs,
        dex,
        class,
        cp: &cp,
        hierarchy: &hierarchy,
        super_proof: true,
    };
    println!(
        "OWNER flags={:#x} inventory_structural_unknown={} methods={}",
        class.access_flags,
        inventory.has_structural_unknown(),
        inventory.scanned_methods()
    );
    for method in data.direct_methods.iter().chain(&data.virtual_methods) {
        println!(
            "ORIGINAL {} {:?} flags={:#x} code={}",
            method.method_idx,
            method_key(dex, method.method_idx),
            method.access_flags,
            method.code.is_some()
        );
    }
    for field in &data.static_fields {
        let key = decomp::java_class_kinds::initializers::write_inventory::field_key(
            dex,
            field.field_idx,
        )
        .unwrap();
        println!(
            "FIELD {} {key:?} flags={:#x} positive_reads={:?} stores={:?}",
            field.field_idx,
            field.access_flags,
            inventory.known_reads(&key),
            inventory.explicit_store_writes(&key)
        );
    }
    let ctor = original_ssa(dex, class, 19538);
    let clinit = original_ssa(dex, class, 19537);
    let ctor_method = resolver.method(MethodRef::new(19538)).unwrap();
    let ctor_code = data
        .direct_methods
        .iter()
        .find(|method| method.method_idx == 19538)
        .unwrap()
        .code
        .as_ref()
        .unwrap();
    let ctor_bindings = bindings(&ctor, &ctor_method, ctor_code);
    let constructors = [EnumConstructorDeclarationInput {
        reference: MethodRef::new(19538),
        ir: ctor.ir(),
        bindings: &ctor_bindings,
    }];
    let plan = enum_init::plan_enum_initialization_family_with_prepared_values(
        &inputs,
        1,
        class,
        &constructors,
        MethodRef::new(19537),
        clinit.ir(),
        &resolver,
        &inventory,
    );
    println!("PLAN {plan:?}");
    if std::env::var_os("ENUM_EXPECT_OLD_REJECTION").is_some() {
        assert!(plan.is_err());
        return;
    }
    let plan = plan.unwrap();
    assert!(!plan.encoded_declaration_initializers.is_empty());
    let prepared: std::collections::BTreeSet<_> = plan
        .encoded_declaration_initializers
        .keys()
        .copied()
        .collect();
    let prepared_keys: std::collections::BTreeSet<_> = prepared
        .iter()
        .map(|field| {
            decomp::java_class_kinds::initializers::write_inventory::field_key(dex, *field).unwrap()
        })
        .collect();
    let mut raw_accesses = Vec::new();
    for (pool, input) in inputs.iter().enumerate() {
        for owner in &input.classes {
            let Some(members) = &owner.class_data else {
                continue;
            };
            for method in members
                .direct_methods
                .iter()
                .chain(&members.virtual_methods)
            {
                let Some(code) = &method.code else { continue };
                for instruction in dex_decompiler::decode_method(code).unwrap() {
                    let dex_decompiler::Opcode::Supported(info) = instruction.opcode else {
                        continue;
                    };
                    if !(0x52..=0x6d).contains(&info.opcode) {
                        continue;
                    }
                    if instruction
                        .pool_index
                        .and_then(|index| {
                            decomp::java_class_kinds::initializers::write_inventory::field_key(
                                input, index,
                            )
                        })
                        .is_some_and(|key| prepared_keys.contains(&key))
                    {
                        raw_accesses.push((
                            pool,
                            method.method_idx,
                            instruction.offset,
                            info.opcode,
                        ));
                    }
                }
            }
        }
    }
    assert!(
        raw_accesses.is_empty(),
        "all original raw reads/stores {raw_accesses:?}"
    );
    println!("FULL_RAW_READ_STORE_ABSENCE targets={prepared_keys:?}; accesses={raw_accesses:?}");
    let methods = enum_init::emission::plan_enum_methods(dex, class, &plan).unwrap();
    let fields = data
        .static_fields
        .iter()
        .chain(&data.instance_fields)
        .filter(|field| {
            field.field_idx != plan.absorbed_values_field
                && !plan
                    .constants
                    .iter()
                    .any(|constant| constant.field_idx == field.field_idx)
        })
        .map(|field| {
            (
                field.field_idx,
                decomp::java_members::MemberResolver { dex }
                    .field_declaration_without_initialization(field.field_idx)
                    .unwrap(),
            )
        })
        .collect();
    let mut members = std::collections::BTreeMap::new();
    for method in data.direct_methods.iter().chain(&data.virtual_methods) {
        if plan.absorbed_constructors.contains(&method.method_idx)
            || [plan.absorbed_clinit, methods.values, methods.value_of].contains(&method.method_idx)
            || Some(method.method_idx) == plan.absorbed_values_helper
        {
            continue;
        }
        let ssa = original_ssa(dex, class, method.method_idx);
        enum_init::emission::validate_enum_member_ir(class, &plan, ssa.ir()).unwrap();
        let declaration = resolver.method(MethodRef::new(method.method_idx)).unwrap();
        let bound = bindings(&ssa, &declaration, method.code.as_ref().unwrap());
        let mut source = match backend_java::emit_method_declaration(
            ssa.ir(),
            &declaration,
            &bound,
            &resolver,
        ) {
            backend_java::SourceResult::Complete { source, .. } => source,
            other => panic!("original method {}: {other:?}", method.method_idx),
        };
        // Mirror the facade's original declaration modifiers, rather than the
        // backend helper's public method default. Original getNumber is final.
        if method.access_flags & 0x10 != 0 {
            assert!(source.starts_with("public "));
            source = source.replacen("public ", "public final ", 1);
        }
        members.insert(method.method_idx, source);
    }
    let source =
        enum_init::emission::emit_enum_declaration(class, &plan, &methods, &fields, &members)
            .unwrap();
    let source = format!("package com.samsung.android.samsungpass;\n{source}");
    let evidence = PathBuf::from("/tmp/opencode/wave13-enum-public-prepared");
    std::fs::write(evidence.join("ExportFailureState.java"), &source).unwrap();
    println!("EMITTED_ORIGINAL {source}");
    assert!(source.contains("public final int getNumber()"));
    for field in &prepared {
        let original = data
            .static_fields
            .iter()
            .find(|candidate| candidate.field_idx == *field)
            .unwrap();
        assert_eq!(original.access_flags, 0x19);
        let name = dex.string(dex.field_id(*field).unwrap().name_idx).unwrap();
        assert!(source.contains(&format!("public static final int {name} = ")));
    }
    enum_runtime_oracle(&source);
    if std::env::var_os("ENUM_ORIGINAL_RUNTIME_ONLY").is_some() {
        return;
    }

    let attempt = |changed: &[DexFile], inventory: &WriteInventory| {
        let changed_class = changed[1]
            .classes
            .iter()
            .find(|owner| owner.class_idx == class.class_idx)
            .unwrap();
        let changed_resolver = Resolver {
            inputs: changed,
            dex: &changed[1],
            class: changed_class,
            cp: &cp,
            hierarchy: &hierarchy,
            super_proof: true,
        };
        enum_init::plan_enum_initialization_family_with_prepared_values(
            changed,
            1,
            changed_class,
            &constructors,
            MethodRef::new(19537),
            clinit.ir(),
            &changed_resolver,
            inventory,
        )
    };
    let first_prepared = *prepared.first().unwrap();
    let mut negatives = 0;
    for opcode in [0x52u16, 0x59, 0x60, 0x67] {
        let mut changed = inputs.clone();
        let owner = changed[1]
            .classes
            .iter_mut()
            .find(|owner| owner.class_idx == class.class_idx)
            .unwrap();
        let code = owner
            .class_data
            .as_mut()
            .unwrap()
            .direct_methods
            .iter_mut()
            .find(|method| method.method_idx == 19539)
            .unwrap()
            .code
            .as_mut()
            .unwrap();
        code.insns = [0x0012, opcode, first_prepared as u16, 0x000e]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect();
        code.insns_size = 4;
        code.tries.clear();
        code.handlers.clear();
        assert!(
            attempt(&changed, &inventory).is_err(),
            "raw access absent from positive inventory opcode={opcode:#x}"
        );
        println!("NEGATIVE raw opcode={opcode:#x} stale-positive-inventory");
        negatives += 1;
        if opcode == 0x67 {
            let changed_inventory = WriteInventory::scan_with_metadata(&changed, &cp);
            assert!(
                attempt(&changed, &changed_inventory).is_err(),
                "explicit original store"
            );
            println!("NEGATIVE complete original inventory explicit store");
            negatives += 1;
        }
    }
    let mut changed = inputs.clone();
    changed[1]
        .classes
        .iter_mut()
        .find(|owner| owner.class_idx == class.class_idx)
        .unwrap()
        .access_flags &= !0x10;
    assert!(attempt(&changed, &inventory).is_err(), "nonfinal enum");
    negatives += 1;
    let mut changed = inputs.clone();
    let owner = changed[1]
        .classes
        .iter_mut()
        .find(|owner| owner.class_idx == class.class_idx)
        .unwrap();
    owner.static_values.as_mut().unwrap()[2].value =
        dex_format::encoded_value::EncodedValueKind::Long(2);
    assert!(
        attempt(&changed, &inventory).is_err(),
        "wrong encoded literal type"
    );
    negatives += 1;
    let mut changed = inputs.clone();
    let array_type = changed[1]
        .field_id(plan.absorbed_values_field)
        .unwrap()
        .type_idx;
    changed[1].field_ids[first_prepared as usize].type_idx = array_type;
    assert!(attempt(&changed, &inventory).is_err(), "reference array");
    negatives += 1;
    assert!(
        attempt(&inputs[..2], &inventory).is_err(),
        "subset input with whole inventory"
    );
    negatives += 1;
    let mut changed = inputs.clone();
    let owner = changed[1]
        .classes
        .iter_mut()
        .find(|owner| owner.class_idx == class.class_idx)
        .unwrap();
    let code = owner
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .iter_mut()
        .find(|method| method.method_idx == 19539)
        .unwrap()
        .code
        .as_mut()
        .unwrap();
    code.insns[0] = 0x3e;
    let changed_inventory = WriteInventory::scan_with_metadata(&changed, &cp);
    assert!(changed_inventory.has_structural_unknown());
    assert!(
        attempt(&changed, &changed_inventory).is_err(),
        "structural decode uncertainty"
    );
    negatives += 1;
    assert!(
        attempt(&changed, &inventory).is_err(),
        "raw decode uncertainty missing from stale inventory"
    );
    negatives += 1;
    for access in [0x18u32, 0x1a, 0x1c] {
        let mut changed = inputs.clone();
        let owner = changed[1]
            .classes
            .iter_mut()
            .find(|owner| owner.class_idx == class.class_idx)
            .unwrap();
        for field in &mut owner.class_data.as_mut().unwrap().static_fields {
            if prepared.contains(&field.field_idx) {
                field.access_flags = access;
            }
        }
        let changed_inventory = WriteInventory::scan_with_metadata(&changed, &cp);
        let plan = attempt(&changed, &changed_inventory).unwrap();
        assert_eq!(plan.encoded_declaration_initializers.len(), 3);
        println!("POSITIVE original-shaped static final visibility={access:#x}");
    }

    let mut changed = inputs.clone();
    let string_type = changed[1]
        .type_ids
        .iter()
        .position(|ty| changed[1].string(ty.descriptor_idx).unwrap() == "Ljava/lang/String;")
        .unwrap() as u16;
    let value_string = changed[1].field_id(first_prepared).unwrap().name_idx;
    changed[1].field_ids[first_prepared as usize].type_idx = string_type;
    let owner = changed[1]
        .classes
        .iter_mut()
        .find(|owner| owner.class_idx == class.class_idx)
        .unwrap();
    let position = owner
        .class_data
        .as_ref()
        .unwrap()
        .static_fields
        .iter()
        .position(|field| field.field_idx == first_prepared)
        .unwrap();
    owner.static_values.as_mut().unwrap()[position].value =
        dex_format::encoded_value::EncodedValueKind::String(value_string);
    let changed_inventory = WriteInventory::scan_with_metadata(&changed, &cp);
    let string_plan = attempt(&changed, &changed_inventory).unwrap();
    assert!(
        string_plan
            .encoded_declaration_initializers
            .get(&first_prepared)
            .unwrap()
            .starts_with('"')
    );
    println!("POSITIVE original-shaped public static final String");
    println!("NEGATIVES PASSED {negatives}");
}

fn enum_runtime_oracle(source: &str) {
    use std::process::Command;
    let root = PathBuf::from("/tmp/opencode/wave13-enum-public-prepared/runtime");
    std::fs::create_dir_all(&root).unwrap();
    let frozen = PathBuf::from("/tmp/opencode/samsung-pass-100-wave12/java25/sources");
    let mut sources = Vec::new();
    for name in [
        "com/google/protobuf/Internal$EnumLite.java",
        "com/google/protobuf/Internal$EnumLiteMap.java",
        "com/google/protobuf/Internal$EnumVerifier.java",
        "com/samsung/android/samsungpass/ExportFailureState$1.java",
        "com/samsung/android/samsungpass/ExportFailureState$ExportFailureStateVerifier.java",
    ] {
        let target = root.join(PathBuf::from(name).file_name().unwrap());
        std::fs::copy(frozen.join(name), &target).unwrap();
        sources.push(target);
    }
    let target = root.join("ExportFailureState.java");
    std::fs::write(&target, source).unwrap();
    sources.push(target);
    let output = Command::new(java_home().join("bin/javac"))
        .arg("-d")
        .arg(&root)
        .args(&sources)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "javac: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let check = r#"package com.samsung.android.samsungpass;
public class Check {
 public static void main(String[] args) throws Exception {
  for (ExportFailureState item : ExportFailureState.values()) {
   if(ExportFailureState.valueOf(item.name()) != item) throw new AssertionError("name identity");
   if(item.name().equals("UNRECOGNIZED")) {
    try {item.getNumber();throw new AssertionError("unrecognized getNumber");}
    catch(IllegalArgumentException expected){} continue;
   }
   int value=item.getNumber();
   if(ExportFailureState.forNumber(value)!=item) throw new AssertionError("forNumber payload");
   if(ExportFailureState.internalGetValueMap().findValueByNumber(value)!=item) throw new AssertionError("value map");
   if(!ExportFailureState.internalGetVerifier().isInRange(value)) throw new AssertionError("verifier");
   String field=item.name()+"_VALUE";
   if(ExportFailureState.class.getField(field).getInt(null)!=value) throw new AssertionError("prepared value");
   System.out.println(item.name()+":"+item.ordinal()+":"+value);
  }
  if(ExportFailureState.forNumber(Integer.MIN_VALUE)!=null) throw new AssertionError("unknown number");
  if(ExportFailureState.internalGetVerifier().isInRange(Integer.MIN_VALUE)) throw new AssertionError("unknown verifier");
 }
}"#;
    let check_path = root.join("Check.java");
    std::fs::write(&check_path, check).unwrap();
    let output = Command::new(java_home().join("bin/javac"))
        .arg("-cp")
        .arg(&root)
        .arg("-d")
        .arg(&root)
        .arg(&check_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "oracle javac: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(java_home().join("bin/java"))
        .arg("-cp")
        .arg(&root)
        .arg("com.samsung.android.samsungpass.Check")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "runtime: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    println!("RUNTIME {}", String::from_utf8(output.stdout).unwrap());
    let capture = root.join("PrepareCapture.java");
    let recorder = root.join("PreparedRecorder.java");
    std::fs::write(
        &capture,
        include_str!("fixtures/enum_public_prepared/PrepareCapture.java"),
    )
    .unwrap();
    std::fs::write(
        &recorder,
        include_str!("fixtures/enum_public_prepared/PreparedRecorder.java"),
    )
    .unwrap();
    let output = Command::new(java_home().join("bin/javac"))
        .args([
            "--add-exports",
            "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
            "-d",
        ])
        .arg(&root)
        .arg(&capture)
        .arg(&recorder)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "instrumenter javac: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let enum_class = root.join("com/samsung/android/samsungpass/ExportFailureState.class");
    let pristine = std::fs::read(&enum_class).unwrap();
    let output = Command::new(java_home().join("bin/java"))
        .args([
            "--add-exports",
            "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
            "-cp",
        ])
        .arg(&root)
        .arg("PrepareCapture")
        .arg(root.join("com/samsung/android/samsungpass/ExportFailureState.class"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "instrumenter: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(java_home().join("bin/java"))
        .arg("-cp")
        .arg(&root)
        .arg("com.samsung.android.samsungpass.Check")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "instrumented runtime: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout.matches("PREPARED_CONSTRUCTOR_ENTRY 0:1:2").count(),
        4
    );
    println!("PREPARATION_TIMING_ORACLE {stdout}");
    // Distinct class directory and JVM invocation: preserve original method
    // bodies while independently representing DEX encoded-value installation
    // with entry stores instead of the source class's ConstantValue attributes.
    let reference = root.with_file_name("runtime-reference");
    if reference.exists() {
        std::fs::remove_dir_all(&reference).unwrap();
    }
    copy_oracle_classes(&root, &reference);
    let reference_class =
        reference.join("com/samsung/android/samsungpass/ExportFailureState.class");
    std::fs::write(&reference_class, pristine).unwrap();
    let output = Command::new(java_home().join("bin/java"))
        .args([
            "--add-exports",
            "java.base/jdk.internal.org.objectweb.asm=ALL-UNNAMED",
            "-cp",
        ])
        .arg(&root)
        .arg("PrepareCapture")
        .arg(&reference_class)
        .arg("reference")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "reference builder: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("reference entry stores constants=0 stores=3")
    );
    assert_ne!(
        std::fs::read(&enum_class).unwrap(),
        std::fs::read(&reference_class).unwrap()
    );
    let output = Command::new(java_home().join("bin/java"))
        .arg("-cp")
        .arg(&reference)
        .arg("com.samsung.android.samsungpass.Check")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "reference runtime: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        stdout,
        "source/reference preparation and payload observations"
    );
    println!(
        "DISTINCT_REFERENCE same preparation/payload observations; explicit entry stores vs source ConstantValue; JVM boundary only"
    );
}

fn copy_oracle_classes(source: &std::path::Path, target: &std::path::Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_oracle_classes(&entry.path(), &target.join(entry.file_name()));
        } else if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "class")
        {
            std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
#[ignore = "original Samsung private enum suffix probe; complete APK inventory and metadata"]
fn original_private_day_of_week_suffix_wave17() {
    use decomp::java_class_kinds::enum_initialization::{
        self as enum_init, EnumConstructorDeclarationInput,
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
        .find(|c| descriptor(dex, c.class_idx) == "Lorg/threeten/bp/DayOfWeek;")
        .unwrap();
    let resolver = Resolver {
        inputs: &inputs,
        dex,
        class,
        cp: &cp,
        hierarchy: &hierarchy,
        super_proof: true,
    };
    let ctor = original_ssa(dex, class, 35588);
    let clinit = original_ssa(dex, class, 35587);
    let method = resolver.method(MethodRef::new(35588)).unwrap();
    let code = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|m| m.method_idx == 35588)
        .unwrap()
        .code
        .as_ref()
        .unwrap();
    let params = bindings(&ctor, &method, code);
    let constructors = [EnumConstructorDeclarationInput {
        reference: MethodRef::new(35588),
        ir: ctor.ir(),
        bindings: &params,
    }];
    let plan = enum_init::plan_enum_initialization_family_with_prepared_values(
        &inputs,
        3,
        class,
        &constructors,
        MethodRef::new(35587),
        clinit.ir(),
        &resolver,
        &inventory,
    );
    let plan = plan.unwrap();
    assert_eq!(
        plan.constants
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        [
            "MONDAY",
            "TUESDAY",
            "WEDNESDAY",
            "THURSDAY",
            "FRIDAY",
            "SATURDAY",
            "SUNDAY"
        ]
    );
    assert_eq!(plan.absorbed_constructor, 35588);
    assert_eq!(plan.absorbed_clinit, 35587);
    let suffix = plan.static_initializer_source.as_ref().unwrap();
    println!("ORIGINAL_DAY_SUFFIX {suffix}");
    assert!(suffix.contains("new org.threeten.bp.DayOfWeek$1()"));
    assert!(suffix.contains("org.threeten.bp.DayOfWeek.values()"));
    assert!(suffix.contains("FROM ="));
    assert!(suffix.contains("ENUMS ="));
    assert_eq!(
        plan.suffix_runtime_capabilities,
        inventory
            .unknown_observations()
            .iter()
            .filter(|observation| !matches!(
                observation.kind,
                decomp::java_class_kinds::initializers::write_inventory::UnknownKind::Structural
            ))
            .count()
    );
}
