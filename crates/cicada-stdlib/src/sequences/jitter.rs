//! The `jitter` node.

use cicada_core::marshal::ElemSlot;
use cicada_core::scalar::IndexMap;
use cicada_macros::{Ports, node};

use super::support::{seed_state, unit_draw};

/// Inputs for [`jitter`].
#[derive(Ports, Clone, Debug)]
pub struct JitterIn {
    /// The list to shuffle (any kind; absent slots shuffle too).
    pub list: Vec<ElemSlot>,
    /// How far from the original order to stray, `0.0` (unchanged) to
    /// `1.0` (a fully random order).
    #[port(default = 1.0)]
    pub strength: f64,
    /// PRNG seed — explicit, always (docs/08 rule 2).
    pub seed: i64,
}

/// Outputs of [`jitter`].
#[derive(Ports, Clone, Debug)]
pub struct JitterOut {
    /// The shuffled list.
    pub list: Vec<ElemSlot>,
    /// Provenance: `map[i]` is the source index of `list[i]`.
    pub map: IndexMap,
}

/// Jitter — shuffle a list by a seeded random amount, keeping provenance:
/// every slot gets the key `(1 - strength) · index + strength · n · u`
/// with `u` a `splitmix64` draw in `[0, 1)` (the same generator as
/// `random`), and the list is stably sorted by key — strength `0` is the
/// identity, strength `1` a uniform random order, in between a local
/// scramble.
///
/// # Panics
///
/// Panics when `strength` lies outside `0.0..=1.0`.
///
/// # Examples
///
/// ```cic
/// xs = [1.0, 2.0, 3.0, 4.0, 5.0]
/// shuffled, sources = jitter(list=xs, seed=7)
/// ```
#[node(
    category = "Sequences & random",
    tier = "1",
    version = 1,
    gh = "Jitter"
)]
#[must_use]
pub fn jitter(input: JitterIn) -> JitterOut {
    assert!(
        (0.0..=1.0).contains(&input.strength),
        "jitter: strength must lie in 0..=1, got {}",
        input.strength
    );
    let n = input.list.len();
    let mut state = seed_state(input.seed);
    #[allow(clippy::cast_precision_loss)] // list lengths are far below 2^53
    let keys: Vec<f64> = (0..n)
        .map(|i| {
            let draw = unit_draw(&mut state);
            (1.0 - input.strength) * i as f64 + input.strength * n as f64 * draw
        })
        .collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| keys[a].total_cmp(&keys[b]));
    JitterOut {
        list: order.iter().map(|&i| input.list[i].clone()).collect(),
        map: IndexMap(order.iter().map(|&i| i as u64).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lists::support::{hex, holed_list, numbers, slots};

    #[test]
    fn jitter_table_cases() {
        let xs = numbers(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
        // Strength 0 is the identity with the identity map.
        let still = jitter(JitterIn {
            list: xs.clone(),
            strength: 0.0,
            seed: 7,
        });
        assert_eq!(still.list, xs);
        assert_eq!(still.map, IndexMap((0..8).collect()));
        // Strength 1 reorders (for this seed), the same way every time, and
        // differently for another seed.
        let shuffled = jitter(JitterIn {
            list: xs.clone(),
            strength: 1.0,
            seed: 7,
        });
        assert_ne!(shuffled.list, xs);
        assert_eq!(
            shuffled.list,
            jitter(JitterIn {
                list: xs.clone(),
                strength: 1.0,
                seed: 7,
            })
            .list
        );
        assert_ne!(
            shuffled.map,
            jitter(JitterIn {
                list: xs.clone(),
                strength: 1.0,
                seed: 8,
            })
            .map
        );
        // Holes shuffle like any slot; the empty list shuffles to itself.
        let holed = jitter(JitterIn {
            list: slots(&[Some(1.0), None, Some(3.0)]),
            strength: 1.0,
            seed: 1,
        });
        assert_eq!(holed.list.iter().filter(|s| !s.is_present()).count(), 1);
        let empty = jitter(JitterIn {
            list: vec![],
            strength: 1.0,
            seed: 1,
        });
        assert!(empty.list.is_empty() && empty.map.0.is_empty());
    }

    #[test]
    #[should_panic(expected = "strength must lie in 0..=1")]
    fn jitter_strength_out_of_range_is_red() {
        let _ = jitter(JitterIn {
            list: numbers(&[1.0]),
            strength: 1.5,
            seed: 0,
        });
    }

    proptest::proptest! {
        // The map is a permutation and list[i] == source[map[i]] — for any
        // strength and seed; strength 0 is the identity.
        #[test]
        fn jitter_property_permutation_with_provenance(
            values in holed_list(30),
            strength in 0.0..=1.0_f64,
            seed in proptest::num::i64::ANY,
        ) {
            let list = slots(&values);
            let out = jitter(JitterIn { list: list.clone(), strength, seed });
            proptest::prop_assert_eq!(out.list.len(), list.len());
            let mut seen = vec![false; list.len()];
            for (i, &source) in out.map.0.iter().enumerate() {
                #[allow(clippy::cast_possible_truncation)]
                let source = source as usize;
                proptest::prop_assert!(!seen[source], "a permutation visits each index once");
                seen[source] = true;
                proptest::prop_assert_eq!(&out.list[i], &list[source]);
            }
            let still = jitter(JitterIn { list: list.clone(), strength: 0.0, seed });
            proptest::prop_assert_eq!(still.list, list);
        }
    }

    // Golden hashes of both sealed outputs: the PRNG sequence is part of the
    // contract, so the shuffled order for a seed is platform-free.
    #[test]
    fn jitter_determinism_golden_hash() {
        let out = jitter(JitterIn {
            list: slots(&[Some(1.0), None, Some(3.0), Some(4.0), Some(5.0), Some(6.0)]),
            strength: 1.0,
            seed: 7,
        });
        assert_eq!(
            hex(out.list),
            "445cdba61a266bc288960ddd2d2b9bf9cce95bce5c8892a531e1a1c84188aa81"
        );
        assert_eq!(
            hex(out.map),
            "feda2d04c89cd5aee4b62f1ad9be12079c58d18ed18e03d02c2e5e81e4f2f9d7"
        );
    }
}
