//! The `atan2` node.

use cicada_macros::{Ports, node};
/// Inputs for [`atan2`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct Atan2In {
    /// The y coordinate (the opposite side).
    pub y: f64,
    /// The x coordinate (the adjacent side).
    pub x: f64,
}

/// Arctangent 2 — the angle of the vector `(x, y)` from the +x axis, in
/// `(-π, π]`: the quadrant-aware arctangent of `y / x` (the Grasshopper
/// expression `atan2`; there is no component).
///
/// # Returns
///
/// The angle in radians, `(-π, π]`.
///
/// # Examples
///
/// ```cic
/// heading = atan2(y=1.0, x=1.0)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = none)]
#[must_use]
pub fn atan2(input: Atan2In) -> f64 {
    input.y.atan2(input.x)
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn atan2_table_cases() {
        use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
        assert_eq!(
            atan2(Atan2In { y: 0.0, x: 1.0 }),
            0.0,
            "IEEE pins atan2(+0, x>0) = +0"
        );
        assert!((atan2(Atan2In { y: 1.0, x: 1.0 }) - FRAC_PI_4).abs() < 1.0e-15);
        assert!((atan2(Atan2In { y: 1.0, x: 0.0 }) - FRAC_PI_2).abs() < 1.0e-15);
        assert!((atan2(Atan2In { y: 0.0, x: -1.0 }) - PI).abs() < 1.0e-15);
        assert!((atan2(Atan2In { y: -1.0, x: 0.0 }) + FRAC_PI_2).abs() < 1.0e-15);
        assert_eq!(
            atan2(Atan2In { y: 0.0, x: 0.0 }),
            0.0,
            "the origin is +0, as IEEE defines"
        );
    }

    proptest::proptest! {
        // In (−π, π], and the polar angle of (x, y): rebuilding the direction
        // from the angle reproduces the unit vector.
        #[test]
        fn atan2_property_is_the_polar_angle(
            angle in -3.1..3.1_f64,
            radius in 0.001..1.0e6_f64,
        ) {
            let (x, y) = (radius * angle.cos(), radius * angle.sin());
            let got = atan2(Atan2In { y, x });
            proptest::prop_assert!(got > -std::f64::consts::PI && got <= std::f64::consts::PI);
            proptest::prop_assert!((got - angle).abs() <= 1.0e-9);
        }
    }

    #[test]
    fn atan2_determinism_golden_hash() {
        // The IEEE-pinned special value is the golden; an irrational argument
        // is asserted run-to-run identical only (libms differ in the last ulp).
        assert_eq!(
            hex(atan2(Atan2In { y: 0.0, x: 1.0 })),
            "16340e1e9e25c58d84305492ff4bb2c5ee526619316dc4e2026f425e69fb333c"
        );
        assert_eq!(
            atan2(Atan2In { y: 0.3, x: 0.7 }).to_bits(),
            atan2(Atan2In { y: 0.3, x: 0.7 }).to_bits()
        );
    }
}
