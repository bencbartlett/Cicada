//! The `deconstruct_vector` node.

use cicada_core::spatial::Vector;
use cicada_macros::{Ports, node};

/// Inputs for [`deconstruct_vector`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct DeconstructVectorIn {
    /// The vector.
    pub vector: Vector,
}

/// Outputs of [`deconstruct_vector`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct DeconstructVectorOut {
    /// X component.
    pub x: f64,
    /// Y component.
    pub y: f64,
    /// Z component.
    pub z: f64,
}

/// Deconstruct Vector — the x/y/z components of a vector.
///
/// # Examples
///
/// ```cic
/// lean = construct_vector(x=1.0, y=0.0, z=2.0)
/// x, y, z = deconstruct_vector(vector=lean)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "1",
    version = 1,
    gh = "Deconstruct Vector"
)]
#[must_use]
pub fn deconstruct_vector(input: DeconstructVectorIn) -> DeconstructVectorOut {
    DeconstructVectorOut {
        x: input.vector.0.x,
        y: input.vector.0.y,
        z: input.vector.0.z,
    }
}

// The construct ∘ deconstruct round-trip also lives in `construct_vector.rs`
// with the primary node; the three tests below are this node's own.
#[cfg(test)]
#[allow(clippy::float_cmp)] // exact component pass-through is the contract
mod tests {
    use super::*;
    use crate::points::construct_vector::{ConstructVectorIn, construct_vector};
    use crate::points::support::testing::hex;

    #[test]
    fn deconstruct_vector_table() {
        let parts = |vector| {
            let out = deconstruct_vector(DeconstructVectorIn { vector });
            (out.x, out.y, out.z)
        };
        assert_eq!(parts(Vector::new(0.0, 0.0, 0.0)), (0.0, 0.0, 0.0));
        assert_eq!(parts(Vector::new(1.5, -2.0, 0.25)), (1.5, -2.0, 0.25));
        assert_eq!(
            parts(Vector::new(-1.0e9, 3.0e-9, 7.0)),
            (-1.0e9, 3.0e-9, 7.0),
            "components pass through exactly, whatever their magnitude"
        );
    }

    proptest::proptest! {
        // Deconstruct then construct is the identity on the vector — the
        // inverse direction of the round-trip in `construct_vector.rs`.
        #[test]
        fn property_deconstruct_vector_roundtrip(
            x in -1.0e9..1.0e9_f64,
            y in -1.0e9..1.0e9_f64,
            z in -1.0e9..1.0e9_f64,
        ) {
            let vector = Vector::new(x, y, z);
            let out = deconstruct_vector(DeconstructVectorIn { vector });
            proptest::prop_assert_eq!(
                construct_vector(ConstructVectorIn { x: out.x, y: out.y, z: out.z }),
                vector
            );
        }
    }

    // Golden hashes: each output through the value model, arithmetic-exact
    // input (blessed via run-once).
    #[test]
    fn deconstruct_vector_determinism_golden_hash() {
        let out = deconstruct_vector(DeconstructVectorIn {
            vector: Vector::new(1.5, -2.0, 0.25),
        });
        assert_eq!(
            [hex(out.x), hex(out.y), hex(out.z)],
            [
                "193cb930efc458d6c52cd619c036f833da80d9404b8870becc567e0cbfa4ef03",
                "cc547e4fc9487f8991958b5f3d38e5a199bba3cbbdfe302c611d7f6ba944ad12",
                "71b099e9be5351c658523316836088b7b65d8d393e485cc825e0ce991ef90f01",
            ]
        );
    }
}
