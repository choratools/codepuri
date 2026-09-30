//! Topology-only control-flow graph analyses.

mod dominance;
mod frontier;
mod loops;
mod post_dominance;
mod reducibility;
mod scc;
mod traversal;

pub use dominance::{Dominators, dominators};
pub use frontier::dominance_frontier;
pub use loops::{NaturalLoop, natural_loops};
pub use post_dominance::{PostDominators, post_dominators};
pub use reducibility::is_reducible;
pub use scc::strongly_connected_components;
pub use traversal::{postorder, reverse_postorder};
