//! Eager Java25 prologue operations. Values are materialized at their original
//! SSA sites; no effect expression is moved into the receiver initializer.
//! Ordinary allocation pairing retains its independent allocation/OOM proof.
use super::*;

pub(super) fn emit<R: ConstructorResolver>(
    e: &mut Emitter<'_, R>,
    insn: &ManagedInsn,
    signature: &JavaMethod,
    aliases: &HashSet<ValueId>,
) -> Result<bool, Unsupported> {
    let fail = || Unsupported::Instruction("Java25 early-construction receiver use".into());
    // These have dedicated caller handling: this copies introduce no Java
    // `this` expression statement; own blank-field stores are allowed LHS uses;
    // the one receiver initializer is emitted as a top-level invocation.
    match insn {
        ManagedInsn::Move { value, .. } if aliases.contains(value) => return Ok(false),
        ManagedInsn::FieldPut {
            object: Some(object),
            ..
        } if aliases.contains(object) => {
            return Ok(false);
        }
        ManagedInsn::Invoke {
            kind: InvokeKind::Special,
            args,
            ..
        } if args.first().is_some_and(|v| aliases.contains(v)) => return Ok(false),
        ManagedInsn::Const {
            value: Constant::Null,
            ..
        } => return Ok(false),
        ManagedInsn::Const {
            result,
            value: Constant::Zero,
        } if e.ty(*result).is_err() => {
            return Ok(false);
        }
        ManagedInsn::Branch { .. } | ManagedInsn::Opaque { .. } => return Ok(false),
        _ => {}
    }
    let reads = match insn {
        ManagedInsn::Const { .. } | ManagedInsn::NewInstance { .. } => Vec::new(),
        ManagedInsn::Move { value, .. }
        | ManagedInsn::Neg { value, .. }
        | ManagedInsn::Not { value, .. }
        | ManagedInsn::Cast { value, .. }
        | ManagedInsn::InstanceOf { value, .. } => vec![*value],
        ManagedInsn::Arithmetic { lhs, rhs, .. }
        | ManagedInsn::Compare { lhs, rhs, .. }
        | ManagedInsn::OrderingCompare { lhs, rhs, .. } => vec![*lhs, *rhs],
        ManagedInsn::FieldGet { object, .. } => object.iter().copied().collect(),
        ManagedInsn::FieldPut { object, value, .. } => {
            object.iter().copied().chain([*value]).collect()
        }
        ManagedInsn::NewArray { length, .. } => vec![*length],
        ManagedInsn::NewFilledArray { elements, .. } => elements.clone(),
        ManagedInsn::ArrayGet { array, index, .. } => vec![*array, *index],
        ManagedInsn::ArrayPut {
            array,
            index,
            value,
        } => vec![*array, *index, *value],
        ManagedInsn::ArrayLength { array, .. } => vec![*array],
        ManagedInsn::FillArray { array, values } => std::iter::once(*array)
            .chain(values.iter().copied())
            .collect(),
        ManagedInsn::Invoke { args, .. } => args.clone(),
        _ => return Ok(false),
    };
    if reads.iter().any(|v| aliases.contains(v)) {
        return Err(fail());
    }
    match insn {
        ManagedInsn::Invoke { kind, method, .. } => {
            let target = e.resolver.method(*method).ok_or_else(fail)?;
            let flags = e.resolver.method_access(*method).ok_or_else(fail)?;
            let static_call = *kind == InvokeKind::Static;
            if target.is_static != static_call
                || (flags & 8 != 0) != static_call
                || !arguments::constructor_call_access(&signature.owner, &target.owner, flags)
                || target
                    .throws
                    .iter()
                    .any(|t| !e.resolver.exception_covered(t, &signature.throws))
            {
                return Err(Unsupported::Metadata(
                    "Java25 early invoke declaration/access/exceptions".into(),
                ));
            }
        }
        ManagedInsn::FieldGet { field, .. } | ManagedInsn::FieldPut { field, .. } => {
            let target = e.resolver.field(*field).ok_or_else(fail)?;
            let flags = e.resolver.field_access(*field).ok_or_else(fail)?;
            if !arguments::constructor_call_access(&signature.owner, &target.owner, flags)
                || matches!(insn, ManagedInsn::FieldPut { .. }) && flags & 0x10 != 0
                // Java constant-variable reads may erase an original sget and
                // its class-initialization boundary. Exact source declaration
                // evidence can attest a blank final or nonconstant initializer;
                // flags and absent DEX encoded values alone cannot.
                || matches!(insn, ManagedInsn::FieldGet { object: None, .. })
                    && flags & 0x10 != 0
                    && (!target.ty.reference() || target.ty == JavaType::Class("java.lang.String".into()))
                    && !e.resolver.field_source_nonconstant(*field)
            {
                return Err(Unsupported::Metadata(
                    "Java25 early field source/access proof".into(),
                ));
            }
        }
        _ => {}
    }
    // The shared instruction emitter defines a local for each effect result.
    // Its object planner independently validates uninitialized new references,
    // argument traces, exception scopes, and allocation-before-arguments order.
    if e.instruction(insn, signature)? {
        return Err(Unsupported::ControlFlow(
            "Java25 prologue terminated before initializer".into(),
        ));
    }
    Ok(true)
}
