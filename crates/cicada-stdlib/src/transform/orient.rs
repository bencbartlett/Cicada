//! The `orient` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Transformable;
use cicada_core::spatial::Plane;
use cicada_geom::frame::orthonormal;
use cicada_geom::transform::Similarity;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`orient`].
#[derive(Ports, Clone, Debug)]
pub struct OrientIn {
    /// The geometry to orient.
    pub geometry: Transformable,
    /// The frame the geometry currently sits in.
    pub source: Plane,
    /// The frame to carry it onto.
    pub target: Plane,
}

/// Orient — the rigid motion carrying the source plane onto the target
/// plane (the wall's part-to-plate workhorse).
///
/// # Panics
///
/// Panics when either plane is degenerate.
#[node(category = "Transform", tier = "S", version = 1, uses_tolerance)]
#[must_use]
pub fn orient(config: &ProjectConfig, input: OrientIn) -> Transformable {
    let source = red(orthonormal(&input.source, config.tol()));
    let target = red(orthonormal(&input.target, config.tol()));
    Similarity::plane_remap(&source, &target).apply(&input.geometry)
}

#[cfg(test)]
mod tests {
    use cicada_core::spatial::{Point, Vector};
    use cicada_geom::tol;

    use super::*;
    use crate::transform::support::{config, expect_point, expect_point_hash, point};

    #[test]
    fn orient_carries_source_to_target() {
        let out = orient(
            &config(),
            OrientIn {
                geometry: point(1.0, 0.0, 0.0),
                source: Plane::world_xy(),
                target: Plane {
                    origin: Point::new(10.0, 0.0, 0.0),
                    ..Plane::world_xy()
                },
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(11.0, 0.0, 0.0),
            1e-12
        ));
    }

    proptest::proptest! {
        // Orient onto an axis-permuted target frame re-expresses local
        // coordinates exactly (a rigid motion, pure arithmetic here).
        #[test]
        fn property_orient_remaps_coordinates(
            px in -100.0..100.0_f64, py in -100.0..100.0_f64, pz in -100.0..100.0_f64,
            tx in -100.0..100.0_f64, ty in -100.0..100.0_f64, tz in -100.0..100.0_f64,
        ) {
            // target: x̂ = world y, ŷ = world z, so ẑ = world x.
            let target = Plane {
                origin: Point::new(tx, ty, tz),
                x: Vector::new(0.0, 1.0, 0.0),
                y: Vector::new(0.0, 0.0, 1.0),
            };
            let out = orient(
                &config(),
                OrientIn {
                    geometry: point(px, py, pz),
                    source: Plane::world_xy(),
                    target,
                },
            );
            let want = Point::new(tx + pz, ty + px, tz + py);
            proptest::prop_assert!(tol::coincident(expect_point(&out), want, 1e-9));
        }
    }

    #[test]
    fn orient_determinism_golden_hash() {
        let out = orient(
            &config(),
            OrientIn {
                geometry: point(1.0, 2.0, 3.0),
                source: Plane::world_xy(),
                target: Plane {
                    origin: Point::new(10.0, -5.0, 2.5),
                    x: Vector::new(0.0, 1.0, 0.0),
                    y: Vector::new(0.0, 0.0, 1.0),
                },
            },
        );
        assert_eq!(
            expect_point_hash(&out),
            "475816028129da08b14677700e053eed31a98af10a7a2850650d2893e1a4a736"
        );
    }
}
