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
    requires_scan: bool,
}

impl SourceMap {
    /// Returns all half-open mappings in render order.
    pub fn mappings(&self) -> &[Mapping] {
        &self.mappings
    }

    /// Finds the provenance covering an output coordinate.
    pub fn lookup(&self, position: Position) -> Option<ProvenanceId> {
        if !self.requires_scan {
            let index = self
                .mappings
                .partition_point(|mapping| mapping.start <= position);
            return index.checked_sub(1).and_then(|index| {
                let mapping = self.mappings[index];
                mapping.contains(position).then_some(mapping.provenance)
            });
        }
        self.mappings
            .iter()
            .find(|mapping| mapping.contains(position))
            .map(|mapping| mapping.provenance)
    }

    pub(crate) fn push(&mut self, mapping: Mapping) {
        // Overlaps and out-of-order input retain the original first-match rule.
        self.requires_scan |= mapping.start.line != mapping.end.line
            || mapping.start >= mapping.end
            || self
                .mappings
                .last()
                .is_some_and(|last| last.end > mapping.start);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_and_fallback_lookup_match_first_covering_interval() {
        let cases = [
            vec![(0, 1, 3), (0, 3, 5), (1, 0, 4), (2, 2, 6)],
            vec![(0, 1, 6), (0, 2, 3), (0, 1, 4)],
            vec![(1, 1, 4), (0, 2, 5), (0, 0, 3)],
            vec![(0, 1, 1), (0, 1, 4), (0, 4, 2)],
        ];
        for (case, intervals) in cases.into_iter().enumerate() {
            let mut map = SourceMap::default();
            for (id, (line, start, end)) in intervals.into_iter().enumerate() {
                map.push(Mapping::new(
                    Position::new(line, start),
                    Position::new(line, end),
                    ProvenanceId::new(id as u64),
                ));
            }
            assert_eq!(map.requires_scan, case != 0);
            for line in 0..4 {
                for column in 0..8 {
                    let position = Position::new(line, column);
                    let expected = map
                        .mappings()
                        .iter()
                        .find(|m| m.contains(position))
                        .map(|m| m.provenance());
                    assert_eq!(map.lookup(position), expected);
                }
            }
        }
        let mut map = SourceMap::default();
        map.push(Mapping::new(
            Position::new(0, 0),
            Position::new(1, 3),
            ProvenanceId::new(0),
        ));
        assert_eq!(map.lookup(Position::new(0, 1)), None);
        assert_eq!(SourceMap::default().lookup(Position::new(0, 0)), None);
    }
}
