//! Caller-owned opt-in Java export measurements. Durations are elapsed, not CPU time.
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
/// Completed export measurements, including work preceding a returned error.
pub struct JavaProfile {
    /// Entire export call, excluding APK parsing and diagnostic serialization.
    pub wall: Duration,
    /// Non-overlapping caller-thread inventory, preparation and sink stages.
    pub serial: BTreeMap<&'static str, Duration>,
    /// Class task records in original input order, including filtered tasks.
    pub classes: Vec<ClassProfile>,
    /// Legacy fixed-batch records; persistent scheduling leaves this empty.
    pub batches: Vec<BatchProfile>,
    /// Actual persistent-worker accounting, one record per DEX.
    pub schedulers: Vec<crate::java_integration::scheduler::SchedulerProfile>,
    /// Whether this export retained method SSA results.
    pub ssa_cache_enabled: bool,
    /// Calls reusing an existing cache entry, including concurrent waits.
    pub ssa_cache_hits: usize,
    /// Distinct cache keys whose frontend pipeline was computed.
    pub ssa_cache_misses: usize,
    /// Entries retained until this export returned.
    pub ssa_cache_entries: usize,
    /// Failed frontend computations retained as typed errors.
    pub ssa_cache_failures: usize,
    /// Whether immutable Java member resolution results were retained.
    pub resolution_cache_enabled: bool,
    /// Member lookups reusing an existing method or field result.
    pub resolution_cache_hits: usize,
    /// Distinct method and field indices resolved during this export.
    pub resolution_cache_misses: usize,
    /// Cached member lookups whose result was unavailable.
    pub resolution_cache_failures: usize,
    /// Whether contextual field proofs were cached for this export.
    pub field_use_cache_enabled: bool,
    /// Completed resolution, source-spelling and receiver stage reuse counts.
    pub field_use_cache_hits: usize,
    /// Completed stage computations; multiple stages can share one full key.
    pub field_use_cache_misses: usize,
    /// Distinct full caller/receiver/staticness/symbolic-field keys retained.
    pub field_use_cache_entries: usize,
    /// Failed stage computations retained as their original typed errors.
    pub field_use_cache_failures: usize,
}

#[derive(Debug, Default)]
/// One class task; task elapsed time includes filtering and source collection.
pub struct ClassProfile {
    /// Input DEX ordinal.
    pub dex: usize,
    /// DEX class type-pool index.
    pub class: u32,
    /// Original DEX descriptor.
    pub descriptor: String,
    /// Offset from the DEX scheduler start; includes dispatch delay.
    pub start: Duration,
    /// Task start through completion; excludes thread startup and join delay.
    pub elapsed: Duration,
    /// Non-overlapping timed sections; remaining task time is unattributed.
    pub stages: BTreeMap<&'static str, Duration>,
    /// Decode/lift/merge and SSA substages nested within frontend; do not
    /// add these to frontend totals. Keys identify disjoint nested stages.
    pub ssa: BTreeMap<&'static str, Duration>,
    /// Method source attempts, including pre-frontend rejections.
    pub methods: usize,
}

#[derive(Debug, Default)]
/// A join barrier and its available worker-slot accounting.
pub struct BatchProfile {
    /// Input DEX ordinal.
    pub dex: usize,
    /// First class ordinal within that DEX.
    pub first_class: usize,
    /// Number of spawned tasks, including filtered classes.
    pub tasks: usize,
    /// Configured class-worker slots.
    pub capacity: usize,
    /// Spawn through final join, excludes ordered sink delivery.
    pub wall: Duration,
    /// Sum of elapsed task durations (not CPU time).
    pub task_time: Duration,
    /// Sum of (batch wall - task finish offset), one term per started task.
    pub finished_idle: Duration,
    /// Sum of task start offsets; includes thread creation and scheduling delay.
    pub startup_idle: Duration,
    /// Unused slots in a partial final batch.
    pub empty_slots: usize,
}

pub(crate) struct Timer<'a> {
    output: Option<&'a mut Duration>,
    start: Option<Instant>,
}
impl<'a> Timer<'a> {
    pub(crate) fn new(output: Option<&'a mut Duration>) -> Self {
        Self {
            start: output.as_ref().map(|_| Instant::now()),
            output,
        }
    }
}
impl Drop for Timer<'_> {
    fn drop(&mut self) {
        if let (Some(output), Some(start)) = (self.output.as_deref_mut(), self.start) {
            *output += start.elapsed();
        }
    }
}
pub(crate) fn stage<'a>(p: &'a mut Option<ClassProfile>, name: &'static str) -> Timer<'a> {
    Timer::new(p.as_mut().map(|p| p.stages.entry(name).or_default()))
}
