//! Typed generational identifiers used by the control-flow graph.

use slotmap::new_key_type;

new_key_type! {
    /// A generational identifier for a basic block.
    pub struct BlockId;

    /// A generational identifier for a control-flow edge.
    pub struct EdgeId;
}
