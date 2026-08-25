//! The `scale_nu` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Transformable;
use cicada_core::spatial::Plane;
use cicada_geom::frame::orthonormal;
use cicada_geom::tol;
use cicada_geom::transform::Affine;
use cicada_macros::{Ports, node};
use glam::DVec3;

use crate::red;

/// Inputs for [`scale_nu`].
#[derive(Ports, Clone, Debug)]
pub struct ScaleNuIn {
    /// The geometry to scale.
    pub geometry: Transformable,
    /// The scaling frame: about its origin, along its axes.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// Factor along the frame's x axis (negative = mirrored across the
    /// frame's yz plane).
    #[port(default = 1.0)]
    pub x: f64,
    /// Factor along the frame's y axis.
    #[port(default = 1.0)]
    pub y: f64,
    /// Factor along the frame's z axis.
    #[port(default = 1.0)]
    pub z: f64,
}

/// Scale NU — non-uniform scale about a plane's origin, a factor per axis.
///
/// Equal factors are the uniform `scale` (exactly: every kind is carried,
/// a Solid through the kernel). Unequal factors stretch, and a stretch is
/// carried only where the result is still the same kind — points, vectors,
/// lines, polylines and meshes always; a plane while its axes stay
/// perpendicular (aligned with the frame, or not turned); a circle while
/// its own plane is scaled evenly (an XY circle under a z-only stretch
/// stays a circle); a rectangle aligned with the frame, its domains
/// scaled. A circle stretched unevenly would be an ellipse, which has no
/// kind, and a Solid takes similarities only (the kernel's general
/// transform is not wired): both are red with the numbers, never an
/// approximation.
///
/// # Returns
///
/// The geometry scaled by `x`, `y`, `z` along the frame's axes about its
/// origin.
///
/// # Panics
///
/// Panics when a factor is within tolerance of zero (geometry would
/// collapse), the plane is degenerate, or the stretch cannot carry the
/// kind exactly — a `Circle` whose plane it stretches unevenly (an
/// ellipse), a `Plane` or `Rectangle` it skews, any `Solid` under unequal
/// factors (tessellate it and scale the mesh) — or, under equal factors,
/// for a `Solid` the OCCT kernel refuses to transform.
///
/// # Examples
///
/// ```cic
/// ring = circle(radius=2.0)
/// tall = scale_nu(geometry=ring, x=1.0, y=1.0, z=3.0)
/// ```
#[node(
    category = "Transform",
    tier = "1",
    version = 1,
    gh = "Scale NU",
    uses_tolerance
)]
#[must_use]
pub fn scale_nu(config: &ProjectConfig, input: ScaleNuIn) -> Transformable {
    for (name, factor) in [("x", input.x), ("y", input.y), ("z", input.z)] {
        assert!(
            !tol::near_zero(factor.abs(), config.tol()),
            "scale_nu: |{name}| = {} is within tolerance of zero — geometry would collapse",
            factor.abs()
        );
    }
    let frame = red(orthonormal(&input.plane, config.tol()));
    let stretch = Affine::scale_in_frame(&frame, DVec3::new(input.x, input.y, input.z));
    red(stretch.try_apply(&input.geometry, config.tol()))
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::{Circle, Curve, Rectangle};
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::{Point, Vector};
    use cicada_geom::meshbuild::{box_mesh, signed_volume};

    use super::*;
    use crate::solids::support::{brep_box, with_kernel};
    use crate::transform::scale::{ScaleIn, scale};
    use crate::transform::support::{config, expect_point, expect_point_hash, point};

    fn frame_at(x: f64, y: f64, z: f64) -> Plane {
        Plane {
            origin: Point::new(x, y, z),
            ..Plane::world_xy()
        }
    }

    #[test]
    fn scale_nu_table() {
        // Each factor acts along its own axis about the frame's origin.
        let out = scale_nu(
            &config(),
            ScaleNuIn {
                geometry: point(3.0, 3.0, 3.0),
                plane: frame_at(1.0, 1.0, 1.0),
                x: 2.0,
                y: 0.5,
                z: -1.0,
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(5.0, 2.0, -1.0),
            1e-12
        ));
        // A circle under a z-only stretch stays a circle at its radius; its
        // plane's origin moves with the stretch.
        let ring = Transformable::Curve(Curve::Circle(Circle {
            plane: frame_at(0.0, 0.0, 2.0),
            radius: 1.5,
        }));
        let Transformable::Curve(Curve::Circle(tall)) = scale_nu(
            &config(),
            ScaleNuIn {
                geometry: ring.clone(),
                plane: Plane::world_xy(),
                x: 1.0,
                y: 1.0,
                z: 3.0,
            },
        ) else {
            panic!("kind and variant preserved")
        };
        assert!((tall.radius - 1.5).abs() < 1e-12);
        assert!(tol::coincident(
            tall.plane.origin,
            Point::new(0.0, 0.0, 6.0),
            1e-12
        ));
        // An aligned rectangle keeps its kind with scaled domains.
        let Transformable::Curve(Curve::Rectangle(wide)) = scale_nu(
            &config(),
            ScaleNuIn {
                geometry: Transformable::Curve(Curve::Rectangle(Rectangle {
                    plane: Plane::world_xy(),
                    x: Domain::new(0.0, 1.0),
                    y: Domain::new(-1.0, 1.0),
                })),
                plane: Plane::world_xy(),
                x: 4.0,
                y: 0.5,
                z: 1.0,
            },
        ) else {
            panic!("kind and variant preserved")
        };
        assert!((wide.x.end - 4.0).abs() < 1e-12);
        assert!((wide.y.start + 0.5).abs() < 1e-12 && (wide.y.end - 0.5).abs() < 1e-12);
        // A mesh stretches with its volume; a negative factor keeps the
        // windings outward.
        let block = box_mesh(
            &Plane::world_xy(),
            Domain::new(0.0, 1.0),
            Domain::new(0.0, 1.0),
            Domain::new(0.0, 1.0),
            1e-9,
        )
        .unwrap();
        let Transformable::Mesh(squashed) = scale_nu(
            &config(),
            ScaleNuIn {
                geometry: Transformable::Mesh(block),
                plane: Plane::world_xy(),
                x: -2.0,
                y: 3.0,
                z: 0.5,
            },
        ) else {
            panic!("kind preserved")
        };
        assert!(squashed.is_watertight());
        assert!((signed_volume(&squashed) - 3.0).abs() < 1e-9);
        // A vector scales by the linear part only (the frame's origin is
        // immaterial to a displacement).
        let Transformable::Vector(v) = scale_nu(
            &config(),
            ScaleNuIn {
                geometry: Transformable::Vector(Vector::new(1.0, 1.0, 1.0)),
                plane: frame_at(10.0, 10.0, 10.0),
                x: 2.0,
                y: 3.0,
                z: 4.0,
            },
        ) else {
            panic!("kind preserved")
        };
        assert!((v.0 - glam::DVec3::new(2.0, 3.0, 4.0)).length() < 1e-12);
    }

    #[test]
    #[should_panic(expected = "|y| = 0 is within tolerance of zero")]
    fn scale_nu_zero_factor_is_red() {
        let _ = scale_nu(
            &config(),
            ScaleNuIn {
                geometry: point(1.0, 0.0, 0.0),
                plane: Plane::world_xy(),
                x: 1.0,
                y: 0.0,
                z: 1.0,
            },
        );
    }

    #[test]
    #[should_panic(expected = "a Circle cannot take this transform exactly")]
    fn scale_nu_of_a_circle_into_an_ellipse_is_red() {
        let _ = scale_nu(
            &config(),
            ScaleNuIn {
                geometry: Transformable::Curve(Curve::Circle(Circle {
                    plane: Plane::world_xy(),
                    radius: 1.0,
                })),
                plane: Plane::world_xy(),
                x: 2.0,
                y: 1.0,
                z: 1.0,
            },
        );
    }

    // A Solid: equal factors go through the kernel (the uniform scale —
    // volume × 8); unequal ones are refused before any kernel is asked, in
    // both worlds, with the typed message.
    #[test]
    fn scale_nu_of_a_solid_is_uniform_or_red() {
        let block = brep_box([0.0, 0.0, 0.0], [1.0, 2.0, 3.0]);
        if let Some(Transformable::Solid(doubled)) = with_kernel(|| {
            scale_nu(
                &config(),
                ScaleNuIn {
                    geometry: Transformable::Solid(block.clone()),
                    plane: Plane::world_xy(),
                    x: 2.0,
                    y: 2.0,
                    z: 2.0,
                },
            )
        }) {
            let volume = cicada_geom::solid::volume(&doubled).unwrap().volume;
            assert!((volume - 48.0).abs() < 1e-9, "{volume}");
        }
        let message = std::panic::catch_unwind(|| {
            scale_nu(
                &config(),
                ScaleNuIn {
                    geometry: Transformable::Solid(block),
                    plane: Plane::world_xy(),
                    x: 2.0,
                    y: 1.0,
                    z: 1.0,
                },
            )
        })
        .expect_err("a stretched solid is refused");
        let message = message.downcast_ref::<String>().unwrap();
        assert!(
            message.contains("a Solid cannot take this transform exactly")
                && message.contains("tessellate"),
            "{message}"
        );
        // The refusal is the node's own judgement, not the kernel's: the
        // same text whether or not a kernel is linked (never `expect_red`,
        // which expects the KERNEL's refusal in the kernel-free world).
        let squashed = std::panic::catch_unwind(|| {
            scale_nu(
                &config(),
                ScaleNuIn {
                    geometry: Transformable::Solid(brep_box([0.0; 3], [1.0; 3])),
                    plane: Plane::world_xy(),
                    x: 1.0,
                    y: 1.0,
                    z: 0.5,
                },
            )
        })
        .expect_err("a squashed solid is refused in both worlds");
        assert!(
            squashed
                .downcast_ref::<String>()
                .unwrap()
                .contains("a Solid cannot take this transform exactly"),
            "{squashed:?}"
        );
    }

    proptest::proptest! {
        // Equal factors ARE the `scale` node: the two agree on any point.
        #[test]
        fn property_equal_factors_are_the_uniform_scale(
            x in -100.0..100.0_f64, y in -100.0..100.0_f64, z in -100.0..100.0_f64,
            cx in -50.0..50.0_f64, cy in -50.0..50.0_f64,
            factor in proptest::prop_oneof![0.01f64..8.0, -8.0f64..-0.01],
        ) {
            let center = Point::new(cx, cy, 0.0);
            let nu = scale_nu(
                &config(),
                ScaleNuIn {
                    geometry: point(x, y, z),
                    plane: Plane { origin: center, ..Plane::world_xy() },
                    x: factor,
                    y: factor,
                    z: factor,
                },
            );
            let uniform = scale(&config(), ScaleIn { geometry: point(x, y, z), center, factor });
            proptest::prop_assert!(tol::coincident(expect_point(&nu), expect_point(&uniform), 1e-9));
        }

        // Each coordinate's offset from the frame origin scales by its own
        // factor, for any (non-zero) factors.
        #[test]
        fn property_each_axis_scales_by_its_factor(
            x in -100.0..100.0_f64, y in -100.0..100.0_f64, z in -100.0..100.0_f64,
            fx in proptest::prop_oneof![0.01f64..8.0, -8.0f64..-0.01],
            fy in proptest::prop_oneof![0.01f64..8.0, -8.0f64..-0.01],
            fz in proptest::prop_oneof![0.01f64..8.0, -8.0f64..-0.01],
        ) {
            let out = scale_nu(
                &config(),
                ScaleNuIn {
                    geometry: point(x, y, z),
                    plane: frame_at(1.0, 2.0, 3.0),
                    x: fx,
                    y: fy,
                    z: fz,
                },
            );
            let want = Point::new(1.0 + (x - 1.0) * fx, 2.0 + (y - 2.0) * fy, 3.0 + (z - 3.0) * fz);
            proptest::prop_assert!(tol::coincident(expect_point(&out), want, 1e-9 * want.0.length().max(1.0)));
        }
    }

    #[test]
    fn scale_nu_determinism_golden_hash() {
        // Dyadic factors about the world origin: pure arithmetic.
        let out = scale_nu(
            &config(),
            ScaleNuIn {
                geometry: point(3.0, -2.0, 1.0),
                plane: Plane::world_xy(),
                x: 2.0,
                y: 0.5,
                z: -4.0,
            },
        );
        assert_eq!(
            expect_point_hash(&out),
            "f5bf4d883dafb2485d88c0fc0c3fcd4cd43819d42d22d807db6164102a5db163"
        );
    }
}
