//! Constructor body recovery independent of unavailable static initialization.
#![allow(clippy::unwrap_used)]
use backend_java::constructors::{
    ConstructorClass, ConstructorResolver, Initializer, TrustedConstructorResolver,
};
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType};
use decomp::java_class_kinds::initializers::{self, write_inventory::WriteInventory};
use decomp::java_external_metadata::{Classpath, ExternalMetadata, MethodMetadata};
use dex_format::{DexFile, class::ClassDef};
use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

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
    fn instruction_type(&self, reference: managed_ir::refs::TypeRef) -> Option<JavaType> {
        decomp::dex_java::descriptor_type(descriptor(self.dex, reference.index()))
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
        if self.cp.to_hierarchy().unwrap().assignable(actual, expected)
            == backend_java::type_compatibility::Compatibility::Assignable
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
#[ignore = "explicit retained Samsung corpus and local JDK"]
fn actual_kangaroo_and_asn1_static_proofs_are_independent_of_missing_constructors() {
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let inventory = WriteInventory::scan(&inputs);
    let cp = Classpath::from_paths([java_home().join("jmods/java.base.jmod")]).unwrap();
    for (name, index) in [
        (
            "Lorg/bouncycastle/crypto/digests/Kangaroo$KangarooSponge;",
            9675,
        ),
        ("Lorg/bouncycastle/asn1/ASN1RelativeOID;", 5713),
    ] {
        let dex = &inputs[3];
        let class = dex
            .classes
            .iter()
            .find(|c| descriptor(dex, c.class_idx) == name)
            .unwrap();
        let r = Resolver {
            inputs: &inputs,
            dex,
            class,
            cp: &cp,
            super_proof: true,
        };
        let code = class
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods
            .iter()
            .find(|m| m.method_idx == index)
            .unwrap()
            .code
            .as_ref()
            .unwrap();
        let ssa = dex_decompiler::DexPasses::run_ssa_with(
            code,
            &dex_decompiler::DexTypes::new(dex, Some(index)),
        )
        .unwrap();
        eprintln!("ACTUAL {name} {index}");
        let proof = initializers::plan_static_initializer_explicit_stores(
            &inputs,
            3,
            class,
            MethodRef::new(index),
            ssa.ir(),
            &r,
            &inventory,
        )
        .unwrap();
        assert!(!proof.runtime_capabilities.is_empty());
        assert!(proof.plan.encoded.blank_instance_finals.is_empty());
        assert!(
            initializers::plan_class_initialization_with_explicit_stores(
                &inputs,
                3,
                class,
                &[],
                Some((MethodRef::new(index), ssa.ir())),
                &r,
                &inventory
            )
            .is_err()
        );
        if index == 9675 {
            let root = std::env::temp_dir()
                .join(format!("kangaroo-static-explicit-{}", std::process::id()));
            std::fs::create_dir_all(&root).unwrap();
            let source = format!(
                "public class StaticHarness {{private static final long[] KeccakRoundConstants; {} {} public static void main(String[]x){{if(KeccakRoundConstants.length!=24||KeccakRoundConstants[0]!=1L||KeccakRoundConstants[23]!=-9223372034707259384L)throw new AssertionError();}}}}",
                proof.plan.encoded.prefix_block, proof.plan.source
            );
            let file = write_java(&root, "StaticHarness.java", &source);
            javac(&root, &[file]);
            let out = Command::new(java_home().join("bin/java"))
                .args(["-cp", root.to_str().unwrap(), "StaticHarness"])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        if index == 9675 {
            let decoded = dex_decompiler::decode_method(code).unwrap();
            let store=decoded.iter().find(|i| matches!(i.opcode,dex_decompiler::Opcode::Supported(info) if (0x67..=0x6d).contains(&info.opcode))).unwrap();
            let offset = store.offset as usize * 2;
            let mut branch = code.clone();
            let register = branch.registers_size;
            assert!(register < 256);
            branch.registers_size += 1;
            let units = [0x0013 | (register << 8), 0, 0x0038 | (register << 8), 4];
            branch
                .insns
                .splice(offset..offset, units.iter().flat_map(|u| u.to_le_bytes()));
            branch.insns_size += 4;
            let mut changed = inputs.clone();
            let position = changed[3]
                .classes
                .iter()
                .position(|c| c.class_idx == class.class_idx)
                .unwrap();
            changed[3].classes[position]
                .class_data
                .as_mut()
                .unwrap()
                .direct_methods
                .iter_mut()
                .find(|m| m.method_idx == index)
                .unwrap()
                .code = Some(branch.clone());
            let inv = WriteInventory::scan(&changed);
            let ir = dex_decompiler::DexPasses::run_ssa_with(
                &branch,
                &dex_decompiler::DexTypes::new(dex, Some(index)),
            )
            .unwrap();
            assert!(
                initializers::plan_static_initializer_explicit_stores(
                    &changed,
                    3,
                    &changed[3].classes[position],
                    MethodRef::new(index),
                    ir.ir(),
                    &r,
                    &inv
                )
                .is_err(),
                "normal-return path skips final assignment"
            );
            let mut duplicate = code.clone();
            let bytes = duplicate.insns[offset..offset + 4].to_vec();
            duplicate.insns.splice(offset..offset, bytes);
            duplicate.insns_size += 2;
            let duplicate_ir = dex_decompiler::DexPasses::run_ssa_with(
                &duplicate,
                &dex_decompiler::DexTypes::new(dex, Some(index)),
            )
            .unwrap();
            assert!(
                initializers::plan_static_initializer_explicit_stores(
                    &inputs,
                    3,
                    class,
                    MethodRef::new(index),
                    duplicate_ir.ir(),
                    &r,
                    &inventory
                )
                .is_err(),
                "duplicate final assignment"
            );
        }
        // Retaining the same field store under a different original constructor is illegal.
        let mut altered = inputs.clone();
        let position = altered[3]
            .classes
            .iter()
            .position(|c| c.class_idx == class.class_idx)
            .unwrap();
        let ctor = altered[3].classes[position]
            .class_data
            .as_mut()
            .unwrap()
            .direct_methods
            .iter_mut()
            .find(|m| m.method_idx != index)
            .unwrap();
        ctor.code = Some(code.clone());
        let invalid = WriteInventory::scan(&altered);
        assert!(
            initializers::plan_static_initializer_explicit_stores(
                &altered,
                3,
                &altered[3].classes[position],
                MethodRef::new(index),
                ssa.ir(),
                &r,
                &invalid
            )
            .is_err()
        );
    }
}

#[test]
fn separate_static_prefix_preserves_active_use_order_and_nonconstant_final_reads() {
    let root = std::env::temp_dir().join(format!("static-contract-order-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let file = write_java(
        &root,
        "StaticOrder.java",
        r#"public class StaticOrder {static String trace="";static class Input {static final int encoded;static {encoded=7;trace+="E";}static final int computed;static {trace+="B";computed=encoded+4;trace+="C";}final int instance;Input(int x){instance=x;}}public static void main(String[]x)throws Exception{if(!trace.equals(""))throw new AssertionError();java.lang.reflect.Field f=Input.class.getDeclaredField("computed");if(f.getInt(null)!=11||!trace.equals("EBC"))throw new AssertionError(trace);if(f.getInt(null)!=11||!trace.equals("EBC"))throw new AssertionError();}}"#,
    );
    javac(&root, &[file]);
    let out = Command::new(java_home().join("bin/java"))
        .args(["-cp", root.to_str().unwrap(), "StaticOrder"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
