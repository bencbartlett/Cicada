//! The `xz_plane` node.

use cicada_core::spatial::Plane;
use cicada_macros::node;

use super::WorldPlaneIn;

/// XZ Plane — the world XZ frame at an origin.
///
/// # Returns
///
/// The world-aligned XZ frame with its origin at `origin`.
///
/// # Examples
///
/// ```cic
/// at = construct_point(x=5.0, y=6.0, z=7.0)
/// frame = xz_plane(origin=at)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "S",
    version = 1,
    gh = "XZ Plane"
)]
#[must_use]
pub fn xz_plane(input: WorldPlaneIn) -> Plane {
    Plane {
        origin: input.origin,
        ..Plane::world_xz()
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact coordinate pass-through is the contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;
    use cicada_core::spatial::{Point, Vector};

    #[test]
    fn xz_plane_table() {
        let at = Point::new(5.0, 6.0, 7.0);
        let xz = xz_plane(WorldPlaneIn { origin: at });
        assert_eq!(xz.y, Vector::new(0.0, 0.0, 1.0));
    }

    proptest::proptest! {
        // The world-plane node carries the origin through exactly and
        // keeps its fixed world axes.
        #[test]
        fn property_xz_plane_carries_origin(
            x in -1.0e9..1.0e9_f64,
            y in -1.0e9..1.0e9_f64,
            z in -1.0e9..1.0e9_f64,
        ) {
            let origin = Point::new(x, y, z);
            let input = WorldPlaneIn { origin };
            proptest::prop_assert_eq!(
                xz_plane(input),
                Plane { origin, ..Plane::world_xz() }
            );
        }
    }

    // Golden hash of one representative output, arithmetic-exact input
    // (blessed via run-once).
    #[test]
    fn xz_plane_determinism_golden_hash() {
        let hash = |data: ValueData| HashedValue::new(data).unwrap().hash().to_hex();
        let at = Point::new(5.0, 6.0, 7.0);
        assert_eq!(
            hash(ValueData::Plane(xz_plane(WorldPlaneIn { origin: at }))),
            "e6ae5bb16a22a69b55b52449e36ad07afa2effbf4b4aae8c6ce1b9070e67635e"
        );
    }
}
