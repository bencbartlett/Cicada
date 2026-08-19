//! Ear-clipping triangulation of simple 2D polygons (extrude caps,
//! Voronoi cells). O(n²) with deterministic clip order — first valid ear
//! from the list head, every run, every platform.
//!
//! Boundary contract: the triangulation USES every input vertex —
//! collinear vertices emit a zero-area triangle rather than being skipped,
//! so a cap's boundary edges match the polygon's edges exactly (extrude
//! walls stitch against them; skipping a vertex would tear the mesh).

use glam::DVec2;

use crate::GeomError;

/// Twice the signed area of a polygon (positive = counter-clockwise).
#[must_use]
pub fn signed_area_doubled(polygon: &[DVec2]) -> f64 {
    let mut sum = 0.0;
    for (i, a) in polygon.iter().enumerate() {
        let b = polygon[(i + 1) % polygon.len()];
        sum += a.x * b.y - b.x * a.y;
    }
    sum
}

/// Triangulate a simple polygon. Output triangles are indices into the
/// input slice and wind counter-clockwise regardless of input winding.
///
/// `tol` is the linear tolerance; area comparisons use `tol²`.
///
/// # Errors
///
/// [`GeomError::NotSimple`] when the polygon has under 3 vertices,
/// effectively zero area, no clippable ear remains (self-intersection), or
/// the finished triangulation fails post-validation — a strictly clockwise
/// triangle or a signed-area mismatch, symptoms of a self-intersecting
/// polygon (e.g. crossing edges with no vertex inside any ear) slipping
/// past the per-ear tests.
pub fn ear_clip(polygon: &[DVec2], tol: f64) -> Result<Vec<[u32; 3]>, GeomError> {
    if polygon.len() < 3 {
        return Err(GeomError::NotSimple {
            reason: format!("{} vertices (need 3)", polygon.len()),
        });
    }
    let area2 = signed_area_doubled(polygon);
    let area_tol = tol * tol;
    if area2.abs() <= area_tol {
        return Err(GeomError::NotSimple {
            reason: format!("effectively zero area ({})", area2 / 2.0),
        });
    }

    // Work on an index list ordered counter-clockwise; emit CCW triangles.
    let mut order: Vec<u32> =
        (0..u32::try_from(polygon.len()).map_err(|_| GeomError::NotSimple {
            reason: "more than u32::MAX vertices".to_owned(),
        })?)
            .collect();
    if area2 < 0.0 {
        order.reverse();
    }

    let cross = |o: DVec2, a: DVec2, b: DVec2| (a - o).perp_dot(b - o);
    let mut triangles = Vec::with_capacity(polygon.len() - 2);
    while order.len() > 3 {
        let n = order.len();
        let mut clipped = false;
        for i in 0..n {
            let prev = polygon[order[(i + n - 1) % n] as usize];
            let here = polygon[order[i] as usize];
            let next = polygon[order[(i + 1) % n] as usize];
            let turn = cross(prev, here, next);
            if turn < -area_tol {
                continue; // reflex — never an ear
            }
            // Collinear (|turn| <= area_tol): a zero-area ear; nothing can
            // lie STRICTLY inside it, so it clips immediately, keeping the
            // boundary edges intact (see module docs).
            let is_ear = turn.abs() <= area_tol
                || order
                    .iter()
                    .enumerate()
                    .filter(|&(j, _)| j != i && j != (i + n - 1) % n && j != (i + 1) % n)
                    .all(|(_, &candidate)| {
                        let p = polygon[candidate as usize];
                        // Closed-triangle test, tolerance-padded outward: a
                        // candidate ON the ear's boundary blocks it too. A
                        // reflex vertex exactly on the closing diagonal (or
                        // a pinch vertex on an edge) would make the clip
                        // cover geometry outside the remaining polygon and
                        // force a compensating clockwise triangle later.
                        !(cross(prev, here, p) >= -area_tol
                            && cross(here, next, p) >= -area_tol
                            && cross(next, prev, p) >= -area_tol)
                    });
            if is_ear {
                triangles.push([order[(i + n - 1) % n], order[i], order[(i + 1) % n]]);
                order.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            return Err(GeomError::NotSimple {
                reason: "no clippable ear (self-intersecting polygon?)".to_owned(),
            });
        }
    }
    triangles.push([order[0], order[1], order[2]]);

    // Post-validate. A pinched (self-touching) polygon can slip past the
    // ear tests — an exactly-on-edge vertex does not block an ear — and
    // emit a CLOCKWISE triangle that double-covers part of the polygon:
    // structurally watertight downstream, geometrically self-intersecting.
    // Two loud checks, both in doubled-area units like `turn` above:
    // (a) no triangle may be strictly clockwise (zero-area collinear ears
    //     stay legal — only below -area_tol fails);
    // (b) the signed doubled areas must sum to the polygon's CCW-normalized
    //     doubled area. The identity is exact in real arithmetic (each clip
    //     splits the shoelace sum exactly), so the budget is only tolerated
    //     slack: up to `area_tol` per clipped ear (n of them) plus a
    //     relative term for f64 rounding at large coordinates.
    #[allow(clippy::cast_precision_loss)] // vertex counts are far below 2^53
    let sum_tol = area_tol * polygon.len() as f64 + 1e-12 * area2.abs();
    let mut signed_sum = 0.0;
    for &[a, b, c] in &triangles {
        let doubled = cross(
            polygon[a as usize],
            polygon[b as usize],
            polygon[c as usize],
        );
        if doubled < -area_tol {
            return Err(GeomError::NotSimple {
                reason: format!(
                    "triangulation emitted a clockwise triangle (signed area {}); \
                     pinched/self-touching polygon",
                    doubled / 2.0
                ),
            });
        }
        signed_sum += doubled;
    }
    if (signed_sum - area2.abs()).abs() > sum_tol {
        return Err(GeomError::NotSimple {
            reason: format!(
                "triangulation area {} does not match polygon area {}; \
                 self-intersecting polygon",
                signed_sum / 2.0,
                area2.abs() / 2.0
            ),
        });
    }
    Ok(triangles)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f64 = 1e-6;

    fn total_area(polygon: &[DVec2], triangles: &[[u32; 3]]) -> f64 {
        triangles
            .iter()
            .map(|t| {
                let [a, b, c] = [
                    polygon[t[0] as usize],
                    polygon[t[1] as usize],
                    polygon[t[2] as usize],
                ];
                (b - a).perp_dot(c - a) / 2.0
            })
            .sum()
    }

    #[test]
    fn square_gives_two_ccw_triangles() {
        let square = [
            DVec2::new(0.0, 0.0),
            DVec2::new(1.0, 0.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(0.0, 1.0),
        ];
        let triangles = ear_clip(&square, TOL).expect("triangulates");
        assert_eq!(triangles.len(), 2);
        assert!((total_area(&square, &triangles) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn clockwise_input_still_emits_ccw_triangles() {
        let cw = [
            DVec2::new(0.0, 0.0),
            DVec2::new(0.0, 1.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(1.0, 0.0),
        ];
        let triangles = ear_clip(&cw, TOL).expect("triangulates");
        assert!(
            (total_area(&cw, &triangles) - 1.0).abs() < 1e-12,
            "positive = CCW"
        );
    }

    #[test]
    fn concave_polygon_triangulates_to_its_area() {
        // An L shape: area 3.
        let l_shape = [
            DVec2::new(0.0, 0.0),
            DVec2::new(2.0, 0.0),
            DVec2::new(2.0, 1.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(1.0, 2.0),
            DVec2::new(0.0, 2.0),
        ];
        let triangles = ear_clip(&l_shape, TOL).expect("triangulates");
        assert_eq!(triangles.len(), 4);
        assert!((total_area(&l_shape, &triangles) - 3.0).abs() < 1e-12);
        // Regression: the reflex corner (1,1) lies exactly on the diagonal
        // (0,2)-(2,0); the on-edge vertex used not to block that ear, and
        // this triangulation contained a clockwise triangle (area -0.5)
        // hidden by the telescoping area sum. Every triangle must be CCW.
        for area in signed_areas(&l_shape, &triangles) {
            assert!(area >= -1e-12, "clockwise triangle (area {area})");
        }
    }

    #[test]
    fn collinear_vertex_is_consumed_not_skipped() {
        // Midpoint of the bottom edge is collinear.
        let with_mid = [
            DVec2::new(0.0, 0.0),
            DVec2::new(1.0, 0.0),
            DVec2::new(2.0, 0.0),
            DVec2::new(2.0, 2.0),
            DVec2::new(0.0, 2.0),
        ];
        let triangles = ear_clip(&with_mid, TOL).expect("triangulates");
        // n-2 triangles ALWAYS — every vertex consumed (boundary contract).
        assert_eq!(triangles.len(), 3);
        assert!((total_area(&with_mid, &triangles) - 4.0).abs() < 1e-12);
    }

    fn signed_areas(polygon: &[DVec2], triangles: &[[u32; 3]]) -> Vec<f64> {
        triangles
            .iter()
            .map(|t| {
                let [a, b, c] = [
                    polygon[t[0] as usize],
                    polygon[t[1] as usize],
                    polygon[t[2] as usize],
                ];
                (b - a).perp_dot(c - a) / 2.0
            })
            .collect()
    }

    #[test]
    fn pinched_polygon_never_emits_a_cw_triangle() {
        // Vertex 3 = (1, 0) lies exactly ON the bottom edge, pinching the
        // polygon into two triangular lobes that touch there. On-edge
        // vertices used not to block an ear, so the clip covered geometry
        // outside the polygon and the run ended in a CLOCKWISE triangle
        // double-covering the left lobe — silently, since signed areas
        // telescope. The closed-triangle block now steers clipping around
        // the pinch: every triangle non-negative, areas summing to the two
        // lobes (the pinch edge itself becomes a legal zero-area triangle).
        let pinched = [
            DVec2::new(0.0, 0.0),
            DVec2::new(2.0, 0.0),
            DVec2::new(2.0, 2.0),
            DVec2::new(1.0, 0.0),
            DVec2::new(0.0, 2.0),
        ];
        let triangles = ear_clip(&pinched, TOL).expect("triangulates");
        assert_eq!(triangles.len(), 3);
        for area in signed_areas(&pinched, &triangles) {
            assert!(area >= -1e-12, "clockwise triangle (area {area})");
        }
        assert!((total_area(&pinched, &triangles) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn edge_crossing_polygon_refuses_via_post_validation() {
        // Self-intersecting quad: edges (4,0)-(1,2) and (3,2)-(0,0) cross
        // at (2, 4/3) with no vertex inside any ear, so the ear tests never
        // notice. This used to return Ok — first ear (3,2),(0,0),(4,0),
        // then a final CLOCKWISE triangle (4,0),(1,2),(3,2) of area -2 that
        // poisons downstream booleans. Post-validation now refuses loudly.
        let crossing = [
            DVec2::new(0.0, 0.0),
            DVec2::new(4.0, 0.0),
            DVec2::new(1.0, 2.0),
            DVec2::new(3.0, 2.0),
        ];
        assert!(matches!(
            ear_clip(&crossing, TOL),
            Err(GeomError::NotSimple { .. })
        ));
    }

    #[test]
    fn degenerate_polygons_refuse() {
        assert!(matches!(
            ear_clip(&[DVec2::new(0.0, 0.0), DVec2::new(1.0, 0.0)], TOL),
            Err(GeomError::NotSimple { .. })
        ));
        let zero_area = [
            DVec2::new(0.0, 0.0),
            DVec2::new(1.0, 0.0),
            DVec2::new(2.0, 0.0),
        ];
        assert!(matches!(
            ear_clip(&zero_area, TOL),
            Err(GeomError::NotSimple { .. })
        ));
    }

    proptest::proptest! {
        // Any convex polygon (regular n-gon, arbitrary rotation/scale)
        // triangulates to n-2 triangles covering its exact area.
        #[test]
        fn property_regular_polygons_triangulate_exactly(
            sides in 3usize..24,
            rotation in 0.0f64..std::f64::consts::TAU,
            radius in 0.1f64..100.0,
        ) {
            let polygon: Vec<DVec2> = (0..sides)
                .map(|i| {
                    #[allow(clippy::cast_precision_loss)]
                    let angle = rotation + std::f64::consts::TAU * i as f64 / sides as f64;
                    DVec2::new(radius * angle.cos(), radius * angle.sin())
                })
                .collect();
            let triangles = ear_clip(&polygon, TOL).expect("convex always triangulates");
            proptest::prop_assert_eq!(triangles.len(), sides - 2);
            let expected = signed_area_doubled(&polygon) / 2.0;
            let got = total_area(&polygon, &triangles);
            proptest::prop_assert!((got - expected).abs() <= 1e-9 * expected.abs());
        }
    }
}
