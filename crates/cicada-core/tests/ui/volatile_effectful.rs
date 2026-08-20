use cicada_macros::{Ports, node};

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    pub x: f64,
}

/// Bad — `volatile` and `effectful` contradict each other: an effectful
/// node already bypasses the memo and never auto-runs (explicit runs only),
/// a volatile node recomputes in EVERY generation. A node wearing both
/// would mean "an exporter that fires every generation" — refused.
///
/// # Returns
///
/// The value.
#[node(category = "Maths & logic", tier = "S", version = 1, gh = none, volatile, effectful)]
pub fn bad(input: In) -> f64 {
    input.x
}

fn main() {}
