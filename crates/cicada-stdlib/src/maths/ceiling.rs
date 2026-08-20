//! The `ceiling` node.

use cicada_macros::node;

use super::UnaryIn;
/// Ceiling — the smallest integer value not below `x` (`2.2` → `3`,
/// `-2.7` → `-2`; Grasshopper's Round component's `Ceiling` output — there
/// is no Ceiling component, so the GH tag is `Round`).
///
/// # Returns
///
/// `⌈x⌉` as a Number.
///
/// # Examples
///
/// ```cic
/// whole = ceiling(x=2.2)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Round")]
#[must_use]
pub fn ceiling(input: UnaryIn) -> f64 {
    input.x.ceil()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn ceiling_table_cases() {
        assert_eq!(ceiling(UnaryIn { x: 2.2 }), 3.0);
        assert_eq!(ceiling(UnaryIn { x: -2.7 }), -2.0);
        assert_eq!(
            ceiling(UnaryIn { x: 3.0 }),
            3.0,
            "integers are fixed points"
        );
        assert_eq!(ceiling(UnaryIn { x: -0.5 }), 0.0);
    }

    proptest::proptest! {
        // Integral, and x lies in (ceiling(x) − 1, ceiling(x)].
        #[test]
        fn ceiling_property_brackets_from_above(x in -1.0e9..1.0e9_f64) {
            let c = ceiling(UnaryIn { x });
            proptest::prop_assert_eq!(c.fract(), 0.0);
            proptest::prop_assert!(c - 1.0 < x && x <= c);
        }
    }

    #[test]
    fn ceiling_determinism_golden_hash() {
        assert_eq!(
            hex(ceiling(UnaryIn { x: 2.5 })),
            "3bdd49f7095a9833bd7f48f0d919bf202601764cb8e6d0781e2daa887ca37caa"
        );
    }
}
