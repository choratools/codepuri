//! Output-coordinate to opaque-provenance mappings.

/// An opaque identity supplied by the caller.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProvenanceId(u64);

impl ProvenanceId {
    /// Creates an identity from caller-owned provenance data.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the opaque numeric value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// A zero-based output coordinate measured in Unicode scalar columns.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Position {
    /// The zero-based output line.
    pub line: usize,
    /// The zero-based Unicode scalar column.
    pub column: usize,
}

impl Position {
    /// Creates an output coordinate.
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

/// A half-open rendered range associated with one provenance identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mapping {
    start: Position,
    end: Position,
    provenance: ProvenanceId,
}

impl Mapping {
    /// Returns the inclusive start coordinate.
    pub const fn start(self) -> Position {
        self.start
    }

    /// Returns the exclusive end coordinate.
    pub const fn end(self) -> Position {
        self.end
    }

    /// Returns the caller-supplied provenance identity.
    pub const fn provenance(self) -> ProvenanceId {
        self.provenance
    }

    const fn contains(self, position: Position) -> bool {
        position.line == self.start.line
            && position.line == self.end.line
            && position.column >= self.start.column
            && position.column < self.end.column
    }
}

/// Source mappings accumulated in render order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourceMap {
    mappings: Vec<Mapping>,
}

impl SourceMap {
    /// Returns all half-open mappings in render order.
    pub fn mappings(&self) -> &[Mapping] {
        &self.mappings
    }

    /// Finds the provenance covering an output coordinate.
    pub fn lookup(&self, position: Position) -> Option<ProvenanceId> {
        self.mappings
            .iter()
            .find(|mapping| mapping.contains(position))
            .map(|mapping| mapping.provenance)
    }

    pub(crate) fn push(&mut self, mapping: Mapping) {
        self.mappings.push(mapping);
    }
}

impl Mapping {
    pub(crate) const fn new(start: Position, end: Position, provenance: ProvenanceId) -> Self {
        Self {
            start,
            end,
            provenance,
        }
    }
}
