//! The `construct_xform` node.

use cicada_core::spatial::Xform;
use cicada_geom::transform::Affine;
use cicada_macros::{Ports, node};

/// Inputs for [`construct_xform`].
#[derive(Ports, Clone, Debug)]
pub struct ConstructXformIn {
    /// Twelve numbers: the 3×4 affine matrix row by row, `[a, b, c, tx, d,
    /// e, f, ty, g, h, i, tz]`, mapping `(x, y, z)` to `(ax + by + cz + tx,
    /// dx + ey + fz + ty, gx + hy + iz + tz)` — the identity is `[1, 0, 0,
    /// 0, 0, 1, 0, 0, 0, 0, 1, 0]`.
    pub rows: Vec<f64>,
}

/// Construct Xform — an affine transform from its 3×4 matrix, row by row.
///
/// The one way to write a transform down in the text: a translation puts
/// the motion in the fourth column, a scale its factors on the diagonal, a
/// rotation its cosines and sines in the 3×3 block; any affine, a shear
/// included, is admitted here — `transform` decides per kind what it can
/// carry exactly. `compose_xform` multiplies them.
///
/// # Returns
///
/// The transform the twelve numbers spell.
///
/// # Panics
///
/// Panics when `rows` has not exactly twelve numbers, or one of them is not
/// finite.
///
/// # Examples
///
/// ```cic
/// shift = construct_xform(rows=[1.0, 0.0, 0.0, 5.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0])
/// corner = construct_point(x=1.0, y=0.0, z=0.0)
/// moved = transform(geometry=corner, xform=shift)
/// ```
#[node(
    category = "Transform",
    tier = "1",
    version = 1,
    gh = "Construct Matrix"
)]
#[must_use]
pub fn construct_xform(input: ConstructXformIn) -> Xform {
    let rows: [f64; 12] = input.rows.as_slice().try_into().unwrap_or_else(|_| {
        panic!(
            "construct_xform: rows has {} number(s) — a transform is its 3×4 matrix, twelve \
             numbers row by row",
            input.rows.len()
        )
    });
    for (index, value) in rows.iter().enumerate() {
        assert!(
            value.is_finite(),
            "construct_xform: rows[{index}] = {value} is not finite"
        );
    }
    Affine::from_rows(&rows).xform()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact: the matrix is stored as written
mod tests {
    use cicada_core::spatial::Point;
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;

    const IDENTITY: [f64; 12] = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0,
    ];

    #[test]
    fn construct_xform_table() {
        assert_eq!(
            construct_xform(ConstructXformIn {
                rows: IDENTITY.to_vec()
            }),
            Xform::identity()
        );
        // The rows apply the written map: a translation, a scale, a shear.
        let shift = construct_xform(ConstructXformIn {
            rows: vec![
                1.0, 0.0, 0.0, 5.0, //
                0.0, 1.0, 0.0, -1.0, //
                0.0, 0.0, 1.0, 0.5,
            ],
        });
        assert_eq!(
            Affine::from_xform(&shift).apply_point(Point::new(1.0, 2.0, 3.0)),
            Point::new(6.0, 1.0, 3.5)
        );
        let stretch = construct_xform(ConstructXformIn {
            rows: vec![
                2.0, 0.0, 0.0, 0.0, //
                0.0, 3.0, 0.0, 0.0, //
                0.0, 0.0, 4.0, 0.0,
            ],
        });
        assert_eq!(
            Affine::from_xform(&stretch).apply_point(Point::new(1.0, 1.0, 1.0)),
            Point::new(2.0, 3.0, 4.0)
        );
        let shear = construct_xform(ConstructXformIn {
            rows: vec![
                1.0, 0.5, 0.0, 0.0, //
                0.0, 1.0, 0.0, 0.0, //
                0.0, 0.0, 1.0, 0.0,
            ],
        });
        assert_eq!(
            Affine::from_xform(&shear).apply_point(Point::new(0.0, 2.0, 0.0)),
            Point::new(1.0, 2.0, 0.0)
        );
        // And read back as written.
        assert_eq!(
            Affine::from_xform(&shear).rows(),
            [
                1.0, 0.5, 0.0, 0.0, //
                0.0, 1.0, 0.0, 0.0, //
                0.0, 0.0, 1.0, 0.0,
            ]
        );
    }

    #[test]
    #[should_panic(expected = "rows has 9 number(s) — a transform is its 3×4 matrix")]
    fn construct_xform_wrong_count_is_red() {
        let _ = construct_xform(ConstructXformIn {
            rows: vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        });
    }

    #[test]
    #[should_panic(expected = "rows[3] = inf is not finite")]
    fn construct_xform_non_finite_is_red() {
        let mut rows = IDENTITY.to_vec();
        rows[3] = f64::INFINITY;
        let _ = construct_xform(ConstructXformIn { rows });
    }

    proptest::proptest! {
        // Whatever twelve finite numbers are written come back, in order,
        // from the transform's rows — the matrix is stored as written.
        #[test]
        fn property_rows_round_trip(rows in proptest::array::uniform12(-1.0e6f64..1.0e6)) {
            let out = construct_xform(ConstructXformIn { rows: rows.to_vec() });
            proptest::prop_assert_eq!(Affine::from_xform(&out).rows(), rows);
        }
    }

    #[test]
    fn construct_xform_determinism_golden_hash() {
        let out = construct_xform(ConstructXformIn {
            rows: vec![
                2.0, 0.0, 0.0, 5.0, //
                0.0, 0.5, 0.0, -1.0, //
                0.0, 0.0, 1.0, 0.25,
            ],
        });
        assert_eq!(
            HashedValue::new(ValueData::Xform(out))
                .unwrap()
                .hash()
                .to_hex(),
            "695e623ffb981ec19ef575eff47e9b738cb500fca66c15d388d71f4b3d6b56ff"
        );
    }
}
