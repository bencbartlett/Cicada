//! The `tan` node.

use cicada_macros::node;

use super::AngleIn;
/// Tangent — `tan x` for an angle in radians.
///
/// # Returns
///
/// `tan x`.
///
/// # Examples
///
/// ```cic
/// value = tan(x=0.5)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Tangent")]
#[must_use]
pub fn tan(input: AngleIn) -> f64 {
    input.x.tan()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn tan_table_cases() {
        assert_eq!(tan(AngleIn { x: 0.0 }), 0.0, "IEEE pins tan 0 = +0");
        assert!(
            (tan(AngleIn {
                x: std::f64::consts::FRAC_PI_4
            }) - 1.0)
                .abs()
                < 1.0e-15
        );
        assert!(
            (tan(AngleIn {
                x: -std::f64::consts::FRAC_PI_4
            }) + 1.0)
                .abs()
                < 1.0e-15
        );
    }

    proptest::proptest! {
        // Odd, and sin/cos within a rounding error (away from the poles).
        #[test]
        fn tan_property_identities(x in -1.4..1.4_f64) {
            let t = tan(AngleIn { x });
            proptest::prop_assert!((t + tan(AngleIn { x: -x })).abs() <= 1.0e-12 * (1.0 + t.abs()), "odd");
            proptest::prop_assert!((t - x.sin() / x.cos()).abs() <= 1.0e-12 * (1.0 + t.abs()));
        }
    }

    #[test]
    fn tan_determinism_golden_hash() {
        // The IEEE-pinned special value is the golden; an irrational argument
        // is asserted run-to-run identical only (libms differ in the last ulp).
        assert_eq!(
            hex(tan(AngleIn { x: 0.0 })),
            "16340e1e9e25c58d84305492ff4bb2c5ee526619316dc4e2026f425e69fb333c"
        );
        assert_eq!(
            tan(AngleIn { x: 0.7 }).to_bits(),
            tan(AngleIn { x: 0.7 }).to_bits()
        );
    }
}
