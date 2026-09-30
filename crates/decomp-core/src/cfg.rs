//! Generic control-flow graph storage and mutation.

mod mutation;
mod validation;

use slotmap::SlotMap;
use thiserror::Error;

use crate::ids::{BlockId, EdgeId};

/// The semantic category of a control-flow edge.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum EdgeKind {
    /// Control always transfers to the target.
    Unconditional,
    /// Control transfers when a branch condition is true.
    BranchTrue,
    /// Control transfers when a branch condition is false.
    BranchFalse,
    /// Control transfers for the specified integral switch value.
    Switch(u64),
    /// Exceptional control flow for a named exception type.
    ///
    /// The string is a family-neutral placeholder for a future typed exception
    /// reference owned by a family IR.
    Exception(String),
}

/// A basic block containing a caller-defined payload and ordered adjacency.
#[derive(Clone, Debug)]
pub struct Block<T> {
    payload: T,
    incoming: Vec<EdgeId>,
    outgoing: Vec<EdgeId>,
}

impl<T> Block<T> {
    /// Returns the caller-defined block payload.
    pub const fn payload(&self) -> &T {
        &self.payload
    }

    /// Returns incoming edges in insertion order.
    pub fn incoming(&self) -> &[EdgeId] {
        &self.incoming
    }

    /// Returns outgoing edges in insertion order.
    pub fn outgoing(&self) -> &[EdgeId] {
        &self.outgoing
    }
}

/// A first-class directed control-flow edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edge {
    source: BlockId,
    target: BlockId,
    kind: EdgeKind,
}

impl Edge {
    /// Returns the source block.
    pub const fn source(&self) -> BlockId {
        self.source
    }

    /// Returns the target block.
    pub const fn target(&self) -> BlockId {
        self.target
    }

    /// Returns the edge category.
    pub const fn kind(&self) -> &EdgeKind {
        &self.kind
    }
}

/// An error raised by CFG lookup, mutation, or validation.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CfgError {
    /// A supplied block identifier is not live.
    #[error("block {0:?} does not exist")]
    MissingBlock(BlockId),
    /// A supplied edge identifier is not live.
    #[error("edge {0:?} does not exist")]
    MissingEdge(EdgeId),
    /// The entry block cannot be removed.
    #[error("entry block {0:?} cannot be removed")]
    CannotRemoveEntry(BlockId),
    /// The tracked entry identifier is not live.
    #[error("entry block {0:?} is dangling")]
    DanglingEntry(BlockId),
    /// An adjacency list names an edge that is not live.
    #[error("block {block:?} has dangling {direction} edge {edge:?}")]
    DanglingAdjacency {
        /// Block containing the bad adjacency.
        block: BlockId,
        /// Missing edge identifier.
        edge: EdgeId,
        /// Whether the adjacency is incoming or outgoing.
        direction: &'static str,
    },
    /// An edge endpoint disagrees with the adjacency list containing it.
    #[error("edge {edge:?} has inconsistent {direction} endpoint at block {block:?}")]
    InconsistentEndpoint {
        /// Block containing the adjacency.
        block: BlockId,
        /// Inconsistent edge identifier.
        edge: EdgeId,
        /// Whether the adjacency is incoming or outgoing.
        direction: &'static str,
    },
    /// An adjacency list contains the same edge more than once.
    #[error("block {block:?} repeats {direction} edge {edge:?}")]
    DuplicateAdjacency {
        /// Block containing the duplicate.
        block: BlockId,
        /// Duplicated edge identifier.
        edge: EdgeId,
        /// Whether the adjacency is incoming or outgoing.
        direction: &'static str,
    },
    /// A live edge names a block that is not live.
    #[error("edge {edge:?} has dangling {direction} block {block:?}")]
    DanglingEndpoint {
        /// Edge with the dangling endpoint.
        edge: EdgeId,
        /// Missing endpoint block.
        block: BlockId,
        /// Whether the endpoint is a source or target.
        direction: &'static str,
    },
    /// A live edge is absent from one of its endpoint adjacency lists.
    #[error("edge {edge:?} is absent from block {block:?} {direction} adjacency")]
    MissingAdjacency {
        /// Endpoint block missing the edge.
        block: BlockId,
        /// Edge absent from the adjacency.
        edge: EdgeId,
        /// Whether the adjacency is incoming or outgoing.
        direction: &'static str,
    },
}

/// A family-neutral directed control-flow graph.
#[derive(Debug)]
pub struct Cfg<T> {
    blocks: SlotMap<BlockId, Block<T>>,
    edges: SlotMap<EdgeId, Edge>,
    entry: BlockId,
    revision: u64,
}

impl<T> Cfg<T> {
    /// Creates a graph containing exactly one entry block.
    pub fn new(entry_payload: T) -> Self {
        let mut blocks = SlotMap::with_key();
        let entry = blocks.insert(Block {
            payload: entry_payload,
            incoming: Vec::new(),
            outgoing: Vec::new(),
        });
        Self {
            blocks,
            edges: SlotMap::with_key(),
            entry,
            revision: 0,
        }
    }

    /// Returns the single entry block.
    pub const fn entry(&self) -> BlockId {
        self.entry
    }

    /// Returns the current graph revision.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns a block or an error when its generational ID is stale.
    pub fn block(&self, block: BlockId) -> Result<&Block<T>, CfgError> {
        self.blocks.get(block).ok_or(CfgError::MissingBlock(block))
    }

    /// Returns an edge or an error when its generational ID is stale.
    pub fn edge(&self, edge: EdgeId) -> Result<&Edge, CfgError> {
        self.edges.get(edge).ok_or(CfgError::MissingEdge(edge))
    }

    /// Iterates over live block identifiers.
    pub fn block_ids(&self) -> impl Iterator<Item = BlockId> + '_ {
        self.blocks.keys()
    }

    /// Iterates over live edge identifiers.
    pub fn edge_ids(&self) -> impl Iterator<Item = EdgeId> + '_ {
        self.edges.keys()
    }

    /// Returns the number of live blocks.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Returns the number of live edges.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
}
