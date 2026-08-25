//! The `rectangular_array` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Transformable;
use cicada_core::spatial::{Plane, Vector};
use cicada_geom::frame::orthonormal;
use cicada_geom::transform::Similarity;
use cicada_macros::{Ports, node};

use super::support::payload_bytes;
use crate::{checked_floor, checked_product, red};

/// Inputs for [`rectangular_array`].
#[derive(Ports, Clone, Debug)]
pub struct RectangularArrayIn {
    /// The geometry to repeat.
    pub geometry: Transformable,
    /// The array's frame: the copies step along its x, y and z axes.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// The cell: its x, y and z components are the steps between copies
    /// along the plane's x, y and z axes (Grasshopper's Cell box, reduced
    /// to its three extents; a negative step runs the other way).
    pub cell: Vector,
    /// Copies along the plane's x axis (the first at the original).
    pub x_count: i64,
    /// Copies along the plane's y axis.
    pub y_count: i64,
    /// Copies along the plane's z axis (layers).
    #[port(default = 1)]
    pub z_count: i64,
}

/// Rectangular Array — a grid of copies stepped along a plane's axes.
///
/// `x_count × y_count × z_count` copies, the original first, ordered x
/// fastest, then y, then z (`index = (k × y_count + j) × x_count + i`): a
/// row, then the next row, then the next layer. A Solid moves through the
/// kernel like every similarity.
///
/// # Returns
///
/// The copies, x fastest, then y, then z — the original first.
///
/// # Panics
///
/// Panics when a count is below 1, when the total is above the shared
/// ceilings (2^22 slots, or 1 GiB of copies — each copy costed as its slot
/// PLUS the mesh, polyline or solid it transforms, since every copy is a
/// distinct geometry), when the plane is degenerate, or for a `Solid` the
/// OCCT kernel refuses to transform (a `Solid` moves through the kernel —
/// its B-rep geometry is rewritten, never a mesh in disguise).
///
/// # Examples
///
/// ```cic
/// peg = construct_point(x=0.0, y=0.0, z=0.0)
/// pitch = construct_vector(x=3.0, y=2.0, z=1.0)
/// grid = rectangular_array(geometry=peg, cell=pitch, x_count=4, y_count=3)
/// ```
#[node(
    category = "Transform",
    tier = "1",
    version = 1,
    gh = "Rectangular Array",
    uses_tolerance
)]
#[must_use]
pub fn rectangular_array(config: &ProjectConfig, input: RectangularArrayIn) -> Vec<Transformable> {
    // The floors per count, then the ceiling on the PRODUCT the node emits
    // (docs/08 rule 7: charged on what the value model will hash), every
    // copy costed with its payload like `linear_array`'s — before any copy
    // is built. The product is formed by `checked_product`: three counts
    // the dialect admits can multiply past u128::MAX, and that too is the
    // node's refusal, never rustc's overflow panic.
    let x = checked_floor("rectangular_array", "x_count", input.x_count, 1);
    let y = checked_floor("rectangular_array", "y_count", input.y_count, 1);
    let z = checked_floor("rectangular_array", "z_count", input.z_count, 1);
    let total = checked_product(
        "rectangular_array",
        &format!(
            "copies at {}={}, {}={}, {}={} (x × y × z)",
            "x_count", input.x_count, "y_count", input.y_count, "z_count", input.z_count
        ),
        &[x, y, z],
        size_of::<Transformable>() + payload_bytes(&input.geometry),
    );
    let frame = red(orthonormal(&input.plane, config.tol()));
    let step_x = frame.x * input.cell.0.x;
    let step_y = frame.y * input.cell.0.y;
    let step_z = frame.z * input.cell.0.z;
    let mut copies = Vec::with_capacity(total);
    for k in 0..z {
        for j in 0..y {
            for i in 0..x {
                #[allow(clippy::cast_precision_loss)] // counts stay below 2^22
                let motion = step_x * i as f64 + step_y * j as f64 + step_z * k as f64;
                copies.push(Similarity::translation(Vector(motion)).apply(&input.geometry));
            }
        }
    }
    copies
}

#[cfg(test)]
mod tests {
    use cicada_core::spatial::Point;
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::tol;

    use super::*;
    use crate::transform::support::{config, expect_point, point, strip_mesh};

    fn grid(cell: Vector, x_count: i64, y_count: i64, z_count: i64) -> Vec<Transformable> {
        rectangular_array(
            &config(),
            RectangularArrayIn {
                geometry: point(0.0, 0.0, 0.0),
                plane: Plane::world_xy(),
                cell,
                x_count,
                y_count,
                z_count,
            },
        )
    }

    #[test]
    fn rectangular_array_table() {
        // 3 × 2 × 2 in the world frame: x fastest, then y, then z.
        let copies = grid(Vector::new(1.0, 10.0, 100.0), 3, 2, 2);
        assert_eq!(copies.len(), 12);
        let want = |i: f64, j: f64, k: f64| Point::new(i, 10.0 * j, 100.0 * k);
        assert!(tol::coincident(
            expect_point(&copies[0]),
            want(0.0, 0.0, 0.0),
            1e-12
        ));
        assert!(tol::coincident(
            expect_point(&copies[1]),
            want(1.0, 0.0, 0.0),
            1e-12
        ));
        assert!(tol::coincident(
            expect_point(&copies[2]),
            want(2.0, 0.0, 0.0),
            1e-12
        ));
        assert!(tol::coincident(
            expect_point(&copies[3]),
            want(0.0, 1.0, 0.0),
            1e-12
        ));
        assert!(tol::coincident(
            expect_point(&copies[6]),
            want(0.0, 0.0, 1.0),
            1e-12
        ));
        assert!(tol::coincident(
            expect_point(&copies[11]),
            want(2.0, 1.0, 1.0),
            1e-12
        ));
        // A turned frame: the cell's x runs along world y, its y along world
        // z (exact axis permutation); a negative step runs the other way.
        let turned = rectangular_array(
            &config(),
            RectangularArrayIn {
                geometry: point(0.0, 0.0, 0.0),
                plane: Plane {
                    origin: Point::new(5.0, 5.0, 5.0),
                    x: Vector::new(0.0, 1.0, 0.0),
                    y: Vector::new(0.0, 0.0, 1.0),
                },
                cell: Vector::new(2.0, -3.0, 1.0),
                x_count: 2,
                y_count: 2,
                z_count: 1,
            },
        );
        assert!(tol::coincident(
            expect_point(&turned[1]),
            Point::new(0.0, 2.0, 0.0),
            1e-12
        ));
        assert!(tol::coincident(
            expect_point(&turned[2]),
            Point::new(0.0, 0.0, -3.0),
            1e-12
        ));
        // The frame's origin is immaterial to a translation array.
        assert!(tol::coincident(
            expect_point(&turned[0]),
            Point::origin(),
            1e-12
        ));
        // A single copy.
        assert_eq!(grid(Vector::new(1.0, 1.0, 1.0), 1, 1, 1).len(), 1);
    }

    #[test]
    #[should_panic(expected = "y_count must be >= 1, got 0")]
    fn rectangular_array_zero_count_is_red() {
        let _ = grid(Vector::new(1.0, 1.0, 1.0), 3, 0, 1);
    }

    // The absurd count a literal or an Integer wire can carry, on one axis:
    // 10^11 point copies is an 11 TB buffer no machine holds — with the
    // guard after the copies this test binary would abort on allocation
    // failure (`catch_unwind` cannot catch that), so passing proves the
    // refusal precedes the allocation. The message names the product.
    #[test]
    #[should_panic(
        expected = "rectangular_array: copies at x_count=100000000000, y_count=1, z_count=1 (x × y × z) would be 100000000000 — above the 4194304 (2^22) slot ceiling"
    )]
    fn rectangular_array_absurd_count_is_refused_not_allocated() {
        let _ = grid(Vector::new(1.0, 1.0, 1.0), 100_000_000_000, 1, 1);
    }

    // The PRODUCT is what the ceiling sees: three counts each far under the
    // slot ceiling whose product is absurd (10^5 × 10^5 × 10) are refused
    // with the product named.
    #[test]
    #[should_panic(
        expected = "rectangular_array: copies at x_count=100000, y_count=100000, z_count=10 (x × y × z) would be 100000000000 — above the 4194304 (2^22) slot ceiling"
    )]
    fn rectangular_array_absurd_product_of_modest_counts_is_refused() {
        let _ = grid(Vector::new(1.0, 1.0, 1.0), 100_000, 100_000, 10);
    }

    // Three counts each under the dialect's 2^53 literal ceiling whose
    // product is past u128::MAX ((4 × 10^15)^3 = 6.4 × 10^46): the refusal
    // is the node's typed ceiling naming the counts — before this guard the
    // bare `x * y * z` was rustc's "attempt to multiply with overflow" (the
    // C2b review reproduced it on the engine; red either way, but the text
    // is the contract).
    #[test]
    #[should_panic(
        expected = "rectangular_array: copies at x_count=4000000000000000, y_count=4000000000000000, z_count=4000000000000000 (x × y × z) would be beyond 2^128 — above the 4194304 (2^22) slot ceiling"
    )]
    fn rectangular_array_product_past_u128_is_refused_with_the_ceiling_text() {
        let four_e15 = 4_000_000_000_000_000;
        let _ = grid(Vector::new(1.0, 1.0, 1.0), four_e15, four_e15, four_e15);
    }

    // A fat copy is charged its payload: a million-vertex strip is refused
    // at the first grid whose copies cross 1 GiB (30 copies — 5 × 6), where
    // the slot count alone would admit it.
    #[test]
    fn rectangular_array_fat_copies_are_refused_by_their_payload() {
        let mesh = strip_mesh(1_000_003);
        let per_copy =
            size_of::<Transformable>() + payload_bytes(&Transformable::Mesh(mesh.clone()));
        assert_eq!(usize::try_from(crate::MAX_BYTES).unwrap() / per_copy, 29);
        let panic = std::panic::catch_unwind(|| {
            rectangular_array(
                &config(),
                RectangularArrayIn {
                    geometry: Transformable::Mesh(mesh.clone()),
                    plane: Plane::world_xy(),
                    cell: Vector::new(1.0, 1.0, 1.0),
                    x_count: 5,
                    y_count: 6,
                    z_count: 1,
                },
            )
        })
        .expect_err("30 copies cross the byte ceiling");
        assert!(
            panic.downcast_ref::<String>().unwrap().contains(&format!(
                "would be 30 — {} bytes at {per_copy} bytes each",
                30 * per_copy
            )),
            "{panic:?}"
        );
        // Under the ceiling: distinct, stepped copies of the whole mesh.
        let two = rectangular_array(
            &config(),
            RectangularArrayIn {
                geometry: Transformable::Mesh(mesh),
                plane: Plane::world_xy(),
                cell: Vector::new(5.0, 0.0, 0.0),
                x_count: 2,
                y_count: 1,
                z_count: 1,
            },
        );
        let Transformable::Mesh(second) = &two[1] else {
            panic!("meshes stay meshes")
        };
        assert!((second.positions()[0] - 5.0).abs() < 1e-12);
    }

    proptest::proptest! {
        // Copy (i, j, k) sits at exactly i·cx x̂ + j·cy ŷ + k·cz ẑ, in the
        // documented order.
        #[test]
        fn property_rectangular_array_order_and_spacing(
            x_count in 1i64..6, y_count in 1i64..6, z_count in 1i64..4,
            cx in -10.0..10.0_f64, cy in -10.0..10.0_f64, cz in -10.0..10.0_f64,
        ) {
            let copies = grid(Vector::new(cx, cy, cz), x_count, y_count, z_count);
            proptest::prop_assert_eq!(copies.len(), usize::try_from(x_count * y_count * z_count).unwrap());
            for (index, copy) in copies.iter().enumerate() {
                let index = i64::try_from(index).unwrap();
                let (i, j, k) = (index % x_count, (index / x_count) % y_count, index / (x_count * y_count));
                #[allow(clippy::cast_precision_loss)]
                let want = Point::new(cx * i as f64, cy * j as f64, cz * k as f64);
                proptest::prop_assert!(tol::coincident(expect_point(copy), want, 1e-9 * want.0.length().max(1.0)));
            }
        }
    }

    #[test]
    fn rectangular_array_determinism_golden_hash() {
        // Dyadic steps, integer counts: pure arithmetic.
        let copies = rectangular_array(
            &config(),
            RectangularArrayIn {
                geometry: point(1.0, 2.0, 3.0),
                plane: Plane::world_xy(),
                cell: Vector::new(0.5, -0.25, 2.0),
                x_count: 2,
                y_count: 2,
                z_count: 2,
            },
        );
        let slots = copies
            .into_iter()
            .map(|copy| {
                let Transformable::Point(p) = copy else {
                    panic!("points stay points")
                };
                Some(HashedValue::new(ValueData::Point(p)).unwrap())
            })
            .collect();
        let list = HashedValue::new(ValueData::List(cicada_core::value::List {
            axis: None,
            slots,
        }))
        .unwrap();
        assert_eq!(
            list.hash().to_hex(),
            "ac1241303bb2facfecc33816dfc4aad57921abc411a81c4c4a0ef4014d26a233"
        );
    }
}
