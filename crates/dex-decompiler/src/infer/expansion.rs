//! Method-local, topology-only expansion cache. Types must never be cached.
//!
//! The first query costs O(W_expand) and allocates traversal storage; subsequent
//! queries cost O(1) to obtain the ordered leaves, then O(leaves) for lattice
//! operations. Storage is O(sum of queried roots' distinct reachable leaves).
//! Per-root DFS deliberately does not splice another root's cached expansion:
//! in an SCC its visited prefix can change leaf order and therefore diagnostics.
//! Each traversal retains the old LIFO order and once-per-value semantics.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use managed_ir::insn::ManagedInsn;
use managed_ir::method::InsnArena;
use managed_ir::value::{InsnId, ValueId};

#[derive(Default)]
pub(super) struct Expansions {
    leaves: HashMap<(ValueId, bool), Arc<[ValueId]>>,
    #[cfg(test)]
    pub(super) visits: usize,
    #[cfg(test)]
    pub(super) builds: usize,
}

impl Expansions {
    pub(super) fn leaves(
        &mut self,
        root: ValueId,
        join: bool,
        insns: &InsnArena,
        definitions: &HashMap<ValueId, InsnId>,
        phis: &HashMap<ValueId, InsnId>,
    ) -> Arc<[ValueId]> {
        if let Some(leaves) = self.leaves.get(&(root, join)) {
            return Arc::clone(leaves);
        }
        #[cfg(test)]
        {
            self.builds += 1;
        }
        let mut seen = HashSet::new();
        let mut pending = vec![root];
        if join {
            seen.insert(root);
            pending.clear();
            if let Some(ManagedInsn::Phi { inputs, .. }) =
                definitions.get(&root).and_then(|id| insns.get(*id))
            {
                pending.extend(inputs.iter().map(|(_, v)| *v));
            }
        }
        let mut leaves = Vec::new();
        while let Some(value) = pending.pop() {
            #[cfg(test)]
            {
                self.visits += 1;
            }
            if !seen.insert(value) {
                continue;
            }
            let definition = if join {
                definitions.get(&value)
            } else {
                phis.get(&value)
            };
            match definition.and_then(|id| insns.get(*id)) {
                Some(ManagedInsn::Phi { inputs, .. }) => {
                    pending.extend(inputs.iter().map(|(_, v)| *v))
                }
                Some(ManagedInsn::Move { value, .. }) if join => pending.push(*value),
                _ if join || !phis.contains_key(&value) => leaves.push(value),
                _ => {}
            }
        }
        let leaves: Arc<[ValueId]> = leaves.into();
        self.leaves.insert((root, join), Arc::clone(&leaves));
        leaves
    }
}
