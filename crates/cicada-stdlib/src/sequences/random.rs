//! The `random` node.

use cicada_macros::{Ports, node};

/// Inputs for [`random`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct RandomIn {
    /// The interval values are drawn from (uniform).
    pub domain: cicada_core::scalar::Domain,
    /// Number of values.
    pub count: i64,
    /// PRNG seed — explicit, always (docs/08 rule 2).
    pub seed: i64,
}

/// Random — seeded uniform random numbers in a domain.
///
/// The generator is `splitmix64` (a fixed, documented algorithm — the
/// sequence is part of the node's contract and identical on every
/// platform); each draw uses the high 53 bits, so results are exact
/// dyadic fractions of the domain.
///
/// # Returns
///
/// `count` uniform draws in `domain`, in draw order — identical for the same
/// seed on every platform.
///
/// # Panics
///
/// Panics when `count` is negative.
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=10.0)
/// draws = random(domain=span, count=5, seed=7)
/// ```
#[node(
    category = "Sequences & random",
    tier = "S",
    version = 1,
    gh = "Random"
)]
#[must_use]
pub fn random(input: RandomIn) -> Vec<f64> {
    assert!(
        input.count >= 0,
        "random: count must be >= 0, got {}",
        input.count
    );
    #[allow(clippy::cast_sign_loss)] // seed bits, not magnitude
    let mut state = input.seed as u64;
    let span = input.domain.end - input.domain.start;
    (0..input.count)
        .map(|_| {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            // High 53 bits → [0, 1) with full f64 resolution.
            #[allow(clippy::cast_precision_loss)]
            let unit = (z >> 11) as f64 / 9_007_199_254_740_992.0;
            span.mul_add(unit, input.domain.start)
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use cicada_core::value::{HashedValue, List, ValueData};

    use super::*;

    #[test]
    fn random_table_cases() {
        use cicada_core::scalar::Domain;
        let out = random(RandomIn {
            domain: Domain::new(0.0, 1.0),
            count: 5,
            seed: 42,
        });
        assert_eq!(out.len(), 5);
        assert!(out.iter().all(|&x| (0.0..1.0).contains(&x)));
        // Same seed → same sequence; different seed → different.
        let again = random(RandomIn {
            domain: Domain::new(0.0, 1.0),
            count: 5,
            seed: 42,
        });
        assert_eq!(out, again);
        let other = random(RandomIn {
            domain: Domain::new(0.0, 1.0),
            count: 5,
            seed: 43,
        });
        assert_ne!(out, other);
        assert_eq!(
            random(RandomIn {
                domain: Domain::new(2.0, 2.0),
                count: 3,
                seed: 1,
            }),
            vec![2.0, 2.0, 2.0],
            "empty domain draws its single value"
        );
    }

    #[test]
    #[should_panic(expected = "count must be >= 0")]
    fn random_negative_count_is_red() {
        let _ = random(RandomIn {
            domain: cicada_core::scalar::Domain::new(0.0, 1.0),
            count: -1,
            seed: 0,
        });
    }

    proptest::proptest! {
        // Every draw lands inside the (normalized) domain; length = count.
        #[test]
        fn random_property_in_domain(
            start in -1.0e6..1.0e6_f64,
            span in 0.0f64..1.0e6,
            count in 0i64..300,
            seed in proptest::num::i64::ANY,
        ) {
            use cicada_core::scalar::Domain;
            let out = random(RandomIn {
                domain: Domain::new(start, start + span),
                count,
                seed,
            });
            #[allow(clippy::cast_possible_wrap)]
            let got_len = out.len() as i64;
            proptest::prop_assert_eq!(got_len, count);
            for &x in &out {
                proptest::prop_assert!(x >= start && x <= start + span);
            }
        }
    }

    // The splitmix64 sequence is part of the contract: golden hash locks it
    // cross-platform (blessed via run-once).
    #[test]
    fn random_determinism_golden_hash() {
        use cicada_core::scalar::Domain;
        let slots = random(RandomIn {
            domain: Domain::new(-1.0, 1.0),
            count: 8,
            seed: 2026,
        })
        .into_iter()
        .map(|x| Some(HashedValue::new(ValueData::Number(x)).unwrap()))
        .collect();
        let list = HashedValue::new(ValueData::List(List { axis: None, slots })).unwrap();
        assert_eq!(
            list.hash().to_hex(),
            "5588a7ad3d8249b4e800df87d29ef806e67ac5f1375e269d0a9d8654c1544039"
        );
    }
}
