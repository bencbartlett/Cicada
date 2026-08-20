//! The `construct_plane` node.

use cicada_core::config::ProjectConfig;
use cicada_core::spatial::{Plane, Point, Vector};
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`construct_plane`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct ConstructPlaneIn {
    /// Plane origin.
    #[port(default = Point::origin(), default_doc = "origin")]
    pub origin: Point,
    /// The x axis (any length; unitized).
    #[port(default = Vector::new(1.0, 0.0, 0.0), default_doc = "unit_x")]
    pub x: Vector,
    /// The y direction (any length; its component along x is removed, then
    /// unitized — Gram–Schmidt).
    #[port(default = Vector::new(0.0, 1.0, 0.0), default_doc = "unit_y")]
    pub y: Vector,
}

/// Construct Plane — a frame from an origin and two axes: x unitized, y
/// orthonormalized against x (Gram–Schmidt), so the stored plane is a
/// right-handed orthonormal frame with normal x × y.
///
/// # Panics
///
/// Panics when `x` has no length at tolerance, or `y` is parallel to `x`
/// at tolerance (its component off the x line has no length) — red with
/// the measured length, never a NaN frame.
///
/// # Examples
///
/// ```cic
/// at = construct_point(x=1.0, y=2.0, z=3.0)
/// along = unit_x(factor=3.0)
/// up = unit_z(factor=2.0)
/// frame = construct_plane(origin=at, x=along, y=up)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "S",
    version = 1,
    gh = "Construct Plane",
    uses_tolerance
)]
#[must_use]
pub fn construct_plane(config: &ProjectConfig, input: ConstructPlaneIn) -> Plane {
    let frame = red(cicada_geom::frame::orthonormal(
        &Plane {
            origin: input.origin,
            x: input.x,
            y: input.y,
        },
        config.tol(),
    ));
    Plane {
        origin: frame.origin,
        x: Vector(frame.x),
        y: Vector(frame.y),
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact coordinate pass-through is the contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    #[test]
    fn construct_plane_table() {
        let config = ProjectConfig::default();
        // Already orthonormal: passes through exactly (unit axes, exact
        // arithmetic).
        let plane = construct_plane(
            &config,
            ConstructPlaneIn {
                origin: Point::new(1.0, 2.0, 3.0),
                x: Vector::new(1.0, 0.0, 0.0),
                y: Vector::new(0.0, 1.0, 0.0),
            },
        );
        assert_eq!(plane.origin, Point::new(1.0, 2.0, 3.0));
        assert_eq!(plane.x, Vector::new(1.0, 0.0, 0.0));
        assert_eq!(plane.y, Vector::new(0.0, 1.0, 0.0));
        // Scaled axes unitize exactly here: (3,4,0) → (0.6,0.8,0), and the
        // already-orthogonal (0,0,2) → (0,0,1).
        let skewed = construct_plane(
            &config,
            ConstructPlaneIn {
                origin: Point::origin(),
                x: Vector::new(3.0, 4.0, 0.0),
                y: Vector::new(0.0, 0.0, 2.0),
            },
        );
        assert_eq!(skewed.x, Vector::new(0.6, 0.8, 0.0));
        assert_eq!(skewed.y, Vector::new(0.0, 0.0, 1.0));
        let tilted = construct_plane(
            &config,
            ConstructPlaneIn {
                origin: Point::origin(),
                x: Vector::new(2.0, 0.0, 0.0),
                y: Vector::new(5.0, 0.0, -0.25),
            },
        );
        // Gram–Schmidt: y = (5,0,-0.25) loses its x component, leaving
        // (0,0,-0.25) → (0,0,-1) — exact arithmetic.
        assert_eq!(tilted.x, Vector::new(1.0, 0.0, 0.0));
        assert_eq!(tilted.y, Vector::new(0.0, 0.0, -1.0));
        // The defaults are the world XY frame.
        let world = construct_plane(
            &config,
            ConstructPlaneIn {
                origin: Point::origin(),
                x: Vector::new(1.0, 0.0, 0.0),
                y: Vector::new(0.0, 1.0, 0.0),
            },
        );
        assert_eq!(world, Plane::world_xy());
    }

    #[test]
    #[should_panic(expected = "x axis has length 0")]
    fn construct_plane_zero_x_is_red() {
        let _ = construct_plane(
            &ProjectConfig::default(),
            ConstructPlaneIn {
                origin: Point::origin(),
                x: Vector::new(0.0, 0.0, 0.0),
                y: Vector::new(0.0, 1.0, 0.0),
            },
        );
    }

    #[test]
    #[should_panic(expected = "y axis is parallel to x")]
    fn construct_plane_parallel_y_is_red() {
        let _ = construct_plane(
            &ProjectConfig::default(),
            ConstructPlaneIn {
                origin: Point::origin(),
                x: Vector::new(1.0, 1.0, 0.0),
                y: Vector::new(-2.0, -2.0, 0.0),
            },
        );
    }

    proptest::proptest! {
        // Any non-degenerate axes give a right-handed orthonormal frame
        // whose x is the unitized input x and whose plane contains the
        // input y (z ⟂ y): |x| = |y| = 1, x·y = 0, (x×y)·y_in = 0.
        #[test]
        fn property_construct_plane_is_orthonormal(
            xx in -10.0..10.0_f64, xy in -10.0..10.0_f64, xz in -10.0..10.0_f64,
            yx in -10.0..10.0_f64, yy in -10.0..10.0_f64, yz in -10.0..10.0_f64,
        ) {
            let x_in = Vector::new(xx, xy, xz).0;
            let y_in = Vector::new(yx, yy, yz).0;
            // Stay clear of the refusal band (length / parallel within 1e-6).
            proptest::prop_assume!(x_in.length() > 1e-3);
            proptest::prop_assume!(x_in.cross(y_in).length() > 1e-3 * x_in.length());
            let plane = construct_plane(
                &ProjectConfig::default(),
                ConstructPlaneIn {
                    origin: Point::origin(),
                    x: Vector(x_in),
                    y: Vector(y_in),
                },
            );
            let (x, y) = (plane.x.0, plane.y.0);
            proptest::prop_assert!((x.length() - 1.0).abs() < 1e-9);
            proptest::prop_assert!((y.length() - 1.0).abs() < 1e-9);
            proptest::prop_assert!(x.dot(y).abs() < 1e-9);
            proptest::prop_assert!((x - x_in.normalize()).length() < 1e-9);
            proptest::prop_assert!(x.cross(y).dot(y_in).abs() < 1e-9 * y_in.length());
            // Right-handed: y has a positive component along the input y.
            proptest::prop_assert!(y.dot(y_in) > 0.0);
        }
    }

    #[test]
    fn construct_plane_determinism_golden_hash() {
        // Exact arithmetic inputs (3-4-5 triangles, no transcendental).
        let plane = construct_plane(
            &ProjectConfig::default(),
            ConstructPlaneIn {
                origin: Point::new(1.0, -2.0, 0.5),
                x: Vector::new(3.0, 4.0, 0.0),
                y: Vector::new(0.0, 0.0, -2.0),
            },
        );
        assert_eq!(
            HashedValue::new(ValueData::Plane(plane))
                .unwrap()
                .hash()
                .to_hex(),
            "e07712f0f01191672c675a04261f321a7ae29c7702b4d0b9909d8080739b8fbb"
        );
    }
}
