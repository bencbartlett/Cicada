//! The `loft` node: the B-rep loft, OCCT-backed (v0.1 item 3 WP-C; the
//! spike's two-section mesh loft continues as `mesh_loft`).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve, Solid};
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`loft`].
#[derive(Ports, Clone, Debug)]
pub struct LoftIn {
    /// The closed, planar sections in order (at least two).
    pub profiles: Vec<Closed<Curve>>,
    /// Straight (ruled) surfaces between consecutive sections — GH Loft's
    /// "Straight" — or, when false, one smooth B-spline surface through all
    /// of them (GH's "Normal").
    #[port(default = true)]
    pub ruled: bool,
}

/// Loft — a B-rep solid through two or more closed planar sections, capped
/// at both ends: ruled between consecutive sections by default (a polyline
/// frustum has planar faces; the wall's shape), or smooth through them.
/// Sections are made compatible before the surface is built — edge counts
/// matched, orientations aligned, seams aligned — so polylines of
/// different vertex counts loft too.
///
/// # Returns
///
/// The lofted solid.
///
/// # Panics
///
/// Panics when fewer than two profiles are given, a profile is degenerate,
/// non-planar or self-intersecting at tolerance (the message names its
/// index), or the kernel refuses (coincident sections, a loft that does
/// not close into one solid).
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=2.0)
/// base = rectangle(x=span, y=span)
/// up = unit_z(factor=3.0)
/// sections = linear_array(geometry=base, direction=up, count=2)
/// bar = loft(profiles=sections)
/// ```
#[node(
    category = "Surface & solid",
    tier = "S",
    version = 1,
    gh = "Loft",
    uses_tolerance
)]
#[must_use]
pub fn loft(config: &ProjectConfig, input: LoftIn) -> Solid {
    let profiles: Vec<Curve> = input.profiles.into_iter().map(|c| c.0).collect();
    red(cicada_geom::solid::loft(
        &profiles,
        input.ruled,
        config.tol(),
    ))
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use cicada_core::geometry::Circle;
    use cicada_core::spatial::{Plane, Point};

    use super::*;
    use crate::solids::support::{
        close_rel, config, frustum_volume, platform_golden, ring, solid_hash, unit_square_profile,
        volume_of, with_kernel,
    };

    #[test]
    fn loft_table_cases() {
        // Square → half-size square, 2 up: the exact square frustum.
        let base = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        let top = [(0.5, 0.5), (1.5, 0.5), (1.5, 1.5), (0.5, 1.5)];
        let Some(frustum) = with_kernel(|| {
            loft(
                &config(),
                LoftIn {
                    profiles: vec![ring(&base, 0.0), ring(&top, 2.0)],
                    ruled: true,
                },
            )
        }) else {
            return;
        };
        assert!(close_rel(
            volume_of(&frustum),
            frustum_volume(4.0, 0.5, 2.0),
            1e-12
        ));
        // Three sections: two stacked frusta.
        let tower = loft(
            &config(),
            LoftIn {
                profiles: vec![ring(&base, 0.0), ring(&top, 2.0), ring(&base, 4.0)],
                ruled: true,
            },
        );
        assert!(close_rel(
            volume_of(&tower),
            2.0 * frustum_volume(4.0, 0.5, 2.0),
            1e-12
        ));
        // Circle → circle: an exact cone frustum.
        let cone = loft(
            &config(),
            LoftIn {
                profiles: vec![
                    Closed(Curve::Circle(Circle {
                        plane: Plane::world_xy(),
                        radius: 1.0,
                    })),
                    Closed(Curve::Circle(Circle {
                        plane: Plane {
                            origin: Point::new(0.0, 0.0, 1.0),
                            ..Plane::world_xy()
                        },
                        radius: 0.25,
                    })),
                ],
                ruled: true,
            },
        );
        let want = PI / 3.0 * (1.0 + 0.25 + 0.0625);
        assert!(close_rel(volume_of(&cone), want, 1e-9));
        // Different vertex counts loft once made compatible: a square to a
        // triangle.
        let wedge = loft(
            &config(),
            LoftIn {
                profiles: vec![
                    ring(&base, 0.0),
                    ring(&[(0.5, 0.5), (1.5, 0.5), (1.0, 1.5)], 2.0),
                ],
                ruled: true,
            },
        );
        assert!(volume_of(&wedge) > 0.0);
        // Smooth through three sections is a different solid from the
        // ruled one — the port does something.
        let smooth = loft(
            &config(),
            LoftIn {
                profiles: vec![ring(&base, 0.0), ring(&top, 2.0), ring(&base, 4.0)],
                ruled: false,
            },
        );
        assert_ne!(smooth, tower);
        assert!(volume_of(&smooth) > 0.0);
    }

    #[test]
    #[should_panic(expected = "a loft needs at least two profiles")]
    fn loft_one_profile_is_red() {
        let _ = loft(
            &config(),
            LoftIn {
                profiles: vec![unit_square_profile()],
                ruled: true,
            },
        );
    }

    #[test]
    #[should_panic(expected = "profile 1:")]
    fn loft_degenerate_section_is_red_with_its_index() {
        let _ = loft(
            &config(),
            LoftIn {
                profiles: vec![
                    unit_square_profile(),
                    // Collinear: no plane, no area.
                    ring(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)], 1.0),
                ],
                ruled: true,
            },
        );
    }

    proptest::proptest! {
        // Lofts between a rectangle and its homothetic copy (any scale,
        // offset, height, either side): the exact frustum volume.
        #[test]
        fn property_loft_frustum_volume(
            w in 0.1..10.0_f64, h in 0.1..10.0_f64,
            scale in 0.05..3.0_f64,
            dx in -2.0..2.0_f64, dy in -2.0..2.0_f64,
            height in proptest::prop_oneof![-20.0f64..-0.05, 0.05f64..20.0],
        ) {
            if cicada_geom::solid::kernel_available() {
                let base = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
                let top: Vec<(f64, f64)> = base
                    .iter()
                    .map(|&(x, y)| (dx + x * scale, dy + y * scale))
                    .collect();
                let out = loft(
                    &config(),
                    LoftIn {
                        profiles: vec![ring(&base, 0.0), ring(&top, height)],
                        ruled: true,
                    },
                );
                let want = frustum_volume(w * h, scale, height.abs());
                proptest::prop_assert!(close_rel(volume_of(&out), want, 1e-9), "got {} want {}", volume_of(&out), want);
            }
        }
    }

    #[test]
    fn loft_determinism_golden_hash() {
        // Arithmetic-only inputs: the wall's pentagonal cell → tip cap, as
        // polylines with exact coordinates. Blessed via run-once on win-64
        // (2026-08-20).
        let Some(frustum) = with_kernel(|| {
            loft(
                &config(),
                LoftIn {
                    profiles: vec![
                        ring(
                            &[(0.0, 0.0), (4.0, 0.0), (5.0, 3.0), (2.0, 5.0), (-1.0, 3.0)],
                            0.0,
                        ),
                        ring(
                            &[(2.0, 1.0), (2.75, 1.5), (3.5, 2.0), (2.0, 3.5), (0.5, 2.0)],
                            12.0,
                        ),
                    ],
                    ruled: true,
                },
            )
        }) else {
            return;
        };
        assert_eq!(
            solid_hash(&frustum),
            platform_golden("bf5a61c9a03e5e9add5fb41899d27618cc3205df556f611cb2cc229bf4a6a617")
        );
    }
}
