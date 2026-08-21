//! The `mesh_union` node.

use cicada_core::geometry::{Mesh, Watertight};
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`mesh_union`].
#[derive(Ports, Clone, Debug)]
pub struct MeshUnionIn {
    /// The solids to union (empty list → the empty solid).
    pub meshes: Vec<Watertight<Mesh>>,
}

/// Mesh Union — the union of watertight meshes via Manifold (docs/08:
/// watertight, parallel, seconds).
///
/// # Returns
///
/// One solid: the union of all the meshes.
///
/// # Panics
///
/// Panics when Manifold refuses an operand (named by index) — its
/// ε-validity is stricter than structural watertightness in corner cases.
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=2.0)
/// block = mesh_box(x=span, y=span, z=span)
/// shift = unit_x(factor=1.0)
/// blocks = linear_array(geometry=block, direction=shift, count=2)
/// fused = mesh_union(meshes=blocks)
/// ```
#[node(category = "Mesh & field", tier = "S", version = 1, gh = "Mesh Union")]
#[must_use]
pub fn mesh_union(input: MeshUnionIn) -> Watertight<Mesh> {
    let meshes: Vec<Mesh> = input.meshes.into_iter().map(|w| w.0).collect();
    Watertight(red(cicada_geom::boolean::union(&meshes)))
}

// The inclusion–exclusion property exercises all three booleans and lives
// here.
#[cfg(test)]
mod tests {
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::{Plane, Point};
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::meshbuild::signed_volume;

    use super::*;
    use crate::meshes::mesh_box::{MeshBoxIn, mesh_box};
    use crate::meshes::mesh_difference::{MeshDifferenceIn, mesh_difference};
    use crate::meshes::mesh_intersection::{MeshIntersectionIn, mesh_intersection};
    use crate::solids::support::config;

    #[test]
    fn mesh_union_table_cases() {
        let unit = |origin: f64| {
            mesh_box(
                &config(),
                MeshBoxIn {
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
        let joined = mesh_union(MeshUnionIn {
            meshes: vec![unit(0.0), unit(0.5)],
        });
        assert!((signed_volume(&joined.0) - 1.875).abs() < 1e-9);
        // Documented edge: an empty operand list unions to the empty solid.
        let empty = mesh_union(MeshUnionIn { meshes: vec![] });
        assert!(signed_volume(&empty.0).abs() < 1e-12);
    }

    proptest::proptest! {
        // The three booleans agree with measure theory: inclusion–exclusion
        // for union/intersection, and difference = A minus the overlap.
        #[test]
        fn property_boolean_inclusion_exclusion(
            ax in 0.5..3.0_f64, ay in 0.5..3.0_f64, az in 0.5..3.0_f64,
            ox in -1.5..3.5_f64, oy in -1.5..3.5_f64, oz in -1.5..3.5_f64,
        ) {
            let a = || mesh_box(
                &config(),
                MeshBoxIn {
                    plane: Plane::world_xy(),
                    x: Domain::new(0.0, ax),
                    y: Domain::new(0.0, ay),
                    z: Domain::new(0.0, az),
                },
            );
            let b = || mesh_box(
                &config(),
                MeshBoxIn {
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
    }

    #[test]
    fn mesh_union_determinism_golden_hash() {
        let cube = |origin: f64| {
            mesh_box(
                &config(),
                MeshBoxIn {
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
}
