//! Value interning (docs/12): same hash → same `Arc`. Repeated geometry
//! dedupes in memory automatically; the viewer exploits the same fact for
//! instanced draws. An explicit instance, never a global — no ambient state.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use crate::hash::ValueHash;
use crate::value::HashedValue;

/// Deduplicates values by content hash. Holds weak references only: dropping
/// every strong `Arc` lets the value die; the stale entry is pruned lazily.
#[derive(Default)]
pub struct Interner {
    map: Mutex<HashMap<ValueHash, Weak<HashedValue>>>,
}

impl Interner {
    /// A fresh, empty interner.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the canonical `Arc` for this value's hash. The first arrival
    /// becomes canonical; later equal values return the original allocation.
    ///
    /// A poisoned mutex (a panic elsewhere while interning) is recovered,
    /// not propagated: the map holds only weak entries, so the worst
    /// post-poison outcome is a missed dedup — equality stays hash-based
    /// and correctness never depends on interning.
    #[must_use]
    pub fn intern(&self, value: Arc<HashedValue>) -> Arc<HashedValue> {
        let mut map = self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(canonical) = map.get(&value.hash()).and_then(Weak::upgrade) {
            canonical
        } else {
            map.insert(value.hash(), Arc::downgrade(&value));
            value
        }
    }

    /// Drop entries whose values have died. Called by cache maintenance
    /// (stage 3); exposed so tests can assert the weak behavior.
    pub fn prune(&self) {
        let mut map = self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.retain(|_, weak| weak.strong_count() > 0);
    }

    /// Number of live entries (dead weaks may still be counted until
    /// [`Self::prune`] runs).
    #[must_use]
    pub fn len(&self) -> usize {
        self.map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// True when no entries exist.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::ValueData;

    #[test]
    fn same_hash_same_arc() {
        let interner = Interner::new();
        let a = interner.intern(HashedValue::new(ValueData::Number(1.0)).unwrap());
        let b = interner.intern(HashedValue::new(ValueData::Number(1.0)).unwrap());
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn different_values_stay_distinct() {
        let interner = Interner::new();
        let a = interner.intern(HashedValue::new(ValueData::Number(1.0)).unwrap());
        let b = interner.intern(HashedValue::new(ValueData::Number(2.0)).unwrap());
        assert!(!Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn dead_values_can_be_pruned_and_reinterned() {
        let interner = Interner::new();
        let a = interner.intern(HashedValue::new(ValueData::Number(1.0)).unwrap());
        drop(a);
        interner.prune();
        assert!(interner.is_empty());
        let b = interner.intern(HashedValue::new(ValueData::Number(1.0)).unwrap());
        assert_eq!(interner.len(), 1);
        drop(b);
    }
}
