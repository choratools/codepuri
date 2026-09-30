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
        for (block_id, block) in &self.blocks {
            self.validate_adjacency(block_id, &block.outgoing, "outgoing", true)?;
            self.validate_adjacency(block_id, &block.incoming, "incoming", false)?;
        }
        for (edge_id, edge) in &self.edges {
            self.validate_endpoint(edge_id, edge.source, "source", true)?;
            self.validate_endpoint(edge_id, edge.target, "target", false)?;
        }
        Ok(())
    }

    fn validate_adjacency(
        &self,
        block: BlockId,
        adjacency: &[EdgeId],
        direction: &'static str,
        outgoing: bool,
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
        }
        Ok(())
    }

    fn validate_endpoint(
        &self,
        edge: EdgeId,
        block: BlockId,
        direction: &'static str,
        source: bool,
    ) -> Result<(), CfgError> {
        let endpoint = self.blocks.get(block).ok_or(CfgError::DanglingEndpoint {
            edge,
            block,
            direction,
        })?;
        let adjacency = if source {
            &endpoint.outgoing
        } else {
            &endpoint.incoming
        };
        if !adjacency.contains(&edge) {
            return Err(CfgError::MissingAdjacency {
                block,
                edge,
                direction: if source { "outgoing" } else { "incoming" },
            });
        }
        Ok(())
    }
}
