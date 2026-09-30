use std::collections::HashSet;

use super::{Block, Cfg, CfgError, Edge, EdgeKind};
use crate::ids::{BlockId, EdgeId};

impl<T> Cfg<T> {
    /// Adds a disconnected block and advances the revision.
    pub fn add_block(&mut self, payload: T) -> BlockId {
        let block = self.blocks.insert(Block {
            payload,
            incoming: Vec::new(),
            outgoing: Vec::new(),
        });
        self.bump_revision();
        block
    }

    /// Adds an edge and updates both endpoint adjacency lists.
    ///
    /// # Errors
    /// Returns [`CfgError::MissingBlock`] when either endpoint is not live.
    pub fn add_edge(
        &mut self,
        source: BlockId,
        target: BlockId,
        kind: EdgeKind,
    ) -> Result<EdgeId, CfgError> {
        self.require_block(source)?;
        self.require_block(target)?;
        let edge = self.insert_edge(source, target, kind);
        self.bump_revision();
        Ok(edge)
    }

    /// Inserts a new prefix before a block, moving all incoming edges to it.
    ///
    /// An unconditional edge connects the new prefix to the old block, which
    /// remains the suffix. Splitting the entry promotes the prefix to entry.
    ///
    /// # Errors
    /// Returns [`CfgError::MissingBlock`] when `suffix` is not live.
    pub fn split_block(&mut self, suffix: BlockId, prefix_payload: T) -> Result<BlockId, CfgError> {
        self.require_block(suffix)?;
        let incoming = self.blocks[suffix].incoming.clone();
        let prefix = self.blocks.insert(Block {
            payload: prefix_payload,
            incoming: incoming.clone(),
            outgoing: Vec::new(),
        });
        for edge in incoming {
            self.edges[edge].target = prefix;
        }
        self.blocks[suffix].incoming.clear();
        self.insert_edge(prefix, suffix, EdgeKind::Unconditional);
        if self.entry == suffix {
            self.entry = prefix;
        }
        self.bump_revision();
        Ok(prefix)
    }

    /// Removes a non-entry block and all incident edges.
    ///
    /// # Errors
    /// Returns an error when the block is absent or is the entry block.
    pub fn remove_block(&mut self, block: BlockId) -> Result<T, CfgError> {
        self.require_block(block)?;
        if block == self.entry {
            return Err(CfgError::CannotRemoveEntry(block));
        }
        let mut seen = HashSet::new();
        let incident: Vec<_> = self.blocks[block]
            .incoming
            .iter()
            .chain(&self.blocks[block].outgoing)
            .copied()
            .filter(|edge| seen.insert(*edge))
            .collect();
        for edge in incident {
            self.remove_edge_record(edge);
        }
        let removed = self
            .blocks
            .remove(block)
            .ok_or(CfgError::MissingBlock(block))?;
        self.bump_revision();
        Ok(removed.payload)
    }

    /// Redirects an edge to a new target while preserving its kind and source.
    ///
    /// # Errors
    /// Returns an error when the edge or target is not live.
    pub fn redirect_edge(&mut self, edge: EdgeId, new_target: BlockId) -> Result<(), CfgError> {
        self.require_block(new_target)?;
        let old_target = self.edge(edge)?.target;
        if old_target != new_target {
            self.blocks[old_target]
                .incoming
                .retain(|item| *item != edge);
            self.blocks[new_target].incoming.push(edge);
            self.edges[edge].target = new_target;
        }
        self.bump_revision();
        Ok(())
    }

    /// Retargets every edge of `kind` currently pointing at `old_target`.
    ///
    /// Returns the number of matching edges.
    ///
    /// # Errors
    /// Returns an error when either target is not live.
    pub fn replace_all_targets(
        &mut self,
        old_target: BlockId,
        kind: &EdgeKind,
        new_target: BlockId,
    ) -> Result<usize, CfgError> {
        self.require_block(old_target)?;
        self.require_block(new_target)?;
        let matches: Vec<_> = self.blocks[old_target]
            .incoming
            .iter()
            .copied()
            .filter(|edge| {
                self.edges
                    .get(*edge)
                    .is_some_and(|record| record.kind == *kind)
            })
            .collect();
        if old_target != new_target {
            let selected: HashSet<_> = matches.iter().copied().collect();
            self.blocks[old_target]
                .incoming
                .retain(|edge| !selected.contains(edge));
            self.blocks[new_target]
                .incoming
                .extend(matches.iter().copied());
            for edge in &matches {
                self.edges[*edge].target = new_target;
            }
        }
        self.bump_revision();
        Ok(matches.len())
    }

    /// Inserts a new block on an edge.
    ///
    /// The original edge keeps its kind and now targets the new block. A new
    /// unconditional edge connects the inserted block to the old target.
    ///
    /// # Errors
    /// Returns [`CfgError::MissingEdge`] when `edge` is not live.
    pub fn insert_between(&mut self, edge: EdgeId, payload: T) -> Result<BlockId, CfgError> {
        let (source, old_target) = {
            let record = self.edge(edge)?;
            (record.source, record.target)
        };
        let middle = self.blocks.insert(Block {
            payload,
            incoming: vec![edge],
            outgoing: Vec::new(),
        });
        self.blocks[old_target]
            .incoming
            .retain(|item| *item != edge);
        self.edges[edge].target = middle;
        self.insert_edge(middle, old_target, EdgeKind::Unconditional);
        debug_assert_eq!(self.edges[edge].source, source);
        self.bump_revision();
        Ok(middle)
    }

    fn require_block(&self, block: BlockId) -> Result<(), CfgError> {
        self.blocks
            .contains_key(block)
            .then_some(())
            .ok_or(CfgError::MissingBlock(block))
    }

    fn insert_edge(&mut self, source: BlockId, target: BlockId, kind: EdgeKind) -> EdgeId {
        let edge = self.edges.insert(Edge {
            source,
            target,
            kind,
        });
        self.blocks[source].outgoing.push(edge);
        self.blocks[target].incoming.push(edge);
        edge
    }

    fn remove_edge_record(&mut self, edge: EdgeId) {
        if let Some(record) = self.edges.remove(edge) {
            self.blocks[record.source]
                .outgoing
                .retain(|item| *item != edge);
            self.blocks[record.target]
                .incoming
                .retain(|item| *item != edge);
        }
    }
}
