//! The `negative` node.

use cicada_macros::node;

use super::UnaryIn;
/// Negative — the negation `-x`.
///
/// # Returns
///
/// `-x`.
///
/// # Examples
///
/// ```cic
/// flipped = negative(x=2.5)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Negative")]
#[must_use]
pub fn negative(input: UnaryIn) -> f64 {
    -input.x
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn negative_table_cases() {
        assert_eq!(negative(UnaryIn { x: 2.5 }), -2.5);
        assert_eq!(negative(UnaryIn { x: -2.5 }), 2.5);
        assert_eq!(
            negative(UnaryIn { x: 0.0 }),
            0.0,
            "−0.0 == 0.0 (and seals canonical)"
        );
        assert_eq!(negative(UnaryIn { x: f64::MAX }), f64::MIN);
        assert_eq!(negative(UnaryIn { x: f64::INFINITY }), f64::NEG_INFINITY);
    }

    proptest::proptest! {
        // An exact involution whose sum with the operand is zero.
        #[test]
        fn negative_property_is_an_exact_involution(x in -1.0e12..1.0e12_f64) {
            let once = negative(UnaryIn { x });
            proptest::prop_assert_eq!(once + x, 0.0);
            proptest::prop_assert_eq!(negative(UnaryIn { x: once }), x);
        }
    }

    #[test]
    fn negative_determinism_golden_hash() {
        assert_eq!(
            hex(negative(UnaryIn { x: 2.5 })),
            "82b9021790d66fc43b9e065c5d588a8dd37a57a2a093ecaacc4d2b52de9e0385"
        );
    }
}
