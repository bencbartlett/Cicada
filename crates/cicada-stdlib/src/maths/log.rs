//! The `log` node.

use cicada_macros::{Ports, node};
/// Inputs for [`log`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct LogIn {
    /// The operand.
    pub x: f64,
    /// The logarithm base.
    #[port(default = 10.0)]
    pub base: f64,
}

/// Logarithm — `log_base x`, base 10 by default (`ln` for base e). Bases
/// 10 and 2 use the dedicated libm routines (so `log(1000)` is `3`, not
/// `2.9999…`); any other base is `ln x / ln base`.
///
/// # Returns
///
/// `log_base x` (`log 0` is −∞, as IEEE defines it).
///
/// # Panics
///
/// Panics when `x` is negative, or when `base` is not positive or is `1`
/// — the logarithm is undefined there (loud refusal, never a silent NaN).
///
/// # Examples
///
/// ```cic
/// decades = log(x=1000.0)
/// octaves = log(x=8.0, base=2.0)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Logarithm")]
#[must_use]
#[allow(clippy::float_cmp)] // exact base detection — the contract says bases 10 and 2
pub fn log(input: LogIn) -> f64 {
    assert!(
        input.x >= 0.0,
        "log: x must be >= 0 (the real logarithm is undefined), got {}",
        input.x
    );
    assert!(
        input.base > 0.0 && input.base != 1.0,
        "log: base must be positive and not 1, got {}",
        input.base
    );
    if input.base == 10.0 {
        input.x.log10()
    } else if input.base == 2.0 {
        input.x.log2()
    } else {
        input.x.ln() / input.base.ln()
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn log_table_cases() {
        assert_eq!(
            log(LogIn { x: 1.0, base: 10.0 }),
            0.0,
            "IEEE pins log 1 = +0"
        );
        assert!(
            (log(LogIn {
                x: 1000.0,
                base: 10.0
            }) - 3.0)
                .abs()
                < 1.0e-15
        );
        assert!((log(LogIn { x: 8.0, base: 2.0 }) - 3.0).abs() < 1.0e-15);
        assert!((log(LogIn { x: 27.0, base: 3.0 }) - 3.0).abs() < 1.0e-14);
        assert_eq!(log(LogIn { x: 0.0, base: 10.0 }), f64::NEG_INFINITY);
        assert!(
            log(LogIn { x: 2.0, base: 0.5 }) < 0.0,
            "a base below 1 flips the sign"
        );
    }

    #[test]
    #[should_panic(expected = "x must be >= 0")]
    fn log_of_a_negative_is_red() {
        let _ = log(LogIn {
            x: -1.0,
            base: 10.0,
        });
    }

    #[test]
    #[should_panic(expected = "base must be positive and not 1")]
    fn log_base_one_is_red() {
        let _ = log(LogIn { x: 10.0, base: 1.0 });
    }

    #[test]
    #[should_panic(expected = "base must be positive and not 1")]
    fn log_non_positive_base_is_red() {
        let _ = log(LogIn { x: 10.0, base: 0.0 });
    }

    proptest::proptest! {
        // log_b(b^k) ≈ k for every base, within a rounding error.
        #[test]
        fn log_property_inverts_the_power(base in 1.5..20.0_f64, k in -8.0..8.0_f64) {
            let x = base.powf(k);
            let got = log(LogIn { x, base });
            proptest::prop_assert!((got - k).abs() <= 1.0e-12 * (1.0 + k.abs()));
        }
    }

    #[test]
    fn log_determinism_golden_hash() {
        // log 1 = +0 is IEEE-pinned; the rest is run-to-run identity.
        assert_eq!(
            hex(log(LogIn { x: 1.0, base: 10.0 })),
            "16340e1e9e25c58d84305492ff4bb2c5ee526619316dc4e2026f425e69fb333c"
        );
        assert_eq!(
            log(LogIn { x: 7.5, base: 3.0 }).to_bits(),
            log(LogIn { x: 7.5, base: 3.0 }).to_bits()
        );
    }
}
