//! Shared by the arithmetic, comparison, logic, and reducer nodes.

use cicada_macros::Ports;

/// Inputs shared by the two-number arithmetic nodes.
#[derive(Ports, Clone, Copy, Debug)]
pub struct BinaryIn {
    /// Left operand.
    pub a: f64,
    /// Right operand.
    pub b: f64,
}

/// Inputs shared by the one-number nodes (`negative`, `absolute`, `round`,
/// `sqrt`, `ln`, `exp`, the inverse trig functions, …).
#[derive(Ports, Clone, Copy, Debug)]
pub struct UnaryIn {
    /// The operand.
    pub x: f64,
}

/// Inputs shared by the trigonometric functions of an angle.
#[derive(Ports, Clone, Copy, Debug)]
pub struct AngleIn {
    /// The angle, in radians.
    #[port(dimension = angle)]
    pub x: f64,
}

/// Inputs shared by the two-boolean logic gates.
#[derive(Ports, Clone, Copy, Debug)]
pub struct GateIn {
    /// Left operand.
    pub a: bool,
    /// Right operand.
    pub b: bool,
}

/// Inputs shared by the numeric reducers (`mass_addition`, `average`,
/// `bounds`).
#[derive(Ports, Clone, Debug)]
pub struct ReduceIn {
    /// The numbers to reduce.
    pub list: Vec<f64>,
}

/// Test helpers shared by the maths nodes.
#[cfg(test)]
pub(crate) mod testing {
    use cicada_core::marshal::IntoValue;

    /// The blake3 hex of a sealed value — one number, a boolean, a domain, a
    /// list of numbers. The goldens hash arithmetic-exact outputs only
    /// (dyadic values, IEEE-pinned special cases), never a libm-dependent
    /// bit pattern.
    pub(crate) fn hex<V: IntoValue>(value: V) -> String {
        value.into_value().unwrap().hash().to_hex()
    }
}
