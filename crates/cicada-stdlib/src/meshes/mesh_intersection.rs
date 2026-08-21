//! The `mesh_intersection` node.

use cicada_core::geometry::{Mesh, Watertight};
use cicada_macros::{Ports, node};

use crate::red;

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
/// # Returns
///
/// The solid common to `a` and `b` (possibly empty).
///
/// # Panics
///
/// Panics when Manifold refuses an operand.
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=2.0)
/// first = mesh_box(x=span, y=span, z=span)
/// at = construct_point(x=1.0, y=1.0, z=1.0)
/// frame = xy_plane(origin=at)
/// second = mesh_box(plane=frame, x=span, y=span, z=span)
/// overlap = mesh_intersection(a=first, b=second)
/// ```
#[node(
    category = "Mesh & field",
    tier = "S",
    version = 1,
    gh = "Mesh Intersection"
)]
#[must_use]
pub fn mesh_intersection(input: MeshIntersectionIn) -> Watertight<Mesh> {
    Watertight(red(cicada_geom::boolean::intersection(
        &input.a.0, &input.b.0,
    )))
}

// Property coverage: the inclusion–exclusion property in `mesh_union.rs`
// exercises this node.
// The three-boolean inclusion–exclusion property in `mesh_union.rs`
// exercises this node as well; the property below is the intersection's own.
#[cfg(test)]
mod tests {
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::{Plane, Point};
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::meshbuild::signed_volume;

    use super::*;
    use crate::meshes::mesh_box::{MeshBoxIn, mesh_box};
    use crate::meshes::support::{aligned_box, overlap_volume};
    use crate::solids::support::config;

    #[test]
    fn mesh_intersection_table_cases() {
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
        let overlap = mesh_intersection(MeshIntersectionIn {
            a: unit(0.0),
            b: unit(0.5),
        });
        assert!((signed_volume(&overlap.0) - 0.125).abs() < 1e-9);
    }

    proptest::proptest! {
        // The intersection's volume is the analytic overlap of the two
        // boxes (zero when they are apart — the empty solid), it is
        // symmetric in its operands, and a solid intersected with itself
        // is itself.
        #[test]
        fn mesh_intersection_property_volume_is_the_overlap(
            ax in 0.5..3.0_f64, ay in 0.5..3.0_f64, az in 0.5..3.0_f64,
            ox in -1.5..3.5_f64, oy in -1.5..3.5_f64, oz in -1.5..3.5_f64,
            other in 0.5..2.0_f64,
        ) {
            let extents = [ax, ay, az];
            let origin = [ox, oy, oz];
            let size = [other, other, other];
            let volume_a = ax * ay * az;
            let tolerance = 1e-8 * (volume_a + 1.0);

            let expected = overlap_volume([0.0; 3], extents, origin, size);
            let ab = signed_volume(
                &mesh_intersection(MeshIntersectionIn {
                    a: aligned_box([0.0; 3], extents),
                    b: aligned_box(origin, size),
                })
                .0,
            );
            let ba = signed_volume(
                &mesh_intersection(MeshIntersectionIn {
                    a: aligned_box(origin, size),
                    b: aligned_box([0.0; 3], extents),
                })
                .0,
            );
            proptest::prop_assert!((ab - expected).abs() <= tolerance, "{ab} vs {expected}");
            proptest::prop_assert!((ab - ba).abs() <= tolerance, "not symmetric: {ab} vs {ba}");

            let with_itself = signed_volume(
                &mesh_intersection(MeshIntersectionIn {
                    a: aligned_box([0.0; 3], extents),
                    b: aligned_box([0.0; 3], extents),
                })
                .0,
            );
            proptest::prop_assert!((with_itself - volume_a).abs() <= tolerance);
        }
    }

    #[test]
    fn mesh_intersection_determinism_golden_hash() {
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
}
