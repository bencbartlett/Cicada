//! The `extrude` node.

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::{Closed, Curve, Mesh, Watertight};
use cicada_core::spatial::Vector;
use cicada_macros::{Ports, node};

use crate::{PRISM_BYTES_PER_PROFILE_VERTEX, checked_count, red};

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
/// # Returns
///
/// The watertight prism: the profile swept along `direction`.
///
/// # Panics
///
/// Panics when the profile is degenerate or non-planar at tolerance, the
/// direction lies in the profile plane, `segments < 3`, the profile
/// polygon is self-intersecting, or — for a circle profile, the one that
/// tessellates to `segments` vertices — `segments` is above the shared
/// ceilings (2^22 slots, or 1 GiB of prism at 96 bytes a profile vertex;
/// 4,194,304 is the last allowed; the message names the count and the
/// ceiling that bit).
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
    version = 2,
    gh = "Extrude",
    uses_tolerance
)]
#[must_use]
pub fn extrude(config: &ProjectConfig, input: ExtrudeIn) -> Watertight<Mesh> {
    // Only a circle profile tessellates to `segments` vertices (a polyline
    // or rectangle is its own corner chain, the port unused), so only a
    // circle's `segments` sizes an allocation; the kernel keeps the floor
    // (`segments < 3`) for every profile.
    if matches!(input.profile.0, Curve::Circle(_)) && input.segments >= 3 {
        let _ = checked_count(
            "extrude",
            "segments",
            input.segments,
            3,
            PRISM_BYTES_PER_PROFILE_VERTEX,
        );
    }
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

    fn unit_circle_profile() -> Closed<Curve> {
        Closed(Curve::Circle(cicada_core::geometry::Circle {
            plane: Plane::world_xy(),
            radius: 1.0,
        }))
    }

    /// One profile vertex past the ceiling that bites first for a prism:
    /// the slot ceiling (2^22 vertices at 96 bytes is 384 MiB, under the
    /// byte ceiling).
    fn one_past_the_prism_ceiling() -> i64 {
        let bytes = u64::try_from(PRISM_BYTES_PER_PROFILE_VERTEX).unwrap();
        assert!(u64::try_from(crate::MAX_SLOTS).unwrap() * bytes <= crate::MAX_BYTES);
        crate::MAX_SLOTS + 1
    }

    #[test]
    #[should_panic(expected = "segments = 2 is out of range: must be >= 3")]
    fn extrude_too_few_segments_is_red() {
        let _ = extrude(
            &config(),
            ExtrudeIn {
                profile: unit_circle_profile(),
                direction: Vector::new(0.0, 0.0, 1.0),
                segments: 2,
            },
        );
    }

    // A circle profile tessellates to `segments` vertices: one past the
    // slot ceiling is red with the count and the ceiling in the message.
    // This pins where the guard sits (a guard moved after the tessellation
    // would pass it too — slowly, through the O(n²) cap clip); the absurd
    // case below is what detects that mutation.
    #[test]
    fn extrude_circle_one_past_the_ceiling_is_red() {
        let segments = one_past_the_prism_ceiling();
        assert_eq!(segments, 4_194_305);
        let panic = std::panic::catch_unwind(|| {
            extrude(
                &config(),
                ExtrudeIn {
                    profile: unit_circle_profile(),
                    direction: Vector::new(0.0, 0.0, 1.0),
                    segments,
                },
            )
        })
        .expect_err("one profile vertex past the slot ceiling refuses");
        assert_eq!(
            *panic.downcast_ref::<String>().unwrap(),
            "extrude: segments is 4194305 — above the 4194304 (2^22) slot ceiling of one node \
             output"
        );
    }

    // The absurd density a literal or an Integer wire can carry: a circle
    // at 10^11 segments is a 2.4 TB loop (`tessellate_closed` sizes its
    // collect up front) no machine holds — with the guard after it this
    // test binary would abort on allocation failure (`catch_unwind` cannot
    // catch that), so passing proves the refusal precedes the allocation.
    #[test]
    #[should_panic(
        expected = "extrude: segments is 100000000000 — above the 4194304 (2^22) slot ceiling of \
                    one node output"
    )]
    fn extrude_circle_absurd_segments_are_refused_not_allocated() {
        let _ = extrude(
            &config(),
            ExtrudeIn {
                profile: unit_circle_profile(),
                direction: Vector::new(0.0, 0.0, 1.0),
                segments: 100_000_000_000,
            },
        );
    }

    // A rectangle (or polyline) profile is its own corner chain — the port
    // sizes nothing there, so the same count builds the same prism it
    // always did (the guard is on the allocation, not the port).
    #[test]
    fn extrude_chain_profile_ignores_segments_as_before() {
        let prism = extrude(
            &config(),
            ExtrudeIn {
                profile: unit_square_profile(),
                direction: Vector::new(0.0, 0.0, 2.0),
                segments: one_past_the_prism_ceiling(),
            },
        );
        assert_eq!(prism.0.triangle_count(), 12);
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
