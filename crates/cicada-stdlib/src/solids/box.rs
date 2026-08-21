//! The `box` node (`fn box_` — the dialect name is the keyword `box`):
//! the B-rep box, OCCT-backed (v0.1 item 3 WP-C; DECISIONS.md row 42:
//! B-rep is the default working mode — the spike's mesh-backed box
//! continues as `mesh_box`).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Solid;
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

/// Box — a B-rep box spanning three domains in a plane's frame (six planar
/// faces, exact; the default working mode's box — `mesh_box` is the mesh
/// tier's). Decreasing domains are normalized.
///
/// # Returns
///
/// The box solid.
///
/// # Panics
///
/// Panics when any extent is empty at tolerance, the plane is degenerate,
/// or the kernel refuses.
///
/// # Examples
///
/// ```cic
/// span = construct_domain(start=0.0, end=2.0)
/// block = box(x=span, y=span, z=span)
/// ```
// `version = 2`: the tier flip (WP-C) changed this op's OUTPUT KIND under
// an unchanged name and port list — Watertight<Mesh> (the spike's box) →
// Solid — and the memo key is (op, version, tolerance, input hashes): at
// version 1 a store warmed by a pre-flip engine served the mesh box for the
// Solid-typed node, green (the review's blocker). Old caches recompute once.
#[node(
    category = "Surface & solid",
    tier = "S",
    version = 2,
    gh = "Domain Box",
    uses_tolerance
)]
#[must_use]
pub fn box_(config: &ProjectConfig, input: BoxIn) -> Solid {
    red(cicada_geom::solid::box_in_plane(
        &input.plane,
        input.x,
        input.y,
        input.z,
        config.tol(),
    ))
}

#[cfg(test)]
mod tests {
    use cicada_core::spatial::{Point, Vector};
    use cicada_geom::tol;

    use super::*;
    use crate::solids::support::{
        bounds_of, close_rel, config, plane_at, platform_golden, solid_hash, volume_of, with_kernel,
    };

    #[test]
    fn box_table_cases() {
        let Some(cube) = with_kernel(|| {
            box_(
                &config(),
                BoxIn {
                    plane: Plane::world_xy(),
                    x: Domain::new(0.0, 2.0),
                    y: Domain::new(0.0, 2.0),
                    z: Domain::new(0.0, 2.0),
                },
            )
        }) else {
            return;
        };
        assert!(close_rel(volume_of(&cube), 8.0, 1e-12));
        // A decreasing domain normalizes; the min corner is the domains'
        // start in the frame.
        let flipped = box_(
            &config(),
            BoxIn {
                plane: plane_at(1.0, 2.0, 3.0),
                x: Domain::new(4.0, 0.0),
                y: Domain::new(0.0, 1.0),
                z: Domain::new(-1.0, 1.0),
            },
        );
        let (min, max) = bounds_of(&flipped);
        assert!(tol::coincident(min, Point::new(1.0, 2.0, 2.0), 1e-9));
        assert!(tol::coincident(max, Point::new(5.0, 3.0, 4.0), 1e-9));
        // A frame with permuted axes (exact): x along world y, y along
        // world z, so the box's z runs along world x.
        let turned = box_(
            &config(),
            BoxIn {
                plane: Plane {
                    origin: Point::origin(),
                    x: Vector::new(0.0, 1.0, 0.0),
                    y: Vector::new(0.0, 0.0, 1.0),
                },
                x: Domain::new(0.0, 1.0),
                y: Domain::new(0.0, 2.0),
                z: Domain::new(0.0, 3.0),
            },
        );
        let (min, max) = bounds_of(&turned);
        assert!(tol::coincident(min, Point::origin(), 1e-9));
        assert!(
            tol::coincident(max, Point::new(3.0, 1.0, 2.0), 1e-9),
            "{max:?}"
        );
    }

    #[test]
    #[should_panic(expected = "box extent must be above tolerance")]
    fn box_empty_extent_is_red() {
        // Input refusals precede the kernel call: the same red in both
        // worlds.
        let _ = box_(
            &config(),
            BoxIn {
                plane: Plane::world_xy(),
                x: Domain::new(0.0, 1.0),
                y: Domain::new(2.0, 2.0),
                z: Domain::new(0.0, 1.0),
            },
        );
    }

    proptest::proptest! {
        // Boxes at any placement: volume = product of extents.
        #[test]
        fn property_box_volume(
            dx in 0.01f64..50.0, dy in 0.01f64..50.0, dz in 0.01f64..50.0,
            ox in -100.0f64..100.0,
        ) {
            if cicada_geom::solid::kernel_available() {
                let out = box_(
                    &config(),
                    BoxIn {
                        plane: plane_at(ox, 0.0, 0.0),
                        x: Domain::new(0.0, dx),
                        y: Domain::new(0.0, dy),
                        z: Domain::new(0.0, dz),
                    },
                );
                let want = dx * dy * dz;
                proptest::prop_assert!(close_rel(volume_of(&out), want, 1e-9));
            }
        }
    }

    #[test]
    fn box_determinism_golden_hash() {
        // The probe's 1 × 2 × 3 box in the world frame: transcendental-free
        // canonical bytes. Blessed via run-once on win-64 (2026-08-20).
        let Some(block) = with_kernel(|| {
            box_(
                &config(),
                BoxIn {
                    plane: Plane::world_xy(),
                    x: Domain::new(0.0, 1.0),
                    y: Domain::new(0.0, 2.0),
                    z: Domain::new(0.0, 3.0),
                },
            )
        }) else {
            return;
        };
        assert_eq!(
            solid_hash(&block),
            platform_golden("2cd192d819ac8e052a47658c65e323883485a996c32a35bb8c69bf1f3e0bffce")
        );
    }
}
