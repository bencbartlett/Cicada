//! Transform nodes (docs/08 §Catalog 10) — kind-preserving over the `T`
//! type variable: a moved `Closed<Curve>` is still a `Closed<Curve>`,
//! statically (checker) and at runtime (the [`Transformable`] enum).
//! Every spike transform is a similarity, so analytic curves transform
//! EXACTLY (`cicada_geom::transform`).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Transformable;
use cicada_core::spatial::{Plane, Point, Vector};
use cicada_geom::frame::orthonormal;
use cicada_geom::transform::Similarity;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`move_`].
#[derive(Ports, Clone, Debug)]
pub struct MoveIn {
    /// The geometry to move.
    pub geometry: Transformable,
    /// The translation.
    pub motion: Vector,
}

/// Move — translate geometry along a vector.
#[node(category = "Transform", tier = "S", version = 1)]
#[must_use]
pub fn move_(input: MoveIn) -> Transformable {
    Similarity::translation(input.motion).apply(&input.geometry)
}

/// Inputs for [`rotate`].
#[derive(Ports, Clone, Debug)]
pub struct RotateIn {
    /// The geometry to rotate.
    pub geometry: Transformable,
    /// Rotation angle in radians (right-handed about the plane's normal).
    #[port(dimension = angle)]
    pub angle: f64,
    /// Rotation frame: about its z axis, through its origin.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
}

/// Rotate — rotate geometry about a plane's normal through its origin.
///
/// # Panics
///
/// Panics when the plane is degenerate (zero-length or parallel axes).
#[node(category = "Transform", tier = "S", version = 1, uses_tolerance)]
#[must_use]
pub fn rotate(config: &ProjectConfig, input: RotateIn) -> Transformable {
    let frame = red(orthonormal(&input.plane, config.tol()));
    Similarity::rotation(&frame, input.angle).apply(&input.geometry)
}

/// Inputs for [`scale`].
#[derive(Ports, Clone, Debug)]
pub struct ScaleIn {
    /// The geometry to scale.
    pub geometry: Transformable,
    /// Center of scaling.
    #[port(default = Point::origin(), default_doc = "origin")]
    pub center: Point,
    /// Uniform scale factor (negative = point reflection).
    pub factor: f64,
}

/// Scale — uniform scale about a center. Negative factors point-reflect
/// (mesh orientation is preserved by winding correction).
///
/// # Panics
///
/// Panics when `|factor|` is within tolerance of zero — geometry would
/// collapse to a point.
#[node(category = "Transform", tier = "S", version = 1, uses_tolerance)]
#[must_use]
pub fn scale(config: &ProjectConfig, input: ScaleIn) -> Transformable {
    assert!(
        input.factor.abs() > config.tol(),
        "scale: |factor| = {} is within tolerance of zero — geometry would collapse",
        input.factor.abs()
    );
    Similarity::scale_about(input.center, input.factor).apply(&input.geometry)
}

/// Inputs for [`orient`].
#[derive(Ports, Clone, Debug)]
pub struct OrientIn {
    /// The geometry to orient.
    pub geometry: Transformable,
    /// The frame the geometry currently sits in.
    pub source: Plane,
    /// The frame to carry it onto.
    pub target: Plane,
}

/// Orient — the rigid motion carrying the source plane onto the target
/// plane (the wall's part-to-plate workhorse).
///
/// # Panics
///
/// Panics when either plane is degenerate.
#[node(category = "Transform", tier = "S", version = 1, uses_tolerance)]
#[must_use]
pub fn orient(config: &ProjectConfig, input: OrientIn) -> Transformable {
    let source = red(orthonormal(&input.source, config.tol()));
    let target = red(orthonormal(&input.target, config.tol()));
    Similarity::plane_remap(&source, &target).apply(&input.geometry)
}

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
#[node(category = "Transform", tier = "S", version = 1)]
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
    use cicada_core::geometry::{Circle, Curve};
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::tol;

    use super::*;

    fn config() -> ProjectConfig {
        ProjectConfig::default()
    }

    fn point(x: f64, y: f64, z: f64) -> Transformable {
        Transformable::Point(Point::new(x, y, z))
    }

    fn expect_point(value: &Transformable) -> Point {
        match value {
            Transformable::Point(p) => *p,
            other => panic!("expected Point, got {other:?}"),
        }
    }

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

    #[test]
    fn rotate_quarter_turn_table() {
        let out = rotate(
            &config(),
            RotateIn {
                geometry: point(1.0, 0.0, 0.0),
                angle: std::f64::consts::FRAC_PI_2,
                plane: Plane::world_xy(),
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(0.0, 1.0, 0.0),
            1e-12
        ));
    }

    #[test]
    fn scale_about_center_table() {
        let out = scale(
            &config(),
            ScaleIn {
                geometry: point(3.0, 0.0, 0.0),
                center: Point::new(1.0, 0.0, 0.0),
                factor: 2.0,
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(5.0, 0.0, 0.0),
            1e-12
        ));
        // Circles stay circles with scaled radius (analytic exactness).
        let circle = Transformable::Curve(Curve::Circle(Circle {
            plane: Plane::world_xy(),
            radius: 2.0,
        }));
        let Transformable::Curve(Curve::Circle(scaled)) = scale(
            &config(),
            ScaleIn {
                geometry: circle,
                center: Point::origin(),
                factor: 3.0,
            },
        ) else {
            panic!("kind and variant preserved")
        };
        assert!((scaled.radius - 6.0).abs() < 1e-12);
    }

    #[test]
    #[should_panic(expected = "collapse")]
    fn scale_zero_factor_is_red() {
        let _ = scale(
            &config(),
            ScaleIn {
                geometry: point(1.0, 0.0, 0.0),
                center: Point::origin(),
                factor: 0.0,
            },
        );
    }

    #[test]
    fn orient_carries_source_to_target() {
        let out = orient(
            &config(),
            OrientIn {
                geometry: point(1.0, 0.0, 0.0),
                source: Plane::world_xy(),
                target: Plane {
                    origin: Point::new(10.0, 0.0, 0.0),
                    ..Plane::world_xy()
                },
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(11.0, 0.0, 0.0),
            1e-12
        ));
    }

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

        // Rotation about world z is an isometry that fixes z: distance to
        // the axis origin is preserved for ANY angle.
        #[test]
        fn property_rotate_preserves_radius(
            x in -100.0..100.0_f64, y in -100.0..100.0_f64, z in -100.0..100.0_f64,
            angle in -10.0..10.0_f64,
        ) {
            let out = rotate(
                &config(),
                RotateIn {
                    geometry: point(x, y, z),
                    angle,
                    plane: Plane::world_xy(),
                },
            );
            let got = expect_point(&out);
            let before = Point::new(x, y, z).0.length();
            proptest::prop_assert!((got.0.length() - before).abs() <= 1e-9 * before.max(1.0));
            proptest::prop_assert!((got.0.z - z).abs() <= 1e-9 * z.abs().max(1.0));
        }

        // Scaling multiplies distances from the center by |factor| —
        // including negative (point-reflection) factors.
        #[test]
        fn property_scale_scales_distances(
            x in -100.0..100.0_f64, y in -100.0..100.0_f64,
            cx in -50.0..50.0_f64, cy in -50.0..50.0_f64,
            factor in -8.0..8.0_f64,
        ) {
            proptest::prop_assume!(factor.abs() > 1e-3);
            let center = Point::new(cx, cy, 0.0);
            let out = scale(
                &config(),
                ScaleIn {
                    geometry: point(x, y, 0.0),
                    center,
                    factor,
                },
            );
            let got = expect_point(&out);
            let before = Point::new(x, y, 0.0).0.distance(center.0);
            let want = factor.abs() * before;
            proptest::prop_assert!((got.0.distance(center.0) - want).abs() <= 1e-9 * want.max(1.0));
        }

        // Orient onto an axis-permuted target frame re-expresses local
        // coordinates exactly (a rigid motion, pure arithmetic here).
        #[test]
        fn property_orient_remaps_coordinates(
            px in -100.0..100.0_f64, py in -100.0..100.0_f64, pz in -100.0..100.0_f64,
            tx in -100.0..100.0_f64, ty in -100.0..100.0_f64, tz in -100.0..100.0_f64,
        ) {
            // target: x̂ = world y, ŷ = world z, so ẑ = world x.
            let target = Plane {
                origin: Point::new(tx, ty, tz),
                x: Vector::new(0.0, 1.0, 0.0),
                y: Vector::new(0.0, 0.0, 1.0),
            };
            let out = orient(
                &config(),
                OrientIn {
                    geometry: point(px, py, pz),
                    source: Plane::world_xy(),
                    target,
                },
            );
            let want = Point::new(tx + pz, ty + px, tz + py);
            proptest::prop_assert!(tol::coincident(expect_point(&out), want, 1e-9));
        }

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

    // Golden-hash inputs stay transcendental-free (docs/14): translation,
    // scale, axis-permuted orient, and a ZERO-angle rotation are pure
    // arithmetic (sin 0 = 0 and cos 0 = 1 are exact in every libm);
    // non-trivial rotation angles would make the hash platform-dependent
    // and are forbidden in goldens.

    fn expect_point_hash(value: &Transformable) -> String {
        let Transformable::Point(p) = value else {
            panic!("point stays a point")
        };
        HashedValue::new(ValueData::Point(*p))
            .unwrap()
            .hash()
            .to_hex()
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
    fn rotate_determinism_golden_hash() {
        let out = rotate(
            &config(),
            RotateIn {
                geometry: point(1.0, 2.0, 3.0),
                angle: 0.0,
                plane: Plane::world_xy(),
            },
        );
        assert_eq!(
            expect_point_hash(&out),
            "1a6f8073cd8ceb247b753adbb96e270c282cc660b09bb99c4719b64d687b1ca2"
        );
    }

    #[test]
    fn scale_determinism_golden_hash() {
        let out = scale(
            &config(),
            ScaleIn {
                geometry: point(3.0, -2.0, 1.0),
                center: Point::new(1.0, 1.0, 1.0),
                factor: 2.5,
            },
        );
        assert_eq!(
            expect_point_hash(&out),
            "4b937822d72c8469269a8a87c71c9df0faea13bf20ddd37ae6294d897a46c4f1"
        );
    }

    #[test]
    fn orient_determinism_golden_hash() {
        let out = orient(
            &config(),
            OrientIn {
                geometry: point(1.0, 2.0, 3.0),
                source: Plane::world_xy(),
                target: Plane {
                    origin: Point::new(10.0, -5.0, 2.5),
                    x: Vector::new(0.0, 1.0, 0.0),
                    y: Vector::new(0.0, 0.0, 1.0),
                },
            },
        );
        assert_eq!(
            expect_point_hash(&out),
            "475816028129da08b14677700e053eed31a98af10a7a2850650d2893e1a4a736"
        );
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
