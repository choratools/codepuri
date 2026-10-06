//! Descriptor contexts for type inference.
//!
//! [`NoTypes`] provides no pool resolution (every lookup is `None`), which
//! keeps inference usable without a DEX model. [`DexTypes`] resolves type,
//! field, and method pool indexes against one parsed `DexFile` and records
//! unresolvable references so callers can surface them as diagnostics
//! instead of silently dropping the constraints.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use dex_format::DexFile;
use managed_ir::refs::ClassRef;
use managed_ir::types::ManagedType;
use managed_ir::value::ValueId;

use super::TypeContext;
use super::descriptor::parse;

/// A context with no pool resolution.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTypes;

impl TypeContext for NoTypes {}

/// Immutable snapshot of one DEX pool. Build once and share by reference (or
/// put in an `Arc`) across method workers. No pointer-keyed or global cache is
/// used; subsequent mutations to the source DEX cannot change this snapshot.
#[derive(Debug, Clone)]
pub struct DexMetadata {
    types: Vec<Option<ManagedType>>,
    fields: Vec<u32>,
    methods: Vec<u32>,
    static_initializer_methods: Vec<u32>,
    dex_identity: usize,
    protos: Vec<(u32, Vec<u32>)>,
    string_type: Option<ManagedType>,
    class_literal_type: Option<ManagedType>,
}

impl DexMetadata {
    /// Decodes each type descriptor exactly once for this pool snapshot.
    pub fn new(dex: &DexFile) -> Self {
        let mut by_descriptor = HashMap::with_capacity(dex.type_ids.len());
        for (index, type_id) in dex.type_ids.iter().enumerate() {
            if let Ok(descriptor) = dex.string(type_id.descriptor_idx) {
                by_descriptor.insert(descriptor, index as u32);
            }
        }
        let reverse = |descriptor: &str| by_descriptor.get(descriptor).copied();
        let mut static_initializer_methods = Vec::new();
        let methods = dex
            .method_ids
            .iter()
            .enumerate()
            .map(|(index, method)| {
                if dex.string(method.name_idx).ok() == Some("<clinit>") {
                    static_initializer_methods.push(index as u32);
                }
                u32::from(method.proto_idx)
            })
            .collect();
        Self {
            types: dex
                .type_ids
                .iter()
                .map(|id| {
                    dex.string(id.descriptor_idx)
                        .ok()
                        .and_then(|s| parse(s, &reverse))
                })
                .collect(),
            fields: dex
                .field_ids
                .iter()
                .map(|id| u32::from(id.type_idx))
                .collect(),
            methods,
            static_initializer_methods,
            // Identity only. It is compared while the export owns the DEX and
            // is never dereferenced by this metadata snapshot.
            dex_identity: std::ptr::from_ref(dex) as usize,
            protos: dex
                .proto_ids
                .iter()
                .map(|p| {
                    (
                        p.return_type_idx,
                        p.parameters
                            .as_ref()
                            .map(|p| p.types.iter().map(|t| u32::from(*t)).collect())
                            .unwrap_or_default(),
                    )
                })
                .collect(),
            class_literal_type: by_descriptor
                .get("Ljava/lang/Class;")
                .map(|index| ManagedType::Reference(ClassRef::new(*index))),
            string_type: by_descriptor
                .get("Ljava/lang/String;")
                .map(|index| ManagedType::Reference(ClassRef::new(*index))),
        }
    }

    fn type_at(&self, index: u32) -> Option<ManagedType> {
        self.types.get(index as usize).cloned().flatten()
    }

    fn proto(&self, method: u32) -> Option<&(u32, Vec<u32>)> {
        self.protos
            .get(*self.methods.get(method as usize)? as usize)
    }
}

/// Method-local resolution state backed by immutable pool metadata.
#[derive(Debug)]
pub struct DexTypes<'a> {
    initial_registers: HashMap<(u16, bool), ManagedType>,
    initial_types: HashMap<ValueId, ManagedType>,
    metadata: Cow<'a, DexMetadata>,
    enclosing_return: Option<ManagedType>,
    enclosing_method: Option<u32>,
    string_type: Option<ManagedType>,
    unresolved_types: RefCell<HashSet<u32>>,
    unresolved: RefCell<HashMap<&'static str, u64>>,
}

impl<'a> DexTypes<'a> {
    /// Explicit validated incoming descriptor layout.
    pub fn with_initial_registers(mut self, registers: HashMap<(u16, bool), ManagedType>) -> Self {
        self.initial_registers = registers;
        self
    }
    /// Attach signature types only after the caller proves entry identities and
    /// physical word provenance. This never maps slots or guesses ValueIds.
    pub fn with_initial_types(mut self, initial_types: HashMap<ValueId, ManagedType>) -> Self {
        self.initial_types = initial_types;
        self
    }
    /// Builds a context for `dex`, optionally naming the enclosing method
    /// for return-type constraints.
    pub fn new(dex: &'a DexFile, enclosing_method: Option<u32>) -> Self {
        Self::from_metadata(Cow::Owned(DexMetadata::new(dex)), enclosing_method)
    }

    /// Creates cheap method-local state from a shared per-DEX snapshot.
    pub fn with_metadata(metadata: &'a DexMetadata, enclosing_method: Option<u32>) -> Self {
        Self::from_metadata(Cow::Borrowed(metadata), enclosing_method)
    }

    fn from_metadata(metadata: Cow<'a, DexMetadata>, enclosing_method: Option<u32>) -> Self {
        let enclosing_return = enclosing_method
            .and_then(|m| metadata.proto(m))
            .and_then(|p| metadata.type_at(p.0));
        let string_type = metadata.string_type.clone();
        Self {
            initial_registers: HashMap::new(),
            initial_types: HashMap::new(),
            metadata,
            enclosing_return,
            enclosing_method,
            string_type,
            unresolved_types: RefCell::new(HashSet::new()),
            unresolved: RefCell::new(HashMap::new()),
        }
    }

    fn resolve_type(&self, type_idx: u32) -> Option<ManagedType> {
        self.metadata.type_at(type_idx)
    }
}

impl TypeContext for DexTypes<'_> {
    fn original_static_initializer_identity(&self) -> Option<(usize, u32)> {
        if !self.initial_registers.is_empty() || !self.initial_types.is_empty() {
            return None;
        }
        let method = self.enclosing_method?;
        self.metadata
            .static_initializer_methods
            .binary_search(&method)
            .ok()
            .map(|_| (self.metadata.dex_identity, method))
    }

    fn initial_register_type(&self, register: u16, wide: bool) -> Option<ManagedType> {
        self.initial_registers.get(&(register, wide)).cloned()
    }
    fn initial_type(&self, value: ValueId) -> Option<ManagedType> {
        self.initial_types.get(&value).cloned()
    }
    fn class_literal_type(&self) -> Option<ManagedType> {
        self.metadata.class_literal_type.clone()
    }

    fn string_type(&self) -> Option<ManagedType> {
        self.string_type.clone()
    }

    fn type_at(&self, type_idx: u32) -> Option<ManagedType> {
        let resolved = self.resolve_type(type_idx);
        if resolved.is_none() && self.unresolved_types.borrow_mut().insert(type_idx) {
            *self.unresolved.borrow_mut().entry("type").or_default() += 1;
        }
        resolved
    }

    fn field_type(&self, field_idx: u32) -> Option<ManagedType> {
        self.type_at(*self.metadata.fields.get(field_idx as usize)?)
    }

    fn method_return(&self, method_idx: u32) -> Option<ManagedType> {
        self.type_at(self.metadata.proto(method_idx)?.0)
    }

    fn method_params(&self, method_idx: u32, params: &mut Vec<ManagedType>) -> bool {
        let Some(proto) = self.metadata.proto(method_idx) else {
            return false;
        };
        params.clear();
        params.extend(
            proto
                .1
                .iter()
                .map(|ty| self.type_at(*ty).unwrap_or(ManagedType::Unknown)),
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
