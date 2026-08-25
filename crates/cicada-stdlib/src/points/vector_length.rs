//! The `vector_length` node.

use cicada_core::spatial::Vector;
use cicada_macros::{Ports, node};

/// Inputs for [`vector_length`].
#[derive(Ports, Clone, Copy, Debug)]
pub struct VectorLengthIn {
    /// The vector.
    pub vector: Vector,
}

/// Vector Length — the Euclidean length of a vector.
///
/// # Returns
///
/// The length `|vector|`, in document units (0 for the zero vector).
///
/// # Examples
///
/// ```cic
/// lean = construct_vector(x=3.0, y=4.0, z=0.0)
/// span = vector_length(vector=lean)
/// ```
#[node(
    category = "Point · Vector · Plane",
    tier = "1",
    version = 1,
    gh = "Vector Length"
)]
#[must_use]
pub fn vector_length(input: VectorLengthIn) -> f64 {
    input.vector.0.length()
}

#[cfg(test)]
mod tests {
    use cicada_geom::tol;

    use super::*;
    use crate::points::support::testing::hex;

    #[test]
    fn vector_length_table() {
        let len = |x, y, z| {
            vector_length(VectorLengthIn {
                vector: Vector::new(x, y, z),
            })
        };
        assert!(tol::close(len(3.0, 4.0, 0.0), 5.0, 1e-12));
        assert!(tol::near_zero(len(0.0, 0.0, 0.0), 0.0));
        assert!(tol::close(len(0.0, 0.0, -2.5), 2.5, 1e-12));
        assert!(tol::close(len(1.0, 1.0, 1.0), 3.0_f64.sqrt(), 1e-12));
        assert!(tol::close(len(-6.0, 8.0, 0.0), 10.0, 1e-12));
    }

    proptest::proptest! {
        // A norm: non-negative, and absolutely homogeneous — |s·v| = |s|·|v|
        // (at tolerance relative to the magnitude).
        #[test]
        fn property_vector_length_is_homogeneous(
            x in -1.0e3..1.0e3_f64, y in -1.0e3..1.0e3_f64, z in -1.0e3..1.0e3_f64,
            s in -100.0..100.0_f64,
        ) {
            let len = |v: Vector| vector_length(VectorLengthIn { vector: v });
            let v = Vector::new(x, y, z);
            proptest::prop_assert!(len(v) >= 0.0);
            let scaled = Vector(v.0 * s);
            proptest::prop_assert!(tol::close(len(scaled), s.abs() * len(v), 1e-9 * (1.0 + len(v) * s.abs())));
        }
    }

    // Golden hash: the 3-4-5 length (exact sqrt), blessed via run-once.
    #[test]
    fn vector_length_determinism_golden_hash() {
        assert_eq!(
            hex(vector_length(VectorLengthIn {
                vector: Vector::new(3.0, 4.0, 0.0),
            })),
            "94eef958b3fd1d43bbe6037ff14183719f8a823fbceb89128625befb93c6ca40"
        );
    }
}
