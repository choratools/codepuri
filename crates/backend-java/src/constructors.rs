//! Explicit constructor lowering and atomic, metadata-proven family planning.
use super::*;
use decomp_core::ids::BlockId;
#[path = "constructor_arguments_guards.rs"]
mod argument_guards;
#[path = "constructor_arguments.rs"]
mod arguments;
#[path = "constructor_exception_suffix.rs"]
mod exception_suffix;
#[path = "constructor_initialized_suffix.rs"]
mod initialized_suffix;
#[path = "constructor_mask_choices.rs"]
pub mod mask_choices;
#[path = "constructor_prefix_diamonds.rs"]
mod prefix_diamonds;
#[path = "constructors_source25.rs"]
mod source25;
#[path = "constructor_suffix_control.rs"]
mod suffix_control;

/// An explicitly resolved initializer, never inferred from an ordinary method.
#[derive(Clone, Debug)]
pub struct Initializer {
    /// Must be a non-static, void `<init>` signature.
    pub method: JavaMethod,
    /// For same-class targets, the complete, caller-verified `this` delegation
    /// chain. `None` means this target terminates by calling its superclass.
    pub delegates_to: Option<MethodRef>,
}

/// Exact source scopes associated with the original constructor IR.
pub trait ConstructorExceptionMetadata {
    /// Original scopes, including source catch order and instruction offsets.
    fn scopes(&self) -> Vec<&dyn crate::exceptions::RegionMetadata>;
}

/// Additional metadata required for constructor lowering.
pub trait ConstructorResolver: JavaResolver {
    /// Explicit initializer mapping. Missing entries fail closed.
    fn initializer(&self, reference: MethodRef) -> Option<Initializer>;
    /// Return source scopes only for this exact original constructor IR.
    fn constructor_exception_metadata(
        &self,
        _reference: MethodRef,
        _ir: &MethodIr<SsaForm>,
    ) -> Option<Box<dyn ConstructorExceptionMetadata + '_>> {
        None
    }
    /// Attest an exact genuine non-final field store with legal source access.
    fn constructor_exception_field_store(&self, _field: FieldRef) -> bool {
        false
    }
    /// Explicit requested Java source level. Java25 preserves original
    /// constructor prologues; legacy callers retain Java17 lowering.
    fn constructor_source_level(&self) -> u16 {
        17
    }
    /// Attest this exact own instance field will be emitted without a field
    /// declaration initializer. Java25 early assignment requires that source
    /// condition even for non-final fields; unknown publication fails closed.
    fn constructor_early_field_store(&self, _field: FieldRef) -> bool {
        false
    }
    /// Attest equality of the original DEX invoke-super target and Java's
    /// descriptor-pinned `super` selection. This includes genuine declaration,
    /// access, hierarchy and frozen-name evidence; a pool reference is not proof.
    fn constructor_super_target(&self, _caller: &str, _reference: MethodRef) -> bool {
        false
    }
    /// Return an inert-chain proof for these exact original initializer
    /// identities. The resolver must verify that `proof.caller()` is the
    /// original declaring descriptor of `caller`, and `proof.chain()[0]` is
    /// exactly `target`, before source aliases are applied. Missing original
    /// declarations, complete bodies, finalizer audits or Object root evidence
    /// must return None. The backend separately proves each relocated operation.
    fn constructor_prelude_chain(
        &self,
        _caller: MethodRef,
        _target: MethodRef,
    ) -> Option<constructor_prechecks::InertChainProof> {
        None
    }
    /// Override only with trusted checked-exception hierarchy evidence.
    fn exception_covered(&self, exception: &str, declared: &[String]) -> bool {
        declared.iter().any(|t| t == exception)
    }
}

/// Declaration evidence required by the strict family planner. Pool references
/// alone must never be supplied as declarations.
#[derive(Clone, Debug)]
pub struct ConstructorClass {
    /// Frozen source class name.
    pub name: String,
    /// Exact direct-super declaration, or None for Object.
    pub superclass: Option<String>,
    /// Original declaration access flags.
    pub access_flags: u32,
}

/// Trusted input/classpath metadata used for atomic constructor planning.
pub trait TrustedConstructorResolver: ConstructorResolver {
    /// Return a declaration, never an inferred pool-only class.
    fn constructor_class(&self, name: &str) -> Option<ConstructorClass>;
    /// Original flags for this exact constructor overload.
    fn constructor_access(&self, reference: MethodRef) -> Option<u32>;
    /// Complete declared initializer set; not merely the retained methods.
    fn declared_constructors(&self, owner: &str) -> Option<Vec<MethodRef>>;
}

/// Signature-preserving bindings accepted by original constructor family inputs.
pub trait ConstructorBindings {
    /// Preserve original signature arity, with absent values for unused inputs.
    fn declaration_bindings(&self) -> DeclarationBindings;
}
impl ConstructorBindings for ParameterBindings {
    fn declaration_bindings(&self) -> DeclarationBindings {
        DeclarationBindings {
            receiver: self.receiver,
            parameters: self.parameters.iter().copied().map(Some).collect(),
            names: self.names.clone(),
        }
    }
}
impl ConstructorBindings for DeclarationBindings {
    fn declaration_bindings(&self) -> DeclarationBindings {
        self.clone()
    }
}

/// One retained constructor and its validated SSA input.
pub struct ConstructorInput<'a> {
    /// DEX-local initializer identity.
    pub reference: MethodRef,
    /// Original constructor body.
    pub ir: &'a MethodIr<SsaForm>,
    /// Exact incoming receiver and parameters.
    pub bindings: &'a dyn ConstructorBindings,
}

/// Caller supplies complete inventory-proven write counts for this field,
/// keyed by initializer reference in this resolver's identity domain. Any write
/// outside these constructors must cause the caller to reject the class first.
pub struct BlankFinal {
    /// Exact declared instance-final field identity.
    pub reference: FieldRef,
    /// Complete write counts from the whole-input inventory.
    pub writes: HashMap<MethodRef, usize>,
}

/// Path-sensitive local evidence, independent of whole-input write exclusivity
/// and of whether the source emitter supports this graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstructorFieldEvidence {
    /// Exact field in this IR's reference domain.
    pub reference: FieldRef,
    /// Reachable writes before successful superclass/this initialization.
    pub before_chain: HashSet<InsnId>,
    /// Reachable writes after successful initialization.
    pub after_chain: HashSet<InsnId>,
    /// Local successful assignment counts at normal returns (2 means 2 or more).
    pub normal_return_counts: std::collections::BTreeSet<u8>,
    /// A read through this precedes local assignment or a completed this call.
    /// This blocks blank-final definite assignment; non-final default reads
    /// remain valid and must not be rejected solely on this diagnostic flag.
    pub read_before_assignment: bool,
}

/// Local constructor effects. An empty return set is not a completeness proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstructorInitializationEvidence {
    /// All fields requested by the caller, including fields never mentioned.
    pub fields: Vec<ConstructorFieldEvidence>,
    /// Exact successful initializer target at each normal return.
    pub normal_chains: HashSet<(ChainKind, MethodRef)>,
    /// Exceptional edges were traversed with the pre-instruction state.
    pub has_exception_paths: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InitializationState {
    aliases: HashSet<ValueId>,
    counts: Vec<u8>,
    chain: Option<(ChainKind, MethodRef)>,
}

/// Collect effects over every reachable normal and catch path. Failed invokes
/// and failed field stores propagate their incoming state to exception edges.
/// Saturating counts make loops finite; an analysis budget fails closed rather
/// than silently dropping paths. No assignment is moved across the chain call.
pub fn constructor_initialization_evidence<R: ConstructorResolver>(
    input: &ConstructorInput<'_>,
    direct_super: &str,
    fields: &[FieldRef],
    resolver: &R,
) -> Result<ConstructorInitializationEvidence, Unsupported> {
    let bindings = input.bindings.declaration_bindings();
    let mut captures = HashSet::new();
    if fields.is_empty()
        && let Some(metadata) = resolver.constructor_exception_metadata(input.reference, input.ir)
    {
        // The real declaration planner first proves the untouched chain and
        // exact suffix. Capture evidence never substitutes for that proof.
        plan_constructor_declaration(input.ir, input.reference, direct_super, &bindings, resolver)?;
        let scopes = metadata.scopes();
        let bound = bindings
            .parameters
            .iter()
            .flatten()
            .copied()
            .chain(bindings.receiver)
            .collect::<Vec<_>>();
        let declarations = contextual_locals::declaration_plan(input.ir, &JavaType::Void, resolver)
            .map_err(|error| {
                Unsupported::Type(format!("constructor capture declarations: {error:?}"))
            })?;
        let plan =
            crate::exceptions::dispatch::plan(input.ir, &scopes, &bound, resolver, &declarations)
                .map_err(|error| Unsupported::ControlFlow(error.to_string()))?;
        captures.extend(plan.capture_blocks);
    }
    receiver_initialization_evidence(
        input.ir,
        input.reference,
        bindings.receiver,
        direct_super,
        fields,
        resolver,
        &captures,
    )
}

/// Receiver-only diagnostic entrypoint. Unused descriptor parameters need no
/// fabricated SSA definitions: local field effects do not depend on binding
/// unused inputs. This is not a constructor declaration or family proof.
pub fn constructor_receiver_initialization_evidence<R: ConstructorResolver>(
    ir: &MethodIr<SsaForm>,
    reference: MethodRef,
    receiver: Option<ValueId>,
    direct_super: &str,
    fields: &[FieldRef],
    resolver: &R,
) -> Result<ConstructorInitializationEvidence, Unsupported> {
    receiver_initialization_evidence(
        ir,
        reference,
        receiver,
        direct_super,
        fields,
        resolver,
        &HashSet::new(),
    )
}

fn receiver_initialization_evidence<R: ConstructorResolver>(
    ir: &MethodIr<SsaForm>,
    reference: MethodRef,
    receiver: Option<ValueId>,
    direct_super: &str,
    fields: &[FieldRef],
    resolver: &R,
    captures: &HashSet<decomp_core::ids::BlockId>,
) -> Result<ConstructorInitializationEvidence, Unsupported> {
    let fail = || Unsupported::Metadata("constructor initialization evidence unavailable".into());
    let declaration = resolver.initializer(reference).ok_or_else(fail)?;
    init_signature(&declaration.method)?;
    let receiver = receiver.ok_or_else(fail)?;
    let receiver_def = ir.value(receiver).map_err(|_| fail())?.def();
    if !matches!(ir.instruction(receiver_def).map_err(|_| fail())?,
        ManagedInsn::Opaque { reads, writes, effects, .. }
        if reads.is_empty() && writes.contains(&receiver) && *effects == managed_ir::effects::EffectFlags::NONE)
    {
        return Err(fail());
    }
    let cfg = ir.cfg();
    cfg.validate().map_err(|_| fail())?;
    let mut unique = HashSet::new();
    let mut evidence = ConstructorInitializationEvidence {
        fields: fields
            .iter()
            .map(|reference| ConstructorFieldEvidence {
                reference: *reference,
                before_chain: HashSet::new(),
                after_chain: HashSet::new(),
                normal_return_counts: std::collections::BTreeSet::new(),
                read_before_assignment: false,
            })
            .collect(),
        normal_chains: HashSet::new(),
        has_exception_paths: false,
    };
    for field in fields {
        let f = resolver.field(*field).ok_or_else(fail)?;
        if !unique.insert(*field) || f.owner != declaration.method.owner || f.is_static {
            return Err(fail());
        }
    }
    let initial = InitializationState {
        aliases: HashSet::from([receiver]),
        counts: vec![0; fields.len()],
        chain: None,
    };
    let mut pending = std::collections::VecDeque::from([(cfg.entry(), initial, None)]);
    let mut visited =
        HashMap::<_, Vec<(InitializationState, Option<decomp_core::ids::EdgeId>)>>::new();
    let mut budget = 0;
    while let Some((block, incoming, selected_edge)) = pending.pop_front() {
        let states = visited.entry(block).or_default();
        if states.contains(&(incoming.clone(), selected_edge)) {
            continue;
        }
        budget += 1;
        if budget > 16384 {
            return Err(fail());
        }
        states.push((incoming.clone(), selected_edge));
        let node = cfg.block(block).map_err(|_| fail())?;
        let id = *node.payload();
        let insn = ir.instruction(id).map_err(|_| fail())?;
        let mut outgoing = incoming.clone();
        match insn {
            ManagedInsn::Opaque { effects, .. }
                if *effects != managed_ir::effects::EffectFlags::NONE
                    && !captures.contains(&block) =>
            {
                return Err(fail());
            }
            ManagedInsn::Move { result, value } => {
                if incoming.aliases.contains(value) {
                    outgoing.aliases.insert(*result);
                } else {
                    outgoing.aliases.remove(result);
                }
            }
            ManagedInsn::Phi { result, inputs } => {
                // Follow the exact original predecessor through the complete
                // simultaneous Phi cluster. A branch-local this copy need not
                // be available on another branch; every path is checked itself.
                let Some(selected_edge) = selected_edge else {
                    return Err(fail());
                };
                let selected: Vec<_> = inputs
                    .iter()
                    .filter(|(edge, _)| *edge == selected_edge)
                    .collect();
                if selected.len() != 1 {
                    return Err(fail());
                }
                if incoming.aliases.contains(&selected[0].1) {
                    outgoing.aliases.insert(*result);
                } else {
                    outgoing.aliases.remove(result);
                }
            }
            ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                method,
                args,
                result: None,
            } if args.first().is_some_and(|v| incoming.aliases.contains(v)) => {
                if let Some(target) = resolver.initializer(*method) {
                    init_signature(&target.method)?;
                    if incoming.chain.is_some() {
                        return Err(fail());
                    }
                    let chain = if target.method.owner == declaration.method.owner {
                        ChainKind::This
                    } else if target.method.owner == direct_super {
                        ChainKind::Super
                    } else {
                        return Err(fail());
                    };
                    let args = invoke_arguments::operands(&target.method, args)?;
                    if args.len() != target.method.parameters.len() + 1 {
                        return Err(fail());
                    }
                    outgoing.chain = Some((chain, *method));
                }
            }
            ManagedInsn::FieldPut { field, object, .. } if fields.contains(field) => {
                if !object.is_some_and(|v| incoming.aliases.contains(&v)) {
                    return Err(fail());
                }
                let index = fields.iter().position(|f| f == field).ok_or_else(fail)?;
                let sites = if incoming.chain.is_some() {
                    &mut evidence.fields[index].after_chain
                } else {
                    &mut evidence.fields[index].before_chain
                };
                sites.insert(id);
                outgoing.counts[index] = (outgoing.counts[index] + 1).min(2);
            }
            ManagedInsn::FieldGet { field, object, .. }
                if fields.contains(field)
                    && object.is_some_and(|v| incoming.aliases.contains(&v)) =>
            {
                let index = fields.iter().position(|f| f == field).ok_or_else(fail)?;
                if incoming.counts[index] == 0
                    && !matches!(incoming.chain, Some((ChainKind::This, _)))
                {
                    evidence.fields[index].read_before_assignment = true;
                }
            }
            ManagedInsn::Return { value: None } => {
                if !node.outgoing().is_empty() {
                    return Err(fail());
                }
                evidence
                    .normal_chains
                    .insert(incoming.chain.ok_or_else(fail)?);
                for (field, count) in evidence.fields.iter_mut().zip(&incoming.counts) {
                    field.normal_return_counts.insert(*count);
                }
            }
            ManagedInsn::Return { .. } => return Err(fail()),
            _ => {}
        }
        for &edge_id in node.outgoing() {
            let edge = cfg.edge(edge_id).map_err(|_| fail())?;
            let state = if matches!(edge.kind(), EdgeKind::Exception(_)) {
                evidence.has_exception_paths = true;
                incoming.clone()
            } else {
                outgoing.clone()
            };
            let target = cfg.block(edge.target()).map_err(|_| fail())?;
            let continuation = if matches!(insn, ManagedInsn::Phi { .. })
                && edge.kind() == &EdgeKind::Unconditional
                && matches!(
                    ir.instruction(*target.payload()).map_err(|_| fail())?,
                    ManagedInsn::Phi { .. }
                ) {
                selected_edge
            } else {
                Some(edge_id)
            };
            pending.push_back((edge.target(), state, continuation));
        }
    }
    Ok(evidence)
}

/// Atomic class constructor planning: complete this chains, exact hierarchy,
/// trusted access/throws, and one assignment per final along every chain.
pub fn plan_constructor_family<R: TrustedConstructorResolver>(
    owner: &str,
    inputs: &[ConstructorInput<'_>],
    finals: &[BlankFinal],
    resolver: &R,
) -> Result<Vec<ConstructorPlan>, Unsupported> {
    plan_constructor_group(owner, inputs, finals, resolver, true)
}

/// Exact original identity paired with its reachable, locally proved body.
/// This does not attest global field-write exclusivity or complete class initialization.
#[derive(Debug)]
pub struct ReachableConstructor {
    /// Original initializer identity in the resolver's domain.
    pub reference: MethodRef,
    /// Exact original body and constructor-chain plan.
    pub plan: ConstructorPlan,
}

/// Stage at which a reachable constructor chain was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReachableRejectionPhase {
    /// The attested `this` chain repeats an original identity.
    Cycle,
    /// An attested `this` target has no supplied original body.
    MissingInput,
    /// An attempted identity has no initializer metadata.
    MissingMetadata,
    /// The complete attempted chain failed existing group validation.
    ChainValidation,
}

/// Chain-level rejection evidence; this is not an instruction-level root failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReachableConstructorRejection {
    /// Original root whose complete reachable chain was attempted.
    pub reference: MethodRef,
    /// Attempted identities in order, including a missing or repeated target.
    pub attempted_chain: Vec<MethodRef>,
    /// Exact walk or validation stage.
    pub phase: ReachableRejectionPhase,
    /// Missing or repeated target, when the walk did not complete.
    pub missing_target: Option<MethodRef>,
    /// Existing typed group rejection, only for chain validation.
    pub error: Option<Unsupported>,
}

/// Retained bodies and rejected roots in original input order.
/// Neither collection attests global initialization or field-write exclusivity.
#[derive(Debug)]
pub struct ReachableConstructorReport {
    /// Same bodies and order as the legacy reachable-subset API.
    pub retained: Vec<ReachableConstructor>,
    /// Only roots absent from the final retained set.
    pub rejected: Vec<ReachableConstructorRejection>,
}

/// Recover available constructor chains without requiring unrelated overload bodies.
/// Every retained `this` target must itself be retained and terminate in a proved
/// direct-super call. The complete original declaration catalog is still checked.
/// Blank-final counts remain caller-supplied evidence; this API grants no global
/// initialization or whole-class completeness token.
pub fn plan_constructor_reachable_subset<R: TrustedConstructorResolver>(
    owner: &str,
    inputs: &[ConstructorInput<'_>],
    finals: &[BlankFinal],
    resolver: &R,
) -> Result<Vec<ReachableConstructor>, Unsupported> {
    Ok(plan_constructor_reachable_subset_detailed(owner, inputs, finals, resolver)?.retained)
}

/// Plan the same reachable subset with chain-level diagnostics, without extra
/// inference or validation attempts. Catalog errors remain atomic API errors.
pub fn plan_constructor_reachable_subset_detailed<R: TrustedConstructorResolver>(
    owner: &str,
    inputs: &[ConstructorInput<'_>],
    finals: &[BlankFinal],
    resolver: &R,
) -> Result<ReachableConstructorReport, Unsupported> {
    // Validate the complete catalog and duplicate input identities even if no
    // individual body can be lowered. Empty groups deliberately skip body gates.
    plan_constructor_group(owner, &[], finals, resolver, false)?;
    let fail = || Unsupported::Metadata("untrusted constructor subset".into());
    let mut by_ref = HashMap::new();
    for input in inputs {
        if by_ref.insert(input.reference, input).is_some() {
            return Err(fail());
        }
    }
    let declared: HashSet<_> = resolver
        .declared_constructors(owner)
        .ok_or_else(fail)?
        .into_iter()
        .collect();
    if by_ref.keys().any(|r| !declared.contains(r)) {
        return Err(fail());
    }
    let mut accepted = HashMap::new();
    let mut rejected = Vec::new();
    for input in inputs {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut next = Some(input.reference);
        let mut failure = None;
        let mut attempted_chain = Vec::new();
        while let Some(reference) = next {
            attempted_chain.push(reference);
            if !seen.insert(reference) {
                failure = Some((ReachableRejectionPhase::Cycle, reference));
                break;
            }
            let Some(original) = by_ref.get(&reference) else {
                failure = Some((ReachableRejectionPhase::MissingInput, reference));
                break;
            };
            let Some(metadata) = resolver.initializer(reference) else {
                failure = Some((ReachableRejectionPhase::MissingMetadata, reference));
                break;
            };
            chain.push(ConstructorInput {
                reference,
                ir: original.ir,
                bindings: original.bindings,
            });
            next = metadata.delegates_to;
        }
        if let Some((phase, target)) = failure {
            rejected.push(ReachableConstructorRejection {
                reference: input.reference,
                attempted_chain,
                phase,
                missing_target: Some(target),
                error: None,
            });
            continue;
        }
        match plan_constructor_group(owner, &chain, finals, resolver, false) {
            Ok(plans) => {
                for (original, plan) in chain.iter().zip(plans) {
                    accepted.insert(original.reference, plan);
                }
            }
            Err(error) => rejected.push(ReachableConstructorRejection {
                reference: input.reference,
                attempted_chain,
                phase: ReachableRejectionPhase::ChainValidation,
                missing_target: None,
                error: Some(error),
            }),
        }
    }
    rejected.retain(|failure| !accepted.contains_key(&failure.reference));
    let retained = inputs
        .iter()
        .filter_map(|input| {
            accepted
                .remove(&input.reference)
                .map(|plan| ReachableConstructor {
                    reference: input.reference,
                    plan,
                })
        })
        .collect();
    Ok(ReachableConstructorReport { retained, rejected })
}

fn plan_constructor_group<R: TrustedConstructorResolver>(
    owner: &str,
    inputs: &[ConstructorInput<'_>],
    finals: &[BlankFinal],
    resolver: &R,
    require_complete: bool,
) -> Result<Vec<ConstructorPlan>, Unsupported> {
    let fail = || Unsupported::Metadata("untrusted constructor family".into());
    let class = resolver.constructor_class(owner).ok_or_else(fail)?;
    if class.name != owner || class.access_flags & 0x200 != 0 {
        return Err(fail());
    }
    let direct = class.superclass.as_deref().ok_or_else(fail)?;
    let package = |n: &str| {
        n.rsplit_once('.')
            .map(|(p, _)| p.to_string())
            .unwrap_or_default()
    };
    let mut visited = HashSet::from([owner.to_string()]);
    let mut next = Some(direct.to_string());
    while let Some(name) = next {
        if !visited.insert(name.clone()) {
            return Err(fail());
        }
        let c = resolver.constructor_class(&name).ok_or_else(fail)?;
        if c.name != name
            || c.access_flags & (2 | 4 | 0x10 | 0x200 | 0x2000 | 0x4000) != 0
            || (c.access_flags & 1 == 0 && package(&name) != package(owner))
        {
            return Err(fail());
        }
        if name == "java.lang.Object" {
            if c.superclass.is_some() {
                return Err(fail());
            }
            next = None;
        } else {
            next = Some(c.superclass.ok_or_else(fail)?);
        }
    }
    let mut by_ref = HashMap::new();
    for input in inputs {
        if by_ref.insert(input.reference, input).is_some() {
            return Err(fail());
        }
        let m = resolver.initializer(input.reference).ok_or_else(fail)?;
        if m.method.owner != owner {
            return Err(fail());
        }
    }
    let declared = resolver.declared_constructors(owner).ok_or_else(fail)?;
    let declared_set = declared.iter().copied().collect::<HashSet<_>>();
    if declared_set.len() != declared.len()
        || by_ref.keys().any(|r| !declared_set.contains(r))
        || (require_complete && declared_set != by_ref.keys().copied().collect())
    {
        return Err(fail());
    }
    let mut signatures = HashSet::new();
    for reference in &declared {
        let m = resolver.initializer(*reference).ok_or_else(fail)?;
        init_signature(&m.method)?;
        let flags = resolver.constructor_access(*reference).ok_or_else(fail)?;
        if m.method.owner != owner
            || (flags & 7).count_ones() > 1
            || flags & !(1 | 2 | 4 | 0x80 | 0x1000 | 0x10000) != 0
        {
            return Err(fail());
        }
        let signature = m
            .method
            .parameters
            .iter()
            .map(JavaType::source)
            .collect::<Result<Vec<_>, _>>()?;
        if !signatures.insert(signature) {
            return Err(fail());
        }
    }
    if !require_complete {
        let mut seen = HashSet::new();
        for field in finals {
            let declaration = resolver.field(field.reference).ok_or_else(fail)?;
            if !seen.insert(field.reference)
                || declaration.owner != owner
                || declaration.is_static
                || declaration.ty == JavaType::Void
                || field.writes.keys().any(|r| !declared_set.contains(r))
            {
                return Err(fail());
            }
            member(&declaration.name)?;
        }
    }
    if inputs.is_empty() {
        return if require_complete {
            Err(fail())
        } else {
            Ok(Vec::new())
        };
    }
    let mut plans = Vec::new();
    for input in inputs {
        let declaration = resolver.initializer(input.reference).ok_or_else(fail)?;
        let mut p = plan_constructor_declaration(
            input.ir,
            input.reference,
            direct,
            &input.bindings.declaration_bindings(),
            resolver,
        )?;
        let target = resolver.initializer(p.target).ok_or_else(fail)?;
        let access = resolver.constructor_access(p.target).ok_or_else(fail)?;
        if access & !(1 | 2 | 4 | 0x80 | 0x1000 | 0x10000) != 0
            || (access & 7).count_ones() > 1
            || (target.method.owner != owner
                && (access & 2 != 0
                    || (access & 5 == 0 && package(&target.method.owner) != package(owner))))
            || target
                .method
                .throws
                .iter()
                .any(|t| !resolver.exception_covered(t, &declaration.method.throws))
        {
            return Err(fail());
        }
        if p.chain == ChainKind::This && !by_ref.contains_key(&p.target) {
            return Err(fail());
        }
        let own_access = resolver
            .constructor_access(input.reference)
            .ok_or_else(fail)?;
        if (own_access & 7).count_ones() > 1
            || own_access & !(1 | 2 | 4 | 0x80 | 0x1000 | 0x10000) != 0
        {
            return Err(fail());
        }
        let prefix = match own_access & 7 {
            1 => "public ",
            2 => "private ",
            4 => "protected ",
            _ => "",
        };
        p.source = p.source.replacen("public ", prefix, 1);
        plans.push(p);
    }
    let mut allocation_defaults = Vec::new();
    let mut field_seen = HashSet::new();
    let references = finals.iter().map(|f| f.reference).collect::<Vec<_>>();
    let effects = inputs
        .iter()
        .map(|input| constructor_initialization_evidence(input, direct, &references, resolver))
        .collect::<Result<Vec<_>, _>>()?;
    let plan_indices = inputs
        .iter()
        .enumerate()
        .map(|(i, input)| (input.reference, i))
        .collect::<HashMap<_, _>>();
    // Validate the emitted graph, not only a resolver's delegation annotation.
    for (index, plan) in plans.iter().enumerate() {
        if effects[index].normal_chains != HashSet::from([(plan.chain, plan.target)]) {
            return Err(fail());
        }
        let mut current = index;
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(current) {
                return Err(fail());
            }
            if plans[current].chain == ChainKind::Super {
                break;
            }
            current = *plan_indices.get(&plans[current].target).ok_or_else(fail)?;
        }
    }
    for (field_index, final_field) in finals.iter().enumerate() {
        if !field_seen.insert(final_field.reference) {
            return Err(fail());
        }
        let f = resolver.field(final_field.reference).ok_or_else(fail)?;
        if f.owner != owner || f.is_static || f.ty == JavaType::Void {
            return Err(fail());
        }
        member(&f.name)?;
        if final_field.writes.keys().any(|r| {
            if require_complete {
                !by_ref.contains_key(r)
            } else {
                !declared_set.contains(r)
            }
        }) {
            return Err(fail());
        }
        // A source-only allocation-default recipe, not an original DEX store.
        // Require complete inventory and every constructor normal path zero.
        let allocation_default = require_complete
            && final_field.writes.is_empty()
            && resolver.constructor_source_level() >= 25
            && resolver.constructor_early_field_store(final_field.reference)
            && effects.iter().all(|effect| {
                let field = &effect.fields[field_index];
                !effect.has_exception_paths
                    && field.before_chain.is_empty()
                    && field.after_chain.is_empty()
                    && field.normal_return_counts == std::collections::BTreeSet::from([0])
            });
        if allocation_default {
            let literal = match f.ty {
                JavaType::Boolean => "false",
                JavaType::Byte | JavaType::Short | JavaType::Char | JavaType::Int => "0",
                JavaType::Long => "0L",
                JavaType::Float => "0.0f",
                JavaType::Double => "0.0d",
                JavaType::Class(_) | JavaType::Array(_) => "null",
                JavaType::Void => return Err(fail()),
            };
            allocation_defaults.push((
                final_field.reference,
                format!("this.{} = {literal};", f.name),
            ));
        }
        for (index, input) in inputs.iter().enumerate() {
            let effect = &effects[index].fields[field_index];
            let count = effect.before_chain.union(&effect.after_chain).count();
            if !effect.before_chain.iter().all(|site| {
                plans[index].relocated_pre_chain_fields.contains(site)
                    || plans[index].preserved_pre_chain_fields.contains(site)
            }) || effect.read_before_assignment
                || count
                    != final_field
                        .writes
                        .get(&input.reference)
                        .copied()
                        .unwrap_or(0)
                || effect.normal_return_counts.len() != 1
                || effect.normal_return_counts.first().copied()
                    != Some(u8::from(
                        !allocation_default && plans[index].chain == ChainKind::Super,
                    ))
            {
                return Err(Unsupported::Metadata(format!(
                    "constructor final assignment: field {:?}, method {:?}, observed stores {count}, inventory stores {}, normal returns {:?}, expected local {}",
                    final_field.reference,
                    input.reference,
                    final_field
                        .writes
                        .get(&input.reference)
                        .copied()
                        .unwrap_or(0),
                    effect.normal_return_counts,
                    u8::from(!allocation_default && plans[index].chain == ChainKind::Super),
                )));
            }
            let mut current = index;
            let mut count = 0u8;
            loop {
                let local = &effects[current].fields[field_index].normal_return_counts;
                if local.len() != 1 {
                    return Err(fail());
                }
                count = count.saturating_add(*local.first().ok_or_else(fail)?);
                if plans[current].chain == ChainKind::Super {
                    break;
                }
                current = *plan_indices.get(&plans[current].target).ok_or_else(fail)?;
            }
            if count != u8::from(!allocation_default) {
                return Err(fail());
            }
        }
    }
    // Commit after the complete original family and every field validate.
    // Default writes must precede original prologue effects and super: a parent
    // callback/reflection write must never be reset afterward.
    for plan in &mut plans {
        if plan.chain != ChainKind::Super || allocation_defaults.is_empty() {
            continue;
        }
        let prefix = allocation_defaults
            .iter()
            .map(|(_, line)| format!("    {line}\n"))
            .collect::<String>();
        plan.source = plan.source.replacen("{\n", &format!("{{\n{prefix}"), 1);
        plan.prologue.splice(
            0..0,
            allocation_defaults.iter().map(|(_, line)| line.clone()),
        );
        plan.allocation_default_finals = allocation_defaults
            .iter()
            .map(|(field, _)| *field)
            .collect();
    }
    Ok(plans)
}

/// Source-level constructor chain kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChainKind {
    /// Invoke the direct superclass initializer.
    Super,
    /// Delegate to another initializer in the declaring class.
    This,
}

/// Validated constructor plan, with the chain separate from the supported body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstructorPlan {
    /// Chain dispatch.
    pub chain: ChainKind,
    /// Explicit initializer reference.
    pub target: MethodRef,
    /// Explicit Java initializer invocation, including the semicolon.
    pub first_statement: String,
    /// Original-order statements before the initializer (Java25 and later).
    pub prologue: Vec<String>,
    /// Statements following initialization, in managed instruction order.
    pub body: Vec<String>,
    /// Complete constructor declaration.
    pub source: String,
    /// Exact declaring class simple name.
    pub constructor_name: String,
    /// Sanitized parameter names in descriptor order.
    pub parameter_names: Vec<String>,
    /// Exact original own-field writes moved over a proved inert super chain.
    /// These are emitted in original prelude order immediately after `super`.
    pub relocated_pre_chain_fields: HashSet<InsnId>,
    /// Exact original own-field writes retained before the chain under Java25.
    /// These grant no Java17 reordering permission.
    pub preserved_pre_chain_fields: HashSet<InsnId>,
    /// Synthetic allocation-default source recipes, separate from original
    /// instruction/store provenance. Only complete Java25 families attest these.
    pub allocation_default_finals: Vec<FieldRef>,
}

fn init_signature(m: &JavaMethod) -> Result<(), Unsupported> {
    qualified(&m.owner)?;
    if m.name != "<init>" || m.is_static || m.return_type != JavaType::Void {
        return Err(Unsupported::Metadata(
            "invalid initializer signature".into(),
        ));
    }
    for t in &m.parameters {
        if *t == JavaType::Void {
            return Err(Unsupported::Metadata("void initializer parameter".into()));
        }
        t.source()?;
    }
    for t in &m.throws {
        qualified(t)?;
    }
    Ok(())
}

// Pure SSA aliases of the exact incoming receiver carry identity only. A
// fixed point cannot admit an ungrounded cycle or a Phi with one foreign input.
// Actual prefix availability/edge selection is independently proved before use.
fn constructor_receiver_lineage(ir: &MethodIr<SsaForm>, receiver: ValueId) -> HashSet<ValueId> {
    let mut aliases = HashSet::from([receiver]);
    loop {
        let before = aliases.len();
        for insn in ir.instructions().values() {
            match insn {
                ManagedInsn::Move { result, value } if aliases.contains(value) => {
                    aliases.insert(*result);
                }
                ManagedInsn::Phi { result, inputs }
                    if !inputs.is_empty() && inputs.iter().all(|(_, v)| aliases.contains(v)) =>
                {
                    aliases.insert(*result);
                }
                _ => {}
            }
        }
        if aliases.len() == before {
            return aliases;
        }
    }
}

/// Plan one explicit constructor from a linear initialization prefix and a
/// supported return-terminated branch tree. `reference`
/// identifies this initializer; `direct_super` is caller-owned hierarchy data.
/// No constructor is synthesized when the IR lacks an initializer invocation.
pub fn plan_constructor<R: ConstructorResolver>(
    ir: &MethodIr<SsaForm>,
    reference: MethodRef,
    direct_super: &str,
    bindings: &ParameterBindings,
    resolver: &R,
) -> Result<ConstructorPlan, Unsupported> {
    plan_constructor_declaration(
        ir,
        reference,
        direct_super,
        &DeclarationBindings {
            receiver: bindings.receiver,
            parameters: bindings.parameters.iter().copied().map(Some).collect(),
            names: bindings.names.clone(),
        },
        resolver,
    )
}

/// Lower an original constructor while preserving every descriptor parameter,
/// including incoming parameters with no SSA use. Missing values are not invented.
pub fn plan_constructor_declaration<R: ConstructorResolver>(
    ir: &MethodIr<SsaForm>,
    reference: MethodRef,
    direct_super: &str,
    bindings: &DeclarationBindings,
    resolver: &R,
) -> Result<ConstructorPlan, Unsupported> {
    let declaration = resolver
        .initializer(reference)
        .ok_or_else(|| Unsupported::Metadata("unresolved constructor".into()))?;
    let m = &declaration.method;
    init_signature(m)?;
    qualified(direct_super)?;
    if direct_super == m.owner
        || bindings.parameters.len() != m.parameters.len()
        || (!bindings.names.is_empty() && bindings.names.len() != m.parameters.len())
    {
        return Err(Unsupported::Metadata(
            "constructor bindings/hierarchy".into(),
        ));
    }
    let receiver = bindings
        .receiver
        .ok_or_else(|| Unsupported::Metadata("constructor receiver missing".into()))?;
    let mut reserved = HashSet::from(["java".to_string()]);
    reserve_class(&mut reserved, &m.owner);
    reserve_class(&mut reserved, direct_super);
    for (_, value) in ir.values() {
        if let Ok(t) = managed_type(value.ty(), resolver) {
            reserve_type(&mut reserved, &t);
        }
    }
    for (_, insn) in ir.instructions() {
        match insn {
            ManagedInsn::Invoke { method, .. } => {
                if let Some(target) = resolver.method(*method) {
                    reserve_class(&mut reserved, &target.owner);
                    reserve_type(&mut reserved, &target.return_type);
                    for t in &target.parameters {
                        reserve_type(&mut reserved, t);
                    }
                }
            }
            ManagedInsn::FieldGet { field, .. } | ManagedInsn::FieldPut { field, .. } => {
                if let Some(f) = resolver.field(*field) {
                    reserve_class(&mut reserved, &f.owner);
                    reserve_type(&mut reserved, &f.ty);
                }
            }
            _ => {}
        }
    }
    for t in &m.parameters {
        reserve_type(&mut reserved, t);
    }
    let mut e = Emitter {
        allocation_readable: None,
        object_planning: Default::default(),
        ir,
        resolver,
        values: HashMap::new(),
        lines: Vec::new(),
        next: 0,
        parameters: HashSet::new(),
        reserved,
        current: None,
        hoisted: HashSet::new(),
        declarations: contextual_locals::declaration_plan(ir, &m.return_type, resolver).map_err(
            |error| Unsupported::Type(format!("constructor declaration plan: {error:?}")),
        )?,
    };
    let mut names = Vec::new();
    let mut params = Vec::new();
    for (i, (v, t)) in bindings.parameters.iter().zip(&m.parameters).enumerate() {
        let mut name = sanitize_identifier(
            bindings
                .names
                .get(i)
                .map(String::as_str)
                .unwrap_or(&format!("p{i}")),
        );
        if name
            .strip_prefix('v')
            .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        {
            name.insert_str(0, "p_");
        }
        let base = name.clone();
        let mut suffix = 1;
        while e.reserved.contains(&name) || names.contains(&name) {
            name = format!("{base}_{suffix}");
            suffix += 1;
        }
        if let Some(v) = v {
            bind(&mut e, *v, name.clone(), t.clone())?;
        }
        params.push(format!("{} {name}", t.source()?));
        names.push(name);
    }
    bind(
        &mut e,
        receiver,
        "this".into(),
        JavaType::Class(m.owner.clone()),
    )?;
    let mut aliases = constructor_receiver_lineage(ir, receiver);
    let cfg = ir.cfg();
    cfg.validate()
        .map_err(|err| Unsupported::ControlFlow(err.to_string()))?;
    let mut block = cfg.entry();
    let mut incoming_edge = None;
    let mut seen = HashSet::new();
    let mut chain = None;
    let mut prefix_arguments = arguments::PrefixArguments::default();
    let mut prelude_operands = Vec::new();
    let mut relocated_pre_chain_fields = HashSet::new();
    let mut preserved_pre_chain_fields = HashSet::new();
    let mut has_prelude = false;
    let original_order_prelude = resolver.constructor_source_level() >= 25;
    let mut prologue = Vec::new();
    if let Ok(plan) = mask_choices::plan(
        &ConstructorInput {
            reference,
            ir,
            bindings,
        },
        resolver,
    ) {
        // Preserve the original pure evaluations and selected Phi transfers in
        // Java25's constructor prologue. These are uninitialized source locals,
        // never invented field or local default assignments. Zero carriers are
        // kept integral until the existing exact-null operand proof is used by
        // an original edge copy into its reference-typed Phi destination.
        for (value, ty) in &plan.locals {
            e.declarations.insert(*value, ty.clone());
            let mut name = format!("v{}", e.next);
            e.next += 1;
            while e.reserved.contains(&name) {
                name = format!("v{}", e.next);
                e.next += 1;
            }
            e.lines.push(format!("{} {name};", ty.source()?));
            e.reserved.insert(name.clone());
            e.values.insert(*value, (name, ty.clone()));
            e.hoisted.insert(*value);
        }
        for value in &plan.null_defaults {
            e.declarations.insert(*value, JavaType::Int);
            if let Some((_, ty)) = e.values.get_mut(value) {
                *ty = JavaType::Int;
            }
        }
        render_steps(&mut e, &plan.steps, m)?;
        block = plan.chain_block;
        incoming_edge = cfg
            .block(block)
            .map_err(|error| Unsupported::ControlFlow(error.to_string()))?
            .incoming()
            .first()
            .copied();
        seen.extend(plan.prefix_blocks);
        has_prelude = true;
    }
    loop {
        if !seen.insert(block) {
            return Err(Unsupported::ControlFlow("constructor cycle".into()));
        }
        let node = cfg
            .block(block)
            .map_err(|err| Unsupported::ControlFlow(err.to_string()))?;
        let id = *node.payload();
        let insn = ir
            .instruction(id)
            .map_err(|err| Unsupported::Instruction(err.to_string()))?;
        e.current = Some(id);
        let mut returned = false;
        if chain.is_some()
            && let Some(metadata) = resolver.constructor_exception_metadata(reference, ir)
        {
            seen.remove(&block);
            exception_suffix::emit(&mut e, m, bindings, &seen, &*metadata)?;
            seen.extend(cfg.block_ids());
            break;
        }
        if chain.is_some() && initialized_argument_phi_entry(&e, block, insn) {
            seen.remove(&block);
            if initialized_suffix::emit_initialized_suffix(
                &mut e,
                m,
                block,
                incoming_edge,
                &mut seen,
            )
            .is_ok()
            {
                break;
            }
            // The transactional attempt denied proof; retain the original
            // linear allocation route and its established fallback behavior.
            seen.insert(block);
        }
        if chain.is_some() && initialized_suffix_entry(insn, node.incoming().len()) {
            // The prefix has already bound and initialized this. No chain call
            // is moved into/out of a branch. Existing acyclic suffixes keep
            // their path-sensitive blank-final spelling; the initialized
            // helper owns only loop-bearing control suffixes.
            seen.remove(&block);
            if initialized_suffix_has_cycle(cfg, block) {
                initialized_suffix::emit_initialized_suffix(
                    &mut e,
                    m,
                    block,
                    incoming_edge,
                    &mut seen,
                )?;
            } else if matches!(
                decomp_structure::extended::structure_flow(cfg),
                Ok(decomp_structure::extended::FlowStructuredCfg::Shared(_))
            ) {
                // Shared blocks get one placement after the original chain.
                // A denied proof restores the complete emitter state before
                // the existing path-sensitive suffix grammar is attempted.
                if initialized_suffix::emit_shared_initialized_suffix(
                    &mut e,
                    m,
                    block,
                    incoming_edge,
                    &mut seen,
                )
                .is_err()
                {
                    emit_constructor_tail(&mut e, m, block, &mut seen, 0)?;
                }
            } else if initialized_suffix::emit_initialized_suffix(
                &mut e,
                m,
                block,
                incoming_edge,
                &mut seen,
            )
            .is_err()
            {
                emit_constructor_tail(&mut e, m, block, &mut seen, 0)?;
            }
            break;
        }
        if chain.is_none() {
            if matches!(
                insn,
                ManagedInsn::Branch {
                    condition: Some(_),
                    ..
                }
            ) {
                if original_order_prelude
                    && let Some(next) =
                        emit_abrupt_pre_init_branch(&mut e, m, block, &aliases, &mut seen)?
                {
                    block = next;
                    has_prelude = true;
                } else if original_order_prelude
                    && let Some(next) =
                        argument_guards::emit(&mut e, m, block, &aliases, &mut seen)?
                {
                    block = next;
                    has_prelude = true;
                } else if original_order_prelude
                    && let Some(next) =
                        prefix_diamonds::emit(&mut e, m, block, &aliases, &mut seen)?
                {
                    block = next;
                    has_prelude = true;
                } else {
                    block = prefix_arguments.diamond(&mut e, m, block, &aliases, &mut seen)?;
                }
                continue;
            }
            if original_order_prelude && source25::emit(&mut e, insn, m, &aliases)? {
                has_prelude = true;
            } else {
                match insn {
                    ManagedInsn::Invoke {
                        kind: InvokeKind::Static,
                        method,
                        result: None,
                        args,
                    } => {
                        let fail = || {
                            Unsupported::Instruction(
                                "unproved pre-initializer static statement".into(),
                            )
                        };
                        let target = resolver.method(*method).ok_or_else(fail)?;
                        let flags = resolver.method_access(*method).ok_or_else(fail)?;
                        let operands = invoke_arguments::operands(&target, args)?;
                        if !target.is_static
                            || flags & 8 == 0
                            || target.return_type != JavaType::Void
                            || !arguments::constructor_call_access(&m.owner, &target.owner, flags)
                            || target
                                .throws
                                .iter()
                                .any(|t| !resolver.exception_covered(t, &m.throws))
                            || operands.iter().any(|v| {
                                !commuting_operand(&e, *v, &aliases, true, &mut HashSet::new(), 0)
                            })
                        {
                            return Err(fail());
                        }
                        member(&target.name)?;
                        let mut actual = Vec::new();
                        for (v, ty) in operands.iter().zip(&target.parameters) {
                            let (expression, actual_ty) = e.operand(*v, ty)?;
                            if !resolver.assignable(&actual_ty, ty) {
                                return Err(fail());
                            }
                            // Descriptor-pinned safe conversion, never a downcast
                            // invented to make an unresolved operand compile.
                            actual.push(format!("({}) ({expression})", ty.source()?));
                        }
                        e.lines.push(format!(
                            "{}.{}({});",
                            crate::static_method_qualifier::resolved_owner(
                                &target, *method, &m.owner, resolver
                            )?,
                            target.name,
                            actual.join(", ")
                        ));
                        prelude_operands.extend(operands);
                        has_prelude = true;
                    }
                    ManagedInsn::FieldPut {
                        object: Some(object),
                        field,
                        value,
                    } if aliases.contains(object) => {
                        let fail = || {
                            Unsupported::Instruction(
                                "unproved pre-initializer capture store".into(),
                            )
                        };
                        let target = resolver.field(*field).ok_or_else(fail)?;
                        let flags = resolver.field_access(*field).ok_or_else(fail)?;
                        if target.is_static
                            || flags & (8 | if original_order_prelude { 0 } else { 0x40 }) != 0
                            || target.owner != m.owner
                            || original_order_prelude
                                && !resolver.constructor_early_field_store(*field)
                            || if original_order_prelude {
                                aliases.contains(value) || e.get(*value).is_err()
                            } else {
                                !commuting_operand(
                                    &e,
                                    *value,
                                    &aliases,
                                    true,
                                    &mut HashSet::new(),
                                    0,
                                )
                            }
                        {
                            return Err(fail());
                        }
                        let (_, actual_ty) = e.operand(*value, &target.ty)?;
                        if !resolver.assignable(&actual_ty, &target.ty) {
                            return Err(fail());
                        }
                        e.instruction(insn, m)?;
                        prelude_operands.push(*value);
                        if original_order_prelude {
                            preserved_pre_chain_fields.insert(id);
                        } else {
                            relocated_pre_chain_fields.insert(id);
                        }
                        has_prelude = true;
                    }
                    ManagedInsn::Compare {
                        op,
                        result,
                        lhs,
                        rhs,
                    } => {
                        let (left, right) = pre_init_comparison_operands(&e, *op, *lhs, *rhs)?;
                        if e.ty(*result)? != JavaType::Boolean {
                            return Err(Unsupported::Type("pre-init comparison result".into()));
                        }
                        let symbol = match op {
                            CompareOp::Equal => "==",
                            CompareOp::NotEqual => "!=",
                            CompareOp::LessThan => "<",
                            CompareOp::LessEqual => "<=",
                            CompareOp::GreaterThan => ">",
                            CompareOp::GreaterEqual => ">=",
                        };
                        inline(
                            &mut e,
                            *result,
                            format!("(({left}) {symbol} ({right}))"),
                            JavaType::Boolean,
                        )?;
                    }

                    ManagedInsn::NewInstance { .. } => prefix_arguments.allocation(&e, id)?,
                    ManagedInsn::Invoke {
                        kind: InvokeKind::Static | InvokeKind::Virtual | InvokeKind::Interface,
                        result: Some(_),
                        args,
                        ..
                    } if !args.iter().any(|value| aliases.contains(value)) => {
                        prefix_arguments.expression(&mut e, id, m)?;
                    }
                    ManagedInsn::NewArray { .. } | ManagedInsn::FieldGet { object: None, .. } => {
                        prefix_arguments.expression(&mut e, id, m)?;
                    }
                    ManagedInsn::Invoke {
                        kind: InvokeKind::Special,
                        args,
                        ..
                    } if args.first().is_some_and(|v| !aliases.contains(v)) => {
                        prefix_arguments.initialize(&mut e, id, m)?;
                    }

                    ManagedInsn::Move { result, value } => {
                        let (expression, ty) = e.get(*value)?;
                        inline(&mut e, *result, expression, ty)?;
                        prefix_arguments.copy(*result, *value);
                        if aliases.contains(value) {
                            aliases.insert(*result);
                        }
                    }
                    ManagedInsn::Const {
                        result,
                        value: Constant::Null,
                    } => inline_contextual_literal(&mut e, *result, Constant::Null)?,
                    ManagedInsn::Const {
                        result,
                        value: Constant::Zero,
                    } if e.ty(*result).map_or(true, |ty| ty.reference()) => {
                        inline_contextual_literal(&mut e, *result, Constant::Zero)?
                    }
                    ManagedInsn::Const { result, value } => {
                        if matches!(value, Constant::String(_) | Constant::ClassLiteral(_)) {
                            prefix_arguments.literal(
                                *result,
                                id,
                                matches!(value, Constant::String(_)),
                            )?;
                        }
                        let ty = e.ty(*result)?;
                        if !ty.numeric()
                            && ty != JavaType::Boolean
                            && !matches!(
                                value,
                                Constant::String(_) | Constant::ClassLiteral(_) | Constant::Null
                            )
                        {
                            return Err(Unsupported::Instruction(
                                "non-primitive pre-init constant".into(),
                            ));
                        }
                        let expression = constant(*value, &ty, resolver)?;
                        inline(&mut e, *result, expression, ty)?;
                    }
                    ManagedInsn::Arithmetic {
                        op,
                        result,
                        lhs,
                        rhs,
                    } if !matches!(op, ArithOp::Div | ArithOp::Rem) => {
                        let (left, lt) = e.get(*lhs)?;
                        let (right, rt) = e.get(*rhs)?;
                        let ty = e.ty(*result)?;
                        let shift = matches!(
                            op,
                            ArithOp::ShiftLeft | ArithOp::ShiftRight | ArithOp::UnsignedShiftRight
                        );
                        let bit = matches!(op, ArithOp::And | ArithOp::Or | ArithOp::Xor);
                        if matches!(ty, JavaType::Byte | JavaType::Short | JavaType::Char)
                            || lt != ty
                            || (!shift && rt != ty)
                            || (shift && (!lt.integral() || rt != JavaType::Int))
                            || !(ty.numeric() || bit && ty == JavaType::Boolean)
                            || (bit && !ty.integral() && ty != JavaType::Boolean)
                        {
                            return Err(Unsupported::Type(
                                "pre-init arithmetic operand types".into(),
                            ));
                        }
                        let symbol = match op {
                            ArithOp::Add => "+",
                            ArithOp::Sub => "-",
                            ArithOp::Mul => "*",
                            ArithOp::And => "&",
                            ArithOp::Or => "|",
                            ArithOp::Xor => "^",
                            ArithOp::ShiftLeft => "<<",
                            ArithOp::ShiftRight => ">>",
                            ArithOp::UnsignedShiftRight => ">>>",
                            _ => {
                                return Err(Unsupported::Instruction(
                                    "throwing pre-init arithmetic".into(),
                                ));
                            }
                        };
                        // Only non-throwing pure expressions may be duplicated or reordered
                        // into Java's left-to-right constructor argument evaluation.
                        inline(
                            &mut e,
                            *result,
                            format!("(({left}) {symbol} ({right}))"),
                            ty,
                        )?;
                    }
                    ManagedInsn::Invoke {
                        kind: InvokeKind::Special,
                        method,
                        args,
                        result: None,
                    } => {
                        let target = resolver
                            .initializer(*method)
                            .ok_or_else(|| Unsupported::Metadata("unmapped initializer".into()))?;
                        init_signature(&target.method)?;
                        let t = &target.method;
                        let args = invoke_arguments::operands(t, args)?;
                        let kind = if t.owner == m.owner {
                            ChainKind::This
                        } else if t.owner == direct_super {
                            ChainKind::Super
                        } else {
                            return Err(Unsupported::Metadata(
                                "initializer is not this/direct super".into(),
                            ));
                        };
                        if args.len() != t.parameters.len() + 1 || !aliases.contains(&args[0]) {
                            return Err(Unsupported::Type("initializer receiver/arity".into()));
                        }
                        if t.throws
                            .iter()
                            .any(|x| !resolver.exception_covered(x, &m.throws))
                        {
                            return Err(Unsupported::Metadata(
                                "undeclared initializer exception".into(),
                            ));
                        }
                        if kind == ChainKind::This {
                            if declaration.delegates_to != Some(*method) {
                                return Err(Unsupported::Metadata(
                                    "this delegation mapping mismatch".into(),
                                ));
                            }
                            let mut visited = HashSet::from([reference]);
                            let mut next = Some(*method);
                            while let Some(current) = next {
                                if !visited.insert(current) {
                                    return Err(Unsupported::ControlFlow(
                                        "this delegation cycle".into(),
                                    ));
                                }
                                let mapped = resolver.initializer(current).ok_or_else(|| {
                                    Unsupported::Metadata("incomplete delegation mapping".into())
                                })?;
                                init_signature(&mapped.method)?;
                                if mapped.method.owner != m.owner {
                                    return Err(Unsupported::Metadata(
                                        "this delegation owner".into(),
                                    ));
                                }
                                next = mapped.delegates_to;
                            }
                        } else if declaration.delegates_to.is_some() {
                            return Err(Unsupported::Metadata(
                                "super delegation mapping mismatch".into(),
                            ));
                        }
                        if has_prelude && original_order_prelude {
                            // Preserve the constructor call frame, original side
                            // effects and failure/finalizer eligibility ordering.
                            // Every prefix operand was independently proved free
                            // of prohibited early-construction receiver uses.
                            prelude_operands.extend_from_slice(&args[1..]);
                            prefix_arguments.check(&e, &prelude_operands)?;
                            prologue = std::mem::take(&mut e.lines);
                        } else if has_prelude {
                            // Java17 requires the chain first. Only cross a complete
                            // inert direct-super chain and argument evaluation that
                            // cannot throw, initialize a class, allocate or read this.
                            if kind != ChainKind::Super
                                || resolver
                                    .constructor_prelude_chain(reference, *method)
                                    .is_none()
                                || args[1..].iter().any(|v| {
                                    !commuting_operand(
                                        &e,
                                        *v,
                                        &aliases,
                                        false,
                                        &mut HashSet::new(),
                                        0,
                                    )
                                })
                            {
                                return Err(Unsupported::Instruction(
                                    "pre-initializer inert-chain proof unavailable".into(),
                                ));
                            }
                            prelude_operands.extend_from_slice(&args[1..]);
                            prefix_arguments.check(&e, &prelude_operands)?;
                        } else {
                            prefix_arguments.check(&e, &args[1..])?;
                        }
                        let mut actual = Vec::new();
                        for (v, ty) in args[1..].iter().zip(&t.parameters) {
                            if aliases.contains(v) {
                                return Err(Unsupported::Type(
                                    "uninitialized receiver argument".into(),
                                ));
                            }
                            let (expression, actual_ty) = e.operand(*v, ty)?;
                            if !resolver.assignable(&actual_ty, ty) {
                                return Err(Unsupported::Type(format!(
                                    "assignment {actual_ty:?} to {ty:?}"
                                )));
                            }
                            actual.push(
                                if actual_ty != *ty
                                    || expression == "null"
                                    || matches!(
                                        ty,
                                        JavaType::Byte | JavaType::Short | JavaType::Char
                                    )
                                {
                                    format!("({}) ({expression})", ty.source()?)
                                } else {
                                    expression
                                },
                            );
                        }
                        chain = Some((
                            kind,
                            *method,
                            format!(
                                "{}({});",
                                if kind == ChainKind::This {
                                    "this"
                                } else {
                                    "super"
                                },
                                actual.join(", ")
                            ),
                        ));
                    }
                    ManagedInsn::Opaque {
                        reads,
                        writes,
                        effects,
                        ..
                    } if reads.is_empty()
                        && !writes.is_empty()
                        && *effects == managed_ir::effects::EffectFlags::NONE
                        && writes.iter().all(|v| e.parameters.contains(v)) => {}
                    _ => {
                        return Err(Unsupported::Instruction(
                            "unsupported pre-initialization operation".into(),
                        ));
                    }
                }
            }
        } else {
            if let ManagedInsn::Move { result, value } = insn
                && aliases.contains(value)
            {
                inline(
                    &mut e,
                    *result,
                    "this".into(),
                    JavaType::Class(m.owner.clone()),
                )?;
                aliases.insert(*result);
            } else if let ManagedInsn::Invoke {
                kind: kind @ (InvokeKind::Special | InvokeKind::Super),
                method,
                args,
                result,
            } = insn
                && (*kind == InvokeKind::Super
                    || resolver.constructor_super_target(&m.owner, *method))
            {
                let (target, flags) = if *kind == InvokeKind::Super {
                    let binding =
                        resolver
                            .super_invocation(&m.owner, *method)
                            .ok_or_else(|| {
                                Unsupported::Metadata(
                                    "constructor exact super target proof unavailable".into(),
                                )
                            })?;
                    (binding.method, binding.access_flags)
                } else {
                    // Special remains pinned to its symbolic declaration and
                    // the existing constructor-specific target proof.
                    (
                        resolver.method(*method).ok_or_else(|| {
                            Unsupported::Metadata("unresolved constructor super method".into())
                        })?,
                        resolver.method_access(*method).ok_or_else(|| {
                            Unsupported::Metadata("constructor super declaration flags".into())
                        })?,
                    )
                };
                if target.name.starts_with('<')
                    || target.is_static
                    || flags & (2 | 8 | 0x80 | 0x400) != 0
                    || !args.first().is_some_and(|v| aliases.contains(v))
                    || target
                        .throws
                        .iter()
                        .any(|t| !resolver.exception_covered(t, &m.throws))
                {
                    return Err(Unsupported::Metadata(
                        "invalid constructor super target".into(),
                    ));
                }
                member(&target.name)?;
                let operands = invoke_arguments::operands(&target, args)?;
                let mut actual = Vec::new();
                for (value, ty) in operands[1..].iter().zip(&target.parameters) {
                    let (expression, actual_ty) = e.operand(*value, ty)?;
                    e.compatible(&actual_ty, ty)?;
                    // Always pin the original descriptor, including identical
                    // static types, rather than trusting overload inference.
                    actual.push(format!("({}) ({expression})", ty.source()?));
                }
                let expression = format!("super.{}({})", target.name, actual.join(", "));
                if let Some(value) = result {
                    if target.return_type == JavaType::Void {
                        return Err(Unsupported::Type("void super result".into()));
                    }
                    e.define(*value, expression, target.return_type)?;
                } else {
                    e.lines.push(format!("{expression};"));
                }
            } else {
                returned = e.instruction(insn, m)?;
            }
        }
        if returned {
            if !node.outgoing().is_empty() {
                return Err(Unsupported::ControlFlow("return with successors".into()));
            }
            break;
        }
        if node.outgoing().len() != 1 {
            return Err(Unsupported::ControlFlow(
                "constructor requires linear return".into(),
            ));
        }
        let selected = node.outgoing()[0];
        let edge = cfg
            .edge(selected)
            .map_err(|err| Unsupported::ControlFlow(err.to_string()))?;
        if *edge.kind() != EdgeKind::Unconditional {
            return Err(Unsupported::ControlFlow(
                "constructor branch/exception edge".into(),
            ));
        }
        incoming_edge = Some(selected);
        block = edge.target();
    }
    if seen.len() != cfg.block_count() {
        return Err(Unsupported::ControlFlow(
            "unreachable constructor blocks".into(),
        ));
    }
    let (kind, target, first) =
        chain.ok_or_else(|| Unsupported::Instruction("missing explicit initializer".into()))?;
    let name = m.owner.rsplit('.').next().unwrap_or(&m.owner).to_string();
    member(&name)?;
    let throws = if m.throws.is_empty() {
        String::new()
    } else {
        format!(" throws {}", m.throws.join(", "))
    };
    let statements = prologue
        .iter()
        .chain(std::iter::once(&first))
        .chain(e.lines.iter())
        .map(|s| format!("    {s}"))
        .collect::<Vec<_>>()
        .join("\n");
    let source = format!(
        "public {name}({}){throws} {{\n{statements}\n}}\n",
        params.join(", ")
    );
    Ok(ConstructorPlan {
        chain: kind,
        target,
        first_statement: first,
        prologue,
        body: e.lines,
        source,
        constructor_name: name,
        parameter_names: names,
        relocated_pre_chain_fields,
        preserved_pre_chain_fields,
        allocation_default_finals: Vec::new(),
    })
}

/// Pure values may be duplicated or moved across the inert chain. Native
/// strings are allowed only at their original prelude statements, where the
/// PrefixArguments event proof retains resolution order. Class literals,
/// casts, field reads, allocations, invokes and phis fail closed.
fn commuting_operand<R: ConstructorResolver>(
    e: &Emitter<'_, R>,
    value: ValueId,
    aliases: &HashSet<ValueId>,
    strings: bool,
    active: &mut HashSet<ValueId>,
    depth: usize,
) -> bool {
    if depth > 32 || aliases.contains(&value) || !active.insert(value) {
        return false;
    }
    let result = if e.parameters.contains(&value) {
        true
    } else if let Ok(record) = e.ir.value(value) {
        match e.ir.instruction(record.def()) {
            Ok(ManagedInsn::Const {
                value: constant, ..
            }) => match constant {
                Constant::ClassLiteral(_) => false,
                Constant::String(_) => strings,
                _ => true,
            },
            Ok(ManagedInsn::Move { value, .. }) => {
                commuting_operand(e, *value, aliases, strings, active, depth + 1)
            }
            Ok(ManagedInsn::Arithmetic { op, lhs, rhs, .. })
                if !matches!(op, ArithOp::Div | ArithOp::Rem) =>
            {
                commuting_operand(e, *lhs, aliases, strings, active, depth + 1)
                    && commuting_operand(e, *rhs, aliases, strings, active, depth + 1)
            }
            Ok(ManagedInsn::Compare { lhs, rhs, .. }) => {
                commuting_operand(e, *lhs, aliases, strings, active, depth + 1)
                    && commuting_operand(e, *rhs, aliases, strings, active, depth + 1)
            }
            _ => false,
        }
    } else {
        false
    };
    active.remove(&value);
    result
}

/// Narrow structured suffix grammar. Branch environments are independent;
/// shared acyclic suffixes can be cloned into mutually exclusive branches under
/// a fixed lowering budget. Bounded acyclic Phi joins retain original edge
/// copies; cycles and catch state remain unsupported. This preserves Java's
/// blank-final path-sensitive assignment.
pub(super) fn emit_constructor_tail<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    m: &JavaMethod,
    block: decomp_core::ids::BlockId,
    seen: &mut HashSet<decomp_core::ids::BlockId>,
    depth: usize,
) -> Result<(), Unsupported> {
    let path = seen.clone();
    let mut budget = e.ir.cfg().block_count().saturating_mul(8).min(4096);
    if suffix_needs_selected_paths(e.ir, block) {
        let mut lowering = SelectedSuffixPaths {
            seen,
            budget: &mut budget,
            placements: HashMap::new(),
        };
        return lowering.emit(e, m, block, None, &path, depth);
    }
    emit_constructor_tail_path(e, m, block, seen, &path, &mut budget, depth)
}

// Preserve the compact, single-placement spelling of existing linear two-arm
// diamonds. Other acyclic joins use exclusive source paths; their original
// incoming edge, rather than a guessed enclosing branch, selects each Phi.
fn suffix_needs_selected_paths(ir: &MethodIr<SsaForm>, entry: decomp_core::ids::BlockId) -> bool {
    let cfg = ir.cfg();
    let post = std::cell::OnceCell::new();
    let mut reachable = HashSet::new();
    let mut pending = vec![entry];
    while let Some(block) = pending.pop() {
        if !reachable.insert(block) {
            continue;
        }
        let Ok(node) = cfg.block(block) else {
            return true;
        };
        for edge in node.outgoing() {
            let Ok(edge) = cfg.edge(*edge) else {
                return true;
            };
            pending.push(edge.target());
        }
    }
    for head in &reachable {
        let Ok(node) = cfg.block(*head) else {
            return true;
        };
        if !matches!(ir.instruction(*node.payload()), Ok(ManagedInsn::Phi { .. })) {
            continue;
        }
        // Later Phi instructions in a simultaneous cluster retain the head's
        // input edge IDs, despite having just one intra-cluster CFG predecessor.
        if node.incoming().len() == 1
            && cfg
                .edge(node.incoming()[0])
                .ok()
                .and_then(|edge| cfg.block(edge.source()).ok())
                .is_some_and(|prior| {
                    matches!(
                        ir.instruction(*prior.payload()),
                        Ok(ManagedInsn::Phi { .. })
                    )
                })
        {
            continue;
        }
        if node.incoming().len() != 2 {
            return true;
        }
        let post = post.get_or_init(|| decomp_core::analysis::post_dominators(cfg));
        let compact = reachable.iter().any(|branch| {
            if post.immediate_post_dominator(*branch) != Some(*head) {
                return false;
            }
            let Ok(branch_node) = cfg.block(*branch) else {
                return false;
            };
            if !matches!(
                ir.instruction(*branch_node.payload()),
                Ok(ManagedInsn::Branch {
                    condition: Some(_),
                    ..
                })
            ) || branch_node.outgoing().len() != 2
            {
                return false;
            }
            let mut arms = HashSet::new();
            branch_node.outgoing().iter().all(|first| {
                let mut edge_id = *first;
                for _ in 0..=32 {
                    let Ok(edge) = cfg.edge(edge_id) else {
                        return false;
                    };
                    let block = edge.target();
                    if block == *head {
                        return true;
                    }
                    let Ok(node) = cfg.block(block) else {
                        return false;
                    };
                    if !arms.insert(block)
                        || node.incoming() != [edge_id]
                        || node.outgoing().len() != 1
                    {
                        return false;
                    }
                    if matches!(
                        ir.instruction(*node.payload()),
                        Ok(ManagedInsn::Phi { .. }
                            | ManagedInsn::Branch {
                                condition: Some(_),
                                ..
                            }
                            | ManagedInsn::Return { .. }
                            | ManagedInsn::Throw { .. })
                    ) {
                        return false;
                    }
                    edge_id = node.outgoing()[0];
                    let Ok(edge) = cfg.edge(edge_id) else {
                        return false;
                    };
                    if edge.kind() != &EdgeKind::Unconditional {
                        return false;
                    }
                }
                false
            })
        });
        if !compact {
            return true;
        }
    }
    false
}

// Route a proven-shape post-chain allocation before argument Phi copies need
// their source locals. This selects the existing validated suffix planner only;
// allocation, access, availability and original-edge proof remain mandatory.
fn initialized_argument_phi_entry<R: JavaResolver>(
    e: &Emitter<'_, R>,
    entry: decomp_core::ids::BlockId,
    insn: &ManagedInsn,
) -> bool {
    let ManagedInsn::NewInstance { result, class } = insn else {
        return false;
    };
    let Some(owner) = e.resolver.class(*class) else {
        return false;
    };
    let cfg = e.ir.cfg();
    let Ok(start) = cfg.block(entry) else {
        return false;
    };
    if start.outgoing().len() != 1 {
        return false;
    }
    let mut pending = start
        .outgoing()
        .iter()
        .filter_map(|id| cfg.edge(*id).ok().map(|edge| edge.target()))
        .collect::<Vec<_>>();
    let mut visited = HashSet::new();
    let mut initializers = HashSet::new();
    let mut dependencies = Vec::new();
    let mut branch = false;
    let mut phi = false;
    while let Some(block) = pending.pop() {
        if !visited.insert(block) {
            continue;
        }
        if visited.len() > 128 {
            return false;
        }
        let Ok(node) = cfg.block(block) else {
            return false;
        };
        let Ok(instruction) = e.ir.instruction(*node.payload()) else {
            return false;
        };
        if let ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            result: None,
            method,
            args,
        } = instruction
            && args.first() == Some(result)
        {
            let Some(target) = e.resolver.method(*method) else {
                return false;
            };
            if target.name != "<init>" || target.owner != owner {
                return false;
            }
            initializers.insert(*node.payload());
            continue;
        }
        match instruction {
            ManagedInsn::Phi { .. } => phi = true,
            ManagedInsn::Branch {
                condition: Some(_), ..
            } => branch = true,
            ManagedInsn::Return { .. } | ManagedInsn::Throw { .. } => return false,
            _ => {}
        }
        if node.outgoing().is_empty() {
            return false;
        }
        for edge in node.outgoing() {
            let Ok(edge) = cfg.edge(*edge) else {
                return false;
            };
            if !matches!(
                edge.kind(),
                EdgeKind::Unconditional | EdgeKind::BranchTrue | EdgeKind::BranchFalse
            ) {
                return false;
            }
            dependencies.push((block, edge.target()));
            pending.push(edge.target());
        }
    }
    if !branch || !phi || initializers.len() != 1 {
        return false;
    }
    // A join may be visited twice; a real cycle must never be mistaken for it.
    let mut indegree = visited
        .iter()
        .map(|block| (*block, 0usize))
        .collect::<HashMap<_, _>>();
    for (_, target) in &dependencies {
        let Some(degree) = indegree.get_mut(target) else {
            return false;
        };
        *degree += 1;
    }
    let mut ready = indegree
        .iter()
        .filter_map(|(block, degree)| (*degree == 0).then_some(*block))
        .collect::<Vec<_>>();
    let mut count = 0;
    while let Some(block) = ready.pop() {
        count += 1;
        for (_, target) in dependencies.iter().filter(|(source, _)| *source == block) {
            let Some(degree) = indegree.get_mut(target) else {
                return false;
            };
            *degree -= 1;
            if *degree == 0 {
                ready.push(*target);
            }
        }
    }
    count == visited.len()
}

fn initialized_suffix_entry(insn: &ManagedInsn, incoming: usize) -> bool {
    incoming > 1
        || matches!(
            insn,
            ManagedInsn::Phi { .. }
                | ManagedInsn::Switch { .. }
                | ManagedInsn::Branch {
                    condition: Some(_),
                    ..
                }
        )
}

fn initialized_suffix_has_cycle<T>(
    cfg: &decomp_core::cfg::Cfg<T>,
    entry: decomp_core::ids::BlockId,
) -> bool {
    let dominators = decomp_core::analysis::dominators(cfg);
    let loops = decomp_core::analysis::natural_loops(cfg, &dominators);
    if loops.is_empty() {
        return false;
    }
    let mut reachable = HashSet::new();
    let mut pending = vec![entry];
    while let Some(block) = pending.pop() {
        if !reachable.insert(block) {
            continue;
        }
        let Ok(node) = cfg.block(block) else {
            return false;
        };
        for edge in node.outgoing() {
            if let Ok(edge) = cfg.edge(*edge) {
                pending.push(edge.target());
            }
        }
    }
    loops
        .iter()
        .any(|lp| lp.blocks().iter().any(|block| reachable.contains(block)))
}

struct SelectedSuffixPaths<'a> {
    placements: HashMap<InsnId, decomp_core::ids::BlockId>,
    seen: &'a mut HashSet<decomp_core::ids::BlockId>,
    budget: &'a mut usize,
}

impl SelectedSuffixPaths<'_> {
    fn emit<R: JavaResolver>(
        &mut self,
        e: &mut Emitter<'_, R>,
        method: &JavaMethod,
        block: decomp_core::ids::BlockId,
        incoming: Option<decomp_core::ids::EdgeId>,
        ancestors: &HashSet<decomp_core::ids::BlockId>,
        depth: usize,
    ) -> Result<(), Unsupported> {
        let fail = || Unsupported::ControlFlow("unproved constructor selected suffix path".into());
        if depth > 128 || *self.budget == 0 || ancestors.contains(&block) {
            return Err(fail());
        }
        let node = e.ir.cfg().block(block).map_err(|_| fail())?;
        let id = *node.payload();
        let instruction = e.ir.instruction(id).map_err(|_| fail())?;
        if self
            .placements
            .insert(id, block)
            .is_some_and(|prior| prior != block)
        {
            return Err(fail());
        }
        let mut path = ancestors.clone();
        if matches!(instruction, ManagedInsn::Phi { .. }) {
            return self.cluster(e, method, block, incoming.ok_or_else(fail)?, &path, depth);
        }
        *self.budget -= 1;
        path.insert(block);
        self.seen.insert(block);
        e.current = Some(id);
        if let ManagedInsn::Branch {
            condition: Some(condition),
            then_edge,
            else_edge: Some(else_edge),
        } = instruction
        {
            if then_edge == else_edge
                || node.outgoing().len() != 2
                || !node.outgoing().contains(then_edge)
                || !node.outgoing().contains(else_edge)
            {
                return Err(fail());
            }
            let yes = e.ir.cfg().edge(*then_edge).map_err(|_| fail())?;
            let no = e.ir.cfg().edge(*else_edge).map_err(|_| fail())?;
            if yes.kind() != &EdgeKind::BranchTrue || no.kind() != &EdgeKind::BranchFalse {
                return Err(fail());
            }
            let (condition, ty) = e.get(*condition)?;
            compatible(&ty, &JavaType::Boolean)?;
            let base = e.values.clone();
            e.lines.push(format!("if ({condition}) {{"));
            self.emit(e, method, yes.target(), Some(*then_edge), &path, depth + 1)?;
            e.values = base.clone();
            e.lines.push("} else {".into());
            self.emit(e, method, no.target(), Some(*else_edge), &path, depth + 1)?;
            e.values = base;
            e.lines.push("}".into());
            return Ok(());
        }
        match instruction {
            ManagedInsn::Branch {
                condition: None,
                then_edge,
                else_edge: None,
            } if node.outgoing() == [*then_edge] => {}
            ManagedInsn::Move { result, value }
                if e.get(*value).is_ok_and(|(name, ty)| {
                    name == "this" && ty == JavaType::Class(method.owner.clone())
                }) && e.ty(*result)? == JavaType::Class(method.owner.clone()) =>
            {
                if e.values.contains_key(result)
                    || e.ir.value(*result).map_err(|_| fail())?.def() != id
                {
                    return Err(fail());
                }
                // This suffix is entered only after the original receiver chain
                // completes. Keep the exact initialized receiver spelling so
                // Java blank finals are assigned through this, never a local.
                e.values.insert(
                    *result,
                    ("this".into(), JavaType::Class(method.owner.clone())),
                );
            }
            ManagedInsn::Branch { .. }
            | ManagedInsn::Switch { .. }
            | ManagedInsn::Invoke {
                kind: InvokeKind::Super,
                ..
            } => return Err(fail()),
            ManagedInsn::Invoke {
                kind: InvokeKind::Special,
                args,
                ..
            } => {
                let root = args
                    .first()
                    .and_then(|value| e.allocation_root(*value))
                    .ok_or_else(fail)?;
                if e.object_plan(root)?.initializer != id {
                    return Err(fail());
                }
                if e.instruction(instruction, method)? {
                    return Err(fail());
                }
            }
            ManagedInsn::Return { value: None } if method.name == "class_initializer_body" => {
                return if node.outgoing().is_empty() {
                    Ok(())
                } else {
                    Err(fail())
                };
            }
            _ => {
                if e.instruction(instruction, method)? {
                    return if node.outgoing().is_empty() {
                        Ok(())
                    } else {
                        Err(fail())
                    };
                }
            }
        }
        if node.outgoing().len() != 1 {
            return Err(fail());
        }
        let selected = node.outgoing()[0];
        let edge = e.ir.cfg().edge(selected).map_err(|_| fail())?;
        if edge.kind() != &EdgeKind::Unconditional {
            return Err(fail());
        }
        self.emit(e, method, edge.target(), Some(selected), &path, depth + 1)
    }

    fn cluster<R: JavaResolver>(
        &mut self,
        e: &mut Emitter<'_, R>,
        method: &JavaMethod,
        head: decomp_core::ids::BlockId,
        selected: decomp_core::ids::EdgeId,
        ancestors: &HashSet<decomp_core::ids::BlockId>,
        depth: usize,
    ) -> Result<(), Unsupported> {
        let fail = || Unsupported::ControlFlow("unproved constructor suffix Phi transfer".into());
        let cfg = e.ir.cfg();
        let incoming = cfg.block(head).map_err(|_| fail())?.incoming();
        if incoming.is_empty()
            || !incoming.contains(&selected)
            || incoming.iter().copied().collect::<HashSet<_>>().len() != incoming.len()
        {
            return Err(fail());
        }
        for edge in incoming {
            let edge = cfg.edge(*edge).map_err(|_| fail())?;
            if edge.target() != head || matches!(edge.kind(), EdgeKind::Exception(_)) {
                return Err(fail());
            }
        }
        let mut path = ancestors.clone();
        let mut current = head;
        let mut copies = Vec::new();
        let mut destinations = HashSet::new();
        let (exit, last_edge) = loop {
            if copies.len() >= 32 || *self.budget == 0 || !path.insert(current) {
                return Err(fail());
            }
            let node = cfg.block(current).map_err(|_| fail())?;
            if self
                .placements
                .insert(*node.payload(), current)
                .is_some_and(|prior| prior != current)
            {
                return Err(fail());
            }
            let ManagedInsn::Phi { result, inputs } =
                e.ir.instruction(*node.payload()).map_err(|_| fail())?
            else {
                return Err(fail());
            };
            if inputs.len() != incoming.len()
                || inputs.iter().map(|(edge, _)| *edge).collect::<HashSet<_>>()
                    != incoming.iter().copied().collect()
                || e.values.contains_key(result)
                || !destinations.insert(*result)
                || node.outgoing().len() != 1
            {
                return Err(fail());
            }
            if e.ir.value(*result).map_err(|_| fail())?.def() != *node.payload() {
                return Err(fail());
            }
            let source = inputs
                .iter()
                .find(|(edge, _)| *edge == selected)
                .map(|(_, value)| *value)
                .ok_or_else(fail)?;
            let ty = e.ty(*result)?;
            // Read all sources against the pre-transfer environment. No Phi
            // destination is available until the whole selected batch is read.
            let (expression, actual) = e.operand(source, &ty)?;
            e.compatible(&actual, &ty)?;
            let snapshot = loop {
                let name = format!("v{}", e.next);
                e.next += 1;
                if e.reserved.insert(name.clone()) {
                    break name;
                }
            };
            e.lines
                .push(format!("{} {snapshot} = {expression};", ty.source()?));
            let receiver = ty == JavaType::Class(method.owner.clone())
                && e.get(source)
                    .is_ok_and(|(name, actual)| name == "this" && actual == ty);
            copies.push((*result, snapshot, ty, receiver));
            *self.budget -= 1;
            self.seen.insert(current);
            let next_edge = node.outgoing()[0];
            let edge = cfg.edge(next_edge).map_err(|_| fail())?;
            if edge.kind() != &EdgeKind::Unconditional {
                return Err(fail());
            }
            let next = edge.target();
            let next_node = cfg.block(next).map_err(|_| fail())?;
            if matches!(
                e.ir.instruction(*next_node.payload()),
                Ok(ManagedInsn::Phi { .. })
            ) {
                if next_node.incoming() != [next_edge] {
                    return Err(fail());
                }
                current = next;
            } else {
                break (next, next_edge);
            }
        };
        for (result, snapshot, ty, receiver) in copies {
            let name = if receiver {
                "this".to_owned()
            } else {
                snapshot
            };
            if e.values.insert(result, (name, ty)).is_some() {
                return Err(fail());
            }
        }
        self.emit(e, method, exit, Some(last_edge), &path, depth + 1)
    }
}

fn emit_constructor_tail_path<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    m: &JavaMethod,
    block: decomp_core::ids::BlockId,
    seen: &mut HashSet<decomp_core::ids::BlockId>,
    path: &HashSet<decomp_core::ids::BlockId>,
    budget: &mut usize,
    depth: usize,
) -> Result<(), Unsupported> {
    let fail = || Unsupported::ControlFlow("unsupported constructor branch suffix".into());
    if depth > 128 || *budget == 0 || path.contains(&block) {
        return Err(fail());
    }
    let node = e.ir.cfg().block(block).map_err(|_| fail())?;
    let id = *node.payload();
    let insn = e.ir.instruction(id).map_err(|_| fail())?;
    // A shared acyclic suffix may appear in exclusive source branches. Each
    // execution follows only one branch; original-ID coverage is separate from
    // generated occurrences. Phi joins are handled by the structured helper;
    // catch state remains unsupported here.
    *budget -= 1;
    let mut path = path.clone();
    path.insert(block);
    seen.insert(block);
    e.current = Some(id);
    if m.name == "class_initializer_body" && matches!(insn, ManagedInsn::Return { value: None }) {
        return if node.outgoing().is_empty() {
            Ok(())
        } else {
            Err(fail())
        };
    }
    if let ManagedInsn::Branch {
        condition: Some(condition),
        then_edge,
        else_edge,
    } = insn
    {
        let (expression, ty) = e.get(*condition)?;
        compatible(&ty, &JavaType::Boolean)?;
        let mut yes = None;
        let mut no = None;
        for edge_id in node.outgoing() {
            let edge = e.ir.cfg().edge(*edge_id).map_err(|_| fail())?;
            match edge.kind() {
                EdgeKind::BranchTrue if yes.is_none() && edge_id == then_edge => {
                    yes = Some(edge.target())
                }
                EdgeKind::BranchFalse if no.is_none() && *else_edge == Some(*edge_id) => {
                    no = Some(edge.target())
                }
                _ => return Err(fail()),
            }
        }
        if suffix_control::emit_phi_diamond(e, m, block, seen, &path, budget, depth)? {
            return Ok(());
        }
        let values = e.values.clone();
        e.lines.push(format!("if ({expression}) {{"));
        emit_constructor_tail_path(e, m, yes.ok_or_else(fail)?, seen, &path, budget, depth + 1)?;
        e.values = values.clone();
        e.lines.push("} else {".into());
        emit_constructor_tail_path(e, m, no.ok_or_else(fail)?, seen, &path, budget, depth + 1)?;
        e.values = values;
        e.lines.push("}".into());
        return Ok(());
    }
    if matches!(insn, ManagedInsn::Phi { .. }) {
        return Err(fail());
    }
    if let ManagedInsn::Invoke {
        kind: InvokeKind::Special,
        args,
        ..
    } = insn
    {
        // A suffix may initialize its own freshly allocated ordinary object,
        // never this/super or an arbitrary special-dispatch receiver. The
        // ordinary typed object hook retains pairing, use and effect proofs.
        let root = args
            .first()
            .and_then(|value| e.allocation_root(*value))
            .ok_or_else(fail)?;
        if e.object_plan(root)?.initializer != id {
            return Err(fail());
        }
    }
    if e.instruction(insn, m)? {
        return if node.outgoing().is_empty() {
            Ok(())
        } else {
            Err(fail())
        };
    }
    if node.outgoing().len() != 1 {
        return Err(fail());
    }
    let edge = e.ir.cfg().edge(node.outgoing()[0]).map_err(|_| fail())?;
    if *edge.kind() != EdgeKind::Unconditional {
        return Err(fail());
    }
    emit_constructor_tail_path(e, m, edge.target(), seen, &path, budget, depth + 1)
}

// A Java25 prologue may retain a linear abrupt arm before its receiver chain.
// The other original edge continues normally; no original effect is cloned or
// evaluated eagerly, and no value defined in the throwing arm escapes it.
fn emit_abrupt_pre_init_branch<R: ConstructorResolver>(
    e: &mut Emitter<'_, R>,
    signature: &JavaMethod,
    block: BlockId,
    aliases: &HashSet<ValueId>,
    seen: &mut HashSet<BlockId>,
) -> Result<Option<BlockId>, Unsupported> {
    let cfg = e.ir.cfg();
    let node = cfg
        .block(block)
        .map_err(|error| Unsupported::ControlFlow(error.to_string()))?;
    let ManagedInsn::Branch {
        condition: Some(condition),
        then_edge,
        else_edge: Some(else_edge),
    } =
        e.ir.instruction(*node.payload())
            .map_err(|error| Unsupported::Instruction(error.to_string()))?
    else {
        return Ok(None);
    };
    if node.outgoing().len() != 2
        || !node.outgoing().contains(then_edge)
        || !node.outgoing().contains(else_edge)
        || then_edge == else_edge
        || cfg.edge(*then_edge).map_or(true, |edge| {
            edge.source() != block || edge.kind() != &EdgeKind::BranchTrue
        })
        || cfg.edge(*else_edge).map_or(true, |edge| {
            edge.source() != block || edge.kind() != &EdgeKind::BranchFalse
        })
    {
        return Err(Unsupported::ControlFlow(
            "invalid pre-init abrupt-arm edges".into(),
        ));
    }
    let then_target = cfg
        .edge(*then_edge)
        .map_err(|error| Unsupported::ControlFlow(error.to_string()))?
        .target();
    let else_target = cfg
        .edge(*else_edge)
        .map_err(|error| Unsupported::ControlFlow(error.to_string()))?
        .target();
    let path = |start| -> Option<Vec<(BlockId, InsnId)>> {
        let mut cursor = start;
        let mut visited = HashSet::new();
        let mut steps = Vec::new();
        for _ in 0..128 {
            if seen.contains(&cursor) || !visited.insert(cursor) {
                return None;
            }
            let node = cfg.block(cursor).ok()?;
            if node.incoming().len() != 1 {
                return None;
            }
            let id = *node.payload();
            let instruction = e.ir.instruction(id).ok()?;
            steps.push((cursor, id));
            if matches!(instruction, ManagedInsn::Throw { .. }) {
                return node.outgoing().is_empty().then_some(steps);
            }
            if !matches!(
                instruction,
                ManagedInsn::Const { .. }
                    | ManagedInsn::NewInstance { .. }
                    | ManagedInsn::Invoke {
                        kind: InvokeKind::Special,
                        ..
                    }
                    | ManagedInsn::Move { .. }
                    | ManagedInsn::Branch {
                        condition: None,
                        ..
                    }
            ) || node.outgoing().len() != 1
            {
                return None;
            }
            let edge = cfg.edge(node.outgoing()[0]).ok()?;
            if *edge.kind() != EdgeKind::Unconditional {
                return None;
            }
            cursor = edge.target();
        }
        None
    };
    let (steps, continuation, invert) = match (path(then_target), path(else_target)) {
        (Some(steps), None) => (steps, else_target, false),
        (None, Some(steps)) => (steps, then_target, true),
        _ => return Ok(None),
    };
    let (condition, ty) = e.get(*condition)?;
    if ty != JavaType::Boolean {
        return Err(Unsupported::Type("pre-init abrupt-arm condition".into()));
    }
    let available = e.values.clone();
    let line_start = e.lines.len();
    for &(_, id) in &steps {
        e.current = Some(id);
        let insn =
            e.ir.instruction(id)
                .map_err(|error| Unsupported::Instruction(error.to_string()))?;
        match insn {
            ManagedInsn::Branch {
                condition: None, ..
            } => {}
            ManagedInsn::Throw { value } => {
                if aliases.contains(value) || !e.instruction(insn, signature)? {
                    return Err(Unsupported::Instruction(
                        "invalid abrupt pre-init throw".into(),
                    ));
                }
            }
            _ => {
                if !source25::emit(e, insn, signature, aliases)? {
                    return Err(Unsupported::Instruction(
                        "unproved abrupt pre-init operation".into(),
                    ));
                }
            }
        }
    }
    let arm_lines = e.lines.drain(line_start..).collect::<Vec<_>>();
    e.values = available;
    e.lines.push(if invert {
        format!("if (!({condition})) {{")
    } else {
        format!("if ({condition}) {{")
    });
    e.lines
        .extend(arm_lines.into_iter().map(|line| format!("    {line}")));
    e.lines.push("}".into());
    seen.extend(steps.into_iter().map(|(block, _)| block));
    Ok(Some(continuation))
}

// Match ordinary source comparison semantics without introducing a statement
// before the original initializer or re-evaluating either captured operand.
fn pre_init_comparison_operands<R: JavaResolver>(
    e: &Emitter<'_, R>,
    op: CompareOp,
    lhs: ValueId,
    rhs: ValueId,
) -> Result<(String, String), Unsupported> {
    let (_, left_type) = e.get(lhs)?;
    let (_, right_type) = e.get(rhs)?;
    let context = |ty: &JavaType| match ty {
        JavaType::Byte | JavaType::Short | JavaType::Char => JavaType::Int,
        _ => ty.clone(),
    };
    let (mut left, lt) = e.operand(lhs, &context(&right_type))?;
    let (mut right, rt) = e.operand(rhs, &context(&left_type))?;
    let equality = matches!(op, CompareOp::Equal | CompareOp::NotEqual);
    let category_one = |ty: &JavaType| {
        matches!(
            ty,
            JavaType::Byte | JavaType::Short | JavaType::Char | JavaType::Int
        )
    };
    if lt == JavaType::Boolean && category_one(&rt) {
        left = format!("({left} ? 1 : 0)");
    } else if rt == JavaType::Boolean && category_one(&lt) {
        right = format!("({right} ? 1 : 0)");
    } else if equality && lt.reference() && rt.reference() {
        left = format!("((java.lang.Object) {left})");
        right = format!("((java.lang.Object) {right})");
    } else if !((lt == rt && (lt.numeric() || equality && lt == JavaType::Boolean))
        || category_one(&lt) && category_one(&rt))
    {
        return Err(Unsupported::Type("pre-init comparison types".into()));
    }
    Ok((left, right))
}

fn inline_contextual_literal<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    value: ValueId,
    literal: Constant,
) -> Result<(), Unsupported> {
    if e.values.contains_key(&value)
        || e.ir
            .value(value)
            .map_err(|x| Unsupported::Type(x.to_string()))?
            .def()
            != e.current
                .ok_or_else(|| Unsupported::Instruction("null definition identity".into()))?
    {
        return Err(Unsupported::Type("invalid pre-init null definition".into()));
    }
    let entry = match literal {
        Constant::Null => ("null".into(), JavaType::Class("java.lang.Object".into())),
        Constant::Zero => ("0".into(), JavaType::Int),
        _ => return Err(Unsupported::Type("non-contextual literal".into())),
    };
    e.values.insert(value, entry);
    Ok(())
}

fn inline<R: JavaResolver>(
    e: &mut Emitter<'_, R>,
    v: ValueId,
    expression: String,
    ty: JavaType,
) -> Result<(), Unsupported> {
    if e.values.contains_key(&v)
        || e.ir
            .value(v)
            .map_err(|x| Unsupported::Type(x.to_string()))?
            .def()
            != e.current
                .unwrap_or_else(|| unreachable!("constructor instruction is current"))
        || e.ty(v)? != ty
    {
        return Err(Unsupported::Type("invalid pre-init definition".into()));
    }
    e.values.insert(v, (expression, ty));
    Ok(())
}

#[cfg(test)]
#[path = "constructors_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "constructor_wide_arguments_tests.rs"]
mod wide_arguments_tests;

#[cfg(test)]
#[path = "constructor_contextual_prefix_tests.rs"]
mod contextual_prefix_tests;

#[cfg(test)]
#[path = "constructor_fcm_receiver_prefix_tests.rs"]
mod fcm_receiver_prefix_tests;

#[cfg(test)]
#[path = "constructor_arguments_guards_tests.rs"]
mod arguments_guards_tests;
