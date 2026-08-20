//! The `degrees` node.

use cicada_macros::{Ports, node};
/// Inputs for [`degrees`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct DegreesIn {
    /// The angle in radians.
    #[port(dimension = angle)]
    pub radians: f64,
}

/// Degrees — convert an angle from radians to degrees (`× 180/π`).
///
/// # Returns
///
/// The angle in degrees.
///
/// # Examples
///
/// ```cic
/// readable = degrees(radians=1.5707963267948966)
/// ```
#[node(category = "Maths & logic", tier = "1", version = 1, gh = "Degrees")]
#[must_use]
pub fn degrees(input: DegreesIn) -> f64 {
    input.radians.to_degrees()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact IEEE arithmetic is this node's contract
mod tests {
    use super::*;
    use crate::maths::support::testing::hex;

    #[test]
    fn degrees_table_cases() {
        assert_eq!(degrees(DegreesIn { radians: 0.0 }), 0.0);
        assert!(
            (degrees(DegreesIn {
                radians: std::f64::consts::PI
            }) - 180.0)
                .abs()
                < 1.0e-12
        );
        assert!(
            (degrees(DegreesIn {
                radians: std::f64::consts::FRAC_PI_2
            }) - 90.0)
                .abs()
                < 1.0e-12
        );
        assert!(
            (degrees(DegreesIn {
                radians: -std::f64::consts::FRAC_PI_4
            }) + 45.0)
                .abs()
                < 1.0e-12
        );
    }

    proptest::proptest! {
        // Linear, odd, and the inverse of `radians` within a rounding error.
        #[test]
        fn degrees_property_linear_inverse_of_radians(radians in -1.0e4..1.0e4_f64) {
            let d = degrees(DegreesIn { radians });
            proptest::prop_assert_eq!(d, -degrees(DegreesIn { radians: -radians }));
            let back = crate::maths::radians::radians(crate::maths::radians::RadiansIn { degrees: d });
            proptest::prop_assert!((back - radians).abs() <= 1.0e-9 * (1.0 + radians.abs()));
        }
    }

    #[test]
    fn degrees_determinism_golden_hash() {
        // 0 rad is exact; 180/π is a rounded constant, so other arguments are
        // asserted run-to-run identical only.
        assert_eq!(
            hex(degrees(DegreesIn { radians: 0.0 })),
            "16340e1e9e25c58d84305492ff4bb2c5ee526619316dc4e2026f425e69fb333c"
        );
        assert_eq!(
            degrees(DegreesIn { radians: 0.5 }).to_bits(),
            degrees(DegreesIn { radians: 0.5 }).to_bits()
        );
    }
}
