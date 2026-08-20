//! The `smaller` node.

use cicada_macros::node;

use super::BinaryIn;
/// Smaller Than — true when `a` is strictly below `b` (`not` of the other
/// comparison, or `equals`, gives the non-strict forms).
///
/// # Returns
///
/// `a < b`.
///
/// # Examples
///
/// ```cic
/// flag = smaller(a=1.5, b=2.5)
/// ```
#[node(
    category = "Maths & logic",
    tier = "1",
    version = 1,
    gh = "Smaller Than"
)]
#[must_use]
pub fn smaller(input: BinaryIn) -> bool {
    input.a < input.b
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn smaller_table_cases() {
        assert!(smaller(BinaryIn { a: 1.5, b: 2.5 }));
        assert!(!smaller(BinaryIn { a: 2.5, b: 1.5 }));
        assert!(
            !smaller(BinaryIn { a: 3.0, b: 3.0 }),
            "strict: equal operands are neither"
        );
        assert!(!smaller(BinaryIn { a: -1.0, b: -2.0 }));
        assert!(smaller(BinaryIn {
            a: f64::NEG_INFINITY,
            b: f64::INFINITY
        }));
    }

    proptest::proptest! {
        // Irreflexive, asymmetric, and exactly one of a<b / a>b / a==b holds.
        #[test]
        fn smaller_property_strict_order(a in -1.0e12..1.0e12_f64, b in -1.0e12..1.0e12_f64) {
            proptest::prop_assert!(!smaller(BinaryIn { a, b: a }), "irreflexive");
            let forward = smaller(BinaryIn { a, b });
            let backward = smaller(BinaryIn { a: b, b: a });
            proptest::prop_assert!(!(forward && backward));
            proptest::prop_assert_eq!(forward || backward, a != b);
        }
    }

    #[test]
    fn smaller_determinism_golden_hash() {
        assert_eq!(
            hex(smaller(BinaryIn { a: 1.5, b: 2.5 })),
            "ba22722512edb5aa23326f7be45f93cc564eda753e7dcef017012eb24b476552"
        );
    }
}
