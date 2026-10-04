//! Explicit DEX stores and runtime capability evidence are independent contracts.
#![allow(clippy::unwrap_used)]
use decomp::java_class_kinds::{DeclarationUnsupported as Error, initializers::*};
use dex_format::{
    DexFile, RawSpan,
    class::{ClassData, ClassDef, EncodedField, EncodedMethod},
    ids::{FieldId, MethodId, ProtoId, TypeId},
};
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

fn native(dex: &mut DexFile, class: &mut ClassDef) {
    let mut m = element(dex, class, "nativeMutation", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    m.access_flags = 0x109;
    class.class_data.as_mut().unwrap().direct_methods.push(m);
}
#[test]
fn native_capability_retained_while_encoded_blank_final_is_not_inlined() {
    let (mut dex, mut class) = fixture(1);
    field(&mut dex, &mut class, "N", "I", 0x19);
    native(&mut dex, &mut class);
    dex.classes = vec![class.clone()];
    let inv = write_inventory::WriteInventory::scan(std::slice::from_ref(&dex));
    assert_eq!(
        plan_encoded_initialization(&dex, &class, &inv),
        Err(Error::FieldInitializationUnavailable)
    );
    let explicit = plan_encoded_initialization_explicit_stores(&dex, &class, &inv).unwrap();
    assert_eq!(explicit.runtime_capabilities.len(), 1);
    assert!(matches!(
        explicit.runtime_capabilities[0].kind,
        write_inventory::UnknownKind::NativeCapability { .. }
    ));
    assert!(explicit.plan.prefix_block.contains("N = 0"));
    assert!(!explicit.plan.prefix_block.contains("final"));
    let mut structural = dex.clone();
    structural.classes[0]
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods[0]
        .access_flags = 9;
    let inv = write_inventory::WriteInventory::scan(&[structural]);
    assert!(plan_encoded_initialization_explicit_stores(&dex, &class, &inv).is_err());
}
#[test]
fn out_of_constructor_store_is_not_filtered_even_when_unreachable() {
    let (mut dex, mut class) = fixture(1);
    let f = field(&mut dex, &mut class, "F", "I", 0x11);
    let mut m = element(&mut dex, &mut class, "writer", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    m.access_flags = 1;
    m.code_off = 1;
    m.code = Some(code(&[0x000e, 0x1059, f as u16, 0x000e]));
    class.class_data.as_mut().unwrap().direct_methods.push(m);
    dex.classes = vec![class.clone()];
    let inv = write_inventory::WriteInventory::scan(std::slice::from_ref(&dex));
    assert!(
        prove_instance_final_explicit_stores(&[dex], 0, &class, &Default::default(), &inv).is_err()
    );
}
#[test]
#[ignore = "explicit local Samsung corpus"]
fn actual_a4_constructors_keep_original_store_owner_and_delegation() {
    let inputs = dex_format::apk::parse_apk_path(std::path::Path::new(
        "../../tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    ))
    .unwrap();
    let inv = write_inventory::WriteInventory::scan(&inputs);
    let dex = &inputs[0];
    let class = dex
        .classes
        .iter()
        .find(|c| {
            dex.string(dex.type_ids[c.class_idx as usize].descriptor_idx)
                .unwrap()
                == "La4/a$c;"
        })
        .unwrap();
    let proof =
        prove_instance_final_explicit_stores(&inputs, 0, class, &[110, 111].into(), &inv).unwrap();
    assert!(!proof.runtime_capabilities.is_empty());
    assert!(!proof.plan.is_empty());
    for f in proof.plan {
        assert!(
            f.writes
                .contains_key(&managed_ir::refs::MethodRef::new(110))
        );
        assert!(
            !f.writes
                .contains_key(&managed_ir::refs::MethodRef::new(111))
        );
    }
}
#[test]
fn reflected_final_remains_runtime_mutable_without_constant_inlining() {
    let root = std::env::temp_dir().join(format!("explicit-final-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("ExplicitFinal.java"),r#"public class ExplicitFinal {public final int value;ExplicitFinal(int x){value=x;}int get(){return value;}public static void main(String[]x)throws Exception{ExplicitFinal a=new ExplicitFinal(7);java.lang.reflect.Field f=ExplicitFinal.class.getField("value");f.setAccessible(true);f.setInt(a,19);if(a.get()!=19)throw new AssertionError();}}"#).unwrap();
    for (tool, args) in [
        ("javac", vec!["ExplicitFinal.java"]),
        ("java", vec!["-cp", ".", "ExplicitFinal"]),
    ] {
        let o =
            std::process::Command::new(format!("/home/jtjisgod/tools/jdk-17.0.13+11/bin/{tool}"))
                .args(args)
                .current_dir(&root)
                .output()
                .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
}

struct EmptyResolver<'a>(&'a DexFile);
impl backend_java::JavaResolver for EmptyResolver<'_> {
    fn method(&self, r: managed_ir::refs::MethodRef) -> Option<backend_java::JavaMethod> {
        let id = self.0.method_ids.get(r.index() as usize)?;
        Some(backend_java::JavaMethod {
            owner: self.class(managed_ir::refs::ClassRef::new(u32::from(id.class_idx)))?,
            name: self.0.string(id.name_idx).ok()?.into(),
            is_static: false,
            parameters: vec![],
            return_type: backend_java::JavaType::Void,
            throws: vec![],
        })
    }
    fn field(&self, r: managed_ir::refs::FieldRef) -> Option<backend_java::JavaField> {
        let id = self.0.field_ids.get(r.index() as usize)?;
        Some(backend_java::JavaField {
            owner: "Fixture".into(),
            name: self.0.string(id.name_idx).ok()?.into(),
            ty: backend_java::JavaType::Int,
            is_static: true,
        })
    }
    fn class(&self, r: managed_ir::refs::ClassRef) -> Option<String> {
        let desc = self
            .0
            .string(self.0.type_id(r.index()).ok()?.descriptor_idx)
            .ok()?;
        Some(desc.strip_prefix('L')?.strip_suffix(';')?.replace('/', "."))
    }
    fn string(&self, _: managed_ir::refs::StringRef) -> Option<Vec<u16>> {
        None
    }
}
impl backend_java::constructors::ConstructorResolver for EmptyResolver<'_> {
    fn initializer(
        &self,
        r: managed_ir::refs::MethodRef,
    ) -> Option<backend_java::constructors::Initializer> {
        let method = backend_java::JavaResolver::method(self, r)?;
        (method.name == "<init>").then_some(backend_java::constructors::Initializer {
            method,
            delegates_to: None,
        })
    }
}
impl backend_java::constructors::TrustedConstructorResolver for EmptyResolver<'_> {
    fn declared_constructors(&self, owner: &str) -> Option<Vec<managed_ir::refs::MethodRef>> {
        (owner == "Fixture").then(|| {
            self.0.classes[0]
                .class_data
                .as_ref()
                .unwrap()
                .direct_methods
                .iter()
                .filter(|m| {
                    self.0
                        .string(self.0.method_ids[m.method_idx as usize].name_idx)
                        .ok()
                        == Some("<init>")
                })
                .map(|m| managed_ir::refs::MethodRef::new(m.method_idx))
                .collect()
        })
    }
    fn constructor_class(
        &self,
        name: &str,
    ) -> Option<backend_java::constructors::ConstructorClass> {
        let parent = match name {
            "Fixture" => Some("java.lang.Object".into()),
            "java.lang.Object" => None,
            _ => return None,
        };
        Some(backend_java::constructors::ConstructorClass {
            name: name.into(),
            superclass: parent,
            access_flags: 1,
        })
    }
    fn constructor_access(&self, r: managed_ir::refs::MethodRef) -> Option<u32> {
        if backend_java::JavaResolver::method(self, r)?.owner == "java.lang.Object" {
            return Some(1);
        }
        self.0.classes[0]
            .class_data
            .as_ref()?
            .direct_methods
            .iter()
            .find(|m| m.method_idx == r.index())
            .map(|m| m.access_flags)
    }
}
#[test]
fn atomic_explicit_contract_keeps_encoded_prefix_and_runtime_capabilities() {
    let (mut dex, mut class) = fixture(1);
    field(&mut dex, &mut class, "N", "I", 0x19);
    native(&mut dex, &mut class);
    let mut ctor = element(&mut dex, &mut class, "<init>", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    let mut parent = dex.method_ids[ctor.method_idx as usize].clone();
    parent.class_idx = class.superclass_idx as u16;
    let target = dex.method_ids.len() as u32;
    dex.method_ids.push(parent);
    ctor.access_flags = 0x10001;
    ctor.code_off = 1;
    let mut raw = code(&[0x1070, target as u16, 0x0000, 0x000e]);
    raw.registers_size = 1;
    raw.ins_size = 1;
    raw.outs_size = 1;
    ctor.code = Some(raw);
    class
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(ctor.clone());
    dex.classes = vec![class.clone()];
    let inputs = [dex];
    let inv = write_inventory::WriteInventory::scan(&inputs);
    let resolver = EmptyResolver(&inputs[0]);
    let ssa = dex_decompiler::DexPasses::run_ssa_with(
        ctor.code.as_ref().unwrap(),
        &dex_decompiler::DexTypes::new(&inputs[0], Some(ctor.method_idx)),
    )
    .unwrap();
    let method = backend_java::JavaResolver::method(
        &resolver,
        managed_ir::refs::MethodRef::new(ctor.method_idx),
    )
    .unwrap();
    let bindings = decomp::dex_java::parameter_bindings(ssa.ir(), &method, 1, 1).unwrap();
    let ctors = [backend_java::constructors::ConstructorInput {
        reference: managed_ir::refs::MethodRef::new(ctor.method_idx),
        ir: ssa.ir(),
        bindings: &bindings,
    }];
    assert!(
        plan_class_initialization_with_inventory(&inputs, 0, &class, &ctors, None, &resolver, &inv)
            .is_err()
    );
    let plan = plan_class_initialization_with_explicit_stores(
        &inputs, 0, &class, &ctors, None, &resolver, &inv,
    )
    .unwrap();
    assert_eq!(plan.runtime_capabilities.len(), 1);
    assert!(plan.plan.encoded.prefix_block.contains("N = 0"));
    assert!(plan.plan.static_initializer.is_none());
}

#[test]
fn malformed_opaque_and_duplicate_inputs_do_not_become_runtime_capabilities() {
    let (mut dex, mut class) = fixture(1);
    field(&mut dex, &mut class, "N", "I", 0x19);
    let mut method = element(&mut dex, &mut class, "broken", "V");
    class.class_data.as_mut().unwrap().virtual_methods.clear();
    method.access_flags = 9;
    method.code_off = 1;
    method.code = Some(code(&[0x0073, 0x000e]));
    class
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .push(method);
    dex.classes = vec![class.clone()];
    let inventory = write_inventory::WriteInventory::scan(std::slice::from_ref(&dex));
    assert!(plan_encoded_initialization_explicit_stores(&dex, &class, &inventory).is_err());
    let (mut clean, mut c) = fixture(1);
    field(&mut clean, &mut c, "N", "I", 0x19);
    clean.classes = vec![c.clone()];
    let duplicate = write_inventory::WriteInventory::scan(&[clean.clone(), clean.clone()]);
    assert!(plan_encoded_initialization_explicit_stores(&clean, &c, &duplicate).is_err());
}
