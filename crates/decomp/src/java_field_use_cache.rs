//! Completed field-use proofs over one frozen input/metadata/alias snapshot.
//!
//! Publication and initialization status are deliberately absent. Recursive
//! hierarchy walkers still own their path-local cycle checks; only completed
//! top-level results, including typed failures, enter this cache.
use super::{FieldSourceUse, PublicationBlocker, source_use_indexed};
use crate::{
    java_aliases::{FieldKey, JavaAliases},
    java_external_metadata::{
        ExternalMetadata, FieldAccessContext, FieldOrigin, FieldRequirement, ResolvedField,
        fields::{InputIndex, resolve_field_indexed, retain_field_receiver_with_aliases_indexed},
    },
};
use dex_format::DexFile;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

#[cfg(test)]
#[path = "java_field_use_cache_tests.rs"]
mod tests;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Key {
    // The original symbolic owner must not collapse to the declaring owner.
    field: FieldKey,
    caller: String,
    receiver: Option<String>,
    is_static: bool,
}

impl Key {
    fn new(field: &FieldKey, context: &FieldAccessContext<'_>) -> Self {
        Self {
            field: field.clone(),
            caller: context.caller.into(),
            receiver: context.receiver.map(str::to_owned),
            is_static: context.is_static,
        }
    }
}

#[derive(Default)]
struct Entry {
    resolution: OnceLock<Result<ResolvedField, FieldRequirement>>,
    source_use: OnceLock<Result<FieldSourceUse, PublicationBlocker>>,
    retain: OnceLock<Result<bool, PublicationBlocker>>,
}

/// Counts are exact after workers finish; nested stages have separate counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct StageStats {
    pub(crate) hits: usize,
    pub(crate) misses: usize,
    pub(crate) failures: usize,
}

/// Keys may have only the stages that were actually requested initialized.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FieldUseStats {
    pub(crate) entries: usize,
    pub(crate) resolution: StageStats,
    pub(crate) source_use: StageStats,
    pub(crate) retain: StageStats,
}

#[derive(Default)]
struct Counters {
    hits: AtomicUsize,
    misses: AtomicUsize,
    failures: AtomicUsize,
}

impl Counters {
    fn complete<T: Clone, E: Clone>(
        &self,
        cell: &OnceLock<Result<T, E>>,
        compute: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        if let Some(result) = cell.get() {
            self.hits.fetch_add(1, Ordering::Relaxed);
            return result.clone();
        }
        let mut computed = false;
        let result = cell.get_or_init(|| {
            let result = compute();
            computed = true;
            self.misses.fetch_add(1, Ordering::Relaxed);
            if result.is_err() {
                self.failures.fetch_add(1, Ordering::Relaxed);
            }
            result
        });
        if !computed {
            self.hits.fetch_add(1, Ordering::Relaxed);
        }
        result.clone()
    }

    fn stats(&self) -> StageStats {
        StageStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
        }
    }
}

/// Scope one instance to immutable inputs, provider metadata and frozen aliases.
/// No mutation/publication capability is accepted. Changing any snapshot needs
/// a new instance; class selection and owner publication do not affect proofs.
pub(crate) struct FieldUseCache<'a> {
    enabled: bool,
    inputs: &'a [DexFile],
    external: &'a dyn ExternalMetadata,
    aliases: &'a JavaAliases,
    index: Option<&'a InputIndex<'a>>,
    entries: Mutex<BTreeMap<Key, Arc<Entry>>>,
    resolution: Counters,
    source_use: Counters,
    retain: Counters,
}

impl<'a> FieldUseCache<'a> {
    pub(crate) fn new(
        enabled: bool,
        inputs: &'a [DexFile],
        external: &'a dyn ExternalMetadata,
        aliases: &'a JavaAliases,
        index: Option<&'a InputIndex<'a>>,
    ) -> Self {
        Self {
            enabled,
            inputs,
            external,
            aliases,
            index,
            entries: Mutex::new(BTreeMap::new()),
            resolution: Counters::default(),
            source_use: Counters::default(),
            retain: Counters::default(),
        }
    }

    fn entry(&self, field: &FieldKey, context: &FieldAccessContext<'_>) -> Arc<Entry> {
        // No computation occurs while holding the map lock. An initializer
        // panic leaves its OnceLock uninitialized and retryable.
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(Key::new(field, context))
            .or_default()
            .clone()
    }

    /// Original-context descriptor/access proof. Failures remain distinct from
    /// later source spelling/receiver failures, preserving backend fallback.
    pub(crate) fn resolve(
        &self,
        field: &FieldKey,
        context: &FieldAccessContext<'_>,
    ) -> Result<ResolvedField, FieldRequirement> {
        let compute =
            || resolve_field_indexed(self.inputs, field, self.external, context, self.index);
        if !self.enabled {
            return compute();
        }
        let entry = self.entry(field, context);
        self.resolution.complete(&entry.resolution, compute)
    }

    /// Frozen source spelling and access proof; never owner publication status.
    pub(crate) fn source_use(
        &self,
        field: &FieldKey,
        context: &FieldAccessContext<'_>,
    ) -> Result<FieldSourceUse, PublicationBlocker> {
        let compute = || {
            let resolved = self
                .resolve(field, context)
                .map_err(PublicationBlocker::Resolution)?;
            source_use_indexed(
                self.inputs,
                &resolved,
                self.external,
                context,
                self.aliases,
                self.index,
            )
        };
        if !self.enabled {
            return compute();
        }
        let entry = self.entry(field, context);
        self.source_use.complete(&entry.source_use, compute)
    }

    /// Computes classpath receiver proof only on demand. Input proofs reuse the
    /// source-use stage. Call `resolve` first where failure invokes a fallback;
    /// a receiver/source failure after successful resolution means rejection.
    pub(crate) fn retain_receiver(
        &self,
        field: &FieldKey,
        context: &FieldAccessContext<'_>,
    ) -> Result<bool, PublicationBlocker> {
        let compute = || {
            let resolved = self
                .resolve(field, context)
                .map_err(PublicationBlocker::Resolution)?;
            if resolved.origin == FieldOrigin::Classpath {
                retain_field_receiver_with_aliases_indexed(
                    self.inputs,
                    &resolved,
                    self.external,
                    context,
                    self.aliases,
                    self.index,
                )
                .map_err(PublicationBlocker::Resolution)
            } else {
                self.source_use(field, context)
                    .map(|source| source.receiver == super::FieldReceiver::Retain)
            }
        };
        if !self.enabled {
            return compute();
        }
        let entry = self.entry(field, context);
        self.retain.complete(&entry.retain, compute)
    }

    pub(crate) fn stats(&self) -> FieldUseStats {
        FieldUseStats {
            entries: self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            resolution: self.resolution.stats(),
            source_use: self.source_use.stats(),
            retain: self.retain.stats(),
        }
    }
}
