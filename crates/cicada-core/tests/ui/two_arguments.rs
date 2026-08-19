use cicada_macros::{Ports, node};

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    pub x: f64,
}

/// Bad — takes two arguments instead of one input struct.
#[node(category = "Maths & logic", tier = "S", version = 1)]
pub fn bad(input: In, extra: f64) -> f64 {
    input.x + extra
}

fn main() {}
