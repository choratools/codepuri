use crate::infer::TypeContext;
use managed_ir::{types::ManagedType, value::ValueId};
use std::collections::HashMap;
pub(super) struct InitialContext<'a> {
    pub base: &'a dyn TypeContext,
    pub seeds: HashMap<ValueId, ManagedType>,
}
impl TypeContext for InitialContext<'_> {
    fn initial_type(&self, value: ValueId) -> Option<ManagedType> {
        self.seeds
            .get(&value)
            .cloned()
            .or_else(|| self.base.initial_type(value))
    }
    fn class_literal_type(&self) -> Option<ManagedType> {
        self.base.class_literal_type()
    }
    fn string_type(&self) -> Option<ManagedType> {
        self.base.string_type()
    }
    fn type_at(&self, index: u32) -> Option<ManagedType> {
        self.base.type_at(index)
    }
    fn field_type(&self, index: u32) -> Option<ManagedType> {
        self.base.field_type(index)
    }
    fn method_return(&self, index: u32) -> Option<ManagedType> {
        self.base.method_return(index)
    }
    fn method_params(&self, index: u32, params: &mut Vec<ManagedType>) -> bool {
        self.base.method_params(index, params)
    }
    fn enclosing_return(&self) -> Option<ManagedType> {
        self.base.enclosing_return()
    }
    fn note_unresolved(&self, kind: &'static str, index: u32) {
        self.base.note_unresolved(kind, index);
    }
    fn take_unresolved(&self) -> Vec<(&'static str, u64)> {
        self.base.take_unresolved()
    }
}
