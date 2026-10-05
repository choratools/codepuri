//! Body-proven compiler enum methods and closed member emission.
use super::EnumInitializationPlan;
use crate::java_class_kinds::DeclarationUnsupported as Error;
use dex_format::{
    DexFile,
    class::{ClassDef, EncodedMethod},
};
use std::collections::{BTreeMap, BTreeSet};

/// Exact method identities recreated by javac; neither names nor flags suffice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumMethodsPlan {
    /// Original zero-argument values method pool index.
    pub values: u32,
    /// Original String-taking valueOf method pool index.
    pub value_of: u32,
}

fn desc(dex: &DexFile, i: u32) -> Option<&str> {
    dex.string(dex.type_id(i).ok()?.descriptor_idx).ok()
}
fn signature(dex: &DexFile, i: u32, owner: &str, name: &str, params: &[&str], ret: &str) -> bool {
    (|| {
        let m = dex.method_id(i).ok()?;
        let p = dex.proto_id(u32::from(m.proto_idx)).ok()?;
        let actual = p
            .parameters
            .iter()
            .flat_map(|p| &p.types)
            .map(|t| desc(dex, u32::from(*t)))
            .collect::<Option<Vec<_>>>()?;
        Some(
            desc(dex, u32::from(m.class_idx))? == owner
                && dex.string(m.name_idx).ok()? == name
                && actual == params
                && desc(dex, p.return_type_idx)? == ret,
        )
    })()
    .unwrap_or(false)
}
fn words(m: &EncodedMethod, incoming: u16) -> Option<Vec<u16>> {
    let c = m.code.as_ref()?;
    if m.access_flags != 9
        || c.ins_size != incoming
        || c.registers_size < incoming
        || !c.tries.is_empty()
        || !c.handlers.is_empty()
        || c.insns.len() != c.insns_size as usize * 2
    {
        return None;
    }
    Some(
        c.insns
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect(),
    )
}
fn prove_values(dex: &DexFile, m: &EncodedMethod, owner: &str, array: &str, backing: u32) -> bool {
    let Some(w) = words(m, 0) else {
        return false;
    };
    let Some(c) = m.code.as_ref() else {
        return false;
    };
    if w.len() != 9
        || w[0] & 255 != 0x62
        || u32::from(w[1]) != backing
        || w[2] != 0x106e
        || w[4] != w[0] >> 8
        || w[5] & 255 != 0x0c
        || w[6] != ((w[5] >> 8) << 8 | 0x1f)
        || w[8] != ((w[5] >> 8) << 8 | 0x11)
        || w[0] >> 8 >= c.registers_size
        || w[0] >> 8 > 15
        || w[5] >> 8 >= c.registers_size
        || c.outs_size < 1
        || desc(dex, u32::from(w[7])) != Some(array)
    {
        return false;
    }
    let _ = owner;
    signature(
        dex,
        u32::from(w[3]),
        array,
        "clone",
        &[],
        "Ljava/lang/Object;",
    )
}
fn prove_value_of(dex: &DexFile, m: &EncodedMethod, class: &ClassDef, owner: &str) -> bool {
    let Some(w) = words(m, 1) else {
        return false;
    };
    let Some(c) = m.code.as_ref() else {
        return false;
    };
    if w.len() != 9
        || w[0] & 255 != 0x1c
        || u32::from(w[1]) != class.class_idx
        || w[2] != 0x2071
        || w[5] & 255 != 0x0c
        || w[6] != ((w[5] >> 8) << 8 | 0x1f)
        || u32::from(w[7]) != class.class_idx
        || w[8] != ((w[5] >> 8) << 8 | 0x11)
        || w[4] & 15 != w[0] >> 8
        || w[4] >> 4 != c.registers_size - 1
        || w[0] >> 8 >= c.registers_size - 1
        || c.registers_size - 1 > 15
        || w[5] >> 8 >= c.registers_size
        || c.outs_size < 2
    {
        return false;
    }
    let _ = owner;
    signature(
        dex,
        u32::from(w[3]),
        "Ljava/lang/Enum;",
        "valueOf",
        &["Ljava/lang/Class;", "Ljava/lang/String;"],
        "Ljava/lang/Enum;",
    )
}

pub(crate) fn prove_values_helper(
    dex: &DexFile,
    class: &ClassDef,
    index: u32,
    constants: &[super::EnumConstant],
) -> bool {
    let Some(data) = &class.class_data else {
        return false;
    };
    let Some(m) = data.direct_methods.iter().find(|m| m.method_idx == index) else {
        return false;
    };
    let Some(c) = &m.code else {
        return false;
    };
    let Some(owner) = desc(dex, class.class_idx) else {
        return false;
    };
    let Ok(raw) = dex.method_id(index) else {
        return false;
    };
    let Ok(name) = dex.string(raw.name_idx) else {
        return false;
    };
    let array = format!("[{owner}");
    // A private static helper has no virtual dispatch. R8 may additionally
    // mark it final; retain the original identity in the separately proven
    // enum-equivalent ledger rather than treating that bit as a missing body.
    // Public/package/protected helpers still cannot be silently absorbed.
    if m.access_flags & !0x10 != 0x100a
        || c.ins_size != 0
        || !c.tries.is_empty()
        || !c.handlers.is_empty()
        || c.insns.len() != c.insns_size as usize * 2
        || !signature(dex, index, owner, name, &[], &array)
        || constants.is_empty()
    {
        return false;
    }
    prove_values_helper_ir(dex, class, m, constants)
}

// Prove helper semantics rather than one register layout. This accepts both
// filled arrays and ordered new-array/aput chains with any constant count.
fn prove_values_helper_ir(
    dex: &DexFile,
    class: &ClassDef,
    method: &EncodedMethod,
    constants: &[super::EnumConstant],
) -> bool {
    use managed_ir::{
        insn::{Constant, ManagedInsn},
        value::ValueId,
    };
    #[derive(Clone)]
    enum Value {
        Index(usize),
        Constant(u32),
        Array(Vec<u32>),
        Building(Vec<Option<u32>>),
    }
    let Some(code) = &method.code else {
        return false;
    };
    let Ok(ssa) = dex_decompiler::passes::DexPasses::run_ssa_with(
        code,
        &dex_decompiler::DexTypes::new(dex, Some(method.method_idx)),
    ) else {
        return false;
    };
    let ir = ssa.ir();
    let cfg = ir.cfg();
    if cfg.validate().is_err() {
        return false;
    }
    let mut block = cfg.entry();
    let mut seen = BTreeSet::new();
    let mut values = BTreeMap::<ValueId, Value>::new();
    let mut allocations = 0;
    let expected: Vec<_> = constants.iter().map(|c| c.field_idx).collect();
    let Some(owner) = desc(dex, class.class_idx) else {
        return false;
    };
    let array_type = format!("[{owner}");
    loop {
        if !seen.insert(block) {
            return false;
        }
        let Ok(node) = cfg.block(block) else {
            return false;
        };
        let Ok(insn) = ir.instruction(*node.payload()) else {
            return false;
        };
        match insn {
            ManagedInsn::Const {
                result,
                value: Constant::Int(n),
            } if *n >= 0 => {
                values.insert(*result, Value::Index(*n as usize));
            }
            ManagedInsn::Const {
                result,
                value: Constant::Zero,
            } => {
                values.insert(*result, Value::Index(0));
            }
            ManagedInsn::FieldGet {
                result,
                object: None,
                field,
            } if expected.contains(&field.index()) => {
                values.insert(*result, Value::Constant(field.index()));
            }
            ManagedInsn::Move { result, value } => {
                let Some(v) = values.get(value).cloned() else {
                    return false;
                };
                values.insert(*result, v);
            }
            ManagedInsn::NewFilledArray {
                result: Some(result),
                target,
                elements,
            } if desc(dex, target.index()) == Some(array_type.as_str()) => {
                let fields = elements
                    .iter()
                    .map(|v| match values.get(v) {
                        Some(Value::Constant(f)) => Some(*f),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>();
                let Some(fields) = fields.filter(|v| v == &expected) else {
                    return false;
                };
                allocations += 1;
                if allocations != 1 {
                    return false;
                }
                values.insert(*result, Value::Array(fields));
            }
            ManagedInsn::NewArray {
                result,
                element,
                length,
            } if desc(dex, element.index()) == Some(array_type.as_str()) => {
                if !matches!(values.get(length), Some(Value::Index(n)) if *n == expected.len()) {
                    return false;
                }
                allocations += 1;
                if allocations != 1 {
                    return false;
                }
                values.insert(*result, Value::Building(vec![None; expected.len()]));
            }
            ManagedInsn::ArrayPut {
                array,
                index,
                value,
            } => {
                let Some(Value::Index(index)) = values.get(index).cloned() else {
                    return false;
                };
                let Some(Value::Constant(field)) = values.get(value).cloned() else {
                    return false;
                };
                let Some(Value::Building(fields)) = values.get_mut(array) else {
                    return false;
                };
                if expected.get(index) != Some(&field) || fields.get(index) != Some(&None) {
                    return false;
                }
                fields[index] = Some(field);
            }
            ManagedInsn::Return { value: Some(value) } => {
                let exact = match values.get(value) {
                    Some(Value::Array(fields)) => fields == &expected,
                    Some(Value::Building(fields)) => {
                        fields.iter().copied().collect::<Option<Vec<_>>>().as_ref()
                            == Some(&expected)
                    }
                    _ => false,
                };
                return exact
                    && allocations == 1
                    && node.outgoing().is_empty()
                    && seen.len() == cfg.block_count();
            }
            _ => return false,
        }
        if node.outgoing().len() != 1 {
            return false;
        }
        let Ok(edge) = cfg.edge(node.outgoing()[0]) else {
            return false;
        };
        if *edge.kind() != decomp_core::cfg::EdgeKind::Unconditional {
            return false;
        }
        block = edge.target();
    }
}

/// Validate the complete original bodies, including receivers, class literal,
/// casts, returns and absence of extra instructions or exception handlers.
pub fn plan_enum_methods(
    dex: &DexFile,
    class: &ClassDef,
    initialization: &EnumInitializationPlan,
) -> Result<EnumMethodsPlan, Error> {
    let owner = desc(dex, class.class_idx).ok_or(Error::InvalidMetadata)?;
    let array = format!("[{owner}");
    let data = class.class_data.as_ref().ok_or(Error::InvalidMetadata)?;
    let mut values = None;
    let mut value_of = None;
    for m in data.direct_methods.iter().chain(&data.virtual_methods) {
        if signature(dex, m.method_idx, owner, "values", &[], &array)
            && (values.replace(m.method_idx).is_some()
                || !prove_values(dex, m, owner, &array, initialization.absorbed_values_field))
        {
            return Err(Error::EnumConstructorsUnavailable);
        }
        if signature(
            dex,
            m.method_idx,
            owner,
            "valueOf",
            &["Ljava/lang/String;"],
            owner,
        ) && (value_of.replace(m.method_idx).is_some() || !prove_value_of(dex, m, class, owner))
        {
            return Err(Error::EnumConstructorsUnavailable);
        }
    }
    Ok(EnumMethodsPlan {
        values: values.ok_or(Error::EnumConstructorsUnavailable)?,
        value_of: value_of.ok_or(Error::EnumConstructorsUnavailable)?,
    })
}

/// Require this check on each remaining body before ordinary facade lowering.
/// Java cannot expose the compiler-created backing array or construct enums;
/// enum constant fields cannot be written by surviving source methods.
pub fn validate_enum_member_ir(
    class: &ClassDef,
    initialization: &EnumInitializationPlan,
    ir: &managed_ir::method::MethodIr<managed_ir::method::SsaForm>,
) -> Result<(), Error> {
    use managed_ir::insn::ManagedInsn;
    let constants: BTreeSet<_> = initialization
        .constants
        .iter()
        .map(|c| c.field_idx)
        .collect();
    let final_fields: BTreeSet<_> = class
        .class_data
        .as_ref()
        .into_iter()
        .flat_map(|d| d.instance_fields.iter().chain(&d.static_fields))
        .filter(|f| f.access_flags & 0x10 != 0)
        .map(|f| f.field_idx)
        .collect();
    for (id, insn) in ir.instructions() {
        let forbidden = match insn {
            ManagedInsn::FieldGet { field, .. } => {
                field.index() == initialization.absorbed_values_field
            }
            ManagedInsn::FieldPut { field, .. } => {
                field.index() == initialization.absorbed_values_field
                    || constants.contains(&field.index())
                    || final_fields.contains(&field.index())
            }
            ManagedInsn::Invoke { method, .. } => {
                initialization
                    .absorbed_constructors
                    .contains(&method.index())
                    || method.index() == initialization.absorbed_clinit
                    || Some(method.index()) == initialization.absorbed_values_helper
            }
            ManagedInsn::NewInstance {
                class: allocated, ..
            } => allocated.index() == class.class_idx,
            // Unresolved instructions cannot establish absence of forbidden effects.
            // The lifter represents caught-object definitions separately from
            // the original move-exception. This exact synthetic binder neither
            // constructs an enum nor accesses a field. Actual catch scope,
            // object identity and source availability remain backend proofs.
            ManagedInsn::Opaque {
                family_tag: 0x8002,
                reads,
                writes,
                effects,
            } => {
                !reads.is_empty()
                    || writes.len() != 1
                    || *effects != managed_ir::effects::EffectFlags::MAY_THROW
                    || !ir.value(writes[0]).is_ok_and(|value| value.def() == id)
            }
            ManagedInsn::Opaque { family_tag, .. } => !matches!(family_tag, 0x8001 | 0x8003),
            _ => false,
        };
        if forbidden {
            return Err(Error::EnumConstructorsUnavailable);
        }
    }
    Ok(())
}

/// Assemble an enum with explicitly validated remaining declarations. Keys are
/// original pool identities. The owning facade must validate source bodies and
/// forbid references to absorbed fields/constructors before calling this helper.
/// Exact coverage is mandatory: missing or extra declarations are errors.
pub fn emit_enum_declaration(
    class: &ClassDef,
    initialization: &EnumInitializationPlan,
    methods: &EnumMethodsPlan,
    fields: &BTreeMap<u32, String>,
    members: &BTreeMap<u32, String>,
) -> Result<String, Error> {
    let data = class.class_data.as_ref().ok_or(Error::InvalidMetadata)?;
    let absorbed_fields: BTreeSet<_> = initialization
        .constants
        .iter()
        .map(|c| c.field_idx)
        .chain([initialization.absorbed_values_field])
        .collect();
    let mut absorbed_methods = BTreeSet::from([
        initialization.absorbed_constructor,
        initialization.absorbed_clinit,
        methods.values,
        methods.value_of,
    ]);
    absorbed_methods.extend(initialization.absorbed_values_helper);
    absorbed_methods.extend(initialization.absorbed_constructors.iter().copied());
    let remaining_fields: BTreeSet<_> = data
        .static_fields
        .iter()
        .chain(&data.instance_fields)
        .map(|f| f.field_idx)
        .filter(|f| !absorbed_fields.contains(f))
        .collect();
    let remaining_methods: BTreeSet<_> = data
        .direct_methods
        .iter()
        .chain(&data.virtual_methods)
        .map(|m| m.method_idx)
        .filter(|m| !absorbed_methods.contains(m))
        .collect();
    if remaining_fields != fields.keys().copied().collect()
        || remaining_methods != members.keys().copied().collect()
        || fields
            .values()
            .chain(members.values())
            .any(|s| s.trim().is_empty())
    {
        return Err(Error::EnumConstructorsUnavailable);
    }
    let mut source = format!(
        "{} {{\n    {}\n",
        initialization.header, initialization.constants_source
    );
    let mut initialized_fields = fields.clone();
    for (field, expression) in &initialization.encoded_declaration_initializers {
        let declaration = initialized_fields
            .get_mut(field)
            .ok_or(Error::EnumConstructorsUnavailable)?;
        let blank = declaration
            .strip_suffix(';')
            .ok_or(Error::EnumConstructorsUnavailable)?;
        if blank.contains('=') {
            return Err(Error::EnumConstructorsUnavailable);
        }
        *declaration = format!("{blank} = {expression};");
    }
    for s in initialized_fields
        .values()
        .chain(initialization.constructor_sources.values())
        .chain(members.values())
    {
        for line in s.lines() {
            source.push_str("    ");
            source.push_str(line);
            source.push('\n');
        }
    }
    if let Some(suffix) = &initialization.static_initializer_source {
        source.push_str(&format!("    // Original enum suffix stores and Java definite assignment proved; {} runtime native/reflection capability observations retained. No runtime immutability claim.\n",initialization.suffix_runtime_capabilities));
        for line in suffix.lines() {
            source.push_str("    ");
            source.push_str(line);
            source.push('\n');
        }
    }
    source.push_str("}\n");
    Ok(source)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::collapsible_if)] // Fixture assertions intentionally panic.
    use super::*;
    use backend_java::{JavaField, JavaMethod, JavaResolver, ParameterBindings};
    use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
    struct Resolver<'a>(&'a DexFile);
    impl JavaResolver for Resolver<'_> {
        fn class(&self, r: ClassRef) -> Option<String> {
            crate::dex_java::DexJavaResolver { dex: self.0 }.class(r)
        }
        fn string(&self, r: StringRef) -> Option<Vec<u16>> {
            crate::dex_java::DexJavaResolver { dex: self.0 }.string(r)
        }
        fn field(&self, r: FieldRef) -> Option<JavaField> {
            crate::dex_java::DexJavaResolver { dex: self.0 }.field(r)
        }
        fn method(&self, r: MethodRef) -> Option<JavaMethod> {
            let m = self.0.method_id(r.index()).ok()?;
            let p = self.0.proto_id(u32::from(m.proto_idx)).ok()?;
            if desc(self.0, u32::from(m.class_idx)) == Some("Ljava/lang/Enum;")
                && self.0.string(m.name_idx).ok()? == "<init>"
            {
                Some(JavaMethod {
                    owner: "java.lang.Enum".into(),
                    name: "<init>".into(),
                    is_static: false,
                    parameters: p
                        .parameters
                        .iter()
                        .flat_map(|p| &p.types)
                        .map(|t| crate::dex_java::descriptor_type(desc(self.0, u32::from(*t))?))
                        .collect::<Option<_>>()?,
                    return_type: backend_java::JavaType::Void,
                    throws: vec![],
                })
            } else {
                crate::dex_java::DexJavaResolver { dex: self.0 }.method(r)
            }
        }
    }
    impl backend_java::constructors::ConstructorResolver for Resolver<'_> {
        fn initializer(&self, r: MethodRef) -> Option<backend_java::constructors::Initializer> {
            let method = self.method(r)?;
            if method.name != "<init>" {
                return None;
            }
            Some(backend_java::constructors::Initializer {
                method,
                delegates_to: None,
            })
        }
    }
    #[test]
    #[ignore = "local corpus and JDK17 runtime"]
    fn okcashbag_enum_source_runtime() {
        let apk = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk"
        );
        let output = std::process::Command::new("unzip")
            .args(["-p", apk, "classes.dex"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let dex = DexFile::from_container(&output.stdout).unwrap().remove(0);
        let resolver = Resolver(&dex);
        for wanted in [
            "Landroidx/camera/core/CameraState$Type;",
            "Landroidx/camera/core/impl/Timebase;",
        ] {
            let class = dex
                .classes
                .iter()
                .find(|c| desc(&dex, c.class_idx) == Some(wanted))
                .unwrap();
            let data = class.class_data.as_ref().unwrap();
            let find = |name| {
                data.direct_methods
                    .iter()
                    .find(|m| {
                        let raw = dex.method_id(m.method_idx).unwrap();
                        dex.string(raw.name_idx).unwrap() == name
                    })
                    .unwrap()
            };
            let ctor = find("<init>");
            let clinit = find("<clinit>");
            let ctor_ssa = dex_decompiler::passes::DexPasses::run_ssa_with(
                ctor.code.as_ref().unwrap(),
                &dex_decompiler::DexTypes::new(&dex, Some(ctor.method_idx)),
            )
            .unwrap();
            let clinit_ssa = dex_decompiler::passes::DexPasses::run_ssa_with(
                clinit.code.as_ref().unwrap(),
                &dex_decompiler::DexTypes::new(&dex, Some(clinit.method_idx)),
            )
            .unwrap();
            let code = ctor.code.as_ref().unwrap();
            let bindings = crate::java_initial_parameters::parameter_bindings(
                ctor_ssa.ir(),
                &resolver.method(MethodRef::new(ctor.method_idx)).unwrap(),
                code.registers_size,
                code.ins_size,
            )
            .unwrap();
            let bound = |p| match p {
                crate::java_initial_parameters::InitialParameter::Bound(v) => v,
                _ => panic!("unused enum input"),
            };
            let bindings = ParameterBindings {
                receiver: bindings.receiver.map(bound),
                parameters: bindings.parameters.into_iter().map(bound).collect(),
                names: vec![],
            };
            let init = super::super::plan_enum_initialization(
                &dex,
                class,
                super::super::EnumConstructorInput {
                    reference: MethodRef::new(ctor.method_idx),
                    ir: ctor_ssa.ir(),
                    bindings: &bindings,
                },
                MethodRef::new(clinit.method_idx),
                clinit_ssa.ir(),
                &resolver,
            )
            .unwrap();
            let methods = plan_enum_methods(&dex, class, &init).unwrap();
            let source =
                emit_enum_declaration(class, &init, &methods, &BTreeMap::new(), &BTreeMap::new())
                    .unwrap();
            assert!(
                emit_enum_declaration(
                    class,
                    &init,
                    &methods,
                    &BTreeMap::from([(u32::MAX, "int lost;".into())]),
                    &BTreeMap::new()
                )
                .is_err()
            );
            let mut extra_field = class.clone();
            extra_field
                .class_data
                .as_mut()
                .unwrap()
                .instance_fields
                .push(dex_format::class::EncodedField {
                    field_idx: u32::MAX,
                    access_flags: 2,
                });
            assert!(
                emit_enum_declaration(
                    &extra_field,
                    &init,
                    &methods,
                    &BTreeMap::new(),
                    &BTreeMap::new()
                )
                .is_err()
            );
            assert!(
                emit_enum_declaration(
                    &extra_field,
                    &init,
                    &methods,
                    &BTreeMap::from([(u32::MAX, "private int retained;".into())]),
                    &BTreeMap::new()
                )
                .unwrap()
                .contains("private int retained;")
            );
            if let Some(helper) = init.absorbed_values_helper {
                let mut changed = class.clone();
                let c = &mut changed;
                let m = c
                    .class_data
                    .as_mut()
                    .unwrap()
                    .direct_methods
                    .iter_mut()
                    .find(|m| m.method_idx == helper)
                    .unwrap();
                m.code.as_mut().unwrap().insns[2] ^= 1;
                assert!(!prove_values_helper(&dex, c, helper, &init.constants));
            }
            let owner = resolver.class(ClassRef::new(class.class_idx)).unwrap();
            let (package, simple) = owner.rsplit_once('.').unwrap();
            let dir = std::env::temp_dir()
                .join(format!("decomphify-enum-{}-{simple}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join(format!("{simple}.java")),
                format!("package {package};\n{source}"),
            )
            .unwrap();
            let names = init
                .constants
                .iter()
                .map(|c| format!("\"{}\"", c.name))
                .collect::<Vec<_>>()
                .join(",");
            let harness = format!(
                "package {package}; public class EnumCheck {{ public static void main(String[] args) {{\n{simple}[] a = {simple}.values(); String[] n = {{{names}}}; if(a.length != n.length) throw new AssertionError(); for(int i=0;i<a.length;i++) {{ if(!a[i].name().equals(n[i]) || a[i].ordinal()!=i || !a[i].toString().equals(n[i]) || {simple}.valueOf(n[i])!=a[i]) throw new AssertionError(); }} a[0]=null; if({simple}.values()[0]==null) throw new AssertionError(); try {{ {simple}.valueOf(\"__bad__\"); throw new AssertionError(); }} catch(IllegalArgumentException expected) {{}} try {{ {simple}.valueOf(null); throw new AssertionError(); }} catch(NullPointerException expected) {{}} System.out.println(\"PASS {wanted}\"); }} }}"
            );
            std::fs::write(dir.join("EnumCheck.java"), harness).unwrap();
            let compiled = std::process::Command::new("javac")
                .current_dir(&dir)
                .args([
                    "--release",
                    "17",
                    "-d",
                    ".",
                    &format!("{simple}.java"),
                    "EnumCheck.java",
                ])
                .output()
                .unwrap();
            assert!(
                compiled.status.success(),
                "{}",
                String::from_utf8_lossy(&compiled.stderr)
            );
            let ran = std::process::Command::new("java")
                .current_dir(&dir)
                .args(["-cp", ".", &format!("{package}.EnumCheck")])
                .output()
                .unwrap();
            assert!(
                ran.status.success(),
                "{}",
                String::from_utf8_lossy(&ran.stderr)
            );
            println!("{}SOURCE {source}", String::from_utf8_lossy(&ran.stdout));
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
    #[test]
    #[ignore = "local OKcashbag corpus inventory, one DEX only"]
    fn okcashbag_enum_method_evidence() {
        let apk = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/corpus/apks_device/com.skmc.okcashbag.home_google.apk"
        );
        let output = std::process::Command::new("unzip")
            .args(["-p", apk, "classes.dex"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let dex = DexFile::from_container(&output.stdout).unwrap().remove(0);
        let mut proven = 0;
        for class in &dex.classes {
            if class.access_flags & 0x4000 == 0 {
                continue;
            }
            let Some(data) = &class.class_data else {
                continue;
            };
            let owner = desc(&dex, class.class_idx).unwrap();
            let array = format!("[{owner}");
            let Some(backing) = data.static_fields.iter().find(|f| {
                f.access_flags == 0x101a
                    && dex
                        .field_id(f.field_idx)
                        .ok()
                        .and_then(|f| desc(&dex, u32::from(f.type_idx)))
                        == Some(array.as_str())
            }) else {
                continue;
            };
            let all: Vec<_> = data
                .direct_methods
                .iter()
                .chain(&data.virtual_methods)
                .collect();
            let values = all
                .iter()
                .find(|m| signature(&dex, m.method_idx, owner, "values", &[], &array));
            let value_of = all.iter().find(|m| {
                signature(
                    &dex,
                    m.method_idx,
                    owner,
                    "valueOf",
                    &["Ljava/lang/String;"],
                    owner,
                )
            });
            if let (Some(values), Some(value_of)) = (values, value_of) {
                if prove_values(&dex, values, owner, &array, backing.field_idx)
                    && prove_value_of(&dex, value_of, class, owner)
                {
                    proven += 1;
                    println!(
                        "PROVEN {owner} values={} valueOf={} backing={}",
                        values.method_idx, value_of.method_idx, backing.field_idx
                    );
                    let mut changed = (*values).clone();
                    changed.code.as_mut().unwrap().insns[0] = 0;
                    assert!(!prove_values(
                        &dex,
                        &changed,
                        owner,
                        &array,
                        backing.field_idx
                    ));
                    let mut changed = (*value_of).clone();
                    changed.code.as_mut().unwrap().insns[0] = 0;
                    assert!(!prove_value_of(&dex, &changed, class, owner));
                }
            }
        }
        assert!(proven > 0);
    }
}

#[cfg(test)]
#[path = "java_enum_handler_tests.rs"]
mod handler_tests;

#[cfg(test)]
#[path = "java_enum_helper_access_tests.rs"]
mod helper_access_tests;
