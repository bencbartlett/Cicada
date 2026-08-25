//! The `construct_vector` node.

use cicada_core::spatial::Vector;
use cicada_macros::{Ports, node};

/// Inputs for [`construct_vector`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct ConstructVectorIn {
    /// X component.
    #[port(default = 0.0, dimension = length)]
    pub x: f64,
    /// Y component.
    #[port(default = 0.0, dimension = length)]
    pub y: f64,
    /// Z component.
    #[port(default = 0.0, dimension = length)]
    pub z: f64,
}

/// Vector XYZ — a vector from x/y/z components.
///
/// # Returns
///
/// The vector (x, y, z).
///
/// # Examples
///
/// ```cic
/// lean = construct_vector(x=1.0, y=0.0, z=2.0)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "1",
    version = 1,
    gh = "Vector XYZ"
)]
#[must_use]
pub fn construct_vector(input: ConstructVectorIn) -> Vector {
    Vector::new(input.x, input.y, input.z)
}

// The construct/deconstruct round-trip tests exercise both vector nodes and
// live here, with the primary node; `deconstruct_vector.rs` holds its own
// three as well.
#[cfg(test)]
#[allow(clippy::float_cmp)] // exact component pass-through is the contract
mod tests {
    use super::*;
    use crate::points::deconstruct_vector::{DeconstructVectorIn, deconstruct_vector};
    use crate::points::support::testing::hex;

    #[test]
    fn construct_vector_table() {
        let v = |x, y, z| construct_vector(ConstructVectorIn { x, y, z });
        assert_eq!(v(0.0, 0.0, 0.0), Vector::new(0.0, 0.0, 0.0));
        assert_eq!(v(1.5, -2.0, 0.25), Vector::new(1.5, -2.0, 0.25));
        assert_eq!(
            v(-1.0e9, 3.0e-9, 7.0),
            Vector::new(-1.0e9, 3.0e-9, 7.0),
            "components pass through exactly, whatever their magnitude"
        );
        // The defaults are the zero vector (every component 0.0).
        assert_eq!(v(0.0, 0.0, 0.0).0, glam::DVec3::ZERO);
    }

    proptest::proptest! {
        // Construct then deconstruct is the identity on the components.
        #[test]
        fn property_construct_vector_roundtrip(
            x in -1.0e9..1.0e9_f64,
            y in -1.0e9..1.0e9_f64,
            z in -1.0e9..1.0e9_f64,
        ) {
            let vector = construct_vector(ConstructVectorIn { x, y, z });
            let out = deconstruct_vector(DeconstructVectorIn { vector });
            proptest::prop_assert_eq!((out.x, out.y, out.z), (x, y, z));
        }
    }

    // Golden hash of one representative output, arithmetic-exact input
    // (blessed via run-once).
    #[test]
    fn construct_vector_determinism_golden_hash() {
        assert_eq!(
            hex(construct_vector(ConstructVectorIn {
                x: 1.5,
                y: -2.0,
                z: 0.25,
            })),
            "ab992585e08e454ce8c0a8ba01021d5911170e8b52afa2b8ac6bedb5983f4ab9"
        );
    }
}
