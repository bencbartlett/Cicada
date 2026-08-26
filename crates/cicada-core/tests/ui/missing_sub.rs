use cicada_macros::{Ports, node};

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    pub x: f64,
}

/// Bad — no `sub`: every node names its sub-group within its category, the
/// menu bar's column (docs/08 §Catalog; v0.1 wave 5, C2c). Everything else
/// is in order — `# Returns` included — so the missing `sub` is the ONLY
/// reason this does not compile: a macro that stopped requiring it would
/// compile the case, and trybuild would say "should not have compiled" in
/// every mode, the overwrite bless included.
///
/// # Returns
///
/// The value.
#[node(category = "Maths & logic", tier = "S", version = 1, gh = none)]
pub fn bad(input: In) -> f64 {
    input.x
}

fn main() {}
