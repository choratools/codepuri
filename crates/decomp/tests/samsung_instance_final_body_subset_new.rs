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

fn changed_opcode(code: &dex_format::code::CodeItem, offset: usize) -> u8 {
    code.insns[offset * 2]
}
fn field_source_type(ty: &JavaType) -> String {
    match ty {
        JavaType::Boolean => "boolean".into(),
        JavaType::Class(s) => s.clone(),
        JavaType::Int => "int".into(),
        _ => panic!("unexpected field {ty:?}"),
    }
}
#[test]
#[ignore = "requires retained Samsung APK and explicit local JDK17"]
fn real_instance_final_families_preserve_body_proof_only() {
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let dex = &inputs[0];
    let cp = Classpath::from_paths([java_home().join("jmods/java.base.jmod")]).unwrap();
    let inventory = WriteInventory::scan(&inputs);
    assert!(!inventory.unknown_reasons().is_empty());
    let dir =
        std::env::temp_dir().join(format!("decomphify-instance-finals-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut count = 0;
    for class in &dex.classes {
        let target = descriptor(dex, class.class_idx);
        let Some(data) = class.class_data.as_ref() else {
            continue;
        };
        if !data
            .instance_fields
            .iter()
            .any(|f| f.access_flags & 16 != 0)
        {
            continue;
        }
        if descriptor(dex, class.superclass_idx) != "Ljava/lang/Object;" {
            continue;
        }
        let class = dex
            .classes
            .iter()
            .find(|c| descriptor(dex, c.class_idx) == target)
            .unwrap();
        let data = class.class_data.as_ref().unwrap();
        let resolver = Resolver {
            inputs: &inputs,
            dex,
            class,
            cp: &cp,
            super_proof: true,
        };
        let methods = data
            .direct_methods
            .iter()
            .filter(|m| method_key(dex, m.method_idx).name == "<init>")
            .collect::<Vec<_>>();
        if methods.len() != 1 {
            continue;
        }
        let index = methods[0].method_idx;
        if data.instance_fields.iter().any(|f| {
            let t = descriptor(dex, u32::from(dex.field_id(f.field_idx).unwrap().type_idx));
            t.starts_with('L') && !t.starts_with("Ljava/") && t != target
        }) {
            continue;
        }
        let code = methods[0].code.as_ref().unwrap();
        let Ok(ssa) = dex_decompiler::DexPasses::run_ssa_with(
            code,
            &dex_decompiler::DexTypes::new(dex, Some(index)),
        ) else {
            continue;
        };
        if ssa
            .ir()
            .instructions()
            .values()
            .filter(|i| matches!(i, managed_ir::insn::ManagedInsn::Invoke { .. }))
            .count()
            != 1
        {
            continue;
        }
        let signature = resolver.method(MethodRef::new(index)).unwrap();
        let Ok(bindings) = decomp::dex_java::parameter_bindings(
            ssa.ir(),
            &signature,
            code.registers_size,
            code.ins_size,
        ) else {
            continue;
        };
        let input = ConstructorInput {
            reference: MethodRef::new(index),
            ir: ssa.ir(),
            bindings: &bindings,
        };
        let Ok(subset) =
            initializers::plan_constructor_body_subset(dex, class, &[input], &resolver)
        else {
            continue;
        };
        if data.instance_fields.iter().any(|f| {
            !matches!(
                resolver.field(FieldRef::new(f.field_idx)).unwrap().ty,
                JavaType::Boolean | JavaType::Int | JavaType::Class(_)
            )
        }) {
            continue;
        }
        assert!(
            subset.constructor_evidence[0]
                .fields
                .iter()
                .all(|f| f.before_chain.is_empty()
                    && f.normal_return_counts == [1].into_iter().collect())
        );
        let fields = data
            .instance_fields
            .iter()
            .map(|f| {
                let mapped = resolver.field(FieldRef::new(f.field_idx)).unwrap();
                format!(
                    "public {}{} {};",
                    if f.access_flags & 16 != 0 {
                        "final "
                    } else {
                        ""
                    },
                    field_source_type(&mapped.ty),
                    mapped.name
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let owner = source_name(target);
        let Some((package, name)) = owner.rsplit_once('.') else {
            continue;
        };
        let file = write_java(
            &dir,
            &format!("{}/{name}.java", package.replace('.', "/")),
            &format!(
                "package {package}; public class {name} {{ {fields} {} }}",
                subset.constructors[0].source
            ),
        );
        javac(&dir, &[file]);
        assert!(initializers::plan_constructor_body_subset(dex, class, &[], &resolver).is_err());
        // Remove and duplicate one original store, retaining the exact metadata/family.
        let mut offset = 0usize;
        let decoded = dex_decompiler::decode_method(code).unwrap();
        for insn in &decoded {
            if (0x59..=0x5f).contains(&changed_opcode(code, insn.offset as usize)) {
                offset = insn.offset as usize * 2;
                break;
            }
        }
        assert!(offset > 0);
        for duplicate in [false, true] {
            let mut changed = code.clone();
            if duplicate {
                let bytes = changed.insns[offset..offset + 4].to_vec();
                changed.insns.splice(offset..offset, bytes);
                changed.insns_size += 2;
            } else {
                changed.insns[offset..offset + 4].fill(0);
            }
            let changed_ssa = dex_decompiler::DexPasses::run_ssa_with(
                &changed,
                &dex_decompiler::DexTypes::new(dex, Some(index)),
            )
            .unwrap();
            let Ok(changed_binding) = decomp::dex_java::parameter_bindings(
                changed_ssa.ir(),
                &signature,
                changed.registers_size,
                changed.ins_size,
            ) else {
                assert!(!duplicate);
                continue;
            };
            let changed_input = ConstructorInput {
                reference: MethodRef::new(index),
                ir: changed_ssa.ir(),
                bindings: &changed_binding,
            };
            assert!(
                initializers::plan_constructor_body_subset(dex, class, &[changed_input], &resolver)
                    .is_err(),
                "mutation duplicate={duplicate} {target}"
            );
        }
        eprintln!(
            "exact instance-final constructor {target} index={index}: compiled, missing/double stores rejected"
        );
        count += 1;
        if count == 3 {
            break;
        }
    }
    let harness = write_java(
        &dir,
        "ConstructorIdentityHarness.java",
        r#"
public class ConstructorIdentityHarness {
 public static void main(String[] args) {
  Throwable t = new Throwable("original");
  a2.a$c c = new a2.a$c(true, t);
  if (!c.a || c.b != t) throw new AssertionError();
  Runnable r = () -> {};
  java.util.concurrent.Executor x = Runnable::run;
  a2.a$e e = new a2.a$e(r, x);
  if (e.a != r || e.b != x || e.c != null) throw new AssertionError();
  androidx.appcompat.widget.m1 m = new androidx.appcompat.widget.m1(r);
  if (m.a != r) throw new AssertionError();
 }
}
"#,
    );
    javac(
        &dir,
        &["-classpath".into(), dir.to_string_lossy().into(), harness],
    );
    let output = Command::new(java_home().join("bin/java"))
        .arg("-classpath")
        .arg(&dir)
        .arg("ConstructorIdentityHarness")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(count, 3);
    assert!(!inventory.unknown_reasons().is_empty());
}
