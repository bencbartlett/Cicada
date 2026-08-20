//! The `construct_point` node.

use cicada_core::spatial::Point;
use cicada_macros::{Ports, node};

/// Inputs for [`construct_point`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct ConstructPointIn {
    /// X coordinate.
    #[port(default = 0.0, dimension = length)]
    pub x: f64,
    /// Y coordinate.
    #[port(default = 0.0, dimension = length)]
    pub y: f64,
    /// Z coordinate.
    #[port(default = 0.0, dimension = length)]
    pub z: f64,
}

/// Construct Point — a point from x/y/z coordinates.
#[node(category = "Point · Vector · Plane", tier = "S", version = 1)]
#[must_use]
pub fn construct_point(input: ConstructPointIn) -> Point {
    Point::new(input.x, input.y, input.z)
}

// The construct/deconstruct round-trip tests exercise both point nodes and
// live here.
#[cfg(test)]
#[allow(clippy::float_cmp)] // exact coordinate pass-through is the contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;
    use crate::points::deconstruct_point::{DeconstructPointIn, deconstruct_point};

    #[test]
    fn point_roundtrip_table() {
        let point = construct_point(ConstructPointIn {
            x: 1.5,
            y: -2.0,
            z: 0.25,
        });
        assert_eq!(point, Point::new(1.5, -2.0, 0.25));
        let out = deconstruct_point(DeconstructPointIn { point });
        assert_eq!((out.x, out.y, out.z), (1.5, -2.0, 0.25));
    }

    proptest::proptest! {
        // Construct/deconstruct is the exact identity.
        #[test]
        fn property_point_roundtrip(
            x in -1.0e9..1.0e9_f64,
            y in -1.0e9..1.0e9_f64,
            z in -1.0e9..1.0e9_f64,
        ) {
            let out = deconstruct_point(DeconstructPointIn {
                point: construct_point(ConstructPointIn { x, y, z }),
            });
            proptest::prop_assert_eq!((out.x, out.y, out.z), (x, y, z));
        }
    }

    #[test]
    fn determinism_golden_hash() {
        let point = construct_point(ConstructPointIn {
            x: 3.0,
            y: -4.0,
            z: 5.0,
        });
        assert_eq!(
            HashedValue::new(ValueData::Point(point))
                .unwrap()
                .hash()
                .to_hex(),
            "6c5c651282fb21573785b37b6586208778691bfc17435d1180c89f47749be416"
        );
    }
}
