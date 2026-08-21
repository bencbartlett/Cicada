//! The `cylinder` node: the B-rep cylinder, OCCT-backed (v0.1 item 3 WP-C).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Solid;
use cicada_core::spatial::Plane;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`cylinder`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct CylinderIn {
    /// The base plane: the cylinder stands on it, centred at its origin,
    /// rising along its normal.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// The radius.
    #[port(dimension = length)]
    pub radius: f64,
    /// The height along the plane's normal.
    #[port(dimension = length)]
    pub height: f64,
}

/// Cylinder — a B-rep cylinder standing on a plane (two planar caps and one
/// exact cylindrical face — the boss and the drill of simple CAD).
///
/// # Returns
///
/// The cylinder solid.
///
/// # Panics
///
/// Panics when the radius or height is not above tolerance, the plane is
/// degenerate, or the kernel refuses.
///
/// # Examples
///
/// ```cic
/// boss = cylinder(radius=4.0, height=10.0)
/// ```
#[node(
    category = "Surface & solid",
    tier = "1",
    version = 1,
    gh = "Cylinder",
    uses_tolerance
)]
#[must_use]
pub fn cylinder(config: &ProjectConfig, input: CylinderIn) -> Solid {
    red(cicada_geom::solid::cylinder(
        &input.plane,
        input.radius,
        input.height,
        config.tol(),
    ))
}

// No committed golden for `cylinder` (sin/cos-fed seam coordinates,
// `support.rs`): run-to-run identity + the analytic volume instead.
#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use cicada_core::spatial::Point;
    use cicada_geom::tol;

    use super::*;
    use crate::solids::support::{bounds_of, close_rel, config, plane_at, volume_of, with_kernel};

    #[test]
    fn cylinder_table_cases() {
        let Some(peg) = with_kernel(|| {
            cylinder(
                &config(),
                CylinderIn {
                    plane: plane_at(0.0, 0.0, 5.0),
                    radius: 1.5,
                    height: 4.0,
                },
            )
        }) else {
            return;
        };
        assert!(close_rel(volume_of(&peg), PI * 2.25 * 4.0, 1e-9));
        let (min, max) = bounds_of(&peg);
        assert!(
            tol::coincident(min, Point::new(-1.5, -1.5, 5.0), 1e-7),
            "{min:?}"
        );
        assert!(
            tol::coincident(max, Point::new(1.5, 1.5, 9.0), 1e-7),
            "{max:?}"
        );
        let (_, _, faces) =
            cicada_geom::solid::edges_and_vertices(&peg, display_deflection()).unwrap();
        assert_eq!(faces, 3);
    }

    fn display_deflection() -> cicada_geom::solid::Deflection {
        cicada_geom::solid::Deflection::display(&config())
    }

    #[test]
    #[should_panic(expected = "height = -1 is out of range")]
    fn cylinder_negative_height_is_red() {
        let _ = cylinder(
            &config(),
            CylinderIn {
                plane: Plane::world_xy(),
                radius: 1.0,
                height: -1.0,
            },
        );
    }

    proptest::proptest! {
        // Volume = π r² h.
        #[test]
        fn property_cylinder_volume(r in 0.05f64..20.0, h in 0.05f64..40.0) {
            if cicada_geom::solid::kernel_available() {
                let out = cylinder(
                    &config(),
                    CylinderIn {
                        plane: Plane::world_xy(),
                        radius: r,
                        height: h,
                    },
                );
                proptest::prop_assert!(close_rel(volume_of(&out), PI * r * r * h, 1e-9));
            }
        }
    }

    #[test]
    fn cylinder_determinism_run_to_run() {
        let make = || {
            cylinder(
                &config(),
                CylinderIn {
                    plane: Plane::world_xy(),
                    radius: 2.0,
                    height: 3.0,
                },
            )
        };
        let Some(first) = with_kernel(make) else {
            return;
        };
        assert_eq!(first, make());
    }
}
