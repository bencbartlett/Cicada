//! The `mesh_difference` node.

use cicada_core::geometry::{Mesh, Watertight};
use cicada_macros::{Ports, node};

use crate::red;

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
/// # Returns
///
/// The carved solid — `mesh` minus every cutter (possibly empty).
///
/// # Panics
///
/// Panics when Manifold refuses the mesh or a cutter (named by index).
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=2.0)
/// block = box(x=span, y=span, z=span)
/// top = construct_point(x=1.0, y=1.0, z=2.0)
/// top_frame = xy_plane(origin=top)
/// ball = sphere(plane=top_frame, radius=0.75, segments=24)
/// still = unit_x(factor=0.0)
/// cutters = linear_array(geometry=ball, direction=still, count=1)
/// carved = mesh_difference(mesh=block, cutters=cutters)
/// ```
#[node(
    category = "Mesh & field",
    tier = "S",
    version = 1,
    gh = "Mesh Difference"
)]
#[must_use]
pub fn mesh_difference(input: MeshDifferenceIn) -> Watertight<Mesh> {
    let cutters: Vec<Mesh> = input.cutters.into_iter().map(|w| w.0).collect();
    Watertight(red(cicada_geom::boolean::difference(
        &input.mesh.0,
        &cutters,
    )))
}

// The three-boolean inclusion–exclusion property in `mesh_union.rs`
// exercises this node as well; the property below is the carve's own.
#[cfg(test)]
mod tests {
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::{Plane, Point};
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::meshbuild::signed_volume;

    use super::*;
    use crate::meshes::support::{aligned_box, overlap_volume};
    use crate::solids::r#box::{BoxIn, box_};
    use crate::solids::support::config;

    #[test]
    fn mesh_difference_table_cases() {
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
    }

    proptest::proptest! {
        // The carve is measure-theoretic subtraction: the carved volume is
        // the mesh volume minus the analytic overlap with the cutter (a
        // cutter clear of the mesh leaves the volume unchanged, one that
        // contains it empties it), and two cutters subtract like their
        // union — no double counting where they overlap each other.
        #[test]
        fn mesh_difference_property_volume_is_mesh_minus_overlap(
            ax in 0.5..3.0_f64, ay in 0.5..3.0_f64, az in 0.5..3.0_f64,
            ox in -1.5..3.5_f64, oy in -1.5..3.5_f64, oz in -1.5..3.5_f64,
            cutter in 0.5..2.0_f64,
        ) {
            let extents = [ax, ay, az];
            let origin = [ox, oy, oz];
            let size = [cutter, cutter, cutter];
            let mesh_volume = ax * ay * az;
            let tolerance = 1e-8 * (mesh_volume + 1.0);

            let carved = mesh_difference(MeshDifferenceIn {
                mesh: aligned_box([0.0; 3], extents),
                cutters: vec![aligned_box(origin, size)],
            });
            let expected = mesh_volume - overlap_volume([0.0; 3], extents, origin, size);
            proptest::prop_assert!(
                (signed_volume(&carved.0) - expected).abs() <= tolerance,
                "carved {} vs expected {}", signed_volume(&carved.0), expected
            );

            // The same cutter twice carves exactly once.
            let twice = mesh_difference(MeshDifferenceIn {
                mesh: aligned_box([0.0; 3], extents),
                cutters: vec![aligned_box(origin, size), aligned_box(origin, size)],
            });
            proptest::prop_assert!((signed_volume(&twice.0) - expected).abs() <= tolerance);

            // A cutter far away changes nothing.
            let untouched = mesh_difference(MeshDifferenceIn {
                mesh: aligned_box([0.0; 3], extents),
                cutters: vec![aligned_box([10.0, 10.0, 10.0], size)],
            });
            proptest::prop_assert!(
                (signed_volume(&untouched.0) - mesh_volume).abs() <= tolerance
            );
        }
    }

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
}
