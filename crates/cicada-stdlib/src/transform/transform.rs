//! The `transform` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Transformable;
use cicada_core::spatial::Xform;
use cicada_geom::transform::Affine;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`transform`].
#[derive(Ports, Clone, Debug)]
pub struct TransformIn {
    /// The geometry to transform.
    pub geometry: Transformable,
    /// The transform to apply (`construct_xform`, `compose_xform`).
    pub xform: Xform,
}

/// Transform — apply an `Xform` to geometry.
///
/// A transform that is a similarity (a rigid motion, a uniform scale, a
/// reflection — what `move`, `rotate`, `scale`, `mirror` and `orient`
/// build) is applied exactly to every kind, a Solid through the kernel. A
/// general affine — a stretch, a shear — is carried only where the result
/// is still the same kind: points, vectors, lines, polylines and meshes
/// always; a plane while its axes stay perpendicular; a circle while its
/// own plane is scaled evenly; a rectangle while its frame is not skewed,
/// its domains scaled. A circle stretched unevenly would be an ellipse,
/// which has no kind, and a Solid takes similarities only (the kernel's
/// general transform is not wired): both are red with the numbers, never
/// an approximation.
///
/// # Returns
///
/// The geometry under `xform`.
///
/// # Panics
///
/// Panics when the transform cannot carry the kind exactly — a `Circle`
/// whose plane it stretches unevenly (an ellipse), a `Plane` or
/// `Rectangle` it skews or flattens, any `Solid` under a stretch or shear
/// (tessellate it and transform the mesh) — or for a `Solid` the OCCT
/// kernel refuses to transform (a `Solid` moves through the kernel — its
/// B-rep geometry is rewritten, never a mesh in disguise).
///
/// # Examples
///
/// ```cic
/// shift = construct_xform(rows=[1.0, 0.0, 0.0, 5.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0])
/// ring = circle(radius=2.0)
/// moved = transform(geometry=ring, xform=shift)
/// ```
#[node(
    category = "Transform",
    tier = "1",
    version = 1,
    gh = "Transform",
    uses_tolerance
)]
#[must_use]
pub fn transform(config: &ProjectConfig, input: TransformIn) -> Transformable {
    red(Affine::from_xform(&input.xform).try_apply(&input.geometry, config.tol()))
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::{Circle, Curve};
    use cicada_core::spatial::{Plane, Point, Vector};
    use cicada_geom::frame::orthonormal;
    use cicada_geom::tol;
    use cicada_geom::transform::Similarity;

    use super::*;
    use crate::solids::support::{brep_box, with_kernel};
    use crate::transform::support::{config, expect_point, expect_point_hash, point};

    #[test]
    fn transform_table() {
        // A similarity: the point moves as the node that built it would
        // move it.
        let shift = Similarity::translation(Vector::new(1.0, 2.0, 3.0)).xform();
        let out = transform(
            &config(),
            TransformIn {
                geometry: point(1.0, 1.0, 1.0),
                xform: shift,
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(2.0, 3.0, 4.0),
            1e-12
        ));
        // A uniform scale keeps a circle a circle with a scaled radius.
        let double = Similarity::scale_about(Point::origin(), 2.0).xform();
        let Transformable::Curve(Curve::Circle(ring)) = transform(
            &config(),
            TransformIn {
                geometry: Transformable::Curve(Curve::Circle(Circle {
                    plane: Plane::world_xy(),
                    radius: 1.5,
                })),
                xform: double,
            },
        ) else {
            panic!("kind and variant preserved")
        };
        assert!((ring.radius - 3.0).abs() < 1e-12);
        // A shear carries a point exactly and the identity leaves it alone.
        let shear = Affine::from_rows(&[
            1.0, 0.5, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ])
        .xform();
        let out = transform(
            &config(),
            TransformIn {
                geometry: point(1.0, 2.0, 3.0),
                xform: shear,
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(2.0, 2.0, 3.0),
            1e-12
        ));
        let out = transform(
            &config(),
            TransformIn {
                geometry: point(1.0, 2.0, 3.0),
                xform: Xform::identity(),
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(1.0, 2.0, 3.0),
            1e-12
        ));
    }

    #[test]
    #[should_panic(expected = "a Circle cannot take this transform exactly")]
    fn transform_of_a_circle_into_an_ellipse_is_red() {
        let world = orthonormal(&Plane::world_xy(), 1e-9).unwrap();
        let stretch = Affine::scale_in_frame(&world, glam::DVec3::new(2.0, 1.0, 1.0)).xform();
        let _ = transform(
            &config(),
            TransformIn {
                geometry: Transformable::Curve(Curve::Circle(Circle {
                    plane: Plane::world_xy(),
                    radius: 1.0,
                })),
                xform: stretch,
            },
        );
    }

    // A Solid: a similarity goes through the kernel; a stretch is refused
    // before any kernel is asked, in both worlds.
    #[test]
    fn transform_of_a_solid_is_a_similarity_or_red() {
        let block = brep_box([0.0, 0.0, 0.0], [1.0, 2.0, 3.0]);
        let shift = Similarity::translation(Vector::new(10.0, 0.0, 0.0)).xform();
        if let Some(Transformable::Solid(moved)) = with_kernel(|| {
            transform(
                &config(),
                TransformIn {
                    geometry: Transformable::Solid(block.clone()),
                    xform: shift,
                },
            )
        }) {
            let (min, max) = cicada_geom::solid::bounds(&moved).unwrap();
            assert!(tol::coincident(min, Point::new(10.0, 0.0, 0.0), 1e-9));
            assert!(tol::coincident(max, Point::new(11.0, 2.0, 3.0), 1e-9));
        }
        // The stretch is refused by the node's own judgement, with the same
        // text in both worlds (never `expect_red`, which expects the KERNEL's
        // refusal in the kernel-free world).
        let world = orthonormal(&Plane::world_xy(), 1e-9).unwrap();
        let stretch = Affine::scale_in_frame(&world, glam::DVec3::new(1.0, 1.0, 2.0)).xform();
        let refused = std::panic::catch_unwind(|| {
            transform(
                &config(),
                TransformIn {
                    geometry: Transformable::Solid(block),
                    xform: stretch,
                },
            )
        })
        .expect_err("a stretched solid is refused in both worlds");
        assert!(
            refused
                .downcast_ref::<String>()
                .unwrap()
                .contains("a Solid cannot take this transform exactly"),
            "{refused:?}"
        );
    }

    proptest::proptest! {
        // `transform` over the xform a similarity builds agrees with that
        // similarity applied directly — for a random rotation, scale and
        // translation composed.
        #[test]
        fn property_transform_agrees_with_the_similarity(
            x in -50.0..50.0_f64, y in -50.0..50.0_f64, z in -50.0..50.0_f64,
            angle in -6.0..6.0_f64, factor in proptest::prop_oneof![0.1f64..5.0, -5.0f64..-0.1],
            tx in -5.0..5.0_f64,
        ) {
            let frame = orthonormal(&Plane::world_xy(), 1e-9).unwrap();
            let turn = Similarity::rotation(&frame, angle);
            let grow = Similarity::scale_about(Point::new(tx, 1.0, 0.0), factor);
            let composite = Affine::from_xform(&turn.xform()).then(&Affine::from_xform(&grow.xform()));
            let out = transform(
                &config(),
                TransformIn {
                    geometry: point(x, y, z),
                    xform: composite.xform(),
                },
            );
            let want = grow.apply_point(turn.apply_point(Point::new(x, y, z)));
            proptest::prop_assert!(tol::coincident(expect_point(&out), want, 1e-9 * want.0.length().max(1.0)));
        }
    }

    #[test]
    fn transform_determinism_golden_hash() {
        // A translation composed with a dyadic scale: pure arithmetic.
        let xform =
            Affine::from_xform(&Similarity::translation(Vector::new(1.0, -2.0, 0.5)).xform())
                .then(&Affine::from_xform(
                    &Similarity::scale_about(Point::new(1.0, 1.0, 1.0), 4.0).xform(),
                ))
                .xform();
        let out = transform(
            &config(),
            TransformIn {
                geometry: point(3.0, -2.0, 1.0),
                xform,
            },
        );
        assert_eq!(
            expect_point_hash(&out),
            "147b5b5cc146df78e304f39e138d8d1c0d99a52f68b1f7b1d2b187d32d160e2c"
        );
    }
}
