//! The `sphere` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Mesh, Watertight};
use cicada_core::spatial::Plane;
use cicada_macros::{Ports, node};

use crate::red;

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
/// Panics when the radius is not above tolerance, `segments < 3`, or the
/// plane is degenerate.
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
    Watertight(red(cicada_geom::meshbuild::sphere_mesh(
        &input.plane,
        input.radius,
        input.segments,
        config.tol(),
    )))
}

// No mesh golden for `sphere`, deliberately: its vertices come from sin/cos
// (see `support.rs`); the watertight + volume-bound property is its
// determinism-adjacent contract.
#[cfg(test)]
mod tests {
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

    proptest::proptest! {
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
    }
}
