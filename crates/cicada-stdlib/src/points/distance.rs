//! The `distance` node.

use cicada_core::spatial::Point;
use cicada_macros::{Ports, node};

/// Inputs for [`distance`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct DistanceIn {
    /// First point.
    pub a: Point,
    /// Second point.
    pub b: Point,
}

/// Distance — the Euclidean distance between two points.
///
/// # Returns
///
/// The distance `|b − a|`, in document units.
///
/// # Examples
///
/// ```cic
/// here = construct_point(x=1.0, y=1.0, z=0.0)
/// there = construct_point(x=4.0, y=5.0, z=0.0)
/// gap = distance(a=here, b=there)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "1",
    version = 1,
    gh = "Distance"
)]
#[must_use]
pub fn distance(input: DistanceIn) -> f64 {
    input.a.0.distance(input.b.0)
}

#[cfg(test)]
mod tests {
    use cicada_geom::tol;

    use super::*;
    use crate::points::support::testing::hex;

    #[test]
    fn distance_table() {
        let d = |a, b| distance(DistanceIn { a, b });
        // A 3-4-5 triangle: sqrt(25) is exact, so the result is 5 to the bit
        // — asserted at tolerance all the same (geometry rule).
        assert!(tol::close(
            d(Point::new(1.0, 1.0, 0.0), Point::new(4.0, 5.0, 0.0)),
            5.0,
            1e-12
        ));
        assert!(tol::near_zero(
            d(Point::new(-2.5, 7.0, 0.25), Point::new(-2.5, 7.0, 0.25)),
            0.0
        ));
        assert!(tol::close(
            d(Point::new(-1.0, -1.0, -1.0), Point::new(1.0, 1.0, 1.0)),
            12.0_f64.sqrt(),
            1e-12
        ));
        // Along one axis the distance is the coordinate difference.
        assert!(tol::close(
            d(Point::new(0.0, 0.0, -3.0), Point::new(0.0, 0.0, 4.5)),
            7.5,
            1e-12
        ));
    }

    proptest::proptest! {
        // A metric: symmetric, non-negative, zero only at the same point,
        // and the triangle inequality holds (at tolerance — the sums round).
        #[test]
        fn property_distance_is_a_metric(
            ax in -1.0e3..1.0e3_f64, ay in -1.0e3..1.0e3_f64, az in -1.0e3..1.0e3_f64,
            bx in -1.0e3..1.0e3_f64, by in -1.0e3..1.0e3_f64, bz in -1.0e3..1.0e3_f64,
            cx in -1.0e3..1.0e3_f64, cy in -1.0e3..1.0e3_f64, cz in -1.0e3..1.0e3_f64,
        ) {
            let (a, b, c) = (
                Point::new(ax, ay, az),
                Point::new(bx, by, bz),
                Point::new(cx, cy, cz),
            );
            let d = |a, b| distance(DistanceIn { a, b });
            proptest::prop_assert!(tol::close(d(a, b), d(b, a), 1e-9));
            proptest::prop_assert!(d(a, b) >= 0.0);
            proptest::prop_assert!(tol::near_zero(d(a, a), 0.0));
            proptest::prop_assert!(d(a, c) <= d(a, b) + d(b, c) + 1e-9);
        }
    }

    // Golden hash: the 3-4-5 distance (exact sqrt), blessed via run-once.
    #[test]
    fn distance_determinism_golden_hash() {
        let d = distance(DistanceIn {
            a: Point::new(1.0, 1.0, 0.0),
            b: Point::new(4.0, 5.0, 0.0),
        });
        assert_eq!(
            hex(d),
            "94eef958b3fd1d43bbe6037ff14183719f8a823fbceb89128625befb93c6ca40"
        );
    }
}
