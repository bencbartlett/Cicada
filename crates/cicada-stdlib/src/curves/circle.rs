//! The `circle` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Circle, Closed, Curve};
use cicada_core::spatial::{Plane, Vector};
use cicada_geom::frame::orthonormal;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`circle`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct CircleIn {
    /// The circle's frame; origin = center.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// The radius.
    #[port(dimension = length)]
    pub radius: f64,
}

/// Circle — an analytic circle in a plane. The stored frame is
/// orthonormalized at construction, so downstream evaluation is exact.
///
/// # Returns
///
/// The closed circle, centered at the plane's origin.
///
/// # Panics
///
/// Panics when the radius is not above tolerance or the plane's axes are
/// degenerate (zero-length or parallel).
///
/// # Examples
///
/// ```cic
/// ring = circle(radius=2.5)
/// ```
#[node(
    category = "Curve",
    tier = "S",
    version = 1,
    gh = "Circle",
    uses_tolerance
)]
#[must_use]
pub fn circle(config: &ProjectConfig, input: CircleIn) -> Closed<Curve> {
    assert!(
        input.radius > config.tol(),
        "circle: radius {} is not above tolerance {}",
        input.radius,
        config.tol()
    );
    let frame = red(orthonormal(&input.plane, config.tol()));
    Closed(Curve::Circle(Circle {
        plane: Plane {
            origin: frame.origin,
            x: Vector(frame.x),
            y: Vector(frame.y),
        },
        radius: input.radius,
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
    fn circle_normalizes_its_frame() {
        let skewed = Plane {
            origin: Point::new(1.0, 2.0, 3.0),
            x: Vector::new(3.0, 0.0, 0.0),
            y: Vector::new(1.0, 2.0, 0.0),
        };
        let Closed(Curve::Circle(c)) = circle(
            &config(),
            CircleIn {
                plane: skewed,
                radius: 2.0,
            },
        ) else {
            panic!("circle variant")
        };
        assert_eq!(c.plane.x, Vector::new(1.0, 0.0, 0.0));
        assert_eq!(c.plane.y, Vector::new(0.0, 1.0, 0.0));
        assert_eq!(c.radius, 2.0);
    }

    #[test]
    #[should_panic(expected = "radius")]
    fn circle_zero_radius_is_red() {
        let _ = circle(
            &config(),
            CircleIn {
                plane: Plane::world_xy(),
                radius: 0.0,
            },
        );
    }

    proptest::proptest! {
        // Any non-degenerate skewed frame orthonormalizes: unit orthogonal
        // axes; origin and radius pass through exactly.
        #[test]
        fn property_circle_orthonormalizes_any_frame(
            ox in -1.0e3..1.0e3_f64, oy in -1.0e3..1.0e3_f64,
            xa in 0.5..10.0_f64, xb in -5.0..5.0_f64,
            yc in -5.0..5.0_f64, yd in 0.5..10.0_f64,
            radius in 0.001..1.0e3_f64,
        ) {
            // Keep the axes clearly non-parallel so the frame is far from
            // degenerate (near-parallel Gram-Schmidt loses precision).
            proptest::prop_assume!((xa * yd - xb * yc).abs() > 0.5);
            let plane = Plane {
                origin: Point::new(ox, oy, 0.0),
                x: Vector::new(xa, xb, 0.0),
                y: Vector::new(yc, yd, 0.0),
            };
            let Closed(Curve::Circle(c)) = circle(&config(), CircleIn { plane, radius })
            else {
                panic!("circle variant")
            };
            proptest::prop_assert!((c.plane.x.0.length() - 1.0).abs() <= 1e-12);
            proptest::prop_assert!((c.plane.y.0.length() - 1.0).abs() <= 1e-12);
            proptest::prop_assert!(c.plane.x.0.dot(c.plane.y.0).abs() <= 1e-9);
            proptest::prop_assert_eq!(c.plane.origin, plane.origin);
            proptest::prop_assert_eq!(c.radius, radius);
        }
    }

    #[test]
    fn circle_determinism_golden_hash() {
        let Closed(c) = circle(
            &config(),
            CircleIn {
                plane: Plane::world_xy(),
                radius: 2.5,
            },
        );
        assert_eq!(
            HashedValue::new(ValueData::Curve(c))
                .unwrap()
                .hash()
                .to_hex(),
            "49e447cfea5876a978743c331b265d0c8c1824a052e96e9fc8386023e06dffd3"
        );
    }
}
