//! The `equals` node.

use cicada_macros::{Ports, node};
/// Inputs for [`equals`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct EqualsIn {
    /// Left operand.
    pub a: f64,
    /// Right operand.
    pub b: f64,
    /// How far apart `a` and `b` may be and still count as equal (`0.0` is
    /// exact IEEE equality; pass the project tolerance for measured values).
    #[port(default = 0.0)]
    pub tolerance: f64,
}

/// Equals — true when two numbers are equal within a tolerance
/// (`|a - b| <= tolerance`; exact by default).
///
/// # Returns
///
/// `true` when `a` and `b` are within `tolerance` of each other.
///
/// # Panics
///
/// Panics when `tolerance` is negative — nothing is within a negative
/// distance.
///
/// # Examples
///
/// ```cic
/// same = equals(a=0.1, b=0.1)
/// close = equals(a=0.1, b=0.1000001, tolerance=0.001)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Equality")]
#[must_use]
#[allow(clippy::float_cmp)] // exact equality IS the contract at tolerance 0 (pure maths)
pub fn equals(input: EqualsIn) -> bool {
    assert!(
        input.tolerance >= 0.0,
        "equals: tolerance must be >= 0, got {}",
        input.tolerance
    );
    // The exact test first: infinities compare equal to themselves, where the
    // difference would be NaN.
    input.a == input.b || (input.a - input.b).abs() <= input.tolerance
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn equals_table_cases() {
        assert!(equals(EqualsIn {
            a: 0.1,
            b: 0.1,
            tolerance: 0.0
        }));
        assert!(
            !equals(EqualsIn {
                a: 0.1,
                b: 0.1 + f64::EPSILON,
                tolerance: 0.0
            }),
            "exact by default"
        );
        assert!(equals(EqualsIn {
            a: 0.1,
            b: 0.100_000_1,
            tolerance: 0.001
        }));
        assert!(!equals(EqualsIn {
            a: 0.1,
            b: 0.2,
            tolerance: 0.05
        }));
        assert!(
            equals(EqualsIn {
                a: 1.0,
                b: 1.5,
                tolerance: 0.5
            }),
            "the boundary is inclusive"
        );
        assert!(equals(EqualsIn {
            a: f64::INFINITY,
            b: f64::INFINITY,
            tolerance: 0.0
        }));
        assert!(!equals(EqualsIn {
            a: f64::INFINITY,
            b: f64::NEG_INFINITY,
            tolerance: 1.0e300
        }));
    }

    #[test]
    #[should_panic(expected = "tolerance must be >= 0")]
    fn equals_negative_tolerance_is_red() {
        let _ = equals(EqualsIn {
            a: 1.0,
            b: 1.0,
            tolerance: -1.0e-9,
        });
    }

    proptest::proptest! {
        // Reflexive and symmetric; widening the tolerance never turns true
        // into false; exact equality is what `==` says.
        #[test]
        fn equals_property_reflexive_symmetric_monotone(
            a in -1.0e6..1.0e6_f64,
            b in -1.0e6..1.0e6_f64,
            tolerance in 0.0..10.0_f64,
        ) {
            proptest::prop_assert!(equals(EqualsIn { a, b: a, tolerance }), "reflexive");
            let forward = equals(EqualsIn { a, b, tolerance });
            proptest::prop_assert_eq!(forward, equals(EqualsIn { a: b, b: a, tolerance }));
            if forward {
                proptest::prop_assert!(
                equals(EqualsIn { a, b, tolerance: tolerance * 2.0 }),
                "monotone in the tolerance"
            );
            }
            proptest::prop_assert_eq!(equals(EqualsIn { a, b, tolerance: 0.0 }), a == b);
        }
    }

    #[test]
    fn equals_determinism_golden_hash() {
        assert_eq!(
            hex(equals(EqualsIn {
                a: 0.5,
                b: 0.75,
                tolerance: 0.5
            })),
            "ba22722512edb5aa23326f7be45f93cc564eda753e7dcef017012eb24b476552"
        );
    }
}
