//! The `sphere` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Mesh, Watertight};
use cicada_core::spatial::Plane;
use cicada_macros::{Ports, node};

use crate::{MESH_BYTES_PER_VERTEX, checked_size, red};

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
/// # Returns
///
/// The watertight UV-sphere mesh.
///
/// # Panics
///
/// Panics when the radius is not above tolerance, `segments < 3`, the
/// plane is degenerate, or the sphere's vertex count (`segments × rings`,
/// rings = `segments / 2`) would be above the shared ceilings (2^24 slots,
/// or 1 GiB of mesh — the message names the count and the ceiling that
/// bit; 5,794 segments is the first refused).
///
/// # Examples
///
/// ```cic
/// ball = sphere(radius=1.5, segments=24)
/// ```
#[node(
    category = "Surface & solid",
    tier = "S",
    version = 1,
    gh = "Sphere",
    uses_tolerance
)]
#[must_use]
pub fn sphere(config: &ProjectConfig, input: SphereIn) -> Watertight<Mesh> {
    // The vertex count is a PRODUCT of the one port (segments × rings), so
    // the port alone under the slot ceiling is not enough: it is checked
    // as the derived size before the kernel allocates it. The kernel keeps
    // the floor (`segments < 3`).
    if input.segments >= 3 {
        let _ = checked_size(
            "sphere",
            "vertices",
            sphere_vertex_count(input.segments),
            MESH_BYTES_PER_VERTEX,
        );
    }
    Watertight(red(cicada_geom::meshbuild::sphere_mesh(
        &input.plane,
        input.radius,
        input.segments,
        config.tol(),
    )))
}

/// The UV sphere's vertex count for `segments >= 3`, as the kernel lays it
/// out: two poles plus `rings - 1` rings of `segments` vertices, where
/// `rings = max(segments / 2, 2)` (the determinism test pins it against
/// the built mesh).
fn sphere_vertex_count(segments: i64) -> u128 {
    let segments = u128::from(segments.unsigned_abs());
    let rings = (segments / 2).max(2);
    (rings - 1) * segments + 2
}

// No golden hash of the whole mesh for `sphere`, deliberately: its
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
    fn sphere_is_watertight_with_expected_volume() {
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
    #[should_panic(expected = "segments = 2 is out of range: must be >= 3")]
    fn sphere_too_few_segments_is_red() {
        let _ = sphere(
            &config(),
            SphereIn {
                plane: Plane::world_xy(),
                radius: 1.0,
                segments: 2,
            },
        );
    }

    // The slot ceiling on the DERIVED vertex count: 5,793 segments is the
    // last sphere under 2^24 vertices (16,770,737), 5,794 the first over
    // (16,779,426) — red before the kernel allocates a single position.
    #[test]
    fn sphere_vertex_ceiling_sits_between_5793_and_5794_segments() {
        assert!(sphere_vertex_count(5793) <= u128::from(crate::MAX_SLOTS.unsigned_abs()));
        assert_eq!(sphere_vertex_count(5794), 16_779_426);
    }

    #[test]
    #[should_panic(
        expected = "sphere: vertices would be 16779426 — above the 16777216 (2^24) slot ceiling"
    )]
    fn sphere_one_segment_past_the_vertex_ceiling_is_refused_not_allocated() {
        let _ = sphere(
            &config(),
            SphereIn {
                plane: Plane::world_xy(),
                radius: 1.0,
                segments: 5794,
            },
        );
    }

    #[test]
    #[should_panic(expected = "sphere: vertices would be 4999999999999900000000000002 —")]
    fn sphere_absurd_segments_are_refused_not_allocated() {
        let _ = sphere(
            &config(),
            SphereIn {
                plane: Plane::world_xy(),
                radius: 1.0,
                segments: 100_000_000_000_000,
            },
        );
    }

    proptest::proptest! {
        // UV spheres: watertight, inscribed (volume strictly below the
        // ball), and nowhere near degenerate for segments >= 12; the vertex
        // count the ceiling is checked against is the count the kernel
        // builds.
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
            proptest::prop_assert_eq!(
                u128::try_from(out.0.vertex_count()).unwrap(),
                sphere_vertex_count(segments)
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
    fn sphere_determinism_topology_golden_hash() {
        let make = || {
            sphere(
                &config(),
                SphereIn {
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
        assert_eq!(
            sphere_vertex_count(24),
            266,
            "the ceiling's count is the kernel's"
        );
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
