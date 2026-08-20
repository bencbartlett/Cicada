//! The `exp` node.

use cicada_macros::node;

use super::UnaryIn;
/// Exponential — `e^x` (`power(a=e, b=x)` with the dedicated, more accurate
/// libm routine; the inverse of `ln`; GH's Maths › Util › Power of E).
///
/// # Returns
///
/// `e^x`, always positive (`+∞` on overflow, as IEEE defines it).
///
/// # Examples
///
/// ```cic
/// growth = exp(x=2.0)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Power of E")]
#[must_use]
pub fn exp(input: UnaryIn) -> f64 {
    input.x.exp()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn exp_table_cases() {
        assert_eq!(exp(UnaryIn { x: 0.0 }), 1.0, "IEEE pins e^0 = 1");
        assert!((exp(UnaryIn { x: 1.0 }) - std::f64::consts::E).abs() < 1.0e-15);
        assert_eq!(
            exp(UnaryIn {
                x: f64::NEG_INFINITY
            }),
            0.0
        );
        assert_eq!(exp(UnaryIn { x: 1.0e4 }), f64::INFINITY, "overflow is +∞");
    }

    proptest::proptest! {
        // Positive, monotone, and e^x · e^−x ≈ 1.
        #[test]
        fn exp_property_positive_and_reciprocal(x in -300.0..300.0_f64) {
            let e = exp(UnaryIn { x });
            proptest::prop_assert!(e > 0.0);
            proptest::prop_assert!((e * exp(UnaryIn { x: -x }) - 1.0).abs() <= 1.0e-12, "reciprocal");
            proptest::prop_assert!(exp(UnaryIn { x: x + 1.0 }) > e, "monotone");
        }
    }

    #[test]
    fn exp_determinism_golden_hash() {
        // e^0 = 1 is IEEE-pinned; the rest is run-to-run identity.
        assert_eq!(
            hex(exp(UnaryIn { x: 0.0 })),
            "2f4e052fbfa58c8bfc1aefe1b236c4ad7383dd3855684a2f830d183393cbaebf"
        );
        assert_eq!(
            exp(UnaryIn { x: 2.5 }).to_bits(),
            exp(UnaryIn { x: 2.5 }).to_bits()
        );
    }
}
