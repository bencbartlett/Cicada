//! The `and` node.

use cicada_macros::node;

use super::GateIn;
/// And — true when both operands are true.
///
/// # Returns
///
/// `a && b`.
///
/// # Examples
///
/// ```cic
/// both = and(a=True, b=False)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Gate And")]
#[must_use]
pub fn and(input: GateIn) -> bool {
    input.a && input.b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn and_table_cases() {
        // The full truth table.
        assert!(!and(GateIn { a: false, b: false }));
        assert!(!and(GateIn { a: false, b: true }));
        assert!(!and(GateIn { a: true, b: false }));
        assert!(and(GateIn { a: true, b: true }));
    }

    proptest::proptest! {
        // Commutative, and it agrees with the boolean operator.
        #[test]
        fn and_property_commutative(a in proptest::bool::ANY, b in proptest::bool::ANY) {
            proptest::prop_assert_eq!(and(GateIn { a, b }), and(GateIn { a: b, b: a }));
            proptest::prop_assert_eq!(and(GateIn { a, b }), a && b);
        }
    }

    #[test]
    fn and_determinism_golden_hash() {
        assert_eq!(
            hex(and(GateIn { a: true, b: false })),
            "6968713e028ee7bef35d2eaa98f8c7f0c18df33784a242223cef9bf8cddb65f0"
        );
    }
}
