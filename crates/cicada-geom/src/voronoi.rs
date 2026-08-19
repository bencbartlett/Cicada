//! Voronoi partition (docs/08 §9) via the spade seam (DECISIONS.md row 16).
//!
//! Shape of the computation: spade builds the Delaunay triangulation (its
//! robust predicates carry the hard geometry); each seed's cell is then cut
//! by half-plane clipping the boundary against the perpendicular bisectors
//! of its DELAUNAY NEIGHBORS — for a convex boundary this yields the exact
//! clipped Voronoi cell with no unbounded-ray bookkeeping (a cell is the
//! bisector intersection over its neighbors).
//!
//! Spike scope, stated loudly: the boundary must be CONVEX (the wall's
//! boundary is a rectangle); concave boundaries arrive with a real polygon
//! clipping library (`i_overlay`, tier 1). Everything is planar: seeds
//! must lie on the boundary's plane within tolerance.

use cicada_core::geometry::{Curve, Polyline};
use cicada_core::spatial::Point;
use glam::DVec2;
use spade::{DelaunayTriangulation, HasPosition, Point2, Triangulation as _};

use crate::frame::polygon_frame;
use crate::triangulate::signed_area_doubled;
use crate::{GeomError, curve as curve_ops, tol};

struct Seed {
    position: Point2<f64>,
    ordinal: usize,
}

impl HasPosition for Seed {
    type Scalar = f64;

    fn position(&self) -> Point2<f64> {
        self.position
    }
}

/// Compute the Voronoi cells of `seeds` clipped to a convex closed
/// `boundary`. `segments` tessellates curved boundaries (circles).
/// Cell `i` belongs to seed `i` — index-aligned provenance, always.
///
/// # Errors
///
/// [`GeomError`]: open/degenerate/non-convex/non-planar boundary,
/// off-plane or duplicate seeds, a seed owning no area inside the
/// boundary, or under 1 seed.
pub fn voronoi(
    seeds: &[Point],
    boundary: &Curve,
    segments: i64,
    tolerance: f64,
) -> Result<Vec<Curve>, GeomError> {
    if seeds.is_empty() {
        return Err(GeomError::BadParameter {
            name: "seeds",
            value: "0 points".to_owned(),
            requirement: "at least one seed",
        });
    }
    let loop_points = curve_ops::tessellate_closed(boundary, segments, tolerance)?;
    let frame = polygon_frame(&loop_points, tolerance)?;
    let area_tol = tolerance * tolerance;
    let boundary_2d = convex_boundary_2d(&loop_points, &frame, tolerance)?;
    let seeds_2d = planar_distinct_seeds_2d(seeds, &frame, tolerance)?;

    // Delaunay for the neighbor structure.
    let mut triangulation: DelaunayTriangulation<Seed> = DelaunayTriangulation::new();
    let mut handles = Vec::with_capacity(seeds_2d.len());
    for (ordinal, seed) in seeds_2d.iter().enumerate() {
        let handle = triangulation
            .insert(Seed {
                position: Point2::new(seed.x, seed.y),
                ordinal,
            })
            .map_err(|error| GeomError::Kernel {
                reason: format!("spade refused seed {ordinal}: {error:?}"),
            })?;
        handles.push(handle);
    }

    let mut cells = Vec::with_capacity(seeds_2d.len());
    for (ordinal, &handle) in handles.iter().enumerate() {
        // Neighbor ordinals, sorted: clip order is part of the value
        // (floating-point roundoff depends on it), so pin it.
        let mut neighbors: Vec<usize> = triangulation
            .vertex(handle)
            .out_edges()
            .map(|edge| edge.to().data().ordinal)
            .collect();
        neighbors.sort_unstable();
        let mut cell = boundary_2d.clone();
        let here = seeds_2d[ordinal];
        for neighbor in neighbors {
            let other = seeds_2d[neighbor];
            let normal = other - here; // half-plane: (p − mid) · normal <= 0
            let mid = (here + other) / 2.0;
            cell = clip_half_plane(&cell, mid, normal);
            if cell.is_empty() {
                break;
            }
        }
        // Collapse tolerance-slivers before judging emptiness.
        let cleaned = dedupe_loop(&cell, tolerance);
        if cleaned.len() < 3 || signed_area_doubled(&cleaned).abs() <= area_tol {
            return Err(GeomError::Kernel {
                reason: format!(
                    "seed {ordinal} owns no area inside the boundary (cell is empty at \
                     tolerance {tolerance})"
                ),
            });
        }
        cells.push(Curve::Polyline(Polyline {
            vertices: cleaned.iter().map(|p| frame.point_at(p.x, p.y)).collect(),
            closed: true,
        }));
    }
    Ok(cells)
}

/// Boundary loop → planar, convex, CCW 2D polygon — refusing non-planar
/// vertices and reflex corners loudly.
fn convex_boundary_2d(
    loop_points: &[Point],
    frame: &crate::frame::Frame,
    tolerance: f64,
) -> Result<Vec<DVec2>, GeomError> {
    let area_tol = tolerance * tolerance;
    let mut boundary_2d = Vec::with_capacity(loop_points.len());
    for (vertex, point) in loop_points.iter().enumerate() {
        let local = frame.coordinates(*point);
        if !tol::near_zero(local.z, tolerance) {
            return Err(GeomError::NotPlanar {
                vertex,
                distance: local.z,
            });
        }
        boundary_2d.push(DVec2::new(local.x, local.y));
    }
    // Raw sign test sanctioned: an effectively-zero-area boundary yields
    // only empty cells, which the per-seed area cull below refuses loudly
    // — orientation of the degenerate band never decides an output (tol
    // discipline, doc 14).
    if signed_area_doubled(&boundary_2d) < 0.0 {
        boundary_2d.reverse();
    }
    let n = boundary_2d.len();
    for i in 0..n {
        let (a, b, c) = (
            boundary_2d[i],
            boundary_2d[(i + 1) % n],
            boundary_2d[(i + 2) % n],
        );
        if (b - a).perp_dot(c - b) < -area_tol {
            return Err(GeomError::BadParameter {
                name: "boundary",
                value: format!("reflex corner at boundary vertex {}", (i + 1) % n),
                requirement: "must be convex (spike scope; concave boundaries are v0.1)",
            });
        }
    }
    Ok(boundary_2d)
}

/// Seeds → the boundary plane; refuse off-plane and duplicates (cells are
/// index-aligned with seeds — a merged seed would silently shift every
/// later cell).
fn planar_distinct_seeds_2d(
    seeds: &[Point],
    frame: &crate::frame::Frame,
    tolerance: f64,
) -> Result<Vec<DVec2>, GeomError> {
    let mut seeds_2d = Vec::with_capacity(seeds.len());
    for (ordinal, seed) in seeds.iter().enumerate() {
        let local = frame.coordinates(*seed);
        if !tol::near_zero(local.z, tolerance) {
            return Err(GeomError::BadParameter {
                name: "seeds",
                value: format!("seed {ordinal} lies {} off the boundary plane", local.z),
                requirement: "seeds must lie on the boundary plane within tolerance",
            });
        }
        seeds_2d.push(DVec2::new(local.x, local.y));
    }
    for i in 0..seeds_2d.len() {
        for j in (i + 1)..seeds_2d.len() {
            if seeds_2d[i].distance_squared(seeds_2d[j]) <= tolerance * tolerance {
                return Err(GeomError::BadParameter {
                    name: "seeds",
                    value: format!("seeds {i} and {j} coincide at tolerance {tolerance}"),
                    requirement: "seeds must be distinct (cells are index-aligned)",
                });
            }
        }
    }
    Ok(seeds_2d)
}

/// Sutherland–Hodgman against one half-plane `{p : (p − on) · normal <= 0}`.
fn clip_half_plane(polygon: &[DVec2], on: DVec2, normal: DVec2) -> Vec<DVec2> {
    let mut out = Vec::with_capacity(polygon.len() + 1);
    // Exact-predicate clipping BY DESIGN: bisector membership is a pure
    // sign test (tolerance-free, like spade's own predicates); tolerance
    // enters afterwards, where dedupe_loop collapses slivers and the
    // per-seed area cull refuses empty cells — one place, not per edge.
    let inside = |p: DVec2| (p - on).dot(normal) <= 0.0;
    for (i, &a) in polygon.iter().enumerate() {
        let b = polygon[(i + 1) % polygon.len()];
        let (a_in, b_in) = (inside(a), inside(b));
        if a_in {
            out.push(a);
        }
        if a_in != b_in {
            let da = (a - on).dot(normal);
            let db = (b - on).dot(normal);
            // da ≠ db by construction (they straddle the plane).
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

/// Consecutive-coincident dedupe over a closed 2D loop (including the wrap
/// pair).
fn dedupe_loop(polygon: &[DVec2], tolerance: f64) -> Vec<DVec2> {
    let mut out: Vec<DVec2> = Vec::with_capacity(polygon.len());
    for &p in polygon {
        if out
            .last()
            .is_none_or(|last| last.distance_squared(p) > tolerance * tolerance)
        {
            out.push(p);
        }
    }
    while out.len() > 1 {
        let (first, last) = (out[0], *out.last().unwrap_or(&out[0]));
        if first.distance_squared(last) <= tolerance * tolerance {
            out.pop();
        } else {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::Rectangle;
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::{Plane, Vector};

    use super::*;

    const TOL: f64 = 1e-6;

    fn boundary_square(size: f64) -> Curve {
        Curve::Rectangle(Rectangle {
            plane: Plane {
                origin: Point::new(0.0, 0.0, 0.0),
                x: Vector::new(1.0, 0.0, 0.0),
                y: Vector::new(0.0, 1.0, 0.0),
            },
            x: Domain::new(0.0, size),
            y: Domain::new(0.0, size),
        })
    }

    fn cell_area(cell: &Curve) -> f64 {
        let Curve::Polyline(p) = cell else {
            panic!("cells are closed polylines")
        };
        let flat: Vec<DVec2> = p
            .vertices
            .iter()
            .map(|v| DVec2::new(v.0.x, v.0.y))
            .collect();
        signed_area_doubled(&flat).abs() / 2.0
    }

    #[test]
    fn two_seeds_split_the_square_down_the_bisector() {
        let cells = voronoi(
            &[Point::new(2.5, 5.0, 0.0), Point::new(7.5, 5.0, 0.0)],
            &boundary_square(10.0),
            64,
            TOL,
        )
        .expect("computes");
        assert_eq!(cells.len(), 2);
        for cell in &cells {
            assert!(cell.is_closed());
            assert!((cell_area(cell) - 50.0).abs() < 1e-9);
        }
    }

    #[test]
    fn grid_seeds_tile_the_boundary_exactly() {
        let mut seeds = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                seeds.push(Point::new(
                    f64::from(i).mul_add(2.0, 1.0),
                    f64::from(j).mul_add(2.0, 1.0),
                    0.0,
                ));
            }
        }
        let cells = voronoi(&seeds, &boundary_square(8.0), 64, TOL).expect("computes");
        assert_eq!(cells.len(), 16);
        let total: f64 = cells.iter().map(cell_area).sum();
        assert!(
            (total - 64.0).abs() < 1e-6,
            "cells tile the boundary: total {total}"
        );
        // Interior cells of a uniform grid are 2×2 squares.
        assert!((cell_area(&cells[5]) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn refusals_are_loud() {
        // Duplicate seeds.
        assert!(matches!(
            voronoi(
                &[Point::new(1.0, 1.0, 0.0), Point::new(1.0, 1.0, 0.0)],
                &boundary_square(10.0),
                64,
                TOL
            ),
            Err(GeomError::BadParameter { name: "seeds", .. })
        ));
        // Off-plane seed.
        assert!(matches!(
            voronoi(
                &[Point::new(1.0, 1.0, 0.5)],
                &boundary_square(10.0),
                64,
                TOL
            ),
            Err(GeomError::BadParameter { name: "seeds", .. })
        ));
        // Non-convex boundary.
        let l_shape = Curve::Polyline(Polyline {
            vertices: vec![
                Point::new(0.0, 0.0, 0.0),
                Point::new(2.0, 0.0, 0.0),
                Point::new(2.0, 1.0, 0.0),
                Point::new(1.0, 1.0, 0.0),
                Point::new(1.0, 2.0, 0.0),
                Point::new(0.0, 2.0, 0.0),
            ],
            closed: true,
        });
        assert!(matches!(
            voronoi(&[Point::new(0.5, 0.5, 0.0)], &l_shape, 64, TOL),
            Err(GeomError::BadParameter {
                name: "boundary",
                ..
            })
        ));
    }

    #[test]
    fn single_seed_owns_the_whole_boundary() {
        let cells = voronoi(
            &[Point::new(5.0, 5.0, 0.0)],
            &boundary_square(10.0),
            64,
            TOL,
        )
        .expect("computes");
        assert_eq!(cells.len(), 1);
        assert!((cell_area(&cells[0]) - 100.0).abs() < 1e-9);
    }

    proptest::proptest! {
        // For any distinct seed set in the square: cells tile the boundary
        // (areas sum to the boundary area) and every cell is closed and
        // index-aligned.
        #[test]
        fn property_cells_tile_the_boundary(
            seed_grid in proptest::collection::hash_set((1u32..19, 1u32..19), 2..24),
        ) {
            // HashSet iteration order is nondeterministic — sort so a
            // persisted proptest-regressions entry replays the exact same
            // seed order every run.
            let mut seed_grid: Vec<(u32, u32)> = seed_grid.into_iter().collect();
            seed_grid.sort_unstable();
            let seeds: Vec<Point> = seed_grid
                .iter()
                .map(|&(i, j)| Point::new(f64::from(i) / 2.0, f64::from(j) / 2.0, 0.0))
                .collect();
            let cells = voronoi(&seeds, &boundary_square(10.0), 64, TOL).expect("computes");
            proptest::prop_assert_eq!(cells.len(), seeds.len());
            let total: f64 = cells.iter().map(cell_area).sum();
            proptest::prop_assert!((total - 100.0).abs() < 1e-6, "total {}", total);
        }
    }
}
