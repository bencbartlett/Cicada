//! The `xor` node.

use cicada_macros::node;

use super::GateIn;
/// Xor — true when exactly one operand is true.
///
/// # Returns
///
/// `a ^ b`.
///
/// # Examples
///
/// ```cic
/// both = xor(a=True, b=False)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Gate Xor")]
#[must_use]
pub fn xor(input: GateIn) -> bool {
    input.a ^ input.b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn xor_table_cases() {
        // The full truth table.
        assert!(!xor(GateIn { a: false, b: false }));
        assert!(xor(GateIn { a: false, b: true }));
        assert!(xor(GateIn { a: true, b: false }));
        assert!(!xor(GateIn { a: true, b: true }));
    }

    proptest::proptest! {
        // Commutative, and it agrees with the boolean operator.
        #[test]
        fn xor_property_commutative(a in proptest::bool::ANY, b in proptest::bool::ANY) {
            proptest::prop_assert_eq!(xor(GateIn { a, b }), xor(GateIn { a: b, b: a }));
            proptest::prop_assert_eq!(xor(GateIn { a, b }), a ^ b);
        }
    }

    #[test]
    fn xor_determinism_golden_hash() {
        assert_eq!(
            hex(xor(GateIn { a: true, b: false })),
            "ba22722512edb5aa23326f7be45f93cc564eda753e7dcef017012eb24b476552"
        );
    }
}
