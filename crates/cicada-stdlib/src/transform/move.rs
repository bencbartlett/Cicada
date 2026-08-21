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
/// # Panics
///
/// Panics when the geometry is a `Solid` the OCCT kernel refuses to transform (a
/// `Solid` moves through the kernel — its B-rep geometry is rewritten, never a
/// mesh in disguise).
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

    #[test]
    fn a_solid_moves_through_the_kernel_or_is_the_documented_red_path() {
        // The same `Similarity` path serves rotate / scale / orient /
        // linear_array; this pins it at the NODE for one of them. With the
        // kernel a real box moves (its bounds shift, its volume stays);
        // without it the node is red with the typed refusal — both worlds
        // asserted, never a vacuous pass (docs/14).
        use cicada_core::geometry::{SOLID_CANONICAL_HEADER, Solid};
        use cicada_core::scalar::Domain;
        use cicada_core::spatial::Plane;
        if cicada_geom::solid::kernel_available() {
            let block = cicada_geom::solid::box_in_plane(
                &Plane::world_xy(),
                Domain::new(0.0, 1.0),
                Domain::new(0.0, 2.0),
                Domain::new(0.0, 3.0),
                1e-6,
            )
            .unwrap();
            let moved = move_(MoveIn {
                geometry: Transformable::Solid(block),
                motion: Vector::new(10.0, 0.0, -1.0),
            });
            let Transformable::Solid(moved) = moved else {
                panic!("a Solid stays a Solid")
            };
            let (min, max) = cicada_geom::solid::bounds(&moved).unwrap();
            assert!(tol::coincident(min, Point::new(10.0, 0.0, -1.0), 1e-9));
            assert!(tol::coincident(max, Point::new(11.0, 2.0, 2.0), 1e-9));
            let volume = cicada_geom::solid::volume(&moved).unwrap().volume;
            assert!((volume - 6.0).abs() < 1e-9, "{volume}");
        } else {
            let solid = Solid::from_canonical_bytes(SOLID_CANONICAL_HEADER.to_vec()).unwrap();
            let outcome = std::panic::catch_unwind(|| {
                move_(MoveIn {
                    geometry: Transformable::Solid(solid),
                    motion: Vector::new(1.0, 0.0, 0.0),
                })
            });
            let payload = outcome.expect_err("a Solid must be refused, never passed through");
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .expect("a message");
            assert!(message.contains("needs the OCCT kernel"), "{message}");
        }
    }
}
