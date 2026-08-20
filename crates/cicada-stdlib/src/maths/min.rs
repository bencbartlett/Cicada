//! The `min` node.

use cicada_macros::node;

use super::BinaryIn;
/// Minimum — the smaller of two numbers.
///
/// # Returns
///
/// Whichever of `a` and `b` is smaller (either when equal).
///
/// # Examples
///
/// ```cic
/// pick = min(a=1.5, b=2.5)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Minimum")]
#[must_use]
pub fn min(input: BinaryIn) -> f64 {
    input.a.min(input.b)
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn min_table_cases() {
        assert_eq!(min(BinaryIn { a: 1.5, b: 2.5 }), 1.5);
        assert_eq!(min(BinaryIn { a: -1.0, b: -2.0 }), -2.0);
        assert_eq!(min(BinaryIn { a: 3.0, b: 3.0 }), 3.0, "equal operands");
        assert_eq!(
            min(BinaryIn {
                a: f64::NEG_INFINITY,
                b: f64::INFINITY
            }),
            f64::NEG_INFINITY
        );
    }

    proptest::proptest! {
        // Commutative, one of the operands, and ordered against both.
        #[test]
        fn min_property_selects_an_operand(a in -1.0e12..1.0e12_f64, b in -1.0e12..1.0e12_f64) {
            let m = min(BinaryIn { a, b });
            proptest::prop_assert_eq!(m, min(BinaryIn { a: b, b: a }));
            proptest::prop_assert!(m == a || m == b);
            proptest::prop_assert!(m <= a && m <= b);
        }
    }

    #[test]
    fn min_determinism_golden_hash() {
        assert_eq!(
            hex(min(BinaryIn { a: 1.5, b: 2.5 })),
            "193cb930efc458d6c52cd619c036f833da80d9404b8870becc567e0cbfa4ef03"
        );
    }
}
