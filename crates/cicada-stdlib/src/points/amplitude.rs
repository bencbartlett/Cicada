//! The `amplitude` node.

use cicada_core::config::ProjectConfig;
use cicada_core::spatial::Vector;
use cicada_geom::tol;
use cicada_macros::{Ports, node};

/// Inputs for [`amplitude`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct AmplitudeIn {
    /// The vector whose direction is kept.
    pub vector: Vector,
    /// The length the result gets (negative flips the direction, 0 gives the
    /// zero vector).
    #[port(dimension = length)]
    pub length: f64,
}

/// Amplitude — the vector's direction at a given length.
///
/// # Returns
///
/// The vector scaled to `length` along its own direction.
///
/// # Panics
///
/// Panics when `vector` has no length at tolerance — a zero vector has no
/// direction to give a length to.
///
/// # Examples
///
/// ```cic
/// lean = construct_vector(x=3.0, y=4.0, z=0.0)
/// reach = amplitude(vector=lean, length=10.0)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "1",
    version = 1,
    gh = "Amplitude",
    uses_tolerance
)]
#[must_use]
pub fn amplitude(config: &ProjectConfig, input: AmplitudeIn) -> Vector {
    let len = input.vector.0.length();
    assert!(
        !tol::near_zero(len, config.tol()),
        "amplitude: vector has length {len}, within tolerance of zero — \
         a zero vector has no direction to give a length to"
    );
    // The factor first, so a dyadic ratio (10 / 5) keeps the result exact.
    Vector(input.vector.0 * (input.length / len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::points::support::testing::hex;

    #[test]
    fn amplitude_table() {
        let config = ProjectConfig::default();
        let amp = |vector, length| amplitude(&config, AmplitudeIn { vector, length });
        let close = |a: Vector, b: Vector| tol::near_zero((a.0 - b.0).length(), 1e-12);
        assert!(close(
            amp(Vector::new(3.0, 4.0, 0.0), 10.0),
            Vector::new(6.0, 8.0, 0.0)
        ));
        // A negative length flips the direction.
        assert!(close(
            amp(Vector::new(3.0, 4.0, 0.0), -5.0),
            Vector::new(-3.0, -4.0, 0.0)
        ));
        // Length 0 is the zero vector (a direction is still required).
        assert!(close(
            amp(Vector::new(0.0, 0.0, 2.0), 0.0),
            Vector::new(0.0, 0.0, 0.0)
        ));
        // The direction is kept whatever the input length.
        assert!(close(
            amp(Vector::new(0.0, -0.001, 0.0), 3.0),
            Vector::new(0.0, -3.0, 0.0)
        ));
    }

    #[test]
    #[should_panic(expected = "within tolerance of zero")]
    fn amplitude_zero_vector_is_red() {
        let _ = amplitude(
            &ProjectConfig::default(),
            AmplitudeIn {
                vector: Vector::new(0.0, 0.0, 0.0),
                length: 1.0,
            },
        );
    }

    proptest::proptest! {
        // The result has the requested magnitude and is parallel to the
        // input (their cross product vanishes), with the sign of `length`.
        #[test]
        fn property_amplitude_sets_length_keeps_direction(
            x in -1.0e3..1.0e3_f64, y in -1.0e3..1.0e3_f64, z in -1.0e3..1.0e3_f64,
            length in -100.0..100.0_f64,
        ) {
            let v = Vector::new(x, y, z);
            proptest::prop_assume!(v.0.length() > 1e-3);
            let out = amplitude(&ProjectConfig::default(), AmplitudeIn { vector: v, length });
            proptest::prop_assert!(tol::close(out.0.length(), length.abs(), 1e-9));
            proptest::prop_assert!(tol::near_zero(out.0.cross(v.0).length(), 1e-9 * v.0.length() * (1.0 + length.abs())));
            proptest::prop_assert!(out.0.dot(v.0) * length >= 0.0);
        }
    }

    // Golden hash: (3, 4, 0) at length 10 — the factor 10 / 5 = 2 is exact,
    // so the output is (6, 8, 0) to the bit (blessed via run-once).
    #[test]
    fn amplitude_determinism_golden_hash() {
        assert_eq!(
            hex(amplitude(
                &ProjectConfig::default(),
                AmplitudeIn {
                    vector: Vector::new(3.0, 4.0, 0.0),
                    length: 10.0,
                },
            )),
            "308858dd7ca522e46430270bf0ecdf41b6bb9cd60ea80f9b350cb1d15efa554c"
        );
    }
}
