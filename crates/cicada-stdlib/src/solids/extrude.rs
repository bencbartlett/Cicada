//! The `extrude` node: the B-rep extrusion, OCCT-backed (v0.1 item 3 WP-C;
//! the spike's mesh-backed extrusion continues as `mesh_extrude`).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve, Solid};
use cicada_core::spatial::Vector;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`extrude`].
#[derive(Ports, Clone, Debug)]
pub struct ExtrudeIn {
    /// The closed, planar profile to extrude.
    pub profile: Closed<Curve>,
    /// Extrusion direction and length (need not be normal to the profile —
    /// oblique prisms are legal).
    pub direction: Vector,
}

/// Extrude — extrude a closed planar profile into a B-rep solid with exact
/// edges for every curve kind: a polyline or rectangle becomes a prism of
/// planar faces, a circle an exact cylinder (no `segments` — the mesh
/// tier's `mesh_extrude` tessellates instead).
///
/// # Returns
///
/// The prism: the profile swept along `direction`.
///
/// # Panics
///
/// Panics when the profile is degenerate or non-planar at tolerance, the
/// direction lies in the profile plane, the profile polygon is
/// self-intersecting, or the kernel refuses.
///
/// # Examples
///
/// ```cic
/// ring = circle(radius=2.0)
/// up = unit_z(factor=5.0)
/// prism = extrude(profile=ring, direction=up)
/// ```
#[node(
    category = "Surface & solid",
    tier = "S",
    version = 1,
    gh = "Extrude",
    uses_tolerance
)]
#[must_use]
pub fn extrude(config: &ProjectConfig, input: ExtrudeIn) -> Solid {
    red(cicada_geom::solid::extrude(
        &input.profile.0,
        input.direction,
        config.tol(),
    ))
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use cicada_core::geometry::{Circle, Rectangle};
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::{Plane, Point};

    use super::*;
    use crate::solids::support::{
        close_rel, config, platform_golden, ring, solid_hash, unit_square_profile, volume_of,
        with_kernel,
    };

    #[test]
    fn extrude_table_cases() {
        let Some(prism) = with_kernel(|| {
            extrude(
                &config(),
                ExtrudeIn {
                    profile: unit_square_profile(),
                    direction: Vector::new(0.0, 0.0, 2.0),
                },
            )
        }) else {
            return;
        };
        assert!(close_rel(volume_of(&prism), 2.0, 1e-12));
        // A circle extrudes to an exact cylinder.
        let cylinder = extrude(
            &config(),
            ExtrudeIn {
                profile: Closed(Curve::Circle(Circle {
                    plane: Plane::world_xy(),
                    radius: 2.0,
                })),
                direction: Vector::new(0.0, 0.0, 5.0),
            },
        );
        assert!(close_rel(volume_of(&cylinder), PI * 4.0 * 5.0, 1e-9));
        // A non-convex polyline (an L) keeps its area × height.
        let ell = extrude(
            &config(),
            ExtrudeIn {
                profile: ring(
                    &[
                        (0.0, 0.0),
                        (3.0, 0.0),
                        (3.0, 1.0),
                        (1.0, 1.0),
                        (1.0, 3.0),
                        (0.0, 3.0),
                    ],
                    0.0,
                ),
                direction: Vector::new(0.0, 0.0, 2.0),
            },
        );
        assert!(close_rel(volume_of(&ell), 5.0 * 2.0, 1e-12));
        // Downward and oblique both work: |base × normal height|.
        let down = extrude(
            &config(),
            ExtrudeIn {
                profile: unit_square_profile(),
                direction: Vector::new(0.5, 0.25, -3.0),
            },
        );
        assert!(close_rel(volume_of(&down), 3.0, 1e-12));
    }

    #[test]
    #[should_panic(expected = "profile plane")]
    fn extrude_in_plane_direction_is_red() {
        let _ = extrude(
            &config(),
            ExtrudeIn {
                profile: unit_square_profile(),
                direction: Vector::new(1.0, 0.0, 0.0),
            },
        );
    }

    #[test]
    #[should_panic(expected = "not planar")]
    fn extrude_non_planar_profile_is_red() {
        let _ = extrude(
            &config(),
            ExtrudeIn {
                profile: Closed(Curve::Polyline(cicada_core::geometry::Polyline {
                    vertices: vec![
                        Point::new(0.0, 0.0, 0.0),
                        Point::new(2.0, 0.0, 0.0),
                        Point::new(2.0, 2.0, 1.0),
                        Point::new(0.0, 2.0, 0.0),
                    ],
                    closed: true,
                })),
                direction: Vector::new(0.0, 0.0, 1.0),
            },
        );
    }

    proptest::proptest! {
        // Oblique prisms included: volume = base area × normal height for
        // any shear (Cavalieri).
        #[test]
        fn property_extrude_prism_volume(
            dx in 0.1..10.0_f64, dy in 0.1..10.0_f64,
            sx in -3.0..3.0_f64, sy in -3.0..3.0_f64,
            h in 0.1..10.0_f64,
        ) {
            if cicada_geom::solid::kernel_available() {
                let out = extrude(
                    &config(),
                    ExtrudeIn {
                        profile: Closed(Curve::Rectangle(Rectangle {
                            plane: Plane::world_xy(),
                            x: Domain::new(0.0, dx),
                            y: Domain::new(0.0, dy),
                        })),
                        direction: Vector::new(sx, sy, h),
                    },
                );
                proptest::prop_assert!(close_rel(volume_of(&out), dx * dy * h, 1e-9));
            }
        }
    }

    #[test]
    fn extrude_determinism_golden_hash() {
        // Oblique rectangle prism: pure arithmetic (corners + a shear).
        // Blessed via run-once on win-64 (2026-08-20).
        let Some(prism) = with_kernel(|| {
            extrude(
                &config(),
                ExtrudeIn {
                    profile: Closed(Curve::Rectangle(Rectangle {
                        plane: Plane::world_xy(),
                        x: Domain::new(0.0, 1.0),
                        y: Domain::new(0.0, 2.0),
                    })),
                    direction: Vector::new(0.25, 0.0, 3.0),
                },
            )
        }) else {
            return;
        };
        assert_eq!(
            solid_hash(&prism),
            platform_golden("fa6923a27d9f354630b14acb7e54358290d7dd4f76746b5ef1105884e64f94fc")
        );
    }
}
