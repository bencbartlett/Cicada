//! The `not` node.

use cicada_macros::{Ports, node};
/// Inputs for [`not`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct NotIn {
    /// The operand.
    pub x: bool,
}

/// Not — the boolean negation.
///
/// # Returns
///
/// `true` when `x` is false, and vice versa.
///
/// # Examples
///
/// ```cic
/// inverted = not(x=True)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Gate Not")]
#[must_use]
pub fn not(input: NotIn) -> bool {
    !input.x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn not_table_cases() {
        assert!(!not(NotIn { x: true }));
        assert!(not(NotIn { x: false }));
    }

    proptest::proptest! {
        // An involution that never returns its operand.
        #[test]
        fn not_property_involution(x in proptest::bool::ANY) {
            proptest::prop_assert_ne!(not(NotIn { x }), x);
            proptest::prop_assert_eq!(not(NotIn { x: not(NotIn { x }) }), x);
        }
    }

    #[test]
    fn not_determinism_golden_hash() {
        assert_eq!(
            hex(not(NotIn { x: true })),
            "6968713e028ee7bef35d2eaa98f8c7f0c18df33784a242223cef9bf8cddb65f0"
        );
    }
}
