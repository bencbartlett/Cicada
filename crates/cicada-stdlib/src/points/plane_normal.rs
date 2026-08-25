//! The `plane_normal` node.

use cicada_core::config::ProjectConfig;
use cicada_core::spatial::{Plane, Point, Vector};
use cicada_geom::tol;
use cicada_macros::{Ports, node};
use glam::DVec3;

/// Inputs for [`plane_normal`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct PlaneNormalIn {
    /// Plane origin.
    pub origin: Point,
    /// The normal (any length; unitized) — the plane's z axis.
    pub z: Vector,
}

/// Plane Normal — a frame from an origin and a normal: x is the world axis
/// least aligned with the normal, projected into the plane (ties go to
/// world x, then y), and y completes the right-handed frame (`y = z × x`).
///
/// Deterministic for a given normal — and, like every rule turning one
/// direction into a frame, not continuous: a normal crossing a tie flips
/// the in-plane axes.
///
/// # Returns
///
/// The right-handed orthonormal frame at `origin` whose normal is `z`;
/// `plane_normal(z=unit_z)` is the world XY plane, `z=unit_x` the YZ plane.
///
/// # Panics
///
/// Panics when `z` has no length at tolerance — a zero normal spans no
/// plane.
///
/// # Examples
///
/// ```cic
/// at = construct_point(x=1.0, y=2.0, z=3.0)
/// tilt = construct_vector(x=0.0, y=3.0, z=4.0)
/// frame = plane_normal(origin=at, z=tilt)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "1",
    version = 1,
    gh = "Plane Normal",
    uses_tolerance
)]
#[must_use]
pub fn plane_normal(config: &ProjectConfig, input: PlaneNormalIn) -> Plane {
    let len = input.z.0.length();
    assert!(
        !tol::near_zero(len, config.tol()),
        "plane_normal: z has length {len}, within tolerance of zero — \
         a zero normal spans no plane"
    );
    let z = input.z.0 / len;
    // The reference axis: the world axis with the smallest |component| of
    // the unit normal — it is the one furthest from parallel, so its
    // rejection from z has length at least sqrt(2/3) and never degenerates.
    // A min over three magnitudes is an ordering, not a tolerance decision
    // (ties resolve x, then y, then z; a tie is a legitimate flip point).
    let magnitudes = z.abs();
    let reference = if magnitudes.x <= magnitudes.y && magnitudes.x <= magnitudes.z {
        DVec3::X
    } else if magnitudes.y <= magnitudes.z {
        DVec3::Y
    } else {
        DVec3::Z
    };
    let x = (reference - z * reference.dot(z)).normalize();
    Plane {
        origin: input.origin,
        x: Vector(x),
        y: Vector(z.cross(x)),
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact axis pass-through is the contract
mod tests {
    use cicada_geom::frame::orthonormal;

    use super::*;
    use crate::points::support::testing::hex;

    fn frame(origin: Point, z: Vector) -> Plane {
        plane_normal(&ProjectConfig::default(), PlaneNormalIn { origin, z })
    }

    #[test]
    fn plane_normal_table() {
        // The world normals give the world planes (exact arithmetic: unit
        // components, zeros, a 2/2 unitize).
        assert_eq!(
            frame(Point::origin(), Vector::new(0.0, 0.0, 1.0)),
            Plane::world_xy()
        );
        assert_eq!(
            frame(Point::origin(), Vector::new(0.0, 0.0, 2.0)),
            Plane::world_xy()
        );
        assert_eq!(
            frame(Point::origin(), Vector::new(1.0, 0.0, 0.0)),
            Plane::world_yz()
        );
        // Normal +y: x stays world x, y = z × x = -world z (the plane with
        // normal +y; the world XZ plane's normal is -y).
        let up_y = frame(Point::new(1.0, 2.0, 3.0), Vector::new(0.0, 5.0, 0.0));
        assert_eq!(up_y.origin, Point::new(1.0, 2.0, 3.0));
        assert_eq!(up_y.x, Vector::new(1.0, 0.0, 0.0));
        assert_eq!(up_y.y, Vector::new(0.0, 0.0, -1.0));
        // A 3-4-5 normal in the xy plane: the reference is world z (its
        // component is the smallest), so x = world z and y = z × x.
        let tilted = frame(Point::origin(), Vector::new(3.0, 4.0, 0.0));
        assert_eq!(tilted.x, Vector::new(0.0, 0.0, 1.0));
        assert_eq!(tilted.y, Vector::new(0.8, -0.6, 0.0));
        // A negated normal keeps the x axis and flips y.
        let down = frame(Point::origin(), Vector::new(0.0, 0.0, -1.0));
        assert_eq!(down.x, Vector::new(1.0, 0.0, 0.0));
        assert_eq!(down.y, Vector::new(0.0, -1.0, 0.0));
    }

    #[test]
    #[should_panic(expected = "z has length 0")]
    fn plane_normal_zero_normal_is_red() {
        let _ = frame(Point::origin(), Vector::new(0.0, 0.0, 0.0));
    }

    proptest::proptest! {
        // Any non-degenerate normal gives a right-handed orthonormal frame
        // whose derived normal x × y is the unitized input, and the frame
        // passes the geometry module's own orthonormal check unchanged.
        #[test]
        fn property_plane_normal_is_orthonormal_with_that_normal(
            ox in -10.0..10.0_f64, oy in -10.0..10.0_f64, oz in -10.0..10.0_f64,
            zx in -10.0..10.0_f64, zy in -10.0..10.0_f64, zz in -10.0..10.0_f64,
        ) {
            let normal = Vector::new(zx, zy, zz);
            proptest::prop_assume!(normal.0.length() > 1e-3);
            let plane = frame(Point::new(ox, oy, oz), normal);
            let (x, y) = (plane.x.0, plane.y.0);
            proptest::prop_assert!(tol::close(x.length(), 1.0, 1e-9));
            proptest::prop_assert!(tol::close(y.length(), 1.0, 1e-9));
            proptest::prop_assert!(tol::near_zero(x.dot(y), 1e-9));
            proptest::prop_assert!(tol::near_zero((x.cross(y) - normal.0.normalize()).length(), 1e-9));
            proptest::prop_assert_eq!(plane.origin, Point::new(ox, oy, oz));
            let checked = orthonormal(&plane, 1e-6).unwrap();
            proptest::prop_assert!(tol::near_zero((checked.x - x).length(), 1e-9));
            proptest::prop_assert!(tol::near_zero((checked.y - y).length(), 1e-9));
        }
    }

    // Golden hash: a 3-4-5 normal — the unitize and the cross product are
    // exact arithmetic, so the frame is bit-exact (blessed via run-once).
    #[test]
    fn plane_normal_determinism_golden_hash() {
        assert_eq!(
            hex(frame(
                Point::new(1.0, -2.0, 0.5),
                Vector::new(3.0, 4.0, 0.0)
            )),
            "0af90e58e6462549b2e49c91cdaba48fb9bab3017f8aba4d2c821d769507577b"
        );
    }
}
