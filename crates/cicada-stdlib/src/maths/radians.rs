//! The `radians` node.

use cicada_macros::{Ports, node};
/// Inputs for [`radians`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct RadiansIn {
    /// The angle in degrees.
    pub degrees: f64,
}

/// Radians — convert an angle from degrees to radians (`× π/180`).
///
/// # Returns
///
/// The angle in radians.
///
/// # Examples
///
/// ```cic
/// quarter_turn = radians(degrees=90.0)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Radians")]
#[must_use]
pub fn radians(input: RadiansIn) -> f64 {
    input.degrees.to_radians()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn radians_table_cases() {
        assert_eq!(radians(RadiansIn { degrees: 0.0 }), 0.0);
        assert!((radians(RadiansIn { degrees: 180.0 }) - std::f64::consts::PI).abs() < 1.0e-15);
        assert!(
            (radians(RadiansIn { degrees: 90.0 }) - std::f64::consts::FRAC_PI_2).abs() < 1.0e-15
        );
        assert!(
            (radians(RadiansIn { degrees: -45.0 }) + std::f64::consts::FRAC_PI_4).abs() < 1.0e-15
        );
    }

    proptest::proptest! {
        // Linear, odd, and the inverse of `degrees` within a rounding error.
        #[test]
        fn radians_property_linear_inverse_of_degrees(degrees in -1.0e6..1.0e6_f64) {
            let r = radians(RadiansIn { degrees });
            proptest::prop_assert_eq!(r, -radians(RadiansIn { degrees: -degrees }));
            let back = crate::maths::degrees::degrees(crate::maths::degrees::DegreesIn { radians: r });
            proptest::prop_assert!((back - degrees).abs() <= 1.0e-9 * (1.0 + degrees.abs()));
        }
    }

    #[test]
    fn radians_determinism_golden_hash() {
        // 0° is exact; π/180 is a rounded constant, so other arguments are
        // asserted run-to-run identical only.
        assert_eq!(
            hex(radians(RadiansIn { degrees: 0.0 })),
            "16340e1e9e25c58d84305492ff4bb2c5ee526619316dc4e2026f425e69fb333c"
        );
        assert_eq!(
            radians(RadiansIn { degrees: 30.0 }).to_bits(),
            radians(RadiansIn { degrees: 30.0 }).to_bits()
        );
    }
}
