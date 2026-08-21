//! The `revolve` node (v0.1 item 3 WP-C).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve, Solid};
use cicada_core::scalar::Domain;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`revolve`].
#[derive(Ports, Clone, Debug)]
pub struct RevolveIn {
    /// The closed, planar profile to revolve; it may touch the axis but
    /// never cross it.
    pub profile: Closed<Curve>,
    /// The revolution axis: a `Line` curve (the `line` node) lying in the
    /// profile's plane.
    pub axis: Curve,
    /// The sweep in radians, from `start` to `end` about the axis (right-
    /// handed about the line's direction): at most a full turn, either way
    /// round.
    #[port(
        default = Domain::new(0.0, std::f64::consts::TAU),
        default_doc = "full turn",
        dimension = angle
    )]
    pub angle: Domain,
}

/// Revolve — revolve a closed planar profile about a line in its plane into
/// a B-rep solid: a full turn by default (a ring from a square, a disc from
/// a profile touching the axis), a partial sweep through any angle domain.
///
/// # Returns
///
/// The solid of revolution.
///
/// # Panics
///
/// Panics when the profile is degenerate, non-planar or self-intersecting
/// at tolerance, the axis is not a `Line`, has no length, lies off the
/// profile's plane or crosses the profile, the angle domain is empty or
/// spans more than a full turn, or the kernel refuses.
///
/// # Examples
///
/// ```cic
/// xs = [2.0, 3.0, 3.0, 2.0]
/// zs = [0.0, 0.0, 1.0, 1.0]
/// corners = construct_point(x=each(xs), z=each(zs))
/// section = polyline(vertices=corners, closed=True)
/// profile = as_closed(curve=section)
/// bottom = construct_point()
/// top = construct_point(z=1.0)
/// axis = line(a=bottom, b=top)
/// ring = revolve(profile=profile, axis=axis)
/// ```
#[node(
    category = "Surface & solid",
    tier = "1",
    version = 1,
    gh = "Revolution",
    uses_tolerance
)]
#[must_use]
pub fn revolve(config: &ProjectConfig, input: RevolveIn) -> Solid {
    red(cicada_geom::solid::revolve(
        &input.profile.0,
        &input.axis,
        input.angle,
        config.tol(),
        config.tol_angle(),
    ))
}

// No committed golden for `revolve`: a surface of revolution's bytes carry
// sin/cos-fed coordinates for any partial turn and the seam of a full one
// (`support.rs`); run-to-run identity and the analytic volume instead — the
// heap-independence test in `cicada_geom::occt::node_set_tests` covers the
// ring below too.
#[cfg(test)]
mod tests {
    use std::f64::consts::{PI, TAU};

    use cicada_core::geometry::{Line, Polyline};
    use cicada_core::spatial::Point;

    use super::*;
    use crate::solids::support::{bounds_of, close_rel, config, ring, volume_of, with_kernel};

    fn square_profile(inner: f64, outer: f64) -> Closed<Curve> {
        Closed(Curve::Polyline(Polyline {
            vertices: vec![
                Point::new(inner, 0.0, 0.0),
                Point::new(outer, 0.0, 0.0),
                Point::new(outer, 0.0, 1.0),
                Point::new(inner, 0.0, 1.0),
            ],
            closed: true,
        }))
    }

    fn z_axis() -> Curve {
        Curve::Line(Line {
            a: Point::origin(),
            b: Point::new(0.0, 0.0, 1.0),
        })
    }

    #[test]
    fn revolve_table_cases() {
        // A unit square at radius 2..3, full turn: V = 2π · R̄ · A.
        let Some(ring_solid) = with_kernel(|| {
            revolve(
                &config(),
                RevolveIn {
                    profile: square_profile(2.0, 3.0),
                    axis: z_axis(),
                    angle: Domain::new(0.0, TAU),
                },
            )
        }) else {
            return;
        };
        assert!(close_rel(volume_of(&ring_solid), TAU * 2.5, 1e-9));
        // A quarter turn starting at π/2 lands in the second quadrant.
        let quarter = revolve(
            &config(),
            RevolveIn {
                profile: square_profile(2.0, 3.0),
                axis: z_axis(),
                angle: Domain::new(PI / 2.0, PI),
            },
        );
        assert!(close_rel(volume_of(&quarter), TAU * 2.5 / 4.0, 1e-9));
        let (min, max) = bounds_of(&quarter);
        assert!(max.0.x <= 1e-7 && min.0.y >= -1e-7, "{min:?} {max:?}");
        // Touching the axis: a disc.
        let disc = revolve(
            &config(),
            RevolveIn {
                profile: square_profile(0.0, 2.0),
                axis: z_axis(),
                angle: Domain::new(0.0, TAU),
            },
        );
        assert!(close_rel(volume_of(&disc), PI * 4.0, 1e-9));
    }

    #[test]
    #[should_panic(expected = "must be a Line")]
    fn revolve_about_a_non_line_is_red() {
        let _ = revolve(
            &config(),
            RevolveIn {
                profile: square_profile(2.0, 3.0),
                axis: ring(&[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)], 0.0).0,
                angle: Domain::new(0.0, TAU),
            },
        );
    }

    #[test]
    #[should_panic(expected = "one side of its axis")]
    fn revolve_of_a_profile_crossing_the_axis_is_red() {
        let _ = revolve(
            &config(),
            RevolveIn {
                profile: square_profile(-1.0, 2.0),
                axis: z_axis(),
                angle: Domain::new(0.0, TAU),
            },
        );
    }

    proptest::proptest! {
        // Pappus: a rectangle at radius r..r+w revolved through θ has
        // volume θ · (r + w/2) · w · h.
        #[test]
        fn property_revolve_pappus(
            r in 0.0f64..5.0, w in 0.1f64..3.0, h in 0.1f64..3.0,
            theta in 0.1f64..TAU,
        ) {
            if cicada_geom::solid::kernel_available() {
                let profile = Closed(Curve::Polyline(Polyline {
                    vertices: vec![
                        Point::new(r, 0.0, 0.0),
                        Point::new(r + w, 0.0, 0.0),
                        Point::new(r + w, 0.0, h),
                        Point::new(r, 0.0, h),
                    ],
                    closed: true,
                }));
                let out = revolve(
                    &config(),
                    RevolveIn {
                        profile,
                        axis: z_axis(),
                        angle: Domain::new(0.0, theta),
                    },
                );
                let want = theta * (r + w / 2.0) * w * h;
                proptest::prop_assert!(close_rel(volume_of(&out), want, 1e-7), "got {} want {}", volume_of(&out), want);
            }
        }
    }

    #[test]
    fn revolve_determinism_run_to_run() {
        let make = || {
            revolve(
                &config(),
                RevolveIn {
                    profile: square_profile(2.0, 3.0),
                    axis: z_axis(),
                    angle: Domain::new(0.0, TAU),
                },
            )
        };
        let Some(first) = with_kernel(make) else {
            return;
        };
        assert_eq!(first, make());
    }
}
