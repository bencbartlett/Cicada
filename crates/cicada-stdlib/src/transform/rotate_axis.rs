//! The `rotate_axis` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Curve, Transformable};
use cicada_geom::tol;
use cicada_geom::transform::Similarity;
use cicada_macros::{Ports, node};

/// Inputs for [`rotate_axis`].
#[derive(Ports, Clone, Debug)]
pub struct RotateAxisIn {
    /// The geometry to rotate.
    pub geometry: Transformable,
    /// Rotation angle in radians, right-handed about the axis: counter-
    /// clockwise when the axis points at you.
    #[port(dimension = angle)]
    pub angle: f64,
    /// The axis: a `line` (the `line` node) — the rotation is about the
    /// line through its start along its direction; any other curve kind
    /// is red.
    pub axis: Curve,
}

/// Rotate Axis — rotate geometry about an arbitrary axis line.
///
/// The same rotation `rotate` applies about a plane's normal, about a
/// line of the user's choosing instead: the line's start is a point of the
/// axis and its direction the axis direction (any length). A Solid moves
/// through the kernel like every similarity.
///
/// # Returns
///
/// The geometry rotated by `angle` about the axis line.
///
/// # Panics
///
/// Panics when `axis` is not a `line`, or has no length at tolerance (a
/// zero-length axis has no direction), or for a `Solid` the OCCT kernel
/// refuses to transform (a `Solid` moves through the kernel — its B-rep
/// geometry is rewritten, never a mesh in disguise).
///
/// # Examples
///
/// ```cic
/// foot = construct_point(x=0.0, y=0.0, z=0.0)
/// tip = construct_point(x=1.0, y=1.0, z=0.0)
/// hinge = line(a=foot, b=tip)
/// corner = construct_point(x=2.0, y=0.0, z=0.0)
/// lifted = rotate_axis(geometry=corner, angle=1.5707963267948966, axis=hinge)
/// ```
#[node(
    category = "Transform",
    tier = "1",
    version = 1,
    gh = "Rotate Axis",
    uses_tolerance
)]
#[must_use]
pub fn rotate_axis(config: &ProjectConfig, input: RotateAxisIn) -> Transformable {
    let Curve::Line(axis) = &input.axis else {
        panic!(
            "rotate_axis: axis must be a `line`, got a {} — the rotation needs a start point \
             and a direction",
            input.axis.variant_name()
        );
    };
    let direction = axis.b.0 - axis.a.0;
    let len = direction.length();
    assert!(
        !tol::near_zero(len, config.tol()),
        "rotate_axis: axis has length {len}, within tolerance of zero — a zero-length axis has \
         no direction to rotate about"
    );
    Similarity::rotation_about_axis(axis.a, direction / len, input.angle).apply(&input.geometry)
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, PI};

    use cicada_core::geometry::{Line, Polyline};
    use cicada_core::spatial::{Plane, Point, Vector};
    use cicada_geom::frame::orthonormal;

    use super::*;
    use crate::transform::rotate::{RotateIn, rotate};
    use crate::transform::support::{config, expect_point, expect_point_hash, point};

    fn axis(a: Point, b: Point) -> Curve {
        Curve::Line(Line { a, b })
    }

    #[test]
    fn rotate_axis_table() {
        // A quarter turn about the world z line through (1, 0, 0): the
        // point at (2, 0, 0) — one unit out along x — lands one unit out
        // along y from the axis.
        let out = rotate_axis(
            &config(),
            RotateAxisIn {
                geometry: point(2.0, 0.0, 0.0),
                angle: FRAC_PI_2,
                axis: axis(Point::new(1.0, 0.0, 0.0), Point::new(1.0, 0.0, 5.0)),
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(1.0, 1.0, 0.0),
            1e-12
        ));
        // A half turn about the x axis flips y and z; the axis's length is
        // immaterial (a 0.25-long segment rotates like a long one).
        let out = rotate_axis(
            &config(),
            RotateAxisIn {
                geometry: point(3.0, 1.0, 2.0),
                angle: PI,
                axis: axis(Point::origin(), Point::new(0.25, 0.0, 0.0)),
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(3.0, -1.0, -2.0),
            1e-12
        ));
        // A point ON the axis stays put.
        let out = rotate_axis(
            &config(),
            RotateAxisIn {
                geometry: point(1.0, 1.0, 1.0),
                angle: 2.0,
                axis: axis(Point::origin(), Point::new(2.0, 2.0, 2.0)),
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(1.0, 1.0, 1.0),
            1e-12
        ));
    }

    #[test]
    #[should_panic(expected = "axis must be a `line`, got a Polyline")]
    fn rotate_axis_refuses_a_non_line_axis() {
        let _ = rotate_axis(
            &config(),
            RotateAxisIn {
                geometry: point(1.0, 0.0, 0.0),
                angle: 1.0,
                axis: Curve::Polyline(Polyline {
                    vertices: vec![Point::origin(), Point::new(1.0, 0.0, 0.0)],
                    closed: false,
                }),
            },
        );
    }

    #[test]
    #[should_panic(expected = "within tolerance of zero")]
    fn rotate_axis_refuses_a_zero_length_axis() {
        let _ = rotate_axis(
            &config(),
            RotateAxisIn {
                geometry: point(1.0, 0.0, 0.0),
                angle: 1.0,
                axis: axis(Point::new(1.0, 1.0, 1.0), Point::new(1.0, 1.0, 1.0)),
            },
        );
    }

    proptest::proptest! {
        // About a frame's normal through its origin, `rotate_axis` over the
        // line (origin, origin + z) IS `rotate` over the frame — the two
        // nodes share one rotation (the doc's claim), for any angle and any
        // point; and the distance to the axis line is preserved.
        #[test]
        fn property_rotate_axis_agrees_with_rotate_and_keeps_the_axis_distance(
            x in -100.0..100.0_f64, y in -100.0..100.0_f64, z in -100.0..100.0_f64,
            angle in -10.0..10.0_f64,
            ox in -5.0..5.0_f64, oy in -5.0..5.0_f64,
            length in 0.5..20.0_f64,
        ) {
            let plane = Plane {
                origin: Point::new(ox, oy, 1.0),
                x: Vector::new(1.0, 1.0, 0.0),
                y: Vector::new(-1.0, 1.0, 1.0),
            };
            let frame = orthonormal(&plane, 1e-9).unwrap();
            let tip = Point(frame.origin.0 + frame.z * length);
            let by_axis = rotate_axis(
                &config(),
                RotateAxisIn {
                    geometry: point(x, y, z),
                    angle,
                    axis: axis(frame.origin, tip),
                },
            );
            let by_frame = rotate(
                &config(),
                RotateIn {
                    geometry: point(x, y, z),
                    angle,
                    plane,
                },
            );
            let (a, b) = (expect_point(&by_axis), expect_point(&by_frame));
            proptest::prop_assert!(tol::coincident(a, b, 1e-9));
            let before = (Point::new(x, y, z).0 - frame.origin.0).cross(frame.z).length();
            let after = (a.0 - frame.origin.0).cross(frame.z).length();
            proptest::prop_assert!((before - after).abs() <= 1e-9 * before.max(1.0));
        }
    }

    #[test]
    fn rotate_axis_determinism_golden_hash() {
        // A zero-angle turn about an oblique axis: sin 0 = 0 and cos 0 = 1
        // exactly, so the matrix is the identity to the bit and the point
        // hashes as itself — transcendental-free (support.rs). A quarter
        // turn is held to run-to-run identity below.
        let out = rotate_axis(
            &config(),
            RotateAxisIn {
                geometry: point(1.0, 2.0, 3.0),
                angle: 0.0,
                axis: axis(Point::new(0.5, -1.0, 2.0), Point::new(3.5, 3.0, -4.0)),
            },
        );
        assert_eq!(
            expect_point_hash(&out),
            "1a6f8073cd8ceb247b753adbb96e270c282cc660b09bb99c4719b64d687b1ca2"
        );
        let turn = || {
            expect_point_hash(&rotate_axis(
                &config(),
                RotateAxisIn {
                    geometry: point(1.0, 2.0, 3.0),
                    angle: FRAC_PI_2,
                    axis: axis(Point::new(0.5, -1.0, 2.0), Point::new(3.5, 3.0, -4.0)),
                },
            ))
        };
        assert_eq!(turn(), turn());
    }
}
