//! The `move` node (`fn move_` — the dialect name is the keyword `move`).

use cicada_core::geometry::Transformable;
use cicada_core::spatial::Vector;
use cicada_geom::transform::Similarity;
use cicada_macros::{Ports, node};

/// Inputs for [`move_`].
#[derive(Ports, Clone, Debug)]
pub struct MoveIn {
    /// The geometry to move.
    pub geometry: Transformable,
    /// The translation.
    pub motion: Vector,
}

/// Move — translate geometry along a vector.
///
/// # Returns
///
/// The geometry translated by `motion`.
///
/// # Examples
///
/// ```cic
/// corner = construct_point(x=1.0, y=2.0, z=3.0)
/// shift = unit_x(factor=10.0)
/// moved = move(geometry=corner, motion=shift)
/// ```
#[node(category = "Transform", tier = "S", version = 1, gh = "Move")]
#[must_use]
pub fn move_(input: MoveIn) -> Transformable {
    Similarity::translation(input.motion).apply(&input.geometry)
}

#[cfg(test)]
mod tests {
    use cicada_core::spatial::Point;
    use cicada_geom::tol;

    use super::*;
    use crate::transform::support::{expect_point, expect_point_hash, point};

    #[test]
    fn move_table_kind_preserving() {
        let moved = move_(MoveIn {
            geometry: point(1.0, 2.0, 3.0),
            motion: Vector::new(10.0, 0.0, -1.0),
        });
        assert!(tol::coincident(
            expect_point(&moved),
            Point::new(11.0, 2.0, 2.0),
            1e-12
        ));
        // A vector moves nowhere (displacements ignore translation).
        let v = move_(MoveIn {
            geometry: Transformable::Vector(Vector::new(1.0, 0.0, 0.0)),
            motion: Vector::new(5.0, 5.0, 5.0),
        });
        assert_eq!(v, Transformable::Vector(Vector::new(1.0, 0.0, 0.0)));
    }

    proptest::proptest! {
        // move then move back is the identity (exact for translation).
        #[test]
        fn property_move_roundtrip(
            x in -1.0e6..1.0e6_f64, y in -1.0e6..1.0e6_f64,
            dx in -1.0e6..1.0e6_f64, dy in -1.0e6..1.0e6_f64,
        ) {
            let there = move_(MoveIn {
                geometry: point(x, y, 0.0),
                motion: Vector::new(dx, dy, 0.0),
            });
            let back = move_(MoveIn {
                geometry: there,
                motion: Vector::new(-dx, -dy, 0.0),
            });
            // f64 addition then subtraction can round; stay within one ulp
            // of the magnitudes involved.
            let got = expect_point(&back);
            let scale = x.abs().max(dx.abs()).max(1.0);
            proptest::prop_assert!((got.0.x - x).abs() <= 1e-9 * scale);
        }
    }

    #[test]
    fn move_determinism_golden_hash() {
        let moved = move_(MoveIn {
            geometry: point(1.0, 2.0, 3.0),
            motion: Vector::new(0.5, -0.5, 0.25),
        });
        assert_eq!(
            expect_point_hash(&moved),
            "a7db90a4e876014b114cd583946eedee36b32e54bbf54c09ae9450bb6451a286"
        );
    }
}
