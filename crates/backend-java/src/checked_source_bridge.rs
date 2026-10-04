//! Proven caller-local source bridges. Runtime exception facts are retained;
//! this module never changes a callee's Java declaration or invocation target.
//! Synthetic helpers add reflective/live-stack observability and are separately
//! attributed compiler artifacts, not original input-method recoveries.

use crate::{
    DeclarationBindings, JavaField, JavaMethod, JavaResolver, JavaType, SourceResult,
    SuperInvocation, Unsupported,
};
use managed_ir::{
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
    ops::InvokeKind,
    refs::{ClassRef, FieldRef, MethodRef, StringRef, TypeRef},
    value::{InsnId, ValueId},
};
use std::collections::{HashMap, HashSet};

#[path = "checked_source_bridge_bound_target.rs"]
mod bound_target;
#[path = "checked_source_bridge_dispatch.rs"]
mod dispatch_body;
pub use bound_target::{BoundTarget, FieldThrowDefinition};
pub use dispatch_body::{
    BoundDispatchInputs, emit_dispatch_body, plan_dispatch_body, plan_dispatch_body_bound_targets,
};

#[derive(Clone, Debug)]
struct InvokeSite {
    reference: MethodRef,
    kind: InvokeKind,
    result: Option<ValueId>,
    args: Vec<ValueId>,
    target: JavaMethod,
}

/// A complete local recipe, bound to one original SSA and original declaration.
/// A frontend must separately prove a non-interface, nonsynchronized ordinary
/// class method with no encoded handlers, and a collision-free helper name.
#[derive(Clone, Debug)]
pub struct Plan {
    mode: Mode,
    throwable_boundary: bool,
    dispatch_scopes: Option<Vec<dispatch_body::ScopeIdentity>>,
    resolved: HashMap<InsnId, InvokeSite>,
    caller: JavaMethod,
    rendering: JavaMethod,
    helper: String,
    escaping: Option<Vec<String>>,
    invokes: HashMap<InsnId, InvokeSite>,
    throws: HashMap<InsnId, (ValueId, JavaType)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Sites,
    Body,
    DispatchBody,
}

fn same_method(a: &JavaMethod, b: &JavaMethod) -> bool {
    a.owner == b.owner
        && a.name == b.name
        && a.is_static == b.is_static
        && a.parameters == b.parameters
        && a.return_type == b.return_type
        && a.throws == b.throws
}

fn failure(message: &str) -> Unsupported {
    Unsupported::Metadata(format!("checked source bridge: {message}"))
}

fn checked_domain(name: &str, escaping: &[String], r: &impl JavaResolver) -> bool {
    let actual = JavaType::Class(name.into());
    r.assignable(&actual, &JavaType::Class("java.lang.Exception".into()))
        && escaping
            .iter()
            .any(|base| name == base || r.assignable(&actual, &JavaType::Class(base.clone())))
}

fn source_exception(
    name: &str,
    escaping: Option<&[String]>,
    resolver: &impl JavaResolver,
    require_domain: bool,
) -> bool {
    resolver.assignable(
        &JavaType::Class(name.into()),
        &JavaType::Class("java.lang.Exception".into()),
    ) && (!require_domain || escaping.is_some_and(|e| checked_domain(name, e, resolver)))
}

// Object expression plans consume only normal successors of their allocation.
// The union of these successor regions excludes every folded argument/pure/
// sequence site, including loops; arena order is never used as execution order.
fn allocation_reachable_instructions(
    ir: &MethodIr<SsaForm>,
) -> Result<HashSet<InsnId>, Unsupported> {
    let mut blocks = HashMap::new();
    for id in ir.cfg().block_ids() {
        let block = ir.cfg().block(id).map_err(|_| failure("CFG block"))?;
        if blocks.insert(*block.payload(), id).is_some() {
            return Err(failure("ambiguous instruction block"));
        }
    }
    let mut queue = Vec::new();
    for (id, insn) in ir.instructions() {
        if matches!(insn, ManagedInsn::NewInstance { .. }) {
            queue.push(
                *blocks
                    .get(&id)
                    .ok_or_else(|| failure("allocation outside CFG"))?,
            );
        }
    }
    let mut visited = HashSet::new();
    let mut instructions = HashSet::new();
    while let Some(id) = queue.pop() {
        if !visited.insert(id) {
            continue;
        }
        let block = ir
            .cfg()
            .block(id)
            .map_err(|_| failure("allocation CFG block"))?;
        instructions.insert(*block.payload());
        for edge in block.outgoing() {
            let edge = ir
                .cfg()
                .edge(*edge)
                .map_err(|_| failure("allocation CFG edge"))?;
            if !matches!(edge.kind(), decomp_core::cfg::EdgeKind::Exception(_)) {
                queue.push(edge.target());
            }
        }
    }
    Ok(instructions)
}

fn selected_target(
    ir: &MethodIr<SsaForm>,
    caller: &JavaMethod,
    kind: InvokeKind,
    method: MethodRef,
    args: &[ValueId],
    resolver: &impl JavaResolver,
) -> Option<JavaMethod> {
    let receiver = args
        .first()
        .and_then(|v| ir.value(*v).ok())
        .and_then(|v| crate::managed_type(v.ty(), resolver).ok());
    if kind == InvokeKind::Virtual
        && let Some(receiver) = receiver.as_ref()
    {
        if let Some(target) = resolver.virtual_receiver_method(method, &caller.owner, receiver) {
            return Some(target);
        }
        if let Some(target) = resolver.array_clone_method(method, receiver) {
            return Some(target);
        }
    }
    if kind == InvokeKind::Super {
        resolver
            .super_invocation(&caller.owner, method)
            .map(|b| b.method)
    } else {
        resolver.method(method)
    }
}

/// Validate every uncovered invocation/throw before selecting the caller's
/// original throws clause. Unknown types, special initialization and exceptional
/// CFGs reject. A recipe with no necessary sites is not produced.
pub fn plan(
    ir: &MethodIr<SsaForm>,
    original: &JavaMethod,
    runtime_escaping: &[String],
    helper: &str,
    resolver: &impl JavaResolver,
) -> Result<Option<Plan>, Unsupported> {
    plan_inner(
        ir,
        original,
        Some(runtime_escaping),
        helper,
        resolver,
        true,
        Mode::Sites,
    )
}

/// Source-contract recipe independent of finite runtime-domain inference.
/// None retains explicitly unknown runtime evidence; it never means no escapes.
/// All selected Java clauses and original instruction identities remain proven.
/// Allocation recipes may occur only after, never before, an approved site.
pub fn plan_source_contract(
    ir: &MethodIr<SsaForm>,
    original: &JavaMethod,
    runtime_escaping: Option<&[String]>,
    helper: &str,
    resolver: &impl JavaResolver,
) -> Result<Option<Plan>, Unsupported> {
    plan_inner(
        ir,
        original,
        runtime_escaping,
        helper,
        resolver,
        false,
        Mode::Sites,
    )
}

/// One source-only boundary around an otherwise unchanged ordinary body.
/// Internal rendering clauses are never published as original/runtime facts.
pub fn plan_whole_body(
    ir: &MethodIr<SsaForm>,
    original: &JavaMethod,
    runtime_escaping: Option<&[String]>,
    helper: &str,
    resolver: &impl JavaResolver,
) -> Result<Option<Plan>, Unsupported> {
    plan_inner(
        ir,
        original,
        runtime_escaping,
        helper,
        resolver,
        false,
        Mode::Body,
    )
}

/// Target-bound source-only outer boundary. Every broader Throwable invocation
/// must have its own exact selected original witness; all other checks remain.
/// Callee clauses and original caller ABI are retained. Helpers add separately
/// attributed reflective/live-stack and stack-exhaustion observability.
pub fn plan_whole_body_bound_targets(
    ir: &MethodIr<SsaForm>,
    original: &JavaMethod,
    runtime_escaping: Option<&[String]>,
    helper: &str,
    resolver: &impl JavaResolver,
    targets: &[BoundTarget],
) -> Result<Option<Plan>, Unsupported> {
    plan_inner_bound(
        ir,
        original,
        runtime_escaping,
        helper,
        resolver,
        false,
        (Mode::Body, targets),
    )
}

fn plan_inner(
    ir: &MethodIr<SsaForm>,
    original: &JavaMethod,
    runtime_escaping: Option<&[String]>,
    helper: &str,
    resolver: &impl JavaResolver,
    require_domain: bool,
    mode: Mode,
) -> Result<Option<Plan>, Unsupported> {
    plan_inner_bound(
        ir,
        original,
        runtime_escaping,
        helper,
        resolver,
        require_domain,
        (mode, &[]),
    )
}

fn plan_inner_bound(
    ir: &MethodIr<SsaForm>,
    original: &JavaMethod,
    runtime_escaping: Option<&[String]>,
    helper: &str,
    resolver: &impl JavaResolver,
    require_domain: bool,
    boundary: (Mode, &[BoundTarget]),
) -> Result<Option<Plan>, Unsupported> {
    let (mode, bound_targets) = boundary;
    if original.name.starts_with('<') || helper.is_empty() {
        return Err(failure("ordinary method/helper identity"));
    }
    crate::member(helper)?;
    for edge in ir.cfg().edge_ids() {
        if mode != Mode::DispatchBody
            && ir.cfg().edge(edge).map_or(true, |e| {
                matches!(e.kind(), decomp_core::cfg::EdgeKind::Exception(_))
            })
        {
            return Err(failure("exception edges"));
        }
    }
    let mut result = Plan {
        mode,
        throwable_boundary: false,
        dispatch_scopes: None,
        resolved: HashMap::new(),
        caller: original.clone(),
        rendering: original.clone(),
        helper: helper.into(),
        escaping: runtime_escaping.map(<[String]>::to_vec),
        invokes: HashMap::new(),
        throws: HashMap::new(),
    };
    let after_allocation = if mode == Mode::Sites {
        allocation_reachable_instructions(ir)?
    } else {
        HashSet::new()
    };
    for (id, insn) in ir.instructions() {
        match insn {
            ManagedInsn::MonitorEnter { .. } | ManagedInsn::MonitorExit { .. } => {
                return Err(failure("monitor scope"));
            }
            ManagedInsn::NewInstance { .. } if require_domain => {
                return Err(failure("allocation recipe excluded"));
            }
            ManagedInsn::Invoke {
                kind,
                method,
                result: value,
                args,
            } => {
                let target = selected_target(ir, original, *kind, *method, args, resolver)
                    .ok_or_else(|| failure("callee declaration"))?;
                result.resolved.insert(
                    id,
                    InvokeSite {
                        reference: *method,
                        kind: *kind,
                        result: *value,
                        args: args.clone(),
                        target: target.clone(),
                    },
                );
                let Err(uncovered) =
                    crate::checked_exceptions::check(resolver, &target.throws, &original.throws)
                else {
                    continue;
                };
                let constructor =
                    mode != Mode::Sites && *kind == InvokeKind::Special && target.name == "<init>";
                let bound_throwable = mode != Mode::Sites
                    && *kind == InvokeKind::Static
                    && uncovered
                        .types
                        .iter()
                        .all(|name| name == "java.lang.Throwable")
                    && bound_targets
                        .iter()
                        .any(|proof| proof.matches(*method, &target));
                if (target.name.starts_with('<') && !constructor)
                    || !(matches!(
                        kind,
                        InvokeKind::Static | InvokeKind::Virtual | InvokeKind::Interface
                    ) || constructor
                        || (mode != Mode::Sites && *kind == InvokeKind::Super))
                    || uncovered.types.iter().any(|name| {
                        !source_exception(name, runtime_escaping, resolver, require_domain)
                            && !bound_throwable
                    })
                {
                    return Err(failure("unproved invocation runtime domain or dispatch"));
                }
                if after_allocation.contains(&id) {
                    return Err(failure("bridge site may follow allocation recipe"));
                }
                result.throwable_boundary |= bound_throwable;
                result.invokes.insert(
                    id,
                    InvokeSite {
                        reference: *method,
                        kind: *kind,
                        result: *value,
                        args: args.clone(),
                        target,
                    },
                );
            }
            ManagedInsn::Throw { value } if !crate::proven_null_throw(ir, *value) => {
                let ty = ir
                    .value(*value)
                    .ok()
                    .and_then(|v| crate::managed_type(v.ty(), resolver).ok())
                    .ok_or_else(|| failure("throw type"))?;
                let JavaType::Class(name) = &ty else {
                    return Err(failure("nonclass throw"));
                };
                if crate::checked_exceptions::check(
                    resolver,
                    std::slice::from_ref(name),
                    &original.throws,
                )
                .is_ok()
                {
                    continue;
                }
                if !source_exception(name, runtime_escaping, resolver, require_domain) {
                    return Err(failure("unproved throw runtime domain"));
                }
                if after_allocation.contains(&id) {
                    return Err(failure("bridge throw may follow allocation recipe"));
                }
                result.throws.insert(id, (*value, ty));
            }
            _ => {}
        }
    }
    if result.invokes.is_empty() && result.throws.is_empty() {
        return Ok(None);
    }
    result.rendering = result.compute_rendering_method();
    Ok(Some(result))
}

impl Plan {
    fn compute_rendering_method(&self) -> JavaMethod {
        let mut rendering = self.caller.clone();
        if self.mode != Mode::Sites {
            let mut clauses = rendering
                .throws
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            for site in self.invokes.values() {
                clauses.extend(site.target.throws.iter().cloned());
            }
            for (_, ty) in self.throws.values() {
                if let JavaType::Class(name) = ty {
                    clauses.insert(name.clone());
                }
            }
            rendering.throws = clauses.into_iter().collect();
        }
        rendering
    }
    fn rendering_method(&self) -> &JavaMethod {
        &self.rendering
    }
    pub(crate) fn body_original(&self, rendering: &JavaMethod) -> Option<&JavaMethod> {
        (self.mode != Mode::Sites && same_method(self.rendering_method(), rendering))
            .then_some(&self.caller)
    }
    pub(crate) fn wrap_body(
        &self,
        lines: &[String],
        catch_name: &str,
    ) -> Result<Vec<String>, Unsupported> {
        self.wrap_lines(None, lines, catch_name)
    }
    /// Whether one boundary encloses the unchanged complete body rather than sites.
    pub fn is_whole_body(&self) -> bool {
        self.mode != Mode::Sites
    }
    /// Exact original declaration chosen only after complete local validation.
    pub fn original(&self) -> &JavaMethod {
        &self.caller
    }
    /// Independent runtime evidence: None is unknown/unavailable, never an
    /// empty domain. Some(empty) is an independently proven empty domain.
    pub fn runtime_escaping(&self) -> Option<&[String]> {
        self.escaping.as_deref()
    }
    /// Number of original instruction sites requiring the synthetic recipe.
    pub fn site_count(&self) -> usize {
        self.invokes.len() + self.throws.len()
    }
    /// Name of the separately attributed same-owner compiler artifact.
    pub fn helper_name(&self) -> &str {
        &self.helper
    }
    /// Complete helper declaration; never insert an original method-ledger row.
    pub fn helper_source(&self) -> String {
        format!(
            "// Synthetic checked-source bridge; no original DEX method identity.\n@SuppressWarnings(\"unchecked\")\nprivate static <T extends java.lang.Throwable> java.lang.RuntimeException {}(java.lang.Throwable failure) throws T {{\n    throw (T) failure;\n}}\n",
            self.helper
        )
    }
    /// Validate the exact original callsite and resolved current source target.
    pub fn approves_invoke(
        &self,
        id: InsnId,
        insn: &ManagedInsn,
        target: &JavaMethod,
        caller: &JavaMethod,
    ) -> bool {
        let Some(site) = self.invokes.get(&id) else {
            return false;
        };
        same_method(&self.caller, caller)
            && same_method(&site.target, target)
            && matches!(insn, ManagedInsn::Invoke {kind,method,result,args} if *kind==site.kind && *method==site.reference && *result==site.result && *args==site.args)
    }
    pub(crate) fn approves_rendering_invoke(
        &self,
        id: InsnId,
        insn: &ManagedInsn,
        target: &JavaMethod,
        rendering: &JavaMethod,
    ) -> bool {
        self.body_original(rendering).is_some() && self.resolved.get(&id).is_some_and(|site| {
            same_method(&site.target, target) && matches!(insn, ManagedInsn::Invoke {kind,method,result,args}
                if *kind == site.kind && *method == site.reference && *result == site.result && *args == site.args)
        })
    }
    pub(crate) fn approves_throw(
        &self,
        id: InsnId,
        value: ValueId,
        ty: &JavaType,
        caller: &JavaMethod,
    ) -> bool {
        same_method(&self.caller, caller)
            && self
                .throws
                .get(&id)
                .is_some_and(|(v, t)| *v == value && t == ty)
    }
    pub(crate) fn rethrow(&self, expression: &str) -> Result<String, Unsupported> {
        // Type-context qualification also survives package roots shadowed by
        // genuine instance fields. Null is not evaluated as an instance receiver.
        Ok(format!(
            "throw (({}) null).<java.lang.RuntimeException>{}({expression});",
            crate::qualified(&self.caller.owner)?,
            self.helper
        ))
    }
    pub(crate) fn wrap_lines(
        &self,
        declaration: Option<&str>,
        lines: &[String],
        catch_name: &str,
    ) -> Result<Vec<String>, Unsupported> {
        let mut out = Vec::new();
        if let Some(line) = declaration {
            out.push(line.into());
        }
        out.push("try {".into());
        out.extend(lines.iter().map(|s| format!("    {s}")));
        let catch_type = if self.throwable_boundary {
            "java.lang.Throwable"
        } else {
            "java.lang.Exception"
        };
        out.push(format!("}} catch ({catch_type} {catch_name}) {{"));
        out.push(format!("    {}", self.rethrow(catch_name)?));
        out.push("}".into());
        Ok(out)
    }
}

struct Local<'a, R> {
    inner: &'a R,
    plan: &'a Plan,
}
impl<R: JavaResolver> JavaResolver for Local<'_, R> {
    fn checked_source_bridge(&self, caller: &JavaMethod) -> Option<&Plan> {
        (self.plan.mode == Mode::Sites && same_method(caller, &self.plan.caller))
            .then_some(self.plan)
    }
    fn checked_source_body_bridge(&self, caller: &JavaMethod) -> Option<&Plan> {
        self.plan.body_original(caller).map(|_| self.plan)
    }

    fn exception_allocation_chain(&self, c: &[InsnId]) -> bool {
        self.inner.exception_allocation_chain(c)
    }
    fn exception_allocation_sequence_chain(&self, c: &[InsnId]) -> bool {
        self.inner.exception_allocation_sequence_chain(c)
    }
    fn exception_allocation_pure_chain(&self, c: &[InsnId]) -> bool {
        self.inner.exception_allocation_pure_chain(c)
    }
    fn trusted_string_builder_class(&self, r: ClassRef) -> bool {
        self.inner.trusted_string_builder_class(r)
    }
    fn java_super_selects_dex_target(&self, c: &str, r: MethodRef) -> bool {
        self.inner.java_super_selects_dex_target(c, r)
    }
    fn super_invocation(&self, c: &str, r: MethodRef) -> Option<SuperInvocation> {
        self.inner.super_invocation(c, r)
    }
    fn retain_method_receiver(&self, r: MethodRef, c: &str, t: &JavaType) -> bool {
        self.inner.retain_method_receiver(r, c, t)
    }
    fn virtual_receiver_method(&self, r: MethodRef, c: &str, t: &JavaType) -> Option<JavaMethod> {
        self.inner.virtual_receiver_method(r, c, t)
    }
    fn field_access(&self, r: FieldRef) -> Option<u32> {
        self.inner.field_access(r)
    }
    fn static_method_type_context(&self, r: MethodRef, c: &str) -> bool {
        self.inner.static_method_type_context(r, c)
    }
    fn static_method_owner_access(&self, r: MethodRef) -> Option<u32> {
        self.inner.static_method_owner_access(r)
    }
    fn static_field_type_context(&self, r: FieldRef, c: &str) -> bool {
        self.inner.static_field_type_context(r, c)
    }
    fn field_source_nonconstant(&self, r: FieldRef) -> bool {
        self.inner.field_source_nonconstant(r)
    }
    fn retain_field_receiver(&self, r: FieldRef, c: &str, t: &JavaType) -> bool {
        self.inner.retain_field_receiver(r, c, t)
    }
    fn discarded_invoke_source_target(&self, r: MethodRef) -> bool {
        self.inner.discarded_invoke_source_target(r)
    }
    fn method_access(&self, r: MethodRef) -> Option<u32> {
        self.inner.method_access(r)
    }
    fn assignable(&self, a: &JavaType, b: &JavaType) -> bool {
        self.inner.assignable(a, b)
    }
    fn instruction_type(&self, r: TypeRef) -> Option<JavaType> {
        self.inner.instruction_type(r)
    }
    fn array_clone_method(&self, r: MethodRef, t: &JavaType) -> Option<JavaMethod> {
        self.inner.array_clone_method(r, t)
    }
    fn method(&self, r: MethodRef) -> Option<JavaMethod> {
        self.inner.method(r)
    }
    fn field(&self, r: FieldRef) -> Option<JavaField> {
        self.inner.field(r)
    }
    fn class(&self, r: ClassRef) -> Option<String> {
        self.inner.class(r)
    }
    fn string(&self, r: StringRef) -> Option<Vec<u16>> {
        self.inner.string(r)
    }
}

/// Emit transactionally with the original caller clause and current callee
/// clauses. The caller inserts the separately returned helper only on success.
pub fn emit(
    ir: &MethodIr<SsaForm>,
    bindings: &DeclarationBindings,
    resolver: &impl JavaResolver,
    plan: &Plan,
) -> SourceResult {
    if plan.mode == Mode::DispatchBody {
        return SourceResult::Unsupported(failure("dispatch body requires original metadata"));
    }
    if let Err(error) = validate_ownership(ir, resolver, plan) {
        return SourceResult::Unsupported(error);
    }
    crate::emit_method_declaration(
        ir,
        plan.rendering_method(),
        bindings,
        &Local {
            inner: resolver,
            plan,
        },
    )
}

fn validate_ownership(
    ir: &MethodIr<SsaForm>,
    resolver: &impl JavaResolver,
    plan: &Plan,
) -> Result<(), Unsupported> {
    let blocked = if plan.mode == Mode::Sites {
        allocation_reachable_instructions(ir)
    } else {
        Ok(HashSet::new())
    }?;
    let current_invokes: HashSet<_> = ir
        .instructions()
        .iter()
        .filter_map(|(id, insn)| matches!(insn, ManagedInsn::Invoke { .. }).then_some(id))
        .collect();
    if current_invokes.len() != plan.resolved.len()
        || current_invokes.iter().any(|id| !plan.resolved.contains_key(id))
        || plan.resolved.iter().any(|(id, site)| {
        (plan.invokes.contains_key(id) && blocked.contains(id))
            || selected_target(ir, &plan.caller, site.kind, site.reference, &site.args, resolver)
                .is_none_or(|current| !same_method(&current, &site.target))
            || !ir.instruction(*id).is_ok_and(|insn| {
            matches!(insn, ManagedInsn::Invoke { kind, method, result, args }
                if *kind == site.kind && *method == site.reference && *result == site.result && *args == site.args)
        })
    }) || plan.throws.iter().any(|(id, (value, _))| {
        blocked.contains(id) || !ir.instruction(*id).is_ok_and(|insn| {
            matches!(insn, ManagedInsn::Throw { value: actual } if actual == value)
        })
    }) {
        return Err(failure("bridge ownership changed"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "checked_source_bridge_cache_tests.rs"]
mod cache_tests;
