//! The `voronoi` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve};
use cicada_core::spatial::Point;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`voronoi`].
#[derive(Ports, Clone, Debug)]
pub struct VoronoiIn {
    /// Seed points — one cell per seed, index-aligned.
    pub seeds: Vec<Point>,
    /// Convex closed boundary the cells are clipped to.
    pub boundary: Closed<Curve>,
    /// Tessellation density for curved boundaries (circles).
    #[port(default = 64)]
    pub segments: i64,
}

/// Outputs of [`voronoi`].
#[derive(Ports, Clone, Debug)]
pub struct VoronoiOut {
    /// One closed cell per seed, index-aligned with `seeds`.
    pub cells: Vec<Closed<Curve>>,
}

/// Voronoi — planar Voronoi cells of seed points, clipped to a convex
/// boundary. Cells are index-aligned with the seeds (provenance never
/// shifts). Spike scope: the boundary must be convex and everything must
/// lie on its plane.
///
/// # Panics
///
/// Panics when the boundary is open, non-convex, or degenerate; when a
/// seed is off the boundary plane, coincides with another seed within
/// tolerance, or owns no area inside the boundary; or when `seeds` is
/// empty.
#[node(
    category = "Intersect & regions",
    tier = "S",
    version = 1,
    uses_tolerance
)]
#[must_use]
pub fn voronoi(config: &ProjectConfig, input: VoronoiIn) -> VoronoiOut {
    let cells = red(cicada_geom::voronoi::voronoi(
        &input.seeds,
        &input.boundary.0,
        input.segments,
        config.tol(),
    ));
    VoronoiOut {
        cells: cells.into_iter().map(Closed).collect(),
    }
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::Rectangle;
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::Plane;
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    fn config() -> ProjectConfig {
        ProjectConfig::default()
    }

    fn square(size: f64) -> Closed<Curve> {
        Closed(Curve::Rectangle(Rectangle {
            plane: Plane::world_xy(),
            x: Domain::new(0.0, size),
            y: Domain::new(0.0, size),
        }))
    }

    #[test]
    fn cells_are_index_aligned_and_closed() {
        let out = voronoi(
            &config(),
            VoronoiIn {
                seeds: vec![Point::new(2.0, 5.0, 0.0), Point::new(8.0, 5.0, 0.0)],
                boundary: square(10.0),
                segments: 64,
            },
        );
        assert_eq!(out.cells.len(), 2);
        for cell in &out.cells {
            assert!(cell.0.is_closed());
        }
        // The first cell belongs to the first (left) seed: every vertex
        // has x <= 5 (the bisector).
        let Curve::Polyline(p) = &out.cells[0].0 else {
            panic!("cells are polylines")
        };
        assert!(p.vertices.iter().all(|v| v.0.x <= 5.0 + 1e-9));
    }

    #[test]
    #[should_panic(expected = "coincide")]
    fn duplicate_seeds_are_red() {
        let _ = voronoi(
            &config(),
            VoronoiIn {
                seeds: vec![Point::new(1.0, 1.0, 0.0), Point::new(1.0, 1.0, 0.0)],
                boundary: square(10.0),
                segments: 64,
            },
        );
    }

    proptest::proptest! {
        // Any distinct grid subset: one closed cell per seed, tiling the
        // boundary area.
        #[test]
        fn property_cells_tile(
            grid in proptest::collection::hash_set((1u32..9, 1u32..9), 2..12),
        ) {
            // HashSet iteration order is nondeterministic — sort so a
            // persisted proptest-regressions entry replays the exact same
            // seed order every run.
            let mut grid: Vec<(u32, u32)> = grid.into_iter().collect();
            grid.sort_unstable();
            let seeds: Vec<Point> = grid
                .iter()
                .map(|&(i, j)| Point::new(f64::from(i), f64::from(j), 0.0))
                .collect();
            let out = voronoi(
                &config(),
                VoronoiIn {
                    seeds: seeds.clone(),
                    boundary: square(10.0),
                    segments: 64,
                },
            );
            proptest::prop_assert_eq!(out.cells.len(), seeds.len());
            let mut total = 0.0;
            for cell in &out.cells {
                let Curve::Polyline(p) = &cell.0 else {
                    panic!("cells are polylines")
                };
                let mut doubled = 0.0;
                for (i, a) in p.vertices.iter().enumerate() {
                    let b = p.vertices[(i + 1) % p.vertices.len()];
                    doubled += a.0.x * b.0.y - b.0.x * a.0.y;
                }
                total += doubled.abs() / 2.0;
            }
            proptest::prop_assert!((total - 100.0).abs() < 1e-6, "total {}", total);
        }
    }

    // Golden hash over the whole cell list — locks spade's output geometry
    // and the clip order cross-platform.
    #[test]
    fn determinism_golden_hash() {
        let out = voronoi(
            &config(),
            VoronoiIn {
                seeds: vec![
                    Point::new(3.0, 3.0, 0.0),
                    Point::new(7.0, 4.0, 0.0),
                    Point::new(5.0, 8.0, 0.0),
                ],
                boundary: square(10.0),
                segments: 64,
            },
        );
        let slots = out
            .cells
            .into_iter()
            .map(|cell| Some(HashedValue::new(ValueData::Curve(cell.0)).unwrap()))
            .collect();
        let list = HashedValue::new(ValueData::List(cicada_core::value::List {
            axis: None,
            slots,
        }))
        .unwrap();
        assert_eq!(
            list.hash().to_hex(),
            "f9166552acfa7ab5f062b897f4df08f3e508a23e7b5801bbbbd3d4638c69a6fd"
        );
    }
}
