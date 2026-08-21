//! The `sphere` node: the B-rep sphere, OCCT-backed (v0.1 item 3 WP-C;
//! the spike's mesh-backed sphere continues as `mesh_sphere`).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Solid;
use cicada_core::spatial::Plane;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`sphere`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct SphereIn {
    /// Center frame; the plane's z is the polar axis (the seam runs along
    /// its x axis).
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// The radius.
    #[port(dimension = length)]
    pub radius: f64,
}

/// Sphere — a B-rep sphere at a plane's origin (one exact spherical face;
/// the default working mode's sphere — `mesh_sphere` is the mesh tier's
/// UV sphere).
///
/// # Returns
///
/// The sphere solid.
///
/// # Panics
///
/// Panics when the radius is not above tolerance, the plane is
/// degenerate, or the kernel refuses.
///
/// # Examples
///
/// ```cic
/// ball = sphere(radius=1.5)
/// ```
#[node(
    category = "Surface & solid",
    tier = "S",
    version = 1,
    gh = "Sphere",
    uses_tolerance
)]
#[must_use]
pub fn sphere(config: &ProjectConfig, input: SphereIn) -> Solid {
    red(cicada_geom::solid::sphere(
        &input.plane,
        input.radius,
        config.tol(),
    ))
}

// No committed golden for `sphere`: a sphere's canonical bytes carry
// sin/cos-fed coordinates (`support.rs`), so its determinism test asserts
// run-to-run byte identity and the analytic volume instead.
#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use cicada_core::spatial::Point;
    use cicada_geom::tol;

    use super::*;
    use crate::solids::support::{bounds_of, close_rel, config, plane_at, volume_of, with_kernel};

    #[test]
    fn sphere_table_cases() {
        let Some(ball) = with_kernel(|| {
            sphere(
                &config(),
                SphereIn {
                    plane: plane_at(1.0, 2.0, 3.0),
                    radius: 2.0,
                },
            )
        }) else {
            return;
        };
        assert!(close_rel(volume_of(&ball), 4.0 / 3.0 * PI * 8.0, 1e-9));
        let (min, max) = bounds_of(&ball);
        assert!(
            tol::coincident(min, Point::new(-1.0, 0.0, 1.0), 1e-7),
            "{min:?}"
        );
        assert!(
            tol::coincident(max, Point::new(3.0, 4.0, 5.0), 1e-7),
            "{max:?}"
        );
        let centroid = cicada_geom::solid::volume(&ball).unwrap().centroid;
        assert!(tol::coincident(centroid, Point::new(1.0, 2.0, 3.0), 1e-9));
    }

    #[test]
    #[should_panic(expected = "radius = 0 is out of range")]
    fn sphere_zero_radius_is_red() {
        let _ = sphere(
            &config(),
            SphereIn {
                plane: Plane::world_xy(),
                radius: 0.0,
            },
        );
    }

    proptest::proptest! {
        // Volume = 4/3 π r³ at any centre.
        #[test]
        fn property_sphere_volume(r in 0.05f64..20.0, oz in -50.0f64..50.0) {
            if cicada_geom::solid::kernel_available() {
                let out = sphere(
                    &config(),
                    SphereIn {
                        plane: plane_at(0.0, 0.0, oz),
                        radius: r,
                    },
                );
                proptest::prop_assert!(close_rel(volume_of(&out), 4.0 / 3.0 * PI * r * r * r, 1e-9));
            }
        }
    }

    #[test]
    fn sphere_determinism_run_to_run() {
        // Two constructions, byte-identical (the kernel is deterministic
        // on one platform); no golden constant, by the transcendental rule.
        let Some(first) = with_kernel(|| {
            sphere(
                &config(),
                SphereIn {
                    plane: Plane::world_xy(),
                    radius: 1.5,
                },
            )
        }) else {
            return;
        };
        let second = sphere(
            &config(),
            SphereIn {
                plane: Plane::world_xy(),
                radius: 1.5,
            },
        );
        assert_eq!(first, second);
    }
}
