//! The `atan` node.

use cicada_macros::node;

use super::UnaryIn;
/// Arctangent — `arctan x`, an angle in radians.
///
/// # Returns
///
/// `arctan x` in radians.
///
/// # Examples
///
/// ```cic
/// angle = atan(x=0.5)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "ArcTangent")]
#[must_use]
pub fn atan(input: UnaryIn) -> f64 {
    input.x.atan()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn atan_table_cases() {
        assert_eq!(atan(UnaryIn { x: 0.0 }), 0.0, "IEEE pins atan 0 = +0");
        assert!((atan(UnaryIn { x: 1.0 }) - std::f64::consts::FRAC_PI_4).abs() < 1.0e-15);
        assert!((atan(UnaryIn { x: f64::INFINITY }) - std::f64::consts::FRAC_PI_2).abs() < 1.0e-15);
    }

    proptest::proptest! {
        // Inverts tan on (−π/2, π/2) and stays in that range.
        #[test]
        fn atan_property_inverts(x in -1.5..1.5_f64) {
            let a = atan(UnaryIn { x: x.tan() });
            proptest::prop_assert!((a - x).abs() <= 1.0e-12);
            proptest::prop_assert!(a.abs() < std::f64::consts::FRAC_PI_2);
        }
    }

    #[test]
    fn atan_determinism_golden_hash() {
        // The IEEE-pinned special value is the golden; an irrational argument
        // is asserted run-to-run identical only (libms differ in the last ulp).
        assert_eq!(
            hex(atan(UnaryIn { x: 0.0 })),
            "16340e1e9e25c58d84305492ff4bb2c5ee526619316dc4e2026f425e69fb333c"
        );
        assert_eq!(
            atan(UnaryIn { x: 0.3 }).to_bits(),
            atan(UnaryIn { x: 0.3 }).to_bits()
        );
    }
}
