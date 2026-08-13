//! `NodeKey` construction (docs/12 §Cache keys): blake3 over the operation
//! identity, its semantic version, the per-port input hashes in spec order,
//! and the pairing shape — through [`ValueHasher`], so every hash in the
//! system shares one versioned, domain-separated format.
//!
//! What is deliberately **not** in a key: the binding name (renames must
//! never recompute, docs/10) and the unit tag (a relabel must not dirty
//! tolerance caches — `ProjectConfig::tolerance_hash` already excludes it).

use cicada_core::hash::{KindTag, ValueHash, ValueHasher};

/// Salts every `NodeKey` (docs/12: "the engine's major version salts
/// everything"). Bump to orphan all cached keys on a format evolution;
/// never reuse a value.
pub const CACHE_EPOCH: u32 = 1;

/// A scheduler cache key: `blake3(epoch, op, version, body, tolerance,
/// input hashes, pairing shape)`. Distinct type from [`ValueHash`] so memo
/// keys and value addresses can never be confused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeKey(ValueHash);

impl NodeKey {
    /// The underlying hash (persistence currency).
    #[must_use]
    pub const fn as_hash(&self) -> &ValueHash {
        &self.0
    }

    /// Rebuild from a persisted hash (the memo log's loading path).
    #[must_use]
    pub const fn from_hash(hash: ValueHash) -> Self {
        Self(hash)
    }
}

/// Everything that identifies one computation. Borrowed views — the
/// executor assembles these on the fly for node- and element-level keys.
pub struct KeyInputs<'a> {
    /// Operation identity (`add`, `expr`, …) — never the binding name.
    pub op: &'a str,
    /// Semantic version of the operation.
    pub version: u32,
    /// Body hash for body-carrying ops (expression IR, script source).
    pub body_hash: Option<&'a ValueHash>,
    /// Tolerance hash for `uses_tolerance` ops (DECISIONS.md).
    pub tolerance: Option<&'a ValueHash>,
    /// Per-port input value hashes in spec order; `None` = absent (the
    /// port default applies — its meaning is pinned by `version`).
    pub inputs: &'a [Option<ValueHash>],
    /// `each()` depth per port — the pairing shape. All zeros for
    /// element-level keys: an element computation IS the scalar call, so
    /// identical scalar calls dedupe across nodes by construction.
    pub fan: &'a [u8],
}

/// Build the key.
#[must_use]
pub fn node_key(k: &KeyInputs<'_>) -> NodeKey {
    let mut hasher = ValueHasher::new(KindTag::NodeKey)
        .u64(u64::from(CACHE_EPOCH))
        .bytes(k.op.as_bytes())
        .u64(u64::from(k.version));
    hasher = match k.body_hash {
        Some(hash) => hasher.byte(1).child(hash),
        None => hasher.byte(0),
    };
    hasher = match k.tolerance {
        Some(hash) => hasher.byte(1).child(hash),
        None => hasher.byte(0),
    };
    hasher = hasher.u64(k.inputs.len() as u64);
    for input in k.inputs {
        hasher = match input {
            Some(hash) => hasher.byte(1).child(hash),
            None => hasher.byte(0),
        };
    }
    hasher = hasher.u64(k.fan.len() as u64);
    for &depth in k.fan {
        hasher = hasher.byte(depth);
    }
    NodeKey(hasher.finish())
}

#[cfg(test)]
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    fn hash_of(x: f64) -> ValueHash {
        HashedValue::new(ValueData::Number(x)).unwrap().hash()
    }

    fn base_key(inputs: &[Option<ValueHash>], fan: &[u8]) -> NodeKey {
        node_key(&KeyInputs {
            op: "add",
            version: 1,
            body_hash: None,
            tolerance: None,
            inputs,
            fan,
        })
    }

    #[test]
    fn every_component_separates_keys() {
        let a = hash_of(1.0);
        let b = hash_of(2.0);
        let base = base_key(&[Some(a), Some(b)], &[0, 0]);

        // Same inputs, same key.
        assert_eq!(base, base_key(&[Some(a), Some(b)], &[0, 0]));
        // Input value.
        assert_ne!(base, base_key(&[Some(b), Some(a)], &[0, 0]));
        // Presence vs. default.
        assert_ne!(base, base_key(&[Some(a), None], &[0, 0]));
        // Pairing shape.
        assert_ne!(base, base_key(&[Some(a), Some(b)], &[1, 0]));
        // Version.
        let versioned = node_key(&KeyInputs {
            op: "add",
            version: 2,
            body_hash: None,
            tolerance: None,
            inputs: &[Some(a), Some(b)],
            fan: &[0, 0],
        });
        assert_ne!(base, versioned);
        // Operation.
        let other_op = node_key(&KeyInputs {
            op: "sub",
            version: 1,
            body_hash: None,
            tolerance: None,
            inputs: &[Some(a), Some(b)],
            fan: &[0, 0],
        });
        assert_ne!(base, other_op);
        // Tolerance slot.
        let toleranced = node_key(&KeyInputs {
            op: "add",
            version: 1,
            body_hash: None,
            tolerance: Some(&hash_of(3.0)),
            inputs: &[Some(a), Some(b)],
            fan: &[0, 0],
        });
        assert_ne!(base, toleranced);
        // Body slot.
        let bodied = node_key(&KeyInputs {
            op: "add",
            version: 1,
            body_hash: Some(&hash_of(4.0)),
            tolerance: None,
            inputs: &[Some(a), Some(b)],
            fan: &[0, 0],
        });
        assert_ne!(base, bodied);
    }

    // The cross-run/platform determinism contract for keys, same discipline
    // as the value-model golden hashes. Blessed via the run-once path; if
    // this drifts, the KEY format changed — bump CACHE_EPOCH and explain.
    #[test]
    fn golden_key() {
        let key = base_key(&[Some(hash_of(1.0)), None], &[1, 0]);
        assert_eq!(
            key.as_hash().to_hex(),
            "b70abc37e4807ad528bd16ec7cac76e846b1a7d89021c08e7a7f6c2d4cf69ceb"
        );
    }
}
