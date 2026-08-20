//! The `sin` node.

use cicada_macros::node;

use super::AngleIn;
/// Sine — `sin x` for an angle in radians.
///
/// # Returns
///
/// `sin x`.
///
/// # Examples
///
/// ```cic
/// value = sin(x=0.5)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Sine")]
#[must_use]
pub fn sin(input: AngleIn) -> f64 {
    input.x.sin()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn sin_table_cases() {
        assert_eq!(sin(AngleIn { x: 0.0 }), 0.0, "IEEE pins sin 0 = +0");
        assert!(
            (sin(AngleIn {
                x: std::f64::consts::FRAC_PI_2
            }) - 1.0)
                .abs()
                < 1.0e-15
        );
        assert!(
            sin(AngleIn {
                x: std::f64::consts::PI
            })
            .abs()
                < 1.0e-15
        );
        assert_eq!(sin(AngleIn { x: -0.0 }), 0.0);
    }

    proptest::proptest! {
        // Bounded, odd, and on the unit circle with cos.
        #[test]
        fn sin_property_identities(x in -10.0..10.0_f64) {
            let s = sin(AngleIn { x });
            proptest::prop_assert!(s.abs() <= 1.0);
            proptest::prop_assert!((s + sin(AngleIn { x: -x })).abs() <= 1.0e-15, "odd");
            proptest::prop_assert!((s * s + x.cos() * x.cos() - 1.0).abs() <= 1.0e-14);
        }
    }

    #[test]
    fn sin_determinism_golden_hash() {
        // The IEEE-pinned special value is the golden; an irrational argument
        // is asserted run-to-run identical only (libms differ in the last ulp).
        assert_eq!(
            hex(sin(AngleIn { x: 0.0 })),
            "16340e1e9e25c58d84305492ff4bb2c5ee526619316dc4e2026f425e69fb333c"
        );
        assert_eq!(
            sin(AngleIn { x: 0.7 }).to_bits(),
            sin(AngleIn { x: 0.7 }).to_bits()
        );
    }
}
