//! Surface & solid + mesh-tier nodes (docs/08 §Catalog 7–8). Doc 15's
//! honest shim, stated here too: spike `extrude`/`box`/`sphere` carry
//! their v0.1 names but are mesh-backed — they return `Watertight<Mesh>`
//! (the mesh-tier solid), not the B-rep `Solid` that arrives with OCCT in
//! v0.1. The wall corpus is mesh-destined, so nothing in the spike's
//! criteria needs more.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve, Mesh, Watertight};
use cicada_core::scalar::Domain;
use cicada_core::spatial::{Plane, Vector};
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`extrude`].
#[derive(Ports, Clone, Debug)]
pub struct ExtrudeIn {
    /// The closed, planar profile to extrude.
    pub profile: Closed<Curve>,
    /// Extrusion direction and length (need not be normal to the profile —
    /// oblique prisms are legal).
    pub direction: Vector,
    /// Tessellation density for curved profiles (circles).
    #[port(default = 64)]
    pub segments: i64,
}

/// Extrude — extrude a closed planar profile into a watertight prism
/// (mesh-backed under its v0.1 name, doc 15).
///
/// # Panics
///
/// Panics when the profile is degenerate or non-planar at tolerance, the
/// direction lies in the profile plane, `segments < 3`, or the profile
/// polygon is self-intersecting.
#[node(category = "Surface & solid", tier = "S", version = 1, uses_tolerance)]
#[must_use]
pub fn extrude(config: &ProjectConfig, input: ExtrudeIn) -> Watertight<Mesh> {
    Watertight(red(cicada_geom::meshbuild::extrude(
        &input.profile.0,
        input.direction,
        input.segments,
        config.tol(),
    )))
}

/// Inputs for [`box_`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct BoxIn {
    /// The box's frame.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// Extent along the frame's x axis.
    pub x: Domain,
    /// Extent along the frame's y axis.
    pub y: Domain,
    /// Extent along the frame's z axis.
    pub z: Domain,
}

/// Box — an axis-aligned box in a plane's frame (mesh-backed under its
/// v0.1 name, doc 15). Decreasing domains are normalized.
///
/// # Panics
///
/// Panics when any extent is empty at tolerance or the plane is
/// degenerate.
#[node(category = "Surface & solid", tier = "S", version = 1, uses_tolerance)]
#[must_use]
pub fn box_(config: &ProjectConfig, input: BoxIn) -> Watertight<Mesh> {
    Watertight(red(cicada_geom::meshbuild::box_mesh(
        &input.plane,
        input.x,
        input.y,
        input.z,
        config.tol(),
    )))
}

/// Inputs for [`sphere`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct SphereIn {
    /// Center frame; the plane's z is the polar axis.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// The radius.
    #[port(dimension = length)]
    pub radius: f64,
    /// Longitudinal segment count (latitude bands follow as half).
    #[port(default = 32)]
    pub segments: i64,
}

/// Sphere — a UV sphere at a plane's origin (mesh-backed under its v0.1
/// name, doc 15).
///
/// # Panics
///
/// Panics when the radius is not above tolerance, `segments < 3`, or the
/// plane is degenerate.
#[node(category = "Surface & solid", tier = "S", version = 1, uses_tolerance)]
#[must_use]
pub fn sphere(config: &ProjectConfig, input: SphereIn) -> Watertight<Mesh> {
    Watertight(red(cicada_geom::meshbuild::sphere_mesh(
        &input.plane,
        input.radius,
        input.segments,
        config.tol(),
    )))
}

/// Inputs for [`mesh_union`].
#[derive(Ports, Clone, Debug)]
pub struct MeshUnionIn {
    /// The solids to union (empty list → the empty solid).
    pub meshes: Vec<Watertight<Mesh>>,
}

/// Mesh Union — the union of watertight meshes via Manifold (docs/08:
/// watertight, parallel, seconds).
///
/// # Panics
///
/// Panics when Manifold refuses an operand (named by index) — its
/// ε-validity is stricter than structural watertightness in corner cases.
#[node(category = "Mesh & field", tier = "S", version = 1)]
#[must_use]
pub fn mesh_union(input: MeshUnionIn) -> Watertight<Mesh> {
    let meshes: Vec<Mesh> = input.meshes.into_iter().map(|w| w.0).collect();
    Watertight(red(cicada_geom::boolean::union(&meshes)))
}

/// Inputs for [`mesh_difference`].
#[derive(Ports, Clone, Debug)]
pub struct MeshDifferenceIn {
    /// The solid to carve.
    pub mesh: Watertight<Mesh>,
    /// The solids to subtract (the wall's carve: one part, its cutters).
    pub cutters: Vec<Watertight<Mesh>>,
}

/// Mesh Difference — subtract every cutter from a mesh via Manifold; the
/// result may be the empty solid. Lift with `each()` to carve per part
/// (`mesh_difference(mesh=each(frusta), cutters=each(cutter_groups))`).
///
/// # Panics
///
/// Panics when Manifold refuses the mesh or a cutter (named by index).
#[node(category = "Mesh & field", tier = "S", version = 1)]
#[must_use]
pub fn mesh_difference(input: MeshDifferenceIn) -> Watertight<Mesh> {
    let cutters: Vec<Mesh> = input.cutters.into_iter().map(|w| w.0).collect();
    Watertight(red(cicada_geom::boolean::difference(
        &input.mesh.0,
        &cutters,
    )))
}

/// Inputs for [`mesh_intersection`].
#[derive(Ports, Clone, Debug)]
pub struct MeshIntersectionIn {
    /// First solid.
    pub a: Watertight<Mesh>,
    /// Second solid.
    pub b: Watertight<Mesh>,
}

/// Mesh Intersection — the intersection of two watertight meshes via
/// Manifold; the result may be the empty solid.
///
/// # Panics
///
/// Panics when Manifold refuses an operand.
#[node(category = "Mesh & field", tier = "S", version = 1)]
#[must_use]
pub fn mesh_intersection(input: MeshIntersectionIn) -> Watertight<Mesh> {
    Watertight(red(cicada_geom::boolean::intersection(
        &input.a.0, &input.b.0,
    )))
}

/// Inputs for [`as_watertight`].
#[derive(Ports, Clone, Debug)]
pub struct AsWatertightIn {
    /// The mesh to refine.
    pub mesh: Mesh,
}

/// As Watertight — the checked watertight refinement (docs/08: the
/// mesh-tier solid): every edge shared by exactly two consistently
/// oriented triangles.
///
/// # Panics
///
/// Panics when the mesh has open or inconsistently oriented edges — red
/// with the count, never a silent pass (wall lesson 13).
#[node(category = "Mesh & field", tier = "S", version = 1)]
#[must_use]
pub fn as_watertight(input: AsWatertightIn) -> Watertight<Mesh> {
    assert!(
        input.mesh.is_watertight(),
        "as_watertight: mesh ({} triangles) has open or inconsistently oriented \
         edges — not a closed solid",
        input.mesh.triangle_count()
    );
    Watertight(input.mesh)
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::Rectangle;
    use cicada_core::spatial::Point;
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::meshbuild::signed_volume;

    use super::*;

    fn config() -> ProjectConfig {
        ProjectConfig::default()
    }

    fn unit_square_profile() -> Closed<Curve> {
        Closed(Curve::Rectangle(Rectangle {
            plane: Plane::world_xy(),
            x: Domain::new(0.0, 1.0),
            y: Domain::new(0.0, 1.0),
        }))
    }

    #[test]
    fn extrude_box_sphere_are_watertight_with_expected_volumes() {
        let prism = extrude(
            &config(),
            ExtrudeIn {
                profile: unit_square_profile(),
                direction: Vector::new(0.0, 0.0, 2.0),
                segments: 64,
            },
        );
        assert!((signed_volume(&prism.0) - 2.0).abs() < 1e-9);

        let cube = box_(
            &config(),
            BoxIn {
                plane: Plane::world_xy(),
                x: Domain::new(0.0, 2.0),
                y: Domain::new(0.0, 2.0),
                z: Domain::new(0.0, 2.0),
            },
        );
        assert!((signed_volume(&cube.0) - 8.0).abs() < 1e-9);

        let ball = sphere(
            &config(),
            SphereIn {
                plane: Plane::world_xy(),
                radius: 1.0,
                segments: 48,
            },
        );
        let expected = 4.0 / 3.0 * std::f64::consts::PI;
        assert!((signed_volume(&ball.0) - expected).abs() / expected < 1e-2);
    }

    #[test]
    #[should_panic(expected = "profile plane")]
    fn extrude_in_plane_direction_is_red() {
        let _ = extrude(
            &config(),
            ExtrudeIn {
                profile: unit_square_profile(),
                direction: Vector::new(1.0, 0.0, 0.0),
                segments: 64,
            },
        );
    }

    #[test]
    fn as_watertight_accepts_closed_refuses_open() {
        let closed = box_(
            &config(),
            BoxIn {
                plane: Plane::world_xy(),
                x: Domain::new(0.0, 1.0),
                y: Domain::new(0.0, 1.0),
                z: Domain::new(0.0, 1.0),
            },
        )
        .0;
        let refined = as_watertight(AsWatertightIn { mesh: closed });
        assert!(refined.0.is_watertight());
    }

    #[test]
    #[should_panic(expected = "not a closed solid")]
    fn as_watertight_open_mesh_is_red() {
        let open = Mesh::new(
            vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            vec![0, 1, 2],
        )
        .expect("valid open mesh");
        let _ = as_watertight(AsWatertightIn { mesh: open });
    }

    proptest::proptest! {
        // Oblique prisms included: volume = base area × normal height for
        // any shear (Cavalieri), watertight always.
        #[test]
        fn property_extrude_prism_volume(
            dx in 0.1..10.0_f64, dy in 0.1..10.0_f64,
            sx in -3.0..3.0_f64, sy in -3.0..3.0_f64,
            h in 0.1..10.0_f64,
        ) {
            let out = extrude(
                &config(),
                ExtrudeIn {
                    profile: Closed(Curve::Rectangle(Rectangle {
                        plane: Plane::world_xy(),
                        x: Domain::new(0.0, dx),
                        y: Domain::new(0.0, dy),
                    })),
                    direction: Vector::new(sx, sy, h),
                    segments: 8,
                },
            );
            proptest::prop_assert!(out.0.is_watertight());
            let want = dx * dy * h;
            proptest::prop_assert!((signed_volume(&out.0) - want).abs() <= 1e-9 * want.max(1.0));
        }

        // UV spheres: watertight, inscribed (volume strictly below the
        // ball), and nowhere near degenerate for segments >= 12.
        #[test]
        fn property_sphere_watertight_volume_bounds(
            radius in 0.05..10.0_f64,
            segments in 12i64..48,
        ) {
            let out = sphere(
                &config(),
                SphereIn {
                    plane: Plane::world_xy(),
                    radius,
                    segments,
                },
            );
            proptest::prop_assert!(out.0.is_watertight());
            let vol = signed_volume(&out.0);
            let ball = 4.0 / 3.0 * std::f64::consts::PI * radius.powi(3);
            proptest::prop_assert!(vol > 0.8 * ball, "volume {} vs ball {}", vol, ball);
            proptest::prop_assert!(vol < ball * (1.0 + 1e-12));
        }

        // The three booleans agree with measure theory: inclusion–exclusion
        // for union/intersection, and difference = A minus the overlap.
        #[test]
        fn property_boolean_inclusion_exclusion(
            ax in 0.5..3.0_f64, ay in 0.5..3.0_f64, az in 0.5..3.0_f64,
            ox in -1.5..3.5_f64, oy in -1.5..3.5_f64, oz in -1.5..3.5_f64,
        ) {
            let a = || box_(
                &config(),
                BoxIn {
                    plane: Plane::world_xy(),
                    x: Domain::new(0.0, ax),
                    y: Domain::new(0.0, ay),
                    z: Domain::new(0.0, az),
                },
            );
            let b = || box_(
                &config(),
                BoxIn {
                    plane: Plane {
                        origin: Point::new(ox, oy, oz),
                        ..Plane::world_xy()
                    },
                    x: Domain::new(0.0, 1.0),
                    y: Domain::new(0.0, 1.0),
                    z: Domain::new(0.0, 1.0),
                },
            );
            let va = ax * ay * az;
            let vb = 1.0;
            let union = signed_volume(&mesh_union(MeshUnionIn { meshes: vec![a(), b()] }).0);
            let inter = signed_volume(
                &mesh_intersection(MeshIntersectionIn { a: a(), b: b() }).0,
            );
            let diff = signed_volume(
                &mesh_difference(MeshDifferenceIn { mesh: a(), cutters: vec![b()] }).0,
            );
            let total = va + vb;
            proptest::prop_assert!((union + inter - total).abs() <= 1e-8 * total);
            proptest::prop_assert!((diff - (va - inter)).abs() <= 1e-8 * total);
        }

        // as_watertight passes any watertight mesh through unchanged.
        #[test]
        fn property_as_watertight_pass_through(
            dx in 0.01..20.0_f64, dy in 0.01..20.0_f64, dz in 0.01..20.0_f64,
        ) {
            let mesh = box_(
                &config(),
                BoxIn {
                    plane: Plane::world_xy(),
                    x: Domain::new(0.0, dx),
                    y: Domain::new(0.0, dy),
                    z: Domain::new(0.0, dz),
                },
            )
            .0;
            let refined = as_watertight(AsWatertightIn { mesh: mesh.clone() });
            proptest::prop_assert_eq!(refined.0, mesh);
        }

        // Boxes at any placement: volume = product of extents, exactly.
        #[test]
        fn property_box_volume(
            dx in 0.01f64..50.0, dy in 0.01f64..50.0, dz in 0.01f64..50.0,
            ox in -100.0f64..100.0,
        ) {
            let out = box_(
                &config(),
                BoxIn {
                    plane: Plane {
                        origin: Point::new(ox, 0.0, 0.0),
                        ..Plane::world_xy()
                    },
                    x: Domain::new(0.0, dx),
                    y: Domain::new(0.0, dy),
                    z: Domain::new(0.0, dz),
                },
            );
            let want = dx * dy * dz;
            proptest::prop_assert!((signed_volume(&out.0) - want).abs() <= 1e-9 * want.max(1.0));
        }
    }

    #[test]
    fn boolean_nodes_carve_union_intersect() {
        let unit = |origin: f64| {
            box_(
                &config(),
                BoxIn {
                    plane: Plane {
                        origin: Point::new(origin, origin, origin),
                        ..Plane::world_xy()
                    },
                    x: Domain::new(0.0, 1.0),
                    y: Domain::new(0.0, 1.0),
                    z: Domain::new(0.0, 1.0),
                },
            )
        };
        let carved = mesh_difference(MeshDifferenceIn {
            mesh: unit(0.0),
            cutters: vec![unit(0.5)],
        });
        assert!((signed_volume(&carved.0) - 0.875).abs() < 1e-9);
        let joined = mesh_union(MeshUnionIn {
            meshes: vec![unit(0.0), unit(0.5)],
        });
        assert!((signed_volume(&joined.0) - 1.875).abs() < 1e-9);
        let overlap = mesh_intersection(MeshIntersectionIn {
            a: unit(0.0),
            b: unit(0.5),
        });
        assert!((signed_volume(&overlap.0) - 0.125).abs() < 1e-9);
        // Documented edge: an empty operand list unions to the empty solid.
        let empty = mesh_union(MeshUnionIn { meshes: vec![] });
        assert!(signed_volume(&empty.0).abs() < 1e-12);
    }

    // Golden-hash inputs must stay transcendental-free (boxes and
    // rectangle prisms, never spheres or rotations): sin/cos differ in the
    // last ulp across platform libms, so a transcendental-fed golden would
    // be platform-dependent. Cross-platform kernel identity for curved
    // geometry is measured at stage 6, not here — which is also why
    // `sphere` deliberately has NO mesh golden.

    // The carve's determinism is a golden hash: same inputs → the same
    // bytes out of Manifold, across runs and platforms (the stage-6 corpus
    // extends this to the full wall).
    #[test]
    fn boolean_determinism_golden_hash() {
        let base = box_(
            &config(),
            BoxIn {
                plane: Plane::world_xy(),
                x: Domain::new(0.0, 2.0),
                y: Domain::new(0.0, 2.0),
                z: Domain::new(0.0, 2.0),
            },
        );
        // An arithmetic-only cutter: a unit box punched through the top
        // face (transcendental-input goldens are forbidden, see above).
        let cutter = box_(
            &config(),
            BoxIn {
                plane: Plane {
                    origin: Point::new(0.5, 0.5, 1.0),
                    ..Plane::world_xy()
                },
                x: Domain::new(0.0, 1.0),
                y: Domain::new(0.0, 1.0),
                z: Domain::new(0.0, 1.5),
            },
        );
        let carved = mesh_difference(MeshDifferenceIn {
            mesh: base,
            cutters: vec![cutter],
        });
        assert!((signed_volume(&carved.0) - 7.0).abs() < 1e-9);
        let sealed = HashedValue::new(ValueData::Mesh(carved.0)).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            "9db199d7450fc730d16600090943d9253066a31ad47809ced55843efb730df74"
        );
    }

    #[test]
    fn extrude_determinism_golden_hash() {
        // Oblique rectangle prism: pure arithmetic (corner lerps + shear).
        let prism = extrude(
            &config(),
            ExtrudeIn {
                profile: Closed(Curve::Rectangle(Rectangle {
                    plane: Plane::world_xy(),
                    x: Domain::new(0.0, 1.0),
                    y: Domain::new(0.0, 2.0),
                })),
                direction: Vector::new(0.25, 0.0, 3.0),
                segments: 8,
            },
        );
        let sealed = HashedValue::new(ValueData::Mesh(prism.0)).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            "6d59e4bbc7472fc06575a8c88c96be3bedbf7ac45adecad0d0ec5cb84f0d42db"
        );
    }

    #[test]
    fn mesh_union_determinism_golden_hash() {
        let cube = |origin: f64| {
            box_(
                &config(),
                BoxIn {
                    plane: Plane {
                        origin: Point::new(origin, origin, origin),
                        ..Plane::world_xy()
                    },
                    x: Domain::new(0.0, 2.0),
                    y: Domain::new(0.0, 2.0),
                    z: Domain::new(0.0, 2.0),
                },
            )
        };
        let joined = mesh_union(MeshUnionIn {
            meshes: vec![cube(0.0), cube(1.0)],
        });
        let sealed = HashedValue::new(ValueData::Mesh(joined.0)).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            "6f1694e9baeb6890c805763eda3b6bf73c6d1b4c25f2dd2f9386056b1c715c97"
        );
    }

    #[test]
    fn mesh_intersection_determinism_golden_hash() {
        let cube = |origin: f64| {
            box_(
                &config(),
                BoxIn {
                    plane: Plane {
                        origin: Point::new(origin, origin, origin),
                        ..Plane::world_xy()
                    },
                    x: Domain::new(0.0, 2.0),
                    y: Domain::new(0.0, 2.0),
                    z: Domain::new(0.0, 2.0),
                },
            )
        };
        let overlap = mesh_intersection(MeshIntersectionIn {
            a: cube(0.0),
            b: cube(1.0),
        });
        let sealed = HashedValue::new(ValueData::Mesh(overlap.0)).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            "e0ac967780140db5597693d697dbc99cacb43959b3642fd959ece9f707acac80"
        );
    }

    #[test]
    fn box_determinism_golden_hash() {
        let cube = box_(
            &config(),
            BoxIn {
                plane: Plane::world_xy(),
                x: Domain::new(0.0, 1.0),
                y: Domain::new(0.0, 2.0),
                z: Domain::new(0.0, 3.0),
            },
        );
        let sealed = HashedValue::new(ValueData::Mesh(cube.0)).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            "3063b49cbeec12ff1b2dc909b7abe1ffbc060cd66c92f62128c89f7926e42766"
        );
    }

    #[test]
    fn as_watertight_determinism_golden_hash() {
        // Pass-through refinement: the hash is exactly the box golden above.
        let cube = box_(
            &config(),
            BoxIn {
                plane: Plane::world_xy(),
                x: Domain::new(0.0, 1.0),
                y: Domain::new(0.0, 2.0),
                z: Domain::new(0.0, 3.0),
            },
        );
        let refined = as_watertight(AsWatertightIn { mesh: cube.0 });
        let sealed = HashedValue::new(ValueData::Mesh(refined.0)).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            "3063b49cbeec12ff1b2dc909b7abe1ffbc060cd66c92f62128c89f7926e42766"
        );
    }
}
