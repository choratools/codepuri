//! Genuine whole-input and exact seeded SSA evidence for six Kotlin nullable-mask constructors.
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
#[ignore = "requires retained original SamsungPass APK and genuine Java25/Android metadata"]
fn original_six_nullable_masks_have_exact_prefix_and_atomic_family_evidence() {
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
    println!(
        "WHOLE INPUT structuralunknown={} retainedcapabilities={} inputs={}",
        inventory.has_structural_unknown(),
        inventory.unknown_observations().len(),
        inputs.len()
    );
    for (pool, index) in [
        (1, 14488),
        (1, 14525),
        (1, 14544),
        (1, 14565),
        (1, 14605),
        (1, 14670),
    ] {
        let dex = &inputs[pool];
        let key = method_key(dex, index);
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
        println!(
            "CASE pool{pool}#{index} {:?} fields S{} I{} encoded{}",
            key,
            data.static_fields.len(),
            data.instance_fields.len(),
            class.static_values.as_ref().map_or(0, Vec::len)
        );
        let methods = data
            .direct_methods
            .iter()
            .filter(|m| {
                ["<init>", "<clinit>"].contains(&method_key(dex, m.method_idx).name.as_str())
            })
            .collect::<Vec<_>>();
        let constructors = methods
            .iter()
            .filter(|m| method_key(dex, m.method_idx).name == "<init>")
            .map(|m| m.method_idx)
            .collect::<BTreeSet<_>>();
        let instance = initializers::prove_instance_final_explicit_stores(
            &inputs,
            pool,
            class,
            &constructors,
            &inventory,
        );
        println!(
            "INSTANCE WHOLE STORE {}",
            match &instance {
                Ok(p) => format!("OK{}finals", p.plan.len()),
                Err(e) => format!("ERR{e:?}"),
            }
        );
        for f in data
            .static_fields
            .iter()
            .chain(&data.instance_fields)
            .filter(|f| f.access_flags & 16 != 0)
        {
            let k = initializers::write_inventory::field_key(dex, f.field_idx).unwrap();
            let sites = inventory.explicit_store_writes(&k);
            println!(
                "FINAL#{} flags{:x} {:?} stores{:?}",
                f.field_idx, f.access_flags, k, sites
            );
        }
        let mut ssas = vec![];
        let mut bs = vec![];
        let mut retained = vec![];
        for m in &methods {
            let Some(code) = &m.code else {
                println!("METHOD#{} NO ORIGINAL BODY", m.method_idx);
                continue;
            };
            let context = dex_decompiler::DexTypes::new(dex, Some(m.method_idx));
            let mut seeds = HashMap::new();
            let mut word = code.registers_size - code.ins_size;
            if m.access_flags & 8 == 0 {
                seeds.insert(
                    (word, false),
                    context
                        .type_at(u32::from(dex.method_id(m.method_idx).unwrap().class_idx))
                        .unwrap(),
                );
                word += 1;
            }
            let mut ps = vec![];
            assert!(context.method_params(m.method_idx, &mut ps));
            for t in ps {
                let wide = matches!(
                    t,
                    managed_ir::types::ManagedType::Long | managed_ir::types::ManagedType::Double
                );
                seeds.insert((word, wide), t);
                word += if wide { 2 } else { 1 };
            }
            let ssa = match dex_decompiler::DexPasses::run_ssa_with(
                code,
                &context.with_initial_registers(seeds),
            ) {
                Ok(s) => s,
                Err(e) => {
                    println!("METHOD#{} FRONTEND {e:?}", m.method_idx);
                    continue;
                }
            };
            let signature = r.method(MethodRef::new(m.method_idx)).unwrap();
            let b = bindings(&ssa, &signature, code);
            println!(
                "METHOD#{} {} words{} tries{}",
                m.method_idx,
                signature.name,
                code.insns_size,
                code.tries.len()
            );
            let stores = ssa
                .ir()
                .instructions()
                .iter()
                .filter_map(|(id, i)| {
                    if let managed_ir::insn::ManagedInsn::FieldPut { object, field, .. } = i {
                        Some((ssa.offset(id), field.index(), object.is_none()))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            println!("ORIGINAL STORES {stores:?}");
            if signature.name == "<init>" {
                let finals = data
                    .instance_fields
                    .iter()
                    .filter(|f| f.access_flags & 16 != 0)
                    .map(|f| FieldRef::new(f.field_idx))
                    .collect::<Vec<_>>();
                let local =
                    backend_java::constructors::constructor_receiver_initialization_evidence(
                        ssa.ir(),
                        MethodRef::new(m.method_idx),
                        b.receiver,
                        &source_name(descriptor(dex, class.superclass_idx)),
                        &finals,
                        &r,
                    );
                println!("CTOR LOCAL {local:?}");
                let prefix = backend_java::constructors::mask_choices::plan(
                    &backend_java::constructors::ConstructorInput {
                        reference: MethodRef::new(m.method_idx),
                        ir: ssa.ir(),
                        bindings: &b,
                    },
                    &r,
                );
                if m.method_idx == index {
                    let prefix = prefix.unwrap();
                    assert!(
                        prefix
                            .steps
                            .iter()
                            .any(|step| matches!(step, backend_java::branching::Step::If { .. }))
                    );
                    println!(
                        "ORIGINAL #{index} prefix blocks{} exacttarget{}",
                        prefix.prefix_blocks.len(),
                        prefix.target.index()
                    );
                }
                let source = backend_java::constructors::plan_constructor_declaration(
                    ssa.ir(),
                    MethodRef::new(m.method_idx),
                    &source_name(descriptor(dex, class.superclass_idx)),
                    &b,
                    &r,
                );
                println!(
                    "CTOR BODY {}",
                    match source {
                        Ok(p) => format!(
                            "OK {:?} target{} first{}",
                            p.chain,
                            p.target.index(),
                            p.first_statement
                        ),
                        Err(e) => format!("ERR{e:?}"),
                    }
                );
            } else {
                let proof = initializers::plan_static_initializer_explicit_stores_attested(
                    &inputs,
                    pool,
                    class,
                    MethodRef::new(m.method_idx),
                    &ssa,
                    &r,
                    &inventory,
                );
                println!(
                    "STATIC ATTESTED {}",
                    match proof {
                        Ok(p) => format!(
                            "OK source{} prefix{}",
                            p.plan.source.len(),
                            p.plan.encoded.prefix_block.len()
                        ),
                        Err(e) => format!("ERR{e:?}"),
                    }
                );
                let finals = data
                    .static_fields
                    .iter()
                    .filter(|f| f.access_flags & 16 != 0)
                    .map(|f| FieldRef::new(f.field_idx))
                    .collect::<Vec<_>>();
                let ctx = backend_java::static_initializer::StaticInitializerContext {
                    owner: source_name(&key.owner),
                    writable_static_fields: data
                        .static_fields
                        .iter()
                        .filter(|f| f.access_flags & 16 == 0)
                        .map(|f| r.field(FieldRef::new(f.field_idx)).unwrap())
                        .collect(),
                };
                let source = backend_java::static_initializer::emit_static_initializer_body_subset(
                    ssa.ir(),
                    &signature,
                    &backend_java::ParameterBindings::default(),
                    &r,
                    &ctx,
                    &finals,
                );
                println!(
                    "STATIC BACKEND {}",
                    match source {
                        Ok(s) => format!("OK source{}", s.len()),
                        Err(e) => format!("ERR{e:?}"),
                    }
                );
            }
            retained.push(m.method_idx);
            ssas.push(ssa);
            bs.push(b);
        }
        let cs = retained
            .iter()
            .enumerate()
            .filter(|(_, m)| method_key(dex, **m).name == "<init>")
            .map(|(i, m)| backend_java::constructors::ConstructorInput {
                reference: MethodRef::new(*m),
                ir: ssas[i].ir(),
                bindings: &bs[i],
            })
            .collect::<Vec<_>>();
        let ci = retained
            .iter()
            .enumerate()
            .find(|(_, m)| method_key(dex, **m).name == "<clinit>")
            .map(|(i, m)| (MethodRef::new(*m), &ssas[i]));
        let plan = initializers::plan_class_initialization_with_explicit_stores_attested(
            &inputs, pool, class, &cs, ci, &r, &inventory,
        );
        if let Err(error) = plan {
            panic!("original #{index} atomic family: {error:?}");
        }
        println!("ORIGINAL #{index} ATOMIC FAMILY OK");
    }
}

#[test]
#[ignore = "requires retained original SamsungPass APK and genuine Java25/Android metadata"]
fn original_pool2_method25624_autofill_initialized_loop_suffix_supported() {
    use dex_decompiler::infer::TypeContext;
    let inputs = dex_format::apk::parse_apk_path(
        "/storage_0/decomphify/tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    let pool = 2usize;
    let index = 25624u32;
    let dex = &inputs[pool];
    let key = method_key(dex, index);
    assert_eq!(key.name, "<init>");
    assert!(
        key.owner.contains("AutoFill"),
        "unexpected pool{pool}#{index} owner {:?}",
        key
    );
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == key.owner)
        .unwrap();
    let data = class.class_data.as_ref().unwrap();
    let original = data
        .direct_methods
        .iter()
        .find(|m| m.method_idx == index)
        .unwrap();
    let code = original.code.as_ref().unwrap();
    let cp = Classpath::from_paths([
        PathBuf::from("/tmp/decomphify-source25-java-base-649174/java.base.jar"),
        PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
    ])
    .unwrap();
    let hierarchy =
        decomp::java_hierarchy::build_with_metadata(&inputs.iter().collect::<Vec<_>>(), &cp)
            .unwrap();
    let inventory = WriteInventory::scan_with_metadata(&inputs, &cp);
    let resolver = Resolver {
        inputs: &inputs,
        dex,
        class,
        cp: &cp,
        hierarchy: &hierarchy,
        super_proof: true,
    };
    let context = dex_decompiler::DexTypes::new(dex, Some(index));
    let mut seeds = HashMap::new();
    let mut word = code.registers_size - code.ins_size;
    seeds.insert(
        (word, false),
        context
            .type_at(u32::from(dex.method_id(index).unwrap().class_idx))
            .unwrap(),
    );
    word += 1;
    let mut params = vec![];
    assert!(context.method_params(index, &mut params));
    for ty in params {
        let wide = matches!(
            ty,
            managed_ir::types::ManagedType::Long | managed_ir::types::ManagedType::Double
        );
        seeds.insert((word, wide), ty);
        word += if wide { 2 } else { 1 };
    }
    let ssa = dex_decompiler::DexPasses::run_ssa_with(code, &context.with_initial_registers(seeds))
        .unwrap();
    let signature = resolver.method(MethodRef::new(index)).unwrap();
    let primary_bindings = bindings(&ssa, &signature, code);
    let source = backend_java::constructors::plan_constructor_declaration(
        ssa.ir(),
        MethodRef::new(index),
        &source_name(descriptor(dex, class.superclass_idx)),
        &primary_bindings,
        &resolver,
    )
    .unwrap();
    println!(
        "AUTOFILL pool{pool}#{index} {:?} first={} body_lines={} source=\n{}",
        key,
        source.first_statement,
        source.body.len(),
        source.source
    );
    assert!(source.source.contains("super("), "{}", source.source);
    assert_eq!(
        source.source.matches("while (").count(),
        2,
        "{}",
        source.source
    );
    let first_loop = source.source.find("while (").unwrap();
    let final_after_loops = source.source.rfind("this.").unwrap();
    assert!(
        final_after_loops > first_loop,
        "expected own-field store after initialized loops:\n{}",
        source.source
    );
    assert!(
        source
            .body
            .iter()
            .filter(|line| line.contains("this."))
            .count()
            >= 8,
        "{:?}",
        source.body
    );

    let mut ssas = vec![ssa];
    let mut bs = vec![primary_bindings];
    let mut retained = vec![index];
    for m in data.direct_methods.iter().filter(|m| m.method_idx != index) {
        let name = method_key(dex, m.method_idx).name;
        if !["<init>", "<clinit>"].contains(&name.as_str()) {
            continue;
        }
        let Some(code) = &m.code else {
            continue;
        };
        let context = dex_decompiler::DexTypes::new(dex, Some(m.method_idx));
        let ssa = dex_decompiler::DexPasses::run_ssa_with(code, &context).unwrap();
        let signature = resolver.method(MethodRef::new(m.method_idx)).unwrap();
        bs.push(bindings(&ssa, &signature, code));
        ssas.push(ssa);
        retained.push(m.method_idx);
    }
    let constructors = retained
        .iter()
        .enumerate()
        .filter(|(_, method)| method_key(dex, **method).name == "<init>")
        .map(|(i, method)| backend_java::constructors::ConstructorInput {
            reference: MethodRef::new(*method),
            ir: ssas[i].ir(),
            bindings: &bs[i],
        })
        .collect::<Vec<_>>();
    let clinit = retained
        .iter()
        .enumerate()
        .find(|(_, method)| method_key(dex, **method).name == "<clinit>")
        .map(|(i, method)| (MethodRef::new(*method), &ssas[i]));
    initializers::plan_class_initialization_with_explicit_stores_attested(
        &inputs,
        pool,
        class,
        &constructors,
        clinit,
        &resolver,
        &inventory,
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!(
        "decomphify-autofill-initialized-suffix-{}",
        std::process::id()
    ));
    let write = |path: &str, text: &str| {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    };
    write(
        "com/samsung/android/samsungpass/websignin/datamodel/AutoFillQueryDataModelImpl.java",
        &format!(
            r#"package com.samsung.android.samsungpass.websignin.datamodel;
public class AutoFillQueryDataModelImpl {{
  public com.samsung.android.samsungpass.common.db.signindb.WebSigninDBManager webSigninDBManager;
  public com.samsung.android.samsungpass.common.db.autoFillForm.AutoFillFormDBManager autoFillFormDBManager;
  public java.util.HashMap nameHashLocal;
  public android.util.SparseArray progressSubjectLocal;
  public android.util.SparseArray queryTokenListLocal;
  public java.util.HashMap contentObservableMapLocal;
  public io.a compositeDisposableLocal;
  public AutoFillQueryDataModelImpl$contentObserver$1 contentObserver;
{}}}
"#,
            source.source
        ),
    );
    for (path, text) in [
        (
            "evidence/Trace.java",
            r#"package evidence;public class Trace{public static int mode,puts;public static String log="";public static void mark(String s){log+=s;}}"#,
        ),
        (
            "kotlin/jvm/internal/n.java",
            r#"package kotlin.jvm.internal;public class n{public static void f(Object o,String s){evidence.Trace.mark("N"+s+";");if(o==null)throw new NullPointerException(s);}}"#,
        ),
        (
            "android/util/SparseArray.java",
            r#"package android.util;public class SparseArray{public Object[] values=new Object[8];public int size;public void put(int key,Object value){evidence.Trace.mark("S"+key+";");if(evidence.Trace.mode==2&&++evidence.Trace.puts==6)throw new IllegalStateException("loop failure");values[key]=value;size++;}public Object get(int key){return values[key];}}"#,
        ),
        (
            "vp/a.java",
            r#"package vp;public class a{public final Object value;public a(Object value){this.value=value;}public static a V0(Object value){evidence.Trace.mark("P;");return new a(value);}}"#,
        ),
        ("io/a.java", r#"package io;public class a{}"#),
        (
            "com/samsung/android/samsungpass/common/db/signindb/WebSigninDBManager.java",
            r#"package com.samsung.android.samsungpass.common.db.signindb;public class WebSigninDBManager{}"#,
        ),
        (
            "com/samsung/android/samsungpass/common/db/autoFillForm/AutoFillFormDBManager.java",
            r#"package com.samsung.android.samsungpass.common.db.autoFillForm;public class AutoFillFormDBManager{}"#,
        ),
        (
            "com/samsung/android/samsungpass/websignin/datamodel/AutoFillQueryDataModelImpl$contentObserver$1.java",
            r#"package com.samsung.android.samsungpass.websignin.datamodel;public class AutoFillQueryDataModelImpl$contentObserver$1{public final AutoFillQueryDataModelImpl owner;public AutoFillQueryDataModelImpl$contentObserver$1(AutoFillQueryDataModelImpl owner){evidence.Trace.mark("O;");this.owner=owner;}}"#,
        ),
        (
            "Driver.java",
            r#"public class Driver{public static void main(String[]args){evidence.Trace.mode=Integer.parseInt(args[0]);try{var web=evidence.Trace.mode==1?null:new com.samsung.android.samsungpass.common.db.signindb.WebSigninDBManager();var form=new com.samsung.android.samsungpass.common.db.autoFillForm.AutoFillFormDBManager();var o=new com.samsung.android.samsungpass.websignin.datamodel.AutoFillQueryDataModelImpl(web,form);if(o.webSigninDBManager!=web||o.autoFillFormDBManager!=form||o.nameHashLocal==null||o.contentObservableMapLocal==null||o.compositeDisposableLocal==null||o.contentObserver.owner!=o)throw new AssertionError("identity");if(o.progressSubjectLocal.size!=3||o.queryTokenListLocal.size!=3)throw new AssertionError("loop");System.out.println(evidence.Trace.log+"ok");}catch(Throwable t){if(t instanceof AssertionError)throw (AssertionError)t;System.out.println(evidence.Trace.log+t.getClass().getSimpleName());}}}"#,
        ),
    ] {
        write(path, text);
    }
    let java_home = std::env::var("DECOMPHIFY_JDK25_HOME")
        .unwrap_or_else(|_| "/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1".into());
    let run = |tool: &str, args: &[&str]| {
        let out = std::process::Command::new(format!("{java_home}/bin/{tool}"))
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{tool}:{}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    };
    run(
        "javac",
        &[
            "--release",
            "25",
            "Driver.java",
            "evidence/Trace.java",
            "kotlin/jvm/internal/n.java",
            "android/util/SparseArray.java",
            "vp/a.java",
            "io/a.java",
            "com/samsung/android/samsungpass/common/db/signindb/WebSigninDBManager.java",
            "com/samsung/android/samsungpass/common/db/autoFillForm/AutoFillFormDBManager.java",
            "com/samsung/android/samsungpass/websignin/datamodel/AutoFillQueryDataModelImpl$contentObserver$1.java",
            "com/samsung/android/samsungpass/websignin/datamodel/AutoFillQueryDataModelImpl.java",
        ],
    );
    let traces = ["0", "1", "2"]
        .into_iter()
        .flat_map(|mode| run("java", &["-Xverify:all", "Driver", mode]))
        .collect::<Vec<_>>();
    let traces = String::from_utf8(traces).unwrap();
    assert_eq!(
        traces,
        "NwebSigninDBManager;NautoFillFormDBManager;P;S0;P;S1;P;S2;S0;S1;S2;O;ok\nNwebSigninDBManager;NullPointerException\nNwebSigninDBManager;NautoFillFormDBManager;P;S0;P;S1;P;S2;S0;S1;S2;IllegalStateException\n"
    );
    write("runtime-traces.txt", &traces);
    println!("AUTOFILL pool{pool}#{index} initialized loop suffix and atomic family OK");
}
