use cicada_macros::{Ports, node};

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    pub x: f64,
}

/// Bad — duplicate `version`: the second would silently win and corrupt
/// cache-key semantics.
#[node(category = "Maths & logic", tier = "S", version = 1, version = 2, gh = none)]
pub fn bad(input: In) -> f64 {
    input.x
}

fn main() {}
