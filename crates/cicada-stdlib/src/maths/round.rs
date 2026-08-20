//! The `round` node.

use cicada_macros::node;

use super::UnaryIn;
/// Round — the nearest integer value, ties away from zero (`2.5` → `3`,
/// `-2.5` → `-3`; Grasshopper's Round uses .NET's ties-to-even — the
/// schoolbook rule is the one users expect, and it is stated here).
///
/// # Returns
///
/// The nearest integer as a Number (`floor`/`ceiling` for one-sided
/// rounding).
///
/// # Examples
///
/// ```cic
/// nearest = round(x=2.5)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Round")]
#[must_use]
pub fn round(input: UnaryIn) -> f64 {
    input.x.round()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn round_table_cases() {
        assert_eq!(round(UnaryIn { x: 2.5 }), 3.0, "ties away from zero");
        assert_eq!(round(UnaryIn { x: -2.5 }), -3.0);
        assert_eq!(round(UnaryIn { x: 2.4 }), 2.0);
        assert_eq!(round(UnaryIn { x: 2.6 }), 3.0);
        assert_eq!(round(UnaryIn { x: -0.4 }), 0.0);
        assert_eq!(round(UnaryIn { x: 1.0e300 }), 1.0e300, "already integral");
    }

    proptest::proptest! {
        // Integral, and within half a unit of the operand.
        #[test]
        fn round_property_nearest_integer(x in -1.0e9..1.0e9_f64) {
            let r = round(UnaryIn { x });
            proptest::prop_assert_eq!(r.fract(), 0.0);
            proptest::prop_assert!((r - x).abs() <= 0.5);
        }
    }

    #[test]
    fn round_determinism_golden_hash() {
        assert_eq!(
            hex(round(UnaryIn { x: 2.5 })),
            "3bdd49f7095a9833bd7f48f0d919bf202601764cb8e6d0781e2daa887ca37caa"
        );
    }
}
