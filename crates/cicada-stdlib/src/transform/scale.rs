//! The `scale` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Transformable;
use cicada_core::spatial::Point;
use cicada_geom::transform::Similarity;
use cicada_macros::{Ports, node};

/// Inputs for [`scale`].
#[derive(Ports, Clone, Debug)]
pub struct ScaleIn {
    /// The geometry to scale.
    pub geometry: Transformable,
    /// Center of scaling.
    #[port(default = Point::origin(), default_doc = "origin")]
    pub center: Point,
    /// Uniform scale factor (negative = point reflection).
    pub factor: f64,
}

/// Scale — uniform scale about a center. Negative factors point-reflect
/// (mesh orientation is preserved by winding correction).
///
/// # Returns
///
/// The geometry scaled by `factor` about `center`.
///
/// # Panics
///
/// Panics when `|factor|` is within tolerance of zero — geometry would
/// collapse to a point — or when a `Solid` is the geometry — B-rep transforms run in the OCCT kernel and
/// arrive with the OCCT-backed solid nodes (v0.1 item 3 WP-C); until then a
/// Solid input is a loud refusal, never a silent pass-through.
///
/// # Examples
///
/// ```cic
/// ring = circle(radius=2.0)
/// about = construct_point(x=1.0, y=0.0, z=0.0)
/// bigger = scale(geometry=ring, center=about, factor=3.0)
/// ```
#[node(
    category = "Transform",
    tier = "S",
    version = 1,
    gh = "Scale",
    uses_tolerance
)]
#[must_use]
pub fn scale(config: &ProjectConfig, input: ScaleIn) -> Transformable {
    assert!(
        input.factor.abs() > config.tol(),
        "scale: |factor| = {} is within tolerance of zero — geometry would collapse",
        input.factor.abs()
    );
    Similarity::scale_about(input.center, input.factor).apply(&input.geometry)
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::{Circle, Curve};
    use cicada_core::spatial::Plane;
    use cicada_geom::tol;

    use super::*;
    use crate::transform::support::{config, expect_point, expect_point_hash, point};

    #[test]
    fn scale_about_center_table() {
        let out = scale(
            &config(),
            ScaleIn {
                geometry: point(3.0, 0.0, 0.0),
                center: Point::new(1.0, 0.0, 0.0),
                factor: 2.0,
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(5.0, 0.0, 0.0),
            1e-12
        ));
        // Circles stay circles with scaled radius (analytic exactness).
        let circle = Transformable::Curve(Curve::Circle(Circle {
            plane: Plane::world_xy(),
            radius: 2.0,
        }));
        let Transformable::Curve(Curve::Circle(scaled)) = scale(
            &config(),
            ScaleIn {
                geometry: circle,
                center: Point::origin(),
                factor: 3.0,
            },
        ) else {
            panic!("kind and variant preserved")
        };
        assert!((scaled.radius - 6.0).abs() < 1e-12);
    }

    #[test]
    #[should_panic(expected = "collapse")]
    fn scale_zero_factor_is_red() {
        let _ = scale(
            &config(),
            ScaleIn {
                geometry: point(1.0, 0.0, 0.0),
                center: Point::origin(),
                factor: 0.0,
            },
        );
    }

    proptest::proptest! {
        // Scaling multiplies distances from the center by |factor| —
        // including negative (point-reflection) factors.
        #[test]
        fn property_scale_scales_distances(
            x in -100.0..100.0_f64, y in -100.0..100.0_f64,
            cx in -50.0..50.0_f64, cy in -50.0..50.0_f64,
            factor in -8.0..8.0_f64,
        ) {
            proptest::prop_assume!(factor.abs() > 1e-3);
            let center = Point::new(cx, cy, 0.0);
            let out = scale(
                &config(),
                ScaleIn {
                    geometry: point(x, y, 0.0),
                    center,
                    factor,
                },
            );
            let got = expect_point(&out);
            let before = Point::new(x, y, 0.0).0.distance(center.0);
            let want = factor.abs() * before;
            proptest::prop_assert!((got.0.distance(center.0) - want).abs() <= 1e-9 * want.max(1.0));
        }
    }

    #[test]
    fn scale_determinism_golden_hash() {
        let out = scale(
            &config(),
            ScaleIn {
                geometry: point(3.0, -2.0, 1.0),
                center: Point::new(1.0, 1.0, 1.0),
                factor: 2.5,
            },
        );
        assert_eq!(
            expect_point_hash(&out),
            "4b937822d72c8469269a8a87c71c9df0faea13bf20ddd37ae6294d897a46c4f1"
        );
    }
}
