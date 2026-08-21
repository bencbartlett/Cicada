//! The `rotate` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Transformable;
use cicada_core::spatial::Plane;
use cicada_geom::frame::orthonormal;
use cicada_geom::transform::Similarity;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`rotate`].
#[derive(Ports, Clone, Debug)]
pub struct RotateIn {
    /// The geometry to rotate.
    pub geometry: Transformable,
    /// Rotation angle in radians (right-handed about the plane's normal).
    #[port(dimension = angle)]
    pub angle: f64,
    /// Rotation frame: about its z axis, through its origin.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
}

/// Rotate — rotate geometry about a plane's normal through its origin.
///
/// # Returns
///
/// The geometry rotated by `angle` about the plane's normal.
///
/// # Panics
///
/// Panics when the plane is degenerate (zero-length or parallel axes), or
/// for a `Solid` the OCCT kernel refuses to transform (a `Solid` moves through
/// the kernel — its B-rep geometry is rewritten, never a mesh in disguise).
///
/// # Examples
///
/// ```cic
/// ring = circle(radius=2.0)
/// turned = rotate(geometry=ring, angle=1.5707963267948966)
/// ```
#[node(
    category = "Transform",
    tier = "S",
    version = 1,
    gh = "Rotate",
    uses_tolerance
)]
#[must_use]
pub fn rotate(config: &ProjectConfig, input: RotateIn) -> Transformable {
    let frame = red(orthonormal(&input.plane, config.tol()));
    Similarity::rotation(&frame, input.angle).apply(&input.geometry)
}

#[cfg(test)]
mod tests {
    use cicada_core::spatial::Point;
    use cicada_geom::tol;

    use super::*;
    use crate::transform::support::{config, expect_point, expect_point_hash, point};

    #[test]
    fn rotate_quarter_turn_table() {
        let out = rotate(
            &config(),
            RotateIn {
                geometry: point(1.0, 0.0, 0.0),
                angle: std::f64::consts::FRAC_PI_2,
                plane: Plane::world_xy(),
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(0.0, 1.0, 0.0),
            1e-12
        ));
    }

    proptest::proptest! {
        // Rotation about world z is an isometry that fixes z: distance to
        // the axis origin is preserved for ANY angle.
        #[test]
        fn property_rotate_preserves_radius(
            x in -100.0..100.0_f64, y in -100.0..100.0_f64, z in -100.0..100.0_f64,
            angle in -10.0..10.0_f64,
        ) {
            let out = rotate(
                &config(),
                RotateIn {
                    geometry: point(x, y, z),
                    angle,
                    plane: Plane::world_xy(),
                },
            );
            let got = expect_point(&out);
            let before = Point::new(x, y, z).0.length();
            proptest::prop_assert!((got.0.length() - before).abs() <= 1e-9 * before.max(1.0));
            proptest::prop_assert!((got.0.z - z).abs() <= 1e-9 * z.abs().max(1.0));
        }
    }

    #[test]
    fn rotate_determinism_golden_hash() {
        let out = rotate(
            &config(),
            RotateIn {
                geometry: point(1.0, 2.0, 3.0),
                angle: 0.0,
                plane: Plane::world_xy(),
            },
        );
        assert_eq!(
            expect_point_hash(&out),
            "1a6f8073cd8ceb247b753adbb96e270c282cc660b09bb99c4719b64d687b1ca2"
        );
    }
}
