//! The `rotate_vector` node.

use cicada_core::config::ProjectConfig;
use cicada_core::spatial::Vector;
use cicada_geom::tol;
use cicada_macros::{Ports, node};

/// Inputs for [`rotate_vector`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct RotateVectorIn {
    /// The vector to rotate.
    pub vector: Vector,
    /// Rotation angle in radians (right-handed about `axis`).
    #[port(dimension = angle)]
    pub angle: f64,
    /// The axis to rotate about (any length; its direction is what counts).
    pub axis: Vector,
}

/// Rotate Vector — rotate a vector about an axis by an angle (right-handed:
/// counter-clockwise when the axis points at you).
///
/// # Returns
///
/// The vector turned by `angle` about `axis`; its length and its component
/// along the axis are unchanged.
///
/// # Panics
///
/// Panics when `axis` has no length at tolerance — a zero axis has no
/// direction to rotate about.
///
/// # Examples
///
/// ```cic
/// arm = unit_x(factor=3.0)
/// up = unit_z(factor=1.0)
/// turned = rotate_vector(vector=arm, angle=1.5707963267948966, axis=up)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "1",
    version = 1,
    gh = "Rotate",
    uses_tolerance
)]
#[must_use]
pub fn rotate_vector(config: &ProjectConfig, input: RotateVectorIn) -> Vector {
    let len = input.axis.0.length();
    assert!(
        !tol::near_zero(len, config.tol()),
        "rotate_vector: axis has length {len}, within tolerance of zero — \
         a zero axis has no direction to rotate about"
    );
    // The same rotation matrix the `rotate` node builds about a frame's z
    // (`Similarity::rotation`), so a vector and the geometry it came from
    // turn alike.
    Vector(glam::DMat3::from_axis_angle(input.axis.0 / len, input.angle) * input.vector.0)
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, PI};

    use super::*;
    use crate::points::support::testing::hex;

    fn turn(vector: Vector, angle: f64, axis: Vector) -> Vector {
        rotate_vector(
            &ProjectConfig::default(),
            RotateVectorIn {
                vector,
                angle,
                axis,
            },
        )
    }

    fn close(a: Vector, b: Vector) -> bool {
        tol::near_zero((a.0 - b.0).length(), 1e-12)
    }

    #[test]
    fn rotate_vector_table() {
        let (x, y, z) = (
            Vector::new(1.0, 0.0, 0.0),
            Vector::new(0.0, 1.0, 0.0),
            Vector::new(0.0, 0.0, 1.0),
        );
        // A quarter turn about z carries x to y (right-handed), and y to -x.
        assert!(close(turn(x, FRAC_PI_2, z), y));
        assert!(close(turn(y, FRAC_PI_2, z), Vector::new(-1.0, 0.0, 0.0)));
        // The axis's length is irrelevant; a negative angle turns back.
        assert!(close(turn(x, FRAC_PI_2, Vector::new(0.0, 0.0, 7.0)), y));
        assert!(close(turn(x, -FRAC_PI_2, z), Vector::new(0.0, -1.0, 0.0)));
        // A half turn negates the part across the axis; a vector along the
        // axis never moves; angle 0 is the identity.
        assert!(close(
            turn(Vector::new(3.0, 4.0, 5.0), PI, z),
            Vector::new(-3.0, -4.0, 5.0)
        ));
        assert!(close(
            turn(Vector::new(0.0, 0.0, 2.5), 1.234, z),
            Vector::new(0.0, 0.0, 2.5)
        ));
        assert!(close(
            turn(Vector::new(3.0, 4.0, 5.0), 0.0, y),
            Vector::new(3.0, 4.0, 5.0)
        ));
        // Right-handed about x: y goes to z.
        assert!(close(turn(y, FRAC_PI_2, x), z));
    }

    #[test]
    #[should_panic(expected = "axis has length 0")]
    fn rotate_vector_zero_axis_is_red() {
        let _ = turn(Vector::new(1.0, 0.0, 0.0), 1.0, Vector::new(0.0, 0.0, 0.0));
    }

    proptest::proptest! {
        // A rotation is an isometry that fixes its axis: the vector's length
        // and its component along the axis are preserved, and turning back
        // by the negated angle restores the vector.
        #[test]
        fn property_rotate_vector_preserves_length_and_axial_part(
            vx in -100.0..100.0_f64, vy in -100.0..100.0_f64, vz in -100.0..100.0_f64,
            ax in -10.0..10.0_f64, ay in -10.0..10.0_f64, az in -10.0..10.0_f64,
            angle in -10.0..10.0_f64,
        ) {
            let v = Vector::new(vx, vy, vz);
            let axis = Vector::new(ax, ay, az);
            proptest::prop_assume!(axis.0.length() > 1e-3);
            let out = turn(v, angle, axis);
            let unit_axis = axis.0.normalize();
            proptest::prop_assert!(tol::close(out.0.length(), v.0.length(), 1e-9 * (1.0 + v.0.length())));
            proptest::prop_assert!(tol::close(out.0.dot(unit_axis), v.0.dot(unit_axis), 1e-9 * (1.0 + v.0.length())));
            let back = turn(out, -angle, axis);
            proptest::prop_assert!(tol::near_zero((back.0 - v.0).length(), 1e-9 * (1.0 + v.0.length())));
        }
    }

    // Golden hash: the transcendental-free case — angle 0 (sin 0 = 0 and
    // cos 0 = 1 exactly in every libm, so the matrix is the identity to the
    // bit and the output is the input) — blessed via run-once; a general
    // turn is sin/cos-fed and is asserted for run-to-run identity only.
    #[test]
    fn rotate_vector_determinism_golden_hash() {
        let rest = turn(
            Vector::new(1.5, -2.0, 0.25),
            0.0,
            Vector::new(0.0, 0.0, 3.0),
        );
        assert_eq!(
            hex(rest),
            "ab992585e08e454ce8c0a8ba01021d5911170e8b52afa2b8ac6bedb5983f4ab9"
        );
        let quarter = || {
            turn(
                Vector::new(1.5, -2.0, 0.25),
                FRAC_PI_2,
                Vector::new(1.0, 1.0, 0.0),
            )
        };
        assert_eq!(hex(quarter()), hex(quarter()));
    }
}
