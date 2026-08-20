//! The `max` node.

use cicada_macros::node;

use super::BinaryIn;
/// Maximum — the larger of two numbers.
///
/// # Returns
///
/// Whichever of `a` and `b` is larger (either when equal).
///
/// # Examples
///
/// ```cic
/// upper = max(a=1.5, b=2.5)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Maximum")]
#[must_use]
pub fn max(input: BinaryIn) -> f64 {
    input.a.max(input.b)
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn max_table_cases() {
        assert_eq!(max(BinaryIn { a: 1.5, b: 2.5 }), 2.5);
        assert_eq!(max(BinaryIn { a: -1.0, b: -2.0 }), -1.0);
        assert_eq!(max(BinaryIn { a: 3.0, b: 3.0 }), 3.0, "equal operands");
        assert_eq!(
            max(BinaryIn {
                a: f64::NEG_INFINITY,
                b: f64::INFINITY
            }),
            f64::INFINITY
        );
    }

    proptest::proptest! {
        // Commutative, one of the operands, and ordered against both.
        #[test]
        fn max_property_selects_an_operand(a in -1.0e12..1.0e12_f64, b in -1.0e12..1.0e12_f64) {
            let m = max(BinaryIn { a, b });
            proptest::prop_assert_eq!(m, max(BinaryIn { a: b, b: a }));
            proptest::prop_assert!(m == a || m == b);
            proptest::prop_assert!(m >= a && m >= b);
        }
    }

    #[test]
    fn max_determinism_golden_hash() {
        assert_eq!(
            hex(max(BinaryIn { a: 1.5, b: 2.5 })),
            "6b1464756151afd7f45a66e24ed84162dec16817702ea9b7db2bf0690986588f"
        );
    }
}
