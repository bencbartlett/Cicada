//! Shared by the two-number arithmetic nodes.

use cicada_macros::Ports;

/// Inputs shared by the two-number arithmetic nodes.
#[derive(Ports, Clone, Copy, Debug)]
pub struct BinaryIn {
    /// Left operand.
    pub a: f64,
    /// Right operand.
    pub b: f64,
}
