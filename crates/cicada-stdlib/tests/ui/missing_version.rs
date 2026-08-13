use cicada_macros::{Ports, node};

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    pub x: f64,
}

/// Bad — no semantic version declared.
#[node(category = "Maths & logic", tier = "S")]
pub fn bad(input: In) -> f64 {
    input.x
}

fn main() {}
