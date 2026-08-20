//! The `or` node.

use cicada_macros::node;

use super::GateIn;
/// Or — true when either operand is true.
///
/// # Returns
///
/// `a || b`.
///
/// # Examples
///
/// ```cic
/// both = or(a=True, b=False)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Gate Or")]
#[must_use]
pub fn or(input: GateIn) -> bool {
    input.a || input.b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn or_table_cases() {
        // The full truth table.
        assert!(!or(GateIn { a: false, b: false }));
        assert!(or(GateIn { a: false, b: true }));
        assert!(or(GateIn { a: true, b: false }));
        assert!(or(GateIn { a: true, b: true }));
    }

    proptest::proptest! {
        // Commutative, and it agrees with the boolean operator.
        #[test]
        fn or_property_commutative(a in proptest::bool::ANY, b in proptest::bool::ANY) {
            proptest::prop_assert_eq!(or(GateIn { a, b }), or(GateIn { a: b, b: a }));
            proptest::prop_assert_eq!(or(GateIn { a, b }), a || b);
        }
    }

    #[test]
    fn or_determinism_golden_hash() {
        assert_eq!(
            hex(or(GateIn { a: true, b: false })),
            "ba22722512edb5aa23326f7be45f93cc564eda753e7dcef017012eb24b476552"
        );
    }
}
