//! The `linear_array` node.

use cicada_core::geometry::Transformable;
use cicada_core::spatial::Vector;
use cicada_geom::transform::Similarity;
use cicada_macros::{Ports, node};

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
/// (2^24 slots, or 1 GiB of copies up front — the message names the count
/// and the ceiling that bit; the copies' own geometry is not counted).
///
/// # Examples
///
/// ```cic
/// ring = circle(radius=1.0)
/// step = unit_x(factor=3.0)
/// row = linear_array(geometry=ring, direction=step, count=4)
/// ```
#[node(category = "Transform", tier = "S", version = 1, gh = "Linear Array")]
#[must_use]
pub fn linear_array(input: LinearArrayIn) -> Vec<Transformable> {
    // A copy is one `Transformable` slot up front (its geometry payload
    // comes behind it, per copy): the byte ceiling bites before the slot
    // ceiling here.
    let count = checked_count(
        "linear_array",
        "count",
        input.count,
        1,
        size_of::<Transformable>(),
    );
    (0..count)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)] // counts stay below 2^24
            let step = Vector(input.direction.0 * i as f64);
            Similarity::translation(step).apply(&input.geometry)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use cicada_core::spatial::Point;
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::tol;

    use super::*;
    use crate::transform::support::{expect_point, point};

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

    // A copy is a 112-byte `Transformable` slot, so the 1 GiB byte ceiling
    // bites before the 2^24 slot ceiling: one copy past it is red with the
    // count, the bytes and the ceiling in the message — and no allocation
    // (the count would be a 1 GiB Vec).
    #[test]
    fn linear_array_one_copy_past_the_byte_ceiling_is_refused_not_allocated() {
        let slot = size_of::<Transformable>();
        let cap = usize::try_from(crate::MAX_BYTES).unwrap() / slot;
        assert!(
            i64::try_from(cap).unwrap() < crate::MAX_SLOTS,
            "the byte ceiling is the one that bites for copies"
        );
        let count = i64::try_from(cap + 1).unwrap();
        let panic = std::panic::catch_unwind(|| {
            linear_array(LinearArrayIn {
                geometry: point(0.0, 0.0, 0.0),
                direction: Vector::new(1.0, 0.0, 0.0),
                count,
            })
        })
        .expect_err("one copy past the byte ceiling refuses");
        let message = panic.downcast_ref::<String>().unwrap();
        assert_eq!(
            *message,
            format!(
                "linear_array: count is {count} — {} bytes at {slot} bytes a slot, above the \
                 1073741824-byte (1 GiB) ceiling of one node allocation",
                (cap + 1) * slot
            )
        );
    }

    #[test]
    #[should_panic(
        expected = "linear_array: count is 100000000000 — above the 16777216 (2^24) slot ceiling"
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
