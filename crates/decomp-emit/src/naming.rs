//! Deterministic allocation of conservative generated identifiers.

use std::collections::{BTreeMap, BTreeSet};

/// Allocates unique ASCII-safe identifiers while avoiding caller keywords.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NameAllocator {
    unavailable: BTreeSet<String>,
    suffixes: BTreeMap<String, u64>,
}

impl NameAllocator {
    /// Creates an allocator with a caller-supplied reserved-keyword set.
    pub fn new<I, S>(reserved: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            unavailable: reserved.into_iter().map(Into::into).collect(),
            suffixes: BTreeMap::new(),
        }
    }

    /// Sanitizes and reserves a deterministic unique spelling.
    pub fn allocate(&mut self, requested: &str) -> String {
        let base = sanitize(requested);
        if self.unavailable.insert(base.clone()) {
            return base;
        }

        let suffix = self.suffixes.entry(base.clone()).or_insert(2);
        loop {
            let candidate = format!("{base}_{suffix}");
            *suffix = suffix.saturating_add(1);
            if self.unavailable.insert(candidate.clone()) {
                return candidate;
            }
        }
    }
}

impl Default for NameAllocator {
    fn default() -> Self {
        Self::new(std::iter::empty::<String>())
    }
}

fn sanitize(requested: &str) -> String {
    let mut output = String::with_capacity(requested.len());
    for (index, character) in requested.chars().enumerate() {
        if character.is_ascii_alphabetic() || character == '_' {
            output.push(character);
        } else if character.is_ascii_digit() {
            if index == 0 {
                output.push('_');
            }
            output.push(character);
        } else {
            output.push('_');
        }
    }
    if output.is_empty() {
        output.push('_');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_match_restart_with_raw_and_reserved_collisions() {
        let reserved = ["x", "x_2", "x_4", "_", "__2"];
        let mut allocator = NameAllocator::new(reserved);
        let mut used: BTreeSet<String> = reserved.into_iter().map(str::to_owned).collect();
        let requests = [
            "x", "x_3", "x", "x_2", "x", "", "💩", "α", "CON", "con", "9x",
        ];
        for requested in requests.into_iter().cycle().take(2048) {
            let base = sanitize(requested);
            let mut expected = base.clone();
            let mut suffix = 2;
            while !used.insert(expected.clone()) {
                expected = format!("{base}_{suffix}");
                suffix += 1;
            }
            assert_eq!(allocator.allocate(requested), expected);
        }
    }
}
