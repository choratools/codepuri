//! Original static final store provenance and path definite assignment.
#![allow(clippy::unwrap_used)]
use backend_java::constructors::{
    ConstructorClass, ConstructorInput, ConstructorResolver, Initializer,
    TrustedConstructorResolver,
};
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType};
use decomp::java_class_kinds::initializers::{self, write_inventory::WriteInventory};
use decomp::java_external_metadata::{Classpath, ExternalMetadata, MethodMetadata};
use dex_format::{
    DexFile, RawSpan,
    class::{ClassData, ClassDef, EncodedField, EncodedMethod},
    encoded_value::{EncodedValue, EncodedValueKind as V},
    ids::{FieldId, MethodId, ProtoId, TypeId},
};
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
fn base_classpath() -> &'static Classpath {
    static BASE: std::sync::OnceLock<Classpath> = std::sync::OnceLock::new();
    BASE.get_or_init(|| Classpath::from_paths([java_home().join("jmods/java.base.jmod")]).unwrap())
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

const SPAN: RawSpan = RawSpan { offset: 0, size: 0 };
fn string(dex: &mut DexFile, text: &str) -> u32 {
    let mut s = dex.strings[0].clone();
    s.value = text.into();
    let index = dex.strings.len() as u32;
    dex.strings.push(s);
    index
}
fn ty(dex: &mut DexFile, text: &str) -> u32 {
    let descriptor_idx = string(dex, text);
    let index = dex.type_ids.len() as u32;
    dex.type_ids.push(TypeId {
        descriptor_idx,
        span: SPAN,
    });
    index
}
fn fixture(flags: u32) -> (DexFile, ClassDef) {
    let mut dex = DexFile::from_bytes(include_bytes!(
        "../../../tests/corpus/dex/clean/classes.dex"
    ))
    .unwrap();
    let mut class = dex.classes[0].clone();
    class.class_idx = ty(&mut dex, "LFixture;");
    class.access_flags = flags;
    class.superclass_idx = ty(&mut dex, "Ljava/lang/Object;");
    class.interfaces = None;
    class.annotations = None;
    class.static_values = None;
    class.class_data = Some(ClassData {
        static_fields: vec![],
        instance_fields: vec![],
        direct_methods: vec![],
        virtual_methods: vec![],
        span: SPAN,
    });
    (dex, class)
}
fn element(dex: &mut DexFile, class: &mut ClassDef, name: &str, descriptor: &str) -> EncodedMethod {
    let return_type_idx = ty(dex, descriptor);
    let proto_idx = dex.proto_ids.len() as u16;
    dex.proto_ids.push(ProtoId {
        shorty_idx: 0,
        return_type_idx,
        parameters_off: 0,
        parameters: None,
        span: SPAN,
    });
    let name_idx = string(dex, name);
    let method_idx = dex.method_ids.len() as u32;
    dex.method_ids.push(MethodId {
        class_idx: class.class_idx as u16,
        proto_idx,
        name_idx,
        span: SPAN,
    });
    let method = EncodedMethod {
        method_idx,
        access_flags: 0x401,
        code_off: 0,
        code: None,
    };
    class
        .class_data
        .as_mut()
        .unwrap()
        .virtual_methods
        .push(method.clone());
    method
}
fn field(dex: &mut DexFile, class: &mut ClassDef, name: &str, descriptor: &str, flags: u32) -> u32 {
    let type_idx = ty(dex, descriptor) as u16;
    let name_idx = string(dex, name);
    let index = dex.field_ids.len() as u32;
    dex.field_ids.push(FieldId {
        class_idx: class.class_idx as u16,
        type_idx,
        name_idx,
        span: SPAN,
    });
    let f = EncodedField {
        field_idx: index,
        access_flags: flags,
    };
    if flags & 8 != 0 {
        class.class_data.as_mut().unwrap().static_fields.push(f);
    } else {
        class.class_data.as_mut().unwrap().instance_fields.push(f);
    }
    index
}
fn code(units: &[u16]) -> dex_format::code::CodeItem {
    dex_format::code::CodeItem {
        registers_size: 4,
        ins_size: 0,
        outs_size: 0,
        debug_info_off: 0,
        insns_size: units.len() as u32,
        insns: units.iter().flat_map(|u| u.to_le_bytes()).collect(),
        tries: vec![],
        handlers: vec![],
        span: SPAN,
    }
}

fn initializer(dex: &mut DexFile, class: &mut ClassDef, units: &[u16]) -> u32 {
    let mut m = element(dex, class, "<clinit>", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    m.access_flags = 8;
    m.code_off = 1;
    m.code = Some(code(units));
    let index = m.method_idx;
    class.class_data.as_mut().unwrap().direct_methods.push(m);
    dex.classes = vec![class.clone()];
    index
}
fn typed_fixture(ty: &str, encoded: Option<V>) -> (DexFile, ClassDef, u32, u32) {
    let (mut dex, mut class) = fixture(1);
    let field = field(&mut dex, &mut class, "N", ty, 0x19);
    class.static_values = encoded.map(|value| vec![EncodedValue { value, span: SPAN }]);
    let opcode = if ty.starts_with('L') || ty.starts_with('[') {
        0x0069
    } else if ty == "J" || ty == "D" {
        0x0068
    } else {
        match ty {
            "Z" => 0x006a,
            "B" => 0x006b,
            "C" => 0x006c,
            "S" => 0x006d,
            _ => 0x0067,
        }
    };
    let units = if ty == "J" || ty == "D" {
        vec![0x0016, 0, opcode, field as u16, 0x000e]
    } else {
        vec![0x0012, opcode, field as u16, 0x000e]
    };
    let method = initializer(&mut dex, &mut class, &units);
    (dex, class, method, field)
}
fn probe(
    dex: &DexFile,
    class: &ClassDef,
    method: u32,
) -> Result<
    initializers::ExplicitStorePlan<initializers::StaticInitializerBodySubset>,
    decomp::java_class_kinds::DeclarationUnsupported,
> {
    let inputs = std::slice::from_ref(dex);
    let inv = WriteInventory::scan(inputs);
    let cp = base_classpath();
    let r = Resolver {
        inputs,
        dex,
        class,
        cp,
        super_proof: true,
    };
    let original = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|m| m.method_idx == method)
        .unwrap();
    let ssa = dex_decompiler::DexPasses::run_ssa_with(
        original.code.as_ref().unwrap(),
        &dex_decompiler::DexTypes::new(dex, Some(method)),
    )
    .unwrap();
    initializers::plan_static_initializer_explicit_stores_attested(
        inputs,
        0,
        class,
        MethodRef::new(method),
        &ssa,
        &r,
        &inv,
    )
}

#[test]
fn bit_and_type_exact_encoded_vm_defaults_require_original_stores() {
    for (ty, value, allowed) in [
        ("Ljava/lang/Object;", V::Null, true),
        ("[I", V::Null, true),
        ("I", V::Int(0), true),
        ("Z", V::Boolean(false), true),
        ("B", V::Byte(0), true),
        ("S", V::Short(0), true),
        ("C", V::Char(0), true),
        ("I", V::Int(1), false),
        ("I", V::Null, false),
        ("I", V::Byte(0), false),
        ("F", V::Float(0.0), true),
        ("F", V::Float(-0.0), false),
        ("D", V::Double(0.0), true),
        ("D", V::Double(-0.0), false),
        ("J", V::Long(0), true),
    ] {
        let (dex, class, method, field) = typed_fixture(ty, Some(value));
        let result = probe(&dex, &class, method);
        assert_eq!(
            result.is_ok(),
            allowed,
            "field descriptor {ty}: {:?}",
            result.as_ref().err()
        );
        if allowed {
            let proof = result.unwrap();
            assert_eq!(proof.plan.encoded.blank_static_finals, [field].into());
            assert!(proof.plan.encoded.prefix_block.is_empty());
            assert!(proof.plan.source.contains("N ="));
            // No provenance means no newly relaxed permission.
            let inv = WriteInventory::scan(std::slice::from_ref(&dex));
            assert!(
                initializers::plan_encoded_initialization_explicit_stores(&dex, &class, &inv)
                    .is_err()
            );
        }
    }
}

#[test]
fn exclusive_branch_stores_need_all_paths_and_no_external_writers() {
    let (mut dex, mut class) = fixture(1);
    let n = field(&mut dex, &mut class, "N", "I", 0x19);
    let m = field(&mut dex, &mut class, "M", "I", 0x19);
    let units = [
        0x1012, 0x0038, 8, 0x7012, 0x0067, n as u16, 0x0067, m as u16, 0x000e, 0x0012, 0x0067,
        n as u16, 0x0067, m as u16, 0x000e,
    ];
    let index = initializer(&mut dex, &mut class, &units);
    let proof = probe(&dex, &class, index).unwrap();
    assert_eq!(proof.plan.encoded.blank_static_finals, [n, m].into());
    assert!(proof.plan.source.contains("if ("));
    assert_eq!(proof.plan.source.matches("N =").count(), 2);
    assert_eq!(proof.plan.source.matches("M =").count(), 2);
    for duplicate in [false, true] {
        let mut changed = dex.clone();
        let raw = changed.classes[0]
            .class_data
            .as_mut()
            .unwrap()
            .direct_methods[0]
            .code
            .as_mut()
            .unwrap();
        if duplicate {
            raw.insns[14..16].copy_from_slice(&(n as u16).to_le_bytes());
        } else {
            raw.insns[12..16].fill(0);
        }
        assert!(probe(&changed, &changed.classes[0], index).is_err());
    }
    let mut changed = dex.clone();
    let mut writer = changed.classes[0]
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods[0]
        .clone();
    let mut raw = changed.method_ids[index as usize].clone();
    raw.name_idx = string(&mut changed, "outside");
    writer.method_idx = changed.method_ids.len() as u32;
    changed.method_ids.push(raw);
    writer.code = Some(code(&[0x0012, 0x0067, n as u16, 0x000e]));
    changed.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(writer);
    assert!(probe(&changed, &changed.classes[0], index).is_err());

    // Equal field/store count is insufficient: original source offsets must match.
    let inv = WriteInventory::scan(std::slice::from_ref(&dex));
    let cp = base_classpath();
    let inputs = std::slice::from_ref(&dex);
    let r = Resolver {
        inputs,
        dex: &dex,
        class: &class,
        cp,
        super_proof: true,
    };
    let mut shifted = class.class_data.as_ref().unwrap().direct_methods[0]
        .code
        .clone()
        .unwrap();
    shifted.insns.splice(0..0, [0, 0]);
    shifted.insns_size += 1;
    let ssa = dex_decompiler::DexPasses::run_ssa_with(
        &shifted,
        &dex_decompiler::DexTypes::new(&dex, Some(index)),
    )
    .unwrap();
    assert!(
        initializers::plan_static_initializer_explicit_stores_attested(
            inputs,
            0,
            &class,
            MethodRef::new(index),
            &ssa,
            &r,
            &inv
        )
        .is_err()
    );

    let missing = code(&[0x0012, 0x0067, n as u16, 0x000e]);
    let ssa = dex_decompiler::DexPasses::run_ssa_with(
        &missing,
        &dex_decompiler::DexTypes::new(&dex, Some(index)),
    )
    .unwrap();
    assert!(
        initializers::plan_static_initializer_explicit_stores_attested(
            inputs,
            0,
            &class,
            MethodRef::new(index),
            &ssa,
            &r,
            &inv
        )
        .is_err()
    );
}

#[test]
fn encoded_defaults_do_not_hide_preassignment_reads_or_missing_normal_paths() {
    let (mut dex, mut class, index, field) = typed_fixture("Ljava/lang/Object;", Some(V::Null));
    for units in [
        vec![0x0062, field as u16, 0x0069, field as u16, 0x000e],
        vec![0x0012, 0x0027],
    ] {
        class.class_data.as_mut().unwrap().direct_methods[0].code = Some(code(&units));
        dex.classes = vec![class.clone()];
        assert!(probe(&dex, &class, index).is_err());
    }
}
#[test]
#[ignore = "requires retained Samsung APK and JDK17"]
fn actual_branch_static_attested_proof_compiles_and_runs() {
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
    let cp = base_classpath();
    let r = Resolver {
        inputs: &inputs,
        dex,
        class,
        cp,
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
    let plan = initializers::plan_static_initializer_explicit_stores_attested(
        &inputs,
        0,
        class,
        MethodRef::new(14),
        &ssa,
        &r,
        &inv,
    )
    .unwrap()
    .plan;
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

    // Full-class proof uses the same attested static plan and exact original constructor.
    let full = initializers::plan_class_initialization_with_explicit_stores_attested(
        &inputs,
        0,
        class,
        &[ConstructorInput {
            reference: MethodRef::new(15),
            ir: cs.ir(),
            bindings: &binding,
        }],
        Some((MethodRef::new(14), &ssa)),
        &r,
        &inv,
    )
    .unwrap();
    assert_eq!(full.plan.encoded.assignments, plan.encoded.assignments);
    assert_eq!(full.plan.encoded.prefix_block, plan.encoded.prefix_block);
    assert_eq!(
        full.plan.encoded.blank_static_finals,
        plan.encoded.blank_static_finals
    );
    assert_eq!(full.plan.encoded.blank_instance_finals, [36, 37].into());
    assert_eq!(
        full.plan.static_initializer.as_deref(),
        Some(plan.source.as_str())
    );
    for field in [38, 39] {
        let key = initializers::write_inventory::field_key(dex, field).unwrap();
        assert_eq!(inv.explicit_store_writes(&key).unwrap().len(), 2);
    }
}

#[test]
#[ignore = "requires retained Samsung APK and JDK17"]
fn actual_fragment_encoded_null_is_vm_default_not_duplicate_assignment() {
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let dex = &inputs[0];
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == "Landroidx/fragment/app/Fragment;")
        .unwrap();
    let cp = base_classpath();
    let r = Resolver {
        inputs: &inputs,
        dex,
        class,
        cp,
        super_proof: true,
    };
    let original = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|m| m.method_idx == 13501)
        .unwrap();
    let pos = class
        .class_data
        .as_ref()
        .unwrap()
        .static_fields
        .iter()
        .position(|f| f.field_idx == 6452)
        .unwrap();
    assert!(matches!(
        class.static_values.as_ref().unwrap()[pos].value,
        V::Null
    ));
    let ssa = dex_decompiler::DexPasses::run_ssa_with(
        original.code.as_ref().unwrap(),
        &dex_decompiler::DexTypes::new(dex, Some(13501)),
    )
    .unwrap();
    let inv = WriteInventory::scan(&inputs);
    assert!(
        initializers::plan_static_initializer_explicit_stores(
            &inputs,
            0,
            class,
            MethodRef::new(13501),
            ssa.ir(),
            &r,
            &inv
        )
        .is_err()
    );
    let proof = initializers::plan_static_initializer_explicit_stores_attested(
        &inputs,
        0,
        class,
        MethodRef::new(13501),
        &ssa,
        &r,
        &inv,
    )
    .unwrap();
    assert_eq!(proof.plan.encoded.blank_static_finals, [6452].into());
    assert!(
        !proof
            .plan
            .encoded
            .prefix_block
            .contains("USE_DEFAULT_TRANSITION")
    );
    assert_eq!(
        proof
            .plan
            .source
            .matches("USE_DEFAULT_TRANSITION =")
            .count(),
        1
    );
    assert!(proof.plan.source.contains("new java.lang.Object("));
    let dir =
        std::env::temp_dir().join(format!("decomphify-fragment-final-{}", std::process::id()));
    let declarations = class
        .class_data
        .as_ref()
        .unwrap()
        .static_fields
        .iter()
        .map(|f| {
            let mapped = r.field(FieldRef::new(f.field_idx)).unwrap();
            let ty = match mapped.ty {
                JavaType::Boolean => "boolean".into(),
                JavaType::Int => "int".into(),
                JavaType::Class(name) => name,
                other => panic!("unexpected Fragment static type: {other:?}"),
            };
            format!(
                "public static {} {ty} {};",
                if f.access_flags & 0x10 != 0 {
                    "final"
                } else {
                    ""
                },
                mapped.name
            )
        })
        .collect::<String>();
    let file = write_java(
        &dir,
        "androidx/fragment/app/Fragment.java",
        &format!(
            "package androidx.fragment.app; public class Fragment {{ {declarations} {} {} }}",
            proof.plan.encoded.prefix_block, proof.plan.source
        ),
    );
    let harness = write_java(
        &dir,
        "FragmentHarness.java",
        "public class FragmentHarness {public static void main(String[]args){Object x=androidx.fragment.app.Fragment.USE_DEFAULT_TRANSITION;if(x==null||x!=androidx.fragment.app.Fragment.USE_DEFAULT_TRANSITION||x.getClass()!=Object.class)throw new AssertionError();}}",
    );
    javac(&dir, &[file, harness]);
    let run = Command::new(java_home().join("bin/java"))
        .args(["-cp", dir.to_str().unwrap(), "FragmentHarness"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn original_reentrant_reader_observes_default_until_original_final_store() {
    let (mut dex, mut class) = fixture(1);
    let n = field(&mut dex, &mut class, "N", "Ljava/lang/Object;", 0x19);
    let seen = field(&mut dex, &mut class, "seen", "Ljava/lang/Object;", 9);
    class.static_values = Some(vec![EncodedValue {
        value: V::Null,
        span: SPAN,
    }]);
    let mut peek = element(&mut dex, &mut class, "peek", "Ljava/lang/Object;");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    peek.access_flags = 9;
    peek.code_off = 1;
    peek.code = Some(code(&[0x0062, n as u16, 0x0011]));
    class
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(peek.clone());
    let object = class.superclass_idx;
    let constructor = element(&mut dex, &mut class, "<init>", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    dex.method_ids[constructor.method_idx as usize].class_idx = object as u16;
    let index = initializer(
        &mut dex,
        &mut class,
        &[
            0x0071,
            peek.method_idx as u16,
            0,
            0x010c,
            0x0169,
            seen as u16,
            0x0022,
            object as u16,
            0x1070,
            constructor.method_idx as u16,
            0,
            0x0069,
            n as u16,
            0x000e,
        ],
    );
    let inputs = std::slice::from_ref(&dex);
    let cp = base_classpath();
    let r = Resolver {
        inputs,
        dex: &dex,
        class: &class,
        cp,
        super_proof: true,
    };
    let proof = probe(&dex, &class, index).unwrap();
    let peek_ssa = dex_decompiler::DexPasses::run_ssa_with(
        peek.code.as_ref().unwrap(),
        &dex_decompiler::DexTypes::new(&dex, Some(peek.method_idx)),
    )
    .unwrap();
    let signature = r.method(MethodRef::new(peek.method_idx)).unwrap();
    let backend_java::SourceResult::Complete {
        source: peek_source,
        ..
    } = backend_java::emit_method(
        peek_ssa.ir(),
        &signature,
        &backend_java::ParameterBindings::default(),
        &r,
    )
    else {
        panic!("original reader unavailable")
    };
    let dir =
        std::env::temp_dir().join(format!("decomphify-reentrant-final-{}", std::process::id()));
    let file = write_java(
        &dir,
        "Fixture.java",
        &format!(
            "public class Fixture {{static final Object N;static Object seen; {} {} {} public static void main(String[]args){{if(seen!=null||N==null)throw new AssertionError();}}}}",
            proof.plan.encoded.prefix_block, proof.plan.source, peek_source
        ),
    );
    javac(&dir, &[file]);
    let output = Command::new(java_home().join("bin/java"))
        .args(["-cp", dir.to_str().unwrap(), "Fixture"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires retained Samsung APK and trusted Android35 declarations"]
fn actual_item_inventory_scopes_layoutparams_writer_and_keeps_constructor_chain_evidence() {
    let inputs = dex_format::apk::parse_apk_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk"
    ))
    .unwrap();
    let dex = &inputs[2];
    let class = dex
        .classes
        .iter()
        .find(|c| {
            descriptor(dex, c.class_idx) == "Lcom/samsung/android/samsungpass/otp/SignInInfo$Item;"
        })
        .unwrap();
    let cp = Classpath::from_paths([
        java_home().join("jmods/java.base.jmod"),
        PathBuf::from("/home/jtjisgod/Android/Sdk/platforms/android-35/android.jar"),
    ])
    .unwrap();
    let r = Resolver {
        inputs: &inputs,
        dex,
        class,
        cp: &cp,
        super_proof: true,
    };
    let old = WriteInventory::scan(&inputs);
    let scoped = WriteInventory::scan_with_metadata(&inputs, &cp);
    assert!(!scoped.has_structural_unknown());
    assert_eq!(old.unknown_observations(), scoped.unknown_observations());
    let key = initializers::write_inventory::field_key(dex, 2384).unwrap();
    assert_eq!(key.name, "packageName");
    assert!(matches!(
        old.explicit_store_writes(&key),
        Err(initializers::write_inventory::ExplicitStoreError::SymbolicFieldUnknown(_))
    ));
    let writes = scoped.explicit_store_writes(&key).unwrap();
    assert_eq!(
        writes,
        [initializers::write_inventory::WriteSite {
            dex: 2,
            method: 4945,
            offset: 25,
            is_static: false
        }]
        .into()
    );
    assert!(scoped.proven_writes(&key).is_none());
    let external = initializers::write_inventory::field_key(&inputs[0], 722).unwrap();
    assert_eq!(external.owner, "Landroid/view/WindowManager$LayoutParams;");
    assert!(scoped.known_writes(&external).unwrap().contains(
        &initializers::write_inventory::WriteSite {
            dex: 0,
            method: 8803,
            offset: 110,
            is_static: false
        }
    ));
    let refs = class
        .class_data
        .as_ref()
        .unwrap()
        .instance_fields
        .iter()
        .map(|f| FieldRef::new(f.field_idx))
        .collect::<Vec<_>>();
    for (index, kind, target, counts) in [
        (4945, backend_java::constructors::ChainKind::Super, 4963, 1),
        (4946, backend_java::constructors::ChainKind::This, 4945, 0),
    ] {
        let original = class
            .class_data
            .as_ref()
            .unwrap()
            .direct_methods
            .iter()
            .find(|m| m.method_idx == index)
            .unwrap();
        let code = original.code.as_ref().unwrap();
        let ssa = dex_decompiler::DexPasses::run_ssa_with(
            code,
            &dex_decompiler::DexTypes::new(dex, Some(index)),
        )
        .unwrap();
        let receiver_reg = code.registers_size - code.ins_size;
        let receiver = ssa
            .initial_registers()
            .iter()
            .find(|(_, registers)| registers.contains(&receiver_reg))
            .map(|(value, _)| *value)
            .unwrap();
        // Local evidence needs the original receiver, not fabricated bindings for
        // unused/defaulted parameters; complete source emission remains separate.
        let bindings = backend_java::DeclarationBindings {
            receiver: Some(receiver),
            parameters: vec![None; r.method(MethodRef::new(index)).unwrap().parameters.len()],
            names: vec![],
        };
        let input = ConstructorInput {
            reference: MethodRef::new(index),
            ir: ssa.ir(),
            bindings: &bindings,
        };
        let evidence = backend_java::constructors::constructor_initialization_evidence(
            &input,
            &source_name(descriptor(dex, class.superclass_idx)),
            &refs,
            &r,
        )
        .unwrap();
        assert_eq!(
            evidence.normal_chains,
            [(kind, MethodRef::new(target))].into()
        );
        for field in evidence.fields {
            assert!(field.before_chain.is_empty());
            assert_eq!(field.after_chain.len(), counts as usize);
            assert_eq!(field.normal_return_counts, [counts].into());
            assert!(!field.read_before_assignment);
        }
    }
}
