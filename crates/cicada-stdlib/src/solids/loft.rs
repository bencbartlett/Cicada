//! The `loft` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve, Mesh, Watertight};
use cicada_macros::{Ports, node};

use crate::{PRISM_BYTES_PER_PROFILE_VERTEX, checked_count};

/// Inputs for [`loft`].
#[derive(Ports, Clone, Debug)]
pub struct LoftIn {
    /// The first closed section (the wall's base cell).
    pub start: Closed<Curve>,
    /// The second closed section (the wall's tip cap) — same vertex count as
    /// `start`; vertex `i` pairs with vertex `i`, seam = vertex 0.
    pub end: Closed<Curve>,
    /// Tessellation density for analytic sections (circles, rectangles):
    /// `segments` equal arc-length samples from the section's parameter
    /// origin — a circle's +x axis, a rectangle's `(x.start, y.start)`
    /// corner (so a rectangle's corners are hit only when the samples land
    /// on them, e.g. a square with `segments` divisible by 4; feed a
    /// 4-vertex polyline for an exact rectangular frustum).
    #[port(default = 64)]
    pub segments: i64,
}

/// Loft — a ruled solid between two closed sections, capped at both ends
/// (the wall's frusta: Voronoi cell → tip cap; cones and chamfers from
/// circle → circle). Sections pair vertex `i` with vertex `i` (polylines
/// vertex-to-vertex as given — no resampling, seam = vertex 0; analytic
/// sections tessellated to `segments` vertices starting at the plane's x
/// axis), walls are two triangles per quad, caps are ear-clipped
/// (non-convex sections welcome), orientation is fixed by signed volume
/// and the result is re-verified watertight. Lift with `each()` to loft
/// per part (`loft(start=each(cells), end=each(caps))`).
///
/// # Returns
///
/// The watertight solid between the two sections, capped at both ends.
///
/// # Panics
///
/// Panics when the vertex counts differ (both counts in the message), a
/// section is open, degenerate at tolerance, non-planar, or
/// self-intersecting, the sections wind in opposite directions, the
/// sections coincide or are coplanar (zero volume), `segments < 3` for an
/// analytic section, `segments` above the shared ceilings for an analytic
/// section (2^22 slots, or 1 GiB of prism at 96 bytes a profile vertex;
/// 4,194,304 is the last allowed; the message names the count and the
/// ceiling that bit), or the result is not watertight.
///
/// # Examples
///
/// ```cic
/// base = circle(radius=2.0)
/// top_at = construct_point(x=0.0, y=0.0, z=4.0)
/// top_frame = xy_plane(origin=top_at)
/// top = circle(plane=top_frame, radius=1.0)
/// cone = loft(start=base, end=top, segments=32)
/// ```
#[node(
    category = "Surface & solid",
    tier = "S",
    version = 2,
    gh = "Loft",
    uses_tolerance
)]
#[must_use]
pub fn loft(config: &ProjectConfig, input: LoftIn) -> Watertight<Mesh> {
    // Only analytic sections (circle, rectangle) tessellate to `segments`
    // vertices — polylines pair vertex to vertex and the port is unused —
    // so only then does `segments` size an allocation; the kernel keeps
    // the floor (`segments < 3`).
    let analytic = |curve: &Curve| matches!(curve, Curve::Circle(_) | Curve::Rectangle(_));
    if (analytic(&input.start.0) || analytic(&input.end.0)) && input.segments >= 3 {
        let _ = checked_count(
            "loft",
            "segments",
            input.segments,
            3,
            PRISM_BYTES_PER_PROFILE_VERTEX,
        );
    }
    match cicada_geom::meshbuild::loft(&input.start.0, &input.end.0, input.segments, config.tol()) {
        Ok(mesh) => Watertight(mesh),
        Err(error) => panic!("loft: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use cicada_core::spatial::{Plane, Point};
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::meshbuild::signed_volume;

    use super::*;
    use crate::solids::support::{config, frustum_volume, ring, unit_square_profile};

    #[test]
    fn loft_table_cases() {
        // Square → half-size square, 2 up: the exact square frustum.
        let base = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        let top = [(0.5, 0.5), (1.5, 0.5), (1.5, 1.5), (0.5, 1.5)];
        let frustum = loft(
            &config(),
            LoftIn {
                start: ring(&base, 0.0),
                end: ring(&top, 2.0),
                segments: 64,
            },
        );
        assert!(frustum.0.is_watertight());
        assert_eq!(frustum.0.triangle_count(), 12);
        assert!((signed_volume(&frustum.0) - frustum_volume(4.0, 0.5, 2.0)).abs() < 1e-12);
        // The wall's shape: a pentagonal cell → a tip-cap polygon with the
        // same count, three vertices on triangle corners, two on its edges.
        let cell = [(0.0, 0.0), (4.0, 0.0), (5.0, 3.0), (2.0, 5.0), (-1.0, 3.0)];
        let cap = [(2.0, 1.0), (2.75, 1.5), (3.5, 2.0), (2.0, 3.5), (0.5, 2.0)];
        let frustum = loft(
            &config(),
            LoftIn {
                start: ring(&cell, 0.0),
                end: ring(&cap, 12.0),
                segments: 64,
            },
        );
        assert!(frustum.0.is_watertight());
        assert_eq!(frustum.0.triangle_count(), 3 + 3 + 10);
        assert!(signed_volume(&frustum.0) > 0.0);
        // Circle → circle: a cone frustum, seam on +x.
        let cone = loft(
            &config(),
            LoftIn {
                start: Closed(Curve::Circle(cicada_core::geometry::Circle {
                    plane: Plane::world_xy(),
                    radius: 1.0,
                })),
                end: Closed(Curve::Circle(cicada_core::geometry::Circle {
                    plane: Plane {
                        origin: Point::new(0.0, 0.0, 1.0),
                        ..Plane::world_xy()
                    },
                    radius: 0.25,
                })),
                segments: 32,
            },
        );
        assert!(cone.0.is_watertight());
        assert_eq!(cone.0.vertex_count(), 64);
        let want = std::f64::consts::PI / 3.0 * (1.0 + 0.25 + 0.0625);
        assert!((signed_volume(&cone.0) - want).abs() / want < 2e-2);
    }

    #[test]
    #[should_panic(expected = "start has 4, end has 3")]
    fn loft_vertex_count_mismatch_is_red() {
        let _ = loft(
            &config(),
            LoftIn {
                start: ring(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)], 0.0),
                end: ring(&[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)], 1.0),
                segments: 64,
            },
        );
    }

    #[test]
    #[should_panic(expected = "zero volume")]
    fn loft_coincident_sections_are_red() {
        let _ = loft(
            &config(),
            LoftIn {
                start: unit_square_profile(),
                end: unit_square_profile(),
                segments: 64,
            },
        );
    }

    #[test]
    #[should_panic(expected = "end section: degenerate curve")]
    fn loft_degenerate_section_is_red() {
        let _ = loft(
            &config(),
            LoftIn {
                start: unit_square_profile(),
                end: ring(&[(0.0, 0.0), (1.0, 0.0), (1.0, 0.0), (0.0, 1.0)], 1.0),
                segments: 64,
            },
        );
    }

    /// One profile vertex past the 1 GiB prism ceiling (96 bytes each):
    /// the ceiling that bites first for a prism.
    fn one_past_the_prism_ceiling() -> i64 {
        let bytes = u64::try_from(PRISM_BYTES_PER_PROFILE_VERTEX).unwrap();
        assert!(u64::try_from(crate::MAX_SLOTS).unwrap() * bytes <= crate::MAX_BYTES);
        crate::MAX_SLOTS + 1
    }

    // Analytic sections tessellate to `segments` vertices each: one past
    // the slot ceiling (the one that bites first for a prism: 2^22 vertices
    // at 96 bytes is 384 MiB) is red with the count and the ceiling in the
    // message. This pins where the guard sits (a guard moved after the
    // sampling would pass it too — slowly, through the O(n²) cap clip);
    // the absurd case below is what detects that mutation.
    #[test]
    fn loft_analytic_one_past_the_ceiling_is_red() {
        let segments = one_past_the_prism_ceiling();
        assert_eq!(segments, 4_194_305);
        let panic = std::panic::catch_unwind(|| {
            loft(
                &config(),
                LoftIn {
                    start: unit_square_profile(),
                    end: ring(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)], 2.0),
                    segments,
                },
            )
        })
        .expect_err("one profile vertex past the slot ceiling refuses");
        assert_eq!(
            *panic.downcast_ref::<String>().unwrap(),
            "loft: segments is 4194305 — above the 4194304 (2^22) slot ceiling of one node \
             output"
        );
    }

    // The absurd density a literal or an Integer wire can carry: an
    // analytic section at 10^11 segments is an 800 GB parameter buffer and
    // 2.4 TB of samples (`divide` sizes them up front) no machine holds —
    // with the guard after it this test binary would abort on allocation
    // failure (`catch_unwind` cannot catch that), so passing proves the
    // refusal precedes the allocation. Both analytic kinds are the case:
    // a rectangle start here, circles in `loft_too_few_segments_is_red`.
    #[test]
    #[should_panic(
        expected = "loft: segments is 100000000000 — above the 4194304 (2^22) slot ceiling of \
                    one node output"
    )]
    fn loft_analytic_absurd_segments_are_refused_not_allocated() {
        let _ = loft(
            &config(),
            LoftIn {
                start: unit_square_profile(),
                end: ring(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)], 2.0),
                segments: 100_000_000_000,
            },
        );
    }

    // Polyline sections pair vertex to vertex — `segments` sizes nothing,
    // so the same count lofts the same frustum it always did.
    #[test]
    fn loft_polyline_sections_ignore_segments_as_before() {
        let base = [(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)];
        let top = [(0.5, 0.5), (1.5, 0.5), (1.5, 1.5), (0.5, 1.5)];
        let frustum = loft(
            &config(),
            LoftIn {
                start: ring(&base, 0.0),
                end: ring(&top, 2.0),
                segments: one_past_the_prism_ceiling(),
            },
        );
        assert_eq!(frustum.0.triangle_count(), 12);
        assert!((signed_volume(&frustum.0) - frustum_volume(4.0, 0.5, 2.0)).abs() < 1e-12);
    }

    #[test]
    #[should_panic(expected = "segments = 2 is out of range")]
    fn loft_too_few_segments_is_red() {
        let circle = Closed(Curve::Circle(cicada_core::geometry::Circle {
            plane: Plane::world_xy(),
            radius: 1.0,
        }));
        let _ = loft(
            &config(),
            LoftIn {
                start: circle.clone(),
                end: circle,
                segments: 2,
            },
        );
    }

    proptest::proptest! {
        // Lofts between a rectangle and its homothetic copy (any scale,
        // offset, height, either side): watertight with the exact frustum
        // volume — rectangles built as polylines, so the ports see the
        // wall's shape (vertex chains), not analytic sections.
        #[test]
        fn property_loft_frustum_volume(
            w in 0.1..10.0_f64, h in 0.1..10.0_f64,
            scale in 0.05..3.0_f64,
            dx in -2.0..2.0_f64, dy in -2.0..2.0_f64,
            height in proptest::prop_oneof![-20.0f64..-0.05, 0.05f64..20.0],
        ) {
            let base = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
            let top: Vec<(f64, f64)> = base
                .iter()
                .map(|&(x, y)| (dx + x * scale, dy + y * scale))
                .collect();
            let out = loft(
                &config(),
                LoftIn {
                    start: ring(&base, 0.0),
                    end: ring(&top, height),
                    segments: 64,
                },
            );
            proptest::prop_assert!(out.0.is_watertight());
            let want = frustum_volume(w * h, scale, height.abs());
            let got = signed_volume(&out.0);
            proptest::prop_assert!(
                (got - want).abs() <= 1e-9 * want.max(1.0),
                "got {} want {}", got, want
            );
        }
    }

    #[test]
    fn loft_determinism_golden_hash() {
        // Arithmetic-only inputs: polyline sections with exact coordinates
        // (no circles — transcendental-fed goldens are platform-dependent).
        let frustum = loft(
            &config(),
            LoftIn {
                start: ring(
                    &[(0.0, 0.0), (4.0, 0.0), (5.0, 3.0), (2.0, 5.0), (-1.0, 3.0)],
                    0.0,
                ),
                end: ring(
                    &[(2.0, 1.0), (2.75, 1.5), (3.5, 2.0), (2.0, 3.5), (0.5, 2.0)],
                    12.0,
                ),
                segments: 64,
            },
        );
        let sealed = HashedValue::new(ValueData::Mesh(frustum.0)).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            "8e11f929455b5223aed3edddb741337195df1bf36dc60b893a9c8e6e91eb6850"
        );
    }
}
