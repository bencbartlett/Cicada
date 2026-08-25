//! The `cull_duplicates` node.

use std::collections::HashMap;

use cicada_core::scalar::IndexMap;
use cicada_core::spatial::Point;
use cicada_geom::tol;
use cicada_macros::{Ports, node};

/// Inputs for [`cull_duplicates`].
#[derive(Ports, Clone, Debug)]
pub struct CullDuplicatesIn {
    /// The points to deduplicate, in order.
    pub points: Vec<Point>,
    /// Two points this close (Euclidean, inclusive) are duplicates; 0 keeps
    /// only exactly coincident points apart.
    #[port(dimension = length)]
    pub tolerance: f64,
}

/// Outputs of [`cull_duplicates`].
#[derive(Ports, Clone, Debug)]
pub struct CullDuplicatesOut {
    /// The kept points — the first of every group, in source order.
    pub points: Vec<Point>,
    /// Provenance: `map[i]` is the source index of `points[i]` (docs/08
    /// rule 6).
    pub map: IndexMap,
}

/// Cull Duplicates — drop every point within `tolerance` of an earlier kept
/// point, returning the survivors and the index map back into the source.
///
/// The rule is first-kept: a point is compared against the points already
/// kept, never against dropped ones, so the survivors are pairwise further
/// apart than the tolerance and each dropped point is within it of a
/// survivor that came before it.
///
/// # Panics
///
/// Panics when `tolerance` is negative — no two points are that close.
///
/// # Examples
///
/// ```cic
/// xs = [0.0, 1.0, 1.0000001, 5.0, 0.0]
/// pts = construct_point(x=each(xs), y=0.0, z=0.0)
/// unique, sources = cull_duplicates(points=pts, tolerance=0.001)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "1",
    version = 1,
    gh = "Cull Duplicates"
)]
#[must_use]
pub fn cull_duplicates(input: CullDuplicatesIn) -> CullDuplicatesOut {
    let tolerance = input.tolerance;
    assert!(
        tolerance >= 0.0,
        "cull_duplicates: tolerance must be >= 0, got {tolerance}"
    );
    // A uniform grid of cells `tolerance` wide is the candidate filter: two
    // points within `tolerance` of each other differ by at most that per
    // axis, so they sit in the same or an adjacent cell — the 27 cells
    // around a point hold every possible duplicate, and `tol::coincident`
    // decides. At tolerance 0 only exactly equal points coincide, so the
    // cell is the coordinates themselves and only that one cell is
    // searched. Lookups only — no iteration over the map — so the output
    // never depends on hash order.
    // The zero/positive split of the parameter (negative was refused above),
    // not a geometry comparison.
    let exact = tolerance <= 0.0;
    let cell_of = |p: &Point| -> [i64; 3] {
        let c = p.0;
        if exact {
            // Canonicalize -0.0 to 0.0: equal values, one key.
            let bits = |x: f64| i64::from_ne_bytes((x + 0.0).to_bits().to_ne_bytes());
            [bits(c.x), bits(c.y), bits(c.z)]
        } else {
            // `as i64` saturates: a coordinate beyond ±2^63 cells lands in
            // the edge cell with its neighbours — a coarser filter there,
            // never a missed duplicate.
            #[allow(clippy::cast_possible_truncation)]
            let cell = |x: f64| (x / tolerance).floor() as i64;
            [cell(c.x), cell(c.y), cell(c.z)]
        }
    };
    let offsets: &[[i64; 3]] = if exact { &[[0, 0, 0]] } else { &NEIGHBOURS };

    let mut kept: Vec<Point> = Vec::new();
    let mut map: Vec<u64> = Vec::new();
    // Kept-point indices per occupied cell.
    let mut cells: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    for (source, point) in input.points.iter().enumerate() {
        let cell = cell_of(point);
        let duplicate = offsets.iter().any(|offset| {
            let key = [
                cell[0].saturating_add(offset[0]),
                cell[1].saturating_add(offset[1]),
                cell[2].saturating_add(offset[2]),
            ];
            cells.get(&key).is_some_and(|ids| {
                ids.iter()
                    .any(|&k| tol::coincident(kept[k], *point, tolerance))
            })
        });
        if !duplicate {
            cells.entry(cell).or_default().push(kept.len());
            kept.push(*point);
            map.push(source as u64);
        }
    }
    CullDuplicatesOut {
        points: kept,
        map: IndexMap(map),
    }
}

/// The 27 cell offsets around and including a cell.
const NEIGHBOURS: [[i64; 3]; 27] = {
    let mut out = [[0i64; 3]; 27];
    let mut n = 0usize;
    let mut dz = -1i64;
    while dz <= 1 {
        let mut dy = -1i64;
        while dy <= 1 {
            let mut dx = -1i64;
            while dx <= 1 {
                out[n] = [dx, dy, dz];
                n += 1;
                dx += 1;
            }
            dy += 1;
        }
        dz += 1;
    }
    out
};

#[cfg(test)]
#[allow(clippy::float_cmp)] // kept points pass through exactly
mod tests {
    use super::*;
    use crate::points::support::testing::hex;

    fn cull(points: Vec<Point>, tolerance: f64) -> CullDuplicatesOut {
        cull_duplicates(CullDuplicatesIn { points, tolerance })
    }

    fn line(xs: &[f64]) -> Vec<Point> {
        xs.iter().map(|&x| Point::new(x, 0.0, 0.0)).collect()
    }

    #[test]
    fn cull_duplicates_table() {
        // Exact duplicates at tolerance 0, first occurrence kept, in order.
        let out = cull(line(&[0.0, 1.0, 0.0, 2.0, 1.0]), 0.0);
        assert_eq!(out.points, line(&[0.0, 1.0, 2.0]));
        assert_eq!(out.map, IndexMap(vec![0, 1, 3]));
        // Near duplicates at a positive tolerance (inclusive at the bound).
        let near = cull(line(&[0.0, 1.0, 1.0005, 5.0, 0.001]), 0.001);
        assert_eq!(near.points, line(&[0.0, 1.0, 5.0]));
        assert_eq!(near.map, IndexMap(vec![0, 1, 3]));
        // The first-kept rule: 0.0 kept, 0.0008 dropped (within 0.001 of
        // 0.0), 0.0016 KEPT — it is within 0.001 of the dropped 0.0008 but
        // not of the kept 0.0.
        let chain = cull(line(&[0.0, 0.0008, 0.0016]), 0.001);
        assert_eq!(chain.map, IndexMap(vec![0, 2]));
        // Duplicates across cell boundaries are still found (0.999 and
        // 1.001 straddle the cell edge at 1.0 with tolerance 0.01).
        let straddle = cull(line(&[0.999, 1.001]), 0.01);
        assert_eq!(straddle.map, IndexMap(vec![0]));
        // -0.0 and 0.0 are the same point at tolerance 0.
        let zeros = cull(
            vec![Point::new(0.0, 0.0, 0.0), Point::new(-0.0, 0.0, -0.0)],
            0.0,
        );
        assert_eq!(zeros.map, IndexMap(vec![0]));
        // 3-D: the distance is Euclidean, not per-axis.
        let diagonal = cull(
            vec![
                Point::origin(),
                Point::new(0.6, 0.6, 0.6),
                Point::new(0.5, 0.5, 0.5),
            ],
            1.0,
        );
        assert_eq!(diagonal.map, IndexMap(vec![0, 1]));
        // Empty in, empty out; no duplicates, everything kept.
        assert!(cull(vec![], 0.5).points.is_empty());
        let all = cull(line(&[0.0, 1.0, 2.0]), 0.5);
        assert_eq!(all.map, IndexMap(vec![0, 1, 2]));
    }

    #[test]
    #[should_panic(expected = "tolerance must be >= 0")]
    fn cull_duplicates_negative_tolerance_is_red() {
        let _ = cull(line(&[0.0, 1.0]), -1e-9);
    }

    proptest::proptest! {
        // Survivors are pairwise further apart than the tolerance, the map
        // is strictly increasing and points at the sources exactly, and
        // every dropped point is within tolerance of some survivor.
        #[test]
        fn property_cull_duplicates_is_a_tolerance_partition(
            coords in proptest::collection::vec((-5.0..5.0_f64, -5.0..5.0_f64, -5.0..5.0_f64), 0..60),
            tolerance in 0.0..2.0_f64,
        ) {
            let points: Vec<Point> = coords.iter().map(|&(x, y, z)| Point::new(x, y, z)).collect();
            let out = cull(points.clone(), tolerance);
            let map = &out.map.0;
            proptest::prop_assert_eq!(out.points.len(), map.len());
            for (i, &source) in map.iter().enumerate() {
                let source = usize::try_from(source).unwrap();
                proptest::prop_assert_eq!(out.points[i], points[source]);
                if i > 0 {
                    proptest::prop_assert!(map[i - 1] < map[i]);
                }
            }
            for (i, a) in out.points.iter().enumerate() {
                for b in &out.points[i + 1..] {
                    proptest::prop_assert!(!tol::coincident(*a, *b, tolerance));
                }
            }
            for (source, p) in points.iter().enumerate() {
                let kept = map.contains(&(source as u64));
                let represented = out.points.iter().any(|k| tol::coincident(*k, *p, tolerance));
                proptest::prop_assert!(kept || represented);
            }
        }
    }

    // Golden hashes: the kept list and the map through the value model,
    // exact coordinates (blessed via run-once).
    #[test]
    fn cull_duplicates_determinism_golden_hash() {
        let out = cull(
            vec![
                Point::new(0.0, 0.0, 0.0),
                Point::new(1.0, 2.0, 3.0),
                Point::new(0.0, 0.0, 0.0),
                Point::new(1.0, 2.0, 3.0001),
                Point::new(-4.0, 0.5, 0.25),
            ],
            0.001,
        );
        assert_eq!(
            [hex(out.points), hex(out.map)],
            [
                "0d3cfb4c7e0f4195c873ee1e633da8fd07a344d39814a63cd20dc054a127220f",
                "7acc59ac5e55b39f610aae330debe9841783e0cfc46ada526523d660e9c12cbd",
            ]
        );
    }
}
