//! The `extrude_to_point` node (v0.1 item 3 WP-C).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve, Solid};
use cicada_core::spatial::Point;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`extrude_to_point`].
#[derive(Ports, Clone, Debug)]
pub struct ExtrudeToPointIn {
    /// The closed, planar profile (the base).
    pub profile: Closed<Curve>,
    /// The apex every edge of the profile tapers to; must lie off the
    /// profile's plane.
    pub apex: Point,
}

/// Extrude Point — taper a closed planar profile to a point: a pyramid over
/// a polyline or rectangle (planar faces), a cone over a circle (the wall's
/// frusta are `loft`; this is their pointed cousin).
///
/// # Returns
///
/// The pyramid or cone solid.
///
/// # Panics
///
/// Panics when the profile is degenerate, non-planar or self-intersecting
/// at tolerance, the apex lies in the profile plane, or the kernel refuses.
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=-1.0, end=1.0)
/// base = rectangle(x=span, y=span)
/// top = construct_point(x=0.0, y=0.0, z=3.0)
/// pyramid = extrude_to_point(profile=base, apex=top)
/// ```
#[node(
    category = "Surface & solid",
    tier = "1",
    version = 1,
    gh = "Extrude Point",
    uses_tolerance
)]
#[must_use]
pub fn extrude_to_point(config: &ProjectConfig, input: ExtrudeToPointIn) -> Solid {
    red(cicada_geom::solid::extrude_to_point(
        &input.profile.0,
        input.apex,
        config.tol(),
    ))
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use cicada_core::geometry::Circle;
    use cicada_core::spatial::Plane;

    use super::*;
    use crate::solids::support::{
        close_rel, config, platform_golden, ring, solid_hash, unit_square_profile, volume_of,
        with_kernel,
    };

    #[test]
    fn extrude_to_point_table_cases() {
        // A square pyramid: V = A h / 3.
        let Some(pyramid) = with_kernel(|| {
            extrude_to_point(
                &config(),
                ExtrudeToPointIn {
                    profile: ring(&[(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)], 0.0),
                    apex: Point::new(1.0, 1.0, 3.0),
                },
            )
        }) else {
            return;
        };
        assert!(close_rel(volume_of(&pyramid), 4.0 * 3.0 / 3.0, 1e-12));
        let (_, _, faces) = cicada_geom::solid::edges_and_vertices(
            &pyramid,
            cicada_geom::solid::Deflection::display(&config()),
        )
        .unwrap();
        assert_eq!(faces, 5);
        // An oblique apex (off-centre) keeps the volume; below the base too.
        let leaning = extrude_to_point(
            &config(),
            ExtrudeToPointIn {
                profile: unit_square_profile(),
                apex: Point::new(5.0, -2.0, -4.0),
            },
        );
        assert!(close_rel(volume_of(&leaning), 1.0 * 4.0 / 3.0, 1e-12));
        // A circle tapers to a cone.
        let cone = extrude_to_point(
            &config(),
            ExtrudeToPointIn {
                profile: Closed(Curve::Circle(Circle {
                    plane: Plane::world_xy(),
                    radius: 1.0,
                })),
                apex: Point::new(0.0, 0.0, 3.0),
            },
        );
        assert!(close_rel(volume_of(&cone), PI / 3.0 * 3.0, 1e-6));
    }

    #[test]
    #[should_panic(expected = "off the profile plane")]
    fn extrude_to_point_apex_in_plane_is_red() {
        let _ = extrude_to_point(
            &config(),
            ExtrudeToPointIn {
                profile: unit_square_profile(),
                apex: Point::new(5.0, 5.0, 0.0),
            },
        );
    }

    proptest::proptest! {
        // Any rectangle, any apex height and lean: V = w h · height / 3.
        #[test]
        fn property_pyramid_volume(
            w in 0.1..10.0_f64, d in 0.1..10.0_f64,
            ax in -5.0..5.0_f64, ay in -5.0..5.0_f64,
            height in proptest::prop_oneof![-20.0f64..-0.1, 0.1f64..20.0],
        ) {
            if cicada_geom::solid::kernel_available() {
                let out = extrude_to_point(
                    &config(),
                    ExtrudeToPointIn {
                        profile: ring(&[(0.0, 0.0), (w, 0.0), (w, d), (0.0, d)], 0.0),
                        apex: Point::new(ax, ay, height),
                    },
                );
                proptest::prop_assert!(close_rel(volume_of(&out), w * d * height.abs() / 3.0, 1e-9));
            }
        }
    }

    #[test]
    fn extrude_to_point_determinism_golden_hash() {
        // Arithmetic-only: a rectangle base and an apex with exact
        // coordinates. Blessed via run-once on win-64 (2026-08-20).
        let Some(pyramid) = with_kernel(|| {
            extrude_to_point(
                &config(),
                ExtrudeToPointIn {
                    profile: ring(&[(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (0.0, 1.0)], 0.0),
                    apex: Point::new(0.5, 0.25, 3.0),
                },
            )
        }) else {
            return;
        };
        assert_eq!(
            solid_hash(&pyramid),
            platform_golden("89c05969aa2d9e4d28ebd431f5949918f42d881b7e936ed0f2d6dcc50e2a4674")
        );
    }
}
