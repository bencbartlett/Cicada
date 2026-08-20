//! The `rectangle` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve, Rectangle};
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Vector};
use cicada_geom::frame::orthonormal;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`rectangle`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct RectangleIn {
    /// The rectangle's frame.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// Extent along the plane's x axis.
    pub x: Domain,
    /// Extent along the plane's y axis.
    pub y: Domain,
}

/// Rectangle — an analytic rectangle in a plane, always closed. The frame
/// is orthonormalized at construction. (The rounded-`corner` parameter
/// arrives with compound curves, v0.1.)
///
/// # Panics
///
/// Panics when either extent is empty at tolerance or the plane is
/// degenerate.
#[node(category = "Curve", tier = "S", version = 1, uses_tolerance)]
#[must_use]
pub fn rectangle(config: &ProjectConfig, input: RectangleIn) -> Closed<Curve> {
    for (name, domain) in [("x", &input.x), ("y", &input.y)] {
        assert!(
            !cicada_geom::tol::close(domain.start, domain.end, config.tol()),
            "rectangle: {name} extent {}..{} is empty at tolerance {}",
            domain.start,
            domain.end,
            config.tol()
        );
    }
    let frame = red(orthonormal(&input.plane, config.tol()));
    Closed(Curve::Rectangle(Rectangle {
        plane: Plane {
            origin: frame.origin,
            x: Vector(frame.x),
            y: Vector(frame.y),
        },
        x: input.x,
        y: input.y,
    }))
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // constructor pass-through is exact by contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;
    use cicada_core::spatial::Point;

    use crate::curves::support::config;

    #[test]
    fn rectangle_normalizes_frame_and_keeps_extents() {
        let skewed = Plane {
            origin: Point::new(1.0, 2.0, 3.0),
            x: Vector::new(2.0, 0.0, 0.0),
            y: Vector::new(1.0, 4.0, 0.0),
        };
        let Closed(Curve::Rectangle(r)) = rectangle(
            &config(),
            RectangleIn {
                plane: skewed,
                x: Domain::new(0.0, 3.0),
                y: Domain::new(-1.0, 2.0),
            },
        ) else {
            panic!("rectangle variant")
        };
        assert_eq!(r.plane.x, Vector::new(1.0, 0.0, 0.0));
        assert_eq!(r.plane.y, Vector::new(0.0, 1.0, 0.0));
        assert_eq!(r.x, Domain::new(0.0, 3.0));
        assert_eq!(r.y, Domain::new(-1.0, 2.0));
    }

    #[test]
    #[should_panic(expected = "extent")]
    fn rectangle_empty_extent_is_red() {
        let _ = rectangle(
            &config(),
            RectangleIn {
                plane: Plane::world_xy(),
                x: Domain::new(1.0, 1.0),
                y: Domain::new(0.0, 2.0),
            },
        );
    }

    proptest::proptest! {
        // Rectangle keeps its extents exactly for any non-empty domains.
        #[test]
        fn property_rectangle_keeps_domains(
            x0 in -100.0..100.0_f64, dx in 0.01..50.0_f64,
            y0 in -100.0..100.0_f64, dy in 0.01..50.0_f64,
        ) {
            let Closed(Curve::Rectangle(r)) = rectangle(
                &config(),
                RectangleIn {
                    plane: Plane::world_xy(),
                    x: Domain::new(x0, x0 + dx),
                    y: Domain::new(y0, y0 + dy),
                },
            ) else {
                panic!("rectangle variant")
            };
            proptest::prop_assert_eq!(r.x, Domain::new(x0, x0 + dx));
            proptest::prop_assert_eq!(r.y, Domain::new(y0, y0 + dy));
        }
    }

    #[test]
    fn rectangle_determinism_golden_hash() {
        let Closed(r) = rectangle(
            &config(),
            RectangleIn {
                plane: Plane::world_xy(),
                x: Domain::new(0.0, 3.0),
                y: Domain::new(-1.0, 2.0),
            },
        );
        assert_eq!(
            HashedValue::new(ValueData::Curve(r))
                .unwrap()
                .hash()
                .to_hex(),
            "fe8f0016efa7cd5bc86f6f28a0f7e03158e5da9f67235740a7b65de4e773592c"
        );
    }
}
