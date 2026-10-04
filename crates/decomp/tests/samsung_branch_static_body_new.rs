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
        let owner_class = self
            .dex
            .classes
            .iter()
            .find(|c| c.class_idx == u32::from(raw.class_idx))?;
        let data = owner_class.class_data.as_ref()?;
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
#[ignore = "requires retained Samsung APK and JDK17"]
fn actual_branch_static_probe() {
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let dex = &inputs[0];
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == "La2/a$c;")
        .unwrap();
    let cp = Classpath::from_paths([java_home().join("jmods/java.base.jmod")]).unwrap();
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
        .find(|m| m.method_idx == 14)
        .unwrap()
        .code
        .as_ref()
        .unwrap();
    let ssa = dex_decompiler::DexPasses::run_ssa_with(
        code,
        &dex_decompiler::DexTypes::new(dex, Some(14)),
    )
    .unwrap();
    let inv = WriteInventory::scan(&inputs);
    let plan = initializers::plan_static_initializer_body_subset(
        &inputs,
        0,
        class,
        MethodRef::new(14),
        ssa.ir(),
        &r,
        &inv,
    )
    .unwrap();
    assert!(initializers::plan_encoded_initialization(dex, class, &inv).is_err());
    let ctor = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|m| m.method_idx == 15)
        .unwrap()
        .code
        .as_ref()
        .unwrap();
    let cs = dex_decompiler::DexPasses::run_ssa_with(
        ctor,
        &dex_decompiler::DexTypes::new(dex, Some(15)),
    )
    .unwrap();
    let binding = decomp::dex_java::parameter_bindings(
        cs.ir(),
        &r.method(MethodRef::new(15)).unwrap(),
        ctor.registers_size,
        ctor.ins_size,
    )
    .unwrap();
    let family = initializers::plan_constructor_body_subset(
        dex,
        class,
        &[ConstructorInput {
            reference: MethodRef::new(15),
            ir: cs.ir(),
            bindings: &binding,
        }],
        &r,
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!("decomphify-branch-static-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = write_java(
        &dir,
        "a2/a$c.java",
        &format!(
            "package a2; public class a$c {{ public final boolean a; public final Throwable b; public static final a$c c,d; {} {} {} }}",
            plan.encoded.prefix_block, plan.source, family.constructors[0].source
        ),
    );
    // Explicit dependency boundary: genuine input field d, configurable initializer for traces.
    // This does not claim recovery of the original owner a's initialization.
    let raw = dex.field_id(57).unwrap();
    assert_eq!(descriptor(dex, u32::from(raw.type_idx)), "Z");
    let owner = write_java(
        &dir,
        "a2/a.java",
        "package a2; public class a { public static final boolean d; static {d=Boolean.parseBoolean(System.getProperty(\"test.cause\"));} }",
    );
    javac(&dir, &[file, owner]);
    let h = write_java(
        &dir,
        "BranchHarness.java",
        "public class BranchHarness { public static void main(String[] args) { boolean cause=Boolean.parseBoolean(args[0]); if(cause) {if(a2.a$c.c!=null || a2.a$c.d!=null) throw new AssertionError();} else {if(a2.a$c.c==null || a2.a$c.d==null || a2.a$c.c==a2.a$c.d || !a2.a$c.c.a || a2.a$c.d.a || a2.a$c.c.b!=null || a2.a$c.d.b!=null) throw new AssertionError();} } }",
    );
    javac(
        &dir,
        &["-classpath".into(), dir.to_string_lossy().into(), h],
    );
    for cause in ["false", "true"] {
        let output = Command::new(java_home().join("bin/java"))
            .arg(format!("-Dtest.cause={cause}"))
            .arg("-classpath")
            .arg(&dir)
            .arg("BranchHarness")
            .arg(cause)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stores = dex_decompiler::decode_method(code)
        .unwrap()
        .into_iter()
        .filter(|i| (0x67..=0x6d).contains(&code.insns[i.offset as usize * 2]))
        .collect::<Vec<_>>();
    assert_eq!(stores.len(), 4);
    for duplicate in [false, true] {
        let offset = stores[0].offset as usize * 2;
        let mut altered = code.clone();
        if duplicate {
            let bytes = altered.insns[offset..offset + 4].to_vec();
            altered.insns.splice(offset..offset, bytes);
            altered.insns_size += 2;
        } else {
            altered.insns[offset..offset + 4].fill(0);
        }
        // Insertions would require branch-offset relocation; same-width mutation gives
        // a second write to the other final on this path without changing CFG targets.
        if duplicate {
            altered = code.clone();
            let from = stores[0].offset as usize * 2;
            let to = stores[1].offset as usize * 2;
            altered.insns[to..to + 4].copy_from_slice(&code.insns[from..from + 4]);
        }
        let changed = dex_decompiler::DexPasses::run_ssa_with(
            &altered,
            &dex_decompiler::DexTypes::new(dex, Some(14)),
        )
        .unwrap();
        assert!(
            initializers::plan_static_initializer_body_subset(
                &inputs,
                0,
                class,
                MethodRef::new(14),
                changed.ir(),
                &r,
                &inv
            )
            .is_err(),
            "missing/double branch assignment duplicate={duplicate}"
        );
    }
    assert!(!inv.unknown_reasons().is_empty());
    eprintln!(
        "actual a2/a$c#14 body compiles/runs both paths with explicit dependency boundary; final missing/double paths reject"
    );
}
