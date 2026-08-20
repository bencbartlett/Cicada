//! The `ln` node.

use cicada_macros::node;

use super::UnaryIn;
/// Natural Logarithm — `ln x`, the logarithm to base e.
///
/// # Returns
///
/// `ln x` (`ln 0` is −∞, as IEEE defines it).
///
/// # Panics
///
/// Panics when `x` is negative — the real logarithm is undefined there
/// (loud refusal, never a silent NaN).
///
/// # Examples
///
/// ```cic
/// decades = ln(x=100.0)
/// ```
#[node(
    category = "Maths & logic",
    tier = "1",
    version = 1,
    gh = "Natural logarithm"
)]
#[must_use]
pub fn ln(input: UnaryIn) -> f64 {
    assert!(
        input.x >= 0.0,
        "ln: x must be >= 0 (the real logarithm is undefined), got {}",
        input.x
    );
    input.x.ln()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn ln_table_cases() {
        assert_eq!(ln(UnaryIn { x: 1.0 }), 0.0, "IEEE pins ln 1 = +0");
        assert_eq!(ln(UnaryIn { x: 0.0 }), f64::NEG_INFINITY);
        assert!(
            (ln(UnaryIn {
                x: std::f64::consts::E
            }) - 1.0)
                .abs()
                < 1.0e-15
        );
        assert!((ln(UnaryIn { x: 100.0 }) - 4.605_170_185_988_092).abs() < 1.0e-14);
    }

    #[test]
    #[should_panic(expected = "x must be >= 0")]
    fn ln_of_a_negative_is_red() {
        let _ = ln(UnaryIn { x: -1.0 });
    }

    proptest::proptest! {
        // Inverse of exp within a rounding error, and monotone.
        #[test]
        fn ln_property_inverts_exp(y in -40.0..40.0_f64) {
            let x = y.exp();
            proptest::prop_assert!((ln(UnaryIn { x }) - y).abs() <= 1.0e-13 * (1.0 + y.abs()), "inverse");
            proptest::prop_assert!(ln(UnaryIn { x: x * 2.0 }) > ln(UnaryIn { x }), "monotone");
        }
    }

    #[test]
    fn ln_determinism_golden_hash() {
        // ln 1 = +0 is IEEE-pinned on every libm; the rest is run-to-run
        // identity (libms differ in the last ulp, so no irrational golden).
        assert_eq!(
            hex(ln(UnaryIn { x: 1.0 })),
            "16340e1e9e25c58d84305492ff4bb2c5ee526619316dc4e2026f425e69fb333c"
        );
        assert_eq!(
            ln(UnaryIn { x: 7.5 }).to_bits(),
            ln(UnaryIn { x: 7.5 }).to_bits()
        );
    }
}
