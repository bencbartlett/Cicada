//! The `mirror` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Transformable;
use cicada_core::spatial::Plane;
use cicada_geom::frame::orthonormal;
use cicada_geom::transform::Similarity;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`mirror`].
#[derive(Ports, Clone, Debug)]
pub struct MirrorIn {
    /// The geometry to mirror.
    pub geometry: Transformable,
    /// The mirror plane: every point of it stays where it is, everything
    /// else lands the same distance on the other side.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
}

/// Mirror — reflect geometry across a plane.
///
/// The reflection is an isometry (lengths and angles survive, analytic
/// curves stay analytic), so mirroring twice across the same plane is the
/// identity. Orientation flips: a mesh's windings are swapped so outward
/// stays outward, a `Solid` is reversed by the kernel so its volume stays
/// positive, and a mirrored plane's derived normal comes out on the
/// mirrored side. A vector mirrors its direction (displacements ignore
/// where the plane sits).
///
/// # Returns
///
/// The geometry reflected across `plane`.
///
/// # Panics
///
/// Panics when the plane is degenerate (zero-length or parallel axes), or
/// for a `Solid` the OCCT kernel refuses to transform (a `Solid` moves through
/// the kernel — its B-rep geometry is rewritten, never a mesh in disguise).
///
/// # Examples
///
/// ```cic
/// corner = construct_point(x=1.0, y=2.0, z=3.0)
/// reflected = mirror(geometry=corner)
/// ```
#[node(
    category = "Transform",
    tier = "1",
    version = 1,
    gh = "Mirror",
    uses_tolerance
)]
#[must_use]
pub fn mirror(config: &ProjectConfig, input: MirrorIn) -> Transformable {
    let frame = red(orthonormal(&input.plane, config.tol()));
    Similarity::reflection(&frame).apply(&input.geometry)
}

#[cfg(test)]
mod tests {
    use cicada_core::spatial::{Point, Vector};
    use cicada_geom::tol;

    use super::*;
    use crate::transform::support::{config, expect_point, expect_point_hash, point};

    fn plane_at_height(z: f64) -> Plane {
        Plane {
            origin: Point::new(0.0, 0.0, z),
            ..Plane::world_xy()
        }
    }

    #[test]
    fn mirror_table_across_world_xy_and_an_offset_plane() {
        // Across z = 0: the z coordinate flips, x and y stay.
        let out = mirror(
            &config(),
            MirrorIn {
                geometry: point(1.0, 2.0, 3.0),
                plane: Plane::world_xy(),
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(1.0, 2.0, -3.0),
            1e-12
        ));
        // Across z = 5: a point 2 above lands 2 below; a point ON the plane
        // stays (the plane is fixed pointwise — a point reflection would
        // move it).
        let out = mirror(
            &config(),
            MirrorIn {
                geometry: point(1.0, 2.0, 7.0),
                plane: plane_at_height(5.0),
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(1.0, 2.0, 3.0),
            1e-12
        ));
        let fixed = mirror(
            &config(),
            MirrorIn {
                geometry: point(4.0, -4.0, 5.0),
                plane: plane_at_height(5.0),
            },
        );
        assert!(tol::coincident(
            expect_point(&fixed),
            Point::new(4.0, -4.0, 5.0),
            1e-12
        ));
        // A vector mirrors its direction and ignores the plane's offset.
        let v = mirror(
            &config(),
            MirrorIn {
                geometry: Transformable::Vector(Vector::new(1.0, 0.0, 2.0)),
                plane: plane_at_height(5.0),
            },
        );
        let Transformable::Vector(v) = v else {
            panic!("a vector stays a vector")
        };
        assert!(tol::near_zero(
            (v.0 - glam::DVec3::new(1.0, 0.0, -2.0)).length(),
            1e-12
        ));
        // A tilted mirror (the yz plane): x flips.
        let yz = Plane {
            origin: Point::new(0.0, 0.0, 0.0),
            x: Vector::new(0.0, 1.0, 0.0),
            y: Vector::new(0.0, 0.0, 1.0),
        };
        let out = mirror(
            &config(),
            MirrorIn {
                geometry: point(1.0, 2.0, 3.0),
                plane: yz,
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(-1.0, 2.0, 3.0),
            1e-12
        ));
    }

    #[test]
    fn mirror_swaps_mesh_windings_so_outward_stays_outward() {
        use cicada_core::scalar::Domain;
        use cicada_geom::meshbuild::{box_mesh, signed_volume};
        let block = box_mesh(
            &Plane::world_xy(),
            Domain::new(0.0, 1.0),
            Domain::new(0.0, 2.0),
            Domain::new(0.0, 3.0),
            1e-6,
        )
        .unwrap();
        let out = mirror(
            &config(),
            MirrorIn {
                geometry: Transformable::Mesh(block),
                plane: plane_at_height(-1.0),
            },
        );
        let Transformable::Mesh(mirrored) = out else {
            panic!("a mesh stays a mesh")
        };
        assert!(mirrored.is_watertight());
        // Positive: the windings were swapped with the reflection.
        assert!((signed_volume(&mirrored) - 6.0).abs() < 1e-9);
        let (vertices, _) = mirrored.positions().as_chunks::<3>();
        let zs: Vec<f64> = vertices.iter().map(|&[_, _, z]| z).collect();
        let min_z = zs.iter().copied().fold(f64::INFINITY, f64::min);
        let max_z = zs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!(tol::near_zero(min_z + 5.0, 1e-12));
        assert!(tol::near_zero(max_z + 2.0, 1e-12));
    }

    #[test]
    #[should_panic(expected = "degenerate")]
    fn mirror_refuses_a_degenerate_plane() {
        let flat = Plane {
            origin: Point::new(0.0, 0.0, 0.0),
            x: Vector::new(1.0, 0.0, 0.0),
            y: Vector::new(2.0, 0.0, 0.0),
        };
        let _ = mirror(
            &config(),
            MirrorIn {
                geometry: point(1.0, 2.0, 3.0),
                plane: flat,
            },
        );
    }

    proptest::proptest! {
        // A reflection is an involution that fixes the plane: mirroring
        // twice returns the point, and the midpoint of a point and its
        // image lies on the plane, for any height of the mirror.
        #[test]
        fn property_mirror_is_an_involution_fixing_the_plane(
            x in -1.0e3..1.0e3_f64, y in -1.0e3..1.0e3_f64, z in -1.0e3..1.0e3_f64,
            height in -1.0e3..1.0e3_f64,
        ) {
            let plane = plane_at_height(height);
            let there = mirror(&config(), MirrorIn { geometry: point(x, y, z), plane });
            let image = expect_point(&there);
            let back = mirror(&config(), MirrorIn { geometry: there, plane });
            let got = expect_point(&back);
            let scale = x.abs().max(y.abs()).max(z.abs()).max(height.abs()).max(1.0);
            proptest::prop_assert!((got.0 - glam::DVec3::new(x, y, z)).length() <= 1e-9 * scale);
            let mid_z = f64::midpoint(image.0.z, z);
            proptest::prop_assert!((mid_z - height).abs() <= 1e-9 * scale);
            proptest::prop_assert!((image.0.x - x).abs() <= 1e-9 * scale);
            proptest::prop_assert!((image.0.y - y).abs() <= 1e-9 * scale);
        }
    }

    #[test]
    fn mirror_determinism_golden_hash() {
        // An axis-aligned mirror is exact arithmetic (the matrix is
        // diag(1, 1, −1) built from 1 − 0 and 1 − 2; transcendental-free).
        let out = mirror(
            &config(),
            MirrorIn {
                geometry: point(1.0, 2.0, 3.0),
                plane: plane_at_height(0.5),
            },
        );
        assert!(tol::coincident(
            expect_point(&out),
            Point::new(1.0, 2.0, -2.0),
            1e-12
        ));
        assert_eq!(
            expect_point_hash(&out),
            "70604c1a28f3c3bcf3e4e13da8a542f7017d9b9eb1fd4a1a4ec2655ddb5f7716"
        );
        // The exact image, bit for bit (no negative zero from the matrix).
        assert_eq!(
            expect_point_hash(&out),
            expect_point_hash(&point(1.0, 2.0, -2.0))
        );
    }

    #[test]
    fn a_solid_mirrors_through_the_kernel_or_is_the_documented_red_path() {
        // With the kernel a real block is reversed by the kernel: its bounds
        // land on the other side of the plane, its volume stays positive.
        // Without it the node is red with the typed refusal — both worlds
        // asserted, never a vacuous pass (docs/14).
        use cicada_core::geometry::{SOLID_CANONICAL_HEADER, Solid};
        use cicada_core::scalar::Domain;
        if cicada_geom::solid::kernel_available() {
            let block = cicada_geom::solid::box_in_plane(
                &Plane::world_xy(),
                Domain::new(0.0, 1.0),
                Domain::new(0.0, 2.0),
                Domain::new(0.0, 3.0),
                1e-6,
            )
            .unwrap();
            let out = mirror(
                &config(),
                MirrorIn {
                    geometry: Transformable::Solid(block),
                    plane: plane_at_height(-1.0),
                },
            );
            let Transformable::Solid(mirrored) = out else {
                panic!("a Solid stays a Solid")
            };
            let (min, max) = cicada_geom::solid::bounds(&mirrored).unwrap();
            assert!(tol::coincident(min, Point::new(0.0, 0.0, -5.0), 1e-9));
            assert!(tol::coincident(max, Point::new(1.0, 2.0, -2.0), 1e-9));
            let volume = cicada_geom::solid::volume(&mirrored).unwrap().volume;
            assert!((volume - 6.0).abs() < 1e-9, "{volume}");
            assert!(cicada_geom::solid::is_valid(&mirrored).unwrap());
        } else {
            let solid = Solid::from_canonical_bytes(SOLID_CANONICAL_HEADER.to_vec()).unwrap();
            let outcome = std::panic::catch_unwind(|| {
                mirror(
                    &config(),
                    MirrorIn {
                        geometry: Transformable::Solid(solid),
                        plane: Plane::world_xy(),
                    },
                )
            });
            let payload = outcome.expect_err("a Solid must be refused, never passed through");
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .expect("a message");
            assert!(message.contains("needs the OCCT kernel"), "{message}");
        }
    }
}
