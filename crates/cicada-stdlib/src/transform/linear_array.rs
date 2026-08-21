//! The `linear_array` node.

use cicada_core::geometry::Transformable;
use cicada_core::spatial::Vector;
use cicada_geom::transform::Similarity;
use cicada_macros::{Ports, node};

use super::support::payload_bytes;
use crate::checked_count;

/// Inputs for [`linear_array`].
#[derive(Ports, Clone, Debug)]
pub struct LinearArrayIn {
    /// The geometry to repeat.
    pub geometry: Transformable,
    /// Step between consecutive copies.
    pub direction: Vector,
    /// Number of copies (the first sits at the original position).
    pub count: i64,
}

/// Linear Array — `count` copies stepped along a direction, the first at
/// the original position.
///
/// # Returns
///
/// The `count` copies, the original first, each stepped by `direction`.
///
/// # Panics
///
/// Panics when `count < 1`, or when `count` is above the shared ceilings
/// (2^22 slots, or 1 GiB of copies — each copy costed as its slot PLUS the
/// mesh or polyline it transforms, since every copy is a distinct
/// geometry: a million-vertex mesh, 36 MB, is refused at 30 copies; the
/// message names the count, the bytes and the ceiling that bit).
///
/// # Examples
///
/// ```cic
/// ring = circle(radius=1.0)
/// step = unit_x(factor=3.0)
/// row = linear_array(geometry=ring, direction=step, count=4)
/// ```
#[node(category = "Transform", tier = "S", version = 2, gh = "Linear Array")]
#[must_use]
pub fn linear_array(input: LinearArrayIn) -> Vec<Transformable> {
    // A copy costs its `Transformable` slot AND the geometry it transforms:
    // every copy is a fresh mesh or polyline (nothing is shared), so the
    // byte ceiling is charged per copy with the payload — the slot alone
    // admitted millions of copies of a mesh the machine could not hold.
    let count = checked_count(
        "linear_array",
        "count",
        input.count,
        1,
        size_of::<Transformable>() + payload_bytes(&input.geometry),
    );
    (0..count)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)] // counts stay below 2^22
            let step = Vector(input.direction.0 * i as f64);
            Similarity::translation(step).apply(&input.geometry)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::{Circle, Curve, Polyline};
    use cicada_core::spatial::{Plane, Point};
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::tol;

    use super::*;
    use crate::transform::support::{expect_point, point, strip_mesh};

    #[test]
    fn linear_array_table() {
        let row = linear_array(LinearArrayIn {
            geometry: point(0.0, 0.0, 0.0),
            direction: Vector::new(2.0, 0.0, 0.0),
            count: 3,
        });
        assert_eq!(row.len(), 3);
        assert!(tol::coincident(
            expect_point(&row[0]),
            Point::origin(),
            1e-12
        ));
        assert!(tol::coincident(
            expect_point(&row[2]),
            Point::new(4.0, 0.0, 0.0),
            1e-12
        ));
    }

    #[test]
    #[should_panic(expected = "count must be >= 1")]
    fn linear_array_zero_count_is_red() {
        let _ = linear_array(LinearArrayIn {
            geometry: point(0.0, 0.0, 0.0),
            direction: Vector::new(1.0, 0.0, 0.0),
            count: 0,
        });
    }

    // A thin copy (a point) is its `Transformable` slot alone: the slot
    // ceiling bites first (2^22 × 112 bytes is 448 MiB) — one past it is
    // red with the count and the ceiling, and no allocation.
    #[test]
    fn linear_array_one_copy_past_the_slot_ceiling_is_refused_not_allocated() {
        let count = crate::MAX_SLOTS + 1;
        let panic = std::panic::catch_unwind(|| {
            linear_array(LinearArrayIn {
                geometry: point(0.0, 0.0, 0.0),
                direction: Vector::new(1.0, 0.0, 0.0),
                count,
            })
        })
        .expect_err("one copy past the slot ceiling refuses");
        assert_eq!(
            *panic.downcast_ref::<String>().unwrap(),
            format!(
                "linear_array: count is {count} — above the 4194304 (2^22) slot ceiling of one \
                 node output"
            )
        );
        // And the slot ceiling itself is under the byte ceiling for a thin
        // copy: the byte half never bites a point.
        assert!(
            u64::try_from(crate::MAX_SLOTS).unwrap() * size_of::<Transformable>() as u64
                <= crate::MAX_BYTES
        );
    }

    // A fat copy is charged what it allocates: a 1,000,003-vertex strip
    // (24 MB of positions, 12 MB of indices) is refused at the first count
    // whose copies cross 1 GiB — 30 — with the count, the per-copy bytes
    // and the ceiling in the message, before a single copy is transformed
    // (the old guard counted 112 bytes a copy and admitted 9.5 million;
    // v0.1 follow-up 2 review measured 3.5 GB committed at count=100).
    #[test]
    fn linear_array_fat_copies_are_refused_by_their_payload_not_allocated() {
        let mesh = strip_mesh(1_000_003);
        let payload = payload_bytes(&Transformable::Mesh(mesh.clone()));
        assert_eq!(payload, 1_000_003 * 24 + 1_000_001 * 12);
        let per_copy = size_of::<Transformable>() + payload;
        let last_allowed = usize::try_from(crate::MAX_BYTES).unwrap() / per_copy;
        assert_eq!(last_allowed, 29);
        let count = i64::try_from(last_allowed + 1).unwrap();
        let panic = std::panic::catch_unwind(|| {
            linear_array(LinearArrayIn {
                geometry: Transformable::Mesh(mesh.clone()),
                direction: Vector::new(1.0, 0.0, 0.0),
                count,
            })
        })
        .expect_err("the copies' payload crosses the byte ceiling");
        assert_eq!(
            *panic.downcast_ref::<String>().unwrap(),
            format!(
                "linear_array: count is {count} — {} bytes at {per_copy} bytes a slot, above \
                 the 1073741824-byte (1 GiB) ceiling of one node allocation",
                (last_allowed + 1) * per_copy
            )
        );
        // Under the ceiling the node builds what it always built: distinct,
        // stepped copies of the whole mesh.
        let two = linear_array(LinearArrayIn {
            geometry: Transformable::Mesh(mesh),
            direction: Vector::new(5.0, 0.0, 0.0),
            count: 2,
        });
        assert_eq!(two.len(), 2);
        let Transformable::Mesh(second) = &two[1] else {
            panic!("meshes stay meshes")
        };
        assert_eq!(second.vertex_count(), 1_000_003);
        assert!((second.positions()[0] - 5.0).abs() < 1e-12);
    }

    // A polyline copy is charged its vertices, the analytic kinds nothing:
    // the same count of a circle is admitted where the polyline is refused.
    #[test]
    fn linear_array_charges_polyline_vertices_and_not_analytic_curves() {
        let vertices = 2_000_000;
        let polyline = Transformable::Curve(Curve::Polyline(Polyline {
            vertices: (0..vertices)
                .map(|i| Point::new(f64::from(i), 0.0, 0.0))
                .collect(),
            closed: false,
        }));
        let per_copy = size_of::<Transformable>() + usize::try_from(vertices).unwrap() * 24;
        let count =
            i64::try_from(usize::try_from(crate::MAX_BYTES).unwrap() / per_copy + 1).unwrap();
        assert_eq!(count, 23);
        let panic = std::panic::catch_unwind(|| {
            linear_array(LinearArrayIn {
                geometry: polyline,
                direction: Vector::new(1.0, 0.0, 0.0),
                count,
            })
        })
        .expect_err("the polyline copies cross the byte ceiling");
        assert!(panic.downcast_ref::<String>().unwrap().contains(&format!(
            "count is {count} — {} bytes at {per_copy} bytes a slot",
            { usize::try_from(count).unwrap() * per_copy }
        )),);
        let circles = linear_array(LinearArrayIn {
            geometry: Transformable::Curve(Curve::Circle(Circle {
                plane: Plane::world_xy(),
                radius: 1.0,
            })),
            direction: Vector::new(1.0, 0.0, 0.0),
            count,
        });
        assert_eq!(circles.len(), 23);
    }

    #[test]
    #[should_panic(
        expected = "linear_array: count is 100000000000 — above the 4194304 (2^22) slot ceiling"
    )]
    fn linear_array_absurd_count_is_refused_not_allocated() {
        let _ = linear_array(LinearArrayIn {
            geometry: point(0.0, 0.0, 0.0),
            direction: Vector::new(1.0, 0.0, 0.0),
            count: 100_000_000_000,
        });
    }

    proptest::proptest! {
        // linear_array copy i sits at exactly i × direction.
        #[test]
        fn property_linear_array_spacing(count in 1i64..40, step in -1.0e3..1.0e3_f64) {
            let row = linear_array(LinearArrayIn {
                geometry: point(0.0, 0.0, 0.0),
                direction: Vector::new(step, 0.0, 0.0),
                count,
            });
            for (i, copy) in row.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let want = step * i as f64;
                proptest::prop_assert!((expect_point(copy).0.x - want).abs() <= 1e-12 * want.abs().max(1.0));
            }
        }
    }

    #[test]
    fn linear_array_determinism_golden_hash() {
        let row = linear_array(LinearArrayIn {
            geometry: point(1.0, 2.0, 3.0),
            direction: Vector::new(0.5, 0.25, -1.0),
            count: 3,
        });
        let slots = row
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
            "c3a55cca187973910c5073a8655d140f5347c112a34f1c91e04a05cc43a39753"
        );
    }
}
