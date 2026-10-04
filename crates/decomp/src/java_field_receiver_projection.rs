//! Reuse the backend's atomically proved selected-Phi declaration for access.
//! This is type evidence only. Actual body publication still requires the
//! backend's original-edge availability and successful-initializer proofs.
use backend_java::{JavaResolver, JavaType};
use managed_ir::{
    insn::ManagedInsn,
    method::{MethodIr, SsaForm},
    types::ManagedType,
    value::ValueId,
};
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Projection {
    declarations: Option<HashMap<ValueId, JavaType>>,
}
impl Projection {
    pub(super) fn selected_class<'a>(
        &'a mut self,
        ir: &MethodIr<SsaForm>,
        value: ValueId,
        return_type: &JavaType,
        resolver: &impl JavaResolver,
    ) -> Option<&'a str> {
        let original = ir.value(value).ok()?;
        if original.ty() != &ManagedType::Unknown
            || !matches!(
                ir.instruction(original.def()).ok()?,
                ManagedInsn::Phi { .. } | ManagedInsn::Move { .. }
            )
        {
            return None;
        }
        let map = self.declarations.get_or_insert_with(|| {
            backend_java::contextual_locals::declaration_plan(ir, return_type, resolver)
                .unwrap_or_default()
        });
        match map.get(&value)? {
            JavaType::Class(name) => Some(name),
            _ => None,
        }
    }
}
#[cfg(test)]
#[path = "java_field_receiver_projection_tests.rs"]
mod tests;
