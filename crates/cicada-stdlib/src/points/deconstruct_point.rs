//! The `deconstruct_point` node.

use cicada_core::spatial::Point;
use cicada_macros::{Ports, node};

/// Inputs for [`deconstruct_point`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct DeconstructPointIn {
    /// The point.
    pub point: Point,
}

/// Outputs of [`deconstruct_point`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct DeconstructPointOut {
    /// X coordinate.
    pub x: f64,
    /// Y coordinate.
    pub y: f64,
    /// Z coordinate.
    pub z: f64,
}

/// Deconstruct Point — the x/y/z coordinates of a point.
///
/// # Examples
///
/// ```cic
/// corner = construct_point(x=1.0, y=2.0, z=0.5)
/// x, y, z = deconstruct_point(point=corner)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "S",
    version = 1,
    gh = "Deconstruct"
)]
#[must_use]
pub fn deconstruct_point(input: DeconstructPointIn) -> DeconstructPointOut {
    DeconstructPointOut {
        x: input.point.0.x,
        y: input.point.0.y,
        z: input.point.0.z,
    }
}

// Table and property coverage: the construct/deconstruct round-trip tests
// in `construct_point.rs` exercise this node.
// The construct ∘ deconstruct round-trip also lives in `construct_point.rs`
// with the primary node; the three tests below are this node's own.
#[cfg(test)]
#[allow(clippy::float_cmp)] // exact coordinate pass-through is the contract
mod tests {
    use cicada_core::value::{HashedValue, ValueData};

    use super::*;
    use crate::points::construct_point::{ConstructPointIn, construct_point};

    #[test]
    fn deconstruct_point_table_cases() {
        let coords = |point| {
            let out = deconstruct_point(DeconstructPointIn { point });
            (out.x, out.y, out.z)
        };
        assert_eq!(coords(Point::origin()), (0.0, 0.0, 0.0));
        assert_eq!(coords(Point::new(1.5, -2.0, 0.25)), (1.5, -2.0, 0.25));
        assert_eq!(
            coords(Point::new(-1.0e9, 3.0e-9, 7.0)),
            (-1.0e9, 3.0e-9, 7.0),
            "coordinates pass through exactly, whatever their magnitude"
        );
    }

    proptest::proptest! {
        // Deconstruct then construct is the identity on the point — the
        // inverse direction of the round-trip in `construct_point.rs`.
        #[test]
        fn deconstruct_point_property_roundtrip(
            x in -1.0e9..1.0e9_f64,
            y in -1.0e9..1.0e9_f64,
            z in -1.0e9..1.0e9_f64,
        ) {
            let point = Point::new(x, y, z);
            let out = deconstruct_point(DeconstructPointIn { point });
            proptest::prop_assert_eq!(
                construct_point(ConstructPointIn { x: out.x, y: out.y, z: out.z }),
                point
            );
        }
    }

    // Golden hashes: each output through the value model, arithmetic-exact
    // inputs only (blessed via run-once).
    #[test]
    fn deconstruct_point_determinism_golden_hash() {
        let hash = |data: ValueData| HashedValue::new(data).unwrap().hash().to_hex();
        let out = deconstruct_point(DeconstructPointIn {
            point: Point::new(1.5, -2.0, 0.25),
        });
        assert_eq!(
            hash(ValueData::Number(out.x)),
            "193cb930efc458d6c52cd619c036f833da80d9404b8870becc567e0cbfa4ef03"
        );
        assert_eq!(
            hash(ValueData::Number(out.y)),
            "cc547e4fc9487f8991958b5f3d38e5a199bba3cbbdfe302c611d7f6ba944ad12"
        );
        assert_eq!(
            hash(ValueData::Number(out.z)),
            "71b099e9be5351c658523316836088b7b65d8d393e485cc825e0ce991ef90f01"
        );
    }
}
