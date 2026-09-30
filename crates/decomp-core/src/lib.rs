//! Family-neutral decompiler core: typed arena CFG, graph analyses, pass
//! registry, diagnostics.
//!
//! Invariant: this crate must never depend on an IR dialect (`dex-format`,
//! future native crates). Dependency direction is strictly upward:
//! `core <- analysis/pipeline <- family IR <- frontend/backend <- CLI`.
//!
//! CFG mutations go exclusively through the [`crate::cfg`] API; every
//! mutation bumps a revision so analyses can be invalidated. Edges are
//! first-class records (switch labels, exceptional flow, and fallthrough are
//! distinguishable).

pub mod analysis;
pub mod cfg;
pub mod diagnostics;
pub mod ids;
pub mod pipeline;
