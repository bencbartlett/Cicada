//! The `mesh_sphere` node — the spike's mesh-backed UV sphere, continuing
//! the mesh tier under its `mesh_` name since the OCCT-backed `sphere`
//! arrived (DECISIONS.md row 42, revised 2026-08-19).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Mesh, Watertight};
use cicada_core::spatial::Plane;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`mesh_sphere`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct MeshSphereIn {
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

/// Mesh Sphere — a UV sphere at a plane's origin as a watertight mesh
/// (the mesh tier's sphere, `segments` longitudes — the wall's pins;
/// the B-rep `sphere` is the default working mode).
///
/// # Returns
///
/// The watertight UV-sphere mesh.
///
/// # Panics
///
/// Panics when the radius is not above tolerance, `segments < 3`, or the
/// plane is degenerate.
///
/// # Examples
///
/// ```cic
/// ball = mesh_sphere(radius=1.5, segments=24)
/// ```
#[node(
    category = "Mesh & field",
    tier = "S",
    version = 1,
    gh = "Mesh Sphere",
    uses_tolerance
)]
#[must_use]
pub fn mesh_sphere(config: &ProjectConfig, input: MeshSphereIn) -> Watertight<Mesh> {
    Watertight(red(cicada_geom::meshbuild::sphere_mesh(
        &input.plane,
        input.radius,
        input.segments,
        config.tol(),
    )))
}

// No golden hash of the whole mesh for `mesh_sphere`, deliberately: its
// vertices come from sin/cos (see `support.rs`). Its determinism test
// below hashes what IS transcendental-free — the topology — and asserts
// run-to-run byte identity of the rest.
#[cfg(test)]
mod tests {
    use cicada_core::marshal::IntoValue;
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::meshbuild::signed_volume;

    use super::*;
    use crate::solids::support::config;

    #[test]
    fn mesh_sphere_is_watertight_with_expected_volume() {
        let ball = mesh_sphere(
            &config(),
            MeshSphereIn {
                plane: Plane::world_xy(),
                radius: 1.0,
                segments: 48,
            },
        );
        let expected = 4.0 / 3.0 * std::f64::consts::PI;
        assert!((signed_volume(&ball.0) - expected).abs() / expected < 1e-2);
    }

    proptest::proptest! {
        // UV spheres: watertight, inscribed (volume strictly below the
        // ball), and nowhere near degenerate for segments >= 12.
        #[test]
        fn property_mesh_sphere_watertight_volume_bounds(
            radius in 0.05..10.0_f64,
            segments in 12i64..48,
        ) {
            let out = mesh_sphere(
                &config(),
                MeshSphereIn {
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
    }

    // Determinism: the index buffer is integer arithmetic (a golden hash,
    // cross-platform — blessed via run-once), the UV layout has exact
    // vertex and triangle counts, and the same inputs give byte-identical
    // positions run to run (a constant for those would be libm-dependent).
    #[test]
    fn mesh_sphere_determinism_topology_golden_hash() {
        let make = || {
            mesh_sphere(
                &config(),
                MeshSphereIn {
                    plane: Plane::world_xy(),
                    radius: 1.5,
                    segments: 24,
                },
            )
        };
        let ball = make().0;
        // segments = 24 → 12 rings: 2 poles + 11 × 24 ring vertices; 2 × 24
        // cap triangles + 10 bands × 24 quads × 2.
        assert_eq!(ball.vertex_count(), 266);
        assert_eq!(ball.triangle_count(), 528);
        let topology: Vec<i64> = ball.indices().iter().map(|&i| i64::from(i)).collect();
        assert_eq!(
            topology.into_value().unwrap().hash().to_hex(),
            "7b0a6519a9d3036e0da265a5e90a6ff01f850a641edddbbd60317bbd41e56335"
        );
        let again = make().0;
        assert_eq!(
            HashedValue::new(ValueData::Mesh(ball)).unwrap().hash(),
            HashedValue::new(ValueData::Mesh(again)).unwrap().hash(),
            "same inputs, same bytes"
        );
    }
}
