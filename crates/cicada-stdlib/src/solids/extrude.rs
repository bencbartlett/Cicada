//! The `extrude` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve, Mesh, Watertight};
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
    /// Tessellation density for curved profiles (circles).
    #[port(default = 64)]
    pub segments: i64,
}

/// Extrude — extrude a closed planar profile into a watertight prism
/// (mesh-backed under its v0.1 name, doc 15).
///
/// # Panics
///
/// Panics when the profile is degenerate or non-planar at tolerance, the
/// direction lies in the profile plane, `segments < 3`, or the profile
/// polygon is self-intersecting.
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
pub fn extrude(config: &ProjectConfig, input: ExtrudeIn) -> Watertight<Mesh> {
    Watertight(red(cicada_geom::meshbuild::extrude(
        &input.profile.0,
        input.direction,
        input.segments,
        config.tol(),
    )))
}

#[cfg(test)]
mod tests {
    use cicada_core::geometry::Rectangle;
    use cicada_core::scalar::Domain;
    use cicada_core::spatial::Plane;
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::meshbuild::signed_volume;

    use super::*;
    use crate::solids::support::{config, unit_square_profile};

    #[test]
    fn extrude_is_watertight_with_expected_volume() {
        let prism = extrude(
            &config(),
            ExtrudeIn {
                profile: unit_square_profile(),
                direction: Vector::new(0.0, 0.0, 2.0),
                segments: 64,
            },
        );
        assert!((signed_volume(&prism.0) - 2.0).abs() < 1e-9);
    }

    #[test]
    #[should_panic(expected = "profile plane")]
    fn extrude_in_plane_direction_is_red() {
        let _ = extrude(
            &config(),
            ExtrudeIn {
                profile: unit_square_profile(),
                direction: Vector::new(1.0, 0.0, 0.0),
                segments: 64,
            },
        );
    }

    proptest::proptest! {
        // Oblique prisms included: volume = base area × normal height for
        // any shear (Cavalieri), watertight always.
        #[test]
        fn property_extrude_prism_volume(
            dx in 0.1..10.0_f64, dy in 0.1..10.0_f64,
            sx in -3.0..3.0_f64, sy in -3.0..3.0_f64,
            h in 0.1..10.0_f64,
        ) {
            let out = extrude(
                &config(),
                ExtrudeIn {
                    profile: Closed(Curve::Rectangle(Rectangle {
                        plane: Plane::world_xy(),
                        x: Domain::new(0.0, dx),
                        y: Domain::new(0.0, dy),
                    })),
                    direction: Vector::new(sx, sy, h),
                    segments: 8,
                },
            );
            proptest::prop_assert!(out.0.is_watertight());
            let want = dx * dy * h;
            proptest::prop_assert!((signed_volume(&out.0) - want).abs() <= 1e-9 * want.max(1.0));
        }
    }

    #[test]
    fn extrude_determinism_golden_hash() {
        // Oblique rectangle prism: pure arithmetic (corner lerps + shear).
        let prism = extrude(
            &config(),
            ExtrudeIn {
                profile: Closed(Curve::Rectangle(Rectangle {
                    plane: Plane::world_xy(),
                    x: Domain::new(0.0, 1.0),
                    y: Domain::new(0.0, 2.0),
                })),
                direction: Vector::new(0.25, 0.0, 3.0),
                segments: 8,
            },
        );
        let sealed = HashedValue::new(ValueData::Mesh(prism.0)).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            "6d59e4bbc7472fc06575a8c88c96be3bedbf7ac45adecad0d0ec5cb84f0d42db"
        );
    }
}
