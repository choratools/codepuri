//! Deterministic allocation of conservative generated identifiers.

use std::collections::BTreeSet;

/// Allocates unique ASCII-safe identifiers while avoiding caller keywords.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NameAllocator {
    unavailable: BTreeSet<String>,
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
        }
    }

    /// Sanitizes and reserves a deterministic unique spelling.
    pub fn allocate(&mut self, requested: &str) -> String {
        let base = sanitize(requested);
        if self.unavailable.insert(base.clone()) {
            return base;
        }

        let mut suffix = 2_u64;
        loop {
            let candidate = format!("{base}_{suffix}");
            if self.unavailable.insert(candidate.clone()) {
                return candidate;
            }
            suffix = suffix.saturating_add(1);
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
