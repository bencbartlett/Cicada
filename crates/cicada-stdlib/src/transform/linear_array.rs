//! The `linear_array` node.

use cicada_core::geometry::Transformable;
use cicada_core::spatial::Vector;
use cicada_geom::transform::Similarity;
use cicada_macros::{Ports, node};

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
/// # Panics
///
/// Panics when `count < 1`.
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
    assert!(
        input.count >= 1,
        "linear_array: count must be >= 1, got {}",
        input.count
    );
    (0..input.count)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)] // counts far below 2^53
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
