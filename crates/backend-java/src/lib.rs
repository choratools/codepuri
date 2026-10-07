//! Conservative, frontend-independent Java source generation from managed SSA.
//! Every definition is materialized in instruction order; expressions are never
//! duplicated or moved across effects. Unsupported methods produce no source.

use decomp_core::cfg::EdgeKind;
use managed_ir::{
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
    ops::{ArithOp, CompareOp, Constant, InvokeKind, OrderingOp},
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
    types::ManagedType,
    value::{InsnId, ValueId},
};
use std::collections::{HashMap, HashSet};
pub mod branching;
pub mod class_literal;
pub mod dispatch;
pub mod loop_control;
pub use decomp_structure::loop_control as loop_control_regions;
mod exception_emitter;
pub mod exceptions;
use exception_emitter::ExceptionEmitter;
mod arithmetic_operands;
/// Attested checked-exception declaration coverage.
pub mod checked_exceptions;
/// Caller-local checked-source recipes with separately retained runtime facts.
pub mod checked_source_bridge;
/// Evidence for commuting receiver-independent checks over inert parent initialization.
pub mod constructor_prechecks;
pub mod constructors;
pub mod contextual_locals;
mod filled_arrays;
mod invoke_arguments;
mod java_string_literal;
mod managed_ops;
mod object_plan;
mod pure_expression;
pub mod shared_regions;
pub mod special_invokes;
mod static_field_qualifier;
pub mod static_initializer;
pub mod switching;
pub mod type_compatibility;

/// A Java source-level type. Class names use dots, including nested classes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JavaType {
    /// No return value (not valid for parameters or locals).
    Void,
    /// Boolean.
    Boolean,
    /// Signed byte.
    Byte,
    /// Signed short.
    Short,
    /// UTF-16 code unit.
    Char,
    /// Signed 32-bit integer.
    Int,
    /// Signed 64-bit integer.
    Long,
    /// IEEE binary32.
    Float,
    /// IEEE binary64.
    Double,
    /// Fully qualified source class name.
    Class(String),
    /// Array component type.
    Array(Box<JavaType>),
}
impl JavaType {
    fn source(&self) -> Result<String, Unsupported> {
        Ok(match self {
            Self::Void => "void".into(),
            Self::Boolean => "boolean".into(),
            Self::Byte => "byte".into(),
            Self::Short => "short".into(),
            Self::Char => "char".into(),
            Self::Int => "int".into(),
            Self::Long => "long".into(),
            Self::Float => "float".into(),
            Self::Double => "double".into(),
            Self::Class(n) => qualified(n)?,
            Self::Array(t) if **t != Self::Void => format!("{}[]", t.source()?),
            Self::Array(_) => return Err(Unsupported::Metadata("void array".into())),
        })
    }
    fn numeric(&self) -> bool {
        matches!(
            self,
            Self::Byte
                | Self::Short
                | Self::Char
                | Self::Int
                | Self::Long
                | Self::Float
                | Self::Double
        )
    }
    fn integral(&self) -> bool {
        matches!(
            self,
            Self::Byte | Self::Short | Self::Char | Self::Int | Self::Long
        )
    }
    fn reference(&self) -> bool {
        matches!(self, Self::Class(_) | Self::Array(_))
    }
}

/// A source-level method signature, also used for resolved invocation targets.
#[derive(Clone, Debug)]
pub struct JavaMethod {
    /// Declaring class (source-level qualified name).
    pub owner: String,
    /// Source member name. Declaration names are sanitized; target names must be valid.
    pub name: String,
    /// Static dispatch / declaration.
    pub is_static: bool,
    /// Return type.
    pub return_type: JavaType,
    /// Parameter types in descriptor order, excluding the receiver.
    pub parameters: Vec<JavaType>,
    /// Declared checked exceptions as source-level qualified names. Callers
    /// without exception metadata must conservatively reject such targets.
    pub throws: Vec<String>,
}
/// Exact declaration selected by both DEX invoke-super and Java super dispatch.
#[derive(Clone, Debug)]
pub struct SuperInvocation {
    /// Selected source signature, including aliases and declared exceptions.
    pub method: JavaMethod,
    /// Genuine access flags of the selected declaration.
    pub access_flags: u32,
}
/// Prove an exact null throw operand from its SSA definitions, independently of
/// nominal type annotations. Copies and all-null phis retain this evidence;
/// missing, mixed, or cyclic definitions do not.
/// Consumers must still validate control flow, phi selection and availability.
pub fn proven_null_throw(ir: &MethodIr<SsaForm>, value: ValueId) -> bool {
    let mut active = HashSet::new();
    let mut proven = HashSet::new();
    let mut pending = vec![(value, false)];
    while let Some((value, completed)) = pending.pop() {
        if completed {
            active.remove(&value);
            proven.insert(value);
            continue;
        }
        if proven.contains(&value) {
            continue;
        }
        if !active.insert(value) {
            return false;
        }
        let Some(definition) = ir
            .value(value)
            .ok()
            .and_then(|record| ir.instruction(record.def()).ok())
        else {
            return false;
        };
        pending.push((value, true));
        match definition {
            ManagedInsn::Const {
                result,
                value: Constant::Null | Constant::Zero,
            } if *result == value => {}
            ManagedInsn::Move {
                result,
                value: source,
            } if *result == value => {
                pending.push((*source, false));
            }
            ManagedInsn::Phi { result, inputs } if *result == value && !inputs.is_empty() => {
                pending.extend(inputs.iter().map(|(_, input)| (*input, false)));
            }
            _ => return false,
        }
    }
    true
}
/// Resolved field metadata.
#[derive(Clone, Debug)]
pub struct JavaField {
    /// Declaring class.
    pub owner: String,
    /// Exact source field name.
    pub name: String,
    /// Field type.
    pub ty: JavaType,
    /// Static field flag.
    pub is_static: bool,
}
/// Caller-owned metadata; no DEX dependency or pool interpretation is needed.
pub trait JavaResolver {
    /// Exact caller-local bridge plan. Ordinary resolvers provide no capability;
    /// original callee source contracts remain unchanged.
    fn checked_source_bridge(&self, _caller: &JavaMethod) -> Option<&checked_source_bridge::Plan> {
        None
    }
    /// Private whole-body rendering recipe; never a callee/source contract.
    fn checked_source_body_bridge(
        &self,
        _caller: &JavaMethod,
    ) -> Option<&checked_source_bridge::Plan> {
        None
    }

    /// Attest an exact allocation/operand/initializer chain inside one original
    /// exception scope after validating handler-visible prethrow state. Object
    /// planners independently prove expression order, use and initializer identity.
    /// Ordinary resolvers reject; exception renderers may provide a local proof.
    fn exception_allocation_chain(&self, _chain: &[InsnId]) -> bool {
        false
    }
    /// Attest an exact original-order allocation/producer/initializer expression
    /// across original ranges, with equivalent selected handler states. Every
    /// resolving producer keeps its position after NEW; its exception domain
    /// must be positively excluded from every newly enclosing original catch.
    /// This never permits producer hoisting. Ordinary resolvers reject.
    fn exception_allocation_sequence_chain(&self, _chain: &[InsnId]) -> bool {
        false
    }
    /// Attest a pure allocation chain crossing original exception scopes whose
    /// ordered catches, handler-visible values and selected transfers are proved
    /// equivalent. Only primitive constants and moves from available values may
    /// separate allocation and initialization; ordinary resolvers reject.
    fn exception_allocation_pure_chain(&self, _chain: &[InsnId]) -> bool {
        false
    }
    /// Attest a genuine external public final standard-library StringBuilder.
    /// Input definitions, shadowed declarations, and name-only evidence cannot
    /// satisfy this capability. The default rejects fluent intrinsic lowering.
    fn trusted_string_builder_class(&self, _reference: ClassRef) -> bool {
        false
    }
    /// Attest exact direct-parent Java and DEX target identity, before aliases.
    fn java_super_selects_dex_target(&self, _caller: &str, _reference: MethodRef) -> bool {
        false
    }
    /// Bind the proven super target before signature, access, or exception checks.
    /// The legacy capability is sufficient only when the symbolic declaration
    /// already supplies the exact selected target's metadata.
    fn super_invocation(&self, caller: &str, reference: MethodRef) -> Option<SuperInvocation> {
        if !self.java_super_selects_dex_target(caller, reference) {
            return None;
        }
        Some(SuperInvocation {
            method: self.method(reference)?,
            access_flags: self.method_access(reference)?,
        })
    }
    /// Exact direct-interface source qualifier for an already selected super binding.
    fn super_interface_qualifier(&self, _caller: &str, _reference: MethodRef) -> Option<String> {
        None
    }
    /// Attest that retaining this static receiver preserves descriptor dispatch
    /// and Java access. Default is deliberately closed.
    fn retain_method_receiver(
        &self,
        _reference: MethodRef,
        _caller: &str,
        _receiver: &JavaType,
    ) -> bool {
        false
    }
    /// Select a genuine public class declaration through a proven static
    /// receiver. The same proof authorizes retaining that receiver in source.
    /// Runtime virtual dispatch is unchanged. Missing evidence stays closed.
    fn virtual_receiver_method(
        &self,
        _reference: MethodRef,
        _caller: &str,
        _receiver: &JavaType,
    ) -> Option<JavaMethod> {
        None
    }
    /// Genuine field flags, when declaration metadata is available.
    fn field_access(&self, _reference: FieldRef) -> Option<u32> {
        None
    }
    /// Attest a lexical field collision with this static method owner's package root.
    fn static_method_type_context(&self, _reference: MethodRef, _caller: &str) -> bool {
        false
    }
    /// Genuine declaring class flags for the exact resolved method owner.
    /// Missing metadata cannot authorize a static primary expression.
    fn static_method_owner_access(&self, _reference: MethodRef) -> Option<u32> {
        None
    }
    /// Select a type-context qualifier for a genuine static field owner when
    /// a caller's lexical field name shadows its package or class qualifier.
    /// This does not change the field declaration or add a receiver operation.
    fn static_field_type_context(&self, _reference: FieldRef, _caller: &str) -> bool {
        false
    }
    /// Attest that the exact resolved source field is not a Java constant
    /// variable. Reads must remain field accesses with their original declaring
    /// class initialization boundary; final flags alone are insufficient.
    fn field_source_nonconstant(&self, _reference: FieldRef) -> bool {
        false
    }
    /// Attest that retaining the receiver selects the same field declaration.
    fn retain_field_receiver(
        &self,
        _reference: FieldRef,
        _caller: &str,
        _receiver: &JavaType,
    ) -> bool {
        false
    }
    /// Prove that a discarded non-void source call selects the original target,
    /// including its erased return descriptor. Source syntax cannot name that
    /// descriptor, so fabricated or unresolved declarations are insufficient.
    fn discarded_invoke_source_target(&self, _reference: MethodRef) -> bool {
        false
    }
    /// Genuine declaration flags; pool references alone are insufficient.
    fn method_access(&self, _reference: MethodRef) -> Option<u32> {
        None
    }
    /// Proven directed assignment compatibility; missing hierarchy fails closed.
    fn assignable(&self, actual: &JavaType, expected: &JavaType) -> bool {
        type_compatibility::Hierarchy::default().assignable(actual, expected)
            == type_compatibility::Compatibility::Assignable
    }
    /// Resolve an exact instruction type, including frontend numeric targets.
    fn instruction_type(&self, _reference: TypeRef) -> Option<JavaType> {
        None
    }
    /// Resolve a proven JVM array-clone intrinsic. Default resolvers reject.
    fn array_clone_method(
        &self,
        _reference: MethodRef,
        _receiver: &JavaType,
    ) -> Option<JavaMethod> {
        None
    }
    /// Resolve a method's descriptor and name.
    fn method(&self, reference: MethodRef) -> Option<JavaMethod>;
    /// Resolve a field's descriptor and name.
    fn field(&self, reference: FieldRef) -> Option<JavaField>;
    /// Resolve a class to its source-level qualified name.
    fn class(&self, reference: ClassRef) -> Option<String>;
    /// Resolve a string literal to UTF-16 code units (including unpaired surrogates).
    fn string(&self, reference: StringRef) -> Option<Vec<u16>>;
}
/// Explicit mapping of SSA values to descriptor parameters and receiver.
#[derive(Clone, Debug, Default)]
pub struct ParameterBindings {
    /// Receiver value for an instance method; must be absent for a static method.
    pub receiver: Option<ValueId>,
    /// Exactly one value per descriptor parameter, not per register word.
    pub parameters: Vec<ValueId>,
    /// Optional source hints; empty means `p0`, `p1`, etc.
    pub names: Vec<String>,
}
/// Descriptor positions may have no SSA identity when their input is unused.
#[derive(Clone, Debug, Default)]
pub struct DeclarationBindings {
    /// Proven receiver identity, if read by the body.
    pub receiver: Option<ValueId>,
    /// One position per declared parameter; `None` preserves an unused input.
    pub parameters: Vec<Option<ValueId>>,
    /// Optional source hints in descriptor order.
    pub names: Vec<String>,
}
/// Typed reason why a method cannot be faithfully emitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// Invalid/missing metadata or parameter bindings.
    Metadata(String),
    /// Unsupported managed instruction (includes opaque, phi and branch).
    Instruction(String),
    /// Unsupported graph, including exceptional edges and non-linear control flow.
    ControlFlow(String),
    /// Missing definition or incompatible value types.
    Type(String),
}
/// A successful result is a complete method declaration, never a stub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceResult {
    /// Faithfully emitted supported method; insert into the declaring class.
    Complete {
        /// Complete Java method declaration.
        source: String,
        /// Sanitized declaration name.
        method_name: String,
        /// Sanitized, unique parameter names in descriptor order.
        parameter_names: Vec<String>,
    },
    /// No source emitted. A parent may generate a separately labeled fallback.
    Unsupported(Unsupported),
}
/// Emit a public Java method from a linear CFG. Synthetic parameter definitions
/// may be outside the CFG: explicit bindings supply their values and types.
pub fn emit_method(
    ir: &MethodIr<SsaForm>,
    method: &JavaMethod,
    bindings: &ParameterBindings,
    resolver: &impl JavaResolver,
) -> SourceResult {
    emit_method_declaration(
        ir,
        method,
        &DeclarationBindings {
            receiver: bindings.receiver,
            parameters: bindings.parameters.iter().copied().map(Some).collect(),
            names: bindings.names.clone(),
        },
        resolver,
    )
}
/// Emit a complete method while retaining unused descriptor positions.
pub fn emit_method_declaration(
    ir: &MethodIr<SsaForm>,
    method: &JavaMethod,
    bindings: &DeclarationBindings,
    resolver: &impl JavaResolver,
) -> SourceResult {
    emit_method_declaration_with_exceptions(ir, method, bindings, resolver, None)
}

/// Emit with exact frontend-provided exception scopes and caught types.
pub fn emit_method_declaration_with_exceptions(
    ir: &MethodIr<SsaForm>,
    method: &JavaMethod,
    bindings: &DeclarationBindings,
    resolver: &impl JavaResolver,
    scopes: Option<&dyn exceptions::ExceptionMetadata>,
) -> SourceResult {
    match emit(ir, method, bindings, resolver, scopes) {
        Ok((source, method_name, parameter_names)) => SourceResult::Complete {
            source,
            method_name,
            parameter_names,
        },
        Err(e) => SourceResult::Unsupported(e),
    }
}

struct Emitter<'a, R> {
    allocation_readable: Option<std::sync::Arc<HashMap<ValueId, HashSet<InsnId>>>>,
    object_planning: std::rc::Rc<object_plan::ObjectPlanning>,
    ir: &'a MethodIr<SsaForm>,
    resolver: &'a R,
    values: HashMap<ValueId, (String, JavaType)>,
    lines: Vec<String>,
    next: usize,
    parameters: HashSet<ValueId>,
    reserved: HashSet<String>,
    current: Option<InsnId>,
    hoisted: HashSet<ValueId>,
    declarations: HashMap<ValueId, JavaType>,
}
impl<R: JavaResolver> Emitter<'_, R> {
    // Unlike literal substitution, this preserves an argument capture assignment
    // or another already-planned producer expression exactly once.
    fn category_one_materialized_expression(
        &self,
        value: ValueId,
        expression: &str,
        actual: &JavaType,
        expected: &JavaType,
    ) -> Option<String> {
        if !matches!(
            (actual, expected),
            (JavaType::Boolean, JavaType::Int) | (JavaType::Int, JavaType::Boolean)
        ) {
            return None;
        }
        contextual_locals::category_one_expression_with_parameters(
            self.ir,
            value,
            actual,
            expected,
            self.resolver,
            &|initial| {
                self.parameters.contains(&initial)
                    && self
                        .values
                        .get(&initial)
                        .is_some_and(|(_, ty)| *ty == JavaType::Boolean)
            },
        )?;
        Some(if *expected == JavaType::Boolean {
            format!("(({expression}) != 0)")
        } else {
            format!("(({expression}) ? 1 : 0)")
        })
    }
    fn category_one_bits(&self, value: ValueId) -> bool {
        contextual_locals::boolean_bits_with_parameters(self.ir, value, self.resolver, &|initial| {
            self.parameters.contains(&initial)
                && self
                    .values
                    .get(&initial)
                    .is_some_and(|(_, ty)| *ty == JavaType::Boolean)
        })
    }
    fn operand(
        &self,
        value: ValueId,
        expected: &JavaType,
    ) -> Result<(String, JavaType), Unsupported> {
        use type_compatibility::{Coercion, Hierarchy, ValueType};
        // Definition evidence never substitutes for path availability.
        // Branch/exception planners establish availability; linear emission
        // additionally requires materialization before replacing a pure literal.
        let materialized = self.get(value)?;
        if contextual_locals::category_one_narrowing(self.ir, value, &materialized.1, expected) {
            return Ok((
                format!("(({}) {})", expected.source()?, materialized.0),
                expected.clone(),
            ));
        }
        if type_compatibility::narrow_register_widening(&materialized.1, expected) {
            // Pin the descriptor type so Java overload selection cannot retain
            // a narrower field/array expression's static type.
            return Ok((format!("((int) {})", materialized.0), JavaType::Int));
        }
        if &materialized.1 != expected
            && let Some(coercion) = contextual_locals::category_one_expression_with_parameters(
                self.ir,
                value,
                &materialized.1,
                expected,
                self.resolver,
                &|initial| {
                    self.parameters.contains(&initial)
                        && self
                            .values
                            .get(&initial)
                            .is_some_and(|(_, ty)| *ty == JavaType::Boolean)
                },
            )
        {
            let expression = match coercion {
                contextual_locals::CategoryOneCoercion::Literal(literal) => literal.to_owned(),
                contextual_locals::CategoryOneCoercion::BooleanToInt => {
                    format!("({} ? 1 : 0)", materialized.0)
                }
                contextual_locals::CategoryOneCoercion::IntBitsToBoolean => {
                    format!("({} != 0)", materialized.0)
                }
            };
            return Ok((expression, expected.clone()));
        }
        if matches!(
            contextual_locals::evidence(self.ir, value, self.resolver),
            Ok(ValueType::RawZero | ValueType::Null)
        ) {
            if let Ok((name, ty)) = self.get(value) {
                if &ty == expected {
                    return Ok((name, ty));
                }
                if matches!(expected, JavaType::Byte | JavaType::Short) {
                    return Err(Unsupported::Type(
                        "contextual narrow operand requires an exact typed local".into(),
                    ));
                }
            }
            let coercion = contextual_locals::expression(
                self.ir,
                value,
                expected,
                &Hierarchy::default(),
                self.resolver,
            )
            .map_err(|e| Unsupported::Type(format!("contextual operand: {e:?}")))?;
            let literal = match coercion {
                Coercion::Identity => return self.get(value),
                _ => constant(
                    if expected.reference() {
                        Constant::Null
                    } else {
                        Constant::Zero
                    },
                    expected,
                    self.resolver,
                )?,
            };
            // A null literal has Java's null type, not `expected`. Preserve the
            // descriptor type at the expression itself: otherwise overload
            // resolution can pick a narrower overload (or become ambiguous).
            let literal = if expected.reference() {
                format!("(({}) {literal})", expected.source()?)
            } else {
                literal
            };
            return Ok((literal, expected.clone()));
        }
        Ok(materialized)
    }
    fn compatible(&self, actual: &JavaType, expected: &JavaType) -> Result<(), Unsupported> {
        if self.resolver.assignable(actual, expected) {
            Ok(())
        } else {
            Err(Unsupported::Type(format!(
                "assignment {actual:?} to {expected:?}"
            )))
        }
    }
    fn get(&self, v: ValueId) -> Result<(String, JavaType), Unsupported> {
        if self.pending_allocation(v) {
            return Err(Unsupported::Type(format!("uninitialized allocation {v:?}")));
        }
        self.values
            .get(&v)
            .cloned()
            .ok_or_else(|| Unsupported::Type(format!("undefined value {v:?}")))
    }
    fn ty(&self, v: ValueId) -> Result<JavaType, Unsupported> {
        if let Some(ty) = self.declarations.get(&v) {
            return Ok(ty.clone());
        }
        let value = self
            .ir
            .value(v)
            .map_err(|e| Unsupported::Type(e.to_string()))?;
        if matches!(value.ty(), ManagedType::Unknown)
            && matches!(
                self.ir.instruction(value.def()),
                Ok(ManagedInsn::Const {
                    value: Constant::ClassLiteral(_),
                    ..
                })
            )
        {
            return Ok(JavaType::Class("java.lang.Class".into()));
        }
        managed_type(value.ty(), self.resolver)
    }
    fn define(&mut self, v: ValueId, expression: String, ty: JavaType) -> Result<(), Unsupported> {
        self.define_local(v, expression, ty, false)
    }
    fn define_local(
        &mut self,
        v: ValueId,
        expression: String,
        ty: JavaType,
        contextual_constant: bool,
    ) -> Result<(), Unsupported> {
        if self.values.contains_key(&v) && !self.hoisted.contains(&v) {
            return Err(Unsupported::Type("duplicate definition".into()));
        }
        if self
            .ir
            .value(v)
            .map_err(|e| Unsupported::Type(e.to_string()))?
            .def()
            != self
                .current
                .ok_or_else(|| Unsupported::Type("missing instruction identity".into()))?
        {
            return Err(Unsupported::Type("definition identity mismatch".into()));
        }
        let contextual = contextual_constant
            && matches!(
                self.ir
                    .value(v)
                    .map_err(|e| Unsupported::Type(e.to_string()))?
                    .ty(),
                ManagedType::Null | ManagedType::Zero
            )
            && matches!(
                self.ir.instruction(
                    self.ir
                        .value(v)
                        .map_err(|e| Unsupported::Type(e.to_string()))?
                        .def()
                ),
                Ok(ManagedInsn::Const {
                    value: Constant::Null | Constant::Zero,
                    ..
                })
            );
        let mut expression = expression;
        let ty = if contextual {
            ty
        } else {
            let declared = self.ty(v)?;
            if declared != ty
                && !(matches!(&ty, JavaType::Class(_) | JavaType::Array(_))
                    && matches!(&declared, JavaType::Class(_) | JavaType::Array(_))
                    && self.resolver.assignable(&ty, &declared))
            {
                if let Some(converted) =
                    self.category_one_materialized_expression(v, &expression, &ty, &declared)
                {
                    expression = converted;
                } else {
                    return Err(Unsupported::Type(format!("result type mismatch {v:?}")));
                }
            }
            // Keep the declared static type: widening must not change later
            // receiver selection or the reserved slot's source declaration.
            declared
        };
        if self.hoisted.contains(&v) {
            // A definition writes its reserved slot; it does not read the old
            // value (which can be an uninitialized allocation placeholder).
            let (name, allocated_type) = self
                .values
                .get(&v)
                .cloned()
                .ok_or_else(|| Unsupported::Type(format!("missing hoisted slot {v:?}")))?;
            if matches!(&ty, JavaType::Class(_) | JavaType::Array(_))
                && matches!(&allocated_type, JavaType::Class(_) | JavaType::Array(_))
            {
                self.compatible(&ty, &allocated_type)?;
            } else {
                compatible(&ty, &allocated_type)?;
            }
            self.lines.push(format!("{name} = {expression};"));
            // Successful definitions include copies of already initialized
            // objects. A reserved copy slot is no longer a pending allocation.
            if self.allocation_root(v).is_some() {
                self.hoisted.remove(&v);
            }
            return Ok(());
        }
        if self.is_pure_expression(v) {
            self.values.insert(v, (expression, ty));
            return Ok(());
        }
        let mut name = format!("v{}", self.next);
        self.next += 1;
        while self.reserved.contains(&name) {
            name = format!("v{}", self.next);
            self.next += 1;
        }
        self.lines
            .push(format!("{} {name} = {expression};", ty.source()?));
        self.values.insert(v, (name, ty));
        Ok(())
    }
    fn instruction(
        &mut self,
        insn: &ManagedInsn,
        signature: &JavaMethod,
    ) -> Result<bool, Unsupported> {
        if self.object_instruction(insn, signature)? {
            return Ok(false);
        }
        match insn {
            ManagedInsn::NewFilledArray {
                result,
                target,
                elements,
            } => self.filled_array(*result, *target, elements)?,
            ManagedInsn::NewArray { .. }
            | ManagedInsn::ArrayGet { .. }
            | ManagedInsn::ArrayPut { .. }
            | ManagedInsn::ArrayLength { .. }
            | ManagedInsn::FillArray { .. }
            | ManagedInsn::Cast { .. }
            | ManagedInsn::InstanceOf { .. }
            | ManagedInsn::NewInstance { .. } => self.managed_operation(insn, signature)?,
            ManagedInsn::Const { result, value } => {
                if self.filled_array_null_constant(*result, signature)? {
                    return Ok(false);
                }
                if self.array_payload_constant(*result, *value, signature)? {
                    return Ok(false);
                }
                let ty = self.ty(*result)?;
                let expression = constant(*value, &ty, self.resolver)?;
                self.define(*result, expression, ty)?;
            }
            ManagedInsn::Move { result, value } => {
                if let Some(declared) = self.declarations.get(result).cloned() {
                    let (name, actual) = self.operand(*value, &declared)?;
                    self.compatible(&actual, &declared)?;
                    self.define(*result, name, declared)?;
                    return Ok(false);
                }
                let (n, t) = self.get(*value)?;
                self.define(*result, n, t)?;
            }
            ManagedInsn::Arithmetic {
                op,
                result,
                lhs,
                rhs,
            } => {
                let t = self.ty(*result)?;
                let (l, lt) = self.operand(*lhs, &t)?;
                let right_expected = if matches!(
                    op,
                    ArithOp::ShiftLeft | ArithOp::ShiftRight | ArithOp::UnsignedShiftRight
                ) {
                    JavaType::Int
                } else {
                    t.clone()
                };
                let (r, rt) = self.operand(*rhs, &right_expected)?;
                arithmetic_operands::validate(*op, &t, &lt, &rt)?;
                let symbol = match op {
                    ArithOp::Add => "+",
                    ArithOp::Sub => "-",
                    ArithOp::Mul => "*",
                    ArithOp::Div => "/",
                    ArithOp::Rem => "%",
                    ArithOp::And => "&",
                    ArithOp::Or => "|",
                    ArithOp::Xor => "^",
                    ArithOp::ShiftLeft => "<<",
                    ArithOp::ShiftRight => ">>",
                    ArithOp::UnsignedShiftRight => ">>>",
                };
                // Narrow arithmetic has ambiguous managed semantics: don't guess truncation.
                if matches!(t, JavaType::Byte | JavaType::Short | JavaType::Char) {
                    return Err(Unsupported::Type("narrow arithmetic".into()));
                }
                self.define(*result, format!("({l} {symbol} {r})"), t)?;
            }
            ManagedInsn::OrderingCompare {
                op,
                result,
                lhs,
                rhs,
            } => {
                let (left, left_type) = self.get(*lhs)?;
                let (right, right_type) = self.get(*rhs)?;
                let expected = match op {
                    OrderingOp::CmpLowFloat | OrderingOp::CmpHighFloat => JavaType::Float,
                    OrderingOp::CmpLowDouble | OrderingOp::CmpHighDouble => JavaType::Double,
                    OrderingOp::CmpLong => JavaType::Long,
                };
                if left_type != expected
                    || right_type != expected
                    || self.ty(*result)? != JavaType::Int
                {
                    return Err(Unsupported::Type("ordering comparison types".into()));
                }
                // Dalvik treats both signed zeros as equal. The fallback branch
                // encodes cmpl/cmpg NaN polarity; Float.compare would differ.
                let expression =
                    if matches!(op, OrderingOp::CmpHighFloat | OrderingOp::CmpHighDouble) {
                        format!("({left} < {right} ? -1 : ({left} == {right} ? 0 : 1))")
                    } else {
                        format!("({left} > {right} ? 1 : ({left} == {right} ? 0 : -1))")
                    };
                self.define(*result, expression, JavaType::Int)?;
            }
            ManagedInsn::Compare {
                op,
                result,
                lhs,
                rhs,
            } => {
                let predicate = {
                    let planned = self.pure_expression_values();
                    planned
                        .contains(result)
                        .then(|| {
                            pure_expression::long_ordering_predicate(
                                self.ir,
                                planned,
                                *op,
                                *lhs,
                                *rhs,
                                |value| self.get(value).ok(),
                            )
                        })
                        .flatten()
                };
                if let Some(expression) = predicate {
                    self.define(*result, expression, self.ty(*result)?)?;
                    return Ok(false);
                }
                let (_, left_type) = self.get(*lhs)?;
                let (_, right_type) = self.get(*rhs)?;
                // A DEX zero is interpreted at the comparison use, not by the
                // declaration chosen for another consumer of the same constant.
                let comparison_context = |ty: &JavaType| match ty {
                    JavaType::Byte | JavaType::Short | JavaType::Char => JavaType::Int,
                    _ => ty.clone(),
                };
                let (mut l, lt) = self.operand(*lhs, &comparison_context(&right_type))?;
                let (mut r, rt) = self.operand(*rhs, &comparison_context(&left_type))?;
                let eq = matches!(op, CompareOp::Equal | CompareOp::NotEqual);
                let category_one_int = |ty: &JavaType| {
                    matches!(
                        ty,
                        JavaType::Byte | JavaType::Short | JavaType::Char | JavaType::Int
                    )
                };
                if lt == JavaType::Boolean && category_one_int(&rt) {
                    // Dalvik Boolean values occupy category-one integer slots.
                    // Preserve their exact 0/1 representation without relying
                    // on Java's forbidden boolean/integer comparison.
                    l = format!("({l} ? 1 : 0)");
                } else if rt == JavaType::Boolean && category_one_int(&lt) {
                    r = format!("({r} ? 1 : 0)");
                } else if eq && lt.reference() && rt.reference() {
                    // Reference equality is identity, regardless of nominal
                    // ancestry. Object casts permit Java-incomparable sibling
                    // types without changing either operand or evaluation order.
                    l = format!("((java.lang.Object) {l})");
                    r = format!("((java.lang.Object) {r})");
                } else if !((lt == rt && (lt.numeric() || eq && lt == JavaType::Boolean))
                    || category_one_int(&lt) && category_one_int(&rt))
                {
                    return Err(Unsupported::Type(format!(
                        "comparison types: {lt:?} {op:?} {rt:?}"
                    )));
                }
                let symbol = match op {
                    CompareOp::Equal => "==",
                    CompareOp::NotEqual => "!=",
                    CompareOp::LessThan => "<",
                    CompareOp::LessEqual => "<=",
                    CompareOp::GreaterThan => ">",
                    CompareOp::GreaterEqual => ">=",
                };
                self.define(*result, format!("({l} {symbol} {r})"), JavaType::Boolean)?;
            }
            ManagedInsn::Neg { result, value } | ManagedInsn::Not { result, value } => {
                let (n, t) = self.get(*value)?;
                let neg = matches!(insn, ManagedInsn::Neg { .. });
                if (neg
                    && !matches!(
                        t,
                        JavaType::Int | JavaType::Long | JavaType::Float | JavaType::Double
                    ))
                    || (!neg && !matches!(t, JavaType::Int | JavaType::Long | JavaType::Boolean))
                {
                    return Err(Unsupported::Type("unary types".into()));
                }
                let op = if neg {
                    "-"
                } else if t == JavaType::Boolean {
                    "!"
                } else {
                    "~"
                };
                self.define(*result, format!("({op}{n})"), t)?;
            }
            ManagedInsn::Invoke {
                kind,
                result,
                method,
                args,
            } => {
                let array_clone_receiver = if *kind == InvokeKind::Virtual && args.len() == 1 {
                    let (name, ty) = self.get(args[0])?;
                    self.resolver
                        .array_clone_method(*method, &ty)
                        .map(|m| (m, name, ty))
                } else {
                    None
                };
                let receiver_method = if *kind == InvokeKind::Virtual && !args.is_empty() {
                    let (_, ty) = self.get(args[0])?;
                    self.resolver
                        .virtual_receiver_method(*method, &signature.owner, &ty)
                } else {
                    None
                };
                let (m, super_flags) = if let Some(m) = &receiver_method {
                    (m.clone(), None)
                } else if let Some((m, _, _)) = &array_clone_receiver {
                    (m.clone(), None)
                } else if *kind == InvokeKind::Super {
                    let binding = self
                        .resolver
                        .super_invocation(&signature.owner, *method)
                        .ok_or_else(|| {
                            Unsupported::Metadata(
                                "exact super invocation binding unavailable".into(),
                            )
                        })?;
                    (binding.method, Some(binding.access_flags))
                } else {
                    (
                        self.resolver
                            .method(*method)
                            .ok_or_else(|| Unsupported::Metadata("unresolved method".into()))?,
                        None,
                    )
                };
                if let Some(recipe) = self.resolver.checked_source_body_bridge(signature)
                    && !self
                        .current
                        .is_some_and(|id| recipe.approves_rendering_invoke(id, insn, &m, signature))
                {
                    return Err(Unsupported::Metadata(
                        "whole-body selected source target changed".into(),
                    ));
                }
                if *kind == InvokeKind::Special && m.name == "<init>" {
                    self.completed_allocation(*result, *method, args)?;
                    return Ok(false);
                }
                let static_call = *kind == InvokeKind::Static;
                if matches!(kind, InvokeKind::Virtual | InvokeKind::Interface)
                    && self
                        .resolver
                        .method_access(*method)
                        .is_some_and(|flags| flags & 2 != 0)
                {
                    return Err(Unsupported::Metadata(
                        "private declaration requires direct invocation".into(),
                    ));
                }
                let checked_bridge =
                    if checked_exceptions::check(self.resolver, &m.throws, &signature.throws)
                        .is_err()
                    {
                        self.current
                            .and_then(|id| {
                                self.resolver
                                    .checked_source_bridge(signature)
                                    .filter(|plan| plan.approves_invoke(id, insn, &m, signature))
                            })
                            .cloned()
                    } else {
                        None
                    };
                if checked_exceptions::check(self.resolver, &m.throws, &signature.throws).is_err()
                    && checked_bridge.is_none()
                {
                    return Err(Unsupported::Metadata(
                        "invoke checked exceptions are not declared by caller".into(),
                    ));
                }
                for name in &m.throws {
                    qualified(name)?;
                }
                let dispatch = if matches!(kind, InvokeKind::Special | InvokeKind::Super) {
                    let super_proof = special_invokes::SuperProof(
                        super_flags.is_some()
                            || self
                                .resolver
                                .java_super_selects_dex_target(&signature.owner, *method),
                    );
                    let flags = super_flags
                        .or_else(|| self.resolver.method_access(*method))
                        .ok_or_else(|| {
                            Unsupported::Metadata(
                                "special dispatch declaration flags unavailable".into(),
                            )
                        })?;
                    let plan = special_invokes::DispatchContext {
                        caller: &signature.owner,
                        declaration_owner: &m.owner,
                        declaration_name: &m.name,
                        access_flags: flags,
                        receiver_is_this: args
                            .first()
                            .and_then(|v| self.values.get(v))
                            .is_some_and(|(name, _)| name == "this"),
                        super_resolver: Some(&super_proof),
                    }
                    .plan(*kind)
                    .map_err(|e| Unsupported::Instruction(format!("special dispatch: {e:?}")))?;
                    Some(plan)
                } else {
                    None
                };
                if !matches!(
                    dispatch,
                    Some(
                        special_invokes::InvocationPlan::DirectPrivate
                            | special_invokes::InvocationPlan::Super
                    )
                ) && !matches!(
                    kind,
                    InvokeKind::Static | InvokeKind::Virtual | InvokeKind::Interface
                ) || static_call != m.is_static
                {
                    return Err(Unsupported::Instruction("invoke dispatch".into()));
                }
                member(&m.name)?;
                let offset = usize::from(!static_call);
                let args = invoke_arguments::operands(&m, args)?;
                let target = if let Some((_, receiver, receiver_ty)) = &array_clone_receiver {
                    if !matches!(receiver_ty, JavaType::Array(_)) {
                        return Err(Unsupported::Type("array clone receiver".into()));
                    }
                    receiver.clone()
                } else if dispatch == Some(special_invokes::InvocationPlan::Super) {
                    match self
                        .resolver
                        .super_interface_qualifier(&signature.owner, *method)
                    {
                        Some(interface) => format!("{}.super", qualified(&interface)?),
                        None => "super".to_owned(),
                    }
                } else if static_call {
                    static_method_qualifier::resolved_owner(
                        &m,
                        *method,
                        &signature.owner,
                        self.resolver,
                    )?
                } else {
                    let (n, t) = self.get(args[0])?;
                    let owner = JavaType::Class(m.owner.clone());
                    self.compatible(&t, &owner)?;
                    if receiver_method.is_some()
                        || t == owner
                        || self
                            .resolver
                            .retain_method_receiver(*method, &signature.owner, &t)
                    {
                        n
                    } else {
                        if self
                            .resolver
                            .method_access(*method)
                            .is_some_and(|f| f & 4 != 0)
                            && signature.owner.rsplit_once('.').map(|x| x.0)
                                != m.owner.rsplit_once('.').map(|x| x.0)
                        {
                            return Err(Unsupported::Metadata(
                                "protected receiver selection proof unavailable".into(),
                            ));
                        }
                        format!("(({}) {n})", owner.source()?)
                    }
                };
                let mut actual = Vec::new();
                for (v, t) in args[offset..].iter().zip(&m.parameters) {
                    let (n, vt) = self.operand(*v, t)?;
                    self.compatible(&vt, t)?;
                    // Pin descriptor overload selection when widening a reference.
                    actual.push(if vt == *t {
                        n
                    } else {
                        format!("({}) {n}", t.source()?)
                    });
                }
                let expr = format!("{target}.{}({})", m.name, actual.join(", "));
                let line_start = self.lines.len();
                let was_hoisted = result.is_some_and(|v| self.hoisted.contains(&v));
                if let Some(v) = result {
                    if m.return_type == JavaType::Void {
                        return Err(Unsupported::Type("void invoke result".into()));
                    }
                    self.define(*v, expr, m.return_type)?;
                } else {
                    self.lines.push(format!("{expr};"));
                }
                if let Some(plan) = checked_bridge {
                    let mut emitted = self.lines.split_off(line_start);
                    let declaration = if let Some(value) = result.filter(|_| !was_hoisted) {
                        let (name, ty) = self.get(value)?;
                        let declaration = format!("{} {name}", ty.source()?);
                        if emitted.len() != 1
                            || !emitted[0].starts_with(&format!("{declaration} = "))
                        {
                            return Err(Unsupported::Metadata(
                                "checked bridge result materialization".into(),
                            ));
                        }
                        emitted[0] = emitted[0].replacen(
                            &format!("{declaration} = "),
                            &format!("{name} = "),
                            1,
                        );
                        Some(format!("{declaration};"))
                    } else {
                        None
                    };
                    let mut n = 0usize;
                    let catch_name = loop {
                        let name = format!("checkedFailure{n}");
                        if !self.reserved.contains(&name)
                            && !self.values.values().any(|(value, _)| *value == name)
                        {
                            break name;
                        }
                        n += 1;
                    };
                    self.reserved.insert(catch_name.clone());
                    self.lines.extend(plan.wrap_lines(
                        declaration.as_deref(),
                        &emitted,
                        &catch_name,
                    )?);
                }
            }
            ManagedInsn::FieldGet {
                result,
                object,
                field,
            } => {
                let f = self
                    .resolver
                    .field(*field)
                    .ok_or_else(|| Unsupported::Metadata("unresolved field".into()))?;
                let expr = self.field_target(*object, &f, *field, &signature.owner)?;
                self.define(*result, expr, f.ty)?;
            }
            ManagedInsn::FieldPut {
                object,
                field,
                value,
            } => {
                let f = self
                    .resolver
                    .field(*field)
                    .ok_or_else(|| Unsupported::Metadata("unresolved field".into()))?;
                let target = self.field_target(*object, &f, *field, &signature.owner)?;
                let (n, t) = self.operand(*value, &f.ty)?;
                self.compatible(&t, &f.ty)?;
                self.lines.push(format!("{target} = {n};"));
            }
            ManagedInsn::Throw { value } => {
                // Keep the ordinary availability check even when replacing an
                // exact null value with its equivalent Java literal.
                self.get(*value)?;
                if proven_null_throw(self.ir, *value) {
                    self.lines.push("throw null;".into());
                    return Ok(true);
                }
                let throwable = JavaType::Class("java.lang.Throwable".into());
                let (name, ty) = self.operand(*value, &throwable)?;
                self.compatible(&ty, &throwable)?;
                let unchecked = self
                    .resolver
                    .assignable(&ty, &JavaType::Class("java.lang.RuntimeException".into()))
                    || self
                        .resolver
                        .assignable(&ty, &JavaType::Class("java.lang.Error".into()));
                let declared = signature.throws.iter().any(|target| {
                    self.resolver
                        .assignable(&ty, &JavaType::Class(target.clone()))
                });
                if !unchecked && !declared {
                    if let Some(plan) = self.current.and_then(|id| {
                        self.resolver
                            .checked_source_bridge(signature)
                            .filter(|plan| plan.approves_throw(id, *value, &ty, signature))
                    }) {
                        self.lines.push(plan.rethrow(&name)?);
                        return Ok(true);
                    }
                    return Err(Unsupported::Metadata(
                        "throw checked exception is not declared by caller".into(),
                    ));
                }
                self.lines.push(format!("throw {name};"));
                return Ok(true);
            }
            ManagedInsn::Return { value } => {
                match value {
                    Some(v) => {
                        let (n, t) = self.operand(*v, &signature.return_type)?;
                        self.compatible(&t, &signature.return_type)?;
                        self.lines.push(format!("return {n};"));
                    }
                    None if signature.return_type == JavaType::Void => {
                        self.lines.push("return;".into())
                    }
                    None => return Err(Unsupported::Type("missing return value".into())),
                }
                return Ok(true);
            }
            // Caller binding is the authority for synthetic parameter definitions.
            ManagedInsn::Opaque {
                reads,
                writes,
                effects,
                ..
            } if reads.is_empty()
                && !writes.is_empty()
                && *effects == managed_ir::effects::EffectFlags::NONE
                && writes.iter().all(|v| self.parameters.contains(v)) => {}
            _ => return Err(Unsupported::Instruction(format!("{insn:?}"))),
        }
        Ok(false)
    }
    fn field_target(
        &self,
        object: Option<ValueId>,
        f: &JavaField,
        reference: FieldRef,
        caller: &str,
    ) -> Result<String, Unsupported> {
        member(&f.name)?;
        if f.ty == JavaType::Void {
            return Err(Unsupported::Metadata("void field".into()));
        }
        let target = match object {
            None if f.is_static => static_field_qualifier::owner(
                f,
                self.resolver.static_field_type_context(reference, caller),
            )?,
            Some(v) if !f.is_static => {
                let (n, t) = self.get(v)?;
                let owner = JavaType::Class(f.owner.clone());
                self.compatible(&t, &owner)?;
                if t == owner || self.resolver.retain_field_receiver(reference, caller, &t) {
                    n
                } else {
                    if self
                        .resolver
                        .field_access(reference)
                        .is_some_and(|f| f & 4 != 0)
                        && caller.rsplit_once('.').map(|x| x.0)
                            != f.owner.rsplit_once('.').map(|x| x.0)
                    {
                        return Err(Unsupported::Metadata(
                            "protected field receiver selection proof unavailable".into(),
                        ));
                    }
                    format!("(({}) {n})", owner.source()?)
                }
            }
            _ => return Err(Unsupported::Metadata("field dispatch".into())),
        };
        Ok(format!("{target}.{}", f.name))
    }
}

fn emit(
    ir: &MethodIr<SsaForm>,
    m: &JavaMethod,
    b: &DeclarationBindings,
    r: &impl JavaResolver,
    scopes: Option<&dyn exceptions::ExceptionMetadata>,
) -> Result<(String, String, Vec<String>), Unsupported> {
    qualified(&m.owner)?;
    if m.name.starts_with('<') {
        return Err(Unsupported::Metadata(
            "constructors/class initializers require dedicated lowering".into(),
        ));
    }
    if b.parameters.len() != m.parameters.len()
        || (!b.names.is_empty() && b.names.len() != b.parameters.len())
        || (m.is_static && b.receiver.is_some())
    {
        return Err(Unsupported::Metadata(
            "parameter binding/signature mismatch".into(),
        ));
    }
    // Java expression names can shadow package/class qualifiers. Reserve all
    // roots used by metadata, including the helpers used for bit literals.
    let mut reserved = HashSet::from(["java".to_string()]);
    reserve_class(&mut reserved, &m.owner);
    reserve_type(&mut reserved, &m.return_type);
    for t in &m.parameters {
        reserve_type(&mut reserved, t);
    }
    for (_, v) in ir.values() {
        if let Ok(t) = managed_type(v.ty(), r) {
            reserve_type(&mut reserved, &t);
        }
    }
    for (_, insn) in ir.instructions() {
        match insn {
            ManagedInsn::Cast { target, .. } | ManagedInsn::InstanceOf { target, .. } => {
                if let Some(t) = r.instruction_type(*target) {
                    reserve_type(&mut reserved, &t);
                }
            }
            ManagedInsn::Invoke { method, .. } => {
                if let Some(target) = r.method(*method) {
                    reserve_class(&mut reserved, &target.owner);
                    reserve_type(&mut reserved, &target.return_type);
                    for t in &target.parameters {
                        reserve_type(&mut reserved, t);
                    }
                }
            }
            ManagedInsn::FieldGet { field, .. } | ManagedInsn::FieldPut { field, .. } => {
                if let Some(f) = r.field(*field) {
                    reserve_class(&mut reserved, &f.owner);
                    reserve_type(&mut reserved, &f.ty);
                }
            }
            _ => {}
        }
    }
    let mut e = Emitter {
        allocation_readable: None,
        object_planning: std::rc::Rc::new(object_plan::ObjectPlanning::for_source_emission(
            scopes.is_none_or(|metadata| metadata.scopes().is_empty()),
        )),
        ir,
        resolver: r,
        values: HashMap::new(),
        lines: Vec::new(),
        next: 0,
        parameters: HashSet::new(),
        reserved,
        current: None,
        hoisted: HashSet::new(),
        declarations: contextual_locals::declaration_plan(ir, &m.return_type, r)
            .map_err(|e| Unsupported::Type(format!("phi declaration plan: {e:?}")))?,
    };
    let mut names = Vec::new();
    let mut used = HashSet::new();
    let mut params = Vec::new();
    for (i, (v, t)) in b.parameters.iter().zip(&m.parameters).enumerate() {
        if *t == JavaType::Void {
            return Err(Unsupported::Metadata("void parameter".into()));
        }
        let hint = b.names.get(i).cloned().unwrap_or_else(|| format!("p{i}"));
        let mut n = sanitize_identifier(&hint);
        // Reserve all v<number> names for SSA temporaries.
        if n.strip_prefix('v')
            .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        {
            n.insert_str(0, "p_");
        }
        let base = n.clone();
        let mut suffix = 1;
        while e.reserved.contains(&n) || !used.insert(n.clone()) {
            n = format!("{base}_{suffix}");
            suffix += 1;
        }
        if let Some(v) = v {
            bind(&mut e, *v, n.clone(), t.clone())?;
        }
        params.push(format!("{} {n}", t.source()?));
        names.push(n);
    }
    e.reserved.extend(names.iter().cloned());
    if let Some(v) = b.receiver {
        bind(&mut e, v, "this".into(), JavaType::Class(m.owner.clone()))?;
    }
    let cfg = ir.cfg();
    // Linear methods retain their original temporary declarations. Branching
    // methods require method-scope locals, with initialization kept in place.
    let branching = cfg.edge_ids().any(|id| {
        cfg.edge(id).is_ok_and(|edge| {
            matches!(
                edge.kind(),
                EdgeKind::BranchTrue | EdgeKind::BranchFalse | EdgeKind::Switch(_)
            )
        })
    }) || cfg.block_ids().any(|id| {
        cfg.block(id).is_ok_and(|b| {
            b.incoming().len() > 1 || (id == cfg.entry() && !b.incoming().is_empty())
        })
    });
    if let Some(scopes) = scopes {
        let bound: Vec<_> = e.parameters.iter().copied().collect();
        let plan = exceptions::plan(ir, scopes, &bound, r)
            .map_err(|err| Unsupported::ControlFlow(err.to_string()))?;
        let caught = plan.handlers.iter().map(|h| h.ty.clone()).collect();
        exceptions::emit(
            &plan,
            &mut ExceptionEmitter {
                emitter: &mut e,
                method: m,
                caught,
                protected: false,
            },
        )?;
    } else if branching {
        let bound: Vec<_> = e.parameters.iter().copied().collect();
        let has_switch = cfg.edge_ids().any(|id| {
            cfg.edge(id)
                .is_ok_and(|edge| matches!(edge.kind(), EdgeKind::Switch(_)))
        });
        let mut control = None;
        let mut forward = None;
        let mut dispatch_plan = None;
        let mut short_circuit = None;
        let declarations = e.declarations.keys().copied().collect();
        let candidate = if m.return_type == JavaType::Boolean && !has_switch {
            branching::plan_short_circuit_boolean_return(ir, &bound, &declarations)
        } else {
            None
        };
        let plan = if let Some(candidate) = candidate {
            let locals = candidate.locals.clone();
            short_circuit = Some(candidate);
            Ok(branching::BranchPlan {
                locals,
                steps: Vec::new(),
            })
        } else if has_switch {
            branching::plan_extended_canonical_checked(ir, &bound, &declarations)
        } else {
            match branching::plan_canonical_checked(ir, &bound, &declarations) {
                Err(branching::PlanError::Structure(_)) => {
                    branching::plan_control(ir, &bound).map(|(locals, steps)| {
                        control = Some(steps);
                        branching::BranchPlan {
                            locals,
                            steps: Vec::new(),
                        }
                    })
                }
                result => result,
            }
        }
        .or_else(|original| {
            branching::plan_forward_checked(ir, &bound, &declarations)
                .map(|plan| {
                    let locals = plan.locals.clone();
                    forward = Some(plan);
                    branching::BranchPlan {
                        locals,
                        steps: Vec::new(),
                    }
                })
                .map_err(|forward_error| {
                    branching::PlanError::InvalidIr(format!(
                        "strict planner: {original}; forward planner: {forward_error}"
                    ))
                })
        })
        .or_else(|original| {
            if matches!(m.name.as_str(), "<init>" | "<clinit>") {
                return Err(original);
            }
            branching::plan_dispatch_checked(ir, &bound, &declarations)
                .map(|plan| {
                    let locals = plan.locals.clone();
                    dispatch_plan = Some(plan);
                    branching::BranchPlan {
                        locals,
                        steps: Vec::new(),
                    }
                })
                .map_err(|error| {
                    branching::PlanError::InvalidIr(format!(
                        "structured planner: {original}; dispatch planner: {error}"
                    ))
                })
        })
        .map_err(|err| Unsupported::ControlFlow(err.to_string()))?;
        for v in &plan.locals {
            if e.is_pure_expression(*v) {
                continue;
            }
            let ty = e.ty(*v)?;
            let mut name = format!("v{}", e.next);
            e.next += 1;
            while e.reserved.contains(&name) {
                name = format!("v{}", e.next);
                e.next += 1;
            }
            if dispatch_plan.is_some() {
                let initial = dispatch_default(&ty)?;
                e.lines
                    .push(format!("{} {name} = {initial};", ty.source()?));
            } else {
                e.lines.push(format!("{} {name};", ty.source()?));
            }
            e.reserved.insert(name.clone());
            e.values.insert(*v, (name, ty));
            e.hoisted.insert(*v);
        }
        if let Some(plan) = short_circuit {
            render_short_circuit_boolean_return(&mut e, &plan, m)?;
        } else if let Some(plan) = dispatch_plan {
            let mut index = 0usize;
            let prefix = loop {
                let prefix = format!("dispatch{index}");
                if !e.reserved.contains(&format!("{prefix}_pc"))
                    && !e.reserved.contains(&format!("{prefix}_loop"))
                {
                    break prefix;
                }
                index += 1;
            };
            e.reserved.insert(format!("{prefix}_pc"));
            e.reserved.insert(format!("{prefix}_loop"));
            let body = dispatch::render(
                cfg,
                &plan.regions,
                &prefix,
                &mut ForwardJava {
                    emitter: &mut e,
                    method: m,
                    copies: &plan.copies,
                },
            )
            .map_err(Unsupported::ControlFlow)?;
            e.lines.push(body);
        } else if let Some(plan) = forward {
            let body = shared_regions::render(
                cfg,
                &plan.regions,
                "forwardBlock",
                &mut ForwardJava {
                    emitter: &mut e,
                    method: m,
                    copies: &plan.copies,
                },
            )
            .map_err(Unsupported::ControlFlow)?;
            e.lines.push(body);
        } else if let Some(steps) = control {
            render_control(&mut e, &steps, m, 0)?;
        } else {
            render_steps(&mut e, &plan.steps, m)?;
        }
    } else {
        cfg.validate()
            .map_err(|err| Unsupported::ControlFlow(err.to_string()))?;
        for edge in cfg.edge_ids() {
            if matches!(
                cfg.edge(edge)
                    .map_err(|err| Unsupported::ControlFlow(err.to_string()))?
                    .kind(),
                EdgeKind::Exception(_)
            ) {
                return Err(Unsupported::ControlFlow("exception edges".into()));
            }
        }
        let mut block = cfg.entry();
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(block) {
                return Err(Unsupported::ControlFlow("cycle".into()));
            }
            let node = cfg
                .block(block)
                .map_err(|err| Unsupported::ControlFlow(err.to_string()))?;
            let insn = ir
                .instruction(*node.payload())
                .map_err(|err| Unsupported::Instruction(err.to_string()))?;
            e.current = Some(*node.payload());
            let returned = if let ManagedInsn::Branch {
                condition: None,
                then_edge,
                else_edge: None,
            } = insn
            {
                // A retained original goto has no expression effect. The
                // linear CFG traversal below performs its exact transfer;
                // require its encoded edge to be that sole CFG successor.
                if node.outgoing() != [*then_edge]
                    || !cfg.edge(*then_edge).is_ok_and(|edge| {
                        edge.source() == block && *edge.kind() == EdgeKind::Unconditional
                    })
                {
                    return Err(Unsupported::ControlFlow(
                        "unconditional instruction/CFG edge mismatch".into(),
                    ));
                }
                false
            } else {
                e.instruction(insn, m)?
            };
            if returned {
                if !node.outgoing().is_empty() {
                    return Err(Unsupported::ControlFlow("return with successors".into()));
                }
                break;
            }
            if node.outgoing().len() != 1 {
                return Err(Unsupported::ControlFlow(
                    "requires linear graph ending in return".into(),
                ));
            }
            let edge = cfg
                .edge(node.outgoing()[0])
                .map_err(|err| Unsupported::ControlFlow(err.to_string()))?;
            if *edge.kind() != EdgeKind::Unconditional {
                return Err(Unsupported::ControlFlow("branch edge".into()));
            }
            block = edge.target();
        }
        if seen.len() != cfg.block_count() {
            return Err(Unsupported::ControlFlow("unreachable blocks".into()));
        }
    }
    let original = if let Some(recipe) = r.checked_source_body_bridge(m) {
        let mut n = 0;
        let catch_name = loop {
            let name = format!("checkedBodyFailure{n}");
            if !e.reserved.contains(&name) && !e.values.values().any(|(v, _)| v == &name) {
                break name;
            }
            n += 1;
        };
        e.lines = recipe.wrap_body(&e.lines, &catch_name)?;
        recipe
            .body_original(m)
            .ok_or_else(|| Unsupported::Metadata("whole-body rendering signature".into()))?
    } else {
        m
    };
    let name = sanitize_identifier(&original.name);
    let throws = if original.throws.is_empty() {
        String::new()
    } else {
        format!(
            " throws {}",
            original
                .throws
                .iter()
                .map(|n| qualified(n))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        )
    };
    let source = format!(
        "public {}{} {name}({}){throws} {{\n{}\n}}\n",
        if original.is_static { "static " } else { "" },
        original.return_type.source()?,
        params.join(", "),
        e.lines
            .iter()
            .map(|s| format!("    {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    Ok((source, name, names))
}
fn render_steps<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    steps: &[branching::Step],
    m: &JavaMethod,
) -> Result<(), Unsupported> {
    render_steps_at(e, steps, m, 0)
}

fn render_short_circuit_boolean_return<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    plan: &branching::ShortCircuitBooleanReturn,
    method: &JavaMethod,
) -> Result<(), Unsupported> {
    if method.return_type != JavaType::Boolean {
        return Err(Unsupported::ControlFlow(
            "short-circuit plan requires Boolean return".into(),
        ));
    }
    if let Some(inline) = direct_short_circuit_boolean_calls(e, plan, method)
        && let Some(declarations) = inline_declarations_to_remove(e, &inline.values)
    {
        for value in &inline.values {
            e.values.remove(value);
            e.hoisted.remove(value);
        }
        e.lines
            .retain(|line| !declarations.iter().any(|declaration| declaration == line));
        e.lines.push(format!("if (!{}) return false;", inline.left));
        e.lines.push(format!("return {};", inline.right));
        return Ok(());
    }
    emit_short_circuit_instructions(e, &plan.before, method)?;
    let (left, left_type) = e.get(plan.left)?;
    compatible(&left_type, &JavaType::Boolean)?;
    e.lines.push(format!("if (!{left}) return false;"));
    emit_short_circuit_instructions(e, &plan.between, method)?;
    let (right, right_type) = e.get(plan.right)?;
    compatible(&right_type, &JavaType::Boolean)?;
    e.lines.push(format!("return {right};"));
    Ok(())
}

struct DirectShortCircuitBooleanCalls {
    left: String,
    right: String,
    values: Vec<ValueId>,
}

fn direct_short_circuit_boolean_calls<R: JavaResolver>(
    e: &Emitter<'_, R>,
    plan: &branching::ShortCircuitBooleanReturn,
    method: &JavaMethod,
) -> Option<DirectShortCircuitBooleanCalls> {
    // Initial receiver definitions are non-executable binding sentinels. The
    // emitter already ignores them, but they can share the linear prefix with
    // the first real field/call pair.
    let mut before = Vec::new();
    let mut saw_this_binding = false;
    for &id in &plan.before {
        let insn = e.ir.instruction(id).ok()?;
        if let ManagedInsn::Opaque {
            family_tag,
            reads,
            writes,
            effects,
        } = insn
            && matches!(*family_tag, 0x8001 | 0x8003)
            && reads.is_empty()
            && writes.len() == 1
            && e.parameters.contains(&writes[0])
            && *effects == managed_ir::effects::EffectFlags::NONE
            && e.values.get(&writes[0]).is_some_and(|(name, ty)| {
                name == "this" && *ty == JavaType::Class(method.owner.clone())
            })
        {
            if saw_this_binding {
                return None;
            }
            saw_this_binding = true;
        } else {
            before.push(id);
        }
    }
    let [left_field, left_invoke] = before.as_slice() else {
        return None;
    };
    let [right_field, right_invoke] = plan.between.as_slice() else {
        return None;
    };

    let (left, left_field_value) =
        direct_short_circuit_boolean_call(e, *left_field, *left_invoke, plan.left, method)?;
    let (right, right_field_value) =
        direct_short_circuit_boolean_call(e, *right_field, *right_invoke, plan.right, method)?;
    let left_branch = short_circuit_condition_branch(e.ir, plan.left)?;
    let right_branch = short_circuit_condition_branch(e.ir, plan.right)?;
    if left_branch == right_branch {
        return None;
    }

    Some(DirectShortCircuitBooleanCalls {
        left,
        right,
        values: vec![left_field_value, plan.left, right_field_value, plan.right],
    })
}

fn short_circuit_condition_branch(ir: &MethodIr<SsaForm>, value: ValueId) -> Option<InsnId> {
    let uses = branching::use_sites(ir, value);
    let [use_id] = uses.as_slice() else {
        return None;
    };
    let instruction = ir.instruction(*use_id).ok()?;
    let branch = match instruction {
        ManagedInsn::Branch {
            condition: Some(condition),
            ..
        } if *condition == value => return Some(*use_id),
        ManagedInsn::Not { result, .. }
        | ManagedInsn::Move { result, .. }
        | ManagedInsn::Compare { result, .. } => *result,
        _ => return None,
    };
    let uses = branching::use_sites(ir, branch);
    let [branch_id] = uses.as_slice() else {
        return None;
    };
    matches!(
        ir.instruction(*branch_id).ok()?,
        ManagedInsn::Branch {
            condition: Some(condition),
            ..
        } if *condition == branch
    )
    .then_some(*branch_id)
}

fn direct_short_circuit_boolean_call<R: JavaResolver>(
    e: &Emitter<'_, R>,
    field_id: InsnId,
    invoke_id: InsnId,
    result_value: ValueId,
    caller: &JavaMethod,
) -> Option<(String, ValueId)> {
    let ManagedInsn::FieldGet {
        result: field_value,
        object: Some(receiver),
        field: field_reference,
    } = e.ir.instruction(field_id).ok()?
    else {
        return None;
    };
    let ManagedInsn::Invoke {
        kind,
        result: Some(invoke_result),
        method: method_reference,
        args,
    } = e.ir.instruction(invoke_id).ok()?
    else {
        return None;
    };
    if invoke_result != &result_value
        || !matches!(kind, InvokeKind::Virtual | InvokeKind::Interface)
        || args.as_slice() != [*field_value]
        || branching::use_sites(e.ir, *field_value).as_slice() != [invoke_id]
        || e.ty(result_value).ok()? != JavaType::Boolean
    {
        return None;
    }

    // Restrict the peephole to an explicitly bound `this` receiver. The normal
    // field target helper retains all source-name, access, and receiver checks.
    let (receiver_name, receiver_type) = e.get(*receiver).ok()?;
    if receiver_name != "this" || receiver_type != JavaType::Class(caller.owner.clone()) {
        return None;
    }
    let field = e.resolver.field(*field_reference)?;
    if field.is_static || field.ty == JavaType::Void || e.ty(*field_value).ok()? != field.ty {
        return None;
    }
    qualified(&field.owner).ok()?;
    let field_expression = e
        .field_target(Some(*receiver), &field, *field_reference, &caller.owner)
        .ok()?;

    let array_clone = if *kind == InvokeKind::Virtual {
        e.resolver.array_clone_method(*method_reference, &field.ty)
    } else {
        None
    };
    let receiver_method = if *kind == InvokeKind::Virtual {
        e.resolver
            .virtual_receiver_method(*method_reference, &caller.owner, &field.ty)
    } else {
        None
    };
    let target_method = receiver_method
        .clone()
        .or(array_clone)
        .or_else(|| e.resolver.method(*method_reference))?;
    if target_method.is_static
        || target_method.return_type != JavaType::Boolean
        || !target_method.parameters.is_empty()
        || e.resolver
            .method_access(*method_reference)
            .is_some_and(|flags| flags & 2 != 0)
        || checked_exceptions::check(e.resolver, &target_method.throws, &caller.throws).is_err()
    {
        return None;
    }
    qualified(&target_method.owner).ok()?;
    member(&target_method.name).ok()?;
    for exception in &target_method.throws {
        qualified(exception).ok()?;
    }
    let operands = invoke_arguments::operands(&target_method, args).ok()?;
    if operands.as_slice() != [*field_value] {
        return None;
    }
    if let Some(recipe) = e.resolver.checked_source_body_bridge(caller)
        && !recipe.approves_rendering_invoke(
            invoke_id,
            e.ir.instruction(invoke_id).ok()?,
            &target_method,
            caller,
        )
    {
        return None;
    }

    let owner = JavaType::Class(target_method.owner.clone());
    e.compatible(&field.ty, &owner).ok()?;
    let receiver_is_selected = receiver_method.is_some();
    let target = if receiver_is_selected
        || field.ty == owner
        || e.resolver
            .retain_method_receiver(*method_reference, &caller.owner, &field.ty)
    {
        field_expression
    } else {
        if e.resolver
            .method_access(*method_reference)
            .is_some_and(|flags| flags & 4 != 0)
            && caller.owner.rsplit_once('.').map(|package| package.0)
                != target_method
                    .owner
                    .rsplit_once('.')
                    .map(|package| package.0)
        {
            return None;
        }
        format!("(({}) {field_expression})", owner.source().ok()?)
    };
    Some((format!("{target}.{}()", target_method.name), *field_value))
}

fn inline_declarations_to_remove(
    e: &Emitter<'_, impl JavaResolver>,
    values: &[ValueId],
) -> Option<Vec<String>> {
    let mut declarations = Vec::new();
    for value in values {
        match (e.values.get(value), e.hoisted.contains(value)) {
            (Some((name, ty)), true) => {
                let declaration = format!("{} {name};", ty.source().ok()?);
                if e.lines.iter().filter(|line| *line == &declaration).count() != 1 {
                    return None;
                }
                declarations.push(declaration);
            }
            (None, false) => {}
            _ => return None,
        }
    }
    Some(declarations)
}

fn emit_short_circuit_instructions<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    instructions: &[InsnId],
    method: &JavaMethod,
) -> Result<(), Unsupported> {
    for &id in instructions {
        let insn =
            e.ir.instruction(id)
                .map_err(|error| Unsupported::Instruction(error.to_string()))?
                .clone();
        if matches!(
            insn,
            ManagedInsn::Branch { .. }
                | ManagedInsn::Switch { .. }
                | ManagedInsn::Phi { .. }
                | ManagedInsn::Return { .. }
                | ManagedInsn::Throw { .. }
        ) {
            return Err(Unsupported::ControlFlow(
                "invalid instruction in short-circuit linear segment".into(),
            ));
        }
        e.current = Some(id);
        e.instruction(&insn, method)?;
    }
    Ok(())
}

struct ForwardJava<'a, 'ir, R> {
    emitter: &'a mut Emitter<'ir, R>,
    method: &'a JavaMethod,
    copies: &'a HashMap<decomp_core::ids::EdgeId, Vec<branching::Copy>>,
}

fn dispatch_default(ty: &JavaType) -> Result<&'static str, Unsupported> {
    match ty {
        JavaType::Boolean => Ok("false"),
        JavaType::Byte | JavaType::Short | JavaType::Int => Ok("0"),
        JavaType::Char => Ok("'\\0'"),
        JavaType::Long => Ok("0L"),
        JavaType::Float => Ok("0.0f"),
        JavaType::Double => Ok("0.0d"),
        JavaType::Class(_) | JavaType::Array(_) => Ok("null"),
        JavaType::Void => Err(Unsupported::Type("void dispatcher local".into())),
    }
}

impl<R: JavaResolver> dispatch::DispatchEmitter for ForwardJava<'_, '_, R> {
    fn instructions(&mut self, block: decomp_core::ids::BlockId) -> Result<String, String> {
        shared_regions::ForwardEmitter::instructions(self, block)
    }
    fn terminal(&mut self, block: decomp_core::ids::BlockId) -> Result<String, String> {
        shared_regions::ForwardEmitter::terminal(self, block)
    }
    fn condition(&mut self, block: decomp_core::ids::BlockId) -> Result<String, String> {
        shared_regions::ForwardEmitter::condition(self, block)
    }
    fn selector(&mut self, block: decomp_core::ids::BlockId) -> Result<String, String> {
        shared_regions::ForwardEmitter::selector(self, block)
    }
    fn copies(&mut self, edge: decomp_core::ids::EdgeId) -> Result<String, String> {
        shared_regions::ForwardEmitter::copies(self, edge)
    }
}
impl<R: JavaResolver> ForwardJava<'_, '_, R> {
    fn insn(&self, block: decomp_core::ids::BlockId) -> Result<(InsnId, &ManagedInsn), String> {
        let id = *self
            .emitter
            .ir
            .cfg()
            .block(block)
            .map_err(|e| e.to_string())?
            .payload();
        Ok((
            id,
            self.emitter.ir.instruction(id).map_err(|e| e.to_string())?,
        ))
    }
    fn emit(&mut self, block: decomp_core::ids::BlockId, terminal: bool) -> Result<String, String> {
        let (id, insn) = self.insn(block)?;
        let insn = insn.clone();
        let is_terminal = matches!(insn, ManagedInsn::Return { .. } | ManagedInsn::Throw { .. });
        if terminal != is_terminal {
            return Err("forward terminal shape mismatch".into());
        }
        if matches!(
            insn,
            ManagedInsn::Phi { .. } | ManagedInsn::Branch { .. } | ManagedInsn::Switch { .. }
        ) {
            return Ok(String::new());
        }
        if self
            .emitter
            .folded_argument_region_step(&branching::Step::Instruction(id))
            .map_err(|e| format!("{e:?}"))?
        {
            return Ok(String::new());
        }
        let start = self.emitter.lines.len();
        self.emitter.current = Some(id);
        self.emitter
            .instruction(&insn, self.method)
            .map_err(|e| format!("{e:?}"))?;
        Ok(self
            .emitter
            .lines
            .drain(start..)
            .collect::<Vec<_>>()
            .join("\n"))
    }
}
impl<R: JavaResolver> shared_regions::ForwardEmitter for ForwardJava<'_, '_, R> {
    fn instructions(&mut self, block: decomp_core::ids::BlockId) -> Result<String, String> {
        if matches!(
            self.insn(block)?.1,
            ManagedInsn::Return { .. } | ManagedInsn::Throw { .. }
        ) {
            Ok(String::new())
        } else {
            self.emit(block, false)
        }
    }
    fn terminal(&mut self, block: decomp_core::ids::BlockId) -> Result<String, String> {
        self.emit(block, true)
    }
    fn condition(&mut self, block: decomp_core::ids::BlockId) -> Result<String, String> {
        if self
            .emitter
            .folded_argument_region_branch(self.insn(block)?.0)
        {
            return Ok("true".into());
        }
        let ManagedInsn::Branch {
            condition: Some(value),
            ..
        } = self.insn(block)?.1
        else {
            return Err("forward condition shape".into());
        };
        let (name, ty) = self.emitter.get(*value).map_err(|e| format!("{e:?}"))?;
        compatible(&ty, &JavaType::Boolean).map_err(|e| format!("{e:?}"))?;
        Ok(name)
    }
    fn selector(&mut self, block: decomp_core::ids::BlockId) -> Result<String, String> {
        let ManagedInsn::Switch { key, .. } = self.insn(block)?.1 else {
            return Err("forward selector shape".into());
        };
        let (name, ty) = self.emitter.get(*key).map_err(|e| format!("{e:?}"))?;
        compatible(&ty, &JavaType::Int).map_err(|e| format!("{e:?}"))?;
        Ok(name)
    }
    fn copies(&mut self, edge: decomp_core::ids::EdgeId) -> Result<String, String> {
        let start = self.emitter.lines.len();
        render_steps(
            self.emitter,
            &[branching::Step::Copies {
                edge,
                copies: self.copies.get(&edge).cloned().unwrap_or_default(),
            }],
            self.method,
        )
        .map_err(|e| format!("{e:?}"))?;
        Ok(self
            .emitter
            .lines
            .drain(start..)
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

fn render_control<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    steps: &[loop_control::ControlStep],
    m: &JavaMethod,
    depth: usize,
) -> Result<(), Unsupported> {
    use loop_control::ControlStep;
    use loop_control_regions::TransferKind;
    for step in steps {
        let indent = "    ".repeat(depth);
        match step {
            ControlStep::Ordinary(step) => {
                render_steps_at(e, std::slice::from_ref(step), m, depth)?
            }
            ControlStep::If {
                condition,
                then_steps,
                else_steps,
            } => {
                let (name, ty) = e.get(*condition)?;
                compatible(&ty, &JavaType::Boolean)?;
                e.lines.push(format!("{indent}if ({name}) {{"));
                render_control(e, then_steps, m, depth + 1)?;
                e.lines.push(format!("{indent}}} else {{"));
                render_control(e, else_steps, m, depth + 1)?;
                e.lines.push(format!("{indent}}}"));
            }
            ControlStep::Loop { header, body } => {
                let label: String = format!("{header:?}")
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .collect();
                e.lines
                    .push(format!("{indent}loop{label}: while (true) {{"));
                render_control(e, body, m, depth + 1)?;
                e.lines.push(format!("{indent}}}"));
            }
            ControlStep::Transfer { kind, .. } => {
                let (keyword, header) = match kind {
                    TransferKind::Break { header } => ("break", header),
                    TransferKind::Continue { header } => ("continue", header),
                };
                let label: String = format!("{header:?}")
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .collect();
                e.lines.push(format!("{indent}{keyword} loop{label};"));
            }
        }
    }
    Ok(())
}

fn render_steps_at<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    steps: &[branching::Step],
    m: &JavaMethod,
    depth: usize,
) -> Result<(), Unsupported> {
    for step in steps {
        if e.folded_argument_region_step(step)? {
            continue;
        }
        let start = e.lines.len();
        let indent = "    ".repeat(depth);
        match step {
            branching::Step::Switch { dispatch, bodies } => {
                let (key, ty) = e.get(dispatch.key)?;
                compatible(&ty, &JavaType::Int)?;
                if dispatch.dispatch.iter().any(|d| !d.copies.is_empty()) {
                    e.lines.push(format!("{indent}switch ({key}) {{"));
                    for entry in &dispatch.dispatch {
                        let label = entry
                            .label
                            .map_or_else(|| "default".into(), |v| format!("case {v}"));
                        e.lines.push(format!("{indent}{label}: {{"));
                        let copies = entry
                            .copies
                            .iter()
                            .map(|c| branching::Copy {
                                destination: c.destination,
                                source: c.source,
                            })
                            .collect();
                        render_steps_at(
                            e,
                            &[branching::Step::Copies {
                                edge: entry.edge,
                                copies,
                            }],
                            m,
                            depth + 1,
                        )?;
                        e.lines.push(format!("{indent}    break;"));
                        e.lines.push(format!("{indent}}}"));
                    }
                    e.lines.push(format!("{indent}}}"));
                }
                e.lines.push(format!("{indent}switch ({key}) {{"));
                for (arm, (steps, continues)) in dispatch.arms.iter().zip(bodies) {
                    for label in &arm.labels {
                        e.lines.push(format!("{indent}case {}:", *label as i64));
                    }
                    if arm.is_default {
                        e.lines.push(format!("{indent}default:"));
                    }
                    e.lines.push(format!("{indent}{{"));
                    render_steps_at(e, steps, m, depth + 1)?;
                    if *continues {
                        e.lines.push(format!("{indent}    break;"));
                    }
                    e.lines.push(format!("{indent}}}"));
                }
                e.lines.push(format!("{indent}}}"));
                continue;
            }
            branching::Step::While {
                evaluation,
                condition,
                body_on_true,
                body_steps,
                exit_steps,
            } => {
                let (name, ty) = e.get(*condition)?;
                compatible(&ty, &JavaType::Boolean)?;
                e.lines.push(format!("{indent}while (true) {{"));
                render_steps_at(e, evaluation, m, depth + 1)?;
                let stop = if *body_on_true {
                    format!("!{name}")
                } else {
                    name
                };
                e.lines.push(format!("{indent}    if ({stop}) {{"));
                render_steps_at(e, exit_steps, m, depth + 2)?;
                e.lines.push(format!("{indent}        break;"));
                e.lines.push(format!("{indent}    }}"));
                render_steps_at(e, body_steps, m, depth + 1)?;
                e.lines.push(format!("{indent}}}"));
                continue;
            }
            branching::Step::Instruction(id) => {
                e.current = Some(*id);
                let insn =
                    e.ir.instruction(*id)
                        .map_err(|err| Unsupported::Instruction(err.to_string()))?;
                e.instruction(insn, m)?;
            }
            branching::Step::If {
                condition,
                then_steps,
                else_steps,
            } => {
                let (name, ty) = e.get(*condition)?;
                compatible(&ty, &JavaType::Boolean)?;
                e.lines.push(format!("{indent}if ({name}) {{"));
                render_steps_at(e, then_steps, m, depth + 1)?;
                e.lines.push(format!("{indent}}} else {{"));
                render_steps_at(e, else_steps, m, depth + 1)?;
                e.lines.push(format!("{indent}}}"));
                continue;
            }
            branching::Step::Copies { copies, .. } => {
                let mut snapshots = Vec::new();
                for copy in copies {
                    let (destination, dt) = e.get(copy.destination)?;
                    let (source, ty) = e.operand(copy.source, &dt)?;
                    e.compatible(&ty, &dt)?;
                    let mut temp = format!("edge{}", e.next);
                    e.next += 1;
                    while e.reserved.contains(&temp) {
                        temp = format!("edge{}", e.next);
                        e.next += 1;
                    }
                    e.reserved.insert(temp.clone());
                    e.lines.push(format!("{} {temp} = {source};", ty.source()?));
                    snapshots.push((destination, temp));
                }
                for (destination, temp) in snapshots {
                    e.lines.push(format!("{destination} = {temp};"));
                }
            }
        }
        for line in &mut e.lines[start..] {
            line.insert_str(0, &indent);
        }
    }
    Ok(())
}
fn bind<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    v: ValueId,
    name: String,
    ty: JavaType,
) -> Result<(), Unsupported> {
    let value =
        e.ir.value(v)
            .map_err(|err| Unsupported::Metadata(err.to_string()))?;
    // Explicit bindings may refine frontend Unknown parameter types, but not contradict known types.
    if !matches!(value.ty(), ManagedType::Unknown) && managed_type(value.ty(), e.resolver)? != ty {
        return Err(Unsupported::Type("parameter type mismatch".into()));
    }
    if e.values.insert(v, (name, ty)).is_some() {
        return Err(Unsupported::Metadata("duplicate parameter binding".into()));
    }
    e.parameters.insert(v);
    Ok(())
}
fn reserve_class(names: &mut HashSet<String>, name: &str) {
    if let Some(root) = name.split('.').next() {
        names.insert(root.into());
    }
}
fn reserve_type(names: &mut HashSet<String>, ty: &JavaType) {
    match ty {
        JavaType::Class(n) => reserve_class(names, n),
        JavaType::Array(t) => reserve_type(names, t),
        _ => {}
    }
}
fn compatible(from: &JavaType, to: &JavaType) -> Result<(), Unsupported> {
    if (from == to && *to != JavaType::Void)
        || type_compatibility::narrow_register_widening(from, to)
    {
        Ok(())
    } else {
        Err(Unsupported::Type(format!("assignment {from:?} to {to:?}")))
    }
}
fn managed_type(t: &ManagedType, r: &impl JavaResolver) -> Result<JavaType, Unsupported> {
    Ok(match t {
        ManagedType::Boolean => JavaType::Boolean,
        ManagedType::Byte => JavaType::Byte,
        ManagedType::Short => JavaType::Short,
        ManagedType::Char => JavaType::Char,
        ManagedType::Int => JavaType::Int,
        ManagedType::Long => JavaType::Long,
        ManagedType::Float => JavaType::Float,
        ManagedType::Double => JavaType::Double,
        ManagedType::Reference(c) => JavaType::Class(
            r.class(*c)
                .ok_or_else(|| Unsupported::Metadata("unresolved class".into()))?,
        ),
        ManagedType::Array { element, .. } => JavaType::Array(Box::new(managed_type(element, r)?)),
        _ => {
            return Err(Unsupported::Type(format!(
                "unresolved or uninitialized type {t:?}"
            )));
        }
    })
}
fn constant(c: Constant, t: &JavaType, r: &impl JavaResolver) -> Result<String, Unsupported> {
    let bad = || Unsupported::Type("constant type mismatch".into());
    Ok(match c {
        Constant::ClassLiteral(target) if *t == JavaType::Class("java.lang.Class".into()) => {
            class_literal::expression(r, target, JavaType::source)?
        }
        Constant::Zero => match t {
            JavaType::Boolean => "false".into(),
            JavaType::Long => "0L".into(),
            JavaType::Float => "0.0f".into(),
            JavaType::Double => "0.0d".into(),
            t if t.reference() => "null".into(),
            t if t.integral() => "0".into(),
            _ => return Err(bad()),
        },
        Constant::Boolean(v) if *t == JavaType::Boolean => v.to_string(),
        Constant::Int(v) if *t == JavaType::Int => v.to_string(),
        Constant::Int(v) if matches!(t, JavaType::Byte | JavaType::Short | JavaType::Char) => {
            let valid = match t {
                JavaType::Byte => i8::try_from(v).is_ok(),
                JavaType::Short => i16::try_from(v).is_ok(),
                _ => u16::try_from(v).is_ok(),
            };
            if !valid {
                return Err(bad());
            }
            v.to_string()
        }
        Constant::Long(v) if *t == JavaType::Long => format!("{v}L"),
        Constant::FloatBits(v) if *t == JavaType::Float => {
            format!("java.lang.Float.intBitsToFloat(0x{v:08x})")
        }
        Constant::DoubleBits(v) if *t == JavaType::Double => {
            format!("java.lang.Double.longBitsToDouble(0x{v:016x}L)")
        }
        Constant::Null if t.reference() => "null".into(),
        Constant::String(s) if *t == JavaType::Class("java.lang.String".into()) => {
            let units = r
                .string(s)
                .ok_or_else(|| Unsupported::Metadata("unresolved string".into()))?;
            java_string_literal::render(&units)?
        }
        _ => return Err(bad()),
    })
}
/// Convert a source hint to a deterministic conservative ASCII Java identifier.
pub fn sanitize_identifier(name: &str) -> String {
    let mut result: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if result.is_empty() {
        result.push('_');
    }
    if result.starts_with(|c: char| c.is_ascii_digit()) {
        result.insert(0, '_');
    }
    if keyword(&result) {
        result.push('_');
    }
    result
}
fn keyword(n: &str) -> bool {
    matches!(
        n,
        "abstract"
            | "assert"
            | "boolean"
            | "break"
            | "byte"
            | "case"
            | "catch"
            | "char"
            | "class"
            | "const"
            | "continue"
            | "default"
            | "do"
            | "double"
            | "else"
            | "enum"
            | "extends"
            | "final"
            | "finally"
            | "float"
            | "for"
            | "goto"
            | "if"
            | "implements"
            | "import"
            | "instanceof"
            | "int"
            | "interface"
            | "long"
            | "native"
            | "new"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "short"
            | "static"
            | "strictfp"
            | "super"
            | "switch"
            | "synchronized"
            | "this"
            | "throw"
            | "throws"
            | "transient"
            | "try"
            | "void"
            | "volatile"
            | "while"
            | "true"
            | "false"
            | "null"
            | "_"
            | "var"
            | "yield"
            | "record"
            | "sealed"
            | "permits"
    )
}
fn member(n: &str) -> Result<(), Unsupported> {
    if sanitize_identifier(n) == n {
        Ok(())
    } else {
        Err(Unsupported::Metadata(format!(
            "invalid source member name {n:?}"
        )))
    }
}
fn qualified(n: &str) -> Result<String, Unsupported> {
    if n.split('.').all(|s| !s.is_empty() && member(s).is_ok()) {
        Ok(n.into())
    } else {
        Err(Unsupported::Metadata(format!(
            "invalid source class name {n:?}"
        )))
    }
}

#[cfg(test)]
mod array_clone_tests;
#[cfg(test)]
mod short_circuit_boolean_tests;
#[cfg(test)]
mod super_invocation_binding_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod throw_null_tests;

mod static_method_qualifier;
