//! Bounded original static-initializer loop witnesses from the final wave4 ledger.
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

#[test]
#[ignore = "requires retained Samsung APK and JDK17"]
fn original_wave4_static_loops_have_exact_final_stores_and_attested_source() {
    let inputs = dex_format::apk::parse_apk_path(
        "../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    let cp = Classpath::from_paths([java_home().join("jmods/java.base.jmod")]).unwrap();
    let inventory = WriteInventory::scan_with_metadata(&inputs, &cp);
    let dex = &inputs[3];
    for index in [10292, 42632, 7343] {
        let owner = method_key(dex, index).owner;
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
            super_proof: true,
        };
        let data = class.class_data.as_ref().unwrap();
        let original = data
            .direct_methods
            .iter()
            .find(|m| m.method_idx == index)
            .unwrap();
        let ssa = dex_decompiler::DexPasses::run_ssa_with(
            original.code.as_ref().unwrap(),
            &dex_decompiler::DexTypes::new(dex, Some(index)),
        )
        .unwrap();
        let finals = data
            .static_fields
            .iter()
            .filter(|f| f.access_flags & 0x10 != 0 && ssa.ir().instructions().values().any(|i|matches!(i,managed_ir::insn::ManagedInsn::FieldPut{field,..} if field.index()==f.field_idx)))
            .map(|f| FieldRef::new(f.field_idx))
            .collect::<Vec<_>>();
        let context = backend_java::static_initializer::StaticInitializerContext {
            owner: source_name(&owner),
            writable_static_fields: data
                .static_fields
                .iter()
                .filter(|f| f.access_flags & 0x10 == 0)
                .map(|f| r.field(FieldRef::new(f.field_idx)).unwrap())
                .collect(),
        };
        let source = backend_java::static_initializer::emit_static_initializer_body_subset(
            ssa.ir(),
            &r.method(MethodRef::new(index)).unwrap(),
            &backend_java::ParameterBindings::default(),
            &r,
            &context,
            &finals,
        );
        if index == 7343 {
            assert!(
                matches!(source, Err(backend_java::Unsupported::Instruction(ref reason)) if reason == "allocation may throw OOM before initializer")
            );
            continue;
        }
        let body = source.unwrap();
        assert!(body.contains("while (true)"));
        let proof = initializers::plan_static_initializer_explicit_stores_attested(
            &inputs,
            3,
            class,
            MethodRef::new(index),
            &ssa,
            &r,
            &inventory,
        )
        .unwrap();
        assert_eq!(proof.plan.source, body);
        let expected: &[(u32, u32)] = if index == 10292 {
            &[
                (5692, 7),
                (5680, 11),
                (5681, 15),
                (5682, 19),
                (5683, 23),
                (5684, 27),
                (5685, 31),
                (5686, 35),
                (5687, 39),
                (5689, 45),
            ]
        } else {
            &[(18252, 5), (18253, 14), (18254, 21), (18255, 28)]
        };
        assert_eq!(finals.len(), expected.len());
        for &(field, offset) in expected {
            let key = initializers::write_inventory::field_key(dex, field).unwrap();
            assert_eq!(
                inventory.explicit_store_writes(&key).unwrap(),
                std::collections::BTreeSet::from([initializers::write_inventory::WriteSite {
                    dex: 3,
                    method: index,
                    offset,
                    is_static: true
                }])
            );
            assert!(proof.plan.encoded.blank_static_finals.contains(&field));
        }
        let dir =
            std::env::temp_dir().join(format!("original-static-loops-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{index}.java")),
            format!("{}\n{body}", proof.plan.encoded.prefix_block),
        )
        .unwrap();
        eprintln!(
            "METHOD {index} exact finals={} attested source={}bytes path={}",
            finals.len(),
            body.len(),
            dir.display()
        );
    }
}
