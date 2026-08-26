use cicada_macros::{Ports, node};

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    pub x: f64,
}

/// Bad — no `sub`: every node names its sub-group within its category, the
/// menu bar's column (docs/08 §Catalog; v0.1 wave 5, C2c).
#[node(category = "Maths & logic", tier = "S", version = 1, gh = none)]
pub fn bad(input: In) -> f64 {
    input.x
}

fn main() {}
