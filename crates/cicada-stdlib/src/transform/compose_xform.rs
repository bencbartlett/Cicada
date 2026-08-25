//! The `compose_xform` node.

use cicada_core::spatial::Xform;
use cicada_geom::transform::Affine;
use cicada_macros::{Ports, node};

/// Inputs for [`compose_xform`].
#[derive(Ports, Clone, Debug)]
pub struct ComposeXformIn {
    /// The transforms, applied in list order: the first acts first, the
    /// last acts last.
    pub xforms: Vec<Xform>,
}

/// Compose Xform — one transform equal to applying a list of transforms in
/// order.
///
/// `transform(geometry, compose_xform([a, b]))` is `transform(transform(
/// geometry, a), b)`: `a` first, then `b`. An empty list composes to the
/// identity. Total — every `Xform` is finite by construction.
///
/// # Returns
///
/// The product of the transforms, the first applied first.
///
/// # Examples
///
/// ```cic
/// shift = construct_xform(rows=[1.0, 0.0, 0.0, 5.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0])
/// double = construct_xform(rows=[2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0])
/// first = duplicate(item=shift, count=1)
/// second = duplicate(item=double, count=1)
/// both = concat(a=first, b=second)
/// shift_then_double = compose_xform(xforms=both)
/// corner = construct_point(x=1.0, y=0.0, z=0.0)
/// placed = transform(geometry=corner, xform=shift_then_double)
/// ```
#[node(category = "Transform", tier = "1", version = 1, gh = "Compound")]
#[must_use]
pub fn compose_xform(input: ComposeXformIn) -> Xform {
    input
        .xforms
        .iter()
        .fold(Affine::identity(), |so_far, next| {
            so_far.then(&Affine::from_xform(next))
        })
        .xform()
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact: identity rows, dyadic factors, integer offsets
mod tests {
    use cicada_core::spatial::{Point, Vector};
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::transform::Similarity;

    use super::*;

    fn translation(x: f64, y: f64, z: f64) -> Xform {
        Similarity::translation(Vector::new(x, y, z)).xform()
    }

    fn scale(factor: f64) -> Xform {
        Similarity::scale_about(Point::origin(), factor).xform()
    }

    #[test]
    fn compose_xform_table() {
        // Empty: the identity.
        assert_eq!(
            compose_xform(ComposeXformIn { xforms: vec![] }),
            Xform::identity()
        );
        // One: itself.
        let shift = translation(1.0, 2.0, 3.0);
        assert_eq!(
            compose_xform(ComposeXformIn {
                xforms: vec![shift]
            }),
            shift
        );
        // Order matters: shift then double moves the origin to (2, 4, 6);
        // double then shift to (1, 2, 3).
        let shift_then_double = compose_xform(ComposeXformIn {
            xforms: vec![shift, scale(2.0)],
        });
        assert_eq!(
            Affine::from_xform(&shift_then_double).apply_point(Point::origin()),
            Point::new(2.0, 4.0, 6.0)
        );
        let double_then_shift = compose_xform(ComposeXformIn {
            xforms: vec![scale(2.0), shift],
        });
        assert_eq!(
            Affine::from_xform(&double_then_shift).apply_point(Point::origin()),
            Point::new(1.0, 2.0, 3.0)
        );
        // Two translations add.
        assert_eq!(
            compose_xform(ComposeXformIn {
                xforms: vec![shift, translation(-1.0, 0.0, 1.0)],
            }),
            translation(0.0, 2.0, 4.0)
        );
    }

    proptest::proptest! {
        // The composite of a list applied to a point equals applying the
        // list's transforms one after another, for random affines.
        #[test]
        fn property_compose_is_sequential_application(
            rows in proptest::collection::vec(proptest::array::uniform12(-2.0f64..2.0), 0..5),
            px in -5.0..5.0_f64, py in -5.0..5.0_f64, pz in -5.0..5.0_f64,
        ) {
            let xforms: Vec<Xform> = rows.iter().map(|r| Affine::from_rows(r).xform()).collect();
            let composite = Affine::from_xform(&compose_xform(ComposeXformIn { xforms: xforms.clone() }));
            let mut p = Point::new(px, py, pz);
            for x in &xforms {
                p = Affine::from_xform(x).apply_point(p);
            }
            let once = composite.apply_point(Point::new(px, py, pz));
            proptest::prop_assert!(
                cicada_geom::tol::coincident(once, p, 1e-9 * p.0.length().max(1.0)),
                "{once:?} vs {p:?}"
            );
        }
    }

    // The Grasshopper name feeds search-to-place for GH migrants: the GH
    // component that multiplies transforms is Transform > Util > Compound.
    // `Compose` — the name C2b first shipped — is no GH component, so a GH
    // user typing `compound` found nothing (the C2b review's finding).
    #[test]
    fn compose_xform_answers_to_grasshoppers_compound() {
        let spec = crate::registry()
            .iter()
            .find(|s| s.name == "compose_xform")
            .expect("compose_xform registered");
        assert_eq!(spec.gh, Some("Compound"));
    }

    #[test]
    fn compose_xform_determinism_golden_hash() {
        // Dyadic factors and integer offsets: pure arithmetic.
        let out = compose_xform(ComposeXformIn {
            xforms: vec![
                translation(1.0, 2.0, 3.0),
                scale(0.5),
                translation(-4.0, 0.0, 8.0),
            ],
        });
        assert_eq!(
            HashedValue::new(ValueData::Xform(out))
                .unwrap()
                .hash()
                .to_hex(),
            "121b8defc57a9c1e96ccc2a6bee91c40930aa1c3c91cc8075110125750c034e8"
        );
    }
}
