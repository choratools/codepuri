//! Descriptor contexts for type inference.
//!
//! [`NoTypes`] provides no pool resolution (every lookup is `None`), which
//! keeps inference usable without a DEX model. [`DexTypes`] resolves type,
//! field, and method pool indexes against one parsed `DexFile` and records
//! unresolvable references so callers can surface them as diagnostics
//! instead of silently dropping the constraints.

use std::cell::RefCell;
use std::collections::HashMap;

use dex_format::DexFile;
use managed_ir::refs::ClassRef;
use managed_ir::types::ManagedType;

use super::TypeContext;
use super::descriptor::parse;

/// A context with no pool resolution.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTypes;

impl TypeContext for NoTypes {}

/// Pool resolution backed by one parsed DEX file.
#[derive(Debug)]
pub struct DexTypes<'a> {
    dex: &'a DexFile,
    by_descriptor: HashMap<&'a str, u32>,
    enclosing_return: Option<ManagedType>,
    string_type: Option<ManagedType>,
    type_cache: RefCell<HashMap<u32, Option<ManagedType>>>,
    unresolved: RefCell<HashMap<&'static str, u64>>,
}

impl<'a> DexTypes<'a> {
    /// Builds a context for `dex`, optionally naming the enclosing method
    /// for return-type constraints.
    pub fn new(dex: &'a DexFile, enclosing_method: Option<u32>) -> Self {
        let mut by_descriptor = HashMap::with_capacity(dex.type_ids.len());
        for (index, type_id) in dex.type_ids.iter().enumerate() {
            if let Ok(descriptor) = dex.string(type_id.descriptor_idx) {
                by_descriptor.insert(descriptor, index as u32);
            }
        }
        let string_type = by_descriptor
            .get("Ljava/lang/String;")
            .map(|index| ManagedType::Reference(ClassRef::new(*index)));
        let reverse = |descriptor: &str| by_descriptor.get(descriptor).copied();
        let enclosing_return = enclosing_method.and_then(|method_idx| {
            let method = dex.method_id(method_idx).ok()?;
            let proto = dex.proto_id(u32::from(method.proto_idx)).ok()?;
            Self::type_of(dex, &reverse, proto.return_type_idx)
        });
        Self {
            dex,
            by_descriptor,
            enclosing_return,
            string_type,
            type_cache: RefCell::new(HashMap::new()),
            unresolved: RefCell::new(HashMap::new()),
        }
    }

    fn type_of(
        dex: &DexFile,
        reverse: &dyn Fn(&str) -> Option<u32>,
        type_idx: u32,
    ) -> Option<ManagedType> {
        let type_id = dex.type_id(type_idx).ok()?;
        let descriptor = dex.string(type_id.descriptor_idx).ok()?;
        parse(descriptor, reverse)
    }

    fn resolve_type(&self, type_idx: u32) -> Option<ManagedType> {
        let reverse = |descriptor: &str| self.by_descriptor.get(descriptor).copied();
        Self::type_of(self.dex, &reverse, type_idx)
    }
}

impl TypeContext for DexTypes<'_> {
    fn string_type(&self) -> Option<ManagedType> {
        self.string_type.clone()
    }

    fn type_at(&self, type_idx: u32) -> Option<ManagedType> {
        if let Some(cached) = self.type_cache.borrow().get(&type_idx) {
            return cached.clone();
        }
        let resolved = self.resolve_type(type_idx);
        if resolved.is_none() {
            *self.unresolved.borrow_mut().entry("type").or_default() += 1;
        }
        self.type_cache
            .borrow_mut()
            .insert(type_idx, resolved.clone());
        resolved
    }

    fn field_type(&self, field_idx: u32) -> Option<ManagedType> {
        let field = self.dex.field_id(field_idx).ok()?;
        self.type_at(u32::from(field.type_idx))
    }

    fn method_return(&self, method_idx: u32) -> Option<ManagedType> {
        let method = self.dex.method_id(method_idx).ok()?;
        let proto = self.dex.proto_id(u32::from(method.proto_idx)).ok()?;
        self.type_at(proto.return_type_idx)
    }

    fn method_params(&self, method_idx: u32, params: &mut Vec<ManagedType>) -> bool {
        let Some(method) = self.dex.method_id(method_idx).ok() else {
            return false;
        };
        let Ok(proto) = self.dex.proto_id(u32::from(method.proto_idx)) else {
            return false;
        };
        let Some(parameters) = &proto.parameters else {
            params.clear();
            return true;
        };
        params.clear();
        params.extend(
            parameters
                .types
                .iter()
                .map(|ty| self.type_at(u32::from(*ty)).unwrap_or(ManagedType::Unknown)),
        );
        true
    }

    fn enclosing_return(&self) -> Option<ManagedType> {
        self.enclosing_return.clone()
    }

    fn note_unresolved(&self, kind: &'static str, index: u32) {
        let _ = index;
        *self.unresolved.borrow_mut().entry(kind).or_default() += 1;
    }

    fn take_unresolved(&self) -> Vec<(&'static str, u64)> {
        let mut counts: Vec<_> = self.unresolved.borrow_mut().drain().collect();
        counts.sort_unstable();
        counts
    }
}
