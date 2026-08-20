//! The `xy_plane` node.

use cicada_core::spatial::Plane;
use cicada_macros::node;

use super::WorldPlaneIn;

/// XY Plane — the world XY frame at an origin.
///
/// # Examples
///
/// ```cic
/// at = construct_point(x=5.0, y=6.0, z=7.0)
/// frame = xy_plane(origin=at)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "S",
    version = 1,
    gh = "XY Plane"
)]
#[must_use]
pub fn xy_plane(input: WorldPlaneIn) -> Plane {
    Plane {
        origin: input.origin,
        ..Plane::world_xy()
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact coordinate pass-through is the contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;
    use cicada_core::spatial::{Point, Vector};

    #[test]
    fn xy_plane_table() {
        let at = Point::new(5.0, 6.0, 7.0);
        let xy = xy_plane(WorldPlaneIn { origin: at });
        assert_eq!(xy.origin, at);
        assert_eq!(xy.x, Vector::new(1.0, 0.0, 0.0));
        assert_eq!(xy.y, Vector::new(0.0, 1.0, 0.0));
    }

    proptest::proptest! {
        // The world-plane node carries the origin through exactly and
        // keeps its fixed world axes.
        #[test]
        fn property_xy_plane_carries_origin(
            x in -1.0e9..1.0e9_f64,
            y in -1.0e9..1.0e9_f64,
            z in -1.0e9..1.0e9_f64,
        ) {
            let origin = Point::new(x, y, z);
            let input = WorldPlaneIn { origin };
            proptest::prop_assert_eq!(
                xy_plane(input),
                Plane { origin, ..Plane::world_xy() }
            );
        }
    }

    // Golden hash of one representative output, arithmetic-exact input
    // (blessed via run-once).
    #[test]
    fn xy_plane_determinism_golden_hash() {
        let hash = |data: ValueData| HashedValue::new(data).unwrap().hash().to_hex();
        let at = Point::new(5.0, 6.0, 7.0);
        assert_eq!(
            hash(ValueData::Plane(xy_plane(WorldPlaneIn { origin: at }))),
            "0f65ed040fc802a10e2801932f6d6860f516f2b97d87305adb5f33292ccebc44"
        );
    }
}
