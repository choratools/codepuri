//! Sparse original-body witnesses for target-bound source-only Throwable bridges.
//! Build once per complete input/catalog. No declaration or runtime facts change.
use crate::{
    java_aliases::MethodKey, java_external_metadata::MethodMetadata,
    java_members::source_closure::method_key,
};
use backend_java::{
    JavaResolver,
    checked_source_bridge::{BoundTarget, FieldThrowDefinition},
};
use dex_format::DexFile;
use managed_ir::refs::MethodRef;
use std::collections::BTreeMap;
fn descriptor(dex: &DexFile, index: u32) -> Option<&str> {
    dex.string(dex.type_id(index).ok()?.descriptor_idx).ok()
}
/// Sparse export-local catalog of complete original field-throw bodies.
#[derive(Default)]
pub struct FieldThrowCatalog {
    definitions: BTreeMap<MethodKey, Option<FieldThrowDefinition>>,
}
impl FieldThrowCatalog {
    /// Derive full code/pool/flags/encoded-throws evidence from genuine definitions.
    /// Duplicate owners or declarations remain unavailable rather than selecting one.
    pub fn new(files: &[DexFile], metadata: &BTreeMap<MethodKey, Option<MethodMetadata>>) -> Self {
        let mut owners: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
        for (di, dex) in files.iter().enumerate() {
            for (ci, c) in dex.classes.iter().enumerate() {
                if let Some(owner) = descriptor(dex, c.class_idx) {
                    owners.entry(owner.into()).or_default().push((di, ci));
                }
            }
        }
        let mut definitions = BTreeMap::new();
        for (di, dex) in files.iter().enumerate() {
            for c in &dex.classes {
                let Some(data) = &c.class_data else { continue };
                for m in data.direct_methods.iter().chain(&data.virtual_methods) {
                    let Some(code) = &m.code else { continue };
                    if code.insns.len() != 20 || code.registers_size != 2 || code.ins_size != 1 {
                        continue;
                    }
                    let words: Vec<u16> = code
                        .insns
                        .chunks_exact(2)
                        .map(|p| u16::from_le_bytes([p[0], p[1]]))
                        .collect();
                    if words[0] != 0x1020
                        || words[2] != 0x39
                        || words[3] != 3
                        || words[4] != 14
                        || words[5] != 0x11f
                        || words[7] != 0x1154
                        || words[9] != 0x127
                        || words[1] != words[6]
                    {
                        continue;
                    }
                    let Some(key) = method_key(dex, m.method_idx) else {
                        continue;
                    };
                    if descriptor(dex, c.class_idx) != Some(key.owner.as_str()) {
                        continue;
                    }
                    let definition = (|| {
                        if owners.get(&key.owner)?.len() != 1 {
                            return None;
                        }
                        let md = metadata.get(&key)?.as_ref()?;
                        if md.access_flags != m.access_flags {
                            return None;
                        }
                        let original_throws = md.exceptions.clone()?;
                        let checked_type = descriptor(dex, u32::from(words[1]))?.to_owned();
                        let field = dex.field_id(u32::from(words[8])).ok()?;
                        let field_owner = descriptor(dex, u32::from(field.class_idx))?.to_owned();
                        let field_type = descriptor(dex, u32::from(field.type_idx))?.to_owned();
                        let location = owners.get(&field_owner)?;
                        if location.len() != 1 {
                            return None;
                        }
                        let (fd, fc) = location[0];
                        let field_data = files[fd].classes[fc].class_data.as_ref()?;
                        let field_name = dex.string(field.name_idx).ok()?;
                        let mut declarations = 0;
                        for f in &field_data.instance_fields {
                            let raw = files[fd].field_id(f.field_idx).ok()?;
                            if descriptor(&files[fd], u32::from(raw.class_idx))? == field_owner
                                && descriptor(&files[fd], u32::from(raw.type_idx))? == field_type
                                && files[fd].string(raw.name_idx).ok()? == field_name
                                && f.access_flags & 8 == 0
                            {
                                declarations += 1
                            }
                        }
                        if declarations != 1 {
                            return None;
                        }
                        Some(FieldThrowDefinition {
                            original_identity: format!("input-dex-{di}#{} {key:?}", m.method_idx),
                            code_offset: m.code_off,
                            access_flags: m.access_flags,
                            registers: code.registers_size,
                            incoming: code.ins_size,
                            words: words.clone(),
                            handlers: code.handlers.len() + code.tries.len(),
                            original_throws,
                            checked_type: checked_type.clone(),
                            cast_type: checked_type,
                            field_owner,
                            field_type,
                            checked_type_index: words[1],
                            field_index: words[8],
                        })
                    })();
                    use std::collections::btree_map::Entry;
                    match definitions.entry(key) {
                        Entry::Vacant(e) => {
                            e.insert(definition);
                        }
                        Entry::Occupied(mut e) => {
                            e.insert(None);
                        }
                    }
                }
            }
        }
        Self { definitions }
    }
    /// Bind a caller-local reference to its exact selected unchanged source target.
    /// Resolver already supplies original alias/access/dispatch selection proof.
    pub fn bind(
        &self,
        caller: &DexFile,
        reference: MethodRef,
        resolver: &impl JavaResolver,
    ) -> Option<BoundTarget> {
        let key = method_key(caller, reference.index())?;
        let evidence = self.definitions.get(&key)?.as_ref()?;
        let selected = resolver.method(reference)?;
        BoundTarget::field_throw(reference, selected, evidence, resolver).ok()
    }
    /// Number of genuine unambiguous structural candidate bodies (not recoveries).
    #[cfg(test)]
    pub fn candidate_count(&self) -> usize {
        self.definitions.values().filter(|d| d.is_some()).count()
    }
}
