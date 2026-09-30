//! Declarative pass metadata and dependency scheduling.

use std::collections::{HashMap, HashSet, VecDeque};

use thiserror::Error;

/// The program unit on which a pass operates.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Scope {
    /// The complete input program.
    Program,
    /// One class or analogous aggregate.
    Class,
    /// One method or analogous callable.
    Method,
}

/// Declarative metadata for a transformation or analysis pass.
pub trait Pass {
    /// Returns the globally unique stable pass identifier.
    fn id(&self) -> &'static str;

    /// Returns the unit on which this pass operates.
    fn scope(&self) -> Scope;

    /// Returns invariants that must be available before this pass.
    fn requires(&self) -> &[&'static str];

    /// Returns invariants made available by this pass.
    fn provides(&self) -> &[&'static str];

    /// Returns invariants invalidated by this pass.
    fn invalidates(&self) -> &[&'static str];
}

/// An error raised while registering or scheduling passes.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PipelineError {
    /// A pass identifier is already registered.
    #[error("pass {0} is already registered")]
    DuplicatePass(&'static str),
    /// No registered pass provides a required invariant.
    #[error("pass {pass} requires invariant {invariant}, which has no provider")]
    MissingProvider {
        /// Pass with the unsatisfied requirement.
        pass: &'static str,
        /// Required invariant without a provider.
        invariant: &'static str,
    },
    /// Pass dependency relationships contain a cycle.
    #[error("pass dependencies contain a cycle")]
    Cycle,
}

/// An insertion-ordered collection of registered passes.
#[derive(Default)]
pub struct Registry {
    passes: Vec<Box<dyn Pass>>,
}

impl Registry {
    /// Creates an empty registry.
    pub const fn new() -> Self {
        Self { passes: Vec::new() }
    }

    /// Registers a pass with a unique identifier.
    ///
    /// # Errors
    /// Returns [`PipelineError::DuplicatePass`] for an existing identifier.
    pub fn register(&mut self, pass: Box<dyn Pass>) -> Result<(), PipelineError> {
        if self
            .passes
            .iter()
            .any(|existing| existing.id() == pass.id())
        {
            return Err(PipelineError::DuplicatePass(pass.id()));
        }
        self.passes.push(pass);
        Ok(())
    }

    /// Topologically orders passes by their required and provided invariants.
    ///
    /// Independent passes preserve registration order. When multiple passes
    /// provide one invariant, all providers are ordered before its consumers.
    ///
    /// # Errors
    /// Returns an error for an unprovided requirement or dependency cycle.
    pub fn schedule(&self) -> Result<Vec<&dyn Pass>, PipelineError> {
        let mut providers: HashMap<&'static str, Vec<usize>> = HashMap::new();
        for (index, pass) in self.passes.iter().enumerate() {
            for invariant in pass.provides() {
                providers.entry(*invariant).or_default().push(index);
            }
        }
        let mut successors = vec![HashSet::new(); self.passes.len()];
        let mut indegree = vec![0usize; self.passes.len()];
        for (consumer, pass) in self.passes.iter().enumerate() {
            for invariant in pass.requires() {
                let Some(matching) = providers.get(invariant) else {
                    return Err(PipelineError::MissingProvider {
                        pass: pass.id(),
                        invariant,
                    });
                };
                for provider in matching {
                    if successors[*provider].insert(consumer) {
                        indegree[consumer] += 1;
                    }
                }
            }
        }
        let mut ready: VecDeque<_> = indegree
            .iter()
            .enumerate()
            .filter_map(|(index, degree)| (*degree == 0).then_some(index))
            .collect();
        let mut ordered = Vec::with_capacity(self.passes.len());
        while let Some(index) = ready.pop_front() {
            ordered.push(self.passes[index].as_ref());
            let mut next: Vec<_> = successors[index].iter().copied().collect();
            next.sort_unstable();
            for successor in next {
                indegree[successor] -= 1;
                if indegree[successor] == 0 {
                    ready.push_back(successor);
                }
            }
        }
        if ordered.len() != self.passes.len() {
            return Err(PipelineError::Cycle);
        }
        Ok(ordered)
    }
}
