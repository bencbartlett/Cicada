//! The `yz_plane` node.

use cicada_core::spatial::Plane;
use cicada_macros::node;

use super::WorldPlaneIn;

/// YZ Plane — the world YZ frame at an origin.
///
/// # Returns
///
/// The world-aligned YZ frame with its origin at `origin`.
///
/// # Examples
///
/// ```cic
/// at = construct_point(x=5.0, y=6.0, z=7.0)
/// frame = yz_plane(origin=at)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "S",
    version = 1,
    gh = "YZ Plane"
)]
#[must_use]
pub fn yz_plane(input: WorldPlaneIn) -> Plane {
    Plane {
        origin: input.origin,
        ..Plane::world_yz()
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact coordinate pass-through is the contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;
    use cicada_core::spatial::{Point, Vector};

    #[test]
    fn yz_plane_table() {
        let at = Point::new(5.0, 6.0, 7.0);
        let yz = yz_plane(WorldPlaneIn { origin: at });
        assert_eq!(yz.x, Vector::new(0.0, 1.0, 0.0));
    }

    proptest::proptest! {
        // The world-plane node carries the origin through exactly and
        // keeps its fixed world axes.
        #[test]
        fn property_yz_plane_carries_origin(
            x in -1.0e9..1.0e9_f64,
            y in -1.0e9..1.0e9_f64,
            z in -1.0e9..1.0e9_f64,
        ) {
            let origin = Point::new(x, y, z);
            let input = WorldPlaneIn { origin };
            proptest::prop_assert_eq!(
                yz_plane(input),
                Plane { origin, ..Plane::world_yz() }
            );
        }
    }

    // Golden hash of one representative output, arithmetic-exact input
    // (blessed via run-once).
    #[test]
    fn yz_plane_determinism_golden_hash() {
        let hash = |data: ValueData| HashedValue::new(data).unwrap().hash().to_hex();
        let at = Point::new(5.0, 6.0, 7.0);
        assert_eq!(
            hash(ValueData::Plane(yz_plane(WorldPlaneIn { origin: at }))),
            "b2042f66a665a6876ebc489fd83419cf03e7811727f50242cee206e969a8b5d7"
        );
    }
}
