//! The `floor` node.

use cicada_macros::node;

use super::UnaryIn;
/// Floor — the largest integer value not above `x` (`2.7` → `2`, `-2.2`
/// → `-3`; Grasshopper's Round component's `Floor` output — there is no
/// Floor component, so the GH tag is `Round`).
///
/// # Returns
///
/// `⌊x⌋` as a Number.
///
/// # Examples
///
/// ```cic
/// whole = floor(x=2.7)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Round")]
#[must_use]
pub fn floor(input: UnaryIn) -> f64 {
    input.x.floor()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn floor_table_cases() {
        assert_eq!(floor(UnaryIn { x: 2.7 }), 2.0);
        assert_eq!(floor(UnaryIn { x: -2.2 }), -3.0);
        assert_eq!(floor(UnaryIn { x: 3.0 }), 3.0, "integers are fixed points");
        assert_eq!(floor(UnaryIn { x: -0.5 }), -1.0);
    }

    proptest::proptest! {
        // Integral, and x lies in [floor(x), floor(x) + 1).
        #[test]
        fn floor_property_brackets_from_below(x in -1.0e9..1.0e9_f64) {
            let f = floor(UnaryIn { x });
            proptest::prop_assert_eq!(f.fract(), 0.0);
            proptest::prop_assert!(f <= x && x < f + 1.0);
        }
    }

    #[test]
    fn floor_determinism_golden_hash() {
        assert_eq!(
            hex(floor(UnaryIn { x: -2.5 })),
            "deef463f528625aadcb78c1d292b86b3cae0eb6ea67c283c9ababa78d6c7ae98"
        );
    }
}
