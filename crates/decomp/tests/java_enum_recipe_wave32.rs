//! Original JavaType recipe constructor evidence (not APK recovery).
#![allow(dead_code)]
#![allow(clippy::unwrap_used)]
use backend_java::constructors::{
    ConstructorClass, ConstructorResolver, Initializer, TrustedConstructorResolver,
};
use backend_java::{JavaField, JavaMethod, JavaResolver, JavaType};

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
    recipe_release: u16,
    inputs: &'a [DexFile],
    dex: &'a DexFile,
    class: &'a ClassDef,
    cp: &'a Classpath,
    hierarchy: &'a backend_java::type_compatibility::Hierarchy,
    super_proof: bool,
    nonconstant_fields: std::collections::HashSet<u32>,
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
    fn field_source_nonconstant(&self, reference: FieldRef) -> bool {
        self.nonconstant_fields.contains(&reference.index())
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

#[allow(dead_code)]
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

mod dex_java {
    pub use decomp::dex_java::*;
}
#[path = "../src/java_enum_recipe.rs"]
mod recipe;
impl recipe::RecipeEvidence for Resolver<'_> {
    fn recipe_source_release(&self) -> u16 {
        self.recipe_release
    }
    fn recipe_operation(
        &self,
        _: managed_ir::value::InsnId,
        insn: &managed_ir::insn::ManagedInsn,
    ) -> bool {
        use managed_ir::insn::ManagedInsn;
        match insn {
            ManagedInsn::Invoke { method, .. } => self
                .method_access(*method)
                .is_some_and(|f| f & (8 | 0x100 | 0x400) == 8),
            ManagedInsn::FieldGet { field, .. } => self.field(*field).is_some(),
            ManagedInsn::Const {
                value: managed_ir::insn::Constant::ClassLiteral(t),
                ..
            } => self.instruction_type(*t).is_some(),
            _ => false,
        }
    }
}
#[test]
#[ignore = "genuine original Samsung APK and Java25 classpath"]
fn original_javatype_int_recipe_order() {
    let inputs = dex_format::apk::parse_apk_path(
        "/storage_0/decomphify/tests/corpus/apks_bulk/com.samsung.android.samsungpass.apk",
    )
    .unwrap();
    let cp = Classpath::from_paths([PathBuf::from(
        "/tmp/decomphify-source25-java-base-649174/java.base.jar",
    )])
    .unwrap();
    let hierarchy =
        decomp::java_hierarchy::build_with_metadata(&inputs.iter().collect::<Vec<_>>(), &cp)
            .unwrap();
    let dex = &inputs[1];
    let class = dex
        .classes
        .iter()
        .find(|c| descriptor(dex, c.class_idx) == "Lcom/google/protobuf/JavaType;")
        .unwrap();
    let ssa = original_ssa(dex, class, 11291);
    let resolver = Resolver {
        recipe_release: 25,
        inputs: &inputs,
        dex,
        class,
        cp: &cp,
        hierarchy: &hierarchy,
        super_proof: false,
        nonconstant_fields: std::collections::HashSet::from([63511]),
    };
    let at = |offset| {
        ssa.ir()
            .instructions()
            .iter()
            .find(|(id, _)| ssa.offset(*id) == Some(offset))
            .unwrap()
            .0
    };
    let plan = recipe::candidate_recipe(
        dex,
        class,
        &ssa,
        recipe::RecipeRequest {
            clinit_index: 11291,
            allocation: at(15),
            call: at(28),
            recipe_number: 1,
        },
        &resolver,
    )
    .unwrap();
    assert_eq!(
        recipe::plan_recipe(
            dex,
            class,
            &ssa,
            recipe::RecipeRequest {
                clinit_index: 11291,
                allocation: at(15),
                call: at(28),
                recipe_number: 1
            },
            &resolver
        )
        .unwrap_err(),
        recipe::RecipeError::Evidence
    );
    // Original hidden const-string21 follows invocation17. Compilation below
    // validates only the candidate subset, never this missing equivalence proof.
    assert_eq!(plan.effects, vec![17, 24, 26]);
    assert_eq!(plan.original_constructor.index(), 11292);
    assert!(
        plan.helper_source.contains("java.lang.Integer.valueOf(0)"),
        "{}",
        plan.helper_source
    );
    assert!(
        plan.helper_source.find("valueOf").unwrap()
            < plan.helper_source.find("Integer.TYPE").unwrap()
    );
    println!("{}", plan.helper_source);
    let original_ctor = original_ssa(dex, class, 11292);
    let ctor_signature = resolver.method(MethodRef::new(11292)).unwrap();
    let ctor_code = class
        .class_data
        .as_ref()
        .unwrap()
        .direct_methods
        .iter()
        .find(|m| m.method_idx == 11292)
        .unwrap()
        .code
        .as_ref()
        .unwrap();
    let ctor_bindings = bindings(&original_ctor, &ctor_signature, ctor_code);
    let ctor = backend_java::constructors::plan_constructor_declaration(
        original_ctor.ir(),
        MethodRef::new(11292),
        "java.lang.Enum",
        &ctor_bindings,
        &resolver,
    )
    .unwrap();
    assert!(ctor.prologue.is_empty());
    assert_eq!(
        ctor.first_statement,
        format!(
            "super({}, {});",
            ctor.parameter_names[0], ctor.parameter_names[1]
        )
    );
    let ctor_source = format!(
        "private JavaType(java.lang.Class {}, java.lang.Class {}, java.lang.Object {}) {{\n{}\n}}",
        ctor.parameter_names[2],
        ctor.parameter_names[3],
        ctor.parameter_names[4],
        ctor.body.join("\n")
    );
    let root = PathBuf::from(format!(
        "/tmp/opencode/wave32-original-int-subset-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    // Two original constants preserve INT's genuine ordinal1. This is an
    // original-method/constant subset, explicitly not full JavaType recovery.
    let source = format!(
        "package com.google.protobuf; public enum JavaType {{ VOID(Void.class,Void.class,null), INT({}); private final Class type,boxedType; private final Object defaultDefault; {} {} }}",
        plan.constant_arguments.join(","),
        ctor_source,
        plan.helper_source
    );
    std::fs::write(root.join("JavaType.java"), source).unwrap();
    std::fs::write(root.join("RecipeOriginalHarness.java"),r#"import com.google.protobuf.JavaType;
public class RecipeOriginalHarness { public static void main(String[] args) throws Exception {
  JavaType e=JavaType.INT;
  if(e.ordinal()!=1 || !e.name().equals("INT") || JavaType.valueOf("INT")!=e)throw new AssertionError();
  for(String name:new String[]{"type","boxedType","defaultDefault"}) {
    var f=JavaType.class.getDeclaredField(name);f.setAccessible(true);Object actual=f.get(e);
    Object expected=name.equals("type")?int.class:name.equals("boxedType")?Integer.class:Integer.valueOf(0);
    if(actual!=expected)throw new AssertionError(name);
  }
  var clone=JavaType.values();clone[1]=JavaType.VOID;
  if(JavaType.values()[1]!=e)throw new AssertionError();
} }"#).unwrap();
    let bin = PathBuf::from("/tmp/opencode/decomphify-jdk25/jdk-25.0.4.1+1/bin");
    let output = std::process::Command::new(bin.join("javac"))
        .args(["--release", "25", "-d"])
        .arg(&root)
        .arg(root.join("JavaType.java"))
        .arg(root.join("RecipeOriginalHarness.java"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        std::process::Command::new(bin.join("java"))
            .arg("-cp")
            .arg(&root)
            .arg("RecipeOriginalHarness")
            .status()
            .unwrap()
            .success()
    );

    let mut missing = Resolver {
        nonconstant_fields: Default::default(),
        ..resolver
    };
    assert_eq!(
        recipe::candidate_recipe(
            dex,
            class,
            &ssa,
            recipe::RecipeRequest {
                clinit_index: 11291,
                allocation: at(15),
                call: at(28),
                recipe_number: 1
            },
            &missing
        )
        .unwrap_err(),
        recipe::RecipeError::Evidence
    );
    missing.nonconstant_fields.insert(63511);
    missing.recipe_release = 24;
    assert_eq!(
        recipe::candidate_recipe(
            dex,
            class,
            &ssa,
            recipe::RecipeRequest {
                clinit_index: 11291,
                allocation: at(15),
                call: at(28),
                recipe_number: 1
            },
            &missing
        )
        .unwrap_err(),
        recipe::RecipeError::Metadata
    );
    missing.recipe_release = 25;
    assert_eq!(
        recipe::candidate_recipe(
            dex,
            class,
            &ssa,
            recipe::RecipeRequest {
                clinit_index: 11291,
                allocation: at(144),
                call: at(154),
                recipe_number: 8
            },
            &missing
        )
        .unwrap_err(),
        recipe::RecipeError::Evidence
    );

    let mut public = class.clone();
    public
        .class_data
        .as_mut()
        .unwrap()
        .direct_methods
        .iter_mut()
        .find(|m| m.method_idx == 11292)
        .unwrap()
        .access_flags = 0x10001;
    assert_eq!(
        recipe::candidate_recipe(
            dex,
            &public,
            &ssa,
            recipe::RecipeRequest {
                clinit_index: 11291,
                allocation: at(15),
                call: at(28),
                recipe_number: 1
            },
            &missing
        )
        .unwrap_err(),
        recipe::RecipeError::Metadata
    );

    assert_eq!(
        recipe::candidate_recipe(
            dex,
            class,
            &ssa,
            recipe::RecipeRequest {
                clinit_index: 11291,
                allocation: at(15),
                call: at(28),
                recipe_number: 0
            },
            &missing
        )
        .unwrap_err(),
        recipe::RecipeError::Evidence
    );
}
