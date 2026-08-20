//! Input structs shared by the unit-vector and world-plane nodes.

use cicada_core::spatial::Point;
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
