use cicada_macros::{Ports, node};

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    pub x: f64,
}

/// Bad — `gh` takes a quoted component name or the bare word `none`;
/// a typo like `None` must not silently register as "no counterpart".
#[node(category = "Maths & logic", tier = "S", version = 1, gh = None)]
pub fn bad(input: In) -> f64 {
    input.x
}

fn main() {}
