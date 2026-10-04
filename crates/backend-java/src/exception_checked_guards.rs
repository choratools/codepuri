//! Legal Java precise-rethrow dispatch for exact zero-checked-contract scopes.
use super::{JavaType, RegionStep};
use crate::{
    JavaResolver,
    type_compatibility::{Compatibility, Hierarchy},
};
use managed_ir::{
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
};

pub(super) fn eligible(
    ir: &MethodIr<SsaForm>,
    body: &[RegionStep],
    types: &[JavaType],
    hierarchy: &Hierarchy,
    resolver: &impl JavaResolver,
) -> bool {
    if types.is_empty() {
        return false;
    }
    let exception = JavaType::Class("java.lang.Exception".into());
    let runtime = JavaType::Class("java.lang.RuntimeException".into());
    if !types.iter().all(|ty| {
        hierarchy.assignable(ty, &exception) == Compatibility::Assignable
            && hierarchy.assignable(ty, &runtime) == Compatibility::Incompatible
    }) {
        return false;
    }
    fn known(ir: &MethodIr<SsaForm>, body: &[RegionStep], resolver: &impl JavaResolver) -> bool {
        body.iter().all(|step| match step {
            RegionStep::Instruction(id) => match ir.instruction(*id) {
                Ok(ManagedInsn::Invoke { method, .. }) => resolver
                    .method(*method)
                    .is_some_and(|target| target.throws.is_empty()),
                // Reference resolution/allocation can throw Errors, which these
                // original Exception handlers cannot catch. Retain their sites.
                Ok(ManagedInsn::NewInstance { class, .. }) => resolver.class(*class).is_some(),
                Ok(ManagedInsn::FieldGet { field, .. } | ManagedInsn::FieldPut { field, .. }) => {
                    resolver.field(*field).is_some()
                }
                Ok(
                    ManagedInsn::Cast { .. }
                    | ManagedInsn::Opaque { .. }
                    | ManagedInsn::MonitorEnter { .. }
                    | ManagedInsn::MonitorExit { .. }
                    | ManagedInsn::Throw { .. },
                )
                | Err(_) => false,
                Ok(_) => true,
            },
            RegionStep::Copies(_) => true,
            RegionStep::If {
                then_body,
                else_body,
                ..
            } => known(ir, then_body, resolver) && known(ir, else_body, resolver),
            _ => false,
        })
    }
    known(ir, body, resolver)
}
