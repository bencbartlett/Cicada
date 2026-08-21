//! The `pipe` node (v0.1 item 3 WP-C).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Curve, Solid};
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`pipe`].
#[derive(Ports, Clone, Debug)]
pub struct PipeIn {
    /// The rail — a line, polyline or circle, open or closed.
    pub rail: Curve,
    /// The pipe's radius.
    #[port(dimension = length)]
    pub radius: f64,
}

/// Pipe — a circular section of `radius`, normal to the rail at its start,
/// swept along the rail into a B-rep solid: a straight rail gives a
/// cylinder, a circle a torus, a polyline a mitred run of cylinders.
///
/// # Returns
///
/// The pipe solid.
///
/// # Panics
///
/// Panics when the radius is not above tolerance, the rail has no length
/// at tolerance, or the kernel refuses (a radius the rail's corners cannot
/// carry, a pipe that does not close into one solid).
///
/// # Examples
///
/// ```cic
/// start = construct_point()
/// end = construct_point(z=12.0)
/// rail = line(a=start, b=end)
/// tube = pipe(rail=rail, radius=1.5)
/// ```
#[node(
    category = "Surface & solid",
    tier = "1",
    version = 1,
    gh = "Pipe",
    uses_tolerance
)]
#[must_use]
pub fn pipe(config: &ProjectConfig, input: PipeIn) -> Solid {
    red(cicada_geom::solid::pipe(
        &input.rail,
        input.radius,
        config.tol(),
    ))
}

// No committed golden for `pipe` (sin/cos-fed section and the sweep's
// approximated surfaces, `support.rs`): run-to-run identity + analytic
// volumes instead.
#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use cicada_core::geometry::{Circle, Line};
    use cicada_core::spatial::{Plane, Point};
    use cicada_geom::tol;

    use super::*;
    use crate::solids::support::{bounds_of, close_rel, config, volume_of, with_kernel};

    fn straight(length: f64) -> Curve {
        Curve::Line(Line {
            a: Point::origin(),
            b: Point::new(0.0, 0.0, length),
        })
    }

    #[test]
    fn pipe_table_cases() {
        // A straight rail: a cylinder, standing exactly on the rail.
        let Some(tube) = with_kernel(|| {
            pipe(
                &config(),
                PipeIn {
                    rail: straight(5.0),
                    radius: 0.5,
                },
            )
        }) else {
            return;
        };
        assert!(close_rel(volume_of(&tube), PI * 0.25 * 5.0, 1e-6));
        let (min, max) = bounds_of(&tube);
        assert!(
            tol::coincident(min, Point::new(-0.5, -0.5, 0.0), 1e-6),
            "{min:?}"
        );
        assert!(
            tol::coincident(max, Point::new(0.5, 0.5, 5.0), 1e-6),
            "{max:?}"
        );
        // A circle rail: a torus, V = 2π² R r².
        let torus = pipe(
            &config(),
            PipeIn {
                rail: Curve::Circle(Circle {
                    plane: Plane::world_xy(),
                    radius: 3.0,
                }),
                radius: 0.5,
            },
        );
        assert!(close_rel(
            volume_of(&torus),
            2.0 * PI * PI * 3.0 * 0.25,
            1e-3
        ));
    }

    #[test]
    #[should_panic(expected = "radius = 0 is out of range")]
    fn pipe_with_zero_radius_is_red() {
        let _ = pipe(
            &config(),
            PipeIn {
                rail: straight(5.0),
                radius: 0.0,
            },
        );
    }

    proptest::proptest! {
        // Straight pipes: volume = π r² L.
        #[test]
        fn property_pipe_cylinders(r in 0.05f64..2.0, length in 0.2f64..20.0) {
            if cicada_geom::solid::kernel_available() {
                let out = pipe(
                    &config(),
                    PipeIn {
                        rail: straight(length),
                        radius: r,
                    },
                );
                proptest::prop_assert!(close_rel(volume_of(&out), PI * r * r * length, 1e-6));
            }
        }
    }

    #[test]
    fn pipe_determinism_run_to_run() {
        let make = || {
            pipe(
                &config(),
                PipeIn {
                    rail: straight(3.0),
                    radius: 0.75,
                },
            )
        };
        let Some(first) = with_kernel(make) else {
            return;
        };
        assert_eq!(first, make());
    }
}
