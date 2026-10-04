//! Bounded original static-initializer exception witnesses from the final wave4 ledger.
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
fn original_static_exception_stores_scopes_and_unchanged_sources_compile_run() {
    let inputs = dex_format::apk::parse_apk_path(
        "../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    let cp = Classpath::from_paths([java_home().join("jmods/java.base.jmod")]).unwrap();
    let mut hierarchy = backend_java::type_compatibility::Hierarchy::default();
    for declaration in cp.hierarchy() {
        hierarchy.insert(declaration).unwrap();
    }
    let inventory = WriteInventory::scan_with_metadata(&inputs, &cp);
    for (di, index) in [(0, 34895), (0, 39626), (0, 29771), (0, 14358), (1, 43563)] {
        let dex = &inputs[di];
        let owner = method_key(dex, index).owner;
        let class = dex
            .classes
            .iter()
            .find(|c| descriptor(dex, c.class_idx) == owner)
            .unwrap();
        let original = class
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods
            .iter()
            .find(|m| m.method_idx == index)
            .unwrap();
        let r = Resolver {
            inputs: &inputs,
            dex,
            class,
            cp: &cp,
            super_proof: true,
        };
        let ssa = dex_decompiler::DexPasses::run_ssa_with(
            original.code.as_ref().unwrap(),
            &dex_decompiler::DexTypes::new(dex, Some(index)),
        )
        .unwrap();
        let proof = initializers::plan_static_initializer_explicit_stores_with_exceptions_attested(
            &inputs,
            di,
            class,
            MethodRef::new(index),
            &ssa,
            &r,
            &inventory,
            &hierarchy,
        )
        .unwrap();
        assert!(proof.plan.encoded.prefix_block.is_empty());
        assert!(!proof.runtime_capabilities.is_empty());
        assert_eq!(
            proof
                .plan
                .source
                .matches("catch (java.lang.NoSuchFieldError ")
                .count(),
            original.code.as_ref().unwrap().tries.len()
        );
        let (field, offset) = match index {
            34895 => (17238, 7),
            39626 => (19459, 7),
            29771 => (15384, 7),
            14358 => (6779, 16),
            _ => (59975, 16),
        };
        assert_eq!(
            proof.plan.encoded.blank_static_finals,
            std::collections::BTreeSet::from([field])
        );
        assert_eq!(
            inventory
                .explicit_store_writes(
                    &initializers::write_inventory::field_key(dex, field).unwrap()
                )
                .unwrap(),
            std::collections::BTreeSet::from([initializers::write_inventory::WriteSite {
                dex: di,
                method: index,
                offset,
                is_static: true
            }])
        );
        {
            let dir = std::env::temp_dir()
                .join(format!("original-static-exceptions-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join(format!("{index}.java")),
                format!("{}\n{}", proof.plan.encoded.prefix_block, proof.plan.source),
            )
            .unwrap();
            if matches!(index, 34895 | 39626) {
                compile_actual_jdk_table(&dir, index, class, &r, &proof.plan.source);
            }
            eprintln!(
                "METHOD {index} source {} finals {:?} {}",
                proof.plan.source.len(),
                proof.plan.encoded.blank_static_finals,
                dir.display()
            );
        }
    }
}

fn compile_actual_jdk_table(
    dir: &std::path::Path,
    index: u32,
    class: &ClassDef,
    r: &Resolver<'_>,
    body: &str,
) {
    use std::process::Command;
    // This retained original unit has exactly the final table and clinit. The
    // compiler witness covers initialization behavior, not InnerClasses flags.
    let owner = source_name(descriptor(r.dex, class.class_idx));
    let (package, name) = owner.rsplit_once('.').unwrap();
    let data = class.class_data.as_ref().unwrap();
    assert_eq!(data.direct_methods.len(), 1);
    assert!(data.virtual_methods.is_empty() && data.instance_fields.is_empty());
    assert_eq!(data.static_fields.len(), 1);
    assert_eq!(class.access_flags, 0x1401);
    let original_field = &data.static_fields[0];
    assert_eq!(original_field.access_flags, 0x1019);
    let field = r.field(FieldRef::new(original_field.field_idx)).unwrap();
    let source = format!(
        "package {package}; public abstract class {name} {{public static final int[] {};\n{body}}}",
        field.name
    );
    let unit = dir.join(format!("{name}.java"));
    std::fs::write(&unit, source).unwrap();
    let expected = if index == 34895 {
        "[4, 2, 5, 3, 7, 6, 8, 1]"
    } else {
        "[1, 2, 3, 0, 0, 0, 0]"
    };
    let check = format!(
        r#"public class Check {{public static void main(String[]args)throws Exception{{Class<?> c=Class.forName("{owner}");java.lang.reflect.Field f=c.getDeclaredField("{}");if(f.getModifiers()!=25)throw new AssertionError("final/access changed");int[] a=(int[])f.get(null);if(!java.util.Arrays.toString(a).equals("{expected}"))throw new AssertionError(java.util.Arrays.toString(a));if(a!=f.get(null))throw new AssertionError("reference changed");System.out.print(java.util.Arrays.toString(a));}}}}"#,
        field.name
    );
    std::fs::write(dir.join("Check.java"), check).unwrap();
    let compile = Command::new(java_home().join("bin/javac"))
        .args(["--release", "17", "-d", "."])
        .arg(unit.file_name().unwrap())
        .arg("Check.java")
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let run = Command::new(java_home().join("bin/java"))
        .args(["-cp", ".", "Check"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8(run.stdout).unwrap(), expected);
}
