//! The `absolute` node.

use cicada_macros::node;

use super::UnaryIn;
/// Absolute — the magnitude `|x|`.
///
/// # Returns
///
/// `|x|`, never negative.
///
/// # Examples
///
/// ```cic
/// magnitude = absolute(x=-2.5)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Absolute")]
#[must_use]
pub fn absolute(input: UnaryIn) -> f64 {
    input.x.abs()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn absolute_table_cases() {
        assert_eq!(absolute(UnaryIn { x: -2.5 }), 2.5);
        assert_eq!(absolute(UnaryIn { x: 2.5 }), 2.5);
        assert_eq!(absolute(UnaryIn { x: 0.0 }), 0.0);
        assert_eq!(absolute(UnaryIn { x: f64::MIN }), f64::MAX);
        assert_eq!(
            absolute(UnaryIn {
                x: f64::NEG_INFINITY
            }),
            f64::INFINITY
        );
    }

    proptest::proptest! {
        // Non-negative, even, and exactly one of x / −x.
        #[test]
        fn absolute_property_is_even_and_exact(x in -1.0e12..1.0e12_f64) {
            let a = absolute(UnaryIn { x });
            proptest::prop_assert!(a >= 0.0);
            proptest::prop_assert_eq!(a, absolute(UnaryIn { x: -x }));
            proptest::prop_assert!(a == x || a == -x);
        }
    }

    #[test]
    fn absolute_determinism_golden_hash() {
        assert_eq!(
            hex(absolute(UnaryIn { x: -2.5 })),
            "6b1464756151afd7f45a66e24ed84162dec16817702ea9b7db2bf0690986588f"
        );
    }
}
