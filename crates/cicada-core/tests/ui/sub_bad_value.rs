use cicada_macros::{Ports, node};

#[derive(Ports, Clone, Copy)]
pub struct In {
    /// Value.
    pub x: f64,
}

/// Bad — `sub` is a name, not a blank: an empty (or padded) sub-group
/// would register a node the menu bar has no column for. Everything else
/// is in order — `# Returns` included — so the blank `sub` is the ONLY
/// reason this does not compile (a macro that accepted blanks would
/// compile the case, and trybuild would say so in every mode).
///
/// # Returns
///
/// The value.
#[node(category = "Maths & logic", sub = " ", tier = "S", version = 1, gh = none)]
pub fn bad(input: In) -> f64 {
    input.x
}

fn main() {}
