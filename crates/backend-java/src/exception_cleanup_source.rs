//! Checked contracts for exact original cleanup-source operations.
use super::*;
use managed_ir::ops::ArithOp;

/// Candidate proof only; super selection is attested again with the caller at emission.
pub(super) fn candidate(insn: &ManagedInsn, resolver: &impl JavaResolver) -> bool {
    match insn {
        ManagedInsn::Invoke {
            kind: managed_ir::ops::InvokeKind::Special,
            method,
            args,
            result,
        } => special_contract(*method, args, *result, None, resolver).is_some(),
        ManagedInsn::Invoke { kind, method, .. } => {
            matches!(
                kind,
                managed_ir::ops::InvokeKind::Static
                    | managed_ir::ops::InvokeKind::Virtual
                    | managed_ir::ops::InvokeKind::Interface
                    | managed_ir::ops::InvokeKind::Super
            ) && resolver.method(*method).is_some()
        }
        ManagedInsn::FieldGet { .. } | ManagedInsn::FieldPut { .. } => field(insn, resolver),
        // Structural candidates only. The final source contract below requires
        // the exact original ID and successful typed operation emission.
        ManagedInsn::Cast { target, .. } => reference_cast_target(*target, resolver).is_some(),
        ManagedInsn::ArrayPut { .. }
        | ManagedInsn::ArrayLength { .. }
        | ManagedInsn::ArrayGet { .. } => true,
        // Structural only: the emitted contract retains the exact thrown class.
        ManagedInsn::Throw { .. } => true,
        ManagedInsn::NewArray { element, .. } => resolver
            .instruction_type(*element)
            .is_some_and(|ty| ty != JavaType::Void),
        ManagedInsn::Arithmetic {
            op: ArithOp::Div | ArithOp::Rem,
            ..
        } => true,
        ManagedInsn::NewFilledArray { target, .. } => {
            matches!(resolver.instruction_type(*target), Some(JavaType::Array(_)))
        }
        ManagedInsn::Const {
            value: managed_ir::ops::Constant::String(s),
            ..
        } => resolver.string(*s).is_some(),
        ManagedInsn::Const {
            value: managed_ir::ops::Constant::ClassLiteral(target),
            ..
        } => crate::class_literal::expression(resolver, *target, JavaType::source).is_ok(),
        ManagedInsn::NewInstance { class, .. } => resolver.class(*class).is_some(),
        _ => false,
    }
}

pub(super) fn field(insn: &ManagedInsn, resolver: &impl JavaResolver) -> bool {
    let (reference, object, store) = match insn {
        ManagedInsn::FieldGet { field, object, .. } => (*field, object, false),
        ManagedInsn::FieldPut { field, object, .. } => (*field, object, true),
        _ => return false,
    };
    let (Some(declaration), Some(flags)) =
        (resolver.field(reference), resolver.field_access(reference))
    else {
        return false;
    };
    if declaration.is_static != object.is_none()
        || declaration.is_static != (flags & 8 != 0)
        || (store && flags & 0x10 != 0)
    {
        return false;
    }
    // A source constant read could remove the original class-initialization effect.
    !declaration.is_static
        || store
        || flags & 0x10 == 0
        || resolver.field_source_nonconstant(reference)
        || matches!(declaration.ty, JavaType::Array(_))
        || matches!(&declaration.ty, JavaType::Class(name) if name != "java.lang.String")
}

pub(crate) fn contract(
    insn: &ManagedInsn,
    caller: &str,
    resolver: &impl JavaResolver,
) -> Option<Vec<String>> {
    match insn {
        ManagedInsn::Invoke {
            kind: managed_ir::ops::InvokeKind::Super,
            method,
            ..
        } => resolver
            .super_invocation(caller, *method)
            .map(|selected| selected.method.throws),
        ManagedInsn::Invoke {
            kind:
                managed_ir::ops::InvokeKind::Static
                | managed_ir::ops::InvokeKind::Virtual
                | managed_ir::ops::InvokeKind::Interface,
            method,
            ..
        } => resolver.method(*method).map(|m| m.throws),
        ManagedInsn::Invoke {
            kind: managed_ir::ops::InvokeKind::Special,
            method,
            args,
            result,
        } => special_contract(*method, args, *result, Some(caller), resolver),
        ManagedInsn::FieldGet { .. } | ManagedInsn::FieldPut { .. } if field(insn, resolver) => {
            Some(vec![])
        }
        _ => None,
    }
}

/// The completed allocation/direct-private emitter independently validates the
/// receiver, exact allocation pairing and descriptor-typed argument casts.
/// This supplies its exact declaration contract; it cannot select a virtual or
/// superclass target from a Special opcode.
fn special_contract(
    reference: managed_ir::refs::MethodRef,
    args: &[ValueId],
    result: Option<ValueId>,
    caller: Option<&str>,
    resolver: &impl JavaResolver,
) -> Option<Vec<String>> {
    let method = resolver.method(reference)?;
    let flags = resolver.method_access(reference)?;
    if method.is_static || flags & (8 | 0x400) != 0 {
        return None;
    }
    let operands = crate::invoke_arguments::operands(&method, args).ok()?;
    if operands.len() != method.parameters.len() + 1
        || operands.is_empty()
        || method.parameters.contains(&JavaType::Void)
    {
        return None;
    }
    if method.name == "<init>" {
        if method.return_type != JavaType::Void || result.is_some() || flags & 0x100 != 0 {
            return None;
        }
        // Public allocation or same-owner initialization has a source access
        // witness. Other constructor accessibility is not inferred here.
        if flags & 1 == 0
            && caller.is_some_and(|caller| {
                caller != method.owner
                    && (flags & 2 != 0
                        || caller.rsplit_once('.').map(|x| x.0)
                            != method.owner.rsplit_once('.').map(|x| x.0))
            })
        {
            return None;
        }
    } else {
        if method.name == "<clinit>"
            || flags & 2 == 0
            || caller.is_some_and(|caller| caller != method.owner)
        {
            return None;
        }
    }
    Some(method.throws)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{JavaField, JavaMethod, SuperInvocation};
    use managed_ir::refs::{ClassRef, FieldRef, MethodRef, StringRef};
    struct Resolver {
        flags: Option<u32>,
        static_field: bool,
        nonconstant: bool,
        selected: Option<Vec<String>>,
    }
    impl JavaResolver for Resolver {
        fn class(&self, _: ClassRef) -> Option<String> {
            None
        }
        fn string(&self, _: StringRef) -> Option<Vec<u16>> {
            None
        }
        fn field(&self, _: FieldRef) -> Option<JavaField> {
            Some(JavaField {
                owner: "Owner".into(),
                name: "flag".into(),
                ty: JavaType::Boolean,
                is_static: self.static_field,
            })
        }
        fn field_access(&self, _: FieldRef) -> Option<u32> {
            self.flags
        }
        fn field_source_nonconstant(&self, _: FieldRef) -> bool {
            self.nonconstant
        }
        fn method(&self, _: MethodRef) -> Option<JavaMethod> {
            Some(JavaMethod {
                owner: "Parent".into(),
                name: "callback".into(),
                is_static: false,
                parameters: vec![],
                return_type: JavaType::Void,
                throws: vec![],
            })
        }
        fn super_invocation(&self, caller: &str, r: MethodRef) -> Option<SuperInvocation> {
            if caller != "Child" {
                return None;
            }
            let mut method = self.method(r)?;
            method.throws = self.selected.clone()?;
            Some(SuperInvocation {
                method,
                access_flags: 1,
            })
        }
    }
    fn resolver() -> Resolver {
        Resolver {
            flags: Some(2),
            static_field: false,
            nonconstant: false,
            selected: Some(vec![]),
        }
    }
    #[test]
    fn genuine_nonfinal_store_requires_exact_flags_and_staticness() {
        let put = ManagedInsn::FieldPut {
            object: Some(ValueId::default()),
            field: FieldRef::new(0),
            value: ValueId::default(),
        };
        let mut r = resolver();
        assert_eq!(contract(&put, "Child", &r), Some(vec![]));
        r.flags = None;
        assert!(!candidate(&put, &r));
        r.flags = Some(0x12);
        assert!(!candidate(&put, &r));
        r.flags = Some(8);
        assert!(!candidate(&put, &r));
        r.flags = Some(2);
        r.static_field = true;
        assert!(!candidate(&put, &r));
    }
    #[test]
    fn static_constant_read_needs_nonconstant_source_attestation() {
        let get = ManagedInsn::FieldGet {
            object: None,
            field: FieldRef::new(0),
            result: ValueId::default(),
        };
        let mut r = resolver();
        r.static_field = true;
        r.flags = Some(0x19);
        assert!(!candidate(&get, &r));
        r.nonconstant = true;
        assert_eq!(contract(&get, "Child", &r), Some(vec![]));
    }
    #[test]
    fn super_contract_is_source_selected_not_symbolic() {
        let invoke = ManagedInsn::Invoke {
            kind: managed_ir::ops::InvokeKind::Super,
            method: MethodRef::new(0),
            args: vec![],
            result: None,
        };
        let mut r = resolver();
        assert!(candidate(&invoke, &r));
        r.selected = Some(vec!["java.io.IOException".into()]);
        assert_eq!(contract(&invoke, "Child", &r), r.selected);
        assert_eq!(contract(&invoke, "Unproved", &r), None);
        r.selected = None;
        assert_eq!(contract(&invoke, "Child", &r), None);
    }
}

#[cfg(test)]
#[path = "exception_special_contract_tests.rs"]
mod special_tests;

/// Source-bound contract tied to the exact original operation and cached
/// allocation plan. No declaration-only constructor attestation can authorize
/// a different initializer or an unpaired object.
pub(crate) fn emitted_contract<R: JavaResolver>(
    emitter: &crate::Emitter<'_, R>,
    caller: &str,
    id: InsnId,
    insn: &ManagedInsn,
) -> Option<Vec<String>> {
    if emitter.ir.instruction(id).ok()? != insn {
        return None;
    }
    if let ManagedInsn::Throw { value } = insn {
        if emitter.current != Some(id) {
            return None;
        }
        let (_, ty @ JavaType::Class(_)) = emitter.get(*value).ok()? else {
            return None;
        };
        if !emitter
            .resolver
            .assignable(&ty, &JavaType::Class("java.lang.Throwable".into()))
        {
            return None;
        }
        let JavaType::Class(name) = ty else {
            return None;
        };
        // Even unchecked classes retain their exact source domain here. A
        // checked IOException must never become an empty operation contract.
        return Some(vec![name]);
    }
    if operation_candidate(insn) {
        return emitted_operation(emitter, id, insn).then(Vec::new);
    }
    if let ManagedInsn::Invoke {
        kind: managed_ir::ops::InvokeKind::Special,
        method,
        args,
        ..
    } = insn
    {
        let declaration = emitter.resolver.method(*method)?;
        if declaration.name == "<init>" {
            let root = emitter.allocation_root(*args.first()?)?;
            let plan = emitter.object_plan(root).ok()?;
            if plan.initializer != id || plan.owner != declaration.owner {
                return None;
            }
        } else {
            let flags = emitter.resolver.method_access(*method)?;
            let selected = crate::special_invokes::DispatchContext {
                caller,
                declaration_owner: &declaration.owner,
                declaration_name: &declaration.name,
                access_flags: flags,
                receiver_is_this: false,
                super_resolver: None,
            }
            .plan(managed_ir::ops::InvokeKind::Special)
            .ok()?;
            if selected != crate::special_invokes::InvocationPlan::DirectPrivate {
                return None;
            }
        }
    }
    contract(insn, caller, emitter.resolver)
}

fn reference_cast_target(
    target: managed_ir::refs::TypeRef,
    resolver: &impl JavaResolver,
) -> Option<JavaType> {
    let ty = resolver.instruction_type(target)?;
    if !matches!(ty, JavaType::Class(_) | JavaType::Array(_)) || ty.source().is_err() {
        return None;
    }
    Some(ty)
}

fn operation_candidate(insn: &ManagedInsn) -> bool {
    matches!(
        insn,
        ManagedInsn::Cast { .. }
            | ManagedInsn::ArrayPut { .. }
            | ManagedInsn::ArrayLength { .. }
            | ManagedInsn::ArrayGet { .. }
            | ManagedInsn::NewArray { .. }
            | ManagedInsn::Arithmetic {
                op: ArithOp::Div | ArithOp::Rem,
                ..
            }
            | ManagedInsn::NewFilledArray { .. }
            | ManagedInsn::NewInstance { .. }
            | ManagedInsn::Const {
                value: managed_ir::ops::Constant::String(_),
                ..
            }
            | ManagedInsn::Const {
                value: managed_ir::ops::Constant::ClassLiteral(_),
                ..
            }
    )
}

/// These exact operations emit Java primitive/array/literal syntax, rather
/// than an unresolved call. Current identity is attested after typed emission;
/// no unknown operation or opaque effect obtains an empty throws contract.
fn emitted_operation<R: JavaResolver>(
    emitter: &crate::Emitter<'_, R>,
    id: InsnId,
    insn: &ManagedInsn,
) -> bool {
    if emitter.current != Some(id) {
        return false;
    }
    match insn {
        ManagedInsn::Cast {
            result,
            value,
            target,
        } => {
            let Some(target) = reference_cast_target(*target, emitter.resolver) else {
                return false;
            };
            // The exact original cast has already passed typed emission. Its
            // operand stays an existing available reference, not an unboxed
            // scalar or synthesized default. ClassCastException/linkage Errors
            // remain routed through the original handler and prethrow state.
            emitter.get(*value).is_ok_and(|(_, ty)| ty.reference())
                && emitter.ty(*result).is_ok_and(|ty| ty == target)
        }
        ManagedInsn::NewArray {
            result,
            element,
            length,
        } => {
            let (Some(target), Ok(ty)) = (
                emitter.resolver.instruction_type(*element),
                emitter.ty(*result),
            ) else {
                return false;
            };
            let JavaType::Array(_) = &ty else {
                return false;
            };
            (target == ty || JavaType::Array(Box::new(target)) == ty)
                && ty.source().is_ok()
                && emitter.get(*length).is_ok_and(|(_, ty)| {
                    matches!(
                        ty,
                        JavaType::Byte | JavaType::Short | JavaType::Char | JavaType::Int
                    )
                })
        }
        ManagedInsn::Arithmetic {
            op: op @ (ArithOp::Div | ArithOp::Rem),
            result,
            lhs,
            rhs,
        } => {
            let Ok(ty @ (JavaType::Int | JavaType::Long)) = emitter.ty(*result) else {
                return false;
            };
            let (Ok((_, left)), Ok((_, right))) =
                (emitter.operand(*lhs, &ty), emitter.operand(*rhs, &ty))
            else {
                return false;
            };
            crate::arithmetic_operands::validate(*op, &ty, &left, &right).is_ok()
        }
        ManagedInsn::ArrayGet {
            result,
            array,
            index,
        } => {
            let Ok((_, JavaType::Array(component))) = emitter.get(*array) else {
                return false;
            };
            let Ok(result) = emitter.ty(*result) else {
                return false;
            };
            (*component == result
                || (matches!(
                    *component,
                    JavaType::Byte | JavaType::Short | JavaType::Char
                ) && result == JavaType::Int))
                && emitter.get(*index).is_ok_and(|(_, ty)| {
                    matches!(
                        ty,
                        JavaType::Byte | JavaType::Short | JavaType::Char | JavaType::Int
                    )
                })
        }
        ManagedInsn::ArrayLength { result, array } => {
            emitter
                .get(*array)
                .is_ok_and(|(_, ty)| matches!(ty, JavaType::Array(_)))
                && emitter.ty(*result).is_ok_and(|ty| ty == JavaType::Int)
        }
        ManagedInsn::ArrayPut {
            array,
            index,
            value,
        } => {
            let Ok((_, JavaType::Array(component))) = emitter.get(*array) else {
                return false;
            };
            emitter.get(*index).is_ok_and(|(_, ty)| {
                matches!(
                    ty,
                    JavaType::Byte | JavaType::Short | JavaType::Char | JavaType::Int
                )
            }) && emitter.operand(*value, &component).is_ok()
        }
        ManagedInsn::NewFilledArray {
            result: Some(result),
            target,
            elements,
        } => {
            let Some(JavaType::Array(component)) = emitter.resolver.instruction_type(*target)
            else {
                return false;
            };
            !matches!(
                *component,
                JavaType::Void | JavaType::Long | JavaType::Double
            ) && emitter
                .ty(*result)
                .is_ok_and(|ty| ty == JavaType::Array(component.clone()))
                && elements.iter().all(|value| emitter.get(*value).is_ok())
        }
        ManagedInsn::Const {
            result,
            value: managed_ir::ops::Constant::String(reference),
        } => {
            emitter
                .ty(*result)
                .is_ok_and(|ty| ty == JavaType::Class("java.lang.String".into()))
                && emitter
                    .resolver
                    .string(*reference)
                    .is_some_and(|units| crate::java_string_literal::render(&units).is_ok())
        }
        ManagedInsn::Const {
            result,
            value: managed_ir::ops::Constant::ClassLiteral(target),
        } => {
            emitter
                .ty(*result)
                .is_ok_and(|ty| ty == JavaType::Class("java.lang.Class".into()))
                && crate::class_literal::expression(emitter.resolver, *target, JavaType::source)
                    .is_ok()
        }
        ManagedInsn::NewInstance { result, class } => {
            let Ok(plan) = emitter.object_plan(id) else {
                return false;
            };
            if plan.allocation != id
                || emitter.allocation_root(*result) != Some(id)
                || emitter.resolver.class(*class).as_deref() != Some(plan.owner.as_str())
            {
                return false;
            }
            // A deferred allocation may not cross a different original catch
            // boundary when its source expression is emitted at initialization.
            fn handlers(
                ir: &MethodIr<SsaForm>,
                id: InsnId,
            ) -> Option<Vec<(decomp_core::cfg::EdgeKind, decomp_core::ids::BlockId)>> {
                let mut blocks = ir
                    .cfg()
                    .block_ids()
                    .filter(|b| ir.cfg().block(*b).is_ok_and(|n| *n.payload() == id));
                let block = blocks.next()?;
                if blocks.next().is_some() {
                    return None;
                }
                ir.cfg()
                    .block(block)
                    .ok()?
                    .outgoing()
                    .iter()
                    .filter_map(|edge| match ir.cfg().edge(*edge) {
                        Ok(edge)
                            if matches!(edge.kind(), decomp_core::cfg::EdgeKind::Exception(_)) =>
                        {
                            Some(Some((edge.kind().clone(), edge.target())))
                        }
                        Ok(_) => None,
                        Err(_) => Some(None),
                    })
                    .collect()
            }
            matches!((handlers(emitter.ir,id),handlers(emitter.ir,plan.initializer)),(Some(a),Some(b)) if a==b)
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "exception_cleanup_operation_tests.rs"]
mod operation_tests;

#[cfg(test)]
#[path = "exception_cleanup_reference_cast_tests.rs"]
mod reference_cast_tests;

#[cfg(test)]
#[path = "exception_cleanup_class_literal_tests.rs"]
mod class_literal_tests;

#[cfg(test)]
#[path = "exception_cleanup_array_length_tests.rs"]
mod array_length_tests;

#[cfg(test)]
#[path = "exception_cleanup_integral_allocation_tests.rs"]
mod integral_allocation_tests;

#[cfg(test)]
#[path = "exception_cleanup_throw_array_get_tests.rs"]
mod throw_array_get_tests;
