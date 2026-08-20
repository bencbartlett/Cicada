//! The `larger` node.

use cicada_macros::node;

use super::BinaryIn;
/// Larger Than — true when `a` is strictly above `b` (`not` of the other
/// comparison, or `equals`, gives the non-strict forms).
///
/// # Returns
///
/// `a > b`.
///
/// # Examples
///
/// ```cic
/// flag = larger(a=1.5, b=2.5)
/// ```
#[node(
    category = "Maths & logic",
    tier = "1",
    version = 1,
    gh = "Larger Than"
)]
#[must_use]
pub fn larger(input: BinaryIn) -> bool {
    input.a > input.b
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn larger_table_cases() {
        assert!(!larger(BinaryIn { a: 1.5, b: 2.5 }));
        assert!(larger(BinaryIn { a: 2.5, b: 1.5 }));
        assert!(
            !larger(BinaryIn { a: 3.0, b: 3.0 }),
            "strict: equal operands are neither"
        );
        assert!(larger(BinaryIn { a: -1.0, b: -2.0 }));
        assert!(!larger(BinaryIn {
            a: f64::NEG_INFINITY,
            b: f64::INFINITY
        }));
    }

    proptest::proptest! {
        // Irreflexive, asymmetric, and exactly one of a<b / a>b / a==b holds.
        #[test]
        fn larger_property_strict_order(a in -1.0e12..1.0e12_f64, b in -1.0e12..1.0e12_f64) {
            proptest::prop_assert!(!larger(BinaryIn { a, b: a }), "irreflexive");
            let forward = larger(BinaryIn { a, b });
            let backward = larger(BinaryIn { a: b, b: a });
            proptest::prop_assert!(!(forward && backward));
            proptest::prop_assert_eq!(forward || backward, a != b);
        }
    }

    #[test]
    fn larger_determinism_golden_hash() {
        assert_eq!(
            hex(larger(BinaryIn { a: 1.5, b: 2.5 })),
            "6968713e028ee7bef35d2eaa98f8c7f0c18df33784a242223cef9bf8cddb65f0"
        );
    }
}
