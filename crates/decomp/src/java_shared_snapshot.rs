//! Export-scoped immutable inventories with isolated mutable fixture setup.
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

pub(super) struct SharedSnapshot<T>(Arc<T>);

impl<T> Clone for SharedSnapshot<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}
impl<T: Default> Default for SharedSnapshot<T> {
    fn default() -> Self {
        Self::from(T::default())
    }
}
impl<T> From<T> for SharedSnapshot<T> {
    fn from(value: T) -> Self {
        Self(Arc::new(value))
    }
}
impl<T> Deref for SharedSnapshot<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}
impl<T: Clone> DerefMut for SharedSnapshot<T> {
    fn deref_mut(&mut self) -> &mut T {
        Arc::make_mut(&mut self.0)
    }
}
impl<A, T: FromIterator<A>> FromIterator<A> for SharedSnapshot<T> {
    fn from_iter<I: IntoIterator<Item = A>>(iter: I) -> Self {
        Self::from(T::from_iter(iter))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn clones_share_until_mutation_and_keep_independent_sessions() {
        let original: SharedSnapshot<BTreeSet<String>> =
            ["original".to_owned()].into_iter().collect();
        let mut other = original.clone();
        assert!(Arc::ptr_eq(&original.0, &other.0));
        other.insert("other".into());
        assert!(!Arc::ptr_eq(&original.0, &other.0));
        assert!(!original.contains("other"));
        assert!(other.contains("original"));
        let independent = SharedSnapshot::from(BTreeSet::from(["original".to_owned()]));
        assert!(!Arc::ptr_eq(&original.0, &independent.0));
    }
}
