//! The `angle` node.

use cicada_core::config::ProjectConfig;
use cicada_geom::tol;
use cicada_macros::node;

use super::VectorPairIn;

/// Angle — the unsigned angle between two vectors, in radians (0 for the
/// same direction, π/2 perpendicular, π opposed; never reflex — there is no
/// reference plane to give it a sign).
///
/// # Returns
///
/// The angle between `a` and `b`, in radians, in `[0, π]`.
///
/// # Panics
///
/// Panics when `a` or `b` has no length at tolerance — a zero vector points
/// nowhere, so no angle is defined.
///
/// # Examples
///
/// ```cic
/// along = unit_x(factor=2.0)
/// lean = construct_vector(x=1.0, y=1.0, z=0.0)
/// turn = angle(a=along, b=lean)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "1",
    version = 1,
    gh = "Angle",
    uses_tolerance
)]
#[must_use]
pub fn angle(config: &ProjectConfig, input: VectorPairIn) -> f64 {
    let (a, b) = (input.a.0, input.b.0);
    for (name, len) in [("a", a.length()), ("b", b.length())] {
        assert!(
            !tol::near_zero(len, config.tol()),
            "angle: {name} has length {len}, within tolerance of zero — \
             a zero vector points nowhere, so no angle is defined"
        );
    }
    // atan2(|a × b|, a · b): well-conditioned at every angle, and it never
    // needs the clamp an acos(a · b / |a||b|) needs when rounding pushes
    // the cosine past ±1.
    a.cross(b).length().atan2(a.dot(b))
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

    use cicada_core::spatial::Vector;

    use super::*;
    use crate::points::support::testing::hex;

    #[test]
    fn angle_table() {
        let config = ProjectConfig::default();
        let between = |a, b| angle(&config, VectorPairIn { a, b });
        let x = Vector::new(1.0, 0.0, 0.0);
        assert!(tol::near_zero(between(x, Vector::new(3.0, 0.0, 0.0)), 0.0));
        assert!(tol::close(
            between(x, Vector::new(0.0, -2.0, 0.0)),
            FRAC_PI_2,
            1e-12
        ));
        assert!(tol::close(
            between(x, Vector::new(-0.5, 0.0, 0.0)),
            PI,
            1e-12
        ));
        assert!(tol::close(
            between(x, Vector::new(1.0, 1.0, 0.0)),
            FRAC_PI_4,
            1e-12
        ));
        assert!(tol::close(
            between(x, Vector::new(-1.0, 0.0, 1.0)),
            3.0 * FRAC_PI_4,
            1e-12
        ));
        // Lengths do not matter, only directions.
        assert!(tol::close(
            between(Vector::new(0.0, 0.0, 1.0e-3), Vector::new(0.0, 1.0e3, 0.0)),
            FRAC_PI_2,
            1e-12
        ));
    }

    #[test]
    #[should_panic(expected = "a has length 0")]
    fn angle_zero_a_is_red() {
        let _ = angle(
            &ProjectConfig::default(),
            VectorPairIn {
                a: Vector::new(0.0, 0.0, 0.0),
                b: Vector::new(1.0, 0.0, 0.0),
            },
        );
    }

    #[test]
    #[should_panic(expected = "b has length 0")]
    fn angle_zero_b_is_red() {
        let _ = angle(
            &ProjectConfig::default(),
            VectorPairIn {
                a: Vector::new(1.0, 0.0, 0.0),
                b: Vector::new(0.0, 0.0, 0.0),
            },
        );
    }

    proptest::proptest! {
        // Symmetric, in [0, π], invariant under positive scaling of either
        // vector, and flipped by negating one of them (θ ↦ π − θ).
        #[test]
        fn property_angle_is_symmetric_and_scale_free(
            ax in -1.0e3..1.0e3_f64, ay in -1.0e3..1.0e3_f64, az in -1.0e3..1.0e3_f64,
            bx in -1.0e3..1.0e3_f64, by in -1.0e3..1.0e3_f64, bz in -1.0e3..1.0e3_f64,
            s in 0.001..1.0e3_f64,
        ) {
            let (a, b) = (Vector::new(ax, ay, az), Vector::new(bx, by, bz));
            proptest::prop_assume!(a.0.length() > 1e-3 && b.0.length() > 1e-3);
            let config = ProjectConfig::default();
            let between = |a, b| angle(&config, VectorPairIn { a, b });
            let theta = between(a, b);
            proptest::prop_assert!((0.0..=PI).contains(&theta));
            proptest::prop_assert!(tol::close(theta, between(b, a), 1e-12));
            proptest::prop_assert!(tol::close(theta, between(Vector(a.0 * s), b), 1e-9));
            proptest::prop_assert!(tol::close(PI - theta, between(Vector(-a.0), b), 1e-9));
        }
    }

    // Golden hashes: the three angles IEEE 754 / C99 Annex F pin to exact
    // values whatever the libm — atan2(+0, x > 0) = +0, atan2(y > 0, ±0) =
    // π/2 and atan2(+0, x < 0) = π are special cases every implementation
    // returns as the constant, not an approximation (blessed via run-once).
    // A general angle is libm-fed and is asserted for run-to-run identity
    // only.
    #[test]
    fn angle_determinism_golden_hash() {
        let config = ProjectConfig::default();
        let between = |a, b| angle(&config, VectorPairIn { a, b });
        let x = Vector::new(2.0, 0.0, 0.0);
        assert_eq!(
            [
                hex(between(x, Vector::new(0.5, 0.0, 0.0))),
                hex(between(x, Vector::new(0.0, 3.0, 0.0))),
                hex(between(x, Vector::new(-1.0, 0.0, 0.0))),
            ],
            [
                "16340e1e9e25c58d84305492ff4bb2c5ee526619316dc4e2026f425e69fb333c",
                "e0ef0cb8f8cf63b04137dfe50c60dd6c12dbd33a729c144ab4530d3c131fed9e",
                "07f90e8a033ff9aa3462af11faaf6d001de35fb3a7e90c839cba7bd67a0e9c8f",
            ]
        );
        let general = || between(Vector::new(1.0, 2.0, 3.0), Vector::new(-4.0, 5.0, 0.5));
        assert_eq!(hex(general()), hex(general()));
    }
}
