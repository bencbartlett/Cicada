//! The `cone` node: the B-rep cone, OCCT-backed (v0.1 item 3 WP-C).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Solid;
use cicada_core::spatial::Plane;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`cone`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct ConeIn {
    /// The base plane: the cone stands on it, centred at its origin, its
    /// apex `height` along the normal.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// The base radius.
    #[port(dimension = length)]
    pub radius: f64,
    /// The height from the base to the apex, along the plane's normal.
    #[port(dimension = length)]
    pub height: f64,
}

/// Cone — a B-rep cone standing on a plane: a planar base of `radius`, an
/// exact conical face to the apex `height` above it (a frustum is
/// `loft(profiles=[circle, circle])`).
///
/// # Returns
///
/// The cone solid.
///
/// # Panics
///
/// Panics when the radius or height is not above tolerance, the plane is
/// degenerate, or the kernel refuses.
///
/// # Examples
///
/// ```cic
/// tip = cone(radius=3.0, height=6.0)
/// ```
#[node(
    category = "Surface & solid",
    tier = "1",
    version = 1,
    gh = "Cone",
    uses_tolerance
)]
#[must_use]
pub fn cone(config: &ProjectConfig, input: ConeIn) -> Solid {
    red(cicada_geom::solid::cone(
        &input.plane,
        input.radius,
        input.height,
        config.tol(),
    ))
}

// No committed golden for `cone` (sin/cos-fed seam coordinates,
// `support.rs`): run-to-run identity + the analytic volume instead.
#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use cicada_core::spatial::Point;
    use cicada_geom::tol;

    use super::*;
    use crate::solids::support::{bounds_of, close_rel, config, plane_at, volume_of, with_kernel};

    #[test]
    fn cone_table_cases() {
        let Some(tip) = with_kernel(|| {
            cone(
                &config(),
                ConeIn {
                    plane: plane_at(1.0, 0.0, 0.0),
                    radius: 3.0,
                    height: 6.0,
                },
            )
        }) else {
            return;
        };
        assert!(close_rel(volume_of(&tip), PI * 9.0 * 6.0 / 3.0, 1e-9));
        let (min, max) = bounds_of(&tip);
        assert!(
            tol::coincident(min, Point::new(-2.0, -3.0, 0.0), 1e-7),
            "{min:?}"
        );
        assert!(
            tol::coincident(max, Point::new(4.0, 3.0, 6.0), 1e-7),
            "{max:?}"
        );
        // The centroid sits a quarter of the way up.
        let centroid = cicada_geom::solid::volume(&tip).unwrap().centroid;
        assert!(tol::coincident(centroid, Point::new(1.0, 0.0, 1.5), 1e-9));
    }

    #[test]
    #[should_panic(expected = "radius = 0 is out of range")]
    fn cone_zero_radius_is_red() {
        let _ = cone(
            &config(),
            ConeIn {
                plane: Plane::world_xy(),
                radius: 0.0,
                height: 1.0,
            },
        );
    }

    proptest::proptest! {
        // Volume = π r² h / 3.
        #[test]
        fn property_cone_volume(r in 0.05f64..20.0, h in 0.05f64..40.0) {
            if cicada_geom::solid::kernel_available() {
                let out = cone(
                    &config(),
                    ConeIn {
                        plane: Plane::world_xy(),
                        radius: r,
                        height: h,
                    },
                );
                proptest::prop_assert!(close_rel(volume_of(&out), PI * r * r * h / 3.0, 1e-9));
            }
        }
    }

    #[test]
    fn cone_determinism_run_to_run() {
        let make = || {
            cone(
                &config(),
                ConeIn {
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
