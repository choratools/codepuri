//! Java25 enum payload scheduling through explicitly synthetic private constructors.
//!
//! This source representation adds a constructor and stack frame. It preserves
//! modeled allocation/payload order, not reflective constructor inventory or
//! stack-trace equivalence. Helpers never represent an original method identity.
use backend_java::{JavaResolver, JavaType};
use dex_format::{DexFile, class::ClassDef};
use managed_ir::{
    insn::{Constant, InvokeKind, ManagedInsn},
    refs::MethodRef,
    value::{InsnId, ValueId},
};
use std::collections::{HashMap, HashSet};

/// Caller proofs unavailable from an instruction pool alone.
pub trait RecipeEvidence: JavaResolver {
    /// Selected Java source release; this transformation needs release25.
    fn recipe_source_release(&self) -> u16;
    /// Exact source dispatch/access/type resolution is legal from this enum.
    /// Native or unavailable implementations and unresolved operations reject.
    fn recipe_operation(&self, instruction: InsnId, operation: &ManagedInsn) -> bool;
    /// Prove compiler-eager hidden name/ordinal evaluation preserves original
    /// resolution/failure timing. Actual const-string can follow payload calls;
    /// constant spelling alone does not establish this proof. Default is closed.
    fn recipe_hidden_inputs(
        &self,
        _ssa: &dex_decompiler::SsaMethod,
        _allocation: InsnId,
        _call: InsnId,
    ) -> bool {
        false
    }
}
/// A synthetic helper, with original operation provenance kept separately.
#[derive(Debug)]
pub struct RecipePlan {
    /// Source arguments for the constant, using a collision-free primitive arity.
    pub constant_arguments: Vec<String>,
    /// Additional private constructor; original payload constructor stays intact.
    pub helper_source: String,
    /// Genuine original constructor called after the retained payload schedule.
    pub original_constructor: MethodRef,
    /// Original effect offsets, in execution order (no commuting/deduplication).
    pub effects: Vec<u32>,
    /// Synthetic representation caveat, never an original-method coverage receipt.
    pub observability_limit: &'static str,
}
/// A missing proof; never authorizes approximating the original operation.
#[derive(Debug, PartialEq, Eq)]
pub enum RecipeError {
    /// Unsupported source target or original metadata.
    Metadata,
    /// A branch, handler, shared block, or nonoriginal instruction.
    Flow,
    /// Unavailable local dependency or operation proof.
    Evidence,
    /// Invalid or contradictory source operand type.
    Type,
}
fn descriptor(dex: &DexFile, index: u32) -> Option<&str> {
    dex.string(dex.type_id(index).ok()?.descriptor_idx).ok()
}
fn source_type(dex: &DexFile, index: u32, r: &impl JavaResolver) -> Option<JavaType> {
    let ty = crate::dex_java::descriptor_type(descriptor(dex, index)?)?;
    match ty {
        JavaType::Class(_) => Some(JavaType::Class(
            r.class(managed_ir::refs::ClassRef::new(index))?,
        )),
        _ => Some(ty),
    }
}
fn reference(ty: &JavaType) -> bool {
    matches!(ty, JavaType::Class(_) | JavaType::Array(_))
}
fn spelling(ty: &JavaType) -> Result<String, RecipeError> {
    Ok(match ty {
        JavaType::Void => "void".into(),
        JavaType::Boolean => "boolean".into(),
        JavaType::Byte => "byte".into(),
        JavaType::Short => "short".into(),
        JavaType::Char => "char".into(),
        JavaType::Int => "int".into(),
        JavaType::Long => "long".into(),
        JavaType::Float => "float".into(),
        JavaType::Double => "double".into(),
        JavaType::Class(name)
            if name.split('.').all(|part| {
                !part.is_empty() && backend_java::sanitize_identifier(part) == part
            }) =>
        {
            name.clone()
        }
        JavaType::Array(element) if **element != JavaType::Void => {
            format!("{}[]", spelling(element)?)
        }
        _ => return Err(RecipeError::Type),
    })
}
fn operand(
    values: &HashMap<ValueId, (String, JavaType)>,
    value: ValueId,
    expected: &JavaType,
    r: &impl JavaResolver,
) -> Result<String, RecipeError> {
    let (source, actual) = values.get(&value).ok_or(RecipeError::Evidence)?;
    if actual == expected {
        return Ok(source.clone());
    }
    if reference(actual) && reference(expected) && r.assignable(actual, expected) {
        return Ok(format!("({}) {source}", spelling(expected)?));
    }
    Err(RecipeError::Type)
}
/// Original segment and compiler constant position, never synthetic provenance.
#[derive(Clone, Copy)]
pub struct RecipeRequest {
    /// Genuine original class initializer pool identity.
    pub clinit_index: u32,
    /// Original enum allocation instruction.
    pub allocation: InsnId,
    /// Exact original payload constructor invocation.
    pub call: InsnId,
    /// Original constant ordinal, checked against its hidden argument.
    pub recipe_number: usize,
}
/// Prove one original linear allocation-to-initializer payload schedule.
///
/// Earlier constant payload values are deliberately unavailable: this helper
/// cannot silently reread a cached field or substitute its identity. The parent
/// still proves the original constructor body, enum family, backing-array and
/// suffix, and records added-helper observability separately before publication.
pub fn plan_recipe<R: RecipeEvidence>(
    dex: &DexFile,
    class: &ClassDef,
    ssa: &dex_decompiler::SsaMethod,
    request: RecipeRequest,
    r: &R,
) -> Result<RecipePlan, RecipeError> {
    let plan = candidate_recipe(dex, class, ssa, request, r)?;
    if !r.recipe_hidden_inputs(ssa, request.allocation, request.call) {
        return Err(RecipeError::Evidence);
    }
    Ok(plan)
}
/// Prepare a representation candidate without approving hidden-input scheduling.
///
/// Compile/runtime checks of this candidate do not attest original equivalence:
/// javac eagerly loads enum name/ordinal before the synthetic helper while the
/// original name literal may occur after payload operations. Production must use
/// `plan_recipe` and supply that additional exact proof before any admission.
pub fn candidate_recipe<R: RecipeEvidence>(
    dex: &DexFile,
    class: &ClassDef,
    ssa: &dex_decompiler::SsaMethod,
    request: RecipeRequest,
    r: &R,
) -> Result<RecipePlan, RecipeError> {
    let RecipeRequest {
        clinit_index,
        allocation,
        call,
        recipe_number,
    } = request;
    if r.recipe_source_release() < 25 || class.access_flags & 0x4000 == 0 {
        return Err(RecipeError::Metadata);
    }
    let data = class.class_data.as_ref().ok_or(RecipeError::Metadata)?;
    let original = data
        .direct_methods
        .iter()
        .find(|m| m.method_idx == clinit_index)
        .ok_or(RecipeError::Metadata)?;
    let raw = dex
        .method_id(clinit_index)
        .map_err(|_| RecipeError::Metadata)?;
    let code = original.code.as_ref().ok_or(RecipeError::Metadata)?;
    if dex.string(raw.name_idx).ok() != Some("<clinit>")
        || u32::from(raw.class_idx) != class.class_idx
        || original.access_flags & 8 == 0
        || !code.tries.is_empty()
        || !code.handlers.is_empty()
    {
        return Err(RecipeError::Flow);
    }
    let ir = ssa.ir();
    let ManagedInsn::NewInstance {
        result: receiver,
        class: allocated,
    } = ir
        .instruction(allocation)
        .map_err(|_| RecipeError::Evidence)?
    else {
        return Err(RecipeError::Evidence);
    };
    if allocated.index() != class.class_idx {
        return Err(RecipeError::Metadata);
    }
    let mut block = ir
        .cfg()
        .block_ids()
        .find(|b| ir.cfg().block(*b).is_ok_and(|n| *n.payload() == allocation))
        .ok_or(RecipeError::Flow)?;
    let mut visited = HashSet::new();
    let mut values = HashMap::<ValueId, (String, JavaType)>::new();
    // Original dominating pure literal definitions may be reused by register
    // allocation across constants; never import an effectful prior value.
    for (_, insn) in ir.instructions() {
        if let ManagedInsn::Const { result, value } = insn {
            let pure = match value {
                Constant::Int(i) => Some((i.to_string(), JavaType::Int)),
                Constant::Zero => Some(("0".into(), JavaType::Int)),
                Constant::Long(i) => Some((format!("{i}L"), JavaType::Long)),
                _ => None,
            };
            if let Some(pair) = pure
                && ssa
                    .offset(ir.value(*result).map_err(|_| RecipeError::Evidence)?.def())
                    .is_some_and(|off| off < ssa.offset(allocation).unwrap_or(0))
                && ir
                    .cfg()
                    .block_ids()
                    .find(|b| {
                        ir.cfg().block(*b).is_ok_and(|n| {
                            *n.payload()
                                == ir
                                    .value(*result)
                                    .ok()
                                    .map(|v| v.def())
                                    .unwrap_or(allocation)
                        })
                    })
                    .is_some_and(|defined| {
                        decomp_core::analysis::dominators(ir.cfg()).dominates(defined, block)
                    })
            {
                values.insert(*result, pair);
            }
        }
    }
    let mut aliases = HashSet::from([*receiver]);
    let mut lines = Vec::new();
    let mut effects = Vec::new();
    let mut previous = ssa.offset(allocation).ok_or(RecipeError::Flow)?;
    let mut next_local = 0usize;
    loop {
        if !visited.insert(block) {
            return Err(RecipeError::Flow);
        }
        let node = ir.cfg().block(block).map_err(|_| RecipeError::Flow)?;
        let id = *node.payload();
        let insn = ir.instruction(id).map_err(|_| RecipeError::Flow)?;
        let offset = ssa.offset(id).ok_or(RecipeError::Flow)?;
        if offset < previous {
            return Err(RecipeError::Flow);
        }
        previous = offset;
        if id == call {
            let ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                method,
                args,
                result: None,
            } = insn
            else {
                return Err(RecipeError::Metadata);
            };
            if args.len() < 3 || !aliases.contains(&args[0]) {
                return Err(RecipeError::Evidence);
            }
            let ctor = data
                .direct_methods
                .iter()
                .find(|m| m.method_idx == method.index())
                .ok_or(RecipeError::Metadata)?;
            if ctor.access_flags & !(2 | 0x1000 | 0x10000) != 0 || ctor.access_flags & 2 == 0 {
                return Err(RecipeError::Metadata);
            }
            let raw = dex
                .method_id(method.index())
                .map_err(|_| RecipeError::Metadata)?;
            let proto = dex
                .proto_id(u32::from(raw.proto_idx))
                .map_err(|_| RecipeError::Metadata)?;
            if u32::from(raw.class_idx) != class.class_idx
                || dex.string(raw.name_idx).ok() != Some("<init>")
                || descriptor(dex, proto.return_type_idx) != Some("V")
            {
                return Err(RecipeError::Metadata);
            }
            let types = proto
                .parameters
                .iter()
                .flat_map(|p| &p.types)
                .map(|t| source_type(dex, u32::from(*t), r))
                .collect::<Option<Vec<_>>>()
                .ok_or(RecipeError::Metadata)?;
            if types.len() + 1 != args.len()
                || !types.starts_with(&[JavaType::Class("java.lang.String".into()), JavaType::Int])
            {
                return Err(RecipeError::Metadata);
            }
            // Hidden parameters must be original constants; helpers must never
            // read or rewrite the compiler-owned name/ordinal inputs.
            let (_, name_ty) = values.get(&args[1]).ok_or(RecipeError::Evidence)?;
            let (ordinal, ordinal_ty) = values.get(&args[2]).ok_or(RecipeError::Evidence)?;
            if *name_ty != JavaType::Class("java.lang.String".into())
                || *ordinal_ty != JavaType::Int
                || ordinal.parse::<usize>().ok() != Some(recipe_number)
            {
                return Err(RecipeError::Evidence);
            }
            let arguments = args[3..]
                .iter()
                .zip(&types[2..])
                .map(|(v, t)| operand(&values, *v, t, r))
                .collect::<Result<Vec<_>, _>>()?;
            let original_method = r.method(*method).ok_or(RecipeError::Metadata)?;
            if original_method.parameters != types
                || original_method.name != "<init>"
                || original_method.is_static
                || original_method.return_type != JavaType::Void
                || !original_method.throws.is_empty()
            {
                return Err(RecipeError::Metadata);
            }
            let existing = data
                .direct_methods
                .iter()
                .filter_map(|m| {
                    let raw = dex.method_id(m.method_idx).ok()?;
                    if dex.string(raw.name_idx).ok()? != "<init>" {
                        return None;
                    }
                    let proto = dex.proto_id(u32::from(raw.proto_idx)).ok()?;
                    Some(
                        proto
                            .parameters
                            .iter()
                            .flat_map(|p| &p.types)
                            .skip(2)
                            .map(|t| descriptor(dex, u32::from(*t)))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>();
            let arity = (1..=64)
                .filter(|n| {
                    !existing
                        .iter()
                        .any(|p| p.len() == *n && p.iter().all(|t| *t == Some("I")))
                })
                .nth(recipe_number)
                .ok_or(RecipeError::Metadata)?;
            let owner = r.class(*allocated).ok_or(RecipeError::Metadata)?;
            if owner != original_method.owner {
                return Err(RecipeError::Metadata);
            }
            let name = owner.rsplit('.').next().ok_or(RecipeError::Metadata)?;
            if backend_java::sanitize_identifier(name) != name {
                return Err(RecipeError::Metadata);
            }
            lines.push(format!("this({});", arguments.join(", ")));
            return Ok(RecipePlan {
                constant_arguments: vec!["0".into(); arity],
                helper_source: format!(
                    "private {name}({}) {{\n{}\n}}",
                    (0..arity)
                        .map(|i| format!("int recipe{i}"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    lines
                        .iter()
                        .map(|line| format!("    {line}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
                original_constructor: *method,
                effects,
                observability_limit: "synthetic constructor inventory and additional stack frame; not reflection/stack-trace equivalent",
            });
        }
        if id != allocation {
            let mut definition = None;
            match insn {
                ManagedInsn::Move { result, value } if aliases.contains(value) => {
                    aliases.insert(*result);
                }
                ManagedInsn::Move { result, value } => {
                    definition = Some((
                        *result,
                        values.get(value).cloned().ok_or(RecipeError::Evidence)?,
                    ));
                }
                ManagedInsn::Const { result, value } => {
                    let pair = match value {
                        Constant::Int(i) => (i.to_string(), JavaType::Int),
                        Constant::Zero => ("0".into(), JavaType::Int),
                        Constant::Long(i) => (format!("{i}L"), JavaType::Long),
                        Constant::String(s) => {
                            let units = r.string(*s).ok_or(RecipeError::Evidence)?;
                            let string =
                                String::from_utf16(&units).map_err(|_| RecipeError::Evidence)?;
                            if !string
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '_')
                            {
                                return Err(RecipeError::Evidence);
                            }
                            (
                                format!("\"{string}\""),
                                JavaType::Class("java.lang.String".into()),
                            )
                        }
                        Constant::ClassLiteral(t) => {
                            if !r.recipe_operation(id, insn) {
                                return Err(RecipeError::Evidence);
                            }
                            let ty = r.instruction_type(*t).ok_or(RecipeError::Evidence)?;
                            effects.push(offset);
                            (
                                format!("{}.class", spelling(&ty)?),
                                JavaType::Class("java.lang.Class".into()),
                            )
                        }
                        _ => return Err(RecipeError::Evidence),
                    };
                    definition = Some((*result, pair));
                }
                ManagedInsn::FieldGet {
                    result,
                    object: None,
                    field,
                } => {
                    if !r.recipe_operation(id, insn) || !r.field_source_nonconstant(*field) {
                        return Err(RecipeError::Evidence);
                    }
                    let f = r.field(*field).ok_or(RecipeError::Evidence)?;
                    let raw = dex
                        .field_id(field.index())
                        .map_err(|_| RecipeError::Metadata)?;
                    let ty = source_type(dex, u32::from(raw.type_idx), r)
                        .ok_or(RecipeError::Metadata)?;
                    if !f.is_static
                        || f.owner
                            != r.class(managed_ir::refs::ClassRef::new(u32::from(raw.class_idx)))
                                .ok_or(RecipeError::Metadata)?
                        || f.ty != ty
                        || backend_java::sanitize_identifier(&f.name) != f.name
                        || dex.string(raw.name_idx).ok() != Some(f.name.as_str())
                    {
                        return Err(RecipeError::Metadata);
                    }
                    effects.push(offset);
                    definition = Some((*result, (format!("{}.{}", f.owner, f.name), f.ty)));
                }
                ManagedInsn::Invoke {
                    kind: InvokeKind::Static,
                    result: Some(result),
                    method,
                    args,
                } => {
                    if !r.recipe_operation(id, insn) {
                        return Err(RecipeError::Evidence);
                    }
                    let m = r.method(*method).ok_or(RecipeError::Evidence)?;
                    let flags = r.method_access(*method).ok_or(RecipeError::Evidence)?;
                    let raw = dex
                        .method_id(method.index())
                        .map_err(|_| RecipeError::Metadata)?;
                    let proto = dex
                        .proto_id(u32::from(raw.proto_idx))
                        .map_err(|_| RecipeError::Metadata)?;
                    let types = proto
                        .parameters
                        .iter()
                        .flat_map(|p| &p.types)
                        .map(|t| source_type(dex, u32::from(*t), r))
                        .collect::<Option<Vec<_>>>()
                        .ok_or(RecipeError::Metadata)?;
                    if flags & (8 | 0x100 | 0x400) != 8
                        || m.owner
                            != r.class(managed_ir::refs::ClassRef::new(u32::from(raw.class_idx)))
                                .ok_or(RecipeError::Metadata)?
                        || !m.is_static
                        || m.parameters != types
                        || m.return_type
                            != source_type(dex, proto.return_type_idx, r)
                                .ok_or(RecipeError::Metadata)?
                        || m.return_type == JavaType::Void
                        || !m.throws.is_empty()
                        || dex.string(raw.name_idx).ok() != Some(m.name.as_str())
                    {
                        return Err(RecipeError::Metadata);
                    }
                    if args.len() != types.len() {
                        return Err(RecipeError::Type);
                    }
                    let args = args
                        .iter()
                        .zip(&types)
                        .map(|(v, t)| operand(&values, *v, t, r))
                        .collect::<Result<Vec<_>, _>>()?;
                    effects.push(offset);
                    definition = Some((
                        *result,
                        (
                            format!("{}.{}({})", m.owner, m.name, args.join(", ")),
                            m.return_type,
                        ),
                    ));
                }
                _ => return Err(RecipeError::Evidence),
            }
            if let Some((result, (expression, ty))) = definition {
                let pure = matches!(
                    insn,
                    ManagedInsn::Const {
                        value: Constant::Int(_)
                            | Constant::Zero
                            | Constant::Long(_)
                            | Constant::String(_),
                        ..
                    } | ManagedInsn::Move { .. }
                );
                let source = if pure {
                    expression
                } else {
                    let local = format!("payload{next_local}");
                    next_local += 1;
                    lines.push(format!("{} {local} = {expression};", spelling(&ty)?));
                    local
                };
                if values.insert(result, (source, ty)).is_some() {
                    return Err(RecipeError::Evidence);
                }
            }
        }
        let [edge] = node.outgoing() else {
            return Err(RecipeError::Flow);
        };
        let edge = ir.cfg().edge(*edge).map_err(|_| RecipeError::Flow)?;
        if edge.kind() != &decomp_core::cfg::EdgeKind::Unconditional {
            return Err(RecipeError::Flow);
        }
        block = edge.target();
        if ir
            .cfg()
            .block(block)
            .map_err(|_| RecipeError::Flow)?
            .incoming()
            .len()
            != 1
        {
            return Err(RecipeError::Flow);
        }
    }
}
