use cicada_macros::{Ports, node};

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    pub x: f64,
}

/// Bad — no `gh`: every node names the Grasshopper component it replaces
/// or says `gh = none` (DECISIONS.md stdlib row, 2026-08-19).
#[node(category = "Maths & logic", tier = "S", version = 1)]
pub fn bad(input: In) -> f64 {
    input.x
}

fn main() {}
