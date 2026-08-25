//! The `polar_array` node.

use std::f64::consts::TAU;

use cicada_core::config::ProjectConfig;
use cicada_core::geometry::Transformable;
use cicada_core::spatial::Plane;
use cicada_geom::frame::orthonormal;
use cicada_geom::tol;
use cicada_geom::transform::Similarity;
use cicada_macros::{Ports, node};

use super::support::payload_bytes;
use crate::{checked_count, red};

/// Inputs for [`polar_array`].
#[derive(Ports, Clone, Debug)]
pub struct PolarArrayIn {
    /// The geometry to repeat.
    pub geometry: Transformable,
    /// The array's frame: copies turn about its normal through its origin.
    #[port(default = Plane::world_xy(), default_doc = "xy_plane")]
    pub plane: Plane,
    /// Number of copies (the first sits at the original position).
    pub count: i64,
    /// The sweep in radians, right-handed about the plane's normal: a full
    /// turn (the default) spaces the copies evenly around the circle; any
    /// other sweep is filled from the original to its end, the last copy on
    /// the end. Negative turns the other way.
    #[port(default = TAU, default_doc = "2π", dimension = angle)]
    pub angle: f64,
}

/// Polar Array — `count` copies turned about a plane's normal, spread over
/// a sweep.
///
/// The fence-post rule, spelled out: over a full turn (`|angle|` within
/// angular tolerance of 2π — the default sweep exactly) the copies sit at
/// `k × angle / count`, so the last one does not land on the first; over
/// any shorter sweep they sit at `k × angle / (count − 1)` — the first at
/// the original, the last exactly on the sweep's end. That is Rhino's
/// `ArrayPolar` rule (its "angle to fill"); Grasshopper's Polar Array
/// component divides EVERY sweep by `count` instead (RH-81087: ten copies
/// over 90° end at 81°), so a `.gh` import translates the angle. The
/// switch sits at the tolerance, not at a looser band: a typed
/// approximation of the full turn (`angle=6.28`) is a shorter sweep and
/// fills, its last copy a hair before the first — omit `angle` for the
/// full turn (a wider band would be an arbitrary constant, or, scaled to
/// the step, a count-dependent cliff at an unremarkable angle). A zero
/// sweep stacks every copy on the original; one copy is the original
/// alone. A Solid moves through the kernel like every similarity.
///
/// # Returns
///
/// The `count` copies, the original first, in turning order.
///
/// # Panics
///
/// Panics when `count < 1`, or when `count` is above the shared ceilings
/// (2^22 slots, or 1 GiB of copies — each copy costed as its slot PLUS the
/// mesh, polyline or solid it transforms, since every copy is a distinct
/// geometry), when `|angle|` is beyond a full turn, when the plane is
/// degenerate, or for a `Solid` the OCCT kernel refuses to transform (a
/// `Solid` moves through the kernel — its B-rep geometry is rewritten,
/// never a mesh in disguise).
///
/// # Examples
///
/// ```cic
/// post = construct_point(x=3.0, y=0.0, z=0.0)
/// ring = polar_array(geometry=post, count=8)
/// ```
#[node(
    category = "Transform",
    tier = "1",
    version = 1,
    gh = "Polar Array",
    uses_tolerance
)]
#[must_use]
pub fn polar_array(config: &ProjectConfig, input: PolarArrayIn) -> Vec<Transformable> {
    // The ceiling first: every copy is a fresh geometry, charged with its
    // payload like `linear_array`'s.
    let count = checked_count(
        "polar_array",
        "count",
        input.count,
        1,
        size_of::<Transformable>() + payload_bytes(&input.geometry),
    );
    let sweep = input.angle.abs();
    let full_turn = tol::close(sweep, TAU, config.tol_angle());
    assert!(
        full_turn || sweep < TAU,
        "polar_array: angle {} sweeps beyond a full turn (2π) — a polar array fills at most the \
         circle",
        input.angle
    );
    let frame = red(orthonormal(&input.plane, config.tol()));
    // Over a full turn the copies divide the circle; over less they fill the
    // sweep from the original to its end (count − 1 gaps). One copy is the
    // original whatever the sweep.
    let step = if count == 1 {
        0.0
    } else if full_turn {
        #[allow(clippy::cast_precision_loss)] // counts stay below 2^22
        let gaps = count as f64;
        input.angle / gaps
    } else {
        #[allow(clippy::cast_precision_loss)] // counts stay below 2^22
        let gaps = (count - 1) as f64;
        input.angle / gaps
    };
    (0..count)
        .map(|k| {
            #[allow(clippy::cast_precision_loss)] // counts stay below 2^22
            let turn = step * k as f64;
            Similarity::rotation(&frame, turn).apply(&input.geometry)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, PI};

    use cicada_core::spatial::Point;
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;
    use crate::transform::support::{config, expect_point, point};

    /// A hand-typed approximation of the full turn — 3.2e-3 rad short of
    /// 2π, far outside the 1e-9 angular tolerance: the band's OUTSIDE.
    #[allow(clippy::approx_constant)] // the typed approximation IS the case
    const TYPED: f64 = 6.28;

    fn ring(count: i64, angle: f64) -> Vec<Transformable> {
        polar_array(
            &config(),
            PolarArrayIn {
                geometry: point(2.0, 0.0, 0.0),
                plane: Plane::world_xy(),
                count,
                angle,
            },
        )
    }

    #[test]
    fn polar_array_table() {
        // A full turn divides the circle: four copies a quarter turn apart,
        // the last NOT on the first.
        let full = ring(4, TAU);
        assert_eq!(full.len(), 4);
        let want = [
            Point::new(2.0, 0.0, 0.0),
            Point::new(0.0, 2.0, 0.0),
            Point::new(-2.0, 0.0, 0.0),
            Point::new(0.0, -2.0, 0.0),
        ];
        for (copy, want) in full.iter().zip(want) {
            assert!(tol::coincident(expect_point(copy), want, 1e-12), "{copy:?}");
        }
        // A half turn is FILLED: the last copy on the sweep's end.
        let half = ring(3, PI);
        assert!(tol::coincident(
            expect_point(&half[1]),
            Point::new(0.0, 2.0, 0.0),
            1e-12
        ));
        assert!(tol::coincident(
            expect_point(&half[2]),
            Point::new(-2.0, 0.0, 0.0),
            1e-12
        ));
        // A negative sweep turns the other way.
        let back = ring(2, -FRAC_PI_2);
        assert!(tol::coincident(
            expect_point(&back[1]),
            Point::new(0.0, -2.0, 0.0),
            1e-12
        ));
        // One copy is the original, whatever the sweep.
        let one = ring(1, 1.0);
        assert_eq!(one.len(), 1);
        assert!(tol::coincident(
            expect_point(&one[0]),
            Point::new(2.0, 0.0, 0.0),
            1e-12
        ));
        // A full turn the other way divides the circle the other way.
        let reverse = ring(4, -TAU);
        assert!(tol::coincident(
            expect_point(&reverse[1]),
            Point::new(0.0, -2.0, 0.0),
            1e-12
        ));
        // The full-turn band is the angular tolerance, pinned on both sides:
        // a hair inside (1e-10 rad short of 2π) divides the circle; a typed
        // approximation (6.28, 3.2e-3 rad short) is a shorter sweep and
        // FILLS — its second copy a third of the way, its last a hair before
        // the first. The cliff is the contract, not an accident (C2b review).
        let inside = ring(4, TAU - 1e-10);
        assert!(tol::coincident(
            expect_point(&inside[1]),
            Point::new(0.0, 2.0, 0.0),
            1e-9
        ));
        let typed = ring(4, TYPED);
        let third = TYPED / 3.0;
        assert!(tol::coincident(
            expect_point(&typed[1]),
            Point::new(2.0 * third.cos(), 2.0 * third.sin(), 0.0),
            1e-12
        ));
        assert!(tol::coincident(
            expect_point(&typed[3]),
            Point::new(2.0 * TYPED.cos(), 2.0 * TYPED.sin(), 0.0),
            1e-12
        ));
        assert!(
            (expect_point(&typed[3]).0 - expect_point(&typed[0]).0).length() < 0.01,
            "the typed approximation's last copy lands a hair before the first"
        );
        // A zero sweep stacks every copy on the original.
        let stacked = ring(3, 0.0);
        assert_eq!(stacked.len(), 3);
        for copy in &stacked {
            assert!(
                tol::coincident(expect_point(copy), Point::new(2.0, 0.0, 0.0), 1e-12),
                "{copy:?}"
            );
        }
        // About an offset frame with a turned normal: the copies keep their
        // distance to the frame's origin.
        let tilted = Plane {
            origin: Point::new(1.0, 1.0, 0.0),
            x: cicada_core::spatial::Vector::new(1.0, 0.0, 0.0),
            y: cicada_core::spatial::Vector::new(0.0, 0.0, 1.0),
        };
        let about = polar_array(
            &config(),
            PolarArrayIn {
                geometry: point(4.0, 1.0, 0.0),
                plane: tilted,
                count: 5,
                angle: TAU,
            },
        );
        for copy in &about {
            let distance = (expect_point(copy).0 - Point::new(1.0, 1.0, 0.0).0).length();
            assert!((distance - 3.0).abs() < 1e-9, "{copy:?}");
        }
    }

    #[test]
    #[should_panic(expected = "count must be >= 1")]
    fn polar_array_zero_count_is_red() {
        let _ = ring(0, TAU);
    }

    #[test]
    #[should_panic(expected = "sweeps beyond a full turn")]
    fn polar_array_beyond_a_full_turn_is_red() {
        let _ = ring(3, 7.0);
    }

    // The absurd count: with the guard after the copies the test binary
    // would abort on allocation failure; passing proves the refusal precedes
    // the allocation (the conformance suite holds every guarded node to it).
    #[test]
    #[should_panic(
        expected = "polar_array: count is 100000000000 — above the 4194304 (2^22) slot ceiling"
    )]
    fn polar_array_absurd_count_is_refused_not_allocated() {
        let _ = ring(100_000_000_000, TAU);
    }

    proptest::proptest! {
        // Every copy keeps the original's distance to the axis and its
        // height along it; over a shorter sweep (these never reach the
        // full-turn band) the copies FILL it: the second one step in, the
        // last exactly on the sweep's end.
        #[test]
        fn property_polar_array_spacing(
            count in 1i64..24, radius in 0.5..50.0_f64, z in -5.0..5.0_f64,
            angle in -6.0..6.0_f64,
        ) {
            let copies = polar_array(
                &config(),
                PolarArrayIn {
                    geometry: point(radius, 0.0, z),
                    plane: Plane::world_xy(),
                    count,
                    angle,
                },
            );
            proptest::prop_assert_eq!(copies.len(), usize::try_from(count).unwrap());
            for copy in &copies {
                let p = expect_point(copy);
                proptest::prop_assert!((p.0.truncate().length() - radius).abs() <= 1e-9 * radius);
                proptest::prop_assert!((p.0.z - z).abs() <= 1e-12);
            }
            if count > 1 {
                #[allow(clippy::cast_precision_loss)] // test counts are tiny
                let step = angle / (count - 1) as f64;
                let last = expect_point(copies.last().unwrap());
                let want = Point::new(radius * angle.cos(), radius * angle.sin(), z);
                proptest::prop_assert!(tol::coincident(last, want, 1e-9 * radius.max(1.0)));
                let second = expect_point(&copies[1]);
                let want = Point::new(radius * step.cos(), radius * step.sin(), z);
                proptest::prop_assert!(tol::coincident(second, want, 1e-9 * radius.max(1.0)));
            }
        }

        // The band's inside, which the filled-sweep property never reaches:
        // over a full turn either way round, copy k sits at k × angle /
        // count — the last one step short of the first, never on it.
        #[test]
        fn property_polar_array_full_turn_divides_the_circle(
            count in 1i64..24, radius in 0.5..50.0_f64, z in -5.0..5.0_f64,
            backwards in proptest::bool::ANY,
        ) {
            let angle = if backwards { -TAU } else { TAU };
            let copies = polar_array(
                &config(),
                PolarArrayIn {
                    geometry: point(radius, 0.0, z),
                    plane: Plane::world_xy(),
                    count,
                    angle,
                },
            );
            proptest::prop_assert_eq!(copies.len(), usize::try_from(count).unwrap());
            #[allow(clippy::cast_precision_loss)] // test counts are tiny
            let step = angle / count as f64;
            for (k, copy) in copies.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)] // test counts are tiny
                let turn = step * k as f64;
                let want = Point::new(radius * turn.cos(), radius * turn.sin(), z);
                proptest::prop_assert!(
                    tol::coincident(expect_point(copy), want, 1e-9 * radius.max(1.0)),
                    "{:?} vs {:?}", copy, want
                );
            }
        }
    }

    #[test]
    fn polar_array_determinism_golden_hash() {
        // A zero sweep: every copy is the original (sin 0 = 0, cos 0 = 1
        // exactly — transcendental-free, support.rs); the full-turn ring is
        // held to run-to-run identity.
        let copies = ring(3, 0.0);
        let hash = |copies: Vec<Transformable>| {
            let slots = copies
                .into_iter()
                .map(|copy| {
                    let Transformable::Point(p) = copy else {
                        panic!("points stay points")
                    };
                    Some(HashedValue::new(ValueData::Point(p)).unwrap())
                })
                .collect();
            HashedValue::new(ValueData::List(cicada_core::value::List {
                axis: None,
                slots,
            }))
            .unwrap()
            .hash()
            .to_hex()
        };
        assert_eq!(
            hash(copies),
            "14a90a63704e7fc02c1e346455c30faf47d80f1c04de2df05886096931f57719"
        );
        assert_eq!(hash(ring(6, TAU)), hash(ring(6, TAU)));
    }
}
