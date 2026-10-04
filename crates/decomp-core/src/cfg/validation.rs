use std::collections::HashSet;

use super::{Cfg, CfgError};
use crate::ids::{BlockId, EdgeId};

impl<T> Cfg<T> {
    /// Verifies all structural CFG invariants.
    ///
    /// Reachability is intentionally not a validation requirement.
    ///
    /// # Errors
    /// Returns the first dangling, duplicate, or inconsistent relationship.
    pub fn validate(&self) -> Result<(), CfgError> {
        if !self.blocks.contains_key(self.entry) {
            return Err(CfgError::DanglingEntry(self.entry));
        }
        // Endpoint agreement makes edge IDs globally unique in each direction.
        // Reuse these indexes for endpoint checks: expected O(B + E + A),
        // rather than scanning each endpoint's adjacency once per edge.
        let mut outgoing = HashSet::with_capacity(self.edges.len());
        let mut incoming = HashSet::with_capacity(self.edges.len());
        for (block_id, block) in &self.blocks {
            self.validate_adjacency(block_id, &block.outgoing, "outgoing", true, &mut outgoing)?;
            self.validate_adjacency(block_id, &block.incoming, "incoming", false, &mut incoming)?;
        }
        for (edge_id, edge) in &self.edges {
            self.validate_endpoint(edge_id, edge.source, "source", true, &outgoing)?;
            self.validate_endpoint(edge_id, edge.target, "target", false, &incoming)?;
        }
        Ok(())
    }

    fn validate_adjacency(
        &self,
        block: BlockId,
        adjacency: &[EdgeId],
        direction: &'static str,
        outgoing: bool,
        indexed: &mut HashSet<EdgeId>,
    ) -> Result<(), CfgError> {
        let mut seen = HashSet::new();
        for edge_id in adjacency {
            if !seen.insert(*edge_id) {
                return Err(CfgError::DuplicateAdjacency {
                    block,
                    edge: *edge_id,
                    direction,
                });
            }
            let edge = self
                .edges
                .get(*edge_id)
                .ok_or(CfgError::DanglingAdjacency {
                    block,
                    edge: *edge_id,
                    direction,
                })?;
            let endpoint = if outgoing { edge.source } else { edge.target };
            if endpoint != block {
                return Err(CfgError::InconsistentEndpoint {
                    block,
                    edge: *edge_id,
                    direction,
                });
            }
            indexed.insert(*edge_id);
        }
        Ok(())
    }

    fn validate_endpoint(
        &self,
        edge: EdgeId,
        block: BlockId,
        direction: &'static str,
        source: bool,
        indexed: &HashSet<EdgeId>,
    ) -> Result<(), CfgError> {
        self.blocks.get(block).ok_or(CfgError::DanglingEndpoint {
            edge,
            block,
            direction,
        })?;
        if !indexed.contains(&edge) {
            return Err(CfgError::MissingAdjacency {
                block,
                edge,
                direction: if source { "outgoing" } else { "incoming" },
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::cfg::EdgeKind;

    fn graph() -> (Cfg<()>, BlockId, EdgeId) {
        let mut cfg = Cfg::new(());
        let target = cfg.add_block(());
        let edge = cfg
            .add_edge(cfg.entry(), target, EdgeKind::Unconditional)
            .unwrap();
        (cfg, target, edge)
    }

    #[test]
    fn rejects_corrupt_adjacency_in_both_directions() {
        for outgoing in [true, false] {
            for corruption in 0..4 {
                let (mut cfg, target, edge) = graph();
                let block = if outgoing { cfg.entry() } else { target };
                let direction = if outgoing { "outgoing" } else { "incoming" };
                let adjacency = if outgoing {
                    &mut cfg.blocks[block].outgoing
                } else {
                    &mut cfg.blocks[block].incoming
                };
                match corruption {
                    0 => adjacency.push(edge),
                    1 => adjacency.clear(),
                    2 => {
                        cfg.edges.remove(edge);
                        if !outgoing {
                            let entry = cfg.entry();
                            cfg.blocks[entry].outgoing.clear();
                        }
                    }
                    _ => {
                        if outgoing {
                            cfg.edges[edge].source = target;
                        } else {
                            cfg.edges[edge].target = cfg.entry();
                        }
                    }
                }
                let expected = match corruption {
                    0 => CfgError::DuplicateAdjacency {
                        block,
                        edge,
                        direction,
                    },
                    1 => CfgError::MissingAdjacency {
                        block,
                        edge,
                        direction,
                    },
                    2 => CfgError::DanglingAdjacency {
                        block,
                        edge,
                        direction,
                    },
                    _ => CfgError::InconsistentEndpoint {
                        block,
                        edge,
                        direction,
                    },
                };
                assert_eq!(cfg.validate(), Err(expected));
            }
        }
    }

    #[test]
    fn rejects_dangling_endpoints_and_entry() {
        for source in [true, false] {
            let (mut cfg, target, edge) = graph();
            let stale = cfg.add_block(());
            cfg.blocks.remove(stale);
            if source {
                let entry = cfg.entry();
                cfg.blocks[entry].outgoing.clear();
                cfg.edges[edge].source = stale;
            } else {
                cfg.blocks[target].incoming.clear();
                cfg.edges[edge].target = stale;
            }
            assert_eq!(
                cfg.validate(),
                Err(CfgError::DanglingEndpoint {
                    edge,
                    block: stale,
                    direction: if source { "source" } else { "target" },
                })
            );
        }
        let (mut cfg, _, _) = graph();
        let entry = cfg.entry();
        cfg.blocks.remove(entry);
        assert_eq!(cfg.validate(), Err(CfgError::DanglingEntry(entry)));
    }

    #[test]
    fn validates_high_indegree_parallel_edges_and_self_loops() {
        let mut cfg = Cfg::new(());
        let join = cfg.add_block(());
        for _ in 0..8192 {
            cfg.add_edge(cfg.entry(), join, EdgeKind::Unconditional)
                .unwrap();
        }
        cfg.add_edge(join, join, EdgeKind::Unconditional).unwrap();
        assert_eq!(cfg.validate(), Ok(()));
        cfg.blocks[join].incoming.pop();
        assert!(matches!(
            cfg.validate(),
            Err(CfgError::MissingAdjacency {
                direction: "incoming",
                ..
            })
        ));
    }
}
