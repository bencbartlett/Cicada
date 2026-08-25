//! The `closest_point` node.

use cicada_core::spatial::Point;
use cicada_macros::{Ports, node};

/// Inputs for [`closest_point`].
#[derive(Ports, Clone, Debug)]
pub struct ClosestPointIn {
    /// The point to search from.
    pub point: Point,
    /// The points to search.
    pub cloud: Vec<Point>,
}

/// Outputs of [`closest_point`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct ClosestPointOut {
    /// The cloud point nearest to `point` (its coordinates exactly as they
    /// are in the cloud).
    pub closest: Point,
    /// Its index in the cloud; at a tie, the lowest index.
    pub index: i64,
    /// The distance from `point` to it.
    pub distance: f64,
}

/// Closest Point — the point of a cloud nearest to a point, with its index
/// and distance.
///
/// A flat scan of the cloud (one pass, no index structure): the node is
/// called once per query point, and building a tree per call would cost
/// more than the scan it replaces — a shared index belongs to a node that
/// takes the queries as a list.
///
/// # Panics
///
/// Panics when the cloud is empty — nothing is closest to anything.
///
/// # Examples
///
/// ```cic
/// xs = [0.0, 3.0, 6.0]
/// cloud = construct_point(x=each(xs), y=0.0, z=0.0)
/// probe = construct_point(x=2.0, y=1.0, z=0.0)
/// nearest, at, gap = closest_point(point=probe, cloud=cloud)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "1",
    version = 1,
    gh = "Closest Point"
)]
#[must_use]
pub fn closest_point(input: ClosestPointIn) -> ClosestPointOut {
    assert!(
        !input.cloud.is_empty(),
        "closest_point: the cloud is empty — nothing is closest to anything"
    );
    let mut best = (0usize, f64::INFINITY);
    for (index, candidate) in input.cloud.iter().enumerate() {
        let d2 = input.point.0.distance_squared(candidate.0);
        // A strict `<` is the tie rule (the lowest index wins) — a
        // min-reduction is an ordering, not a tolerance decision. Squared
        // distances of NaN-free points never compare unordered.
        if d2 < best.1 {
            best = (index, d2);
        }
    }
    let (index, d2) = best;
    #[allow(clippy::cast_possible_wrap)] // cloud lengths are far below i64::MAX
    let index_i64 = index as i64;
    ClosestPointOut {
        closest: input.cloud[index],
        index: index_i64,
        distance: d2.sqrt(),
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // the closest point's coordinates pass through exactly
mod tests {
    use cicada_geom::tol;

    use super::*;
    use crate::points::support::testing::hex;

    fn line(xs: &[f64]) -> Vec<Point> {
        xs.iter().map(|&x| Point::new(x, 0.0, 0.0)).collect()
    }

    #[test]
    fn closest_point_table() {
        let out = closest_point(ClosestPointIn {
            point: Point::new(2.0, 1.0, 0.0),
            cloud: line(&[0.0, 3.0, 6.0]),
        });
        assert_eq!(out.closest, Point::new(3.0, 0.0, 0.0));
        assert_eq!(out.index, 1);
        assert!(tol::close(out.distance, 2.0_f64.sqrt(), 1e-12));
        // A one-point cloud; the query on a cloud point (distance 0).
        let alone = closest_point(ClosestPointIn {
            point: Point::new(-5.0, 2.0, 9.0),
            cloud: vec![Point::new(1.0, 1.0, 1.0)],
        });
        assert_eq!(alone.index, 0);
        assert!(tol::close(alone.distance, 101.0_f64.sqrt(), 1e-12));
        let on_it = closest_point(ClosestPointIn {
            point: Point::new(3.0, 0.0, 0.0),
            cloud: line(&[0.0, 3.0, 6.0]),
        });
        assert_eq!(on_it.index, 1);
        assert!(tol::near_zero(on_it.distance, 0.0));
        // A tie goes to the lowest index; a later point at the same
        // distance never displaces it.
        let tie = closest_point(ClosestPointIn {
            point: Point::new(3.0, 0.0, 0.0),
            cloud: line(&[10.0, 1.0, 5.0, 1.0]),
        });
        assert_eq!(tie.index, 1);
        assert!(tol::close(tie.distance, 2.0, 1e-12));
    }

    #[test]
    #[should_panic(expected = "the cloud is empty")]
    fn closest_point_empty_cloud_is_red() {
        let _ = closest_point(ClosestPointIn {
            point: Point::origin(),
            cloud: vec![],
        });
    }

    proptest::proptest! {
        // The reported point IS the cloud point at the reported index, the
        // reported distance is its distance, and no cloud point is strictly
        // closer (at tolerance).
        #[test]
        fn property_closest_point_is_a_minimum(
            px in -100.0..100.0_f64, py in -100.0..100.0_f64, pz in -100.0..100.0_f64,
            coords in proptest::collection::vec((-100.0..100.0_f64, -100.0..100.0_f64, -100.0..100.0_f64), 1..40),
        ) {
            let point = Point::new(px, py, pz);
            let cloud: Vec<Point> = coords.iter().map(|&(x, y, z)| Point::new(x, y, z)).collect();
            let out = closest_point(ClosestPointIn { point, cloud: cloud.clone() });
            let index = usize::try_from(out.index).unwrap();
            proptest::prop_assert_eq!(out.closest, cloud[index]);
            proptest::prop_assert!(tol::close(out.distance, point.0.distance(cloud[index].0), 1e-12));
            for candidate in &cloud {
                proptest::prop_assert!(point.0.distance(candidate.0) >= out.distance - 1e-9);
            }
        }
    }

    // Golden hashes: each output through the value model on a 3-4-5 layout
    // (the distance's sqrt is exact), blessed via run-once.
    #[test]
    fn closest_point_determinism_golden_hash() {
        let out = closest_point(ClosestPointIn {
            point: Point::new(1.0, 1.0, 0.0),
            cloud: vec![
                Point::new(10.0, 10.0, 10.0),
                Point::new(4.0, 5.0, 0.0),
                Point::new(-7.0, 1.0, 0.0),
            ],
        });
        assert_eq!(
            [hex(out.closest), hex(out.index), hex(out.distance)],
            [
                "6dbc7d2d7558002dc0c7543b658b3f1e15af45df4ee2444e184fd78f4955306e",
                "7556f05f5590fddee7d554e9bb54bfa6db5857a5baaec76bddc00242a3e4e890",
                "94eef958b3fd1d43bbe6037ff14183719f8a823fbceb89128625befb93c6ca40",
            ]
        );
    }
}
