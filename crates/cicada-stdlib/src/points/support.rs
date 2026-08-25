//! Input structs shared by the unit-vector, world-plane and two-vector
//! nodes, plus the test helpers the category's goldens share.

use cicada_core::spatial::{Point, Vector};
use cicada_macros::Ports;

/// Inputs for the unit-vector nodes.
#[derive(Ports, Clone, Copy, Debug)]
pub struct UnitIn {
    /// Length of the produced vector.
    #[port(default = 1.0)]
    pub factor: f64,
}

/// Inputs for the world-plane constructors.
#[derive(Ports, Clone, Copy, Debug)]
pub struct WorldPlaneIn {
    /// Plane origin.
    #[port(default = Point::origin(), default_doc = "origin")]
    pub origin: Point,
}

/// Inputs shared by the two-vector nodes (`cross_product`, `dot_product`,
/// `angle`).
#[derive(Ports, Clone, Copy, Debug)]
pub struct VectorPairIn {
    /// First vector.
    pub a: Vector,
    /// Second vector.
    pub b: Vector,
}

/// Test helpers shared by the point, vector and plane nodes.
#[cfg(test)]
pub(crate) mod testing {
    use cicada_core::marshal::IntoValue;

    /// The blake3 hex of a sealed value — a number, an integer, a point, a
    /// vector, a plane, a list of points, an index map. The goldens hash
    /// arithmetic-exact outputs only (3-4-5 triangles, dyadic factors,
    /// IEEE-pinned special values), never a libm-dependent bit pattern.
    pub(crate) fn hex<V: IntoValue>(value: V) -> String {
        value.into_value().unwrap().hash().to_hex()
    }
}
