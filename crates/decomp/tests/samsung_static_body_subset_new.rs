//! Constructor body recovery independent of unavailable static initialization.
#![allow(clippy::unwrap_used)]
use backend_java::constructors::{
    ConstructorClass, ConstructorInput, ConstructorResolver, Initializer,
    TrustedConstructorResolver,
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
#[ignore = "requires retained Samsung APK and explicit local JDK17"]
fn actual_static_final_body_retains_order_and_unknown_status() {
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let dex = &inputs[0];
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == "Landroidx/compose/ui/platform/c1;")
        .unwrap();
    let cp = Classpath::from_paths([java_home().join("jmods/java.base.jmod")]).unwrap();
    let r = Resolver {
        inputs: &inputs,
        dex,
        class,
        cp: &cp,
        super_proof: true,
    };
    let inventory = WriteInventory::scan(&inputs);
    assert!(!inventory.unknown_reasons().is_empty());
    let data = class.class_data.as_ref().unwrap();
    let raw = data
        .direct_methods
        .iter()
        .find(|m| m.method_idx == 9766)
        .unwrap();
    let code = raw.code.as_ref().unwrap();
    let ssa = dex_decompiler::DexPasses::run_ssa_with(
        code,
        &dex_decompiler::DexTypes::new(dex, Some(9766)),
    )
    .unwrap();
    let subset = initializers::plan_static_initializer_body_subset(
        &inputs,
        0,
        class,
        MethodRef::new(9766),
        ssa.ir(),
        &r,
        &inventory,
    )
    .unwrap();
    assert!(initializers::plan_encoded_initialization(dex, class, &inventory).is_err());
    let ctor = data
        .direct_methods
        .iter()
        .find(|m| m.method_idx == 9767)
        .unwrap();
    let cc = ctor.code.as_ref().unwrap();
    let ci = dex_decompiler::DexPasses::run_ssa_with(
        cc,
        &dex_decompiler::DexTypes::new(dex, Some(9767)),
    )
    .unwrap();
    let sig = r.method(MethodRef::new(9767)).unwrap();
    let binding =
        decomp::dex_java::parameter_bindings(ci.ir(), &sig, cc.registers_size, cc.ins_size)
            .unwrap();
    let constructors = initializers::plan_constructor_body_subset(
        dex,
        class,
        &[ConstructorInput {
            reference: MethodRef::new(9767),
            ir: ci.ir(),
            bindings: &binding,
        }],
        &r,
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!("decomphify-static-subset-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fields = data
        .static_fields
        .iter()
        .map(|f| {
            let mapped = r.field(FieldRef::new(f.field_idx)).unwrap();
            format!(
                "public static final {} {};",
                source_name(descriptor(
                    dex,
                    u32::from(dex.field_id(f.field_idx).unwrap().type_idx)
                )),
                mapped.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let source = format!(
        "package androidx.compose.ui.platform; public class c1 {{ {fields} {} {} {} }}",
        subset.encoded.prefix_block, subset.source, constructors.constructors[0].source
    );
    let file = write_java(&dir, "androidx/compose/ui/platform/c1.java", &source);
    javac(&dir, &[file]);
    let h = write_java(
        &dir,
        "StaticHarness.java",
        "public class StaticHarness { public static void main(String[] args) { androidx.compose.ui.platform.c1 first=androidx.compose.ui.platform.c1.a; if (first==null || first!=androidx.compose.ui.platform.c1.a || first==new androidx.compose.ui.platform.c1()) throw new AssertionError(); } }",
    );
    javac(
        &dir,
        &["-classpath".into(), dir.to_string_lossy().into(), h],
    );
    let output = Command::new(java_home().join("bin/java"))
        .arg("-classpath")
        .arg(&dir)
        .arg("StaticHarness")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Positive known conflicting writer remains a rejection despite body-only mode.
    let mut conflicting = inputs.clone();
    let slot = conflicting[0]
        .classes
        .iter()
        .position(|c| c.class_idx == class.class_idx)
        .unwrap();
    let target = conflicting[0].classes[slot]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .iter_mut()
        .find(|m| m.method_idx == 9767)
        .unwrap();
    target.code = Some(code.clone());
    let conflicting_inventory = WriteInventory::scan(&conflicting);
    assert!(
        initializers::plan_static_initializer_body_subset(
            &conflicting,
            0,
            &conflicting[0].classes[slot],
            MethodRef::new(9766),
            ssa.ir(),
            &r,
            &conflicting_inventory
        )
        .is_err()
    );
    let decoded = dex_decompiler::decode_method(code).unwrap();
    let store = decoded
        .iter()
        .find(|i| (0x67..=0x6d).contains(&code.insns[i.offset as usize * 2]))
        .unwrap();
    let offset = store.offset as usize * 2;
    let mut duplicate = code.clone();
    let bytes = duplicate.insns[offset..offset + 4].to_vec();
    duplicate.insns.splice(offset..offset, bytes);
    duplicate.insns_size += 2;
    let duplicate_ssa = dex_decompiler::DexPasses::run_ssa_with(
        &duplicate,
        &dex_decompiler::DexTypes::new(dex, Some(9766)),
    )
    .unwrap();
    assert!(
        initializers::plan_static_initializer_body_subset(
            &inputs,
            0,
            class,
            MethodRef::new(9766),
            duplicate_ssa.ir(),
            &r,
            &inventory
        )
        .is_err()
    );
    // An explicit encoded value plus the same final's body assignment is a double initialization.
    let mut changed = inputs.clone();
    let slot = changed[0]
        .classes
        .iter()
        .position(|c| c.class_idx == class.class_idx)
        .unwrap();
    changed[0].classes[slot].static_values = Some(vec![dex_format::encoded_value::EncodedValue {
        value: dex_format::encoded_value::EncodedValueKind::Null,
        span: dex_format::RawSpan { offset: 0, size: 0 },
    }]);
    let changed_class = &changed[0].classes[slot];
    assert!(
        initializers::plan_static_initializer_body_subset(
            &changed,
            0,
            changed_class,
            MethodRef::new(9766),
            ssa.ir(),
            &r,
            &inventory
        )
        .is_err()
    );
    assert!(!inventory.unknown_reasons().is_empty());
    eprintln!(
        "actual compose c1 <clinit> #9766 compiled/runtime; original arbitrary unknowns retained"
    );
}
