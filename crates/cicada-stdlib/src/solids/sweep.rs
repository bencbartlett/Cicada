//! The `sweep` node (v0.1 item 3 WP-C).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve, Solid};
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`sweep`].
#[derive(Ports, Clone, Debug)]
pub struct SweepIn {
    /// The rail to sweep along — a line, polyline or circle, open or
    /// closed.
    pub rail: Curve,
    /// The closed, planar section; place it at the rail's start, normal to
    /// the rail, for the classic sweep (it is swept exactly where it is).
    pub profile: Closed<Curve>,
}

/// Sweep — sweep a closed planar section along a rail into a B-rep solid
/// (GH Sweep1): the section keeps its orientation relative to the rail's
/// tangent as it travels, corners of a polyline rail are mitred, and the
/// ends are capped.
///
/// # Returns
///
/// The swept solid.
///
/// # Panics
///
/// Panics when the profile is degenerate, non-planar or self-intersecting
/// at tolerance, the rail has no length at tolerance, or the kernel refuses
/// (a section the sweep cannot carry around a corner, a sweep that does not
/// close into one solid).
///
/// # Examples
///
/// ```cic
/// start = construct_point()
/// top = construct_point(z=10.0)
/// rail = line(a=start, b=top)
/// span = construct_domain(start=-1.0, end=1.0)
/// section = rectangle(x=span, y=span)
/// riser = sweep(rail=rail, profile=section)
/// ```
#[node(
    category = "Surface & solid",
    tier = "1",
    version = 1,
    gh = "Sweep1",
    uses_tolerance
)]
#[must_use]
pub fn sweep(config: &ProjectConfig, input: SweepIn) -> Solid {
    red(cicada_geom::solid::sweep(
        &input.rail,
        &input.profile.0,
        config.tol(),
    ))
}

// No committed golden for `sweep`: MakePipeShell's surfaces are
// approximations whose coefficients the transcendental rule does not
// trust across platforms (`support.rs`); run-to-run identity and analytic
// volumes instead (the elbow is in the node set's heap-independence test).
#[cfg(test)]
mod tests {
    use cicada_core::geometry::{Line, Polyline};
    use cicada_core::spatial::Point;

    use super::*;
    use crate::solids::support::{close_rel, config, ring, volume_of, with_kernel};

    fn centred_square(half: f64) -> Closed<Curve> {
        ring(
            &[(-half, -half), (half, -half), (half, half), (-half, half)],
            0.0,
        )
    }

    fn straight(length: f64) -> Curve {
        Curve::Line(Line {
            a: Point::origin(),
            b: Point::new(0.0, 0.0, length),
        })
    }

    #[test]
    fn sweep_table_cases() {
        // A unit square along a straight 5-long rail: a 1 × 1 × 5 bar.
        let Some(bar) = with_kernel(|| {
            sweep(
                &config(),
                SweepIn {
                    rail: straight(5.0),
                    profile: centred_square(0.5),
                },
            )
        }) else {
            return;
        };
        assert!(close_rel(volume_of(&bar), 5.0, 1e-6));
        // An L-shaped rail with the section centred on it: area × length
        // (the mitre gives one bar what it takes from the other).
        let elbow = Curve::Polyline(Polyline {
            vertices: vec![
                Point::origin(),
                Point::new(0.0, 0.0, 4.0),
                Point::new(3.0, 0.0, 4.0),
            ],
            closed: false,
        });
        let swept = sweep(
            &config(),
            SweepIn {
                rail: elbow,
                profile: centred_square(0.5),
            },
        );
        assert!(close_rel(volume_of(&swept), 7.0, 1e-6));
    }

    #[test]
    #[should_panic(expected = "degenerate curve")]
    fn sweep_along_a_zero_length_rail_is_red() {
        let _ = sweep(
            &config(),
            SweepIn {
                rail: Curve::Line(Line {
                    a: Point::origin(),
                    b: Point::origin(),
                }),
                profile: centred_square(0.5),
            },
        );
    }

    proptest::proptest! {
        // A centred rectangle along a straight rail: volume = w h L.
        #[test]
        fn property_sweep_straight_bars(
            w in 0.1f64..3.0, h in 0.1f64..3.0, length in 0.2f64..20.0,
        ) {
            if cicada_geom::solid::kernel_available() {
                let out = sweep(
                    &config(),
                    SweepIn {
                        rail: straight(length),
                        profile: ring(
                            &[(-w / 2.0, -h / 2.0), (w / 2.0, -h / 2.0), (w / 2.0, h / 2.0), (-w / 2.0, h / 2.0)],
                            0.0,
                        ),
                    },
                );
                proptest::prop_assert!(close_rel(volume_of(&out), w * h * length, 1e-6));
            }
        }
    }

    #[test]
    fn sweep_determinism_run_to_run() {
        let make = || {
            sweep(
                &config(),
                SweepIn {
                    rail: straight(3.0),
                    profile: centred_square(0.5),
                },
            )
        };
        let Some(first) = with_kernel(make) else {
            return;
        };
        assert_eq!(first, make());
    }
}
