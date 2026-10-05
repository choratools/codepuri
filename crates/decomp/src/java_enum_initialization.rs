//! Evidence-proven enum initialization and payload constructor recovery. This is a plan for
//! initialization only; every other method must still pass the facade pipeline.
use super::{ClassKind, DeclarationUnsupported as Error, class_kind, descriptor};
use backend_java::{
    DeclarationBindings, JavaType, ParameterBindings,
    constructors::{
        ChainKind, ConstructorInput, ConstructorResolver, constructor_initialization_evidence,
        plan_constructor_declaration,
    },
};
use decomp_core::cfg::EdgeKind;
use dex_format::{DexFile, class::ClassDef};
use managed_ir::{
    insn::{Constant, InvokeKind, ManagedInsn},
    method::{MethodIr, SsaForm},
    refs::MethodRef,
    value::{InsnId, ValueId},
};
use std::collections::{BTreeMap, HashMap, HashSet};

#[path = "java_enum_emission.rs"]
pub mod emission;

#[path = "java_enum_hidden_parameters.rs"]
mod hidden_parameters;

/// A constant proven to use the declaring enum's exact initializer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumConstant {
    /// Original field pool index.
    pub field_idx: u32,
    /// Exact, unrenamed Java identifier (also the runtime Enum name).
    pub name: String,
    /// Original constructor name/ordinal order.
    pub ordinal: usize,
    /// Typed constant payload expressions in original descriptor order.
    pub arguments: Vec<String>,
}

/// Explicit plan; obtaining one does not approve the remaining class members.
#[derive(Debug)]
pub struct EnumInitializationPlan {
    /// Enum header without opening brace.
    pub header: String,
    /// Constants in initializer execution order.
    pub constants: Vec<EnumConstant>,
    /// Source constant list terminated with `;`.
    pub constants_source: String,
    /// Original prepared final literal values, admitted only when no original
    /// input field read can be replaced by javac constant folding. Runtime
    /// native/reflection capabilities remain explicit and are not write proofs.
    pub encoded_declaration_initializers: BTreeMap<u32, String>,
    /// Constructor whose synthetic (String,int) parameters Java recreates.
    pub absorbed_constructor: u32,
    /// Complete original constructor family, all body-proven and reconstructed.
    pub absorbed_constructors: Vec<u32>,
    /// Source constructors with only the two compiler-owned parameters removed.
    pub constructor_sources: BTreeMap<u32, String>,
    /// Original class initializer covered by the proven compiler prefix and
    /// retained suffix (whose dependencies still require facade approval).
    pub absorbed_clinit: u32,
    /// Synthetic values array field recreated by the Java compiler.
    pub absorbed_values_field: u32,
    /// Optional body-proven static array assembly helper recreated by javac.
    pub absorbed_values_helper: Option<u32>,
    /// Original initializer suffix after the proven compiler backing-array store.
    /// Every suffix effect survives; this is not a declaration initializer.
    pub static_initializer_source: Option<String>,
    /// Verified suffix SSA with original instruction, value and pool identities.
    /// The facade must apply original `<clinit>` access/dependency requirements
    /// and publication closure before approving this plan as a complete body.
    pub static_initializer_ir: Option<MethodIr<SsaForm>>,
    /// Genuine native/reflection observations retained by the input inventory.
    /// Explicit-store source legality never attests runtime immutability.
    pub suffix_runtime_capabilities: usize,
}

/// Exact constructor IR and its descriptor-order initial parameter bindings.
pub struct EnumConstructorInput<'a> {
    /// Initializer pool identity.
    pub reference: MethodRef,
    /// Verified SSA for that initializer.
    pub ir: &'a MethodIr<SsaForm>,
    /// Receiver and synthetic (String,int) parameter values.
    pub bindings: &'a ParameterBindings,
}

/// Original constructor input preserving unused descriptor parameter positions.
pub struct EnumConstructorDeclarationInput<'a> {
    /// Exact initializer pool identity.
    pub reference: MethodRef,
    /// Complete verified original constructor SSA.
    pub ir: &'a MethodIr<SsaForm>,
    /// Descriptor-order bindings, including unused synthetic parameters.
    pub bindings: &'a DeclarationBindings,
}

#[derive(Clone)]
enum Value {
    Int(i32),
    String(Vec<u16>),
    Literal(Constant),
    ReferenceExpression {
        source: String,
        ty: JavaType,
        effects: Vec<InsnId>,
    },
    Instance {
        allocation: ValueId,
        initialized: bool,
        stored: bool,
    },
    Values,
    BuildingValues {
        written: usize,
        length: usize,
    },
}

fn unsupported<T>() -> Result<T, Error> {
    Err(Error::EnumConstructorsUnavailable)
}

fn type_source(ty: &JavaType) -> Result<String, Error> {
    Ok(match ty {
        JavaType::Boolean => "boolean".into(),
        JavaType::Byte => "byte".into(),
        JavaType::Short => "short".into(),
        JavaType::Char => "char".into(),
        JavaType::Int => "int".into(),
        JavaType::Long => "long".into(),
        JavaType::Float => "float".into(),
        JavaType::Double => "double".into(),
        JavaType::Class(name) => name.clone(),
        JavaType::Array(component) => format!("{}[]", type_source(component)?),
        JavaType::Void => return unsupported(),
    })
}

fn constant_source(
    value: &Value,
    ty: &JavaType,
    resolver: &impl backend_java::JavaResolver,
) -> Result<String, Error> {
    let raw = match (value, ty) {
        (Value::Int(i), JavaType::Boolean) if *i == 0 || *i == 1 => {
            if *i == 0 {
                "false".into()
            } else {
                "true".into()
            }
        }
        (Value::Literal(Constant::Boolean(b)), JavaType::Boolean) => b.to_string(),
        (Value::Int(bits), JavaType::Float) => {
            float_source(u32::from_ne_bytes(bits.to_ne_bytes()))?
        }
        (Value::Literal(Constant::Long(bits)), JavaType::Double) => {
            double_source(u64::from_ne_bytes(bits.to_ne_bytes()))?
        }
        (Value::Int(i), JavaType::Byte | JavaType::Short | JavaType::Char | JavaType::Int) => {
            i.to_string()
        }
        (Value::Literal(Constant::Long(i)), JavaType::Long) => format!("{i}L"),
        (Value::Literal(Constant::FloatBits(bits)), JavaType::Float) => float_source(*bits)
            .unwrap_or_else(|_| format!("java.lang.Float.intBitsToFloat(0x{bits:08x})")),
        (Value::Literal(Constant::DoubleBits(bits)), JavaType::Double) => double_source(*bits)
            .unwrap_or_else(|_| format!("java.lang.Double.longBitsToDouble(0x{bits:016x}L)")),
        (
            Value::ReferenceExpression {
                source, ty: actual, ..
            },
            expected,
        ) if actual == expected
            || matches!(expected, JavaType::Class(name) if name == "java.lang.Object")
            || (matches!((actual, expected), (JavaType::Class(_), JavaType::Class(_)))
                && resolver.assignable(actual, expected)) =>
        {
            source.clone()
        }
        (
            Value::Literal(Constant::Null | Constant::Zero) | Value::Int(0),
            JavaType::Class(_) | JavaType::Array(_),
        ) => "null".into(),
        (Value::String(units), JavaType::Class(name))
            if name == "java.lang.String" || name == "java.lang.Object" =>
        {
            let bytes = units.iter().try_fold(0usize, |n, u| {
                let n = n + match u {
                    0 => 2,
                    1..=0x7f => 1,
                    0x80..=0x7ff => 2,
                    _ => 3,
                };
                (n <= 65535).then_some(n)
            });
            if bytes.is_none() || units.len() >= 65535 {
                return unsupported();
            }
            let mut literal = String::from("\"");
            for unit in units {
                match *unit {
                    10 => literal.push_str("\\n"),
                    13 => literal.push_str("\\r"),
                    0..=31 | 34 | 92 | 127 => literal.push_str(&format!("\\{unit:03o}")),
                    32..=126 => literal.push(char::from(*unit as u8)),
                    _ => literal.push_str(&format!("\\u{unit:04x}")),
                }
            }
            literal.push('"');
            literal
        }
        _ => return unsupported(),
    };
    // Explicit original descriptor casts select the same overload, including
    // null references and narrow primitives, and preserve register bit values.
    Ok(format!("({}) ({raw})", type_source(ty)?))
}

// Decimal scientific literals round-trip to the original finite bit pattern.
// No helper invocation or class initialization is added to payload evaluation.
fn float_source(bits: u32) -> Result<String, Error> {
    let value = f32::from_bits(bits);
    if !value.is_finite() {
        return unsupported();
    }
    Ok(format!("{value:e}f"))
}

fn double_source(bits: u64) -> Result<String, Error> {
    let value = f64::from_bits(bits);
    if !value.is_finite() {
        return unsupported();
    }
    Ok(format!("{value:e}d"))
}

fn expression_effects(value: &Value) -> &[InsnId] {
    match value {
        Value::ReferenceExpression { effects, .. } => effects,
        _ => &[],
    }
}

fn payload_effects_match<'a>(
    arguments: impl IntoIterator<Item = &'a Value>,
    observed: &[InsnId],
) -> bool {
    arguments
        .into_iter()
        .flat_map(|value| expression_effects(value).iter().copied())
        .eq(observed.iter().copied())
}

fn invocation_operands(args: &[ValueId], parameters: &[JavaType]) -> Result<Vec<ValueId>, Error> {
    if args.len() == parameters.len() + 1 {
        return Ok(args.to_vec());
    }
    let expected = 1 + parameters
        .iter()
        .map(|t| {
            if matches!(t, JavaType::Long | JavaType::Double) {
                2
            } else {
                1
            }
        })
        .sum::<usize>();
    if args.len() != expected {
        return unsupported();
    }
    let mut result = vec![args[0]];
    let mut at = 1;
    for ty in parameters {
        result.push(args[at]);
        if matches!(ty, JavaType::Long | JavaType::Double) {
            if args[at + 1] != args[at] {
                return unsupported();
            }
            at += 2;
        } else {
            at += 1;
        }
    }
    Ok(result)
}

/// Recover a restricted explicit enum plan from complete linear SSA initializers.
/// Only the enum's own allocations, `(name, ordinal)` constructor invocations,
/// constant stores, a filled values array and its store are accepted. The sole
/// private constructor must do exactly `Enum.<init>(name,ordinal)` with no body.
/// No payload arguments, subclass constants, unproven helper calls, field initializers,
/// branches, catches, user state or arbitrary class-to-enum conversion are allowed.
/// A private synthetic zero-argument values helper is accepted only when its
/// complete DEX body reads the exact constants in ordinal order, assembles their
/// array and returns it without other instructions or exception handlers.
/// The resolver must attest initializer identities and use the frozen class names.
/// Constant field names may not be aliased: changing them changes Enum.name().
/// Wiring: keep the generic enum declaration gate diagnostic until this plan
/// succeeds. Absorb only the listed constructor, clinit, optional proven array
/// helper and backing field; emit
/// the constant fields through `constants_source`. The facade must separately
/// prove and absorb compiler-generated values()/valueOf(String), and reject
/// every remaining reference to the absorbed backing field/constructor or any
/// write to the constants. It must validate all remaining declarations/bodies
/// and their enum access rules. This plan never approves synthetic methods by
/// name or flag alone. Pass verified SSA for exactly the specified DEX methods.
pub fn plan_enum_initialization<R: ConstructorResolver>(
    dex: &DexFile,
    class: &ClassDef,
    constructor_input: EnumConstructorInput<'_>,
    clinit_reference: MethodRef,
    clinit_ir: &MethodIr<SsaForm>,
    resolver: &R,
) -> Result<EnumInitializationPlan, Error> {
    // Compatibility entrypoint remains the deliberately narrow old contract.
    if constructor_input.bindings.parameters.len() != 2 {
        return unsupported();
    }
    let old = plan_constructor_declaration(
        constructor_input.ir,
        constructor_input.reference,
        "java.lang.Enum",
        &DeclarationBindings {
            receiver: constructor_input.bindings.receiver,
            parameters: constructor_input
                .bindings
                .parameters
                .iter()
                .copied()
                .map(Some)
                .collect(),
            names: constructor_input.bindings.names.clone(),
        },
        resolver,
    )
    .map_err(|_| Error::EnumConstructorsUnavailable)?;
    if old.body != ["return;"] {
        return unsupported();
    }
    if class
        .class_data
        .as_ref()
        .is_some_and(|d| !d.instance_fields.is_empty())
    {
        return unsupported();
    }
    let bindings = DeclarationBindings {
        receiver: constructor_input.bindings.receiver,
        parameters: constructor_input
            .bindings
            .parameters
            .iter()
            .copied()
            .map(Some)
            .collect(),
        names: constructor_input.bindings.names.clone(),
    };
    plan_enum_initialization_family(
        dex,
        class,
        &[EnumConstructorDeclarationInput {
            reference: constructor_input.reference,
            ir: constructor_input.ir,
            bindings: &bindings,
        }],
        clinit_reference,
        clinit_ir,
        resolver,
    )
}

/// Prove the entire constructor family and initializer before reconstructing
/// payload constants. Every original constructor body survives in source, except
/// the exact compiler-owned Enum(name, ordinal) chain and hidden parameters.
/// Constant-specific subclasses and user static initializer effects fail closed.
pub fn plan_enum_initialization_family<R: ConstructorResolver>(
    dex: &DexFile,
    class: &ClassDef,
    constructor_inputs: &[EnumConstructorDeclarationInput<'_>],
    clinit_reference: MethodRef,
    clinit_ir: &MethodIr<SsaForm>,
    resolver: &R,
) -> Result<EnumInitializationPlan, Error> {
    plan_enum_initialization_family_inner(
        dex,
        class,
        constructor_inputs,
        clinit_reference,
        clinit_ir,
        resolver,
        None,
        None,
    )
}

/// Recover a genuine enum initializer suffix using the complete original input
/// write inventory. Original explicit stores and source definite assignment are
/// proved separately from retained native/reflection capabilities. No encoded
/// values, default-final assignments or compiler-array aliases are synthesized.
#[allow(clippy::too_many_arguments)] // Original identity/bodies plus explicit input-inventory domain.
pub fn plan_enum_initialization_family_with_writes<R: ConstructorResolver>(
    dex: &DexFile,
    class: &ClassDef,
    constructor_inputs: &[EnumConstructorDeclarationInput<'_>],
    clinit_reference: MethodRef,
    clinit_ir: &MethodIr<SsaForm>,
    resolver: &R,
    dex_index: usize,
    inventory: &super::initializers::write_inventory::WriteInventory,
) -> Result<EnumInitializationPlan, Error> {
    plan_enum_initialization_family_inner(
        dex,
        class,
        constructor_inputs,
        clinit_reference,
        clinit_ir,
        resolver,
        Some((dex_index, inventory)),
        None,
    )
}

/// Extend exact enum reconstruction with original prepared encoded literal
/// values. The complete frozen inputs are scanned for original field reads;
/// positive inventory reads alone are never used as an absence proof. The caller
/// must supply the same complete original input set used by the inventory scan;
/// filtered/substituted inputs are not an absence proof. Native
/// and reflective capabilities are retained, without claiming immutability.
#[allow(clippy::too_many_arguments)]
pub fn plan_enum_initialization_family_with_prepared_values<R: ConstructorResolver>(
    inputs: &[DexFile],
    dex_index: usize,
    class: &ClassDef,
    constructor_inputs: &[EnumConstructorDeclarationInput<'_>],
    clinit_reference: MethodRef,
    clinit_ir: &MethodIr<SsaForm>,
    resolver: &R,
    inventory: &super::initializers::write_inventory::WriteInventory,
) -> Result<EnumInitializationPlan, Error> {
    let dex = inputs.get(dex_index).ok_or(Error::InvalidMetadata)?;
    if !dex.classes.iter().any(|original| original == class) {
        return Err(Error::InvalidMetadata);
    }
    plan_enum_initialization_family_inner(
        dex,
        class,
        constructor_inputs,
        clinit_reference,
        clinit_ir,
        resolver,
        Some((dex_index, inventory)),
        Some(inputs),
    )
}

// Extract only the original suffix and pure numeric definitions it consumes.
// Dropped prefix allocations, strings, calls and fields are never re-created:
// any remaining use fails SSA verification. Original block/edge identities are
// explicitly mapped, including every branch and phi operand.
fn enum_suffix_uses(instruction: &ManagedInsn) -> Vec<ValueId> {
    let mut uses = Vec::new();
    match instruction {
        ManagedInsn::Const { .. } | ManagedInsn::NewInstance { .. } => {}
        ManagedInsn::Move { value, .. }
        | ManagedInsn::Neg { value, .. }
        | ManagedInsn::Not { value, .. }
        | ManagedInsn::Cast { value, .. }
        | ManagedInsn::InstanceOf { value, .. }
        | ManagedInsn::Throw { value } => uses.push(*value),
        ManagedInsn::Phi { inputs, .. } => uses.extend(inputs.iter().map(|(_, value)| *value)),
        ManagedInsn::Arithmetic { lhs, rhs, .. }
        | ManagedInsn::Compare { lhs, rhs, .. }
        | ManagedInsn::OrderingCompare { lhs, rhs, .. } => uses.extend([*lhs, *rhs]),
        ManagedInsn::FieldGet { object, .. } => uses.extend(*object),
        ManagedInsn::FieldPut { object, value, .. } => {
            uses.extend(*object);
            uses.push(*value);
        }
        ManagedInsn::ArrayGet { array, index, .. } => uses.extend([*array, *index]),
        ManagedInsn::ArrayPut {
            array,
            index,
            value,
            ..
        } => uses.extend([*array, *index, *value]),
        ManagedInsn::ArrayLength { array, .. }
        | ManagedInsn::NewArray { length: array, .. }
        | ManagedInsn::MonitorEnter { object: array }
        | ManagedInsn::MonitorExit { object: array }
        | ManagedInsn::Switch { key: array, .. } => uses.push(*array),
        ManagedInsn::FillArray { array, values } => {
            uses.push(*array);
            uses.extend(values);
        }
        ManagedInsn::NewFilledArray { elements: args, .. }
        | ManagedInsn::Invoke { args, .. }
        | ManagedInsn::Opaque { reads: args, .. } => uses.extend(args),
        ManagedInsn::Branch { condition, .. } | ManagedInsn::Return { value: condition } => {
            uses.extend(*condition)
        }
    }
    uses
}
#[allow(clippy::too_many_arguments)]
fn enum_suffix_source<R: ConstructorResolver>(
    dex: &DexFile,
    class: &ClassDef,
    reference: MethodRef,
    ir: &MethodIr<SsaForm>,
    resolver: &R,
    dex_index: usize,
    inventory: &super::initializers::write_inventory::WriteInventory,
    entry: decomp_core::ids::BlockId,
    prefix: &HashSet<decomp_core::ids::BlockId>,
    fields: &[&dex_format::class::EncodedField],
    backing: u32,
    constants: &[EnumConstant],
    constructors: &HashSet<u32>,
    helper: Option<u32>,
) -> Result<(String, MethodIr<SsaForm>), Error> {
    let cfg = ir.cfg();
    let mut suffix = HashSet::new();
    let mut pending = vec![entry];
    while let Some(block) = pending.pop() {
        if prefix.contains(&block) {
            return unsupported();
        }
        if !suffix.insert(block) {
            continue;
        }
        let node = cfg.block(block).map_err(|_| Error::InvalidMetadata)?;
        for edge in node.outgoing() {
            let edge = cfg.edge(*edge).map_err(|_| Error::InvalidMetadata)?;
            if matches!(edge.kind(), EdgeKind::Exception(_)) {
                return unsupported();
            }
            pending.push(edge.target());
        }
        let instruction = ir
            .instruction(*node.payload())
            .map_err(|_| Error::InvalidMetadata)?;
        let forbidden = match instruction {
            ManagedInsn::FieldGet { field, .. } => field.index() == backing,
            ManagedInsn::FieldPut { object, field, .. } => {
                let key = super::initializers::write_inventory::field_key(dex, field.index())
                    .ok_or(Error::InvalidMetadata)?;
                let declaration = inventory
                    .field_declaration(&key)
                    .ok_or(Error::EnumConstructorsUnavailable)?;
                (declaration.access_flags & 0x10 != 0
                    && !(object.is_none()
                        && fields.iter().any(|extra| extra.field_idx == field.index())))
                    || field.index() == backing
                    || constants
                        .iter()
                        .any(|constant| constant.field_idx == field.index())
            }
            ManagedInsn::NewInstance {
                class: allocated, ..
            } => allocated.index() == class.class_idx,
            ManagedInsn::Invoke { method, .. } => {
                constructors.contains(&method.index())
                    || *method == reference
                    || Some(method.index()) == helper
            }
            _ => false,
        };
        if forbidden {
            return unsupported();
        }
    }
    if suffix.len() + prefix.len() != cfg.block_count() {
        return unsupported();
    }
    let suffix_ids: HashSet<_> = suffix
        .iter()
        .map(|block| cfg.block(*block).map(|node| *node.payload()))
        .collect::<Result<_, _>>()
        .map_err(|_| Error::InvalidMetadata)?;
    let mut needed = HashSet::new();
    let mut values_to_check = Vec::new();
    for id in &suffix_ids {
        values_to_check.extend(enum_suffix_uses(
            ir.instruction(*id).map_err(|_| Error::InvalidMetadata)?,
        ));
    }
    while let Some(value) = values_to_check.pop() {
        let definition = ir
            .value(value)
            .map_err(|_| Error::EnumConstructorsUnavailable)?
            .def();
        if suffix_ids.contains(&definition) || !needed.insert(definition) {
            continue;
        }
        match ir
            .instruction(definition)
            .map_err(|_| Error::InvalidMetadata)?
        {
            ManagedInsn::Const { value, .. }
                if !matches!(value, Constant::String(_) | Constant::ClassLiteral(_)) => {}
            ManagedInsn::Move { value, .. } => values_to_check.push(*value),
            _ => return unsupported(),
        }
    }
    let mut kept = HashSet::new();
    let mut prelude = Vec::new();
    let mut block = cfg.entry();
    let mut bridge = None;
    while block != entry {
        if !prefix.contains(&block) {
            return unsupported();
        }
        let node = cfg.block(block).map_err(|_| Error::InvalidMetadata)?;
        let id = *node.payload();
        if needed.contains(&id) {
            kept.insert(id);
            prelude.push(id);
        }
        if node.outgoing().len() != 1 {
            return unsupported();
        }
        let edge = cfg
            .edge(node.outgoing()[0])
            .map_err(|_| Error::InvalidMetadata)?;
        if *edge.kind() != EdgeKind::Unconditional {
            return unsupported();
        }
        bridge = Some(node.outgoing()[0]);
        block = edge.target();
    }
    let mut result_cfg = decomp_core::cfg::Cfg::new(
        *prelude.first().unwrap_or(
            cfg.block(entry)
                .map_err(|_| Error::InvalidMetadata)?
                .payload(),
        ),
    );
    let mut last = result_cfg.entry();
    let mut block_map = HashMap::new();
    let mut edge_map = HashMap::new();
    if prelude.is_empty() {
        block_map.insert(entry, last);
    } else {
        for id in prelude.iter().skip(1) {
            let next = result_cfg.add_block(*id);
            result_cfg
                .add_edge(last, next, EdgeKind::Unconditional)
                .map_err(|_| Error::InvalidMetadata)?;
            last = next;
        }
        let next = result_cfg.add_block(
            *cfg.block(entry)
                .map_err(|_| Error::InvalidMetadata)?
                .payload(),
        );
        let edge = result_cfg
            .add_edge(last, next, EdgeKind::Unconditional)
            .map_err(|_| Error::InvalidMetadata)?;
        edge_map.insert(bridge.ok_or(Error::InvalidMetadata)?, edge);
        block_map.insert(entry, next);
    }
    for original in cfg.block_ids().filter(|block| suffix.contains(block)) {
        let id = *cfg
            .block(original)
            .map_err(|_| Error::InvalidMetadata)?
            .payload();
        kept.insert(id);
        if original != entry {
            block_map.insert(original, result_cfg.add_block(id));
        }
    }
    for original in cfg.edge_ids() {
        let edge = cfg.edge(original).map_err(|_| Error::InvalidMetadata)?;
        if suffix.contains(&edge.source()) {
            let copied = result_cfg
                .add_edge(
                    block_map[&edge.source()],
                    block_map[&edge.target()],
                    edge.kind().clone(),
                )
                .map_err(|_| Error::InvalidMetadata)?;
            edge_map.insert(original, copied);
        }
    }
    let mut insns = ir.instructions().clone();
    insns.retain(|id, _| kept.contains(&id));
    for (_, instruction) in &mut insns {
        match instruction {
            ManagedInsn::Branch {
                then_edge,
                else_edge,
                ..
            } => {
                *then_edge = *edge_map
                    .get(then_edge)
                    .ok_or(Error::EnumConstructorsUnavailable)?;
                if let Some(edge) = else_edge {
                    *edge = *edge_map
                        .get(edge)
                        .ok_or(Error::EnumConstructorsUnavailable)?;
                }
            }
            ManagedInsn::Phi { inputs, .. } => {
                for (edge, _) in inputs {
                    *edge = *edge_map
                        .get(edge)
                        .ok_or(Error::EnumConstructorsUnavailable)?;
                }
            }
            ManagedInsn::Switch {
                default_edge,
                cases,
                ..
            } => {
                *default_edge = *edge_map
                    .get(default_edge)
                    .ok_or(Error::EnumConstructorsUnavailable)?;
                for (_, edge) in cases {
                    *edge = *edge_map
                        .get(edge)
                        .ok_or(Error::EnumConstructorsUnavailable)?;
                }
            }
            _ => {}
        }
    }
    let mut values = ir.values().clone();
    values.retain(|_, value| kept.contains(&value.def()));
    let suffix_ir = MethodIr::<SsaForm>::from_parts(result_cfg, insns, values);
    if !managed_ir::verify::SsaVerifier::new()
        .verify(&suffix_ir)
        .is_empty()
    {
        return unsupported();
    }
    let mut writable = Vec::new();
    let mut finals = Vec::new();
    for field in fields {
        let key = super::initializers::write_inventory::field_key(dex, field.field_idx)
            .ok_or(Error::InvalidMetadata)?;
        let declaration = inventory
            .field_declaration(&key)
            .ok_or(Error::EnumConstructorsUnavailable)?;
        if declaration.dex != dex_index
            || declaration.field != field.field_idx
            || declaration.access_flags != field.access_flags
        {
            return unsupported();
        }
        let field_reference = managed_ir::refs::FieldRef::new(field.field_idx);
        if field.access_flags & 0x10 == 0 {
            writable.push(
                resolver
                    .field(field_reference)
                    .ok_or(Error::InvalidMetadata)?,
            );
        } else {
            let writes = inventory
                .explicit_store_writes(&key)
                .map_err(|_| Error::EnumConstructorsUnavailable)?;
            let local=suffix_ir.instructions().values().filter(|instruction|matches!(instruction,ManagedInsn::FieldPut{object:None,field:target,..}if *target==field_reference)).count();
            if writes.is_empty()
                || writes.len() != local
                || writes.iter().any(|write| {
                    write.dex != dex_index || write.method != reference.index() || !write.is_static
                })
            {
                return unsupported();
            }
            finals.push(field_reference);
        }
    }
    let method = resolver.method(reference).ok_or(Error::InvalidMetadata)?;
    let source = backend_java::static_initializer::emit_static_initializer_body_subset(
        &suffix_ir,
        &method,
        &ParameterBindings {
            receiver: None,
            parameters: Vec::new(),
            names: Vec::new(),
        },
        resolver,
        &backend_java::static_initializer::StaticInitializerContext {
            owner: method.owner.clone(),
            writable_static_fields: writable,
        },
        &finals,
    )
    .map_err(|_| Error::EnumConstructorsUnavailable)?;
    Ok((source, suffix_ir))
}

// Encoded values are installed before the original enum allocations, so a
// later static block is not an equivalent representation. ConstantValue source
// declarations preserve preparation. They are admitted only for final literal
// fields with no explicit input reads or stores: otherwise javac could replace
// an original GETSTATIC and erase native/reflection-visible updates. This is an
// explicit source-instruction proof, never runtime field immutability.
fn enum_encoded_preparation<R: ConstructorResolver>(
    dex: &DexFile,
    class: &ClassDef,
    extra_fields: &[&dex_format::class::EncodedField],
    resolver: &R,
    evidence: Option<(usize, &super::initializers::write_inventory::WriteInventory)>,
    inputs: Option<&[DexFile]>,
) -> Result<BTreeMap<u32, String>, Error> {
    use dex_format::encoded_value::EncodedValueKind as V;
    let values = class.static_values.as_deref().unwrap_or_default();
    let data = class.class_data.as_ref().ok_or(Error::InvalidMetadata)?;
    if values.len() > data.static_fields.len() {
        return unsupported();
    }
    if values.is_empty() && (inputs.is_none() || extra_fields.is_empty()) {
        return Ok(BTreeMap::new());
    }
    let inputs = inputs.ok_or(Error::EnumConstructorsUnavailable)?;
    let (dex_index, inventory) = evidence.ok_or(Error::EnumConstructorsUnavailable)?;
    // Reject a filtered input list against the original complete inventory.
    // Equal counts are not provenance: complete, unchanged inputs remain the
    // caller contract, and every supplied original body is scanned below.
    if inventory.has_structural_unknown()
        || inputs
            .iter()
            .flat_map(|input| &input.classes)
            .filter_map(|owner| owner.class_data.as_ref())
            .map(|members| members.direct_methods.len() + members.virtual_methods.len())
            .sum::<usize>()
            != inventory.scanned_methods()
    {
        return unsupported();
    }
    let mut prepared = BTreeMap::new();
    for (field, value) in data.static_fields.iter().zip(values) {
        let raw = dex
            .field_id(field.field_idx)
            .map_err(|_| Error::InvalidMetadata)?;
        let ty = crate::dex_java::descriptor_type(descriptor(dex, u32::from(raw.type_idx))?)
            .ok_or(Error::InvalidMetadata)?;
        let default = match (&value.value, &ty) {
            (V::Boolean(false), JavaType::Boolean)
            | (V::Byte(0), JavaType::Byte)
            | (V::Short(0), JavaType::Short)
            | (V::Char(0), JavaType::Char)
            | (V::Int(0), JavaType::Int)
            | (V::Long(0), JavaType::Long)
            | (V::Null, JavaType::Class(_) | JavaType::Array(_)) => true,
            (V::Float(v), JavaType::Float) => v.to_bits() == 0,
            (V::Double(v), JavaType::Double) => v.to_bits() == 0,
            _ => false,
        };
        if !extra_fields.iter().any(|f| f.field_idx == field.field_idx) {
            if !default {
                return unsupported();
            }
            continue;
        }
        let key = super::initializers::write_inventory::field_key(dex, field.field_idx)
            .ok_or(Error::InvalidMetadata)?;
        let declaration = inventory
            .field_declaration(&key)
            .ok_or(Error::EnumConstructorsUnavailable)?;
        if declaration.dex != dex_index
            || declaration.field != field.field_idx
            || declaration.access_flags != field.access_flags
        {
            return unsupported();
        }
        let writes = inventory
            .explicit_store_writes(&key)
            .map_err(|_| Error::EnumConstructorsUnavailable)?;
        if !writes.is_empty() || field.access_flags & 0x10 == 0 {
            // An actual suffix store must retain exactly its Java blank-final
            // proof. Only the bit/type-exact VM default can be omitted.
            if !default {
                return unsupported();
            }
            continue;
        }
        // A final enum has no legitimate inherited symbolic read owner. Without
        // that declaration, an exact-owner read map cannot prove no source reads.
        if class.access_flags & 0x10 == 0
            || inventory
                .known_reads(&key)
                .is_some_and(|reads| !reads.is_empty())
            || matches!(ty, JavaType::Class(ref name) if name != "java.lang.String")
            || matches!(ty, JavaType::Array(_) | JavaType::Void)
            || matches!(value.value, V::Null)
        {
            return unsupported();
        }
        if resolver
            .field(managed_ir::refs::FieldRef::new(field.field_idx))
            .is_none_or(|mapped| mapped.ty != ty)
        {
            return Err(Error::InvalidMetadata);
        }
        // Reuse the existing exact kind/type/bit/literal validation without
        // changing any original declaration or published flag.
        let mut literal = class.clone();
        literal.access_flags &= !0x4000;
        literal.static_values = Some(vec![value.clone()]);
        let copied = literal.class_data.as_mut().ok_or(Error::InvalidMetadata)?;
        copied.static_fields = vec![field.clone()];
        copied.instance_fields.clear();
        let rendered =
            super::initializers::encoded_field_initializations(dex, &literal, &HashSet::new())?;
        let [initialization] = rendered.as_slice() else {
            return unsupported();
        };
        prepared.insert(field.field_idx, initialization.expression.clone());
    }
    // DEX omits trailing VM-default encoded values. A final primitive with
    // no original store is already zero before enum allocation; demanding a
    // suffix assignment invents an initialization operation. Preserve that
    // preparation using the same complete-input/no-read contract as explicit
    // literals. Reference defaults are not Java constant variables and remain
    // outside this bounded representation.
    for field in data.static_fields.iter().skip(values.len()) {
        if !extra_fields
            .iter()
            .any(|candidate| candidate.field_idx == field.field_idx)
            || field.access_flags & 0x10 == 0
        {
            continue;
        }
        let raw = dex
            .field_id(field.field_idx)
            .map_err(|_| Error::InvalidMetadata)?;
        let ty = crate::dex_java::descriptor_type(descriptor(dex, u32::from(raw.type_idx))?)
            .ok_or(Error::InvalidMetadata)?;
        let literal = match &ty {
            JavaType::Boolean => "false",
            JavaType::Byte => "(byte) 0",
            JavaType::Short => "(short) 0",
            JavaType::Char => "(char) 0",
            JavaType::Int => "0",
            JavaType::Long => "0L",
            JavaType::Float => "0.0f",
            JavaType::Double => "0.0d",
            _ => continue,
        };
        let key = super::initializers::write_inventory::field_key(dex, field.field_idx)
            .ok_or(Error::InvalidMetadata)?;
        let declaration = inventory
            .field_declaration(&key)
            .ok_or(Error::EnumConstructorsUnavailable)?;
        if declaration.dex != dex_index
            || declaration.field != field.field_idx
            || declaration.access_flags != field.access_flags
        {
            return unsupported();
        }
        let writes = inventory
            .explicit_store_writes(&key)
            .map_err(|_| Error::EnumConstructorsUnavailable)?;
        if !writes.is_empty() {
            continue;
        }
        if class.access_flags & 0x10 == 0
            || inventory
                .known_reads(&key)
                .is_some_and(|reads| !reads.is_empty())
        {
            return unsupported();
        }
        if resolver
            .field(managed_ir::refs::FieldRef::new(field.field_idx))
            .is_none_or(|mapped| mapped.ty != ty)
        {
            return Err(Error::InvalidMetadata);
        }
        prepared.insert(field.field_idx, literal.into());
    }
    if prepared.is_empty() {
        return Ok(prepared);
    }
    // No represented original read may be folded, nor store moved, by a declaration initializer.
    // Scan all bodies including unreachable/dropped code. Code-free genuine
    // native/abstract declarations are runtime capabilities, not Java GETSTATIC
    // instructions; all malformed or undecodable bodies fail closed.
    for input in inputs {
        for owner in &input.classes {
            let Some(members) = &owner.class_data else {
                continue;
            };
            for method in members
                .direct_methods
                .iter()
                .chain(&members.virtual_methods)
            {
                let Some(code) = &method.code else {
                    if method.access_flags & (0x100 | 0x400) == 0 {
                        return unsupported();
                    }
                    continue;
                };
                let decoded = dex_decompiler::decode_method(code)
                    .map_err(|_| Error::EnumConstructorsUnavailable)?;
                for instruction in decoded {
                    let dex_decompiler::Opcode::Supported(info) = instruction.opcode else {
                        continue;
                    };
                    if !(0x52..=0x6d).contains(&info.opcode) {
                        continue;
                    }
                    let key = instruction
                        .pool_index
                        .and_then(|index| {
                            super::initializers::write_inventory::field_key(input, index)
                        })
                        .ok_or(Error::EnumConstructorsUnavailable)?;
                    for field in prepared.keys() {
                        if super::initializers::write_inventory::field_key(dex, *field)
                            .is_some_and(|target| target == key)
                        {
                            return unsupported();
                        }
                    }
                }
            }
        }
    }
    Ok(prepared)
}

#[allow(clippy::too_many_arguments)]
fn plan_enum_initialization_family_inner<R: ConstructorResolver>(
    dex: &DexFile,
    class: &ClassDef,
    constructor_inputs: &[EnumConstructorDeclarationInput<'_>],
    clinit_reference: MethodRef,
    clinit_ir: &MethodIr<SsaForm>,
    resolver: &R,
    inventory: Option<(usize, &super::initializers::write_inventory::WriteInventory)>,
    prepared_inputs: Option<&[DexFile]>,
) -> Result<EnumInitializationPlan, Error> {
    if class_kind(class.access_flags)? != ClassKind::Enum
        || class.access_flags & !(1 | 0x10 | 0x1000 | 0x4000) != 0
        || descriptor(dex, class.superclass_idx)? != "Ljava/lang/Enum;"
        || (class.static_values.as_ref().is_some_and(|v| !v.is_empty())
            && prepared_inputs.is_none())
        || (class.static_values_off != 0 && class.static_values.is_none())
    {
        return unsupported();
    }
    let data = class
        .class_data
        .as_ref()
        .ok_or(Error::EnumConstructorsUnavailable)?;
    let owner = resolver
        .class(managed_ir::refs::ClassRef::new(class.class_idx))
        .ok_or(Error::InvalidMetadata)?;
    let simple = owner.rsplit('.').next().ok_or(Error::InvalidMetadata)?;
    if backend_java::sanitize_identifier(simple) != simple || simple.is_empty() {
        return unsupported();
    }
    // An implements clause retains original metadata, but a type-pool entry
    // alone is not evidence that javac may legally implement that type. This
    // bounded plan accepts exact same-DEX interface declarations and access;
    // external/cross-DEX interfaces need additional declaration evidence.
    let mut interface_indices = HashSet::new();
    let mut interfaces = Vec::new();
    for index in class.interfaces.iter().flat_map(|list| &list.types) {
        let index = u32::from(*index);
        let target = dex
            .classes
            .iter()
            .find(|c| c.class_idx == index)
            .ok_or(Error::EnumConstructorsUnavailable)?;
        let name = resolver
            .class(managed_ir::refs::ClassRef::new(index))
            .ok_or(Error::InvalidMetadata)?;
        if !interface_indices.insert(index)
            || class_kind(target.access_flags)? != ClassKind::Interface
            || target.access_flags & (2 | 4) != 0
            || (target.access_flags & 7).count_ones() > 1
            || (target.access_flags & 1 == 0
                && (name.rsplit_once('.').map(|p| p.0) != owner.rsplit_once('.').map(|p| p.0)
                    || descriptor(dex, index)?.rsplit_once('/').map(|p| p.0)
                        != descriptor(dex, class.class_idx)?
                            .rsplit_once('/')
                            .map(|p| p.0)))
            || name
                .split('.')
                .any(|part| part.is_empty() || backend_java::sanitize_identifier(part) != part)
        {
            return unsupported();
        }
        interfaces.push(name);
    }
    let hidden = vec![JavaType::Class("java.lang.String".into()), JavaType::Int];
    let declared: HashSet<_> = data
        .direct_methods
        .iter()
        .chain(&data.virtual_methods)
        .filter(|m| {
            dex.method_id(m.method_idx)
                .ok()
                .and_then(|r| dex.string(r.name_idx).ok())
                == Some("<init>")
        })
        .map(|m| m.method_idx)
        .collect();
    let supplied: HashSet<_> = constructor_inputs
        .iter()
        .map(|c| c.reference.index())
        .collect();
    if declared.is_empty() || declared != supplied || supplied.len() != constructor_inputs.len() {
        return unsupported();
    }
    if let Some((dex_index, inventory)) = inventory {
        for field in data
            .instance_fields
            .iter()
            .filter(|field| field.access_flags & 0x10 != 0)
        {
            let key = super::initializers::write_inventory::field_key(dex, field.field_idx)
                .ok_or(Error::InvalidMetadata)?;
            let declaration = inventory
                .field_declaration(&key)
                .ok_or(Error::EnumConstructorsUnavailable)?;
            let writes = inventory
                .explicit_store_writes(&key)
                .map_err(|_| Error::EnumConstructorsUnavailable)?;
            if declaration.dex != dex_index
                || declaration.field != field.field_idx
                || declaration.access_flags != field.access_flags
                || writes.iter().any(|write| {
                    write.dex != dex_index || write.is_static || !supplied.contains(&write.method)
                })
            {
                return unsupported();
            }
        }
    }
    let clinits: Vec<_> = data
        .direct_methods
        .iter()
        .chain(&data.virtual_methods)
        .filter(|m| {
            dex.method_id(m.method_idx)
                .ok()
                .and_then(|r| dex.string(r.name_idx).ok())
                == Some("<clinit>")
        })
        .collect();
    if clinits.len() != 1
        || clinits[0].method_idx != clinit_reference.index()
        || clinits[0].access_flags & !(8 | 0x1000 | 0x10000) != 0
        || clinits[0].access_flags & 8 == 0
    {
        return unsupported();
    }
    let finals: Vec<_> = data
        .instance_fields
        .iter()
        .filter(|f| f.access_flags & 0x10 != 0)
        .map(|f| managed_ir::refs::FieldRef::new(f.field_idx))
        .collect();
    let mut signatures = HashMap::new();
    let mut constructor_sources = BTreeMap::new();
    let mut chains = HashMap::new();
    let mut assignments = HashMap::new();
    for input in constructor_inputs {
        let raw = dex
            .method_id(input.reference.index())
            .map_err(|_| Error::InvalidMetadata)?;
        let encoded = data
            .direct_methods
            .iter()
            .find(|m| m.method_idx == input.reference.index())
            .ok_or(Error::EnumConstructorsUnavailable)?;
        let sig = resolver
            .initializer(input.reference)
            .ok_or(Error::InvalidMetadata)?;
        if u32::from(raw.class_idx) != class.class_idx
            || sig.method.owner != owner
            || !sig.method.parameters.starts_with(&hidden)
            || !sig.method.throws.is_empty()
        {
            return unsupported();
        }
        // Optimizers may widen enum constructors to public/package access.
        // Java enum syntax cannot retain that original reflective access. Do
        // not silently privatize it or conflate it with a missing body proof.
        if encoded.access_flags & !(2 | 0x1000 | 0x10000) != 0 || encoded.access_flags & 2 == 0 {
            return Err(Error::EnumConstructorAccessUnrepresentable {
                owner: descriptor(dex, class.class_idx)?.to_owned(),
                method: input.reference.index(),
                access_flags: encoded.access_flags,
            });
        }
        if encoded.access_flags & !(2 | 0x1000 | 0x10000) != 0 {
            // Unsupported modifiers (e.g. varargs) are a separate source
            // declaration issue, not evidence of nonprivate constructor access.
            return Err(Error::InvalidFlags);
        }
        let proto = dex
            .proto_id(u32::from(raw.proto_idx))
            .map_err(|_| Error::InvalidMetadata)?;
        let exact: Vec<_> = proto
            .parameters
            .iter()
            .flat_map(|p| &p.types)
            .map(|t| {
                descriptor(dex, u32::from(*t))
                    .ok()
                    .and_then(crate::dex_java::descriptor_type)
            })
            .collect::<Option<_>>()
            .ok_or(Error::InvalidMetadata)?;
        if exact != sig.method.parameters || descriptor(dex, proto.return_type_idx)? != "V" {
            return unsupported();
        }
        let plan = plan_constructor_declaration(
            input.ir,
            input.reference,
            "java.lang.Enum",
            input.bindings,
            resolver,
        )
        .map_err(|_| Error::EnumConstructorsUnavailable)?;
        if plan.parameter_names.len() < 2
            || !plan.relocated_pre_chain_fields.is_empty()
            || !plan.prologue.is_empty()
            || !plan.preserved_pre_chain_fields.is_empty()
        {
            return unsupported();
        }
        let target = resolver
            .initializer(plan.target)
            .ok_or(Error::InvalidMetadata)?;
        if plan.chain == ChainKind::Super {
            let raw_target = dex
                .method_id(plan.target.index())
                .map_err(|_| Error::InvalidMetadata)?;
            let proto_target = dex
                .proto_id(u32::from(raw_target.proto_idx))
                .map_err(|_| Error::InvalidMetadata)?;
            let params = proto_target
                .parameters
                .iter()
                .flat_map(|p| &p.types)
                .map(|t| descriptor(dex, u32::from(*t)))
                .collect::<Result<Vec<_>, _>>()?;
            if descriptor(dex, u32::from(raw_target.class_idx))? != "Ljava/lang/Enum;"
                || dex.string(raw_target.name_idx).ok() != Some("<init>")
                || params != ["Ljava/lang/String;", "I"]
                || descriptor(dex, proto_target.return_type_idx)? != "V"
            {
                return unsupported();
            }
        }
        let chain = match plan.chain {
            ChainKind::Super
                if target.method.owner == "java.lang.Enum"
                    && target.method.parameters == hidden
                    && target.method.throws.is_empty()
                    && plan.first_statement
                        == format!(
                            "super({}, {});",
                            plan.parameter_names[0], plan.parameter_names[1]
                        ) =>
            {
                None
            }
            ChainKind::This
                if target.method.owner == owner && supplied.contains(&plan.target.index()) =>
            {
                let prefix = format!(
                    "this({}, {}",
                    plan.parameter_names[0], plan.parameter_names[1]
                );
                let suffix = plan
                    .first_statement
                    .strip_prefix(&prefix)
                    .ok_or(Error::EnumConstructorsUnavailable)?;
                let source = if suffix == ");" {
                    "this();".into()
                } else if let Some(args) = suffix.strip_prefix(", ") {
                    format!("this({args}")
                } else {
                    return unsupported();
                };
                Some((plan.target.index(), source))
            }
            _ => return unsupported(),
        };
        // Only original SSA uses decide whether name/ordinal survive. A
        // qualified field such as `this.p1`, or a string containing `p1`, is
        // not a use of the hidden ordinal parameter named `p1`.
        if !hidden_parameters::chain_only(input, plan.target, &target.method.parameters) {
            return unsupported();
        }
        let evidence = constructor_initialization_evidence(
            &ConstructorInput {
                reference: input.reference,
                ir: input.ir,
                bindings: input.bindings,
            },
            "java.lang.Enum",
            &finals,
            resolver,
        )
        .map_err(|_| Error::EnumConstructorsUnavailable)?;
        let mut counts = Vec::new();
        for field in evidence.fields {
            if !field.before_chain.is_empty()
                || field.read_before_assignment
                || field.normal_return_counts.len() != 1
            {
                return unsupported();
            }
            counts.push(
                *field
                    .normal_return_counts
                    .iter()
                    .next()
                    .ok_or(Error::EnumConstructorsUnavailable)?,
            );
        }
        assignments.insert(input.reference.index(), counts);
        chains.insert(input.reference.index(), chain.as_ref().map(|c| c.0));
        let params = sig
            .method
            .parameters
            .iter()
            .zip(&plan.parameter_names)
            .skip(2)
            .map(|(ty, name)| Ok(format!("{} {name}", type_source(ty)?)))
            .collect::<Result<Vec<_>, Error>>()?;
        let statements = chain
            .iter()
            .map(|(_, s)| s.as_str())
            .chain(plan.body.iter().map(String::as_str))
            .map(|s| format!("    {s}"))
            .collect::<Vec<_>>()
            .join("\n");
        constructor_sources.insert(
            input.reference.index(),
            format!(
                "private {simple}({}) {{\n{statements}\n}}",
                params.join(", ")
            ),
        );
        signatures.insert(input.reference, sig.method.parameters);
    }
    for root in &supplied {
        let mut at = Some(*root);
        let mut visited = HashSet::new();
        let mut totals = vec![0u16; finals.len()];
        while let Some(current) = at {
            if !visited.insert(current) {
                return unsupported();
            }
            for (total, count) in totals.iter_mut().zip(
                assignments
                    .get(&current)
                    .ok_or(Error::EnumConstructorsUnavailable)?,
            ) {
                *total = total.saturating_add(u16::from(*count));
            }
            at = *chains
                .get(&current)
                .ok_or(Error::EnumConstructorsUnavailable)?;
        }
        if totals.iter().any(|n| *n != 1) {
            return unsupported();
        }
    }
    // Check the initializer identity directly, independently of the resolver.
    let raw = dex
        .method_id(clinit_reference.index())
        .map_err(|_| Error::InvalidMetadata)?;
    let proto = dex
        .proto_id(u32::from(raw.proto_idx))
        .map_err(|_| Error::InvalidMetadata)?;
    if u32::from(raw.class_idx) != class.class_idx
        || descriptor(dex, proto.return_type_idx)? != "V"
        || proto
            .parameters
            .as_ref()
            .is_some_and(|p| !p.types.is_empty())
    {
        return unsupported();
    }
    let mut fields = HashMap::new();
    let mut backing = None;
    let mut extra_static_fields = Vec::new();
    for f in &data.static_fields {
        let raw = dex
            .field_id(f.field_idx)
            .map_err(|_| Error::InvalidMetadata)?;
        let field = resolver
            .field(managed_ir::refs::FieldRef::new(f.field_idx))
            .ok_or(Error::InvalidMetadata)?;
        if u32::from(raw.class_idx) != class.class_idx || field.owner != owner || !field.is_static {
            return unsupported();
        }
        let raw_name = dex
            .string(raw.name_idx)
            .map_err(|_| Error::InvalidMetadata)?;
        if backend_java::sanitize_identifier(raw_name) != raw_name
            || raw_name.is_empty()
            || field.name != raw_name
        {
            return unsupported();
        }
        if f.access_flags == 0x4019
            && descriptor(dex, u32::from(raw.type_idx))? == descriptor(dex, class.class_idx)?
            && field.ty == JavaType::Class(owner.clone())
        {
            if fields.insert(f.field_idx, raw_name.to_owned()).is_some() {
                return unsupported();
            }
        } else if f.access_flags == 0x101a
            && descriptor(dex, u32::from(raw.type_idx))?
                == format!("[{}", descriptor(dex, class.class_idx)?)
            && field.ty == JavaType::Array(Box::new(JavaType::Class(owner.clone())))
        {
            if backing.replace(f.field_idx).is_some() {
                return unsupported();
            }
        } else {
            if inventory.is_none()
                || f.access_flags & 8 == 0
                || f.access_flags & !(1 | 2 | 4 | 8 | 0x10 | 0x40 | 0x80 | 0x1000) != 0
                || (f.access_flags & 7).count_ones() > 1
                || f.access_flags & 0x50 == 0x50
                || field.ty == JavaType::Void
            {
                return unsupported();
            }
            extra_static_fields.push(f);
        }
    }
    let encoded_declaration_initializers = enum_encoded_preparation(
        dex,
        class,
        &extra_static_fields,
        resolver,
        inventory,
        prepared_inputs,
    )?;
    // Prepared literal finals have no original stores. The original suffix
    // still proves every remaining blank final once on each normal path.
    extra_static_fields
        .retain(|field| !encoded_declaration_initializers.contains_key(&field.field_idx));
    // Restored constructor bodies retain effects, but Java still prohibits
    // writes to compiler-owned enum state and constructing enum instances.
    for input in constructor_inputs {
        for (_, insn) in input.ir.instructions() {
            let forbidden = match insn {
                ManagedInsn::FieldPut { field, .. } => {
                    fields.contains_key(&field.index())
                        || Some(field.index()) == backing
                        || extra_static_fields
                            .iter()
                            .any(|f| f.field_idx == field.index())
                }
                ManagedInsn::FieldGet { field, .. } => {
                    Some(field.index()) == backing
                        || extra_static_fields
                            .iter()
                            .any(|f| f.field_idx == field.index())
                }
                ManagedInsn::Invoke { method, .. } => *method == clinit_reference,
                ManagedInsn::NewInstance {
                    class: allocated, ..
                } => allocated.index() == class.class_idx,
                _ => false,
            };
            if forbidden {
                return unsupported();
            }
        }
    }
    let backing = backing.ok_or(Error::EnumConstructorsUnavailable)?;
    if fields.is_empty() {
        return unsupported();
    }
    let cfg = clinit_ir.cfg();
    cfg.validate()
        .map_err(|_| Error::EnumConstructorsUnavailable)?;
    let mut block = cfg.entry();
    let mut seen = HashSet::new();
    let mut values = HashMap::<ValueId, Value>::new();
    let mut constants = Vec::new();
    let mut instances = Vec::new();
    let mut stored_instances = HashMap::new();
    let mut pending = None;
    let mut payload_effects = Vec::new();
    let mut values_stored = false;
    let mut values_helper = None;
    let mut suffix_entry = None;
    loop {
        if !seen.insert(block) {
            return unsupported();
        }
        let node = cfg.block(block).map_err(|_| Error::InvalidMetadata)?;
        let id = *node.payload();
        let insn = clinit_ir
            .instruction(id)
            .map_err(|_| Error::InvalidMetadata)?;
        if values_stored && !matches!(insn, ManagedInsn::Return { value: None }) {
            if inventory.is_none() {
                return unsupported();
            }
            suffix_entry = Some(block);
            seen.remove(&block);
            break;
        }
        let mut definition = None;
        match insn {
            ManagedInsn::Const { result, value } if !values_stored => {
                definition = Some((
                    *result,
                    match value {
                        Constant::Int(i) => Value::Int(*i),
                        Constant::Zero => Value::Int(0),
                        Constant::String(s) => {
                            Value::String(resolver.string(*s).ok_or(Error::InvalidMetadata)?)
                        }
                        Constant::ClassLiteral(class) if pending.is_some() => {
                            let name = resolver.class(managed_ir::refs::ClassRef::new(class.index())).ok_or(Error::InvalidMetadata)?;
                            payload_effects.push(id);
                            Value::ReferenceExpression {
                                source: format!("{name}.class"),
                                ty: JavaType::Class("java.lang.Class".into()),
                                effects: vec![id],
                            }
                        }
                        Constant::ClassLiteral(_) => return unsupported(),
                        other => Value::Literal(*other),
                    },
                ));
            }
            ManagedInsn::Move { result, value } if !values_stored => {
                definition = Some((
                    *result,
                    values
                        .get(value)
                        .cloned()
                        .ok_or(Error::EnumConstructorsUnavailable)?,
                ));
            }
            ManagedInsn::NewInstance {
                result,
                class: allocated,
            } if pending.is_none() && !values_stored => {
                if allocated.index() != class.class_idx || fields.is_empty() || !payload_effects.is_empty() {
                    return unsupported();
                }
                pending = Some(*result);
                definition = Some((
                    *result,
                    Value::Instance {
                        allocation: *result,
                        initialized: false,
                        stored: false,
                    },
                ));
            }
            ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                result: None,
                method,
                args,
            } if signatures.contains_key(method)
                && args.first().is_some_and(|v| matches!(values.get(v), Some(Value::Instance { allocation, .. }) if pending == Some(*allocation))) =>
            {
                let args = invocation_operands(args, &signatures[method])?;
                if !matches!(
                    values.get(&args[0]),
                    Some(Value::Instance {
                        initialized: false,
                        stored: false,
                        ..
                    })
                ) || !matches!(values.get(&args[1]), Some(Value::String(_)))
                    || !matches!(values.get(&args[2]), Some(Value::Int(i)) if *i >= 0 && *i as usize == constants.len())
                {
                    return unsupported();
                }
                let Value::String(name) = values.get(&args[1]).ok_or(Error::InvalidMetadata)?
                else {
                    return unsupported();
                };
                let name =
                    String::from_utf16(name).map_err(|_| Error::EnumConstructorsUnavailable)?;
                // Java evaluates constant payloads after this original NEW and
                // before its initializer. Require every retained effect exactly
                // once, in original execution order; shared results cannot be
                // duplicated or moved across enum allocations.
                let argument_values = args.iter().skip(3).map(|value| values.get(value).ok_or(Error::EnumConstructorsUnavailable)).collect::<Result<Vec<_>, _>>()?;
                if !payload_effects_match(argument_values, &payload_effects) { return unsupported(); }
                payload_effects.clear();
                // Record name now; the next effect must store this exact instance.
                constants.push(EnumConstant {
                    field_idx: u32::MAX,
                    name,
                    ordinal: constants.len(),
                    arguments: args
                        .iter()
                        .skip(3)
                        .zip(signatures[method].iter().skip(2))
                        .map(|(v, ty)| {
                            constant_source(
                                values.get(v).ok_or(Error::EnumConstructorsUnavailable)?,
                                ty,
                                resolver,
                            )
                        })
                        .collect::<Result<_, _>>()?,
                });
                let allocation = pending.ok_or(Error::EnumConstructorsUnavailable)?;
                for state in values.values_mut() {
                    if let Value::Instance { allocation: root, initialized, .. } = state
                        && *root == allocation { *initialized = true; }
                }

            }
            ManagedInsn::FieldPut {
                object: None,
                field,
                value,
            } if matches!(values.get(value), Some(Value::Instance { allocation, .. }) if pending == Some(*allocation)) => {
                if !matches!(
                    values.get(value),
                    Some(Value::Instance {
                        initialized: true,
                        stored: false,
                        ..
                    })
                ) {
                    return unsupported();
                }
                let constant = constants
                    .last_mut()
                    .ok_or(Error::EnumConstructorsUnavailable)?;
                if fields.remove(&field.index()).as_deref() != Some(constant.name.as_str()) {
                    return unsupported();
                }
                constant.field_idx = field.index();
                let allocation = pending.ok_or(Error::EnumConstructorsUnavailable)?;
                instances.push(allocation);
                stored_instances.insert(field.index(), *value);
                for state in values.values_mut() {
                    if let Value::Instance { allocation: root, stored, .. } = state
                        && *root == allocation { *stored = true; }
                }
                pending = None;
            }
            ManagedInsn::FieldGet {
                result,
                object: None,
                field,
            } if !values_stored && stored_instances.contains_key(&field.index()) => {
                let source = stored_instances[&field.index()];
                // Track identity rather than introducing a second instance.
                definition = Some((
                    *result,
                    values
                        .get(&source)
                        .cloned()
                        .ok_or(Error::EnumConstructorsUnavailable)?,
                ));

            }
            ManagedInsn::FieldGet { result, object: None, field }
                if pending.is_some() && !values_stored =>
            {
                let metadata = resolver.field(*field).ok_or(Error::InvalidMetadata)?;
                // Primitive/String fields could become Java constant variables
                // and erase an original active-use initialization. Reference
                // payload fields retain their actual GETSTATIC and identity.
                // A String requires an exact source nonconstant attestation;
                // otherwise javac may erase this class-initialization boundary.
                if !metadata.is_static || metadata.owner == owner
                    || !matches!(&metadata.ty, JavaType::Class(name)
                        if name != "java.lang.String" || resolver.field_source_nonconstant(*field))
                        && !matches!(&metadata.ty, JavaType::Array(_)) {
                    return unsupported();
                }
                payload_effects.push(id);
                definition = Some((*result, Value::ReferenceExpression {
                    source: format!("{}.{}", metadata.owner, metadata.name),
                    ty: metadata.ty,
                    effects: vec![id],
                }));
            }
            ManagedInsn::Invoke { kind: InvokeKind::Static, result: Some(result), method, args }
                if pending.is_some() && !values_stored =>
            {
                let metadata = resolver.method(*method).ok_or(Error::InvalidMetadata)?;
                if !metadata.is_static || metadata.owner == owner
                    || metadata.name.starts_with('<')
                    || !matches!(metadata.return_type, JavaType::Class(_) | JavaType::Array(_)) {
                    return unsupported();
                }
                // Reuse the original wide-word verification by supplying a
                // temporary receiver slot, discarded before static rendering.
                let words: Vec<_> = std::iter::once(*result).chain(args.iter().copied()).collect();
                let normalized = invocation_operands(&words, &metadata.parameters)?;
                let mut effects = Vec::new();
                let arguments = normalized.iter().skip(1).zip(&metadata.parameters)
                    .map(|(argument, ty)| {
                        let value = values.get(argument).ok_or(Error::EnumConstructorsUnavailable)?;
                        effects.extend_from_slice(expression_effects(value));
                        constant_source(value, ty, resolver)
                    }).collect::<Result<Vec<_>, _>>()?;
                effects.push(id);
                payload_effects.push(id);
                definition = Some((*result, Value::ReferenceExpression {
                    source: format!("{}.{}({})", metadata.owner, metadata.name, arguments.join(", ")),
                    ty: metadata.return_type,
                    effects,
                }));
            }
            ManagedInsn::NewArray {
                result,
                element,
                length,
            } if pending.is_none() && fields.is_empty() && !values_stored => {
                if descriptor(dex, element.index())?
                    != format!("[{}", descriptor(dex, class.class_idx)?)
                    || !matches!(values.get(length), Some(Value::Int(n)) if *n >= 0 && *n as usize == constants.len())
                    || values
                        .values()
                        .any(|v| matches!(v, Value::Values | Value::BuildingValues { .. }))
                {
                    return unsupported();
                }
                definition = Some((
                    *result,
                    Value::BuildingValues {
                        written: 0,
                        length: constants.len(),
                    },
                ));
            }
            ManagedInsn::ArrayPut {
                array,
                index,
                value,
            } if !values_stored => {
                let Some(Value::BuildingValues { written, length }) = values.get(array).cloned()
                else {
                    return unsupported();
                };
                if written >= length
                    || !matches!(values.get(index), Some(Value::Int(n)) if *n >= 0 && *n as usize == written)
                    || !matches!(values.get(value), Some(Value::Instance { allocation, initialized: true, stored: true }) if instances.get(written) == Some(allocation))
                {
                    return unsupported();
                }
                values.insert(
                    *array,
                    if written + 1 == length {
                        Value::Values
                    } else {
                        Value::BuildingValues {
                            written: written + 1,
                            length,
                        }
                    },
                );
            }
            ManagedInsn::NewFilledArray {
                result: Some(result),
                target,
                elements,
            } if pending.is_none() && fields.is_empty() && !values_stored => {
                let allocations = elements.iter().map(|v| match values.get(v) { Some(Value::Instance { allocation, initialized: true, stored: true }) => Some(*allocation), _ => None }).collect::<Option<Vec<_>>>();
                if allocations.as_ref() != Some(&instances)
                    || descriptor(dex, target.index())?
                        != format!("[{}", descriptor(dex, class.class_idx)?)
                    || values
                        .values()
                        .any(|v| matches!(v, Value::Values | Value::BuildingValues { .. }))
                {
                    return unsupported();
                }
                definition = Some((*result, Value::Values));
            }
            ManagedInsn::Invoke {
                kind: InvokeKind::Static,
                result: Some(result),
                method,
                args,
            } if args.is_empty() && pending.is_none() && fields.is_empty() && !values_stored => {
                if values
                    .values()
                    .any(|v| matches!(v, Value::Values | Value::BuildingValues { .. }))
                    || !emission::prove_values_helper(dex, class, method.index(), &constants)
                {
                    return unsupported();
                }
                values_helper = Some(method.index());
                definition = Some((*result, Value::Values));
            }
            ManagedInsn::FieldPut {
                object: None,
                field,
                value,
            } if field.index() == backing
                && matches!(values.get(value), Some(Value::Values))
                && !values_stored =>
            {
                values_stored = true;
            }
            ManagedInsn::Return { value: None }
                if values_stored && pending.is_none() && fields.is_empty() =>
            {
                if !node.outgoing().is_empty() {
                    return unsupported();
                }
                break;
            }
            _ => return unsupported(),
        }
        if let Some((result, value)) = definition
            && (values.insert(result, value).is_some()
                || clinit_ir
                    .value(result)
                    .map_err(|_| Error::InvalidMetadata)?
                    .def()
                    != id)
        {
            return unsupported();
        }
        if node.outgoing().len() != 1 {
            return unsupported();
        }
        let edge = cfg
            .edge(node.outgoing()[0])
            .map_err(|_| Error::InvalidMetadata)?;
        if *edge.kind() != EdgeKind::Unconditional {
            return unsupported();
        }
        block = edge.target();
    }
    if (suffix_entry.is_none() && seen.len() != cfg.block_count())
        || constants.iter().any(|c| c.field_idx == u32::MAX)
    {
        return unsupported();
    }
    let (static_initializer_source, static_initializer_ir) = if let Some(entry) = suffix_entry {
        let (dex_index, inventory) = inventory.ok_or(Error::EnumConstructorsUnavailable)?;
        let (source, suffix_ir) = enum_suffix_source(
            dex,
            class,
            clinit_reference,
            clinit_ir,
            resolver,
            dex_index,
            inventory,
            entry,
            &seen,
            &extra_static_fields,
            backing,
            &constants,
            &supplied,
            values_helper,
        )?;
        (Some(source), Some(suffix_ir))
    } else {
        // A final field with only a VM default has no Java blank-final store.
        // Inventing one or changing it into ConstantValue is not this proof.
        if extra_static_fields
            .iter()
            .any(|field| field.access_flags & 0x10 != 0)
        {
            return unsupported();
        }
        (None, None)
    };
    let suffix_runtime_capabilities =
        if static_initializer_source.is_some() || !encoded_declaration_initializers.is_empty() {
            inventory
                .ok_or(Error::EnumConstructorsUnavailable)?
                .1
                .unknown_observations()
                .iter()
                .filter(|observation| {
                    !matches!(
                        observation.kind,
                        super::initializers::write_inventory::UnknownKind::Structural
                    )
                })
                .count()
        } else {
            0
        };
    if let Some(helper) = values_helper {
        for input in constructor_inputs {
            if input.ir.instructions().values().any(
                |i| matches!(i, ManagedInsn::Invoke { method, .. } if method.index() == helper),
            ) {
                return unsupported();
            }
        }
    }
    let simple = owner.rsplit('.').next().ok_or(Error::InvalidMetadata)?;
    if backend_java::sanitize_identifier(simple) != simple || simple.is_empty() {
        return unsupported();
    }
    Ok(EnumInitializationPlan {
        header: format!(
            "{}enum {simple}{}",
            if class.access_flags & 1 != 0 {
                "public "
            } else {
                ""
            },
            if interfaces.is_empty() {
                String::new()
            } else {
                format!(" implements {}", interfaces.join(", "))
            }
        ),
        constants_source: format!(
            "{};",
            constants
                .iter()
                .map(|c| if c.arguments.is_empty() {
                    c.name.clone()
                } else {
                    format!("{}({})", c.name, c.arguments.join(", "))
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
        constants,
        encoded_declaration_initializers,
        absorbed_constructor: *supplied
            .iter()
            .min()
            .ok_or(Error::EnumConstructorsUnavailable)?,
        absorbed_constructors: constructor_sources.keys().copied().collect(),
        constructor_sources,
        absorbed_clinit: clinit_reference.index(),
        absorbed_values_field: backing,
        absorbed_values_helper: values_helper,
        static_initializer_source,
        static_initializer_ir,
        suffix_runtime_capabilities,
    })
}

#[cfg(test)]
#[path = "java_enum_payload_tests.rs"]
mod payload_tests;

#[cfg(test)]
#[path = "java_enum_payload_trace_wave9_tests.rs"]
mod payload_trace_wave9_tests;

#[cfg(test)]
#[path = "java_enum_constant_upcast_tests.rs"]
mod constant_upcast_tests;

#[path = "java_enum_classification.rs"]
pub mod classification;
