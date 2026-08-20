//! The `box` node (`fn box_` — the dialect name is the keyword `box`).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Mesh, Watertight};
use cicada_core::scalar::Domain;
use cicada_core::spatial::Plane;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`box_`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct BoxIn {
    /// The box's frame.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// Extent along the frame's x axis.
    pub x: Domain,
    /// Extent along the frame's y axis.
    pub y: Domain,
    /// Extent along the frame's z axis.
    pub z: Domain,
}

/// Box — an axis-aligned box in a plane's frame (mesh-backed under its
/// v0.1 name, doc 15). Decreasing domains are normalized.
///
/// # Panics
///
/// Panics when any extent is empty at tolerance or the plane is
/// degenerate.
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=2.0)
/// block = box(x=span, y=span, z=span)
/// ```
#[node(
    category = "Surface & solid",
    tier = "S",
    version = 1,
    gh = "Domain Box",
    uses_tolerance
)]
#[must_use]
pub fn box_(config: &ProjectConfig, input: BoxIn) -> Watertight<Mesh> {
    Watertight(red(cicada_geom::meshbuild::box_mesh(
        &input.plane,
        input.x,
        input.y,
        input.z,
        config.tol(),
    )))
}

#[cfg(test)]
mod tests {
    use cicada_core::spatial::Point;
    use cicada_core::value::{HashedValue, ValueData};
    use cicada_geom::meshbuild::signed_volume;

    use super::*;
    use crate::solids::support::config;

    #[test]
    fn box_is_watertight_with_expected_volume() {
        let cube = box_(
            &config(),
            BoxIn {
                plane: Plane::world_xy(),
                x: Domain::new(0.0, 2.0),
                y: Domain::new(0.0, 2.0),
                z: Domain::new(0.0, 2.0),
            },
        );
        assert!((signed_volume(&cube.0) - 8.0).abs() < 1e-9);
    }

    proptest::proptest! {
        // Boxes at any placement: volume = product of extents, exactly.
        #[test]
        fn property_box_volume(
            dx in 0.01f64..50.0, dy in 0.01f64..50.0, dz in 0.01f64..50.0,
            ox in -100.0f64..100.0,
        ) {
            let out = box_(
                &config(),
                BoxIn {
                    plane: Plane {
                        origin: Point::new(ox, 0.0, 0.0),
                        ..Plane::world_xy()
                    },
                    x: Domain::new(0.0, dx),
                    y: Domain::new(0.0, dy),
                    z: Domain::new(0.0, dz),
                },
            );
            let want = dx * dy * dz;
            proptest::prop_assert!((signed_volume(&out.0) - want).abs() <= 1e-9 * want.max(1.0));
        }
    }

    #[test]
    fn box_determinism_golden_hash() {
        let cube = box_(
            &config(),
            BoxIn {
                plane: Plane::world_xy(),
                x: Domain::new(0.0, 1.0),
                y: Domain::new(0.0, 2.0),
                z: Domain::new(0.0, 3.0),
            },
        );
        let sealed = HashedValue::new(ValueData::Mesh(cube.0)).unwrap();
        assert_eq!(
            sealed.hash().to_hex(),
            "3063b49cbeec12ff1b2dc909b7abe1ffbc060cd66c92f62128c89f7926e42766"
        );
    }
}
