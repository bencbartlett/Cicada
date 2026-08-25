//! The `center_box` node — the B-rep box centred on its plane, OCCT-backed
//! like `box` (catalog C2b; DECISIONS.md row 42: B-rep is the default
//! working mode).

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Solid;
use cicada_core::scalar::Domain;
use cicada_core::spatial::Plane;
use cicada_macros::{Ports, node};

use crate::red;

/// Inputs for [`center_box`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct CenterBoxIn {
    /// The box's frame: the box is centred on its origin.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// Half-extent along the frame's x axis: the box spans `-x..x` (the
    /// default makes a 2 × 2 × 2 cube, as Grasshopper's does).
    #[port(default = 1.0, dimension = length)]
    pub x: f64,
    /// Half-extent along the frame's y axis: the box spans `-y..y`.
    #[port(default = 1.0, dimension = length)]
    pub y: f64,
    /// Half-extent along the frame's z axis: the box spans `-z..z`.
    #[port(default = 1.0, dimension = length)]
    pub z: f64,
}

/// Center Box — a B-rep box centred on a plane's origin, sized by three
/// half-extents.
///
/// `box` with the domains `-x..x`, `-y..y`, `-z..z` in the plane's frame
/// (six planar faces, exact; byte-identical to that `box`) — the box a
/// centre point and a size describe, where `box` is the box two corners
/// describe. The half-extents are Grasshopper's: `x = 1` spans two units.
///
/// # Returns
///
/// The box solid.
///
/// # Panics
///
/// Panics when a half-extent is not above tolerance (the box would be
/// flat), the plane is degenerate, or the kernel refuses.
///
/// # Examples
///
/// ```cic
/// block = center_box(x=1.5, y=1.0, z=0.5)
/// ```
#[node(
    category = "Surface & solid",
    tier = "1",
    version = 1,
    gh = "Center Box",
    uses_tolerance
)]
#[must_use]
pub fn center_box(config: &ProjectConfig, input: CenterBoxIn) -> Solid {
    // A non-positive half-extent is refused here, by name, before the
    // domains reach the kernel (the box builder would only see an empty or
    // a flipped span and name `x..y..z` domains the user never wrote).
    for (name, half) in [("x", input.x), ("y", input.y), ("z", input.z)] {
        assert!(
            half.is_finite() && half > config.tol(),
            "center_box: {name} = {half} is not above tolerance — a half-extent must be positive"
        );
    }
    red(cicada_geom::solid::box_in_plane(
        &input.plane,
        Domain::new(-input.x, input.x),
        Domain::new(-input.y, input.y),
        Domain::new(-input.z, input.z),
        config.tol(),
    ))
}

#[cfg(test)]
mod tests {
    use cicada_core::spatial::{Point, Vector};
    use cicada_geom::tol;

    use super::*;
    use crate::solids::r#box::{BoxIn, box_};
    use crate::solids::support::{
        bounds_of, close_rel, config, plane_at, platform_golden, solid_hash, volume_of, with_kernel,
    };

    #[test]
    fn center_box_table_cases() {
        // The default is Grasshopper's 2 × 2 × 2 cube about the origin.
        let Some(cube) = with_kernel(|| {
            center_box(
                &config(),
                CenterBoxIn {
                    plane: Plane::world_xy(),
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                },
            )
        }) else {
            return;
        };
        assert!(close_rel(volume_of(&cube), 8.0, 1e-12));
        let (min, max) = bounds_of(&cube);
        assert!(tol::coincident(min, Point::new(-1.0, -1.0, -1.0), 1e-9));
        assert!(tol::coincident(max, Point::new(1.0, 1.0, 1.0), 1e-9));
        // Centred on an offset plane: the bounds straddle its origin.
        let block = center_box(
            &config(),
            CenterBoxIn {
                plane: plane_at(10.0, 20.0, 30.0),
                x: 1.5,
                y: 1.0,
                z: 0.5,
            },
        );
        let (min, max) = bounds_of(&block);
        assert!(tol::coincident(min, Point::new(8.5, 19.0, 29.5), 1e-9));
        assert!(tol::coincident(max, Point::new(11.5, 21.0, 30.5), 1e-9));
        assert!(close_rel(volume_of(&block), 3.0 * 2.0 * 1.0, 1e-12));
        // A frame with permuted axes (exact): the box's x runs along world
        // y, its y along world z, so its z runs along world x.
        let turned = center_box(
            &config(),
            CenterBoxIn {
                plane: Plane {
                    origin: Point::origin(),
                    x: Vector::new(0.0, 1.0, 0.0),
                    y: Vector::new(0.0, 0.0, 1.0),
                },
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
        );
        let (min, max) = bounds_of(&turned);
        assert!(tol::coincident(min, Point::new(-3.0, -1.0, -2.0), 1e-9));
        assert!(tol::coincident(max, Point::new(3.0, 1.0, 2.0), 1e-9));
        // Byte-identical to `box` over the mirrored domains: one box, two
        // ways to describe it.
        let by_corners = box_(
            &config(),
            BoxIn {
                plane: plane_at(10.0, 20.0, 30.0),
                x: Domain::new(-1.5, 1.5),
                y: Domain::new(-1.0, 1.0),
                z: Domain::new(-0.5, 0.5),
            },
        );
        assert_eq!(solid_hash(&block), solid_hash(&by_corners));
    }

    #[test]
    #[should_panic(expected = "center_box: y = 0 is not above tolerance")]
    fn center_box_zero_half_extent_is_red() {
        // Input refusals precede the kernel call: the same red in both
        // worlds.
        let _ = center_box(
            &config(),
            CenterBoxIn {
                plane: Plane::world_xy(),
                x: 1.0,
                y: 0.0,
                z: 1.0,
            },
        );
    }

    #[test]
    #[should_panic(expected = "center_box: z = -2 is not above tolerance")]
    fn center_box_negative_half_extent_is_red() {
        let _ = center_box(
            &config(),
            CenterBoxIn {
                plane: Plane::world_xy(),
                x: 1.0,
                y: 1.0,
                z: -2.0,
            },
        );
    }

    proptest::proptest! {
        // Any placement: volume = 8xyz, and the bounds straddle the origin.
        #[test]
        fn property_center_box_volume_and_centre(
            x in 0.01f64..25.0, y in 0.01f64..25.0, z in 0.01f64..25.0,
            ox in -100.0f64..100.0, oy in -100.0f64..100.0,
        ) {
            if cicada_geom::solid::kernel_available() {
                let out = center_box(
                    &config(),
                    CenterBoxIn {
                        plane: plane_at(ox, oy, 0.0),
                        x,
                        y,
                        z,
                    },
                );
                proptest::prop_assert!(close_rel(volume_of(&out), 8.0 * x * y * z, 1e-9));
                let (min, max) = bounds_of(&out);
                let centre = Point((min.0 + max.0) * 0.5);
                proptest::prop_assert!(tol::coincident(centre, Point::new(ox, oy, 0.0), 1e-7));
            }
        }
    }

    #[test]
    fn center_box_determinism_golden_hash() {
        // The 1 × 2 × 3 box centred on the world origin: transcendental-free
        // canonical bytes. Blessed via run-once on win-64 (2026-08-24).
        let Some(block) = with_kernel(|| {
            center_box(
                &config(),
                CenterBoxIn {
                    plane: Plane::world_xy(),
                    x: 0.5,
                    y: 1.0,
                    z: 1.5,
                },
            )
        }) else {
            return;
        };
        assert_eq!(
            solid_hash(&block),
            platform_golden("8879939c993edf9bb3d0f878da845d70d674dbf7b19890e3b479d3e51a4d1378")
        );
    }
}
